use std::{any::Any, sync::Arc};

use iced_core::{
    Element, Layout, Length, Rectangle, Renderer as _, Size, Theme, layout, pointer::mouse,
    renderer, widget, window,
};
use iced_futures::Subscription;
use iced_runtime::Task;
use lapiz_i18n::t;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_utils::wrapper;
use lapiz_widgets::pane_grid;
use parse_display::Display;
use serde::{Deserialize, Serialize};

use crate::{AttachInfo, DockState};

/// A dockable workspace panel that renders UI, processes messages, and manages its lifecycle within [`DockManager`](crate::DockManager).
///
/// Implementations provide a unique [`DockId`], an Iced `view` and `update` loop with access to [`Globals`],
/// optional background subscriptions, lifecycle hooks (`on_open`, `on_close`), and any auxiliary sub-windows owned by the dock.
pub trait Dock: 'static {
    type Message: Send + 'static;

    fn id(&self) -> DockId;
    fn display_name(&self, _globals: &Globals) -> String {
        t!(&self.id())
    }
    fn view<'a>(
        &'a self,
        window_id: window::Id,
        globals: &'a Globals,
    ) -> Element<'a, Self::Message, Theme, Renderer>;
    fn update(&mut self, message: Self::Message, globals: &mut Globals) -> Task<Self::Message>;
    fn subscription(&self, _globals: &Globals) -> Subscription<Self::Message> {
        Subscription::none()
    }
    fn on_open(&mut self, _globals: &mut Globals) -> Task<Self::Message> {
        Task::none()
    }
    fn on_close(&mut self, _globals: &mut Globals) -> Task<Self::Message> {
        Task::none()
    }
    fn sub_windows(&self) -> Vec<window::Id> {
        Vec::new()
    }
}

/// Type-erased [`Dock`] trait used for heterogeneous storage in [`DockManager`](crate::DockManager).
pub trait ErasedDock: 'static {
    fn id(&self) -> DockId;
    fn display_name(&self, globals: &Globals) -> String;
    fn view<'a>(
        &'a self,
        window_id: window::Id,
        globals: &'a Globals,
    ) -> Element<'a, Box<dyn Any + Send>, Theme, Renderer>;
    fn update(
        &mut self,
        message: Box<dyn Any + Send>,
        globals: &mut Globals,
    ) -> Task<Box<dyn Any + Send>>;
    fn subscription(&self, globals: &Globals) -> Subscription<Box<dyn Any + Send>>;
    fn on_open(&mut self, globals: &mut Globals) -> Task<Box<dyn Any + Send>>;
    fn on_close(&mut self, globals: &mut Globals) -> Task<Box<dyn Any + Send>>;
    fn sub_windows(&self) -> Vec<window::Id>;
}

impl<T: Dock> ErasedDock for T {
    fn id(&self) -> DockId {
        self.id()
    }

    fn display_name(&self, globals: &Globals) -> String {
        self.display_name(globals)
    }

    fn view<'a>(
        &'a self,
        window_id: window::Id,
        globals: &'a Globals,
    ) -> Element<'a, Box<dyn Any + Send>, Theme, Renderer> {
        self.view(window_id, globals)
            .map(|m| Box::new(m) as Box<dyn Any + Send>)
    }

    fn update(
        &mut self,
        message: Box<dyn Any + Send>,
        globals: &mut Globals,
    ) -> Task<Box<dyn Any + Send>> {
        let msg = *message
            .downcast::<T::Message>()
            .expect("invalid message type");
        self.update(msg, globals)
            .map(|m| Box::new(m) as Box<dyn Any + Send>)
    }

    fn subscription(&self, globals: &Globals) -> Subscription<Box<dyn Any + Send>> {
        self.subscription(globals)
            .map(|m| Box::new(m) as Box<dyn Any + Send>)
    }

    fn on_open(&mut self, globals: &mut Globals) -> Task<Box<dyn Any + Send>> {
        self.on_open(globals)
            .map(|m| Box::new(m) as Box<dyn Any + Send>)
    }

    fn on_close(&mut self, globals: &mut Globals) -> Task<Box<dyn Any + Send>> {
        self.on_close(globals)
            .map(|m| Box::new(m) as Box<dyn Any + Send>)
    }

    fn sub_windows(&self) -> Vec<window::Id> {
        self.sub_windows()
    }
}

wrapper! {
    /// Unique string identifier for a [`Dock`].
    #[derive(Debug, Clone, PartialEq, Eq, Hash, Display, Deserialize, Serialize)]
    #[display("{0}")]
    pub DockId : Arc<str>
}

/// User interaction action originating from a pane or tab bar in the main dock grid.
#[derive(Debug, Clone)]
pub enum DockAction {
    Pane(PaneEvent),
    Tab(pane_grid::Pane, TabEvent),
}

/// Interaction event emitted by the dock [`mod@pane_grid`].
#[derive(Debug, Clone)]
pub enum PaneEvent {
    Clicked(pane_grid::Pane),
    Resized(pane_grid::ResizeEvent),
}

/// Interaction event emitted by a dock group's tab bar.
#[derive(Debug, Clone)]
pub enum TabEvent {
    Select(DockId),
    Close(DockId),
    CloseGroup,
    Reorder { from: usize, to: usize },
    Detach(DockId),
    TitleBarDrag,
}

const ATTACH_HINT_COLOR: iced_core::Color = iced_core::Color {
    r: 0.15,
    g: 0.55,
    b: 1.0,
    a: 0.35,
};

/// Overlay widget that highlights the target split or merge region when dragging a dock over the main pane grid.
pub(crate) struct PaneHintOverlay<'a> {
    pub state: &'a DockState,
    pub attach_info: AttachInfo,
    pub spacing: f32,
}

impl<Message> iced_core::Widget<Message, Theme, Renderer> for PaneHintOverlay<'_> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut widget::Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn draw(
        &self,
        _tree: &widget::Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();

        let highlight = match self.attach_info {
            AttachInfo::Split { result_edge, pane } => {
                let Some(pane_states) = self.state.panes_state() else {
                    return;
                };

                let regions = pane_states
                    .layout()
                    .pane_regions(self.spacing, 0.0, bounds.size());
                let Some(region) = regions.get(&pane) else {
                    return;
                };

                match result_edge {
                    pane_grid::Edge::Left => Rectangle {
                        x: bounds.x + region.x,
                        y: bounds.y + region.y,
                        width: region.width / 2.0,
                        height: region.height,
                    },
                    pane_grid::Edge::Right => Rectangle {
                        x: bounds.x + region.x + region.width / 2.0,
                        y: bounds.y + region.y,
                        width: region.width / 2.0,
                        height: region.height,
                    },
                    pane_grid::Edge::Top => Rectangle {
                        x: bounds.x + region.x,
                        y: bounds.y + region.y,
                        width: region.width,
                        height: region.height / 2.0,
                    },
                    pane_grid::Edge::Bottom => Rectangle {
                        x: bounds.x + region.x,
                        y: bounds.y + region.y + region.height / 2.0,
                        width: region.width,
                        height: region.height / 2.0,
                    },
                }
            }
            AttachInfo::Merge { pane } => {
                let Some(pane_states) = self.state.panes_state() else {
                    return;
                };
                let regions = pane_states
                    .layout()
                    .pane_regions(self.spacing, 0.0, bounds.size());
                let Some(region) = regions.get(&pane) else {
                    return;
                };
                Rectangle {
                    x: bounds.x + region.x,
                    y: bounds.y + region.y,
                    width: region.width,
                    height: region.height,
                }
            }
            AttachInfo::Initialize => bounds,
        };

        renderer.fill_quad(
            renderer::Quad {
                bounds: highlight,
                ..Default::default()
            },
            iced_core::Background::Color(ATTACH_HINT_COLOR),
        );
    }
}

/// Overlay widget that highlights an entire floating dock window when another floating window is dragged over it to merge.
pub(crate) struct WindowHintOverlay;

impl<Message> iced_core::Widget<Message, Theme, Renderer> for WindowHintOverlay {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut widget::Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn draw(
        &self,
        _tree: &widget::Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        renderer.fill_quad(
            renderer::Quad {
                bounds: layout.bounds(),
                ..Default::default()
            },
            iced_core::Background::Color(ATTACH_HINT_COLOR),
        );
    }
}
