//! Probes the media-io D3D11 frame pool: captures one frame of the primary
//! monitor, converts it to NV12 and copies it into ten pool frames.

use capture::{Gpu, LatestFrame, MonitorCapture, Nv12Converter, list_monitors};
use media_io::{D3d11Device, D3d11FramePool, HwFrame};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    media_io::init()?;
    let gpu = Gpu::create(Some(0))?;
    let hw_device = D3d11Device::from_device(gpu.device())?;
    let pool = D3d11FramePool::new(&hw_device, 1920, 1080, 8)?;

    let monitor = list_monitors()?.into_iter().find(|m| m.primary).ok_or("no primary monitor")?;
    let capture = MonitorCapture::start(&gpu, &monitor, false)?;
    let frame = wait_for_frame(&capture).ok_or("no frame arrived from the capture session")?;
    println!("captured {}x{}", frame.width, frame.height);

    let mut converter = Nv12Converter::new(&gpu, frame.width as u32, frame.height as u32, 1920, 1080, 30)?;
    let nv12 = converter.create_output()?;
    converter.convert(&frame.texture, &nv12)?;

    for i in 0..10i64 {
        // The pool holds eight slices, so each frame is dropped before the
        // next one is taken.
        let mut hw = HwFrame::from_pool(&pool)?;
        hw.copy_from(gpu.context(), &nv12)?;
        hw.set_pts(i);
        let desc = hw.texture_desc().ok_or("pool frame has no texture")?;
        println!("frame {i}: array index {}, pool texture {}x{} format {:?}", hw.array_index(), desc.Width, desc.Height, desc.Format);
    }
    capture.stop();
    println!("clean shutdown");
    Ok(())
}

/// Waits for the first captured frame.
fn wait_for_frame(capture: &MonitorCapture) -> Option<LatestFrame> {
    for _ in 0..100 {
        if let Some(frame) = capture.latest() {
            return Some(frame);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    None
}
