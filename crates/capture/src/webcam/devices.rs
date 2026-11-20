//! Camera enumeration and native format listing through Media Foundation.
#![allow(unsafe_code)]

use std::cmp::Ordering;
use std::ffi::c_void;
use std::fmt;

use super::{ComGuard, MfGuard};
use crate::{CaptureError, with_context};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFAttributes, IMFMediaSource, IMFMediaType, IMFSourceReader, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_SUBTYPE,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MFCreateAttributes, MFCreateSourceReaderFromMediaSource, MFEnumDeviceSources,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::core::{GUID, PWSTR};

/// The pixel format of one camera mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CameraSubtype {
    /// Motion JPEG, decoded by Media Foundation.
    Mjpg,
    /// Planar 4:2:0 with interleaved chroma.
    Nv12,
    /// Packed 4:2:2.
    Yuy2,
    /// Any other format, kept as its FourCC code.
    Other(u32),
}

/// The Media Foundation base GUID tail shared by every FourCC based video subtype.
const FOURCC_GUID_DATA2: u16 = 0;
const FOURCC_GUID_DATA3: u16 = 0x0010;
const FOURCC_GUID_DATA4: [u8; 8] = [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71];

impl CameraSubtype {
    /// Maps a Media Foundation subtype GUID to a CameraSubtype.
    pub fn from_guid(guid: &GUID) -> Self {
        if guid.data2 != FOURCC_GUID_DATA2 || guid.data3 != FOURCC_GUID_DATA3 || guid.data4 != FOURCC_GUID_DATA4 {
            return Self::Other(0);
        }
        Self::from_fourcc(guid.data1)
    }

    /// Maps a FourCC code to a CameraSubtype.
    fn from_fourcc(code: u32) -> Self {
        match &code.to_le_bytes() {
            b"MJPG" => Self::Mjpg,
            b"NV12" => Self::Nv12,
            b"YUY2" => Self::Yuy2,
            _ => Self::Other(code),
        }
    }

    /// The FourCC code of this subtype.
    pub fn fourcc(self) -> u32 {
        match self {
            Self::Mjpg => u32::from_le_bytes(*b"MJPG"),
            Self::Nv12 => u32::from_le_bytes(*b"NV12"),
            Self::Yuy2 => u32::from_le_bytes(*b"YUY2"),
            Self::Other(code) => code,
        }
    }
}

impl fmt::Display for CameraSubtype {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.fourcc().to_le_bytes() {
            let ch = if byte.is_ascii_graphic() { char::from(byte) } else { '?' };
            write!(f, "{ch}")?;
        }
        Ok(())
    }
}

/// One mode a camera offers: pixel format, frame size and frame rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CameraFormat {
    /// The pixel format.
    pub subtype: CameraSubtype,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Frame rate numerator.
    pub fps_num: u32,
    /// Frame rate denominator.
    pub fps_den: u32,
}

impl CameraFormat {
    /// The frame rate as a floating point number, or 0 when the denominator is 0.
    pub fn fps(&self) -> f64 {
        if self.fps_den == 0 {
            return 0.0;
        }
        f64::from(self.fps_num) / f64::from(self.fps_den)
    }
}

impl fmt::Display for CameraFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}x{} @ {:.2} fps ({}/{})", self.subtype, self.width, self.height, self.fps(), self.fps_num, self.fps_den)
    }
}

/// A camera found on this machine with every format it offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraInfo {
    /// The symbolic link that identifies the device.
    pub id: String,
    /// The friendly name shown to the user.
    pub name: String,
    /// The formats the camera offers, largest size first, then highest frame rate.
    pub formats: Vec<CameraFormat>,
}

/// Lists every video capture device with its formats.
///
/// A camera that is busy or fails to open is still listed, with no formats.
pub fn list_cameras() -> Result<Vec<CameraInfo>, CaptureError> {
    let _com = ComGuard::init();
    let _mf = MfGuard::acquire()?;
    let activates = enumerate_activates()?;
    let mut cameras = Vec::with_capacity(activates.len());
    for activate in &activates {
        let Some(id) = attribute_string(activate, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK) else {
            log::warn!("skipping a camera without a symbolic link");
            continue;
        };
        let name = attribute_string(activate, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME).unwrap_or_else(|| id.clone());
        let formats = match read_formats(activate) {
            Ok(formats) => formats,
            Err(error) => {
                log::warn!("could not read the formats of camera {name}: {error}");
                Vec::new()
            }
        };
        cameras.push(CameraInfo { id, name, formats });
    }
    Ok(cameras)
}

/// Removes duplicate formats and sorts by frame size then frame rate, both descending.
pub(crate) fn dedupe_and_sort(mut formats: Vec<CameraFormat>) -> Vec<CameraFormat> {
    formats.sort_by(compare_formats);
    formats.dedup();
    formats
}

/// Orders larger frames first, then higher frame rates, then a fixed tie break.
fn compare_formats(a: &CameraFormat, b: &CameraFormat) -> Ordering {
    let area = |f: &CameraFormat| u64::from(f.width) * u64::from(f.height);
    area(b)
        .cmp(&area(a))
        .then(b.width.cmp(&a.width))
        .then_with(|| compare_fps(b, a))
        .then(a.subtype.cmp(&b.subtype))
        .then(a.fps_num.cmp(&b.fps_num))
        .then(a.fps_den.cmp(&b.fps_den))
}

/// Compares two frame rates as exact fractions.
fn compare_fps(a: &CameraFormat, b: &CameraFormat) -> Ordering {
    (u64::from(a.fps_num) * u64::from(b.fps_den)).cmp(&(u64::from(b.fps_num) * u64::from(a.fps_den)))
}

/// Packs two 32 bit values the way Media Foundation stores sizes and ratios.
pub(crate) fn pack_pair(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

/// Splits a packed Media Foundation size or ratio into its high and low halves.
fn unpack_pair(value: u64) -> (u32, u32) {
    ((value >> 32) as u32, value as u32)
}

/// Reads the format out of a native media type, or None when it is not a sized video type.
fn format_of(media_type: &IMFMediaType) -> Option<CameraFormat> {
    // SAFETY: the media type is a live COM object and each getter only reads an attribute.
    let subtype = unsafe { media_type.GetGUID(&MF_MT_SUBTYPE) }.ok()?;
    // SAFETY: as above.
    let (width, height) = unpack_pair(unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }.ok()?);
    // SAFETY: as above.
    let (fps_num, fps_den) = unpack_pair(unsafe { media_type.GetUINT64(&MF_MT_FRAME_RATE) }.unwrap_or(0));
    if width == 0 || height == 0 {
        return None;
    }
    Some(CameraFormat { subtype: CameraSubtype::from_guid(&subtype), width, height, fps_num, fps_den })
}

/// Every native media type of the first video stream, with its parsed format.
pub(crate) fn native_types(reader: &IMFSourceReader) -> Vec<(CameraFormat, IMFMediaType)> {
    let mut types = Vec::new();
    let mut index = 0u32;
    // SAFETY: the reader is a live COM object; GetNativeMediaType fails with
    // MF_E_NO_MORE_TYPES once the index runs past the last type.
    while let Ok(media_type) = unsafe { reader.GetNativeMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, index) } {
        if let Some(format) = format_of(&media_type) {
            types.push((format, media_type));
        }
        index += 1;
    }
    types
}

/// Opens the camera briefly and lists its deduplicated, sorted formats.
fn read_formats(activate: &IMFActivate) -> Result<Vec<CameraFormat>, CaptureError> {
    // SAFETY: the activate object is live; the source is shut down below.
    let source: IMFMediaSource = with_context("failed to activate the camera", unsafe { activate.ActivateObject() })?;
    let formats = formats_of_source(&source);
    // SAFETY: shuts down the object this function activated, after every user of it is dropped.
    let _ = unsafe { activate.ShutdownObject() };
    formats.map(dedupe_and_sort)
}

/// Lists the native formats of an activated camera source.
fn formats_of_source(source: &IMFMediaSource) -> Result<Vec<CameraFormat>, CaptureError> {
    // SAFETY: the source is live; no attributes are passed.
    let reader =
        with_context("failed to create a source reader for the camera", unsafe { MFCreateSourceReaderFromMediaSource(source, None::<&IMFAttributes>) })?;
    Ok(native_types(&reader).into_iter().map(|(format, _)| format).collect())
}

/// Enumerates the video capture devices as activation objects.
fn enumerate_activates() -> Result<Vec<IMFActivate>, CaptureError> {
    let mut attributes: Option<IMFAttributes> = None;
    // SAFETY: attributes is a valid out pointer.
    with_context("failed to create the device query", unsafe { MFCreateAttributes(&mut attributes, 1) })?;
    let attributes = attributes.ok_or(CaptureError::Missing("MFCreateAttributes returned no attributes"))?;
    // SAFETY: attributes is live and both GUID pointers point at constants.
    with_context("failed to set the device query type", unsafe {
        attributes.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)
    })?;
    let mut array: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: both out pointers are valid; on success the call hands us a
    // CoTaskMem allocated array of count owned interface pointers.
    with_context("failed to enumerate the cameras", unsafe { MFEnumDeviceSources(&attributes, &mut array, &mut count) })?;
    let mut activates = Vec::with_capacity(count as usize);
    if !array.is_null() {
        for index in 0..count as usize {
            // SAFETY: index is below count, and each element is read exactly once,
            // moving its reference into the Vec so it is released on drop.
            if let Some(activate) = unsafe { array.add(index).read() } {
                activates.push(activate);
            }
        }
        // SAFETY: the array was allocated by Media Foundation with CoTaskMemAlloc.
        unsafe { CoTaskMemFree(Some(array as *const c_void)) };
    }
    Ok(activates)
}

/// Reads a string attribute, or None when it is missing or not valid UTF-16.
fn attribute_string(attributes: &IMFAttributes, key: &GUID) -> Option<String> {
    let mut value = PWSTR::null();
    let mut length = 0u32;
    // SAFETY: both out pointers are valid; on success value holds a CoTaskMem string.
    unsafe { attributes.GetAllocatedString(key, &mut value, &mut length) }.ok()?;
    // SAFETY: value is the NUL terminated string just returned.
    let text = unsafe { value.to_string() }.ok();
    // SAFETY: value was allocated by Media Foundation with CoTaskMemAlloc.
    unsafe { CoTaskMemFree(Some(value.0 as *const c_void)) };
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(subtype: CameraSubtype, width: u32, height: u32, fps_num: u32, fps_den: u32) -> CameraFormat {
        CameraFormat { subtype, width, height, fps_num, fps_den }
    }

    #[test]
    fn duplicates_are_removed_wherever_they_appear() {
        let input = vec![
            format(CameraSubtype::Mjpg, 1280, 720, 30, 1),
            format(CameraSubtype::Mjpg, 1920, 1080, 30, 1),
            format(CameraSubtype::Mjpg, 1280, 720, 30, 1),
            format(CameraSubtype::Mjpg, 1920, 1080, 30, 1),
            format(CameraSubtype::Mjpg, 1280, 720, 30, 1),
        ];
        let output = dedupe_and_sort(input);
        assert_eq!(output, vec![format(CameraSubtype::Mjpg, 1920, 1080, 30, 1), format(CameraSubtype::Mjpg, 1280, 720, 30, 1)]);
    }

    #[test]
    fn formats_that_differ_only_by_subtype_are_kept() {
        let output = dedupe_and_sort(vec![format(CameraSubtype::Yuy2, 640, 480, 30, 1), format(CameraSubtype::Nv12, 640, 480, 30, 1)]);
        assert_eq!(output.len(), 2);
    }

    #[test]
    fn sort_is_by_size_then_fps_descending() {
        let input = vec![
            format(CameraSubtype::Nv12, 640, 480, 30, 1),
            format(CameraSubtype::Mjpg, 1920, 1080, 30, 1),
            format(CameraSubtype::Mjpg, 1280, 720, 60, 1),
            format(CameraSubtype::Mjpg, 1920, 1080, 60, 1),
            format(CameraSubtype::Mjpg, 1280, 720, 30000, 1001),
            format(CameraSubtype::Mjpg, 1920, 1080, 15, 1),
        ];
        let output: Vec<(u32, u32, u32, u32)> = dedupe_and_sort(input).iter().map(|f| (f.width, f.height, f.fps_num, f.fps_den)).collect();
        assert_eq!(
            output,
            vec![(1920, 1080, 60, 1), (1920, 1080, 30, 1), (1920, 1080, 15, 1), (1280, 720, 60, 1), (1280, 720, 30000, 1001), (640, 480, 30, 1)]
        );
    }

    #[test]
    fn frame_rates_compare_as_exact_fractions() {
        let ntsc = format(CameraSubtype::Mjpg, 640, 480, 30000, 1001);
        let thirty = format(CameraSubtype::Mjpg, 640, 480, 30, 1);
        assert_eq!(compare_fps(&ntsc, &thirty), Ordering::Less);
        assert_eq!(compare_fps(&thirty, &ntsc), Ordering::Greater);
        assert_eq!(compare_fps(&thirty, &format(CameraSubtype::Mjpg, 640, 480, 60, 2)), Ordering::Equal);
    }

    #[test]
    fn size_and_ratio_pack_round_trips() {
        assert_eq!(unpack_pair(pack_pair(1920, 1080)), (1920, 1080));
        assert_eq!(unpack_pair(pack_pair(30000, 1001)), (30000, 1001));
    }

    #[test]
    fn subtype_guids_map_to_their_fourcc() {
        let guid = GUID::from_values(u32::from_le_bytes(*b"MJPG"), FOURCC_GUID_DATA2, FOURCC_GUID_DATA3, FOURCC_GUID_DATA4);
        assert_eq!(CameraSubtype::from_guid(&guid), CameraSubtype::Mjpg);
        let other = GUID::from_values(u32::from_le_bytes(*b"H264"), FOURCC_GUID_DATA2, FOURCC_GUID_DATA3, FOURCC_GUID_DATA4);
        assert_eq!(CameraSubtype::from_guid(&other).to_string(), "H264");
        assert_eq!(CameraSubtype::from_guid(&GUID::from_u128(1)), CameraSubtype::Other(0));
    }
}
