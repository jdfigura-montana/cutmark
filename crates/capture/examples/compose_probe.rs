//! Composes one frame of the first monitor plus a camera in the picture in picture and cam only layouts,
//! and writes `compose_pip.bmp` and `compose_camonly.bmp` into the current folder.
//!
//! Run with `cargo run -p capture --example compose_probe -- [--device N]`, where N is the camera index.

use std::error::Error;
use std::time::{Duration, Instant};

use capture::{CamFrame, CameraFormat, Compositor, Gpu, Layout, MonitorCapture, PipSettings, WebcamCapture, list_cameras, list_monitors, read_bgra};

/// The size of the composed frames.
const OUT_W: u32 = 1920;
const OUT_H: u32 = 1080;

fn main() -> Result<(), Box<dyn Error>> {
    let device = parse_device();
    let gpu = Gpu::create(Some(0))?;
    println!("adapter: {}", gpu.adapter().name);

    let monitor = list_monitors()?.into_iter().next().ok_or("no monitor was found")?;
    let screen = MonitorCapture::start(&gpu, &monitor, true)?;
    let screen_frame = wait_for(Duration::from_secs(3), || screen.latest()).ok_or("no screen frame arrived")?;
    println!("screen: {}x{}", screen_frame.width, screen_frame.height);

    let cameras = list_cameras()?;
    let camera = cameras.get(device).ok_or_else(|| format!("no camera with index {device}; {} found", cameras.len()))?;
    let format = pick_format(&camera.formats).ok_or("the camera offers no formats")?;
    println!("camera {device}: {} at {}x{} {}", camera.name, format.width, format.height, format.subtype);
    let webcam = WebcamCapture::start(&gpu, &camera.id, &format)?;
    let first: CamFrame = wait_for(Duration::from_secs(8), || webcam.latest()).ok_or("no camera frame arrived")?;
    println!("first camera frame: {}, colour {:?}", first.seq, webcam.color());
    // Give auto exposure a moment, then take the newest frame.
    std::thread::sleep(Duration::from_millis(700));
    let cam_frame = webcam.latest().unwrap_or(first);
    let screen_frame = screen.latest().unwrap_or(screen_frame);

    let mut compositor = Compositor::new(&gpu, OUT_W, OUT_H)?;
    compositor.set_cam_color(webcam.color());
    let pip = PipSettings { border_px: 4.0, radius_px: 24.0, ..PipSettings::default() };
    let background = [32, 32, 32];

    let composed = compositor.compose(Some(&screen_frame.texture), Some(&cam_frame), &Layout::Pip, &pip, background)?;
    write_texture(&gpu, &composed.texture, "compose_pip.bmp")?;
    println!("wrote compose_pip.bmp (frame {})", composed.seq);

    let composed = compositor.compose(Some(&screen_frame.texture), Some(&cam_frame), &Layout::CamOnly, &pip, background)?;
    write_texture(&gpu, &composed.texture, "compose_camonly.bmp")?;
    println!("wrote compose_camonly.bmp (frame {})", composed.seq);

    webcam.stop();
    screen.stop();
    Ok(())
}

/// The camera index from `--device N`, 0 when it is missing.
fn parse_device() -> usize {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--device" {
            return args.next().and_then(|value| value.parse().ok()).unwrap_or(0);
        }
    }
    0
}

/// The largest mode up to 1920x1080 at about 30 fps, or the first one offered.
fn pick_format(formats: &[CameraFormat]) -> Option<CameraFormat> {
    formats
        .iter()
        .filter(|format| format.width <= 1920 && format.height <= 1080 && (29.0..=31.0).contains(&format.fps()))
        .max_by_key(|format| u64::from(format.width) * u64::from(format.height))
        .or_else(|| formats.first())
        .copied()
}

/// Calls `get` until it returns something or the time is up.
fn wait_for<T>(limit: Duration, get: impl Fn() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if let Some(value) = get() {
            return Some(value);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

/// Reads a composed texture back and writes it as a top-down 32 bit BMP.
fn write_texture(gpu: &Gpu, texture: &windows::Win32::Graphics::Direct3D11::ID3D11Texture2D, path: &str) -> Result<(), Box<dyn Error>> {
    let (width, height, bgra) = read_bgra(gpu, texture)?;
    let pixel_bytes = u32::try_from(bgra.len())?;
    let mut out = Vec::with_capacity(54 + bgra.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + pixel_bytes).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&i32::try_from(width)?.to_le_bytes());
    out.extend_from_slice(&(-i32::try_from(height)?).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&pixel_bytes.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&bgra);
    std::fs::write(path, out)?;
    Ok(())
}
