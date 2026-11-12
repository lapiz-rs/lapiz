use bevy_math::IRect;
use lapiz_image::layer::{LayerId, properties::LayerProperties};
use lapiz_runtime::event::Event;

use crate::CanvasId;

/// Event broadcast when a new [`CCanvas`](crate::CCanvas) is registered in [`CanvasManager`](crate::CanvasManager).
#[derive(Debug, Clone, Event)]
pub struct CanvasCreated {
    pub id: CanvasId,
}

/// Event broadcast when the focused canvas in [`CanvasManager`](crate::CanvasManager) changes.
#[derive(Event, Debug, Clone)]
pub struct CurrentCanvasChanged {
    pub from: Option<CanvasId>,
    pub to: Option<CanvasId>,
}

/// Event broadcast when a canvas is closed and removed from [`CanvasManager`](crate::CanvasManager).
#[derive(Event, Debug, Clone)]
pub struct CanvasRemoved {
    pub id: CanvasId,
}

/// Event broadcast when a region of tiles in a canvas is modified and needs recompositing.
#[derive(Event, Debug, Clone)]
pub struct CanvasUpdated {
    pub id: CanvasId,
    pub dirty_tiles: IRect,
}

/// Event broadcast when the active layer of a canvas changes.
#[derive(Event, Debug, Clone)]
pub struct CanvasActiveLayerChanged {
    pub from: LayerId,
    pub to: LayerId,
}

/// Notification payload describing a change to a layer's [`LayerProperties`].
#[derive(Debug, Clone)]
pub struct CanvasLayerPropertyChanged {
    pub layer_id: LayerId,
    pub old: LayerProperties,
}
