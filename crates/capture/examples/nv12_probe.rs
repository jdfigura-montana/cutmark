//! Converts one captured frame of the primary monitor to 1920x1080 NV12 and
//! checks the result. Writes nv12_probe_luma.bmp (the Y plane as grayscale)
//! and times 300 conversions. Run with: cargo run -p capture --example nv12_probe.

#[cfg(windows)]
mod probe {
    use std::time::{Duration, Instant};

    use capture::{Gpu, MonitorCapture, Nv12Converter, list_monitors};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, ID3D11DeviceContext, ID3D11Texture2D,
    };

    /// Runs the NV12 conversion probe.
    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let monitors = list_monitors()?;
        let monitor = monitors.iter().find(|monitor| monitor.primary).ok_or("no primary monitor found")?;
        let gpu = Gpu::create(Some(0))?;
        println!("Device on adapter [{}]: {}", gpu.adapter().index, gpu.adapter().name);
        let capture = MonitorCapture::start(&gpu, monitor, false)?;
        println!("Waiting for a frame of {}x{}...", monitor.width, monitor.height);
        let mut frame = None;
        let start = Instant::now();
        while frame.is_none() && start.elapsed() < Duration::from_secs(2) {
            if let Some(latest) = capture.latest() {
                frame = Some(latest);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let frame = frame.ok_or("no frame arrived within 2 seconds")?;
        println!("Got frame {} (seq {})", frame.seq, capture.frames_received());
        let mut converter = Nv12Converter::new(&gpu, frame.width as u32, frame.height as u32, 1920, 1080, 30)?;
        let output = converter.create_output()?;
        converter.convert(&frame.texture, &output)?;
        println!("Converted {}x{} BGRA to 1920x1080 NV12", frame.width, frame.height);

        let context = gpu.context().clone();
        let staging = create_staging(gpu.device(), &output)?;
        copy_to_staging(&context, &output, &staging)?;
        let (luma, chroma) = read_planes(&context, &staging, 1920, 1080)?;
        let min = luma.iter().min().ok_or("the Y plane is empty")?;
        let max = luma.iter().max().ok_or("the Y plane is empty")?;
        let y_sum: u64 = luma.iter().map(|&b| u64::from(b)).sum();
        let y_avg = y_sum as f64 / luma.len() as f64;
        let uv_sum: u64 = chroma.iter().map(|&b| u64::from(b)).sum();
        let uv_avg = uv_sum as f64 / chroma.len() as f64;
        println!("Y plane: min {} max {} average {:.2}", min, max, y_avg);
        println!("UV plane: average {:.2}", uv_avg);
        write_gray_bmp("nv12_probe_luma.bmp", &luma, 1920, 1080)?;
        println!("Wrote nv12_probe_luma.bmp");

        let start = Instant::now();
        for _ in 0..300 {
            converter.convert(&frame.texture, &output)?;
        }
        let per_ms = start.elapsed().as_secs_f64() * 1000.0 / 300.0;
        println!("Average conversion time over 300 runs: {per_ms:.3} ms");
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

    /// Copies the NV12 output texture into the staging texture.
    #[allow(unsafe_code)]
    fn copy_to_staging(context: &ID3D11DeviceContext, output: &ID3D11Texture2D, staging: &ID3D11Texture2D) -> Result<(), Box<dyn std::error::Error>> {
        // SAFETY: both textures are live, on the same device, with matching size and format.
        unsafe { context.CopyResource(staging, output) };
        Ok(())
    }

    /// Maps the staging texture and returns the packed Y plane and UV plane.
    #[allow(unsafe_code)]
    fn read_planes(
        context: &ID3D11DeviceContext,
        staging: &ID3D11Texture2D,
        width: u32,
        height: u32,
    ) -> Result<(Vec<u8>, Vec<u8>), Box<dyn std::error::Error>> {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: staging is a live texture and mapped is a valid out pointer.
        let result = unsafe { context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) };
        result.map_err(|e| format!("failed to map the staging texture: {e}"))?;
        let planes = copy_mapped(&mapped, width, height);
        // SAFETY: staging is the texture that was mapped above.
        unsafe { context.Unmap(staging, 0) };
        let (luma, chroma) = planes;
        Ok((luma.ok_or("the Y plane was not mapped")?, chroma.ok_or("the UV plane was not mapped")?))
    }

    /// Copies both NV12 planes out of the mapped memory as packed byte rows.
    #[allow(unsafe_code)]
    fn copy_mapped(mapped: &D3D11_MAPPED_SUBRESOURCE, width: u32, height: u32) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        let pitch = mapped.RowPitch as usize;
        if mapped.pData.is_null() || pitch == 0 {
            return (None, None);
        }
        let uv_rows = (height / 2) as usize;
        let total = pitch * (height as usize + uv_rows);
        // SAFETY: pData points at pitch times all rows of the mapped NV12 texture.
        let data = unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, total) };
        let row_bytes = width as usize;
        let mut luma = Vec::with_capacity(row_bytes * height as usize);
        for row in 0..height as usize {
            luma.extend_from_slice(&data[row * pitch..row * pitch + row_bytes]);
        }
        let mut chroma = Vec::with_capacity(row_bytes * uv_rows);
        for row in 0..uv_rows {
            let start = (height as usize + row) * pitch;
            chroma.extend_from_slice(&data[start..start + row_bytes]);
        }
        (Some(luma), Some(chroma))
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
    println!("nv12_probe is only available on Windows");
}
