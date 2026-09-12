use std::{
    collections::HashMap,
    io::{Read, Write},
};

use anyhow::{Result, anyhow, bail};
use flate2::{Compression, read::DeflateDecoder, write::DeflateEncoder};
use glam::{IVec2, UVec2};
use lapiz_lazuli::{ImageProperties, LayerNode, LazuliArchive};
use lapiz_render::render_context::RenderContextAppExt;
use lapiz_runtime::Services;
use moxcms::ColorProfile;
use serde::Serialize;
use uuid::Uuid;
use wgpu::{Device, Queue};

use crate::{
    CImage,
    layer::{
        Layer, LayerId, LayerStack, LayerStackNode, LayerTypeRegistry, SpecialLayers,
        properties::{
            EncodedLayerProperties, LayerProperties, LayerTexelTypePropertyExt, TexelSource,
        },
    },
    texel::TexelType,
    tile::{GpuLayerInfo, GpuTileStorage, TileStorageAppExt},
};

impl CImage {
    pub fn read_archive(archive: &LazuliArchive, services: &Services) -> Result<Self> {
        let image_props = archive.read_image_properties()?;
        let layer_stack = LayerStack::read_entire_tree(
            image_props.root_layer,
            archive,
            services.service::<LayerTypeRegistry>(),
        )?;

        let queue = services.render_queue();
        let tile_storage = services.tile_storage();
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

    pub async fn write_archive(&self, archive: &LazuliArchive, services: &Services) -> Result<()> {
        archive.write_image_properties(&ImageProperties {
            width: self.size.x,
            height: self.size.y,
            tile_size: GpuTileStorage::TILE_SIZE,
            color_profile: self.profile.encode()?,
            root_layer: **self.layers.root_node().id(),
            texel_type: self.texel_type.encode(),
        })?;

        self.layers.write_entire_tree(archive)?;
        let render_context = services.render_context();
        let tile_storage = services.tile_storage();

        let result = futures::future::join_all(
            self.layers
                .iter_layers()
                .filter(|layer| {
                    layer
                        .properties()
                        .get_texel_prop()
                        .is_some_and(|p| p.source == TexelSource::DirectlyDefined)
                })
                .map(|layer| {
                    tile_storage.write_layer(
                        &render_context.device,
                        &render_context.queue,
                        archive,
                        *layer.id(),
                    )
                }),
        )
        .await;
        for r in result {
            r?;
        }

        Ok(())
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

    fn write_tile_data(
        archive: &LazuliArchive,
        layer_id: LayerId,
        tile: IVec2,
        data: &[u8],
    ) -> Result<()> {
        let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
        e.write_all(data)?;
        let buf = e.finish()?;
        archive.write_tile_data(layer_id.into_inner(), tile.x, tile.y, buf)?;
        Ok(())
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
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let encoded = self
            .iter()
            .map(|(k, v)| Ok((k, v.encode()?)))
            .collect::<Result<HashMap<_, _>>>()
            .map_err(<S::Error as serde::ser::Error>::custom)?;
        encoded.serialize(serializer)
    }
}

impl LayerProperties {
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(rmp_serde::to_vec(self)?)
    }

    pub fn decode(data: &[u8], layer: &dyn Layer) -> Result<Self> {
        Self::new_decoded(EncodedLayerProperties::new(data)?, layer)
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
                .get_cloned(node.layer_type)
                .ok_or(anyhow::anyhow!("Unknown layer type: {}", node.layer_type))?;
            let props = LayerProperties::decode(&node.properties, instance.as_ref())?;
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
