//! The pipeline behind `sprite.wgsl`: what the passes after zoom and the shortcut overlay are
//! drawn with.

use crate::atlas::Slot;

/// The floats of one instance: four vectors of four.
const INSTANCE_FLOATS: usize = 16;

const PICTURE: f32 = 0.0;
const BOX: f32 = 1.0;
const SPOTLIGHT: f32 = 2.0;

/// One rectangle to draw. Colours are premultiplied sRGB values from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sprite {
    /// Left, top, width and height in target pixels.
    pub rect: [f32; 4],
    source: [f32; 4],
    pub color: [f32; 4],
    params: [f32; 4],
}

impl Sprite {
    /// A picture in an atlas, placed so that it covers `dest` (left, top, width, height).
    /// `texel` is the size of one of its pixels in the target. `tint` multiplies its colours.
    pub(crate) fn picture(slot: Slot, dest: [f32; 4], texel: (f32, f32), tint: [f32; 4]) -> Self {
        // From the middle of the border texels, which are clear, so nothing outside the slot is
        // ever sampled.
        let rect = [dest[0] - texel.0 / 2.0, dest[1] - texel.1 / 2.0, dest[2] + texel.0, dest[3] + texel.1];
        let source = [slot.x as f32 + 0.5, slot.y as f32 + 0.5, slot.width as f32 - 1.0, slot.height as f32 - 1.0];
        Self { rect, source, color: tint, params: [PICTURE, 0.0, 0.0, 0.0] }
    }

    /// A filled rounded rectangle, or with a `line` width above 0 only its edge.
    pub(crate) fn rounded(rect: [f32; 4], radius: f32, line: f32, color: [f32; 4]) -> Self {
        Self { rect, source: [0.0; 4], color, params: [BOX, radius, line, 0.0] }
    }

    /// A circle's line, with `radius` to the middle of it.
    pub(crate) fn ring((x, y): (f32, f32), radius: f32, line: f32, color: [f32; 4]) -> Self {
        let outer = radius + line / 2.0;
        Self::rounded([x - outer, y - outer, 2.0 * outer, 2.0 * outer], outer, line, color)
    }

    /// A filled circle.
    pub(crate) fn disc((x, y): (f32, f32), radius: f32, color: [f32; 4]) -> Self {
        Self::rounded([x - radius, y - radius, 2.0 * radius, 2.0 * radius], radius, 0.0, color)
    }

    /// `color` over all of a target of `size`, but for a soft circle.
    pub(crate) fn spotlight(size: (f32, f32), (x, y): (f32, f32), radius: f32, soft: f32, color: [f32; 4]) -> Self {
        Self { rect: [0.0, 0.0, size.0, size.1], source: [x, y, 0.0, 0.0], color, params: [SPOTLIGHT, radius, soft, 0.0] }
    }

    fn floats(&self) -> impl Iterator<Item = f32> {
        [self.rect, self.source, self.color, self.params].into_iter().flatten()
    }
}

/// An sRGB colour at an opacity, premultiplied.
pub(crate) fn premultiplied(color: [u8; 3], alpha: f32) -> [f32; 4] {
    let channel = |value: u8| f32::from(value) / 255.0 * alpha;
    [channel(color[0]), channel(color[1]), channel(color[2]), alpha]
}

/// A list of sprites that is ready to draw.
pub(crate) struct Batch {
    bindings: wgpu::BindGroup,
    instances: wgpu::Buffer,
    count: u32,
}

pub(crate) struct SpritePipeline {
    pipeline: wgpu::RenderPipeline,
}

impl SpritePipeline {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::include_wgsl!("sprite.wgsl"));
        let attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4];
        let layout = wgpu::VertexBufferLayout {
            array_stride: (INSTANCE_FLOATS * size_of::<f32>()) as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &attributes,
        };
        let target = wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("studio sprites"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(layout)],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(target)],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline }
    }

    /// Puts sprites on the GPU for a target of `size`, with pictures from an atlas of
    /// `atlas_side`. None for an empty list.
    pub(crate) fn batch(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: (u32, u32),
        (atlas, atlas_side): (&wgpu::TextureView, u32),
        sampler: &wgpu::Sampler,
        sprites: &[Sprite],
    ) -> Option<Batch> {
        if sprites.is_empty() {
            return None;
        }
        let target = crate::uniform(device, "studio sprite target", 4);
        crate::write(queue, &target, &[size.0 as f32, size.1 as f32, atlas_side as f32, 0.0]);
        let floats: Vec<f32> = sprites.iter().flat_map(Sprite::floats).collect();
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("studio sprite instances"),
            size: size_of_val(floats.as_slice()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        crate::write(queue, &instances, &floats);
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("studio sprite bindings"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: target.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(atlas) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        });
        Some(Batch { bindings, instances, count: sprites.len() as u32 })
    }

    /// Draws a batch over what `target` holds, or over transparent black when `clear` is set.
    pub(crate) fn draw(&self, encoder: &mut wgpu::CommandEncoder, label: &str, batch: &Batch, target: &wgpu::TextureView, clear: bool) {
        let load = if clear { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { wgpu::LoadOp::Load };
        let mut pass = crate::begin_pass(encoder, label, target, load);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &batch.bindings, &[]);
        pass.set_vertex_buffer(0, batch.instances.slice(..));
        pass.draw(0..4, 0..batch.count);
    }
}
