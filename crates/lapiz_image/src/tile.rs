use std::{
    borrow::{Borrow, Cow},
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use anyhow::Result;
use bevy_math::IRect;
use dashmap::{DashMap, Entry};
use encase::ShaderType;
use futures::{FutureExt, future::join_all};
use glam::{IVec2, UVec2};
use image::{DynamicImage, GenericImageView};
use indexmap::{IndexMap, IndexSet};
use lapiz_render::{
    buffer::BufferVec, readback::readback_buffer_raw_on_submit_async,
    render_context::RenderContextAppExt, util::DevicePollExt,
};
use lapiz_runtime::{
    Services,
    service::{FromServices, Service},
};
use lapiz_utils::Deref;
use moxcms::{ColorProfile, TransformOptions};
use wgpu::{
    Buffer, BufferDescriptor, BufferUsages, Device, Extent3d, ImageSubresourceRange, Origin3d,
    Queue, TexelCopyBufferInfo, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture,
    TextureAspect, TextureDescriptor, TextureDimension, TextureUsages, TextureView,
    TextureViewDescriptor, TextureViewDimension, util::DeviceExt,
};

use crate::{
    convert::ColorProfileConvertPipeline,
    layer::LayerId,
    texel::{TexelDepth, TexelFormat, TexelType},
};

pub trait TileStorageAppExt {
    fn tile_storage(&self) -> &GpuTileStorage;
}

impl TileStorageAppExt for Services {
    fn tile_storage(&self) -> &GpuTileStorage {
        self.service::<GpuTileStorage>()
    }
}

static EMPTY_LAYER_BINDINGS: OnceLock<HashMap<TexelType, LayerBinding>> = OnceLock::new();

#[derive(Clone, Deref)]
pub struct GpuTileStorage {
    inner: Arc<GpuTileStorageInner>,
}

impl std::fmt::Debug for GpuTileStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuTileStorage").finish()
    }
}

impl Service for GpuTileStorage {}

impl FromServices for GpuTileStorage {
    fn from_services(services: &Services) -> Self {
        let device = services.render_device();
        let queue = services.render_queue();
        let mut dummy_layers = HashMap::new();
        for texel_type in TexelType::ALL_POSSIBLE_FORMATS {
            let mut storage = DynamicLayerStorage::new(
                device.clone(),
                queue.clone(),
                GpuLayerInfo { texel_type },
            );
            storage.get_tile_or_allocate(GpuTileInfo::NULL.index);
            dummy_layers.insert(texel_type, storage.binding().unwrap());
        }
        EMPTY_LAYER_BINDINGS.set(dummy_layers).unwrap();

        Self::new(device.clone(), queue.clone())
    }
}

impl GpuTileStorage {
    pub const TILE_SIZE: u32 = 256;
    pub const TILE_COPY_SIZE: Extent3d = Extent3d {
        width: Self::TILE_SIZE,
        height: Self::TILE_SIZE,
        depth_or_array_layers: 1,
    };

    pub fn new(device: Device, queue: Queue) -> Self {
        Self {
            inner: Arc::new(GpuTileStorageInner::new(device, queue)),
        }
    }

    pub fn pixel_rect_to_tile(pixel_rect: IRect) -> IRect {
        IRect {
            min: pixel_rect.min / IVec2::splat(Self::TILE_SIZE as i32),
            max: (pixel_rect.max - 1) / IVec2::splat(Self::TILE_SIZE as i32) + 1,
        }
    }

    pub fn tile_rect_to_pixel(tile_rect: IRect) -> IRect {
        IRect {
            min: tile_rect.min * IVec2::splat(Self::TILE_SIZE as i32),
            max: tile_rect.max * IVec2::splat(Self::TILE_SIZE as i32),
        }
    }

    pub fn tile_to_pixel_rect(tile: IVec2) -> IRect {
        IRect {
            min: tile * IVec2::splat(Self::TILE_SIZE as i32),
            max: (tile + IVec2::ONE) * IVec2::splat(Self::TILE_SIZE as i32),
        }
    }

    pub fn snap_to_tile_grid(pixel_rect: IRect) -> IRect {
        let tile_rect = Self::pixel_rect_to_tile(pixel_rect);
        IRect {
            min: tile_rect.min * IVec2::splat(Self::TILE_SIZE as i32),
            max: tile_rect.max * IVec2::splat(Self::TILE_SIZE as i32),
        }
    }

    pub fn calc_tile_count(image_size: UVec2) -> UVec2 {
        UVec2::new(
            image_size.x.div_ceil(Self::TILE_SIZE),
            image_size.y.div_ceil(Self::TILE_SIZE),
        )
    }

    pub fn get_empty_layer_binding(texel_type: TexelType) -> LayerBinding {
        EMPTY_LAYER_BINDINGS
            .get()
            .unwrap()
            .get(&texel_type)
            .unwrap()
            .clone()
    }
}

#[derive(ShaderType, Clone, Copy, PartialEq, Eq)]
pub struct GpuTileInfo {
    pub index: IVec2,
    pub origin: IVec2,
}

impl Default for GpuTileInfo {
    fn default() -> Self {
        Self::NULL
    }
}

impl GpuTileInfo {
    pub const NULL: Self = Self {
        index: IVec2::splat(i32::MIN / GpuTileStorage::TILE_SIZE as i32),
        origin: IVec2::splat(
            (i32::MIN / GpuTileStorage::TILE_SIZE as i32) * GpuTileStorage::TILE_SIZE as i32,
        ),
    };

    pub fn new(index: IVec2) -> Self {
        Self {
            index,
            origin: index * GpuTileStorage::TILE_SIZE as i32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuLayerInfo {
    pub texel_type: TexelType,
}

#[derive(Debug, Clone)]
pub struct LayerBinding {
    pub texture: TextureView,
    pub tile_info_buffer: Buffer,
}

pub struct GpuTileStorageInner {
    device: Device,
    queue: Queue,

    layers: DashMap<LayerId, DynamicLayerStorage>,
}

impl GpuTileStorageInner {
    pub fn new(device: Device, queue: Queue) -> Self {
        Self {
            device,
            queue,
            layers: DashMap::new(),
        }
    }

    pub fn clear_layer(&self, layer_id: LayerId) {
        if let Some(mut layer) = self.layers.get_mut(&layer_id) {
            layer.clear();
        }
    }

    pub fn declare_layer(&self, layer_id: LayerId, info: GpuLayerInfo) {
        match self.layers.entry(layer_id) {
            Entry::Occupied(e) => {
                assert_eq!(e.get().layer_info, info, "Declare layer info mismatch")
            }
            Entry::Vacant(e) => {
                e.insert(DynamicLayerStorage::new(
                    self.device.clone(),
                    self.queue.clone(),
                    info,
                ));
            }
        }
    }

    pub fn get_layer_info(&self, layer_id: LayerId) -> Option<GpuLayerInfo> {
        self.layers.get(&layer_id).map(|l| *l.layer_info())
    }

    pub fn get_layer_tiles(&self, layer_id: LayerId) -> Option<Vec<IVec2>> {
        self.layers
            .get(&layer_id)
            .map(|l| l.tiles.keys().cloned().collect())
    }

    pub fn get_layer(
        &self,
        layer_id: LayerId,
    ) -> Option<dashmap::mapref::one::Ref<'_, LayerId, DynamicLayerStorage>> {
        self.layers.get(&layer_id)
    }

    pub fn get_layer_mut(
        &self,
        layer_id: LayerId,
    ) -> Option<dashmap::mapref::one::RefMut<'_, LayerId, DynamicLayerStorage>> {
        self.layers.get_mut(&layer_id)
    }

    pub fn get_layer_binding_or_empty(&self, layer_id: LayerId) -> Option<LayerBinding> {
        Some(self.layers.get(&layer_id)?.binding_or_empty())
    }

    pub fn upload_image(&self, layer_id: LayerId, img: DynamicImage) {
        let width = img.width();
        let height = img.height();

        let layer_info = GpuLayerInfo {
            texel_type: TexelType {
                format: TexelFormat::Rgba,
                depth: TexelDepth::Bit8,
            },
        };
        self.declare_layer(layer_id, layer_info);
        let mut layer = self.layers.get_mut(&layer_id).unwrap();

        let mut ec = self.device.create_command_encoder(&Default::default());
        let n_tiles = GpuTileStorage::calc_tile_count(UVec2::new(width, height));
        let tiles = (0..n_tiles.y)
            .flat_map(|y| (0..n_tiles.x).map(move |x| IVec2::new(x as i32, y as i32)));
        layer.allocate_tiles_batch(tiles.clone());

        for tile_index in tiles {
            let tile = layer.get_tile(tile_index).unwrap();
            let tile_layer = layer.get_tile_layer(tile_index).unwrap();
            log::debug!("Uploading tile: {:?}", tile_index);
            let origin = tile_index.as_uvec2() * GpuTileStorage::TILE_SIZE;

            let sub_img = img.view(
                origin.x,
                origin.y,
                GpuTileStorage::TILE_SIZE.min(width - origin.x),
                GpuTileStorage::TILE_SIZE.min(height - origin.y),
            );
            let data = layer_info
                .texel_type
                .convert_image_to_wgpu(DynamicImage::from(sub_img.to_image()));

            let texture = self.device.create_texture_with_data(
                &self.queue,
                &TextureDescriptor {
                    label: Some("temp tile texture"),
                    size: Extent3d {
                        width: sub_img.width(),
                        height: sub_img.height(),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: layer_info.texel_type.wgpu_format(),
                    usage: TextureUsages::COPY_SRC | TextureUsages::COPY_DST,
                    view_formats: &[],
                },
                Default::default(),
                bytemuck::cast_slice(&data),
            );

            ec.copy_texture_to_texture(
                texture.as_image_copy(),
                TexelCopyTextureInfo {
                    texture: tile.texture(),
                    mip_level: 0,
                    origin: Origin3d {
                        x: 0,
                        y: 0,
                        z: tile_layer,
                    },
                    aspect: TextureAspect::All,
                },
                Extent3d {
                    width: sub_img.width(),
                    height: sub_img.height(),
                    depth_or_array_layers: 1,
                },
            );
        }

        self.queue.submit([ec.finish()]);
    }
}

pub const DEFAULT_LAYER_TEXTURE_USAGES: TextureUsages = TextureUsages::from_bits_truncate(
    TextureUsages::TEXTURE_BINDING.bits()
        | TextureUsages::COPY_DST.bits()
        | TextureUsages::COPY_SRC.bits()
        | TextureUsages::STORAGE_BINDING.bits()
        | TextureUsages::RENDER_ATTACHMENT.bits(),
);

pub const DEFAULT_LAYER_TEXTURE_LABEL: Cow<'static, str> = Cow::Borrowed("tile_texture");

pub const DEFAULT_LAYER_TILE_INFO_BUFFER_USAGES: BufferUsages = BufferUsages::from_bits_truncate(
    BufferUsages::COPY_DST.bits() | BufferUsages::COPY_SRC.bits() | BufferUsages::STORAGE.bits(),
);

pub const DEFAULT_LAYER_TILE_INFO_BUFFER_LABEL: Cow<'static, str> =
    Cow::Borrowed("tile_info_buffer");

pub struct DynamicLayerStorage {
    device: Device,
    queue: Queue,

    texture_usages: TextureUsages,
    texture_label: Cow<'static, str>,
    layer_info: GpuLayerInfo,

    texture: Option<TextureView>,
    tiles: IndexMap<IVec2, TextureView>,
    tile_info_buffer: BufferVec<GpuTileInfo>,
    allocation_generation: u64,
}

impl DynamicLayerStorage {
    pub const GROWTH_RATE: f32 = 1.5;
    pub const TILE_SIZE: u32 = GpuTileStorage::TILE_SIZE;

    pub fn new(device: Device, queue: Queue, info: GpuLayerInfo) -> Self {
        Self::new_full(device, queue, None, None, None, None, info)
    }

    pub fn new_full(
        device: Device,
        queue: Queue,
        texture_label: Option<Cow<'static, str>>,
        tile_info_buffer_label: Option<Cow<'static, str>>,
        texture_usage: Option<TextureUsages>,
        tile_info_buffer_usage: Option<BufferUsages>,
        info: GpuLayerInfo,
    ) -> Self {
        Self {
            device,
            queue,
            texture_label: texture_label.unwrap_or(DEFAULT_LAYER_TEXTURE_LABEL.clone()),
            texture_usages: texture_usage.unwrap_or(DEFAULT_LAYER_TEXTURE_USAGES),
            layer_info: info,

            texture: None,
            tiles: IndexMap::new(),
            tile_info_buffer: BufferVec::new(
                Some(
                    tile_info_buffer_label.unwrap_or(DEFAULT_LAYER_TILE_INFO_BUFFER_LABEL.clone()),
                ),
                tile_info_buffer_usage.unwrap_or(DEFAULT_LAYER_TILE_INFO_BUFFER_USAGES),
            ),
            allocation_generation: 0,
        }
    }

    pub fn allocation_generation(&self) -> u64 {
        self.allocation_generation
    }

    pub fn layer_info(&self) -> &GpuLayerInfo {
        &self.layer_info
    }

    pub fn binding_or_empty(&self) -> LayerBinding {
        self.binding().unwrap_or_else(|| {
            EMPTY_LAYER_BINDINGS
                .get()
                .unwrap()
                .get(&self.layer_info.texel_type)
                .unwrap()
                .clone()
        })
    }

    pub fn binding(&self) -> Option<LayerBinding> {
        let texture = self.texture_view()?.clone();
        let tile_info_buffer = self.tile_info_buffer()?.clone();
        Some(LayerBinding {
            texture,
            tile_info_buffer,
        })
    }

    pub fn get_tile(&self, coord: IVec2) -> Option<TextureView> {
        self.tiles.get(&coord).cloned()
    }

    pub fn get_tile_layer(&self, coord: IVec2) -> Option<u32> {
        self.tiles.get_index_of(&coord).map(|i| i as u32)
    }

    pub fn tile_info_buffer(&self) -> Option<&Buffer> {
        self.tile_info_buffer.inner_buffer()
    }

    pub fn texture_view(&self) -> Option<&TextureView> {
        self.texture.as_ref()
    }

    pub fn texture(&self) -> Option<&Texture> {
        Some(self.texture_view()?.texture())
    }

    pub fn compute_tile_bounds(&self) -> IRect {
        let mut bounds = IRect::EMPTY;
        for coord in self.tiles.keys().copied() {
            bounds.min = bounds.min.min(coord);
            bounds.max = bounds.max.max(coord + IVec2::ONE);
        }
        bounds
    }

    pub fn allocate_pixels(&mut self, pixel_rect: IRect) {
        let tile_area = GpuTileStorage::pixel_rect_to_tile(pixel_rect);
        self.allocate_tiles(tile_area);
    }

    pub fn allocate_tiles(&mut self, tile_rect: IRect) {
        self.allocate_tiles_batch(
            (tile_rect.min.y..tile_rect.max.y)
                .flat_map(|y| (tile_rect.min.x..tile_rect.max.x).map(move |x| IVec2::new(x, y))),
        );
    }

    pub fn allocate_tiles_batch(&mut self, tiles: impl IntoIterator<Item = impl Borrow<IVec2>>) {
        let tile_to_allocate = tiles
            .into_iter()
            .filter_map(|t| {
                if self.tiles.contains_key(t.borrow()) {
                    None
                } else {
                    Some(*t.borrow())
                }
            })
            .collect::<IndexSet<_>>();
        if tile_to_allocate.is_empty() {
            return;
        }

        self.reserve(tile_to_allocate.len());

        let start_index = self.tiles.len() as u32;
        let texture = self.texture.as_ref().unwrap().texture();

        self.tiles.reserve(tile_to_allocate.len());

        for (i, t) in tile_to_allocate.into_iter().enumerate() {
            self.tiles.insert(
                t,
                texture.create_view(&TextureViewDescriptor {
                    base_array_layer: start_index + i as u32,
                    array_layer_count: Some(1),
                    ..Default::default()
                }),
            );
            self.tile_info_buffer.push(&GpuTileInfo::new(t));
        }
        self.tile_info_buffer
            .write_buffer(&self.device, &self.queue);
    }

    pub fn get_tile_or_allocate(&mut self, coord: IVec2) -> TextureView {
        if let Some(tile) = self.tiles.get(&coord) {
            return tile.clone();
        }

        let tile = if let Some(texture) = &self.texture
            && self.tiles.len() < texture.texture().depth_or_array_layers() as usize
        {
            let tile = texture.texture().create_view(&TextureViewDescriptor {
                base_array_layer: self.tiles.len() as u32,
                array_layer_count: Some(1),
                ..Default::default()
            });

            self.tiles.insert(coord, tile.clone());

            tile
        } else {
            let next_size = match &self.texture {
                Some(t) => {
                    (t.texture().depth_or_array_layers() as f32 * Self::GROWTH_RATE).ceil() as u32
                }
                None => 1,
            };
            let new_texture = self.device.create_texture(&TextureDescriptor {
                label: Some(&self.texture_label),
                size: Extent3d {
                    width: Self::TILE_SIZE,
                    height: Self::TILE_SIZE,
                    depth_or_array_layers: next_size,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: self.layer_info.texel_type.wgpu_format(),
                usage: self.texture_usages,
                view_formats: &[],
            });

            if let Some(old_texture) = &self.texture {
                let mut ce = self.device.create_command_encoder(&Default::default());
                ce.copy_texture_to_texture(
                    old_texture.texture().as_image_copy(),
                    new_texture.as_image_copy(),
                    Extent3d {
                        width: Self::TILE_SIZE,
                        height: Self::TILE_SIZE,
                        depth_or_array_layers: old_texture.texture().depth_or_array_layers(),
                    },
                );
                self.queue.submit([ce.finish()]);
            }

            self.texture = Some(new_texture.create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2Array),
                ..Default::default()
            }));
            for (i, tile) in self.tiles.values_mut().enumerate() {
                *tile = new_texture.create_view(&TextureViewDescriptor {
                    base_array_layer: i as u32,
                    array_layer_count: Some(1),
                    ..Default::default()
                });
            }

            let tile = new_texture.create_view(&TextureViewDescriptor {
                base_array_layer: self.tiles.len() as u32,
                array_layer_count: Some(1),
                ..Default::default()
            });
            self.tiles.insert(coord, tile.clone());

            tile
        };

        self.tile_info_buffer.push(&GpuTileInfo::new(coord));
        self.tile_info_buffer
            .write_buffer(&self.device, &self.queue);

        tile
    }

    pub fn reserve(&mut self, additional: usize) {
        let required = self.len() + additional;
        if required <= self.capacity() {
            return;
        }

        let new_capacity = required.max(self.capacity().max(1) * 2);
        let new_texture = self.device.create_texture(&TextureDescriptor {
            label: Some(&self.texture_label),
            size: Extent3d {
                width: Self::TILE_SIZE,
                height: Self::TILE_SIZE,
                depth_or_array_layers: new_capacity as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: self.layer_info.texel_type.wgpu_format(),
            usage: self.texture_usages,
            view_formats: &[],
        });

        if let Some(old_texture) = &self.texture {
            let mut ce = self.device.create_command_encoder(&Default::default());
            ce.copy_texture_to_texture(
                old_texture.texture().as_image_copy(),
                new_texture.as_image_copy(),
                Extent3d {
                    width: Self::TILE_SIZE,
                    height: Self::TILE_SIZE,
                    depth_or_array_layers: old_texture.texture().depth_or_array_layers(),
                },
            );
            self.queue.submit([ce.finish()]);
        }

        self.texture = Some(new_texture.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..Default::default()
        }));

        for (i, tile) in self.tiles.values_mut().enumerate() {
            *tile = new_texture.create_view(&TextureViewDescriptor {
                base_array_layer: i as u32,
                array_layer_count: Some(1),
                ..Default::default()
            });
        }

        self.tile_info_buffer.reserve_capacity(
            &self.device,
            new_capacity * u64::from(GpuTileInfo::min_size()) as usize,
        );
        self.allocation_generation += 1;
    }

    pub fn clear(&mut self) {
        let allocated_layers = self.len() as u32;
        self.tiles.clear();
        self.tile_info_buffer.clear();
        self.tile_info_buffer
            .write_buffer(&self.device, &self.queue);

        if allocated_layers > 0
            && let Some(tex) = self.texture.as_ref()
        {
            let mut ec = self.device.create_command_encoder(&Default::default());
            ec.clear_texture(
                tex.texture(),
                &ImageSubresourceRange {
                    array_layer_count: Some(allocated_layers),
                    ..Default::default()
                },
            );
            self.queue.submit([ec.finish()]);
        }
    }

    pub fn iter_tile_indices(&self) -> impl Iterator<Item = IVec2> {
        self.tiles.keys().copied()
    }

    pub fn iter_tiles(&self) -> impl Iterator<Item = (IVec2, u32, &TextureView)> {
        self.tiles.iter().map(|(coord, texture)| {
            (
                *coord,
                self.tiles.get_index_of(coord).unwrap() as u32,
                texture,
            )
        })
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.texture()
            .map(|t| t.depth_or_array_layers() as usize)
            .unwrap_or_default()
    }

    pub fn create_allocated_empty_sibling(&self) -> Self {
        if self.tiles.is_empty() {
            return Self::new(self.device.clone(), self.queue.clone(), self.layer_info);
        }

        let texture = self.texture.as_ref().unwrap().texture();
        let new_texture = self.device.create_texture(&TextureDescriptor {
            label: Some(&self.texture_label),
            size: Extent3d {
                width: Self::TILE_SIZE,
                height: Self::TILE_SIZE,
                depth_or_array_layers: texture.depth_or_array_layers(),
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: self.layer_info.texel_type.wgpu_format(),
            usage: self.texture_usages,
            view_formats: &[],
        });

        let new_texture_view = new_texture.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..Default::default()
        });

        let mut new_tiles = IndexMap::new();
        let mut new_tile_info_buffer =
            BufferVec::new(Some("tile info buffer clone".into()), BufferUsages::STORAGE);

        for (i, (coord, _)) in self.tiles.iter().enumerate() {
            new_tiles.insert(
                *coord,
                new_texture.create_view(&TextureViewDescriptor {
                    base_array_layer: i as u32,
                    array_layer_count: Some(1),
                    ..Default::default()
                }),
            );
            new_tile_info_buffer.push(&GpuTileInfo::new(*coord));
        }

        new_tile_info_buffer.write_buffer(&self.device, &self.queue);

        Self {
            device: self.device.clone(),
            queue: self.queue.clone(),
            texture_label: self.texture_label.clone(),
            texture_usages: self.texture_usages,
            layer_info: self.layer_info,
            texture: Some(new_texture_view),
            tiles: new_tiles,
            tile_info_buffer: new_tile_info_buffer,
            allocation_generation: 0,
        }
    }

    pub fn deep_clone(&self) -> Self {
        let sibling = self.create_allocated_empty_sibling();

        if !sibling.is_empty() {
            let mut ce = self.device.create_command_encoder(&Default::default());
            ce.copy_texture_to_texture(
                self.texture.as_ref().unwrap().texture().as_image_copy(),
                sibling.texture.as_ref().unwrap().texture().as_image_copy(),
                Extent3d {
                    width: GpuTileStorage::TILE_SIZE,
                    height: GpuTileStorage::TILE_SIZE,
                    depth_or_array_layers: self.len() as u32,
                },
            );
            self.queue.submit([ce.finish()]);
        }

        sibling
    }

    pub fn convert_color_space(
        &self,
        src_pr: &ColorProfile,
        dst_pr: &ColorProfile,
        options: TransformOptions,
    ) -> Result<()> {
        let Some(texture) = &self.texture else {
            return Ok(());
        };

        let converter = ColorProfileConvertPipeline::new(
            &self.device,
            self.layer_info.texel_type,
            src_pr,
            self.layer_info.texel_type.moxcms_layout(),
            dst_pr,
            self.layer_info.texel_type.moxcms_layout(),
            options,
        )?;
        converter.convert(&self.device, &self.queue, texture);

        Ok(())
    }

    pub async fn readback(
        &self,
        device: &Device,
        queue: &Queue,
        tiles: impl IntoIterator<Item = IVec2>,
    ) -> Result<HashMap<IVec2, Vec<u8>>> {
        let Some(texture) = self.texture() else {
            return Ok(HashMap::new());
        };

        let mut readbacks = HashMap::new();
        let pixel_size = self
            .layer_info
            .texel_type
            .wgpu_format()
            .block_copy_size(None)
            .unwrap();
        let tile_size = pixel_size * GpuTileStorage::TILE_SIZE * GpuTileStorage::TILE_SIZE;

        let mut ec = device.create_command_encoder(&Default::default());

        for index in tiles {
            if readbacks.contains_key(&index) {
                continue;
            }

            let Some(layer) = self.tiles.get_index_of(&index) else {
                continue;
            };

            let buffer = device.create_buffer(&BufferDescriptor {
                label: Some(&format!("readback_tile_{:?}", index)),
                size: tile_size as u64,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            ec.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: TextureAspect::All,
                },
                TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(pixel_size * GpuTileStorage::TILE_SIZE),
                        rows_per_image: Some(GpuTileStorage::TILE_SIZE),
                    },
                },
                GpuTileStorage::TILE_COPY_SIZE,
            );

            let readback = readback_buffer_raw_on_submit_async(&mut ec, &buffer, ..);

            readbacks.insert(index, readback);
        }

        let si = queue.submit([ec.finish()]);
        // TODO Don't block!!!
        //      We should use a background task that infinitely polls.
        device.poll_indefinitely_for(si)?;

        let buffers = join_all(
            readbacks
                .into_iter()
                .map(|(index, readback)| readback.into_inner().map(move |buf| (index, buf))),
        )
        .await;

        let mut result = HashMap::with_capacity(buffers.len());
        for (index, buf) in buffers {
            result.insert(index, buf??);
        }

        Ok(result)
    }

    pub fn write_raw(&mut self, queue: &Queue, tile: IVec2, data: &[u8]) {
        self.get_tile_or_allocate(tile);
        let layer = self.tiles.get_index_of(&tile).unwrap();
        let pixel_size = self
            .layer_info
            .texel_type
            .wgpu_format()
            .block_copy_size(None)
            .unwrap();
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: self.texture().unwrap(),
                mip_level: 0,
                origin: Origin3d {
                    x: 0,
                    y: 0,
                    z: layer as u32,
                },
                aspect: TextureAspect::All,
            },
            data,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(pixel_size * GpuTileStorage::TILE_SIZE),
                rows_per_image: Some(GpuTileStorage::TILE_SIZE),
            },
            GpuTileStorage::TILE_COPY_SIZE,
        );
    }
}
