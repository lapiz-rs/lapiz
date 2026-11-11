wesl::wesl_pkg!(pub image);

use std::{
    io::{BufRead, Seek},
    sync::Arc,
};

use anyhow::Result;
use bevy_math::IRect;
use glam::{IVec2, UVec2};
use imagers::{ImageDecoder as _, ImageReader};
use lapiz_i18n::t;
use lapiz_runtime::{Runtime, plugin::Plugin};
use moxcms::ColorProfile;
// TODO move CImage to another place to avoid this.
extern crate image as imagers;

use crate::{
    blend_modes::BlendMode,
    composite::{BlendFunctionRegistry, LayerPreviewOverriders},
    layer::{
        LayerId, LayerNameGenerator, LayerStack, LayerStackNode, LayerTypeRegistry, SpecialLayers,
        group_layer::GroupLayer, pixel_layer::PixelLayer,
        properties::builtin::NamePropertyExt as _,
    },
    texel::TexelType,
    tile::GpuTileStorage,
};

pub mod blend_modes;
pub mod composite;
pub mod convert;
pub mod copy_layer;
pub mod dynamic_intermediate_buffer;
pub mod layer;
pub mod layer_bounds;
pub mod lazuli;
pub mod scan_pixels;
pub mod texel;
pub mod tile;

lapiz_i18n::define_i18n!("image");

/// Runtime plugin that initializes image localization, [`GpuTileStorage`], [`LayerPreviewOverriders`], built-in [`BlendMode`]s, and standard layer types.
pub struct ImagePlugin;

impl Plugin for ImagePlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();
        app.add_global::<GpuTileStorage>()
            .add_global::<LayerPreviewOverriders>();

        let mut blend_functions = BlendFunctionRegistry::default();
        for blend_mode in BlendMode::ALL {
            blend_functions.register(Arc::new(blend_mode));
        }
        app.add_global_instance(blend_functions);

        let mut layer_types = LayerTypeRegistry::default();
        layer_types.register::<PixelLayer>();
        layer_types.register::<GroupLayer>();
        app.add_global_instance(layer_types);
    }
}

/// A multi-layered canvas document image managed by the editing engine.
///
/// `CImage` represents the structural state of an open document, including its pixel dimensions,
/// working [`ColorProfile`], default [`TexelType`], hierarchical [`LayerStack`], and [`SpecialLayers`]
/// (such as the active selection mask). Actual pixel tile data resides on the GPU inside
/// [`GpuTileStorage`], indexed by each layer's [`LayerId`].
#[derive(Debug)]
pub struct CImage {
    size: UVec2,
    profile: ColorProfile,
    texel_type: TexelType,
    layers: LayerStack,
    name_generator: LayerNameGenerator,
    special_layers: SpecialLayers,
}

impl CImage {
    pub fn new(size: UVec2, profile: ColorProfile) -> Self {
        let layers = LayerStack::with_empty_background();

        Self {
            size,
            profile,
            texel_type: TexelType::RGBA8,
            layers,
            name_generator: LayerNameGenerator::default(),
            special_layers: SpecialLayers::new(),
        }
    }

    pub fn from_layer(size: UVec2, layer: LayerStackNode, profile: ColorProfile) -> Self {
        let layers = LayerStack::with_background_layer(layer);

        Self {
            size,
            profile,
            texel_type: TexelType::RGBA8,
            layers,
            name_generator: Default::default(),
            special_layers: SpecialLayers::new(),
        }
    }

    pub fn load_image_with_profile<R: BufRead + Seek>(
        r: R,
    ) -> Result<(imagers::DynamicImage, ColorProfile)> {
        let mut decoder = ImageReader::new(r).with_guessed_format()?.into_decoder()?;
        let profile = match decoder.icc_profile()? {
            Some(buf) => ColorProfile::new_from_slice(&buf)?,
            None => ColorProfile::new_srgb(),
        };
        let img = imagers::DynamicImage::from_decoder(decoder)?;
        Ok((img, profile))
    }

    pub fn from_image(
        img: imagers::DynamicImage,
        profile: ColorProfile,
        tiles: &GpuTileStorage,
    ) -> Self {
        let size = UVec2::new(img.width(), img.height());
        let mut layer = PixelLayer::from_image(img, tiles);
        layer.properties_mut().set_name(t!("background_layer"));
        Self::from_layer(size, layer, profile)
    }

    pub fn next_name_of_layer(&mut self, base: String) -> String {
        self.name_generator.next_of(base)
    }

    pub fn layer_stack(&self) -> &LayerStack {
        &self.layers
    }

    pub fn layer_stack_mut(&mut self) -> &mut LayerStack {
        &mut self.layers
    }

    pub fn profile(&self) -> &ColorProfile {
        &self.profile
    }

    pub fn size(&self) -> UVec2 {
        self.size
    }

    pub fn texel_type(&self) -> TexelType {
        self.texel_type
    }

    pub fn selection_layer(&self) -> LayerId {
        self.special_layers.selection_layer()
    }

    pub fn image_tile_rect(&self) -> IRect {
        GpuTileStorage::pixel_rect_to_tile(self.image_pixel_rect())
    }

    pub fn image_pixel_rect(&self) -> IRect {
        IRect {
            min: IVec2::ZERO,
            max: self.size.as_ivec2(),
        }
    }
}
