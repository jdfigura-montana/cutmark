//! Copies a texture into system memory.

use std::sync::mpsc;

use media_io::player::video::RgbaFrame;

use crate::RenderError;

/// The formats the renderer writes and reads back: eight bits per channel, RGBA or BGRA.
pub(crate) fn is_supported(format: wgpu::TextureFormat) -> bool {
    use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba8Unorm, Rgba8UnormSrgb};
    matches!(format, Rgba8Unorm | Rgba8UnormSrgb | Bgra8Unorm | Bgra8UnormSrgb)
}

pub(crate) fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Result<RgbaFrame, RenderError> {
    let format = texture.format();
    if !is_supported(format) {
        return Err(RenderError::Format(format));
    }
    if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) {
        return Err(RenderError::NotReadable);
    }
    let (width, height) = (texture.width(), texture.height());
    // A copy into a buffer needs rows of a multiple of 256 bytes.
    let row = width as usize * 4;
    let padded_row = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("studio read back"),
        size: padded_row as u64 * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("studio read back") });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded_row as u32), rows_per_image: None },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);

    let (sender, receiver) = mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
        // The receiver is gone only if this function already returned with an error.
        let _ = sender.send(result);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).map_err(failed)?;
    receiver.recv().map_err(failed)?.map_err(failed)?;

    let mut data = Vec::with_capacity(row * height as usize);
    {
        let mapped = buffer.get_mapped_range(..).map_err(failed)?;
        for padded in mapped.chunks_exact(padded_row) {
            data.extend_from_slice(&padded[..row]);
        }
    }
    buffer.unmap();
    if matches!(format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) {
        for pixel in data.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
    }
    Ok(RgbaFrame { width, height, pts_ms: 0, data })
}

fn failed(error: impl std::fmt::Display) -> RenderError {
    RenderError::ReadBack(error.to_string())
}
