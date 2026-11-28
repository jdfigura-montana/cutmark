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
    let settings = framed_by(style);
    renderer.render(&source, &input(view, &settings), &target.create_view(&Default::default())).unwrap();
    Some(renderer.read_back(&target).unwrap())
}

/// The default settings with this frame.
fn framed_by(frame: &FrameStyle) -> RenderSettings {
    RenderSettings { frame: frame.clone(), ..RenderSettings::default() }
}

/// A frame at time 0 with nothing but the picture.
fn input(view_rect: ViewRect, settings: &RenderSettings) -> StudioFrameInput<'_> {
    StudioFrameInput { t_us: 0, view_rect, cursor: None, clicks: &[], chord: None, settings }
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
    let settings = framed_by(&framed(Background::Image(image), 0.0, Shadow::Off));
    renderer.render(&source, &input(ViewRect::full(160, 90), &settings), &target.create_view(&Default::default())).unwrap();
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
    let settings = RenderSettings::default();
    assert!(matches!(renderer.render(&source, &input(empty, &settings), &view), Err(RenderError::View(_))));
    // The target has no TEXTURE_BINDING usage, and the source has no COPY_SRC usage.
    assert!(matches!(renderer.render(&target, &input(ViewRect::full(16, 16), &settings), &view), Err(RenderError::Texture { .. })));
    assert!(matches!(renderer.read_back(&source), Err(RenderError::NotReadable)));
    let mut short_shape = two_tone_shape(1);
    short_shape.frames[0].bgra.pop();
    assert!(matches!(renderer.set_cursor_shapes([&short_shape]), Err(RenderError::Shape { id: 1 })));
}

const GREY: [u8; 3] = [90, 90, 90];
const GREEN: [u8; 3] = [0, 255, 0];
const PURE_RED: [u8; 3] = [255, 0, 0];

/// A solid, opaque BGRA picture of 4 x 4 pixels: red on the left, green on the right.
fn two_tone_shape(id: u32) -> CursorShape {
    let bgra = (0..16).flat_map(|at| if at % 4 < 2 { [0, 0, 255, 255] } else { [0, 255, 0, 255] }).collect();
    CursorShape { id, width: 4, height: 4, hotspot_x: 1, hotspot_y: 1, frames: vec![ShapeFrame { duration_us: 0, bgra }] }
}

/// A renderer over a grey source, with the output at the source's size.
struct Stage {
    renderer: StudioRenderer,
    source: wgpu::Texture,
    target: wgpu::Texture,
    size: (u32, u32),
}

impl Stage {
    fn new(size: (u32, u32)) -> Option<Self> {
        Self::in_format(size, wgpu::TextureFormat::Rgba8Unorm)
    }

    fn in_format(size: (u32, u32), format: wgpu::TextureFormat) -> Option<Self> {
        let (device, queue) = gpu()?;
        let mut renderer = StudioRenderer::new(device.clone(), queue, size, format).unwrap();
        renderer.set_cursor_shapes([&two_tone_shape(7)]).unwrap();
        let source = renderer.upload_rgba(&frame_of(size.0, size.1, |_, _| GREY)).unwrap();
        let target = target(&device, size, format);
        Some(Self { renderer, source, target, size })
    }

    fn full(&self) -> ViewRect {
        ViewRect::full(self.size.0, self.size.1)
    }

    fn render(&mut self, input: &StudioFrameInput<'_>) -> RgbaFrame {
        self.renderer.render(&self.source, input, &self.target.create_view(&Default::default())).unwrap();
        self.renderer.read_back(&self.target).unwrap()
    }
}

/// The test cursor with its hotspot at (100, 50) of the source.
fn cursor(idle_us: i64) -> CursorDraw {
    CursorDraw { x: 100.0, y: 50.0, shape: 7, scale: 1.0, idle_us }
}

fn real_size() -> RenderSettings {
    RenderSettings { cursor: CursorLook { size: 1.0, ..CursorLook::default() }, ..RenderSettings::default() }
}

#[test]
fn the_cursor_lands_on_its_pixel_at_1x_and_2x() {
    let Some(mut stage) = Stage::new((320, 180)) else { return };
    let settings = real_size();
    // At 1x the hotspot (1, 1) puts the picture's top left corner on (99, 49), copied exactly.
    let out = stage.render(&StudioFrameInput { cursor: Some(cursor(0)), ..input(stage.full(), &settings) });
    for (x, y, color) in [(99, 49, PURE_RED), (100, 52, PURE_RED), (101, 49, GREEN), (102, 52, GREEN)] {
        assert_color(&out, x, y, color);
    }
    for (x, y) in [(98, 49), (103, 49), (99, 48), (102, 53)] {
        assert_color(&out, x, y, GREY);
    }
    // At 2x on the view from (50, 25), source (100, 50) is output (100, 50), and the picture is
    // 8 px wide from (98, 48). Its outermost pixels are blended by the enlarging.
    let half = ViewRect { x: 50.0, y: 25.0, width: 160.0, height: 90.0 };
    let out = stage.render(&StudioFrameInput { cursor: Some(cursor(0)), ..input(half, &settings) });
    for (x, y, color) in [(99, 49, PURE_RED), (100, 54, PURE_RED), (103, 49, GREEN), (104, 54, GREEN)] {
        assert_color(&out, x, y, color);
    }
    for (x, y) in [(96, 49), (107, 49), (99, 46), (104, 57)] {
        assert_color(&out, x, y, GREY);
    }
}

#[test]
fn the_cursor_grows_with_size_and_with_its_own_scale() {
    let Some(mut stage) = Stage::new((320, 180)) else { return };
    let mut settings = real_size();
    settings.cursor.size = 2.0;
    // Size 2 on a video at half the desktop's size is real size again.
    let draw = CursorDraw { scale: 0.5, ..cursor(0) };
    let out = stage.render(&StudioFrameInput { cursor: Some(draw), ..input(stage.full(), &settings) });
    assert_color(&out, 99, 49, PURE_RED);
    assert_color(&out, 102, 52, GREEN);
    assert_color(&out, 103, 52, GREY);
}

#[test]
fn the_cursor_is_hidden_after_the_idle_time_with_a_fade() {
    let Some(mut stage) = Stage::new((320, 180)) else { return };
    let mut settings = real_size();
    settings.cursor.hide_idle_s = Some(3.0);
    let red_at_hotspot = |stage: &mut Stage, settings: &RenderSettings, idle_us| {
        let full = stage.full();
        let out = stage.render(&StudioFrameInput { cursor: Some(cursor(idle_us)), ..input(full, settings) });
        pixel(&out, 99, 49)[0]
    };
    assert_eq!(red_at_hotspot(&mut stage, &settings, 3_000_000), 255);
    let fading = red_at_hotspot(&mut stage, &settings, 3_000_000 + IDLE_FADE_US / 2);
    assert!((150..200).contains(&fading), "{fading}");
    assert_eq!(red_at_hotspot(&mut stage, &settings, 3_000_000 + IDLE_FADE_US), GREY[0]);
    assert_eq!(red_at_hotspot(&mut stage, &settings, 60_000_000), GREY[0]);
    settings.cursor.hide_idle_s = None;
    assert_eq!(red_at_hotspot(&mut stage, &settings, 60_000_000), 255);
    settings.cursor.visible = false;
    assert_eq!(red_at_hotspot(&mut stage, &settings, 0), GREY[0]);
}

#[test]
fn an_animated_cursor_follows_the_frame_time_and_a_full_atlas_starts_over() {
    let Some(mut stage) = Stage::new((320, 180)) else { return };
    // 70 frames of the largest size are more than the atlas holds. Frame n is blue n.
    let frames = (0..70u8).map(|n| ShapeFrame { duration_us: 1_000, bgra: [n, 0, 0, 255].repeat(256 * 256) }).collect();
    let shape = CursorShape { id: 1, width: 256, height: 256, hotspot_x: 0, hotspot_y: 0, frames };
    stage.renderer.set_cursor_shapes([&shape]).unwrap();
    let settings = real_size();
    let draw = CursorDraw { x: 0.0, y: 0.0, shape: 1, ..cursor(0) };
    for n in (0..70u8).chain([0, 69]) {
        let full = stage.full();
        let out = stage.render(&StudioFrameInput { t_us: i64::from(n) * 1_000, cursor: Some(draw), ..input(full, &settings) });
        assert_color(&out, 100, 100, [0, 0, n]);
    }
}

#[test]
fn the_click_ring_shows_at_the_click_and_is_gone_after_its_time() {
    // At 540 lines the lengths are half of those at 1080p: a new ring has a radius of 6.
    let Some(mut stage) = Stage::new((960, 540)) else { return };
    let mut settings = RenderSettings::default();
    settings.cursor.click_color = GREEN;
    let clicks = [ClickDraw { x: 480.0, y: 270.0, time_us: 1_000_000 }];
    let mut at = |t_us, view| stage.render(&StudioFrameInput { t_us, clicks: &clicks, ..input(view, &settings) });
    let full = ViewRect::full(960, 540);
    let plain = at(0, full);
    let first = at(1_000_000, full);
    assert!(pixel(&first, 486, 270)[1] > 200, "{:?}", pixel(&first, 486, 270));
    assert_color(&first, 480, 270, GREY);
    // Zoomed 2x on the click, the ring is twice as wide.
    let zoomed = at(1_000_000, ViewRect { x: 240.0, y: 135.0, width: 480.0, height: 270.0 });
    assert!(pixel(&zoomed, 492, 270)[1] > 200, "{:?}", pixel(&zoomed, 492, 270));
    assert_color(&zoomed, 486, 270, GREY);
    // It grows and fades.
    let late = at(1_000_000 + 3 * CLICK_EFFECT_US / 4, full);
    assert_color(&late, 486, 270, GREY);
    let strongest = (488..500).map(|x| pixel(&late, x, 270)[1]).max().unwrap();
    assert!(strongest > GREY[1] + 10 && strongest < 150, "{strongest}");
    for t_us in [999_999, 1_000_000 + CLICK_EFFECT_US, 2_000_000] {
        assert_eq!(at(t_us, full).data, plain.data, "at {t_us}");
    }
}

#[test]
fn the_ripple_fills_its_middle_and_none_draws_nothing() {
    let Some(mut stage) = Stage::new((960, 540)) else { return };
    let mut settings = RenderSettings::default();
    settings.cursor.click_color = GREEN;
    let clicks = [ClickDraw { x: 480.0, y: 270.0, time_us: 0 }];
    let full = stage.full();
    settings.cursor.click_effect = ClickEffect::Ripple;
    let out = stage.render(&StudioFrameInput { t_us: 100_000, clicks: &clicks, ..input(full, &settings) });
    let middle = pixel(&out, 480, 270)[1];
    assert!(middle > GREY[1] + 20 && middle < 200, "{middle}");
    settings.cursor.click_effect = ClickEffect::None;
    let out = stage.render(&StudioFrameInput { t_us: 100_000, clicks: &clicks, ..input(full, &settings) });
    assert!(out.data.as_chunks::<4>().0.iter().all(|pixel| pixel[..3] == GREY));
}

#[test]
fn the_spotlight_darkens_all_but_a_circle_around_the_cursor() {
    let Some(mut stage) = Stage::new((960, 540)) else { return };
    let mut settings = real_size();
    settings.cursor.visible = false;
    // 160 px at 1080p is 80 px here.
    settings.cursor.highlight = Highlight::Spotlight { radius: 160.0 };
    let draw = CursorDraw { x: 480.0, y: 270.0, ..cursor(0) };
    let full = stage.full();
    let out = stage.render(&StudioFrameInput { cursor: Some(draw), ..input(full, &settings) });
    assert_color(&out, 480, 270, GREY);
    assert_color(&out, 480 + 60, 270, GREY);
    // 45% of 90 is left outside.
    assert_color(&out, 480 + 100, 270, [40; 3]);
    assert_color(&out, 0, 0, [40; 3]);
    let edge = pixel(&out, 480 + 80, 270)[0];
    assert!((55..75).contains(&edge), "{edge}");
}

/// The box around every pixel that differs between two frames: left, top, then right and bottom
/// just past it.
fn changed_box(before: &RgbaFrame, after: &RgbaFrame) -> Option<(u32, u32, u32, u32)> {
    let changed = (0..before.height).flat_map(|y| (0..before.width).map(move |x| (x, y))).filter(|&(x, y)| pixel(before, x, y) != pixel(after, x, y));
    changed.fold(None, |found, (x, y)| {
        let (left, top, right, bottom) = found.unwrap_or((x, y, x + 1, y + 1));
        Some((left.min(x), top.min(y), right.max(x + 1), bottom.max(y + 1)))
    })
}

#[test]
fn the_key_caps_sit_at_the_chosen_position_and_do_not_zoom() {
    // At 540 lines a medium cap is 26 px tall and the margin is 28 px.
    let Some(mut stage) = Stage::new((960, 540)) else { return };
    let mut settings = RenderSettings::default();
    let full = stage.full();
    let plain = stage.render(&input(full, &settings));
    for position in [KeysPosition::BottomCenter, KeysPosition::BottomLeft, KeysPosition::BottomRight, KeysPosition::TopCenter] {
        settings.keys.position = position;
        let out = stage.render(&StudioFrameInput { chord: Some("Ctrl+Shift+S"), ..input(full, &settings) });
        let (left, top, right, bottom) = changed_box(&plain, &out).expect("an overlay");
        assert_eq!(bottom - top, 26, "{position:?}");
        assert!((100..300).contains(&(right - left)), "{position:?}: {}", right - left);
        match position {
            KeysPosition::TopCenter => assert_eq!(top, 28),
            _ => assert_eq!(bottom, 540 - 28, "{position:?}"),
        }
        match position {
            KeysPosition::BottomLeft => assert_eq!(left, 28),
            KeysPosition::BottomRight => assert_eq!(right, 960 - 28),
            _ => assert!((left + right).abs_diff(960) <= 1, "{position:?}: {left} to {right}"),
        }
        // Dark glass with light text on it.
        let inside = || (top..bottom).flat_map(|y| (left..right).map(move |x| (x, y))).map(|(x, y)| pixel(&out, x, y)[0]);
        assert!(inside().any(|red| red > 200), "{position:?}: no text");
        assert!(inside().filter(|&red| red < 50).count() > (right - left) as usize * 8, "{position:?}: no cap");
        // The same frame again comes from the kept picture, and zooming moves nothing.
        let half = ViewRect { x: 240.0, y: 135.0, width: 480.0, height: 270.0 };
        let zoomed = stage.render(&StudioFrameInput { chord: Some("Ctrl+Shift+S"), ..input(half, &settings) });
        assert_eq!(zoomed.data, out.data, "{position:?}");
    }
    settings.keys.show = false;
    let out = stage.render(&StudioFrameInput { chord: Some("Ctrl+Shift+S"), ..input(full, &settings) });
    assert_eq!(out.data, plain.data);
}

#[test]
fn key_caps_go_over_the_frame_and_follow_size_and_chord() {
    let Some(mut stage) = Stage::new((960, 540)) else { return };
    let mut settings = framed_by(&framed(Background::Solid(RED), 12.0, Shadow::Off));
    let full = stage.full();
    let plain = stage.render(&input(full, &settings));
    let mut shown = |settings: &RenderSettings, chord: &str| {
        let out = stage.render(&StudioFrameInput { chord: Some(chord), ..input(full, settings) });
        changed_box(&plain, &out).expect("an overlay")
    };
    // 10% padding is 54 px, so the caps reach from the picture into the background.
    let medium = shown(&settings, "Ctrl+S");
    assert_eq!(medium.3, 540 - 28);
    let mut heights = Vec::new();
    for size in [KeysSize::Small, KeysSize::Medium, KeysSize::Large] {
        settings.keys.size = size;
        let (_, top, _, bottom) = shown(&settings, "Ctrl+S");
        heights.push(bottom - top);
    }
    assert_eq!(heights, vec![20, 26, 34]);
    // A new chord replaces the kept picture, and one too wide for the output is made smaller.
    let (left, top, right, bottom) = shown(&settings, &["Shift"; 20].join("+"));
    assert!(left >= 27 && right <= 933 && bottom - top < 34, "{left}, {top}, {right}, {bottom}");
}

#[test]
fn key_caps_have_the_same_colors_on_an_srgb_target() {
    let render = |format| {
        let mut stage = Stage::in_format((960, 540), format)?;
        let settings = RenderSettings::default();
        let full = stage.full();
        Some(stage.render(&StudioFrameInput { chord: Some("Alt+Tab"), ..input(full, &settings) }))
    };
    let (Some(plain), Some(srgb)) = (render(wgpu::TextureFormat::Rgba8Unorm), render(wgpu::TextureFormat::Rgba8UnormSrgb)) else { return };
    let worst = plain.data.iter().zip(&srgb.data).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(worst <= 2, "{worst}");
}

#[test]
fn chords_split_into_keys() {
    assert_eq!(keys::split_chord("Ctrl+Shift+S"), vec!["Ctrl", "Shift", "S"]);
    assert_eq!(keys::split_chord("F5"), vec!["F5"]);
    assert_eq!(keys::split_chord("Ctrl++"), vec!["Ctrl", "+"]);
    assert_eq!(keys::split_chord("+"), vec!["+"]);
    assert!(keys::split_chord("").is_empty());
}
