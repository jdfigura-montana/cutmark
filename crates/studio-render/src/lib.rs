//! Renders a Studio frame on the GPU. The preview and the export use the same code, so they match.
//!
//! See the Studio spec, "Rendering", "Cursor", "Keys" and "Frame". The passes, in order:
//!
//! 1. Zoom: the view rectangle of the source is scaled into a texture of the size the picture has
//!    in the output.
//! 2. Cursor, click effects and spotlight are drawn on that texture, so they zoom with it.
//! 3. Frame: the background, the shadow and the picture with rounded corners go onto the target.
//!    With the frame off, the picture covers the whole target.
//! 4. Shortcut overlay: the key caps go over the framed output and do not zoom. They are drawn
//!    into a picture of their own, which the frame pass's shader lays on last.
//!
//! Every colour is handled as its sRGB value: source and background textures must not be of an
//! sRGB format, so that the GPU hands over the stored values. The target may be of either kind.

mod atlas;
mod effects;
mod input;
mod keys;
mod layout;
mod readback;
mod sprites;
mod style;

#[cfg(test)]
mod tests;

pub use effects::{CLICK_EFFECT_US, IDLE_FADE_US};
pub use events::{CursorShape, ShapeFrame};
pub use input::{ClickDraw, ClickEffect, CursorDraw, CursorLook, Highlight, KeysLook, KeysPosition, KeysSize, RenderSettings, StudioFrameInput};
pub use layout::{ContentRect, ViewRect, content_rect};
pub use media_io::player::video::RgbaFrame;
pub use style::{Background, FrameStyle, GRADIENT_PRESETS, MAX_CORNER_RADIUS, MAX_PADDING, MIN_PADDING, Shadow};

/// The format of the textures made by [`StudioRenderer::upload_rgba`] and of the picture between
/// the two passes.
const PICTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The size of the frame pass's uniform: seven vectors of four floats.
const FRAME_UNIFORM_FLOATS: usize = 28;

/// Why rendering, uploading or reading back failed.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// A width or height of zero, or one the GPU cannot make a texture of.
    #[error("{what} size {width} x {height} is not usable (the largest side is {max})")]
    Size { what: &'static str, width: u32, height: u32, max: u32 },
    /// The renderer cannot write or read this format.
    #[error("texture format {0:?} is not supported")]
    Format(wgpu::TextureFormat),
    /// A source or background texture the shaders cannot read as sRGB values.
    #[error("the {what} texture must be a 2D, single sample, non-sRGB colour texture with TEXTURE_BINDING usage")]
    Texture { what: &'static str },
    /// A texture to be read back without the usage for it.
    #[error("the texture to read back needs COPY_SRC usage")]
    NotReadable,
    /// A frame whose bytes do not fit its size.
    #[error("a {width} x {height} RGBA frame needs {expected} bytes, this one has {actual}")]
    FrameData { width: u32, height: u32, expected: u64, actual: usize },
    /// A view rectangle with no area, or with a value that is not a number.
    #[error("the view rectangle {0:?} is not usable")]
    View(ViewRect),
    /// A cursor shape whose size or bytes are wrong.
    #[error("cursor shape {id} is not usable")]
    Shape { id: u32 },
    /// The font of the shortcut overlay could not be read.
    #[error("the overlay font could not be read: {0}")]
    Font(String),
    /// The GPU did not finish or did not hand the pixels over.
    #[error("reading the frame back from the GPU failed: {0}")]
    ReadBack(String),
}

/// The picture between the two passes, kept while its size stays the same.
struct Picture {
    size: (u32, u32),
    view: wgpu::TextureView,
}

/// Renders Studio frames of one size into textures of one format.
pub struct StudioRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    output_size: (u32, u32),
    format: wgpu::TextureFormat,
    sampler: wgpu::Sampler,
    zoom_pipeline: wgpu::RenderPipeline,
    zoom_uniform: wgpu::Buffer,
    frame_pipeline: wgpu::RenderPipeline,
    frame_uniform: wgpu::Buffer,
    /// Bound in place of the background picture or the key caps when there is none.
    no_texture: wgpu::TextureView,
    picture: Option<Picture>,
    sprite_pipeline: sprites::SpritePipeline,
    cursors: effects::Cursors,
    keys: keys::Keys,
}

impl StudioRenderer {
    /// A renderer for targets of `output_size` pixels and of `format`, which is RGBA8 or BGRA8,
    /// with or without sRGB. `device` and `queue` are handles: pass clones to keep your own.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, output_size: (u32, u32), format: wgpu::TextureFormat) -> Result<Self, RenderError> {
        check_size(&device, "output", output_size)?;
        if !readback::is_supported(format) {
            return Err(RenderError::Format(format));
        }
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("studio linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let zoom_pipeline = pipeline(&device, "studio zoom", wgpu::include_wgsl!("zoom.wgsl"), PICTURE_FORMAT);
        let frame_pipeline = pipeline(&device, "studio frame", wgpu::include_wgsl!("frame.wgsl"), format);
        let zoom_uniform = uniform(&device, "studio zoom uniform", 4);
        let frame_uniform = uniform(&device, "studio frame uniform", FRAME_UNIFORM_FLOATS);
        let no_texture = texture(&device, "studio no texture", (1, 1), PICTURE_FORMAT, wgpu::TextureUsages::TEXTURE_BINDING)
            .create_view(&wgpu::TextureViewDescriptor::default());
        // The picture and the key caps are both of the picture format.
        let sprite_pipeline = sprites::SpritePipeline::new(&device, PICTURE_FORMAT);
        let cursors = effects::Cursors::new(&device);
        let keys = keys::Keys::new(&device)?;
        Ok(Self {
            device,
            queue,
            output_size,
            format,
            sampler,
            zoom_pipeline,
            zoom_uniform,
            frame_pipeline,
            frame_uniform,
            no_texture,
            picture: None,
            sprite_pipeline,
            cursors,
            keys,
        })
    }

    /// Takes the cursor shapes of a take, as `events::EventsReader::shapes` gives them. They
    /// replace those of the take before. [`CursorDraw::shape`] names one by its id.
    pub fn set_cursor_shapes<'a>(&mut self, shapes: impl IntoIterator<Item = &'a CursorShape>) -> Result<(), RenderError> {
        self.cursors.set_shapes(shapes)
    }

    /// The size of the targets this renderer draws.
    pub fn output_size(&self) -> (u32, u32) {
        self.output_size
    }

    /// The format of the targets this renderer draws.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Draws one frame and submits it to the queue. `source` is the video frame at the input's
    /// time. `target` must be of the renderer's size and format, with RENDER_ATTACHMENT usage.
    pub fn render(&mut self, source: &wgpu::Texture, input: &StudioFrameInput<'_>, target: &wgpu::TextureView) -> Result<(), RenderError> {
        let (view_rect, frame_style) = (input.view_rect, &input.settings.frame);
        check_readable_by_shader(source, "source")?;
        if let Background::Image(image) = &frame_style.background
            && frame_style.enabled
        {
            check_readable_by_shader(image, "background")?;
        }
        if !view_rect.is_usable() {
            return Err(RenderError::View(view_rect));
        }
        let content = content_rect(self.output_size, view_rect, frame_style);
        let picture_view = self.picture((content.width, content.height));

        let (source_w, source_h) = (source.width() as f32, source.height() as f32);
        write(&self.queue, &self.zoom_uniform, &[view_rect.x / source_w, view_rect.y / source_h, view_rect.width / source_w, view_rect.height / source_h]);
        let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let zoom_bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("studio zoom bindings"),
            layout: &self.zoom_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.zoom_uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&source_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });

        let picture_size = (content.width, content.height);
        let on_picture = effects::sprites(&mut self.cursors, &self.queue, input, (source.width(), source.height()), picture_size);
        let atlas = (&self.cursors.atlas.view, self.cursors.atlas.side);
        let effects_batch = self.sprite_pipeline.batch(&self.device, &self.queue, picture_size, atlas, &self.sampler, &on_picture);

        let keys = &input.settings.keys;
        let overlay = match input.chord.filter(|_| keys.show) {
            Some(chord) => self.keys.overlay(&self.device, &self.queue, self.output_size, chord, keys.size),
            None => None,
        };
        let overlay_place = overlay.as_ref().map(|overlay| keys::place(self.output_size, overlay.size, keys.position));
        let overlay_batch = overlay.as_ref().and_then(|overlay| {
            let sprites = overlay.draw.as_deref()?;
            let batch = self.sprite_pipeline.batch(&self.device, &self.queue, overlay.size, self.keys.atlas(), &self.sampler, sprites)?;
            Some((batch, &overlay.view))
        });

        let (uniform, backdrop) = self.frame_inputs(content, frame_style, overlay_place);
        write(&self.queue, &self.frame_uniform, &uniform);
        let frame_bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("studio frame bindings"),
            layout: &self.frame_pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.frame_uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&picture_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(backdrop.as_ref().unwrap_or(&self.no_texture)) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(overlay.as_ref().map_or(&self.no_texture, |overlay| &overlay.view)),
                },
            ],
        });

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("studio frame") });
        draw(&mut encoder, "studio zoom pass", &self.zoom_pipeline, &zoom_bindings, &picture_view);
        if let Some(batch) = &effects_batch {
            self.sprite_pipeline.draw(&mut encoder, "studio effects pass", batch, &picture_view, false);
        }
        if let Some((batch, view)) = &overlay_batch {
            self.sprite_pipeline.draw(&mut encoder, "studio keys pass", batch, view, true);
        }
        draw(&mut encoder, "studio frame pass", &self.frame_pipeline, &frame_bindings, target);
        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    /// Makes a source (or background) texture holding an RGBA frame.
    pub fn upload_rgba(&self, frame: &RgbaFrame) -> Result<wgpu::Texture, RenderError> {
        let size = (frame.width, frame.height);
        check_size(&self.device, "frame", size)?;
        let expected = u64::from(frame.width) * u64::from(frame.height) * 4;
        if frame.data.len() as u64 != expected {
            return Err(RenderError::FrameData { width: frame.width, height: frame.height, expected, actual: frame.data.len() });
        }
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let texture = texture(&self.device, "studio source", size, PICTURE_FORMAT, usage);
        self.queue.write_texture(
            texture.as_image_copy(),
            &frame.data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(frame.width * 4), rows_per_image: None },
            texture.size(),
        );
        Ok(texture)
    }

    /// Copies a rendered texture into system memory as RGBA, for export. This waits for the GPU.
    /// The texture needs COPY_SRC usage. The frame's time is 0: the caller knows the real one.
    pub fn read_back(&self, target: &wgpu::Texture) -> Result<RgbaFrame, RenderError> {
        readback::read_back(&self.device, &self.queue, target)
    }

    /// The texture between the two passes, made again when the size changes.
    fn picture(&mut self, size: (u32, u32)) -> wgpu::TextureView {
        if let Some(picture) = &self.picture
            && picture.size == size
        {
            return picture.view.clone();
        }
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let view = texture(&self.device, "studio picture", size, PICTURE_FORMAT, usage).create_view(&wgpu::TextureViewDescriptor::default());
        self.picture = Some(Picture { size, view: view.clone() });
        view
    }

    /// The frame pass's uniform values and, for a picture background, its view.
    fn frame_inputs(&self, content: ContentRect, style: &FrameStyle, overlay: Option<ContentRect>) -> ([f32; FRAME_UNIFORM_FLOATS], Option<wgpu::TextureView>) {
        let mut colors = [[0u8; 3]; 2];
        let mut image_uv = [0.0; 4];
        let mut backdrop = None;
        if style.enabled {
            match &style.background {
                Background::Solid(color) => colors = [*color; 2],
                Background::Gradient(preset) => {
                    colors = GRADIENT_PRESETS[usize::from((*preset).clamp(1, 6)) - 1];
                }
                Background::Image(image) => {
                    image_uv = layout::cover_uv((image.width(), image.height()), self.output_size);
                    backdrop = Some(image.create_view(&wgpu::TextureViewDescriptor::default()));
                }
            }
        }
        let shadow = if style.enabled { style.shadow } else { Shadow::Off }.shape();
        let scale = layout::scale_from_1080p(self.output_size);
        let channel = |value: u8| f32::from(value) / 255.0;
        let [a, b] = colors;
        let caps = overlay.map_or([0.0; 4], |at| [at.x as f32, at.y as f32, at.width as f32, at.height as f32]);
        let uniform = [
            content.x as f32,
            content.y as f32,
            content.width as f32,
            content.height as f32,
            channel(a[0]),
            channel(a[1]),
            channel(a[2]),
            1.0,
            channel(b[0]),
            channel(b[1]),
            channel(b[2]),
            1.0,
            image_uv[0],
            image_uv[1],
            image_uv[2],
            image_uv[3],
            self.output_size.0 as f32,
            self.output_size.1 as f32,
            layout::corner_radius_px(self.output_size, content, style),
            if backdrop.is_some() { 1.0 } else { 0.0 },
            shadow.blur * scale,
            shadow.offset * scale,
            shadow.opacity,
            if self.format.is_srgb() { 1.0 } else { 0.0 },
            caps[0],
            caps[1],
            caps[2],
            caps[3],
        ];
        (uniform, backdrop)
    }
}

fn write(queue: &wgpu::Queue, buffer: &wgpu::Buffer, values: &[f32]) {
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    queue.write_buffer(buffer, 0, &bytes);
}

fn check_size(device: &wgpu::Device, what: &'static str, (width, height): (u32, u32)) -> Result<(), RenderError> {
    let max = device.limits().max_texture_dimension_2d;
    if width == 0 || height == 0 || width > max || height > max {
        return Err(RenderError::Size { what, width, height, max });
    }
    Ok(())
}

fn check_readable_by_shader(texture: &wgpu::Texture, what: &'static str) -> Result<(), RenderError> {
    let format = texture.format();
    let usable = texture.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING)
        && texture.dimension() == wgpu::TextureDimension::D2
        && texture.depth_or_array_layers() == 1
        && texture.sample_count() == 1
        && !format.is_srgb()
        && matches!(format.sample_type(None, None), Some(wgpu::TextureSampleType::Float { filterable: true }));
    if usable { Ok(()) } else { Err(RenderError::Texture { what }) }
}

fn texture(device: &wgpu::Device, label: &str, (width, height): (u32, u32), format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn uniform(device: &wgpu::Device, label: &str, floats: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (floats * size_of::<f32>()) as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A pipeline that draws one full target triangle with the shader's `vertex` and `fragment`.
fn pipeline(device: &wgpu::Device, label: &str, shader: wgpu::ShaderModuleDescriptor<'_>, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(shader);
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vertex"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fragment"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(format.into())],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn begin_pass<'e>(encoder: &'e mut wgpu::CommandEncoder, label: &str, target: &wgpu::TextureView, load: wgpu::LoadOp<wgpu::Color>) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

fn draw(encoder: &mut wgpu::CommandEncoder, label: &str, pipeline: &wgpu::RenderPipeline, bindings: &wgpu::BindGroup, target: &wgpu::TextureView) {
    let mut pass = begin_pass(encoder, label, target, wgpu::LoadOp::Clear(wgpu::Color::BLACK));
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bindings, &[]);
    pass.draw(0..3, 0..1);
}
