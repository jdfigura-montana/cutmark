//! The frame's background picture (spec "Frame": Image), decoded with FFmpeg into straight RGBA.

use std::path::Path;

use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::Pixel;
use ffmpeg_next::software::scaling;
use media_io::player::video::RgbaFrame;

/// The longest side a picture is kept at; a larger one is scaled down.
const MAX_SIDE: u32 = 4096;

/// Why a background picture could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum BackgroundError {
    /// There is no file at the path.
    #[error("the image is missing")]
    Missing,
    /// The file holds nothing that can be drawn.
    #[error("the file holds no picture")]
    NoPicture,
    /// FFmpeg could not read it.
    #[error("the image could not be read: {0}")]
    Decode(#[from] ffmpeg::Error),
}

/// Reads the first picture of the file at `path`.
pub fn load(path: &Path) -> Result<RgbaFrame, BackgroundError> {
    if !path.is_file() {
        return Err(BackgroundError::Missing);
    }
    ffmpeg::init()?;
    let mut options = ffmpeg::Dictionary::new();
    // A `%` in a file name is part of the name, not a numbered sequence.
    options.set("pattern_type", "none");
    let mut input = ffmpeg::format::input_with_dictionary(path, options)?;
    let stream = input.streams().best(ffmpeg::media::Type::Video).ok_or(BackgroundError::NoPicture)?;
    let index = stream.index();
    let mut decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())?.decoder().video()?;
    let mut picture = ffmpeg::frame::Video::empty();
    let mut decoded = false;
    for (stream, packet) in input.packets() {
        if stream.index() != index {
            continue;
        }
        decoder.send_packet(&packet)?;
        if decoder.receive_frame(&mut picture).is_ok() {
            decoded = true;
            break;
        }
    }
    if !decoded {
        decoder.send_eof()?;
        decoder.receive_frame(&mut picture).map_err(|_| BackgroundError::NoPicture)?;
    }
    to_rgba(&picture)
}

/// Converts a decoded picture to tightly packed RGBA, no longer than [`MAX_SIDE`] on either side.
fn to_rgba(picture: &ffmpeg::frame::Video) -> Result<RgbaFrame, BackgroundError> {
    let (width, height) = (picture.width(), picture.height());
    if width == 0 || height == 0 {
        return Err(BackgroundError::NoPicture);
    }
    let (out_width, out_height) = capped(width, height);
    let mut scaler = scaling::Context::get(picture.format(), width, height, Pixel::RGBA, out_width, out_height, scaling::Flags::BILINEAR)?;
    let mut rgba = ffmpeg::frame::Video::empty();
    scaler.run(picture, &mut rgba)?;
    let row = out_width as usize * 4;
    // FFmpeg pads its rows; the renderer takes them packed.
    let data: Vec<u8> = rgba.data(0).chunks(rgba.stride(0)).take(out_height as usize).flat_map(|line| line.iter().take(row)).copied().collect();
    Ok(RgbaFrame { width: out_width, height: out_height, data, pts_ms: 0 })
}

/// The size a picture is kept at.
fn capped(width: u32, height: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= MAX_SIDE {
        return (width, height);
    }
    let side = |value: u32| ((u64::from(value) * u64::from(MAX_SIDE) / u64::from(longest)) as u32).max(1);
    (side(width), side(height))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// A 2 by 2 PNG: red and green over blue and white.
    const PNG: [u8; 75] = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08,
        0x02, 0x00, 0x00, 0x00, 0xfd, 0xd4, 0x9a, 0x73, 0x00, 0x00, 0x00, 0x12, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xc0, 0x00, 0xc2,
        0x0c, 0xff, 0x81, 0x00, 0x00, 0x1f, 0xee, 0x05, 0xfb, 0xf1, 0xab, 0xba, 0x77, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn a_png_is_read_as_packed_rgba_and_a_missing_or_bad_file_is_an_error() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!("cutmark-studio-background-{}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::create_dir_all(&dir)?;
        let path = dir.join("back %d ground.png");
        fs::write(&path, PNG)?;
        let picture = load(&path)?;
        assert_eq!((picture.width, picture.height), (2, 2));
        assert_eq!(&picture.data[..], [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255]);
        assert!(matches!(load(&dir.join("gone.png")), Err(BackgroundError::Missing)));
        let broken = dir.join("broken.png");
        fs::write(&broken, b"not a picture")?;
        assert!(load(&broken).is_err());
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_large_picture_keeps_its_shape_under_the_cap() {
        assert_eq!(capped(1920, 1080), (1920, 1080));
        assert_eq!(capped(8192, 4096), (4096, 2048));
        assert_eq!(capped(10, 100_000), (1, 4096));
    }
}
