//! Lists cameras and measures a webcam capture. Run with:
//!   cargo run -p capture --example webcam_probe -- --list
//!   cargo run -p capture --example webcam_probe -- --device 0 --format 0 --seconds 5
//! Writes webcam_probe_luma.bmp (the Y plane of the last frame as grayscale).

#[cfg(windows)]
mod probe {
    use std::time::{Duration, Instant};

    use capture::{Gpu, WebcamCapture, list_cameras};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, ID3D11DeviceContext, ID3D11Texture2D,
    };

    /// The options parsed from the command line.
    struct Options {
        list: bool,
        device: usize,
        format: usize,
        seconds: u64,
    }

    /// Parses --list, --device N, --format M and --seconds S.
    fn parse_args() -> Result<Options, Box<dyn std::error::Error>> {
        let mut options = Options { list: false, device: 0, format: 0, seconds: 5 };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--list" => options.list = true,
                "--device" => options.device = args.next().ok_or("--device needs a number")?.parse()?,
                "--format" => options.format = args.next().ok_or("--format needs a number")?.parse()?,
                "--seconds" => options.seconds = args.next().ok_or("--seconds needs a number")?.parse()?,
                other => return Err(format!("unknown argument {other}; use --list or --device N --format M --seconds S").into()),
            }
        }
        Ok(options)
    }

    /// Runs the webcam probe.
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let options = parse_args()?;
        let cameras = list_cameras()?;
        if options.list {
            if cameras.is_empty() {
                println!("No cameras found");
            }
            for (index, camera) in cameras.iter().enumerate() {
                println!("[{index}] {} ({})", camera.name, camera.id);
                for (format_index, format) in camera.formats.iter().enumerate() {
                    println!("    [{format_index}] {format}");
                }
            }
            return Ok(());
        }

        let camera = cameras.get(options.device).ok_or_else(|| format!("no camera with index {}; run with --list", options.device))?;
        let format = camera.formats.get(options.format).ok_or_else(|| format!("camera {} has no format with index {}", options.device, options.format))?;
        let gpu = Gpu::create(Some(0))?;
        println!("Device on adapter [{}]: {}", gpu.adapter().index, gpu.adapter().name);
        println!("Camera [{}] {}, format [{}] {format}", options.device, camera.name, options.format);
        let capture = WebcamCapture::start(&gpu, &camera.id, format)?;
        let start = Instant::now();
        let end = Duration::from_secs(options.seconds);
        let mut last = None;
        while start.elapsed() < end {
            if let Some(error) = capture.error() {
                println!("Camera error: {error}");
                break;
            }
            if let Some(frame) = capture.latest() {
                last = Some(frame);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let stats = capture.stats();
        let path = capture.path_used();
        let error = capture.error();
        if let Some(frame) = capture.latest() {
            last = Some(frame);
        }
        let elapsed = start.elapsed().as_secs_f64();

        println!("Frames received: {}", stats.frames);
        println!("Effective fps: {:.2} (frames over {elapsed:.2} s)", stats.frames as f64 / elapsed);
        if stats.interval_mean_ms > 0.0 {
            println!("Effective fps from the mean interval: {:.2}", 1000.0 / stats.interval_mean_ms);
        }
        println!("Frame interval: mean {:.3} ms, standard deviation {:.3} ms", stats.interval_mean_ms, stats.interval_std_ms);
        println!("Arrival minus sample time: mean {:.3} ms, max absolute {:.3} ms", stats.offset_mean_ms, stats.offset_max_abs_ms);
        println!("Path used: {path}");
        if let Some(error) = error {
            println!("Capture ended with an error: {error}");
        }

        let frame = last.ok_or("no frame arrived")?;
        println!("Last frame: seq {}, sample QPC {}", frame.seq, frame.qpc);
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: the frame texture is a live D3D11 texture; GetDesc only writes the descriptor.
        #[allow(unsafe_code)]
        unsafe {
            frame.texture.GetDesc(&mut desc)
        };
        let context = gpu.context().clone();
        let staging = create_staging(gpu.device(), &frame.texture)?;
        // SAFETY: both textures are live, on the same device, with matching size and format.
        #[allow(unsafe_code)]
        unsafe {
            context.CopyResource(&staging, &frame.texture)
        };
        let luma = read_luma(&context, &staging, desc.Width, desc.Height)?;
        let min = luma.iter().min().ok_or("the Y plane is empty")?;
        let max = luma.iter().max().ok_or("the Y plane is empty")?;
        let sum: u64 = luma.iter().map(|&b| u64::from(b)).sum();
        println!("Y plane: min {min} max {max} average {:.2}", sum as f64 / luma.len() as f64);
        write_gray_bmp("webcam_probe_luma.bmp", &luma, desc.Width, desc.Height)?;
        println!("Wrote webcam_probe_luma.bmp");
        capture.stop();
        Ok(())
    }

    /// Creates a CPU-readable staging copy of the given texture.
    #[allow(unsafe_code)]
    fn create_staging(
        device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
        texture: &ID3D11Texture2D,
    ) -> Result<ID3D11Texture2D, Box<dyn std::error::Error>> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: texture is a live D3D11 texture; GetDesc only writes the descriptor.
        unsafe { texture.GetDesc(&mut desc) };
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut created: Option<ID3D11Texture2D> = None;
        // SAFETY: desc comes from the live source texture; created is a valid out pointer.
        let result = unsafe { device.CreateTexture2D(&desc, None, Some(&mut created)) };
        result.map_err(|e| format!("failed to create the staging texture: {e}"))?;
        created.ok_or_else(|| "CreateTexture2D returned no staging texture".into())
    }

    /// Maps the staging texture and returns the packed Y plane.
    #[allow(unsafe_code)]
    fn read_luma(context: &ID3D11DeviceContext, staging: &ID3D11Texture2D, width: u32, height: u32) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: staging is a live texture and mapped is a valid out pointer.
        let result = unsafe { context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) };
        result.map_err(|e| format!("failed to map the staging texture: {e}"))?;
        let pitch = mapped.RowPitch as usize;
        let row_bytes = width as usize;
        let luma = if mapped.pData.is_null() || pitch < row_bytes {
            None
        } else {
            // SAFETY: pData points at pitch times all rows of the mapped NV12 texture, and the Y plane is the first `height` rows.
            let data = unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, pitch * height as usize) };
            let mut luma = Vec::with_capacity(row_bytes * height as usize);
            for row in 0..height as usize {
                luma.extend_from_slice(&data[row * pitch..row * pitch + row_bytes]);
            }
            Some(luma)
        };
        // SAFETY: staging is the texture that was mapped above.
        unsafe { context.Unmap(staging, 0) };
        luma.ok_or_else(|| "the Y plane was not mapped".into())
    }

    /// Writes an 8-bit grayscale BMP with a hand-built header and palette.
    fn write_gray_bmp(path: &str, pixels: &[u8], width: u32, height: u32) -> std::io::Result<()> {
        let width = width as usize;
        let height = height as usize;
        let stride = (width + 3) & !3;
        let pixel_bytes = stride * height;
        let data_offset = 14 + 40 + 256 * 4;
        let mut out = Vec::with_capacity(data_offset + pixel_bytes);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&((data_offset + pixel_bytes) as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(data_offset as u32).to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(width as i32).to_le_bytes());
        out.extend_from_slice(&(height as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&8u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
        out.extend_from_slice(&2835u32.to_le_bytes());
        out.extend_from_slice(&2835u32.to_le_bytes());
        out.extend_from_slice(&256u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for value in 0..256u32 {
            let gray = value as u8;
            out.extend_from_slice(&[gray, gray, gray, 0]);
        }
        for row in (0..height).rev() {
            let start = row * width;
            out.extend_from_slice(&pixels[start..start + width]);
            out.extend(std::iter::repeat_n(0u8, stride - width));
        }
        std::fs::write(path, out)
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    probe::run()
}

#[cfg(not(windows))]
fn main() {
    println!("webcam_probe is only available on Windows");
}
