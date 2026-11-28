//! A texture that small pictures are packed into as they are first needed: cursor frames in one,
//! glyphs in another.

use std::collections::HashMap;
use std::hash::Hash;

/// A picture's place in an atlas, in texels. It includes the clear border of one texel around the
/// picture, which keeps linear filtering from picking up a neighbour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Fills rows from the top left. When it is full the owner clears it and starts again.
pub(crate) struct Packer {
    side: u32,
    next: (u32, u32),
    row_height: u32,
}

impl Packer {
    pub(crate) fn new(side: u32) -> Self {
        Self { side, next: (0, 0), row_height: 0 }
    }

    /// Room for a slot of this size, border included; None when there is none left.
    pub(crate) fn place(&mut self, width: u32, height: u32) -> Option<Slot> {
        if width > self.side || height > self.side {
            return None;
        }
        let (mut x, mut y) = self.next;
        let mut row_height = self.row_height;
        if x + width > self.side {
            (x, y, row_height) = (0, y + row_height, 0);
        }
        if y + height > self.side {
            return None;
        }
        self.next = (x + width, y);
        self.row_height = row_height.max(height);
        Some(Slot { x, y, width, height })
    }
}

/// A square texture of four byte pixels and what is where in it. Each entry keeps a value of its
/// own beside its slot.
pub(crate) struct Atlas<K, V> {
    texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) side: u32,
    packer: Packer,
    entries: HashMap<K, (Slot, V)>,
}

impl<K: Eq + Hash, V: Copy> Atlas<K, V> {
    /// An atlas of `side` texels square, or of the largest side the device has.
    pub(crate) fn new(device: &wgpu::Device, label: &str, side: u32, format: wgpu::TextureFormat) -> Self {
        let side = side.min(device.limits().max_texture_dimension_2d);
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let texture = crate::texture(device, label, (side, side), format, usage);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view, side, packer: Packer::new(side), entries: HashMap::new() }
    }

    pub(crate) fn get(&self, key: &K) -> Option<(Slot, V)> {
        self.entries.get(key).copied()
    }

    /// Adds a picture of `width * height * 4` bytes. None when the atlas has no room left. Draws
    /// already recorded but not submitted would see the new pixels, so after a [`Self::clear`]
    /// everything they use has to be looked up again.
    pub(crate) fn insert(&mut self, queue: &wgpu::Queue, key: K, (width, height): (u32, u32), pixels: &[u8], value: V) -> Option<Slot> {
        let row = width as usize * 4;
        if width == 0 || height == 0 || pixels.len() != row * height as usize {
            return None;
        }
        let slot = self.packer.place(width + 2, height + 2)?;
        let padded_row = slot.width as usize * 4;
        let mut padded = vec![0; padded_row * slot.height as usize];
        for (from, to) in pixels.chunks_exact(row).zip(padded.chunks_exact_mut(padded_row).skip(1)) {
            to[4..4 + row].copy_from_slice(from);
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { origin: wgpu::Origin3d { x: slot.x, y: slot.y, z: 0 }, ..self.texture.as_image_copy() },
            &padded,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(slot.width * 4), rows_per_image: None },
            wgpu::Extent3d { width: slot.width, height: slot.height, depth_or_array_layers: 1 },
        );
        self.entries.insert(key, (slot, value));
        Some(slot)
    }

    /// Forgets every entry.
    pub(crate) fn clear(&mut self) {
        self.packer = Packer::new(self.side);
        self.entries.clear();
    }
}
