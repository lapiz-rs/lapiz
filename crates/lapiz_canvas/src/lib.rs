use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::Result;
use indexmap::IndexSet;
use lapiz_file_dialog::LocalFile;
use lapiz_image::{
    CImage,
    layer::{LayerId, LayerStackNode},
};
use lapiz_lazuli::LazuliArchive;
use lapiz_runtime::{Application, Services, event::Event as _, plugin::Plugin, service::Service};
use lapiz_tools::{ToolProxies, ToolProxy, ToolsAppExt as _};
use lapiz_undo::{QueuedUndoCommand, UndoCommand, UndoStack, UndoStacks};
use lapiz_utils::wrapper;
use parse_display::Display;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    control::CanvasTransform,
    event::{CanvasActiveLayerChanged, CanvasCreated},
    tools::{PanTool, RotateTool, ZoomTool},
};

pub mod command;
pub mod control;
pub mod event;
pub mod recent;
pub mod render;
pub mod resource;
pub mod tools;
pub mod widget;

wrapper! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display)]
    #[display("{0}")]
    pub CanvasId : Uuid
}

pub struct CCanvas {
    id: CanvasId,
    pub image: CImage,
    pub archive: LazuliArchive,
    local_file: Arc<LocalFile>,
    pub transform: CanvasTransform,
    active_layer: LayerId,
    selected_layers: IndexSet<LayerId>,
}

impl CCanvas {
    pub fn new(local_file: LocalFile, image: CImage, archive: LazuliArchive) -> Self {
        let background_layer = *image
            .layer_stack()
            .root_node()
            .children()
            .first()
            .expect("Root layer should have at least one child");

        Self {
            id: CanvasId::new(Uuid::new_v4()),
            local_file: Arc::new(local_file),
            image,
            archive,
            transform: CanvasTransform::default(),
            active_layer: background_layer,
            selected_layers: IndexSet::from([background_layer]),
        }
    }

    pub fn id(&self) -> CanvasId {
        self.id
    }

    pub fn file_path(&self) -> &Path {
        self.local_file.path()
    }

    pub fn local_file(&self) -> &Arc<LocalFile> {
        &self.local_file
    }

    pub fn set_file_path(&mut self, local_file: LocalFile) -> Result<()> {
        self.archive.set_path(local_file.path().to_path_buf())?;
        self.local_file = Arc::new(local_file);
        Ok(())
    }

    pub fn set_active_layer(&mut self, layer_id: LayerId) {
        if layer_id == self.active_layer || layer_id == *self.image.layer_stack().root_id() {
            return;
        }
        let old = self.active_layer;
        self.active_layer = layer_id;
        self.selected_layers.insert(layer_id);
        CanvasActiveLayerChanged::broadcast(CanvasActiveLayerChanged {
            from: old,
            to: layer_id,
        });
    }

    pub fn set_active_layer_and_clear_select(&mut self, layer_id: LayerId) {
        self.selected_layers.clear();
        self.set_active_layer(layer_id);
    }

    pub fn select_layer(&mut self, layer_id: LayerId) {
        self.selected_layers.insert(layer_id);
    }

    pub fn deselect_layer(&mut self, layer_id: LayerId) {
        if self.selected_layers.contains(&layer_id) && self.selected_layers.len() == 1 {
            return;
        }

        self.selected_layers.shift_remove(&layer_id);
        if self.active_layer == layer_id {
            self.set_active_layer(
                self.selected_layers
                    .first()
                    .copied()
                    .expect("A selected layer should remain"),
            );
        }
    }

    pub fn toggle_layer_selection_and_active(&mut self, layer_id: LayerId) {
        if self.selected_layers.contains(&layer_id) {
            self.deselect_layer(layer_id);
        } else {
            self.set_active_layer(layer_id);
        }
    }

    pub fn toggle_layer_selection(&mut self, layer_id: LayerId) {
        if self.selected_layers.contains(&layer_id) {
            self.deselect_layer(layer_id);
        } else {
            self.select_layer(layer_id);
        }
    }

    pub fn selected_layer_ids(&self) -> &IndexSet<LayerId> {
        &self.selected_layers
    }

    pub fn active_layer_id(&self) -> LayerId {
        self.active_layer
    }

    pub fn active_layer_node(&self) -> &LayerStackNode {
        self.image
            .layer_stack()
            .get_layer(&self.active_layer)
            .expect("Active layer should always exist")
    }

    pub fn active_layer_node_mut(&mut self) -> &mut LayerStackNode {
        self.image
            .layer_stack_mut()
            .get_layer_mut(&self.active_layer)
            .expect("Active layer should always exist")
    }

    pub fn parent_id_of_active_layer(&self) -> LayerId {
        let layer = self
            .image
            .layer_stack()
            .get_layer(&self.active_layer)
            .expect("Active layer should always exist");
        *layer
            .parent()
            .expect("Active layer should always have a parent")
    }
}

pub struct CanvasPlugin;

impl Plugin for CanvasPlugin {
    fn build(&self, app: &mut Application) {
        app.add_service::<CanvasManager>();
        let mut runtime = app.runtime_mut();
        runtime
            .services_mut()
            .add_tool_function::<PanTool>()
            .add_tool_function::<RotateTool>()
            .add_tool_function::<ZoomTool>();
    }
}

#[derive(Default)]
pub struct CanvasManager {
    canvases: HashMap<CanvasId, CCanvas>,
    current_canvas: Option<CanvasId>,
}

impl Service for CanvasManager {}

impl CanvasManager {
    pub fn add_canvas(&mut self, canvas: CCanvas) -> CanvasId {
        let id = canvas.id;
        self.current_canvas = Some(id);
        self.canvases.insert(id, canvas);
        CanvasCreated::broadcast(CanvasCreated { id });
        id
    }

    pub fn remove_canvas(&mut self, id: &CanvasId) -> Option<CCanvas> {
        let removed = self.canvases.remove(id)?;
        if self.current_canvas.as_ref() == Some(id) {
            self.current_canvas = self.canvases.keys().next().copied();
        }
        Some(removed)
    }

    pub fn get(&self, id: &CanvasId) -> Option<&CCanvas> {
        self.canvases.get(id)
    }

    pub fn get_mut(&mut self, id: &CanvasId) -> Option<&mut CCanvas> {
        self.canvases.get_mut(id)
    }

    pub fn current(&self) -> Option<&CCanvas> {
        self.get(&self.current_canvas?)
    }

    pub fn current_mut(&mut self) -> Option<&mut CCanvas> {
        self.get_mut(&self.current_canvas?)
    }

    pub fn current_id(&self) -> Option<CanvasId> {
        self.current_canvas
    }

    pub fn set_current(&mut self, id: CanvasId) {
        assert!(self.canvases.contains_key(&id), "Canvas should exist");
        self.current_canvas = Some(id);
    }
}

pub trait CanvasAppExt {
    fn add_canvas(&mut self, canvas: CCanvas) -> CanvasId;
    fn remove_canvas(&mut self, id: &CanvasId) -> Option<CCanvas>;
    fn current_canvas_id(&self) -> Option<CanvasId>;
    fn current_canvas(&self) -> Option<&CCanvas>;
    fn current_canvas_mut(&mut self) -> Option<&mut CCanvas>;
    fn canvas(&self, id: &CanvasId) -> Option<&CCanvas>;
    fn canvas_mut(&mut self, id: &CanvasId) -> Option<&mut CCanvas>;
    fn update_current_canvas<R>(
        &mut self,
        update: impl FnOnce(&mut CCanvas, &mut Services) -> R,
    ) -> Option<R>;
    fn update_canvas<R>(
        &mut self,
        id: &CanvasId,
        update: impl FnOnce(&mut CCanvas, &mut Services) -> R,
    ) -> Option<R>;
    fn set_current_canvas(&mut self, id: CanvasId);
}

impl CanvasAppExt for Services {
    fn add_canvas(&mut self, canvas: CCanvas) -> CanvasId {
        self.service_mut::<CanvasManager>().add_canvas(canvas)
    }

    fn remove_canvas(&mut self, id: &CanvasId) -> Option<CCanvas> {
        self.service_mut::<CanvasManager>().remove_canvas(id)
    }

    fn current_canvas_id(&self) -> Option<CanvasId> {
        self.service::<CanvasManager>().current_id()
    }

    fn current_canvas(&self) -> Option<&CCanvas> {
        self.service::<CanvasManager>().current()
    }

    fn current_canvas_mut(&mut self) -> Option<&mut CCanvas> {
        self.service_mut::<CanvasManager>().current_mut()
    }

    fn canvas(&self, id: &CanvasId) -> Option<&CCanvas> {
        self.service::<CanvasManager>().get(id)
    }

    fn canvas_mut(&mut self, id: &CanvasId) -> Option<&mut CCanvas> {
        self.service_mut::<CanvasManager>().get_mut(id)
    }

    fn update_current_canvas<R>(
        &mut self,
        update: impl FnOnce(&mut CCanvas, &mut Services) -> R,
    ) -> Option<R> {
        let id = self.current_canvas_id()?;
        self.update_canvas(&id, update)
    }

    fn update_canvas<R>(
        &mut self,
        id: &CanvasId,
        update: impl FnOnce(&mut CCanvas, &mut Services) -> R,
    ) -> Option<R> {
        self.service_scope::<CanvasManager, _>(|manager, services| {
            let canvas = manager.get_mut(id)?;
            Some(update(canvas, services))
        })
    }

    fn set_current_canvas(&mut self, id: CanvasId) {
        self.service_mut::<CanvasManager>().set_current(id);
    }
}

pub trait CanvasUndoStackAppExt {
    fn current_canvas_undo_stack(&self) -> Option<&UndoStack>;
    fn current_canvas_undo_stack_mut(&mut self) -> Option<&mut UndoStack>;
    fn undo_stack(&self, id: &CanvasId) -> Option<&UndoStack>;
    fn undo_stack_mut(&mut self, id: &CanvasId) -> Option<&mut UndoStack>;
    fn push_undo_command_to_current<C: UndoCommand>(&mut self, command: C) -> anyhow::Result<()>;
    fn push_undo_command<C: UndoCommand>(
        &mut self,
        id: &CanvasId,
        command: C,
    ) -> anyhow::Result<()>;
    fn push_undo_command_boxed_to_current(
        &mut self,
        command: Box<dyn UndoCommand>,
    ) -> anyhow::Result<()>;
    fn push_undo_command_boxed(
        &mut self,
        id: &CanvasId,
        command: Box<dyn UndoCommand>,
    ) -> anyhow::Result<()>;
    fn queue_undo_command_to_current(&mut self) -> anyhow::Result<QueuedUndoCommand>;
    fn queue_undo_command(&mut self, id: &CanvasId) -> anyhow::Result<QueuedUndoCommand>;
}

impl CanvasUndoStackAppExt for Services {
    fn current_canvas_undo_stack(&self) -> Option<&UndoStack> {
        self.undo_stack(&self.current_canvas_id()?)
    }

    fn current_canvas_undo_stack_mut(&mut self) -> Option<&mut UndoStack> {
        self.undo_stack_mut(&self.current_canvas_id()?)
    }

    fn undo_stack(&self, id: &CanvasId) -> Option<&UndoStack> {
        self.service::<UndoStacks>().get(&**id)
    }

    fn undo_stack_mut(&mut self, id: &CanvasId) -> Option<&mut UndoStack> {
        self.service_mut::<UndoStacks>().get_mut(&**id)
    }

    fn push_undo_command_to_current<C: UndoCommand>(&mut self, command: C) -> anyhow::Result<()> {
        self.push_undo_command_boxed_to_current(Box::new(command))
    }

    fn push_undo_command<C: UndoCommand>(
        &mut self,
        id: &CanvasId,
        command: C,
    ) -> anyhow::Result<()> {
        self.push_undo_command_boxed(id, Box::new(command))
    }

    fn push_undo_command_boxed_to_current(
        &mut self,
        command: Box<dyn UndoCommand>,
    ) -> anyhow::Result<()> {
        let id = self
            .current_canvas_id()
            .ok_or_else(|| anyhow::anyhow!("No current canvas"))?;
        self.push_undo_command_boxed(&id, command)
    }

    fn push_undo_command_boxed(
        &mut self,
        id: &CanvasId,
        command: Box<dyn UndoCommand>,
    ) -> anyhow::Result<()> {
        self.service_scope::<UndoStacks, _>(|stacks, services| {
            stacks
                .get_mut(&**id)
                .ok_or_else(|| anyhow::anyhow!("Undo stack for canvas {} not found", id))?
                .push_boxed(command, services)
        })
    }

    fn queue_undo_command_to_current(&mut self) -> anyhow::Result<QueuedUndoCommand> {
        let id = self
            .current_canvas_id()
            .ok_or_else(|| anyhow::anyhow!("No current canvas"))?;
        self.queue_undo_command(&id)
    }

    fn queue_undo_command(&mut self, id: &CanvasId) -> anyhow::Result<QueuedUndoCommand> {
        self.service_mut::<UndoStacks>()
            .get_mut(&**id)
            .ok_or_else(|| anyhow::anyhow!("Undo stack for canvas {} not found", id))
            .map(UndoStack::queue)
    }
}

pub trait CanvasToolProxyAppExt {
    fn current_tool_proxy(&self) -> Option<&ToolProxy>;
    fn tool_proxy(&self, canvas_id: &CanvasId) -> Option<&ToolProxy>;
    fn current_tool_proxy_mut(&mut self) -> Option<&mut ToolProxy>;
    fn tool_proxy_mut(&mut self, canvas_id: &CanvasId) -> Option<&mut ToolProxy>;
    fn update_current_tool_proxy<R>(
        &mut self,
        f: impl FnOnce(&mut ToolProxy, &mut Services) -> R,
    ) -> Option<R>;
    fn update_tool_proxy<R>(
        &mut self,
        canvas_id: &CanvasId,
        f: impl FnOnce(&mut ToolProxy, &mut Services) -> R,
    ) -> Option<R>;
}

impl CanvasToolProxyAppExt for Services {
    fn current_tool_proxy(&self) -> Option<&ToolProxy> {
        self.tool_proxy(&self.current_canvas_id()?)
    }

    fn tool_proxy(&self, canvas_id: &CanvasId) -> Option<&ToolProxy> {
        self.service::<ToolProxies>().get(canvas_id)
    }

    fn current_tool_proxy_mut(&mut self) -> Option<&mut ToolProxy> {
        self.tool_proxy_mut(&self.current_canvas_id()?)
    }

    fn tool_proxy_mut(&mut self, canvas_id: &CanvasId) -> Option<&mut ToolProxy> {
        self.service_mut::<ToolProxies>().get_mut(canvas_id)
    }

    fn update_current_tool_proxy<R>(
        &mut self,
        f: impl FnOnce(&mut ToolProxy, &mut Services) -> R,
    ) -> Option<R> {
        self.update_tool_proxy(&self.current_canvas_id()?, f)
    }

    fn update_tool_proxy<R>(
        &mut self,
        canvas_id: &CanvasId,
        f: impl FnOnce(&mut ToolProxy, &mut Services) -> R,
    ) -> Option<R> {
        self.service_scope::<ToolProxies, Option<R>>(|proxies, services| {
            Some(f(proxies.get_mut(canvas_id)?, services))
        })
    }
}
