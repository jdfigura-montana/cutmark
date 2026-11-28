//! Render tests on a headless device. Without a GPU adapter they say so and pass.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use super::*;

const RED: [u8; 3] = [200, 30, 40];
const BLUE: [u8; 3] = [20, 40, 220];

/// Waits for one of wgpu's futures. On a native backend they are ready almost at once.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        std::thread::yield_now();
    }
}

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) {
        Ok(adapter) => adapter,
        Err(error) => {
            eprintln!("skipped: no GPU adapter ({error})");
            return None;
        }
    };
    Some(block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("device"))
}

fn frame_of(width: u32, height: u32, color: impl Fn(u32, u32) -> [u8; 3]) -> RgbaFrame {
    let mut data = Vec::new();
    for y in 0..height {
        for x in 0..width {
            data.extend_from_slice(&color(x, y));
            data.push(255);
        }
    }
    RgbaFrame { width, height, pts_ms: 0, data }
}

fn pixel(frame: &RgbaFrame, x: u32, y: u32) -> [u8; 3] {
    let at = (y * frame.width + x) as usize * 4;
    [frame.data[at], frame.data[at + 1], frame.data[at + 2]]
}

/// Equal but for the rounding of the GPU.
fn assert_color(frame: &RgbaFrame, x: u32, y: u32, expected: [u8; 3]) {
    let actual = pixel(frame, x, y);
    let close = actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 2);
    assert!(close, "pixel ({x}, {y}) is {actual:?}, expected {expected:?}");
}

fn target(device: &wgpu::Device, size: (u32, u32), format: wgpu::TextureFormat) -> wgpu::Texture {
    let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
    texture(device, "test target", size, format, usage)
}

/// Renders the whole of `view` from `source` and reads the result back.
fn render_in(format: wgpu::TextureFormat, output: (u32, u32), source: &RgbaFrame, view: ViewRect, style: &FrameStyle) -> Option<RgbaFrame> {
    let (device, queue) = gpu()?;
    let mut renderer = StudioRenderer::new(device.clone(), queue, output, format).unwrap();
    let source = renderer.upload_rgba(source).unwrap();
    let target = target(&device, output, format);
    renderer.render(&source, view, style, &target.create_view(&Default::default())).unwrap();
    Some(renderer.read_back(&target).unwrap())
}

fn render(output: (u32, u32), source: &RgbaFrame, view: ViewRect, style: &FrameStyle) -> Option<RgbaFrame> {
    render_in(wgpu::TextureFormat::Rgba8Unorm, output, source, view, style)
}

fn framed(background: Background, corner_radius: f32, shadow: Shadow) -> FrameStyle {
    FrameStyle { enabled: true, background, padding: 0.10, corner_radius, shadow }
}

#[test]
fn solid_background_fills_the_corner() {
    let source = frame_of(160, 90, |_, _| BLUE);
    let style = framed(Background::Solid(RED), 12.0, Shadow::Off);
    let Some(out) = render((320, 180), &source, ViewRect::full(160, 90), &style) else { return };
    assert_eq!((out.width, out.height, out.data.len()), (320, 180, 320 * 180 * 4));
    for (x, y) in [(0, 0), (319, 0), (0, 179), (319, 179)] {
        assert_color(&out, x, y, RED);
    }
}

#[test]
fn content_sits_inside_the_padding() {
    let source = frame_of(160, 90, |_, _| BLUE);
    let style = framed(Background::Solid(RED), 0.0, Shadow::Off);
    let view = ViewRect::full(160, 90);
    // 10% of 180 is 18 px above and below; 16:9 inside that is 256 x 144.
    let rect = content_rect((320, 180), view, &style);
    assert_eq!(rect, ContentRect { x: 32, y: 18, width: 256, height: 144 });
    let Some(out) = render((320, 180), &source, view, &style) else { return };
    assert_color(&out, 160, 90, BLUE);
    assert_color(&out, rect.x, rect.y, BLUE);
    assert_color(&out, rect.x + rect.width - 1, rect.y + rect.height - 1, BLUE);
    assert_color(&out, rect.x - 1, 90, RED);
    assert_color(&out, rect.x + rect.width, 90, RED);
    assert_color(&out, 160, rect.y - 1, RED);
    assert_color(&out, 160, rect.y + rect.height, RED);
}

#[test]
fn zoom_2x_doubles_the_checkerboard_cells() {
    let source = frame_of(64, 64, |x, y| if (x / 8 + y / 8) % 2 == 0 { [0; 3] } else { [255; 3] });
    // The frame is off, so the picture fills the target.
    let style = FrameStyle::default();
    let Some(plain) = render((64, 64), &source, ViewRect::full(64, 64), &style) else { return };
    let half = ViewRect { x: 0.0, y: 0.0, width: 32.0, height: 32.0 };
    let Some(zoomed) = render((64, 64), &source, half, &style) else { return };
    // Where the top row turns from dark to light or back.
    let edges = |frame: &RgbaFrame| -> Vec<u32> { (1..64).filter(|&x| (pixel(frame, x, 0)[0] > 127) != (pixel(frame, x - 1, 0)[0] > 127)).collect() };
    assert_eq!(edges(&plain), vec![8, 16, 24, 32, 40, 48, 56]);
    assert_eq!(edges(&zoomed), vec![16, 32, 48]);
    assert_eq!(plain.data, source.data);
}

#[test]
fn corner_radius_shows_the_background_in_the_corner() {
    let source = frame_of(1920, 1080, |_, _| BLUE);
    let view = ViewRect::full(1920, 1080);
    let square = framed(Background::Solid(RED), 0.0, Shadow::Off);
    let round = framed(Background::Solid(RED), 32.0, Shadow::Off);
    let rect = content_rect((1920, 1080), view, &round);
    let Some(square) = render((1920, 1080), &source, view, &square) else { return };
    let Some(round) = render((1920, 1080), &source, view, &round) else { return };
    let right = rect.x + rect.width - 1;
    let bottom = rect.y + rect.height - 1;
    for (x, y) in [(rect.x, rect.y), (right, rect.y), (rect.x, bottom), (right, bottom)] {
        assert_color(&square, x, y, BLUE);
        assert_color(&round, x, y, RED);
    }
    // Past the radius along an edge, the picture reaches the edge again.
    assert_color(&round, rect.x + 40, rect.y, BLUE);
}

#[test]
fn shadow_darkens_below_the_picture_and_strong_more_than_soft() {
    let source = frame_of(160, 90, |_, _| BLUE);
    let view = ViewRect::full(160, 90);
    let below = |shadow| {
        let style = framed(Background::Solid(RED), 12.0, shadow);
        let rect = content_rect((320, 180), view, &style);
        render((320, 180), &source, view, &style).map(|out| {
            assert_color(&out, 160, 90, BLUE);
            pixel(&out, 160, rect.y + rect.height + 1)[0]
        })
    };
    let (Some(off), Some(soft), Some(strong)) = (below(Shadow::Off), below(Shadow::Soft), below(Shadow::Strong)) else {
        return;
    };
    assert!(off > soft && soft > strong, "red below the picture: off {off}, soft {soft}, strong {strong}");
}

#[test]
fn gradient_runs_from_the_top_left_to_the_bottom_right() {
    let source = frame_of(160, 90, |_, _| BLUE);
    let style = framed(Background::Gradient(3), 0.0, Shadow::Off);
    let Some(out) = render((320, 180), &source, ViewRect::full(160, 90), &style) else { return };
    let [first, last] = GRADIENT_PRESETS[2];
    assert_color(&out, 0, 0, first);
    assert_color(&out, 319, 179, last);
}

#[test]
fn image_background_covers_the_output_without_distortion() {
    let Some((device, queue)) = gpu() else { return };
    let mut renderer = StudioRenderer::new(device.clone(), queue, (320, 180), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    // A square picture on a 16:9 output keeps its middle rows: the red top and bottom are cut off.
    let image = frame_of(100, 100, |_, y| if (20..80).contains(&y) { [0, 255, 0] } else { RED });
    let image = renderer.upload_rgba(&image).unwrap();
    let source = renderer.upload_rgba(&frame_of(160, 90, |_, _| BLUE)).unwrap();
    let target = target(&device, (320, 180), wgpu::TextureFormat::Rgba8Unorm);
    let style = framed(Background::Image(image), 0.0, Shadow::Off);
    renderer.render(&source, ViewRect::full(160, 90), &style, &target.create_view(&Default::default())).unwrap();
    let out = renderer.read_back(&target).unwrap();
    assert_color(&out, 0, 0, [0, 255, 0]);
    assert_color(&out, 319, 179, [0, 255, 0]);
    assert_color(&out, 160, 90, BLUE);
}

#[test]
fn other_target_formats_give_the_same_colors() {
    use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba8UnormSrgb};
    let color = [10, 128, 250];
    // An odd width, so the rows read back have padding to drop.
    let source = frame_of(101, 57, |_, _| color);
    for format in [Rgba8UnormSrgb, Bgra8Unorm, Bgra8UnormSrgb] {
        let Some(out) = render_in(format, (101, 57), &source, ViewRect::full(101, 57), &FrameStyle::default()) else {
            return;
        };
        assert_eq!(out.data.len(), 101 * 57 * 4);
        assert_color(&out, 0, 0, color);
        assert_color(&out, 100, 56, color);
    }
}

#[test]
fn bad_inputs_are_errors() {
    let Some((device, queue)) = gpu() else { return };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    assert!(matches!(StudioRenderer::new(device.clone(), queue.clone(), (0, 90), format), Err(RenderError::Size { .. })));
    assert!(matches!(StudioRenderer::new(device.clone(), queue.clone(), (16, 16), wgpu::TextureFormat::R8Unorm), Err(RenderError::Format(_))));
    let mut renderer = StudioRenderer::new(device.clone(), queue, (16, 16), format).unwrap();
    let short = RgbaFrame { width: 4, height: 4, pts_ms: 0, data: vec![0; 60] };
    assert!(matches!(renderer.upload_rgba(&short), Err(RenderError::FrameData { .. })));
    let source = renderer.upload_rgba(&frame_of(4, 4, |_, _| RED)).unwrap();
    let target = target(&device, (16, 16), format);
    let view = target.create_view(&Default::default());
    let empty = ViewRect { x: 0.0, y: 0.0, width: 0.0, height: 4.0 };
    assert!(matches!(renderer.render(&source, empty, &FrameStyle::default(), &view), Err(RenderError::View(_))));
    // The target has no TEXTURE_BINDING usage, and the source has no COPY_SRC usage.
    assert!(matches!(renderer.render(&target, ViewRect::full(16, 16), &FrameStyle::default(), &view), Err(RenderError::Texture { .. })));
    assert!(matches!(renderer.read_back(&source), Err(RenderError::NotReadable)));
}
