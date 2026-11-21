use std::{borrow::Cow, collections::HashMap, iter::from_fn};

use anyhow::{anyhow, bail};
use bevy_math::IRect;
use futures::future::BoxFuture;
use glam::IVec2;
use indexmap::IndexSet;
use lapiz_image::{
    layer::{
        LayerId, LayerPosition, LayerStackNode,
        properties::{
            LayerProperties,
            builtin::{LayerTexelTypePropertyExt as _, TexelSource},
        },
    },
    tile::{DynamicLayerStorage, GpuLayerInfo, GpuTileStorage, TileStorageAppExt as _},
};
use lapiz_lazuli::layer_tree::LayerNode;
use lapiz_render::{
    readback::readback_buffer_raw_on_submit_async, render_context::RenderContextAppExt as _,
    util::DevicePollExt as _,
};
use lapiz_runtime::{event::Event as _, global::Globals};
use lapiz_undo::UndoCommand;
use lapiz_utils::log_err::LogErr as _;
use wgpu::{
    BufferDescriptor, BufferUsages, Device, Extent3d, ImageSubresourceRange, Origin3d, Queue,
    TexelCopyBufferInfo, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect,
    TextureDescriptor, TextureDimension, TextureUsages,
};

use crate::{CCanvas, CanvasAppExt as _, CanvasId, event::CanvasUpdated};

/// Undoable command that replaces or clears a set of GPU tiles on a target layer and broadcasts [`CanvasUpdated`].
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
    globals: &mut Globals,
    canvas: CanvasId,
    layer: LayerId,
    replace_tile: &Option<(Texture, Vec<IVec2>)>,
    clear_tile_indices: Vec<IVec2>,
) {
    let mut dirty_min = IVec2::MAX;
    let mut dirty_max = IVec2::MIN;

    let device = globals.render_device();
    let queue = globals.render_queue();

    let tile_storage = globals.tile_storage();
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

fn apply_archive_tile_replace(
    globals: &Globals,
    canvas: CanvasId,
    layer: LayerId,
    tiles: &Option<(Texture, Vec<IVec2>)>,
    previous: &Option<(Texture, Vec<IVec2>)>,
) -> BoxFuture<'static, anyhow::Result<()>> {
    let archive = globals.canvas(&canvas).map(|canvas| canvas.archive.clone());
    let tiles = tiles.clone();
    let writes = tiles
        .iter()
        .flat_map(|(_, indices)| indices.iter().copied())
        .collect::<IndexSet<_>>();
    let deletes = previous
        .iter()
        .flat_map(|(_, indices)| indices.iter().copied())
        .filter(|index| !writes.contains(index))
        .collect::<Vec<_>>();
    let device = globals.render_device().clone();
    let queue = globals.render_queue().clone();

    Box::pin(async move {
        let archive = archive.ok_or_else(|| anyhow!("Canvas {canvas} not found"))?;
        let mut writes = Vec::new();
        if let Some((texture, indices)) = tiles.filter(|(_, indices)| !indices.is_empty()) {
            let tile_size = GpuTileStorage::TILE_SIZE;
            let bytes_per_row = tile_size
                * texture
                    .format()
                    .block_copy_size(None)
                    .ok_or_else(|| anyhow!("Unsupported tile texture format"))?;
            let mut encoder = device.create_command_encoder(&Default::default());
            let mut readbacks = Vec::with_capacity(indices.len());
            for (index, tile) in indices.into_iter().enumerate() {
                let buffer = device.create_buffer(&BufferDescriptor {
                    label: None,
                    size: u64::from(bytes_per_row) * u64::from(tile_size),
                    usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                encoder.copy_texture_to_buffer(
                    TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: 0,
                        origin: Origin3d {
                            x: 0,
                            y: 0,
                            z: u32::try_from(index)?,
                        },
                        aspect: TextureAspect::All,
                    },
                    TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(bytes_per_row),
                            rows_per_image: Some(tile_size),
                        },
                    },
                    GpuTileStorage::TILE_COPY_SIZE,
                );
                readbacks.push((
                    tile,
                    readback_buffer_raw_on_submit_async(&mut encoder, &buffer, ..),
                ));
            }
            let submission = queue.submit([encoder.finish()]);
            device.poll_indefinitely_for(submission)?;

            for (tile, readback) in readbacks {
                let data = readback.into_inner().await??;
                writes.push((tile, GpuTileStorage::encode_tile_data(&data)?));
            }
        }

        archive.apply_tile_changes(*layer, writes, deletes)
    })
}
impl UndoCommand for TileReplaceCommand {
    fn label(&self) -> Cow<'static, str> {
        self.reason.clone()
    }

    #[tracing::instrument(skip_all)]
    fn redo(&self, globals: &mut Globals) -> anyhow::Result<()> {
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

        apply_tile_replace(globals, self.canvas, self.layer, &self.new_tiles, to_clear);
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn undo(&self, globals: &mut Globals) -> anyhow::Result<()> {
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

        apply_tile_replace(globals, self.canvas, self.layer, &self.old_tiles, to_clear);
        Ok(())
    }

    fn redo_write_archive(&self, globals: &Globals) -> BoxFuture<'static, anyhow::Result<()>> {
        apply_archive_tile_replace(
            globals,
            self.canvas,
            self.layer,
            &self.new_tiles,
            &self.old_tiles,
        )
    }

    fn undo_write_archive(&self, globals: &Globals) -> BoxFuture<'static, anyhow::Result<()>> {
        apply_archive_tile_replace(
            globals,
            self.canvas,
            self.layer,
            &self.old_tiles,
            &self.new_tiles,
        )
    }
}

/// Undoable command that inserts a new [`LayerStackNode`] into a canvas's layer hierarchy and selects it.
pub struct InsertLayerCommand {
    canvas: CanvasId,
    layer: LayerStackNode,
    parent_id: LayerId,
    index: usize,
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
        let parent_node = canvas.image.layer_stack().get_layer(&parent).unwrap();
        let index = parent_node
            .resolve_index(position.into())
            .expect("Layer position must be valid");

        Self {
            canvas: canvas.id(),
            parent_id: parent,
            index,
            layer,
            previous_active_layer: active_layer,
            previous_selected_layers: selected_layers,
        }
    }
}

impl UndoCommand for InsertLayerCommand {
    fn label(&self) -> Cow<'static, str> {
        "Create Layer".into()
    }

    fn redo(&self, globals: &mut Globals) -> anyhow::Result<()> {
        if let Some(texel_type) = self.layer.properties().get_texel_type() {
            globals
                .tile_storage()
                .declare_layer(*self.layer.id(), GpuLayerInfo { texel_type });
        }

        globals
            .update_canvas(&self.canvas, |canvas, _| {
                canvas.image.layer_stack_mut().add_layer(
                    self.parent_id,
                    self.index,
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

    fn undo(&self, globals: &mut Globals) -> anyhow::Result<()> {
        globals
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

/// Undoable command that creates a group layer and moves a set of child layers into it.
pub struct GroupLayerCommand {
    pub canvas: CanvasId,
    pub group: LayerStackNode,
    pub children: Vec<LayerWithPosition>,
    pub parent_id: LayerId,
    pub index: usize,
}

/// Records a layer's original parent and sibling position so layer-reordering commands can be undone.
pub struct LayerWithPosition {
    pub id: LayerId,
    pub original_parent: LayerId,
    pub original_index: usize,
}

impl UndoCommand for GroupLayerCommand {
    fn label(&self) -> Cow<'static, str> {
        "Group Layer".into()
    }

    fn redo(&self, globals: &mut Globals) -> anyhow::Result<()> {
        if let Some(texel_type) = self.group.properties().get_texel_type() {
            globals
                .tile_storage()
                .declare_layer(*self.group.id(), GpuLayerInfo { texel_type });
        }

        globals
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

    fn undo(&self, globals: &mut Globals) -> anyhow::Result<()> {
        globals
            .update_canvas(&self.canvas, |canvas, _| {
                let stack = canvas.image.layer_stack_mut();
                let mut subtrees = self
                    .children
                    .iter()
                    .map(|child| (child, stack.remove_layer_hierarchy(&child.id)))
                    .collect::<Vec<_>>();
                stack.remove_layer_hierarchy(self.group.id());

                subtrees.sort_by_key(|(child, _)| child.original_index);
                for (child, nodes) in subtrees {
                    stack.add_layer_hierarchy(
                        child.original_parent,
                        child.original_index,
                        child.id,
                        nodes,
                    );
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
}

/// Undoable command that reparents or reorders one or more layers within the canvas layer stack.
pub struct MoveLayersCommand {
    canvas: CanvasId,
    layers: Vec<LayerWithPosition>,
    new_parent: LayerId,
    new_index: usize,
}

impl MoveLayersCommand {
    pub fn new(
        canvas: &CCanvas,
        layers: impl IntoIterator<Item = LayerId>,
        new_parent: LayerId,
        new_position: impl Into<LayerPosition>,
    ) -> Self {
        let stack = canvas.image.layer_stack();
        let moved = stack
            .reduce_ancestors(layers)
            .into_iter()
            .collect::<IndexSet<_>>();
        let mut layers = stack
            .iter_layers_dfs_display_order_without_root()
            .map(|(node, _)| *node.id())
            .filter(|id| moved.contains(id))
            .map(|id| {
                let (original_parent, original_index) = stack.get_layer_position(&id).unwrap();
                LayerWithPosition {
                    id,
                    original_parent,
                    original_index,
                }
            })
            .collect::<Vec<_>>();
        layers.reverse();

        let position = new_position.into();
        let new_parent_node = stack.get_layer(&new_parent).unwrap();
        let mut new_index = new_parent_node.resolve_index(position).unwrap();
        if !matches!(position, LayerPosition::Absolute(_)) {
            // Relative positions refer to the list before the moved layers are removed.
            new_index -= layers
                .iter()
                .filter(|layer| {
                    layer.original_parent == new_parent && layer.original_index < new_index
                })
                .count();
        }

        Self {
            canvas: canvas.id(),
            layers,
            new_parent,
            new_index,
        }
    }
}

impl UndoCommand for MoveLayersCommand {
    fn label(&self) -> Cow<'static, str> {
        "Move Layer".into()
    }

    fn redo(&self, globals: &mut Globals) -> anyhow::Result<()> {
        globals
            .update_canvas(&self.canvas, |canvas, _| {
                let stack = canvas.image.layer_stack_mut();
                let parent = stack
                    .get_layer(&self.new_parent)
                    .ok_or_else(|| anyhow!("Destination layer is missing"))?;
                if self.layers.iter().any(|layer| {
                    layer.id == self.new_parent || stack.is_ancestor(&layer.id, &self.new_parent)
                }) {
                    bail!("Cannot move layers into a moved subtree");
                }
                if self.layers.iter().any(|layer| {
                    stack.get_layer_position(&layer.id)
                        != Some((layer.original_parent, layer.original_index))
                }) {
                    bail!("A moved layer is no longer at its original position");
                }
                let removed_children = self
                    .layers
                    .iter()
                    .filter(|layer| layer.original_parent == self.new_parent)
                    .count();
                if self.new_index > parent.n_children() - removed_children {
                    bail!("Layer insertion index is out of bounds");
                }

                let subtrees = self
                    .layers
                    .iter()
                    .map(|layer| (layer.id, stack.remove_layer_hierarchy(&layer.id)))
                    .collect::<Vec<_>>();
                for (offset, (id, nodes)) in subtrees.into_iter().enumerate() {
                    stack.add_layer_hierarchy(self.new_parent, self.new_index + offset, id, nodes);
                }
                CanvasUpdated::broadcast(CanvasUpdated {
                    id: self.canvas,
                    dirty_tiles: canvas.image.image_tile_rect(),
                });
                Ok(())
            })
            .ok_or_else(|| anyhow!("Canvas {} not found", self.canvas))?
    }

    fn undo(&self, globals: &mut Globals) -> anyhow::Result<()> {
        globals.update_canvas(&self.canvas, |canvas, _| {
            let stack = canvas.image.layer_stack_mut();
            let mut subtrees = self
                .layers
                .iter()
                .map(|layer| (layer, stack.remove_layer_hierarchy(&layer.id)))
                .collect::<Vec<_>>();
            subtrees.sort_by_key(|(layer, _)| layer.original_index);
            for (layer, nodes) in subtrees {
                stack.add_layer_hierarchy(
                    layer.original_parent,
                    layer.original_index,
                    layer.id,
                    nodes,
                );
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
    original_index: usize,
}

/// Undoable command that removes one or more layer subtrees from a canvas while preserving at least one child of the root layer.
pub struct DeleteLayersCommand {
    canvas: CanvasId,
    active_layer_from_to: (LayerId, LayerId),
    previous_selected_layers: IndexSet<LayerId>,
    deleted: Vec<DeletedNode>,
}

impl DeleteLayersCommand {
    pub fn new(canvas: &CCanvas, layers: Vec<LayerId>) -> anyhow::Result<Self> {
        let filtered_layers = canvas.image.layer_stack().reduce_ancestors(layers);

        // Reject if all layers are going to be deleted, other than the root layer.
        {
            let root = canvas.image.layer_stack().root_node();
            if filtered_layers.contains(root.id()) {
                bail!("Cannot delete the root layer");
            }
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

        let active_layer = canvas.active_layer_id();
        let new_active_layer = if !is_layer_deleted(&active_layer) {
            active_layer
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

            let index = current_parent.child_index(&current).unwrap();
            let siblings = current_parent.children();
            siblings[..index]
                .iter()
                .rev()
                .chain(&siblings[index + 1..])
                .copied()
                .find(|id| !is_layer_deleted(id))
                .unwrap_or(*current_parent.id())
        };

        let sorted_layers = canvas
            .image
            .layer_stack()
            .sort_by_depth_and_index(filtered_layers)
            .unwrap();

        let layer_stack = canvas.image.layer_stack();
        let deleted = sorted_layers
            .into_iter()
            .map(|root| {
                let parent = layer_stack.get_parent_of(&root).unwrap();
                let mut nodes = HashMap::new();
                let mut pending = vec![root];
                while let Some(id) = pending.pop() {
                    let node = layer_stack.get_layer(&id).unwrap();
                    pending.extend_from_slice(node.children());
                    nodes.insert(id, node.clone());
                }

                DeletedNode {
                    root,
                    nodes,
                    original_parent: *parent.id(),
                    original_index: parent.child_index(&root).unwrap(),
                }
            })
            .collect();

        Ok(Self {
            canvas: canvas.id(),
            active_layer_from_to: (active_layer, new_active_layer),
            previous_selected_layers: canvas.selected_layer_ids().clone(),
            deleted,
        })
    }
}

impl UndoCommand for DeleteLayersCommand {
    fn label(&self) -> Cow<'static, str> {
        "Delete Layers".into()
    }

    #[tracing::instrument(skip_all)]
    fn redo(&self, globals: &mut Globals) -> anyhow::Result<()> {
        globals
            .update_canvas(&self.canvas, |canvas, _| {
                let layer_stack = canvas.image.layer_stack_mut();
                if self
                    .deleted
                    .iter()
                    .any(|node| !layer_stack.contains_layer(&node.root))
                {
                    bail!("Cannot delete layers: a layer is missing")
                }

                for node in &self.deleted {
                    layer_stack.remove_layer_hierarchy(&node.root);
                }

                canvas
                    .selected_layers
                    .retain(|id| layer_stack.contains_layer(id));
                canvas.set_active_layer(self.active_layer_from_to.1);
                CanvasUpdated::broadcast(CanvasUpdated {
                    id: self.canvas,
                    dirty_tiles: canvas.image.image_tile_rect(),
                });

                Ok(())
            })
            .expect("Canvas should exist")?;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn undo(&self, globals: &mut Globals) -> anyhow::Result<()> {
        globals
            .update_canvas(&self.canvas, |canvas, _| {
                let layer_stack = canvas.image.layer_stack_mut();
                if self
                    .deleted
                    .iter()
                    .any(|node| layer_stack.contains_layer(&node.root))
                {
                    bail!("Cannot restore layers: a layer already exists")
                }

                for node in &self.deleted {
                    layer_stack.add_layer_hierarchy(
                        node.original_parent,
                        LayerPosition::Absolute(node.original_index),
                        node.root,
                        node.nodes.clone(),
                    );
                }

                canvas.set_active_layer(self.active_layer_from_to.0);
                canvas.selected_layers = self.previous_selected_layers.clone();
                CanvasUpdated::broadcast(CanvasUpdated {
                    id: self.canvas,
                    dirty_tiles: canvas.image.image_tile_rect(),
                });

                Ok(())
            })
            .expect("Canvas should exist")?;

        Ok(())
    }

    fn redo_write_archive(&self, globals: &Globals) -> BoxFuture<'static, anyhow::Result<()>> {
        let canvas = self.canvas;
        let archive = globals.canvas(&canvas).map(|canvas| canvas.archive.clone());
        let roots = self
            .deleted
            .iter()
            .map(|node| node.root)
            .collect::<Vec<_>>();

        Box::pin(async move {
            let archive = archive.ok_or_else(|| anyhow!("Canvas {canvas} not found"))?;
            for root in roots {
                archive.remove_layer_hierarchy(*root)?;
            }
            Ok(())
        })
    }

    fn undo_write_archive(&self, globals: &Globals) -> BoxFuture<'static, anyhow::Result<()>> {
        let canvas = self.canvas;
        let archive = globals.canvas(&canvas).map(|canvas| canvas.archive.clone());
        let layers = self
            .deleted
            .iter()
            .flat_map(|deleted| {
                let mut pending = vec![(deleted.root, deleted.original_index)];
                from_fn(move || {
                    let (id, index) = pending.pop()?;
                    let node = &deleted.nodes[&id];
                    pending.extend(
                        node.children()
                            .iter()
                            .copied()
                            .enumerate()
                            .rev()
                            .map(|(index, id)| (id, index)),
                    );
                    Some((node, index))
                })
            })
            .map(|(node, index)| {
                let layer = LayerNode {
                    id: **node.id(),
                    parent_id: node.parent().map(|id| **id),
                    sort_order: Some(index as u32),
                    layer_type: node.instance().layer_type(),
                    properties: node.properties().encode()?,
                };
                let has_own_tiles = node
                    .properties()
                    .get_texel_prop()
                    .is_some_and(|p| p.source == TexelSource::DirectlyDefined);
                Ok((layer, has_own_tiles))
            })
            .collect::<anyhow::Result<Vec<_>>>();
        let device = globals.render_device().clone();
        let queue = globals.render_queue().clone();
        let tile_storage = globals.tile_storage().clone();

        Box::pin(async move {
            let archive = archive.ok_or_else(|| anyhow!("Canvas {canvas} not found"))?;
            for (layer, has_own_tiles) in layers? {
                archive.insert_layer_node(&layer)?;
                if has_own_tiles {
                    tile_storage
                        .write_layer(&device, &queue, &archive, LayerId(layer.id))
                        .await?;
                }
            }
            Ok(())
        })
    }
}

/// Undoable command that updates the [`LayerProperties`] of a specific layer.
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

    fn redo(&self, globals: &mut Globals) -> anyhow::Result<()> {
        globals.update_canvas(&self.canvas, |canvas, _| {
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

    fn undo(&self, globals: &mut Globals) -> anyhow::Result<()> {
        globals.update_canvas(&self.canvas, |canvas, _| {
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

#[cfg(test)]
mod test {
    use std::array;

    use {
        crate::CanvasManager,
        glam::UVec2,
        lapiz_image::{
            CImage,
            layer::{LayerTypeRegistry, group_layer::GroupLayer, pixel_layer::PixelLayer},
        },
        lapiz_lazuli::LazuliArchive,
        lapiz_undo::BatchedUndoCommand,
        moxcms::ColorProfile,
    };

    use super::*;

    fn pixel() -> LayerStackNode {
        LayerStackNode::without_parent(
            LayerId::random(),
            Box::new(PixelLayer),
            LayerProperties::new(HashMap::new()),
        )
    }

    fn group() -> LayerStackNode {
        LayerStackNode::without_parent(
            LayerId::random(),
            Box::new(GroupLayer),
            LayerProperties::new(HashMap::new()),
        )
    }

    fn canvas_with_layers<const N: usize>() -> (Globals, LayerId, [LayerId; N]) {
        let mut registry = LayerTypeRegistry::default();
        registry.register::<GroupLayer>();
        let mut image =
            CImage::from_layer(UVec2::ONE, pixel(), ColorProfile::new_srgb(), &registry);
        let stack = image.layer_stack_mut();
        let root = *stack.root_id();
        let layers = array::from_fn(|index| {
            if index == 0 {
                return stack.root_node().children()[0];
            }
            let layer = pixel();
            let id = *layer.id();
            stack.add_layer(root, index, layer);
            id
        });

        let mut globals = Globals::default();
        globals.insert_global(CanvasManager::default());
        globals.add_canvas(CCanvas::new(
            None,
            image,
            LazuliArchive::new_in_memory().unwrap(),
        ));
        (globals, root, layers)
    }

    fn children(globals: &Globals, parent: LayerId) -> &[LayerId] {
        globals
            .current_canvas()
            .unwrap()
            .image
            .layer_stack()
            .get_layer(&parent)
            .unwrap()
            .children()
    }

    #[test]
    fn insert_layer_restores_order_and_selection() {
        let (mut globals, root, [a, b]) = canvas_with_layers();
        globals.current_canvas_mut().unwrap().select_layer(b);
        let first = pixel();
        let second = pixel();
        let first_id = *first.id();
        let second_id = *second.id();
        let canvas = globals.current_canvas().unwrap();
        let command = BatchedUndoCommand::new(
            "Insert Layers".into(),
            vec![
                InsertLayerCommand::new(canvas, first, root, LayerPosition::above(b)),
                InsertLayerCommand::new(canvas, second, root, 3),
            ],
        );

        command.redo(&mut globals).unwrap();
        assert_eq!(children(&globals, root), &[a, b, first_id, second_id]);
        assert_eq!(
            globals.current_canvas().unwrap().active_layer_id(),
            second_id
        );
        command.undo(&mut globals).unwrap();
        assert_eq!(children(&globals, root), &[a, b]);
        let canvas = globals.current_canvas().unwrap();
        assert_eq!(canvas.active_layer_id(), a);
        assert_eq!(canvas.selected_layer_ids().as_slice(), &[a, b][..]);
        command.redo(&mut globals).unwrap();
        assert_eq!(children(&globals, root), &[a, b, first_id, second_id]);
    }

    #[test]
    fn group_layers_restores_non_adjacent_siblings_and_subtrees() {
        let (mut globals, root, [a, b, c]) = canvas_with_layers();
        let nested = group();
        let d = *nested.id();
        let child = pixel();
        let child_id = *child.id();
        let stack = globals
            .current_canvas_mut()
            .unwrap()
            .image
            .layer_stack_mut();
        stack.add_layer(root, 3, nested);
        stack.add_layer(d, 0, child);

        let group = group();
        let id = *group.id();
        let command = GroupLayerCommand {
            canvas: globals.current_canvas_id().unwrap(),
            group,
            children: vec![
                LayerWithPosition {
                    id: b,
                    original_parent: root,
                    original_index: 1,
                },
                LayerWithPosition {
                    id: d,
                    original_parent: root,
                    original_index: 3,
                },
            ],
            parent_id: root,
            index: 1,
        };

        command.redo(&mut globals).unwrap();
        assert_eq!(children(&globals, root), &[a, id, c]);
        assert_eq!(children(&globals, id), &[b, d]);
        command.undo(&mut globals).unwrap();
        assert_eq!(children(&globals, root), &[a, b, c, d]);
        assert_eq!(children(&globals, d), &[child_id]);
        assert!(
            !globals
                .current_canvas()
                .unwrap()
                .image
                .layer_stack()
                .contains_layer(&id)
        );
        command.redo(&mut globals).unwrap();
        assert_eq!(children(&globals, id), &[b, d]);
    }

    #[test]
    fn move_layers_preserves_order_and_restores_original_indices() {
        let (mut globals, root, [a, b, c, d, e]) = canvas_with_layers();
        for (position, expected) in [
            (LayerPosition::above(e), [a, c, e, b, d]),
            (LayerPosition::background(), [b, d, a, c, e]),
        ] {
            let command =
                MoveLayersCommand::new(globals.current_canvas().unwrap(), [d, b], root, position);
            command.redo(&mut globals).unwrap();
            assert_eq!(children(&globals, root), &expected);
            command.undo(&mut globals).unwrap();
            assert_eq!(children(&globals, root), &[a, b, c, d, e]);
            command.redo(&mut globals).unwrap();
            assert_eq!(children(&globals, root), &expected);
            command.undo(&mut globals).unwrap();
        }

        let nested = group();
        let nested_id = *nested.id();
        let child = pixel();
        let child_id = *child.id();
        let canvas = globals.current_canvas_mut().unwrap();
        let stack = canvas.image.layer_stack_mut();
        stack.add_layer(root, 1, nested);
        let sibling = pixel();
        let sibling_id = *sibling.id();
        stack.add_layer(nested_id, 0, sibling);
        stack.add_layer(nested_id, 1, child);
        let command =
            MoveLayersCommand::new(canvas, [b, child_id], root, LayerPosition::foreground());
        command.redo(&mut globals).unwrap();
        assert_eq!(
            children(&globals, root),
            &[a, nested_id, c, d, e, child_id, b]
        );
        assert_eq!(children(&globals, nested_id), &[sibling_id]);
        command.undo(&mut globals).unwrap();
        assert_eq!(children(&globals, root), &[a, nested_id, b, c, d, e]);
        assert_eq!(children(&globals, nested_id), &[sibling_id, child_id]);
    }

    #[test]
    fn delete_layers_restores_subtree_order_and_selection() {
        let (mut globals, root, [a, b]) = canvas_with_layers();
        let group = group();
        let group_id = *group.id();
        let child = pixel();
        let child_id = *child.id();
        let canvas = globals.current_canvas_mut().unwrap();
        let stack = canvas.image.layer_stack_mut();
        stack.add_layer(root, 2, group);
        stack.add_layer(group_id, 0, child);
        canvas.set_active_layer_and_clear_select(child_id);
        canvas.select_layer(b);
        let command = DeleteLayersCommand::new(canvas, vec![b, child_id, group_id]).unwrap();

        command.redo(&mut globals).unwrap();
        let canvas = globals.current_canvas().unwrap();
        assert_eq!(children(&globals, root), &[a]);
        assert!(!canvas.image.layer_stack().contains_layer(&child_id));
        assert_eq!(canvas.active_layer_id(), a);
        assert_eq!(
            canvas
                .selected_layer_ids()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [a]
        );
        command.undo(&mut globals).unwrap();
        let canvas = globals.current_canvas().unwrap();
        assert_eq!(children(&globals, root), &[a, b, group_id]);
        assert_eq!(children(&globals, group_id), &[child_id]);
        assert_eq!(canvas.active_layer_id(), child_id);
        assert_eq!(
            canvas
                .selected_layer_ids()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [child_id, b]
        );
        command.redo(&mut globals).unwrap();
        assert_eq!(children(&globals, root), &[a]);
    }
}
