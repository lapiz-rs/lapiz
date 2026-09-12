use std::{borrow::Cow, collections::HashMap};

use anyhow::bail;
use bevy_math::IRect;
use glam::IVec2;
use indexmap::IndexSet;
use lapiz_image::{
    layer::{
        LayerId, LayerPosition, LayerStackNode,
        properties::{LayerProperties, LayerTexelTypePropertyExt},
    },
    tile::{DynamicLayerStorage, GpuLayerInfo, GpuTileStorage, TileStorageAppExt},
};
use lapiz_render::render_context::RenderContextAppExt;
use lapiz_runtime::{Services, event::Event};
use lapiz_undo::UndoCommand;
use lapiz_utils::log_err::LogErr;
use wgpu::{
    Device, Extent3d, ImageSubresourceRange, Origin3d, Queue, TexelCopyTextureInfo, Texture,
    TextureAspect, TextureDescriptor, TextureDimension, TextureUsages,
};

use crate::{CCanvas, CanvasAppExt, CanvasId, event::CanvasUpdated};

pub struct TileReplaceCommand {
    pub reason: Cow<'static, str>,
    pub canvas: CanvasId,
    pub layer: LayerId,
    // The final update rect on undo/redo would be the union of these two.
    // When replacing tiles with one source, tiles only exist in another source will be cleared.
    pub old_tiles: Option<(Texture, Vec<IVec2>)>,
    pub new_tiles: Option<(Texture, Vec<IVec2>)>,
}

impl TileReplaceCommand {
    pub fn new(
        reason: Cow<'static, str>,
        canvas: CanvasId,
        device: &Device,
        queue: &Queue,
        layer_id: LayerId,
        layer_storage: &DynamicLayerStorage,
        // TODO accept Option
        new_tile_indices: Vec<IVec2>,
        new_tiles: Texture,
    ) -> Self {
        let old_tiles = layer_storage.texture().and_then(|layer_texture| {
            let old_tile_indices = new_tile_indices
                .iter()
                .copied()
                .filter(|i| layer_storage.get_tile(*i).is_some())
                .collect::<Vec<_>>();
            if old_tile_indices.is_empty() {
                return None;
            }
            let old_texture = device.create_texture(&TextureDescriptor {
                label: Some("old_texture"),
                size: Extent3d {
                    width: GpuTileStorage::TILE_SIZE,
                    height: GpuTileStorage::TILE_SIZE,
                    depth_or_array_layers: old_tile_indices.len() as u32,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: new_tiles.format(),
                usage: TextureUsages::COPY_DST | TextureUsages::COPY_SRC,
                view_formats: &[],
            });

            let mut ec = device.create_command_encoder(&Default::default());

            ec.push_debug_group("copy_old_tiles");
            for (dst_layer, index) in old_tile_indices.iter().enumerate() {
                let src_layer = layer_storage.get_tile_layer(*index).unwrap();
                ec.copy_texture_to_texture(
                    TexelCopyTextureInfo {
                        texture: layer_texture,
                        mip_level: 0,
                        origin: Origin3d {
                            x: 0,
                            y: 0,
                            z: src_layer,
                        },
                        aspect: TextureAspect::All,
                    },
                    TexelCopyTextureInfo {
                        texture: &old_texture,
                        mip_level: 0,
                        origin: Origin3d {
                            x: 0,
                            y: 0,
                            z: dst_layer as u32,
                        },
                        aspect: TextureAspect::All,
                    },
                    GpuTileStorage::TILE_COPY_SIZE,
                );
            }
            ec.pop_debug_group();

            queue.submit([ec.finish()]);

            Some((old_texture, old_tile_indices))
        });

        Self {
            reason,
            canvas,
            layer: layer_id,
            old_tiles,
            new_tiles: Some((new_tiles, new_tile_indices)),
        }
    }

    pub fn new_clear(
        reason: Cow<'static, str>,
        canvas: CanvasId,
        device: &Device,
        queue: &Queue,
        layer_id: LayerId,
        layer_storage: &DynamicLayerStorage,
    ) -> Self {
        let old_tiles = layer_storage.texture().map(|layer_texture| {
            let old_texture = device.create_texture(&TextureDescriptor {
                label: Some("old_texture"),
                size: layer_texture.size(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: layer_texture.format(),
                usage: TextureUsages::COPY_DST | TextureUsages::COPY_SRC,
                view_formats: &[],
            });

            let mut ec = device.create_command_encoder(&Default::default());

            ec.push_debug_group("copy_old_tiles");
            ec.copy_texture_to_texture(
                layer_texture.as_image_copy(),
                old_texture.as_image_copy(),
                old_texture.size(),
            );
            ec.pop_debug_group();

            queue.submit([ec.finish()]);

            (
                old_texture,
                layer_storage.iter_tiles().map(|(i, _, _)| i).collect(),
            )
        });

        Self {
            reason,
            canvas,
            layer: layer_id,
            old_tiles,
            new_tiles: None,
        }
    }
}

fn apply_tile_replace(
    services: &mut Services,
    canvas: CanvasId,
    layer: LayerId,
    replace_tile: &Option<(Texture, Vec<IVec2>)>,
    clear_tile_indices: Vec<IVec2>,
) {
    let mut dirty_min = IVec2::MAX;
    let mut dirty_max = IVec2::MIN;

    let device = services.render_device();
    let queue = services.render_queue();

    let tile_storage = services.tile_storage();
    let mut layer = tile_storage.get_layer_mut(layer).unwrap();

    let mut ec = device.create_command_encoder(&Default::default());

    if let Some((tiles, tile_indices)) = replace_tile {
        layer.allocate_tiles_batch(tile_indices);

        let layer_texture = layer.texture().unwrap();

        ec.push_debug_group("replace_old_with_new");
        for (src_layer, tile_index) in tile_indices.iter().enumerate() {
            let dst_layer = layer.get_tile_layer(*tile_index).unwrap();
            ec.copy_texture_to_texture(
                TexelCopyTextureInfo {
                    texture: tiles,
                    mip_level: 0,
                    origin: Origin3d {
                        x: 0,
                        y: 0,
                        z: src_layer as u32,
                    },
                    aspect: TextureAspect::All,
                },
                TexelCopyTextureInfo {
                    texture: layer_texture,
                    mip_level: 0,
                    origin: Origin3d {
                        x: 0,
                        y: 0,
                        z: dst_layer,
                    },
                    aspect: TextureAspect::All,
                },
                GpuTileStorage::TILE_COPY_SIZE,
            );

            dirty_min = dirty_min.min(*tile_index);
            dirty_max = dirty_max.max(*tile_index);
        }
        ec.pop_debug_group();
    }

    ec.push_debug_group("clear_old_without_new");
    for tile_index in clear_tile_indices {
        let Some(dst_layer) = layer.get_tile_layer(tile_index) else {
            continue;
        };

        ec.clear_texture(
            layer.texture_view().unwrap().texture(),
            &ImageSubresourceRange {
                aspect: TextureAspect::All,
                base_mip_level: 0,
                mip_level_count: None,
                base_array_layer: dst_layer,
                array_layer_count: Some(1),
            },
        );

        dirty_min = dirty_min.min(tile_index);
        dirty_max = dirty_max.max(tile_index);
    }
    ec.pop_debug_group();

    queue.submit([ec.finish()]);

    CanvasUpdated::broadcast(CanvasUpdated {
        id: canvas,
        dirty_tiles: IRect {
            min: dirty_min,
            max: dirty_max + 1,
        },
    });
}

impl UndoCommand for TileReplaceCommand {
    fn label(&self) -> Cow<'static, str> {
        self.reason.clone()
    }

    #[tracing::instrument(skip_all)]
    fn redo(&mut self, services: &mut Services) -> anyhow::Result<()> {
        let to_clear = self
            .old_tiles
            .as_ref()
            .map(|(_, i)| match &self.new_tiles {
                Some(new_tiles) => i
                    .iter()
                    .copied()
                    .filter(|old| !new_tiles.1.contains(old))
                    .collect::<Vec<_>>(),
                None => i.clone(),
            })
            .unwrap_or_default();

        apply_tile_replace(services, self.canvas, self.layer, &self.new_tiles, to_clear);
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn undo(&mut self, services: &mut Services) -> anyhow::Result<()> {
        let to_clear = self
            .new_tiles
            .as_ref()
            .map(|(_, i)| match &self.old_tiles {
                Some(old_tiles) => i
                    .iter()
                    .copied()
                    .filter(|new| !old_tiles.1.contains(new))
                    .collect::<Vec<_>>(),
                None => i.clone(),
            })
            .unwrap_or_default();

        apply_tile_replace(services, self.canvas, self.layer, &self.old_tiles, to_clear);
        Ok(())
    }
}

pub struct InsertLayerCommand {
    canvas: CanvasId,
    layer: LayerStackNode,
    parent_id: LayerId,
    position: LayerPosition,
    previous_active_layer: LayerId,
    previous_selected_layers: IndexSet<LayerId>,
}

impl InsertLayerCommand {
    pub fn new(
        canvas: &CCanvas,
        layer: LayerStackNode,
        parent: LayerId,
        position: impl Into<LayerPosition>,
    ) -> Self {
        let active_layer = canvas.active_layer_id();
        let selected_layers = canvas.selected_layer_ids().clone();

        Self {
            canvas: canvas.id(),
            layer,
            parent_id: parent,
            position: position.into(),
            previous_active_layer: active_layer,
            previous_selected_layers: selected_layers,
        }
    }
}

impl UndoCommand for InsertLayerCommand {
    fn label(&self) -> Cow<'static, str> {
        "Create Layer".into()
    }

    fn redo(&mut self, services: &mut Services) -> anyhow::Result<()> {
        if let Some(texel_type) = self.layer.properties().get_texel_type() {
            services
                .tile_storage()
                .declare_layer(*self.layer.id(), GpuLayerInfo { texel_type });
        }

        services
            .update_canvas(&self.canvas, |canvas, _| {
                canvas.image.layer_stack_mut().add_layer(
                    self.parent_id,
                    self.position,
                    self.layer.clone(),
                );
                CanvasUpdated::broadcast(CanvasUpdated {
                    id: self.canvas,
                    dirty_tiles: canvas.image.image_tile_rect(),
                });
                canvas.set_active_layer_and_clear_select(*self.layer.id());
            })
            .ok_or(anyhow::anyhow!("Canvas {} not found", self.canvas))
            .log_err();

        Ok(())
    }

    fn undo(&mut self, services: &mut Services) -> anyhow::Result<()> {
        services
            .update_canvas(&self.canvas, |canvas, _| {
                canvas
                    .image
                    .layer_stack_mut()
                    .remove_layer_hierarchy(self.layer.id());
                CanvasUpdated::broadcast(CanvasUpdated {
                    id: self.canvas,
                    dirty_tiles: canvas.image.image_tile_rect(),
                });
                canvas.set_active_layer_and_clear_select(self.previous_active_layer);
                for layer_id in &self.previous_selected_layers {
                    canvas.select_layer(*layer_id);
                }
            })
            .ok_or(anyhow::anyhow!("Canvas {} not found", self.canvas))
            .log_err();

        Ok(())
    }
}

pub struct GroupLayerCommand {
    pub canvas: CanvasId,
    pub group: LayerStackNode,
    pub children: Vec<LayerWithPosition>,
    pub parent_id: LayerId,
    pub index: usize,
}

pub struct LayerWithPosition {
    pub id: LayerId,
    pub original_parent: LayerId,
    pub original_above: Option<LayerId>,
}

impl UndoCommand for GroupLayerCommand {
    fn label(&self) -> Cow<'static, str> {
        "Group Layer".into()
    }

    fn redo(&mut self, services: &mut Services) -> anyhow::Result<()> {
        if let Some(texel_type) = self.group.properties().get_texel_type() {
            services
                .tile_storage()
                .declare_layer(*self.group.id(), GpuLayerInfo { texel_type });
        }

        services
            .update_canvas(&self.canvas, |canvas, _| {
                canvas.image.layer_stack_mut().add_layer(
                    self.parent_id,
                    self.index,
                    self.group.clone(),
                );
                for (i, child) in self.children.iter().enumerate() {
                    canvas
                        .image
                        .layer_stack_mut()
                        .move_layer(child.id, *self.group.id(), i);
                }
                CanvasUpdated::broadcast(CanvasUpdated {
                    id: self.canvas,
                    dirty_tiles: canvas.image.image_tile_rect(),
                });
            })
            .ok_or(anyhow::anyhow!("Canvas {} not found", self.canvas))
            .log_err();

        Ok(())
    }

    fn undo(&mut self, services: &mut Services) -> anyhow::Result<()> {
        services
            .update_canvas(&self.canvas, |canvas, _| {
                let mut removed_nodes = canvas
                    .image
                    .layer_stack_mut()
                    .remove_layer_hierarchy(self.group.id());

                removed_nodes.remove(self.group.id()).unwrap();

                for child in &self.children {
                    canvas.image.layer_stack_mut().add_layer(
                        child.original_parent,
                        LayerPosition::Above(child.original_above),
                        removed_nodes.remove(&child.id).unwrap(),
                    );
                }

                assert!(removed_nodes.is_empty());
                CanvasUpdated::broadcast(CanvasUpdated {
                    id: self.canvas,
                    dirty_tiles: canvas.image.image_tile_rect(),
                });
            })
            .ok_or(anyhow::anyhow!("Canvas {} not found", self.canvas))
            .log_err();

        Ok(())
    }
}

pub struct MoveLayersCommand {
    canvas: CanvasId,
    layers: Vec<LayerWithPosition>,
    new_parent: LayerId,
    new_position: LayerPosition,
}

impl MoveLayersCommand {
    pub fn new(
        canvas: &CCanvas,
        layers: impl IntoIterator<Item = LayerId>,
        new_parent: LayerId,
        new_position: impl Into<LayerPosition>,
    ) -> Self {
        let reduced_layers = canvas.image.layer_stack().reduce_ancestors(layers);

        let sorted = canvas
            .image
            .layer_stack()
            .sort_by_depth_and_index(reduced_layers)
            .unwrap();

        let layers = sorted
            .into_iter()
            .map(|l| {
                let parent = canvas.image.layer_stack().get_parent_of(&l).unwrap();
                let above = parent.child_below(&l);
                LayerWithPosition {
                    id: l,
                    original_parent: *parent.id(),
                    original_above: above,
                }
            })
            .collect();

        Self {
            canvas: canvas.id(),
            layers,
            new_parent,
            new_position: new_position.into(),
        }
    }
}

impl UndoCommand for MoveLayersCommand {
    fn label(&self) -> Cow<'static, str> {
        "Move Layer".into()
    }

    fn redo(&mut self, services: &mut Services) -> anyhow::Result<()> {
        services.update_canvas(&self.canvas, |canvas, _| {
            match self.new_position {
                LayerPosition::Above(_) => {
                    for layer in self.layers.iter().rev() {
                        canvas.image.layer_stack_mut().move_layer(
                            layer.id,
                            self.new_parent,
                            self.new_position,
                        );
                    }
                }
                LayerPosition::Absolute(_) | LayerPosition::Below(_) => {
                    for layer in self.layers.iter() {
                        canvas.image.layer_stack_mut().move_layer(
                            layer.id,
                            self.new_parent,
                            self.new_position,
                        );
                    }
                }
            }
            CanvasUpdated::broadcast(CanvasUpdated {
                id: self.canvas,
                dirty_tiles: canvas.image.image_tile_rect(),
            });
        });

        Ok(())
    }

    fn undo(&mut self, services: &mut Services) -> anyhow::Result<()> {
        services.update_canvas(&self.canvas, |canvas, _| {
            match self.new_position {
                LayerPosition::Above(_) => {
                    for layer in self.layers.iter().rev() {
                        canvas.image.layer_stack_mut().move_layer(
                            layer.id,
                            layer.original_parent,
                            LayerPosition::Above(layer.original_above),
                        );
                    }
                }
                LayerPosition::Absolute(_) | LayerPosition::Below(_) => {
                    for layer in self.layers.iter() {
                        canvas.image.layer_stack_mut().move_layer(
                            layer.id,
                            layer.original_parent,
                            LayerPosition::Above(layer.original_above),
                        );
                    }
                }
            }
            CanvasUpdated::broadcast(CanvasUpdated {
                id: self.canvas,
                dirty_tiles: canvas.image.image_tile_rect(),
            });
        });

        Ok(())
    }
}

struct DeletedNode {
    root: LayerId,
    nodes: HashMap<LayerId, LayerStackNode>,
    original_parent: LayerId,
    original_above: Option<LayerId>,
}

pub struct DeleteLayersCommand {
    canvas: CanvasId,
    active_layer_from_to: Option<(LayerId, LayerId)>,
    nodes: Option<Vec<DeletedNode>>,
    delete_roots: Vec<LayerId>,
}

impl DeleteLayersCommand {
    pub fn new(canvas: &CCanvas, layers: Vec<LayerId>) -> anyhow::Result<Self> {
        let filtered_layers = canvas.image.layer_stack().reduce_ancestors(layers);

        // Reject if all layers are going to be deleted, other than the root layer.
        {
            let root = canvas.image.layer_stack().root_node();
            let mut reject = true;
            for child in root.children() {
                if !filtered_layers.contains(child) {
                    reject = false;
                    break;
                }
            }
            if reject {
                return Err(anyhow::anyhow!("No children of root node after deletion."));
            }
        }

        let is_layer_deleted = |layer: &LayerId| {
            if filtered_layers.contains(layer) {
                return true;
            }

            for deleted in &filtered_layers {
                if canvas.image.layer_stack().is_ancestor(deleted, layer) {
                    return true;
                }
            }

            false
        };

        let new_active_layer = if !is_layer_deleted(&canvas.active_layer_id()) {
            None
        } else {
            let mut current = canvas.active_layer_id();
            let mut current_parent = canvas
                .image
                .layer_stack()
                .get_layer(&canvas.parent_id_of_active_layer())
                .unwrap();
            // Find the first non-deleted parent
            while is_layer_deleted(current_parent.id()) {
                current = *current_parent.id();
                current_parent = canvas
                    .image
                    .layer_stack()
                    .get_layer(current_parent.parent().unwrap())
                    .unwrap();
            }

            let new_active_layer = current_parent
                .child_below(&current)
                .or_else(|| current_parent.child_above(&current))
                .unwrap_or(*current_parent.id());

            Some(new_active_layer)
        };

        let sorted_layers = canvas
            .image
            .layer_stack()
            .sort_by_depth_and_index(filtered_layers)
            .unwrap();

        Ok(Self {
            canvas: canvas.id(),
            active_layer_from_to: new_active_layer.map(|new| (canvas.active_layer_id(), new)),
            delete_roots: sorted_layers,
            nodes: None,
        })
    }
}

impl UndoCommand for DeleteLayersCommand {
    fn label(&self) -> Cow<'static, str> {
        "Delete Layers".into()
    }

    #[tracing::instrument(skip_all)]
    fn redo(&mut self, services: &mut Services) -> anyhow::Result<()> {
        services
            .update_canvas(&self.canvas, |canvas, _| {
                if self.nodes.is_some() {
                    bail!("Called redo twice consecutively is not valid")
                }

                let mut nodes = Vec::with_capacity(self.delete_roots.len());

                for root in &self.delete_roots {
                    let parent = canvas.image.layer_stack().get_parent_of(root).unwrap();
                    let parent_id = *parent.id();
                    let above = parent.child_below(root);
                    let deleted = canvas.image.layer_stack_mut().remove_layer_hierarchy(root);
                    nodes.push(DeletedNode {
                        root: *root,
                        nodes: deleted,
                        original_parent: parent_id,
                        original_above: above,
                    });
                }
                self.nodes = Some(nodes);
                CanvasUpdated::broadcast(CanvasUpdated {
                    id: self.canvas,
                    dirty_tiles: canvas.image.image_tile_rect(),
                });

                if let Some((_, new_active)) = self.active_layer_from_to {
                    canvas.set_active_layer(new_active);
                }

                Ok(())
            })
            .expect("Canvas should exist")?;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn undo(&mut self, services: &mut Services) -> anyhow::Result<()> {
        services
            .update_canvas(&self.canvas, |canvas, _| {
                let Some(nodes) = self.nodes.take() else {
                    bail!("Called undo twice consecutively is not valid")
                };

                for node in nodes.into_iter() {
                    canvas.image.layer_stack_mut().add_layer_hierarchy(
                        node.original_parent,
                        LayerPosition::Above(node.original_above),
                        node.root,
                        node.nodes,
                    );
                }
                CanvasUpdated::broadcast(CanvasUpdated {
                    id: self.canvas,
                    dirty_tiles: canvas.image.image_tile_rect(),
                });

                if let Some((old_active, _)) = self.active_layer_from_to {
                    canvas.set_active_layer(old_active);
                }

                Ok(())
            })
            .expect("Canvas should exist")?;

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct LayerPropertyChangeCommand {
    pub canvas: CanvasId,
    pub layer_id: LayerId,
    pub old: LayerProperties,
    pub new: LayerProperties,
}

impl UndoCommand for LayerPropertyChangeCommand {
    fn label(&self) -> Cow<'static, str> {
        "Layer Property Change".into()
    }

    fn redo(&mut self, services: &mut Services) -> anyhow::Result<()> {
        services.update_canvas(&self.canvas, |canvas, _| {
            let layer = canvas
                .image
                .layer_stack_mut()
                .get_layer_mut(&self.layer_id)
                .unwrap();
            *layer.properties_mut() = self.new.clone();
            CanvasUpdated::broadcast(CanvasUpdated {
                id: self.canvas,
                dirty_tiles: canvas.image.image_tile_rect(),
            });
        });

        Ok(())
    }

    fn undo(&mut self, services: &mut Services) -> anyhow::Result<()> {
        services.update_canvas(&self.canvas, |canvas, _| {
            let layer = canvas
                .image
                .layer_stack_mut()
                .get_layer_mut(&self.layer_id)
                .unwrap();
            *layer.properties_mut() = self.old.clone();
            CanvasUpdated::broadcast(CanvasUpdated {
                id: self.canvas,
                dirty_tiles: canvas.image.image_tile_rect(),
            });
        });

        Ok(())
    }
}
