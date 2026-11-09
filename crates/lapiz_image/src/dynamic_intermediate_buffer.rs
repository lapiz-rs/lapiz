use bevy_math::IRect;
use encase::ShaderType;
use glam::IVec2;
use lapiz_render::{buffer_vec::BufferVec, storage_buffer::StorageBuffer};
use wgpu::{
    Buffer, BufferUsages, Device, Extent3d, Queue, TextureDescriptor, TextureDimension,
    TextureUsages, TextureView, TextureViewDimension, wgt::TextureViewDescriptor,
};

use crate::{
    texel::TexelType,
    tile::{GpuTileInfo, GpuTileStorage},
};

#[derive(ShaderType)]
pub struct DynamicGpuTileInfoBuffer {
    pub n_tiles: u32,
    #[shader(size(runtime))]
    pub buf: Vec<GpuTileInfo>,
}

pub struct DynamicIntermediateBuffer {
    device: Device,
    queue: Queue,
    textures: [TextureView; 2],
    tile_info: StorageBuffer<DynamicGpuTileInfoBuffer>,
    tiles: u32,
}

impl DynamicIntermediateBuffer {
    pub fn new(initial: u32, texel_type: TexelType, device: Device, queue: Queue) -> Self {
        let desc = TextureDescriptor {
            label: Some("dynamic intermediate buffer texture"),
            size: Extent3d {
                width: GpuTileStorage::TILE_SIZE,
                height: GpuTileStorage::TILE_SIZE,
                depth_or_array_layers: initial,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: texel_type.wgpu_format(),
            usage: TextureUsages::COPY_DST
                | TextureUsages::COPY_SRC
                | TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        };

        let texture_a = device
            .create_texture(&desc)
            .create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2Array),
                ..Default::default()
            });
        let texture_b = device
            .create_texture(&desc)
            .create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2Array),
                ..Default::default()
            });

        let mut info = StorageBuffer::from(DynamicGpuTileInfoBuffer {
            n_tiles: 0,
            buf: vec![GpuTileInfo::default(); initial as usize],
        });
        info.set_label(Some("dynamic intermediate buffer"));
        info.add_usages(BufferUsages::COPY_SRC);
        info.write_buffer(&device, &queue);

        Self {
            device,
            queue,
            textures: [texture_a, texture_b],
            tile_info: info,
            tiles: initial,
        }
    }

    pub fn textures(&self) -> &[TextureView; 2] {
        &self.textures
    }

    pub fn tile_info_buffer(&self) -> &Buffer {
        self.tile_info.buffer().unwrap()
    }

    pub fn clear(&mut self) {
        let mut ec = self.device.create_command_encoder(&Default::default());
        for texture in &self.textures {
            ec.clear_texture(texture.texture(), &Default::default());
        }
        self.queue.submit([ec.finish()]);

        self.tile_info.set(DynamicGpuTileInfoBuffer {
            n_tiles: 0,
            buf: vec![GpuTileInfo::NULL; self.tiles as usize],
        });
        self.tile_info.write_buffer(&self.device, &self.queue);
    }
}

#[derive(Debug, Clone)]
pub struct IntermediateBuffer {
    textures: [TextureView; 2],
    tile_info_buffer: Buffer,
    tile_rect: IRect,
    texel_type: TexelType,
}

impl IntermediateBuffer {
    pub fn new(device: &Device, queue: &Queue, tile_rect: IRect, texel_type: TexelType) -> Self {
        let tiles = tile_rect.size().element_product() as u32;
        let desc = TextureDescriptor {
            label: Some("intermediate buffer texture"),
            size: Extent3d {
                width: GpuTileStorage::TILE_SIZE,
                height: GpuTileStorage::TILE_SIZE,
                depth_or_array_layers: tiles,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: texel_type.wgpu_format(),
            usage: TextureUsages::COPY_DST
                | TextureUsages::COPY_SRC
                | TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        };

        let texture_a = device
            .create_texture(&desc)
            .create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2Array),
                ..Default::default()
            });
        let texture_b = device
            .create_texture(&desc)
            .create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2Array),
                ..Default::default()
            });

        let mut tile_info_buffer = BufferVec::new(BufferUsages::STORAGE);
        tile_info_buffer.set_label(Some("intermediate buffer tile info"));
        for y in tile_rect.min.y..tile_rect.max.y {
            for x in tile_rect.min.x..tile_rect.max.x {
                tile_info_buffer.push(GpuTileInfo::new(IVec2::new(x, y)));
            }
        }
        tile_info_buffer.write_buffer(device, queue);

        Self {
            textures: [texture_a, texture_b],
            tile_rect,
            tile_info_buffer: tile_info_buffer.buffer().unwrap().clone(),
            texel_type,
        }
    }

    pub fn coord_to_layer(&self, coord: IVec2) -> Option<u32> {
        if self.tile_rect.contains(coord) {
            Some((coord.y * self.tile_rect.width() + coord.x) as u32)
        } else {
            None
        }
    }

    pub fn tile_rect(&self) -> IRect {
        self.tile_rect
    }

    pub fn textures(&self) -> &[TextureView; 2] {
        &self.textures
    }

    pub fn tile_info_buffer(&self) -> &Buffer {
        &self.tile_info_buffer
    }

    pub fn clear(&self, device: &Device, queue: &Queue) {
        let mut ec = device.create_command_encoder(&Default::default());
        for texture in &self.textures {
            ec.clear_texture(texture.texture(), &Default::default());
        }
        queue.submit([ec.finish()]);
    }

    pub fn texel_type(&self) -> TexelType {
        self.texel_type
    }
}
