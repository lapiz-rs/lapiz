use std::{
    any::Any,
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
};

use bevy_math::IRect;
use dyn_clone::DynClone;
use encase::ShaderType;
use glam::IVec2;
use lapiz_runtime::{Services, service::Service};
use lapiz_utils::wrapper;
use log::error;
use parse_display::Display;
use serde::{Deserialize, Serialize};
use wgpu::{Buffer, ComputePassDescriptor, Device, Queue, TextureView};

use crate::{
    CImage,
    layer::LayerId,
    tile::{DynamicLayerStorage, GpuTileStorage},
};

pub trait BlendFunction: Send + Sync + DynClone + 'static {
    fn id(&self) -> BlendFunctionId;
    fn wgsl_function_call(&self, src_ident: &str, dst_ident: &str) -> String;
}
dyn_clone::clone_trait_object!(BlendFunction);

wrapper! {
    #[derive(Debug, Clone, PartialEq, Eq, Hash, Display, Serialize, Deserialize)]
    #[display("{0}")]
    pub BlendFunctionId : Arc<str>
}

pub trait BlendFunctionAppExt {
    fn add_blend_function(&mut self, func: Arc<dyn BlendFunction>);
}

impl BlendFunctionAppExt for Services {
    fn add_blend_function(&mut self, func: Arc<dyn BlendFunction>) {
        self.service_mut::<BlendFunctionRegistry>().register(func);
    }
}

#[derive(Default)]
pub struct BlendFunctionRegistry {
    functions: HashMap<BlendFunctionId, Arc<dyn BlendFunction>>,
}

impl Service for BlendFunctionRegistry {}

impl BlendFunctionRegistry {
    pub fn register(&mut self, func: Arc<dyn BlendFunction>) {
        match self.functions.entry(func.id()) {
            Entry::Occupied(e) => {
                error!("Blend function '{}' is already registered", e.key());
            }
            Entry::Vacant(e) => {
                e.insert(func);
            }
        }
    }

    pub fn get(&self, name: &BlendFunctionId) -> Option<&Arc<dyn BlendFunction>> {
        self.functions.get(name)
    }

    pub fn all(&self) -> impl Iterator<Item = &Arc<dyn BlendFunction>> {
        self.functions.values()
    }

    pub fn all_ids(&self) -> impl Iterator<Item = &BlendFunctionId> {
        self.functions.keys()
    }
}

#[derive(Default)]
pub struct ImageCompositor {
    cache: HashMap<LayerId, Box<dyn Any + Send + Sync>>,
}

impl ImageCompositor {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    // TODO: incremental cache building and preparing
    pub fn create_cache(
        &mut self,
        overriders: &mut LayerPreviewOverriders,
        image: &CImage,
        tiles: &GpuTileStorage,
        blend_funcs: &BlendFunctionRegistry,
        device: &Device,
        queue: &Queue,
    ) {
        let now = std::time::Instant::now();
        let root_node = image
            .layer_stack()
            .get_layer(image.layer_stack().root_id())
            .unwrap();
        root_node.create_blend_cache(self, overriders, image, tiles, blend_funcs, device, queue);
        log::debug!("Blend cache created in {:?}", now.elapsed());
    }

    pub fn composite(
        &mut self,
        overriders: &LayerPreviewOverriders,
        _: IRect,
        image: &CImage,
        tiles: &GpuTileStorage,
        device: &Device,
        queue: &Queue,
    ) {
        let mut ec = device.create_command_encoder(&Default::default());
        let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
            label: Some("image composite pass"),
            ..Default::default()
        });

        let mut root_layer_tiles = tiles.get_layer_mut(*image.layer_stack().root_id()).unwrap();
        root_layer_tiles.allocate_pixels(IRect {
            min: IVec2::ZERO,
            max: image.size.as_ivec2(),
        });
        let root_layer_binding = root_layer_tiles.binding().unwrap();
        drop(root_layer_tiles);

        let empty_layer_binding = GpuTileStorage::get_empty_layer_binding(image.texel_type());
        let root_node = image
            .layer_stack()
            .get_layer(image.layer_stack().root_id())
            .unwrap();
        let now = std::time::Instant::now();
        root_node.prepare_blend_cache(
            self,
            overriders,
            image,
            tiles,
            &empty_layer_binding,
            &root_layer_binding,
            device,
            queue,
        );
        log::debug!("Blend cache prepared in {:?}", now.elapsed());

        let now = std::time::Instant::now();
        // unsafe {
        //     device.start_graphics_debugger_capture();
        // }
        root_node.dispatch_blend(self, &mut pass, image, tiles);
        log::debug!("Blend dispatched in {:?}", now.elapsed());

        drop(pass);

        queue.submit([ec.finish()]);
        // unsafe {
        //     device.stop_graphics_debugger_capture();
        // }
    }

    pub fn get_blend_cache<T: Send + Sync + 'static>(&self, layer_id: &LayerId) -> Option<&T> {
        self.cache.get(layer_id)?.downcast_ref::<T>()
    }

    pub fn get_blend_cache_mut<T: Send + Sync + 'static>(
        &mut self,
        layer_id: &LayerId,
    ) -> Option<&mut T> {
        self.cache.get_mut(layer_id)?.downcast_mut::<T>()
    }

    pub fn insert_blend_cache<T: Send + Sync + 'static>(&mut self, layer_id: LayerId, cache: T) {
        self.cache.insert(layer_id, Box::new(cache));
    }
}

#[derive(Default)]
pub struct LayerPreviewOverriders {
    overriders: HashMap<LayerId, Box<dyn Any + Send + Sync>>,
}

impl LayerPreviewOverriders {
    pub fn new() -> Self {
        Self {
            overriders: HashMap::new(),
        }
    }

    pub fn get_overrider<T: Send + Sync + 'static>(&self, layer_id: &LayerId) -> Option<&T> {
        let overrider = self.overriders.get(layer_id)?;
        overrider.downcast_ref::<T>()
    }

    pub fn insert_overrider<T: Send + Sync + 'static>(&mut self, layer_id: LayerId, overrider: T) {
        self.overriders.insert(layer_id, Box::new(overrider));
    }

    pub fn remove_overrider(&mut self, layer_id: &LayerId) {
        self.overriders.remove(layer_id);
    }
}

impl Service for LayerPreviewOverriders {}

pub struct PixelPreviewOverrider {
    pub texture: TextureView,
    pub tile_info_buffer: Buffer,
}

impl PixelPreviewOverrider {
    pub fn from_layer_storage(layer: &DynamicLayerStorage) -> Self {
        let binding = layer.binding_or_empty();
        Self {
            texture: binding.texture,
            tile_info_buffer: binding.tile_info_buffer,
        }
    }
}

#[derive(Debug, Clone, Copy, ShaderType)]
pub struct BlendLayerParams {
    pub src_opacity: f32,
    // ...abgr bits
    pub src_disabled_channels: u32,
    pub _pad: [u32; 2],
}
