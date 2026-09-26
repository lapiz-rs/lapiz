use crate::core::window::{self, Id};

use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, WindowEvent};

/// A rectangle in the physical coordinate space of the native window.
#[derive(Debug, Clone, Copy)]
pub struct PhysicalBounds {
    pub position: PhysicalPosition<f64>,
    pub size: PhysicalSize<f64>,
}

/// Tracks the pointer state and the interactive window sessions.
pub struct Router {
    /// The logical window that last received a pointer press, used to tag
    /// keyboard events.
    focused: Option<Id>,
    /// The last known pointer position, in the physical coordinate space of
    /// the native window.
    cursor: Option<PhysicalPosition<f64>>,
    /// An interactive window move session started by `window::drag`.
    drag: Option<Drag>,
    /// An interactive window resize session started by
    /// `window::drag_resize` or a press on a window border.
    drag_resize: Option<DragResize>,
}

/// An interactive move session of a logical window.
#[derive(Debug, Clone, Copy)]
struct Drag {
    id: Id,
    offset: PhysicalPosition<f64>,
}

/// An interactive resize session of a logical window.
#[derive(Debug, Clone, Copy)]
struct DragResize {
    id: Id,
    direction: window::Direction,
    start_bounds: PhysicalBounds,
    start_cursor: PhysicalPosition<f64>,
    /// The size constraints of the window, in the physical coordinate
    /// space of the native window.
    min_size: Option<PhysicalSize<f64>>,
    max_size: Option<PhysicalSize<f64>>,
}

/// The result of routing a native pointer event.
pub enum Routed {
    /// The event should be delivered to the user interface as-is.
    Deliver(WindowEvent),
    /// A move session proposed a new position for a window; the receiver
    /// keeps the window on screen. The event that ended the session, if
    /// any, still needs to be delivered to keep the widget state
    /// consistent.
    Moved {
        id: Id,
        position: PhysicalPosition<f64>,
        release: Option<WindowEvent>,
    },
    /// A resize session proposed new bounds for a window, within its size
    /// constraints. The event that ended the session, if any, still needs
    /// to be delivered to keep the widget state consistent.
    Resized {
        id: Id,
        bounds: PhysicalBounds,
        release: Option<WindowEvent>,
    },
    /// The event was consumed by an interactive session.
    Consumed,
}

impl Router {
    pub fn new() -> Self {
        Self {
            focused: None,
            cursor: None,
            drag: None,
            drag_resize: None,
        }
    }

    pub fn focused(&self) -> Option<Id> {
        self.focused
    }

    pub fn cursor(&self) -> Option<PhysicalPosition<f64>> {
        self.cursor
    }

    /// Focuses a logical window.
    pub fn focus(&mut self, id: Id) {
        self.focused = Some(id);
    }

    /// Forgets a window that is about to be removed.
    pub fn forget(&mut self, id: Id) {
        if self.focused == Some(id) {
            self.focused = None;
        }
    }

    /// Starts an interactive move session for the given window, like
    /// `Window::drag_window` does on the desktop platforms.
    ///
    /// The session is driven by the pointer and ends when a button is
    /// released.
    pub fn start_drag(&mut self, id: Id, bounds: PhysicalBounds) -> bool {
        let Some(cursor) = self.cursor else {
            return false;
        };

        self.drag = Some(Drag {
            id,
            offset: PhysicalPosition::new(
                cursor.x - bounds.position.x,
                cursor.y - bounds.position.y,
            ),
        });

        true
    }

    /// Starts an interactive resize session for the given window, like
    /// `Window::drag_resize_window` does on the desktop platforms. The
    /// size constraints, in physical coordinates, bound the session.
    pub fn start_drag_resize(
        &mut self,
        id: Id,
        direction: window::Direction,
        bounds: PhysicalBounds,
        min_size: Option<PhysicalSize<f64>>,
        max_size: Option<PhysicalSize<f64>>,
    ) -> bool {
        let Some(cursor) = self.cursor else {
            return false;
        };

        self.drag_resize = Some(DragResize {
            id,
            direction,
            start_bounds: bounds,
            start_cursor: cursor,
            min_size,
            max_size,
        });

        true
    }

    /// Routes a native pointer event.
    pub fn pointer_event(&mut self, event: WindowEvent) -> Routed {
        let position = pointer_position(&event);

        if let Some(position) = position {
            self.cursor = Some(position);
        }

        // Interactive sessions take priority: they consume the pointer
        // events that drive them and end on release.
        if self.drag.is_some() || self.drag_resize.is_some() {
            let release = matches!(
                &event,
                WindowEvent::PointerButton {
                    state: ElementState::Released,
                    ..
                }
            )
            .then(|| event.clone());

            if let Some(drag) = self.drag {
                if let Some(cursor) = position {
                    if release.is_some() {
                        self.drag = None;
                    }

                    return Routed::Moved {
                        id: drag.id,
                        position: PhysicalPosition::new(
                            cursor.x - drag.offset.x,
                            cursor.y - drag.offset.y,
                        ),
                        release,
                    };
                }
            } else if let Some(drag_resize) = self.drag_resize {
                if let Some(cursor) = position {
                    if release.is_some() {
                        self.drag_resize = None;
                    }

                    return Routed::Resized {
                        id: drag_resize.id,
                        bounds: resized_bounds(&drag_resize, cursor),
                        release,
                    };
                }
            }

            // Events without a position cannot drive the sessions.
            return Routed::Consumed;
        }

        Routed::Deliver(event)
    }
}

/// Returns the position carried by a pointer event, if any.
fn pointer_position(event: &WindowEvent) -> Option<PhysicalPosition<f64>> {
    match event {
        WindowEvent::PointerMoved { position, .. }
        | WindowEvent::PointerEntered { position, .. }
        | WindowEvent::PointerButton { position, .. } => Some(*position),
        _ => None,
    }
}

/// Computes the bounds proposed by an interactive resize session.
///
/// The size stays within the constraints of the window, and the edges
/// opposite the grabbed border stay in place.
fn resized_bounds(drag_resize: &DragResize, cursor: PhysicalPosition<f64>) -> PhysicalBounds {
    let dx = cursor.x - drag_resize.start_cursor.x;
    let dy = cursor.y - drag_resize.start_cursor.y;

    let start = drag_resize.start_bounds;
    let mut width = start.size.width;
    let mut height = start.size.height;

    match drag_resize.direction {
        window::Direction::North => height -= dy,
        window::Direction::South => height += dy,
        window::Direction::East => width += dx,
        window::Direction::West => width -= dx,
        window::Direction::NorthEast => {
            height -= dy;
            width += dx;
        }
        window::Direction::NorthWest => {
            height -= dy;
            width -= dx;
        }
        window::Direction::SouthEast => {
            height += dy;
            width += dx;
        }
        window::Direction::SouthWest => {
            height += dy;
            width -= dx;
        }
    }

    if let Some(min_size) = drag_resize.min_size {
        width = width.max(min_size.width);
        height = height.max(min_size.height);
    }

    if let Some(max_size) = drag_resize.max_size {
        width = width.min(max_size.width);
        height = height.min(max_size.height);
    }

    // A window must keep a physical size to stay renderable.
    width = width.max(1.0);
    height = height.max(1.0);

    let mut position = start.position;

    if matches!(
        drag_resize.direction,
        window::Direction::West | window::Direction::NorthWest | window::Direction::SouthWest
    ) {
        position.x += start.size.width - width;
    }

    if matches!(
        drag_resize.direction,
        window::Direction::North | window::Direction::NorthWest | window::Direction::NorthEast
    ) {
        position.y += start.size.height - height;
    }

    PhysicalBounds {
        position,
        size: PhysicalSize::new(width, height),
    }
}
