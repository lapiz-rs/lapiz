use bevy_math::{URect, UVec2};
use wgpu::{
    BindingResource, Buffer, BufferUsages, Device, Extent3d, Origin3d, Queue, TexelCopyTextureInfo,
    Texture, TextureAspect, TextureDimension, TextureUsages, TextureView, TextureViewDescriptor,
};

use crate::buffer_vec::BufferVec;

/// Builder that collects individual GPU [`Texture`]s and packs them into a single [`TextureAtlas`].
///
/// Uses a skyline bin-packing algorithm to arrange 2D textures of the same format into a
/// power-of-two width atlas texture, copying texels on the GPU and uploading their bounding
/// rectangles to a storage buffer.
pub struct TextureAtlasBuilder {
    textures: Vec<Texture>,
}

impl Default for TextureAtlasBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TextureAtlasBuilder {
    pub fn new() -> Self {
        Self {
            textures: Vec::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            textures: Vec::with_capacity(capacity),
        }
    }

    pub fn len(&self) -> usize {
        self.textures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }

    pub fn add_texture(&mut self, texture: Texture) -> usize {
        self.textures.push(texture);
        self.textures.len() - 1
    }

    pub fn build(
        self,
        atlas_name: Option<&str>,
        device: &Device,
        queue: &Queue,
    ) -> Option<TextureAtlas> {
        if self.textures.is_empty() {
            return None;
        }

        let sizes = self
            .textures
            .iter()
            .map(|t| {
                let s = t.size();
                UVec2::new(s.width, s.height)
            })
            .collect::<Vec<_>>();

        let (atlas_size, placements) = skyline_pack(&sizes);

        let format = self.textures[0].format();

        let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: atlas_name,
            size: Extent3d {
                width: atlas_size.x,
                height: atlas_size.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_DST
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let mut encoder = device.create_command_encoder(&Default::default());

        let atlas_bounds = placements
            .iter()
            .zip(sizes.iter())
            .map(|(&pos, &size)| URect {
                min: pos,
                max: pos + size,
            })
            .collect::<Vec<_>>();

        for (texture, bounds) in self.textures.iter().zip(atlas_bounds.iter()) {
            encoder.copy_texture_to_texture(
                texture.as_image_copy(),
                TexelCopyTextureInfo {
                    texture: &atlas_texture,
                    mip_level: 0,
                    origin: Origin3d {
                        x: bounds.min.x,
                        y: bounds.min.y,
                        z: 0,
                    },
                    aspect: TextureAspect::All,
                },
                texture.size(),
            );
        }

        queue.submit([encoder.finish()]);

        let texture_view = atlas_texture.create_view(&TextureViewDescriptor::default());

        let atlases = self
            .textures
            .iter()
            .map(|t| t.create_view(&TextureViewDescriptor::default()))
            .collect::<Vec<_>>();

        let mut atlas_bounds_buffer = BufferVec::new(BufferUsages::STORAGE);
        atlas_bounds_buffer.set_label(Some(&format!(
            "{}_atlas_bounds_buffer",
            atlas_name.unwrap_or_default()
        )));
        for bounds in &atlas_bounds {
            atlas_bounds_buffer.push(*bounds);
        }
        atlas_bounds_buffer.write_buffer(device, queue);

        Some(TextureAtlas {
            texture: atlas_texture,
            texture_view,
            atlases,
            atlas_bounds,
            atlas_bounds_buffer: atlas_bounds_buffer.buffer().unwrap().clone(),
        })
    }
}

/// Internal skyline heightfield state used for 2D rectangle bin packing.
struct Skyline {
    nodes: Vec<(u32, u32)>,
    width: u32,
}

impl Skyline {
    fn new(width: u32) -> Self {
        Self {
            nodes: vec![(0, 0)],
            width,
        }
    }

    fn find_placement(&self, w: u32) -> Option<(u32, u32)> {
        let mut best = None::<(u32, u32)>;
        for (i, &(sx, _)) in self.nodes.iter().enumerate() {
            if sx + w > self.width {
                break;
            }
            let x_end = sx + w;
            let max_h = self.nodes[i..]
                .iter()
                .take_while(|&&(nx, _)| nx < x_end)
                .map(|&(_, ny)| ny)
                .max()
                .unwrap_or(0);

            if best.is_none_or(|(_, by)| max_h < by) {
                best = Some((sx, max_h));
            }
        }
        best
    }

    fn place(&mut self, px: u32, py: u32, pw: u32, ph: u32) {
        let new_y = py + ph;
        let rect_end = px + pw;

        self.ensure_node(px);
        if rect_end < self.width {
            self.ensure_node(rect_end);
        }

        let start = self.nodes.partition_point(|&(sx, _)| sx < px);
        let end = self.nodes.partition_point(|&(sx, _)| sx < rect_end);
        for (_, sy) in &mut self.nodes[start..end] {
            *sy = new_y;
        }

        self.nodes.dedup_by_key(|(_, y)| *y);
    }

    fn ensure_node(&mut self, x: u32) {
        let pos = self.nodes.partition_point(|&(sx, _)| sx < x);
        if self.nodes.get(pos).is_some_and(|&(sx, _)| sx == x) {
            return;
        }

        let y = if pos > 0 { self.nodes[pos - 1].1 } else { 0 };
        self.nodes.insert(pos, (x, y));
    }
}

fn skyline_pack(sizes: &[UVec2]) -> (UVec2, Vec<UVec2>) {
    if sizes.is_empty() {
        return (UVec2::ZERO, vec![]);
    }

    let total_area = sizes.iter().map(|s| s.x as u64 * s.y as u64).sum::<u64>();
    let max_w = sizes.iter().map(|s| s.x).max().unwrap_or(1);

    let est_side = (total_area as f64).sqrt() as u32;
    let atlas_width = est_side.max(max_w).next_power_of_two();

    let mut order = (0..sizes.len()).collect::<Vec<usize>>();
    order.sort_by(|&a, &b| {
        sizes[b]
            .y
            .cmp(&sizes[a].y)
            .then_with(|| sizes[b].x.cmp(&sizes[a].x))
    });

    let mut skyline = Skyline::new(atlas_width);
    let mut placements = vec![UVec2::ZERO; sizes.len()];

    for &i in &order {
        let UVec2 { x: w, y: h } = sizes[i];
        let (px, py) = skyline
            .find_placement(w)
            .expect("texture wider than atlas – should never happen");
        placements[i] = UVec2::new(px, py);
        skyline.place(px, py, w, h);
    }

    let actual_size = placements
        .iter()
        .zip(sizes.iter())
        .map(|(p, s)| *p + *s)
        .fold(UVec2::ONE, |a, b| a.max(b));

    (actual_size, placements)
}

/// A packed 2D GPU texture atlas alongside per-entry [`URect`] bounds and individual [`TextureView`]s.
///
/// `TextureAtlas` stores the combined [`Texture`], its full [`TextureView`], the original per-item
/// views, and a GPU storage [`Buffer`] containing the packed [`URect`] coordinates for shader indexing.
#[derive(Clone)]
pub struct TextureAtlas {
    texture: Texture,
    texture_view: TextureView,
    atlases: Vec<TextureView>,
    atlas_bounds: Vec<URect>,
    atlas_bounds_buffer: Buffer,
}

impl TextureAtlas {
    pub fn texture(&self) -> &Texture {
        &self.texture
    }

    pub fn texture_view(&self) -> &TextureView {
        &self.texture_view
    }

    pub fn atlases(&self) -> &[TextureView] {
        &self.atlases
    }

    pub fn atlas_bounds(&self) -> &[URect] {
        &self.atlas_bounds
    }

    pub fn atlas_bounds_buffer_binding(&self) -> BindingResource<'_> {
        self.atlas_bounds_buffer.as_entire_binding()
    }

    pub fn atlas_view(&self, index: usize) -> Option<&TextureView> {
        self.atlases.get(index)
    }

    pub fn atlas_bound(&self, index: usize) -> Option<URect> {
        self.atlas_bounds.get(index).copied()
    }
}
