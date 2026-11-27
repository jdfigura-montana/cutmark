//! Reading a cursor's picture with GDI: GetIconInfo for the size and hotspot, DrawIconEx into a
//! 32-bit DIB for the pixels.

use windows::Win32::Graphics::Gdi::{
    BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, GetObjectW, HBITMAP, HDC,
    HGDIOBJ, SelectObject,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DrawIconEx, GetIconInfo, HCURSOR, HICON, ICONINFO};
use windows::core::{s, w};

use super::shape::{MAX_SIDE, ShapeImage, recover_alpha};

/// The most animation frames kept for one cursor.
const MAX_FRAMES: u32 = 256;
/// A larger step count than this is not a real animation.
const IMPLAUSIBLE_STEPS: u32 = 1024;

/// The undocumented user32 export that describes an animated cursor: cursor, reserved (zero), step,
/// then the step's duration in jiffies (1/60 s) and the number of steps. Returns null on failure.
type FrameInfoFn = unsafe extern "system" fn(HCURSOR, usize, u32, *mut u32, *mut u32) -> HCURSOR;

/// Reads cursor pictures.
pub(crate) struct CursorReader {
    /// None when this Windows build does not export GetCursorFrameInfo.
    frame_info: Option<FrameInfoFn>,
}

impl CursorReader {
    #[allow(unsafe_code)]
    pub(crate) fn new() -> Self {
        // SAFETY: user32 is already loaded and both names are NUL terminated literals.
        let found = unsafe { GetModuleHandleW(w!("user32.dll")).ok().and_then(|module| GetProcAddress(module, s!("GetCursorFrameInfo"))) };
        // SAFETY: the export has had this signature since Windows XP; both types are plain
        // function pointers.
        let frame_info = found.map(|address| unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, FrameInfoFn>(address) });
        Self { frame_info }
    }

    /// The picture behind a cursor handle; None when it cannot be read, for example because its
    /// owner destroyed it.
    pub(crate) fn read(&self, cursor: HCURSOR) -> Option<ShapeImage> {
        let icon = IconBitmaps::of(cursor)?;
        let monochrome = icon.color.is_invalid();
        let (width, height) = if monochrome {
            // A monochrome cursor's mask holds the AND picture above the XOR picture.
            let (width, height) = bitmap_size(icon.mask)?;
            (width, height / 2)
        } else {
            bitmap_size(icon.color)?
        };
        if width == 0 || height == 0 {
            return None;
        }
        // Scale an oversized cursor down, keeping its proportions.
        let longest = width.max(height);
        let scaled = |value: u32| if longest > MAX_SIDE { (u64::from(value) * u64::from(MAX_SIDE) / u64::from(longest)) as u32 } else { value };
        let hotspot = (scaled(icon.hotspot.0), scaled(icon.hotspot.1));
        let (width, height) = (scaled(width).max(1), scaled(height).max(1));
        let canvas = Canvas::new(width, height)?;
        let mut frames = Vec::new();
        for (step, duration_us) in self.steps(cursor).into_iter().enumerate() {
            let on_black = canvas.draw(cursor, step as u32, 0x00)?;
            let on_white = canvas.draw(cursor, step as u32, 0xFF)?;
            frames.push((duration_us, recover_alpha(&on_black, &on_white)));
        }
        Some(ShapeImage::from_mattes(width, height, hotspot, monochrome, frames))
    }

    /// Each animation step's duration in microseconds; one zero for a cursor that is not animated.
    #[allow(unsafe_code)]
    fn steps(&self, cursor: HCURSOR) -> Vec<u32> {
        let Some(frame_info) = self.frame_info else { return vec![0] };
        let query = |step: u32| -> Option<(u32, u32)> {
            let (mut jiffies, mut count) = (0u32, 0u32);
            // SAFETY: both out pointers point at locals; a bad handle or step returns null.
            let frame = unsafe { frame_info(cursor, 0, step, &mut jiffies, &mut count) };
            (!frame.is_invalid()).then_some((jiffies, count))
        };
        let count = match query(0) {
            Some((_, count)) if (2..=IMPLAUSIBLE_STEPS).contains(&count) => count.min(MAX_FRAMES),
            _ => return vec![0],
        };
        (0..count).map(|step| query(step).map_or(0, |(jiffies, _)| (u64::from(jiffies) * 1_000_000 / 60).min(u64::from(u32::MAX)) as u32)).collect()
    }
}

/// The two bitmaps GetIconInfo creates for the caller, deleted on drop.
struct IconBitmaps {
    mask: HBITMAP,
    /// Null for a monochrome cursor.
    color: HBITMAP,
    hotspot: (u32, u32),
}

impl IconBitmaps {
    #[allow(unsafe_code)]
    fn of(cursor: HCURSOR) -> Option<Self> {
        let mut info = ICONINFO::default();
        // SAFETY: info is a valid out pointer; a stale handle only makes the call fail.
        unsafe { GetIconInfo(HICON(cursor.0), &mut info) }.ok()?;
        Some(Self { mask: info.hbmMask, color: info.hbmColor, hotspot: (info.xHotspot, info.yHotspot) })
    }
}

impl Drop for IconBitmaps {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        for bitmap in [self.mask, self.color] {
            if !bitmap.is_invalid() {
                // SAFETY: the bitmap is ours and deleted exactly once.
                let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
            }
        }
    }
}

/// A bitmap's width and height.
#[allow(unsafe_code)]
fn bitmap_size(bitmap: HBITMAP) -> Option<(u32, u32)> {
    let mut description = BITMAP::default();
    let size = std::mem::size_of::<BITMAP>() as i32;
    // SAFETY: the buffer is a BITMAP and size is its size.
    let written = unsafe { GetObjectW(HGDIOBJ(bitmap.0), size, Some((&raw mut description).cast())) };
    (written == size).then(|| (description.bmWidth.unsigned_abs(), description.bmHeight.unsigned_abs()))
}

/// A top-down 32-bit DIB selected into its own memory DC.
struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    /// The DIB's pixel memory, `length` bytes, owned by `bitmap`.
    bits: *mut u8,
    length: usize,
    width: i32,
    height: i32,
}

impl Canvas {
    #[allow(unsafe_code)]
    fn new(width: u32, height: u32) -> Option<Self> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                // Negative height: the first row in memory is the top row.
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: a memory DC compatible with the screen; checked before use.
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            return None;
        }
        let mut bits = std::ptr::null_mut();
        // SAFETY: info describes the bitmap and bits receives the pixel pointer.
        let bitmap = match unsafe { CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) } {
            Ok(bitmap) if !bits.is_null() => bitmap,
            _ => {
                // SAFETY: dc is ours and not used again.
                let _ = unsafe { DeleteDC(dc) };
                return None;
            }
        };
        // SAFETY: both objects are live; the previous bitmap is restored on drop.
        let previous = unsafe { SelectObject(dc, HGDIOBJ(bitmap.0)) };
        let length = width as usize * height as usize * 4;
        Some(Self { dc, bitmap, previous, bits: bits.cast(), length, width: width as i32, height: height as i32 })
    }

    /// Draws one animation step over a background filled with one byte value and returns the pixels.
    #[allow(unsafe_code)]
    fn draw(&self, cursor: HCURSOR, step: u32, background: u8) -> Option<Vec<u8>> {
        // SAFETY: bits points at `length` bytes of the live DIB, and no GDI call is pending.
        unsafe { std::ptr::write_bytes(self.bits, background, self.length) };
        // SAFETY: the DC holds our DIB; a stale cursor handle only makes the call fail.
        unsafe { DrawIconEx(self.dc, 0, 0, HICON(cursor.0), self.width, self.height, step, None, DI_NORMAL) }.ok()?;
        // SAFETY: plain call; it makes GDI finish before the memory is read.
        let _ = unsafe { GdiFlush() };
        // SAFETY: bits points at `length` initialized bytes of the live DIB.
        Some(unsafe { std::slice::from_raw_parts(self.bits, self.length) }.to_vec())
    }
}

impl Drop for Canvas {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the objects are ours; the DIB is deselected before it is deleted.
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.dc);
        }
    }
}
