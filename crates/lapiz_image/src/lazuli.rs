use std::{
    collections::HashMap,
    fs::File,
    io::{Read as _, Write as _},
    path::PathBuf,
    result,
};

use anyhow::{Result, anyhow, bail};
use flate2::{Compression, read::DeflateDecoder, write::DeflateEncoder};
use futures::future::join_all;
use glam::{IVec2, UVec2};
use imagers::{DynamicImage, GenericImageView as _};
use lapiz_i18n::t;
use lapiz_lazuli::{LazuliArchive, image_props::ImageProperties, layer_tree::LayerNode};
use lapiz_render::render_context::RenderContextAppExt as _;
use lapiz_runtime::global::Globals;
use moxcms::ColorProfile;
use serde::{Serialize, Serializer, ser};
use uuid::Uuid;
use wgpu::{Device, Queue};

use crate::{
    Image,
    layer::{
        Layer as _, LayerId, LayerStack, LayerStackNode, LayerTypeRegistry, SpecialLayers,
        pixel_layer::PixelLayer,
        properties::{
            EncodedLayerProperties, LayerProperties,
            builtin::{LayerTexelTypePropertyExt as _, NamePropertyExt as _, TexelSource},
        },
    },
    texel::TexelType,
    tile::{GpuLayerInfo, GpuTileStorage, TileStorageAppExt as _},
};

impl Image {
    pub fn from_lazuli(archive: &LazuliArchive, globals: &Globals) -> Result<Self> {
        let image_props = archive.read_image_properties()?;
        let layer_stack = LayerStack::read_entire_tree(
            image_props.root_layer,
            archive,
            globals.global::<LayerTypeRegistry>(),
        )?;

        let queue = globals.render_queue();
        let tile_storage = globals.tile_storage();
        for layer in layer_stack.iter_layers() {
            let Some(texel_type) = layer.properties().get_texel_type() else {
                continue;
            };

            let tile_data = archive.read_layer_data(**layer.id())?;
            tile_storage.declare_layer(*layer.id(), GpuLayerInfo { texel_type });
            let mut layer = tile_storage.get_layer_mut(*layer.id()).unwrap();
            layer.allocate_tiles_batch(tile_data.keys());

            for (index, data) in tile_data {
                let mut d = DeflateDecoder::new(&data[..]);
                let mut buf = Vec::new();
                d.read_to_end(&mut buf)?;
                layer.write_raw(queue, index, &buf);
            }
        }

        let image_texel_type = TexelType::decode(image_props.texel_type)?;

        Ok(Self {
            size: UVec2::new(image_props.width, image_props.height),
            profile: ColorProfile::new_from_slice(&image_props.color_profile)?,
            texel_type: image_texel_type,
            layers: layer_stack,
            name_generator: Default::default(),
            special_layers: SpecialLayers::new(),
        })
    }

    fn archive_image_properties(&self) -> Result<ImageProperties> {
        Ok(ImageProperties {
            width: self.size.x,
            height: self.size.y,
            tile_size: GpuTileStorage::TILE_SIZE,
            color_profile: self.profile.encode()?,
            root_layer: **self.layers.root_id(),
            texel_type: self.texel_type.encode(),
        })
    }

    pub fn new_in_memory_archive(&self) -> Result<LazuliArchive> {
        let archive = LazuliArchive::new_in_memory()?;
        archive.write_image_properties(&self.archive_image_properties()?)?;
        self.layers.write_entire_tree(&archive)?;
        Ok(archive)
    }

    pub fn write_archive(
        &self,
        path: PathBuf,
        globals: &Globals,
    ) -> impl Future<Output = Result<()>> + Send + use<> {
        self.write_archive_with(move || LazuliArchive::new(path), globals)
    }

    pub fn write_archive_file(
        &self,
        file: File,
        globals: &Globals,
    ) -> impl Future<Output = Result<()>> + Send + use<> {
        self.write_archive_with(move || LazuliArchive::new_file(file), globals)
    }

    fn write_archive_with<F>(
        &self,
        create_archive: F,
        globals: &Globals,
    ) -> impl Future<Output = Result<()>> + Send + use<F>
    where
        F: FnOnce() -> Result<LazuliArchive> + Send + 'static,
    {
        let properties = self.archive_image_properties();
        let layers = self.layers.clone();
        let device = globals.render_device().clone();
        let queue = globals.render_queue().clone();
        let tile_storage = globals.tile_storage().clone();

        async move {
            let properties = properties?;
            let archive = create_archive()?;
            archive.write_image_properties(&properties)?;
            layers.write_entire_tree(&archive)?;
            let layer_ids = layers
                .iter_layers()
                .filter(|layer| {
                    layer
                        .properties()
                        .get_texel_prop()
                        .is_some_and(|p| p.source == TexelSource::DirectlyDefined)
                })
                .map(|layer| *layer.id())
                .collect::<Vec<_>>();
            drop(layers);

            let results = join_all(
                layer_ids
                    .into_iter()
                    .map(|layer| tile_storage.write_layer(&device, &queue, &archive, layer)),
            )
            .await;
            for result in results {
                result?;
            }
            Ok(())
        }
    }

    pub fn image_to_lazuli(
        img: DynamicImage,
        profile: ColorProfile,
        layer_registry: &LayerTypeRegistry,
    ) -> Result<LazuliArchive> {
        let size = UVec2::new(img.width(), img.height());

        let mut layer = LayerStackNode::without_parent(
            LayerId::random(),
            Box::new(PixelLayer),
            layer_registry
                .create_default_properties(PixelLayer.layer_type())
                .unwrap(),
        );
        layer.properties_mut().set_name(t!("background_layer"));
        let layer_id = *layer.id();
        let image = Self::from_layer(size, layer, profile, layer_registry);
        let archive = image.new_in_memory_archive()?;

        let texel_type = image
            .layers
            .get_layer(&layer_id)
            .expect("Imported pixel layer should exist")
            .properties()
            .get_texel_type()
            .expect("Pixel layer should declare its texel type");

        let tile_size = GpuTileStorage::TILE_SIZE;
        let pixel_size = texel_type
            .wgpu_format()
            .block_copy_size(None)
            .expect("Image texel format should have a fixed block size")
            as usize;
        let tile_stride = tile_size as usize * pixel_size;
        let tile_count = GpuTileStorage::calc_tile_count(UVec2::new(img.width(), img.height()));

        for tile_y in 0..tile_count.y {
            for tile_x in 0..tile_count.x {
                let origin = UVec2::new(tile_x, tile_y) * tile_size;
                let width = tile_size.min(img.width() - origin.x);
                let height = tile_size.min(img.height() - origin.y);
                let source = texel_type.convert_image_to_wgpu(DynamicImage::from(
                    img.view(origin.x, origin.y, width, height).to_image(),
                ));
                let source_stride = width as usize * pixel_size;
                let mut tile = vec![0; tile_stride * tile_size as usize];
                for row in 0..height as usize {
                    tile[row * tile_stride..row * tile_stride + source_stride]
                        .copy_from_slice(&source[row * source_stride..(row + 1) * source_stride]);
                }
                GpuTileStorage::write_tile_data(
                    &archive,
                    layer_id,
                    IVec2::new(tile_x as i32, tile_y as i32),
                    &tile,
                )?;
            }
        }

        Ok(archive)
    }
}

impl GpuTileStorage {
    pub async fn write_layer(
        &self,
        device: &Device,
        queue: &Queue,
        archive: &LazuliArchive,
        layer_id: LayerId,
    ) -> Result<()> {
        let layer = self
            .get_layer(layer_id)
            .ok_or_else(|| anyhow!("layer {} doesn't exists", layer_id))?;

        let tile_data = layer
            .readback(device, queue, layer.iter_tile_indices())
            .await?;
        drop(layer);

        for (tile, data) in tile_data {
            Self::write_tile_data(archive, layer_id, tile, &data)?;
        }

        Ok(())
    }

    pub async fn write_tiles(
        &self,
        device: &Device,
        queue: &Queue,
        archive: &LazuliArchive,
        layer_id: LayerId,
        tiles: impl IntoIterator<Item = IVec2>,
    ) -> Result<()> {
        let layer = self
            .get_layer(layer_id)
            .ok_or_else(|| anyhow!("layer {} doesn't exists", layer_id))?;
        let tile_data = layer.readback(device, queue, tiles).await?;

        for (tile, data) in tile_data {
            Self::write_tile_data(archive, layer_id, tile, &data)?;
        }

        Ok(())
    }

    pub(crate) fn write_tile_data(
        archive: &LazuliArchive,
        layer_id: LayerId,
        tile: IVec2,
        data: &[u8],
    ) -> Result<()> {
        let data = Self::encode_tile_data(data)?;
        archive.write_tile_data(layer_id.into_inner(), tile.x, tile.y, data)?;
        Ok(())
    }

    pub fn encode_tile_data(data: &[u8]) -> Result<Vec<u8>> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data)?;
        Ok(encoder.finish()?)
    }

    pub fn read_tiles(
        &self,
        queue: &Queue,
        archive: &LazuliArchive,
        layer_id: Uuid,
        tiles: Vec<IVec2>,
    ) -> Result<()> {
        let mut layer = self
            .get_layer_mut(LayerId(layer_id))
            .ok_or_else(|| anyhow!("layer {} doesn't exists", layer_id))?;

        layer.allocate_tiles_batch(tiles.iter().copied());

        for index in tiles {
            let tile = archive
                .read_tile_data(layer_id, index.x, index.y)?
                .ok_or_else(|| anyhow!("tile ({}, {}) not found", index.x, index.y))?;
            let mut d = DeflateDecoder::new(&tile[..]);
            let mut buf = Vec::new();
            d.read_to_end(&mut buf)?;
            layer.write_raw(queue, index, &buf);
        }

        Ok(())
    }
}

impl Serialize for LayerProperties {
    fn serialize<S>(&self, serializer: S) -> result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let encoded = self
            .iter()
            .map(|(k, v)| Ok((k, v.encode()?)))
            .collect::<Result<HashMap<_, _>>>()
            .map_err(<S::Error as ser::Error>::custom)?;
        encoded.serialize(serializer)
    }
}

impl LayerProperties {
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(rmp_serde::to_vec(self)?)
    }
}

impl LayerStack {
    pub fn write_entire_tree(&self, archive: &LazuliArchive) -> Result<()> {
        for layer in self.iter_layers() {
            let (parent_id, sort_order) = match layer.parent() {
                Some(parent_id) => {
                    let parent = self.get_layer(parent_id).unwrap();
                    let sort_order = parent.child_index(layer.id()).unwrap() as u32;
                    (Some(**parent.id()), Some(sort_order))
                }
                None => (None, None),
            };

            archive.write_layer_node(&LayerNode {
                id: **layer.id(),
                parent_id,
                sort_order,
                layer_type: layer.instance().layer_type(),
                properties: layer.properties().encode()?,
            })?;
        }
        Ok(())
    }

    pub fn read_entire_tree(
        root_layer: Uuid,
        archive: &LazuliArchive,
        layer_types: &LayerTypeRegistry,
    ) -> Result<Self> {
        let mut nodes = archive.read_all_layer_nodes()?;
        let root_index = nodes
            .iter()
            .position(|node| node.id == root_layer)
            .ok_or_else(|| anyhow!("root layer {} not found", root_layer))?;
        let root_node = nodes.swap_remove(root_index);
        if root_node.parent_id.is_some() || root_node.sort_order.is_some() {
            bail!(
                "root layer {} must not have a parent or sort order",
                root_layer
            );
        }

        let mut children_by_parent = HashMap::<Uuid, Vec<LayerNode>>::new();
        for node in nodes {
            let parent_id = node.parent_id.ok_or_else(|| {
                anyhow!(
                    "layer {} has no parent but is not the image root {}",
                    node.id,
                    root_layer
                )
            })?;
            children_by_parent.entry(parent_id).or_default().push(node);
        }

        fn read_children(
            parent_id: Uuid,
            children_by_parent: &mut HashMap<Uuid, Vec<LayerNode>>,
            output: &mut LayerStack,
            layer_types: &LayerTypeRegistry,
        ) -> Result<()> {
            let Some(mut children) = children_by_parent.remove(&parent_id) else {
                return Ok(());
            };
            children.sort_by_key(|node| node.sort_order);

            for (expected_order, node) in children.into_iter().enumerate() {
                let sort_order = node
                    .sort_order
                    .ok_or_else(|| anyhow!("layer {} has no sort order", node.id))?;
                if sort_order as usize != expected_order {
                    bail!(
                        "layer {} has sort order {}, expected {} under parent {}",
                        node.id,
                        sort_order,
                        expected_order,
                        parent_id
                    );
                }

                let layer = read_node(&node, layer_types)?;
                output.add_layer(LayerId(parent_id), expected_order, layer);
                read_children(node.id, children_by_parent, output, layer_types)?;
            }

            Ok(())
        }

        fn read_node(node: &LayerNode, layer_types: &LayerTypeRegistry) -> Result<LayerStackNode> {
            let instance = layer_types
                .create_default(node.layer_type)
                .ok_or_else(|| anyhow!("Unknown layer type: {}", node.layer_type))?;
            let encoded = EncodedLayerProperties::from_slice(&node.properties)?;
            let props =
                LayerProperties::new(layer_types.decode_properties(node.layer_type, encoded)?);
            let root_node = LayerStackNode::without_parent(LayerId(node.id), instance, props);
            Ok(root_node)
        }

        let root_node = read_node(&root_node, layer_types)?;
        let mut output = LayerStack::new(root_node);

        read_children(
            root_layer,
            &mut children_by_parent,
            &mut output,
            layer_types,
        )?;
        if let Some((parent_id, children)) = children_by_parent.iter().next() {
            bail!(
                "layer {} references parent {}, which is not reachable from root {}",
                children[0].id,
                parent_id,
                root_layer
            );
        }

        Ok(output)
    }
}
