use std::{any::Any, sync::Arc};

use iced_core::{
    Element, Layout, Length, Rectangle, Renderer as _, Size, Theme, layout, mouse, renderer,
    widget, window,
};
use iced_futures::Subscription;
use iced_runtime::Task;
use iced_wgpu::Renderer;
use iced_widget::{pane_grid, space, stack};
use lapiz_runtime::Services;
use lapiz_utils::wrapper;
use lapiz_widgets::menu::{ContextMenu, Menu};
use parse_display::Display;
use serde::Serialize;

use crate::{
    AttachInfo, DockState,
    group::{DockGroupData, tab_row::TabRowWidget},
};

pub trait Dock: 'static {
    type Message: Send + 'static;

    fn id(&self) -> DockId;
    fn view<'a>(
        &'a self,
        window_id: window::Id,
        services: &'a Services,
    ) -> Element<'a, Self::Message, Theme, Renderer>;
    fn update(&mut self, message: Self::Message, services: &mut Services) -> Task<Self::Message>;
    fn subscription(&self, _services: &Services) -> Subscription<Self::Message> {
        Subscription::none()
    }
    fn on_open(&mut self) -> Task<Self::Message> {
        Task::none()
    }
    fn on_close(&mut self) -> Task<Self::Message> {
        Task::none()
    }
    fn sub_windows(&self) -> Vec<window::Id> {
        Vec::new()
    }
}

pub trait ErasedDock: 'static {
    fn id(&self) -> DockId;
    fn view<'a>(
        &'a self,
        window_id: window::Id,
        services: &'a Services,
    ) -> Element<'a, Box<dyn Any + Send>, Theme, Renderer>;
    fn update(
        &mut self,
        message: Box<dyn Any + Send>,
        services: &mut Services,
    ) -> Task<Box<dyn Any + Send>>;
    fn subscription(&self, services: &Services) -> Subscription<Box<dyn Any + Send>>;
    fn on_open(&mut self) -> Task<Box<dyn Any + Send>>;
    fn on_close(&mut self) -> Task<Box<dyn Any + Send>>;
    fn sub_windows(&self) -> Vec<window::Id>;
}

impl<T: Dock> ErasedDock for T {
    fn id(&self) -> DockId {
        self.id()
    }

    fn view<'a>(
        &'a self,
        window_id: window::Id,
        services: &'a Services,
    ) -> Element<'a, Box<dyn Any + Send>, Theme, Renderer> {
        self.view(window_id, services)
            .map(|m| Box::new(m) as Box<dyn Any + Send>)
    }

    fn update(
        &mut self,
        message: Box<dyn Any + Send>,
        services: &mut Services,
    ) -> Task<Box<dyn Any + Send>> {
        let msg = *message
            .downcast::<T::Message>()
            .expect("invalid message type");
        self.update(msg, services)
            .map(|m| Box::new(m) as Box<dyn Any + Send>)
    }

    fn subscription(&self, services: &Services) -> Subscription<Box<dyn Any + Send>> {
        self.subscription(services)
            .map(|m| Box::new(m) as Box<dyn Any + Send>)
    }

    fn on_open(&mut self) -> Task<Box<dyn Any + Send>> {
        self.on_open().map(|m| Box::new(m) as Box<dyn Any + Send>)
    }

    fn on_close(&mut self) -> Task<Box<dyn Any + Send>> {
        self.on_close().map(|m| Box::new(m) as Box<dyn Any + Send>)
    }

    fn sub_windows(&self) -> Vec<window::Id> {
        self.sub_windows()
    }
}

wrapper! {
    #[derive(Debug, Clone, PartialEq, Eq, Hash, Display, Serialize)]
    #[display("{0}")]
    pub DockId : Arc<str>
}

#[derive(Debug, Clone)]
pub enum DockAction {
    Pane(PaneEvent),
    Tab(pane_grid::Pane, TabEvent),
}

#[derive(Debug, Clone)]
pub enum PaneEvent {
    Clicked(pane_grid::Pane),
    Resized(pane_grid::ResizeEvent),
}

#[derive(Debug, Clone)]
pub enum TabEvent {
    Select(DockId),
    Close(DockId),
    CloseGroup,
    Reorder { from: usize, to: usize },
    Detach(DockId),
    TitleBarDrag,
}

type DockContentView<'a, Message> =
    Box<dyn Fn(pane_grid::Pane, DockId) -> Element<'a, Message, Theme, Renderer> + 'a>;

type FloatContentView<'a, Message> =
    Box<dyn Fn(DockId) -> Element<'a, Message, Theme, Renderer> + 'a>;

pub struct DockWidget<'a, Message> {
    state: &'a DockState,
    content: Option<DockContentView<'a, Message>>,
    on_action: Box<dyn Fn(DockAction) -> Message + 'a>,
    spacing: f32,
    attach_info: Option<AttachInfo>,
}

impl<'a, Message> DockWidget<'a, Message> {
    pub fn new(state: &'a DockState, on_action: impl Fn(DockAction) -> Message + 'a) -> Self {
        Self {
            state,
            content: None,
            on_action: Box::new(on_action),
            spacing: 2.0,
            attach_info: None,
        }
    }

    pub fn content(
        mut self,
        f: impl Fn(pane_grid::Pane, DockId) -> Element<'a, Message, Theme, Renderer> + 'a,
    ) -> Self {
        self.content = Some(Box::new(f));
        self
    }

    pub fn spacing(mut self, s: f32) -> Self {
        self.spacing = s;
        self
    }

    pub fn attach_info(mut self, split_info: AttachInfo) -> Self {
        self.attach_info = Some(split_info);
        self
    }
}

impl<'a, Message: 'a> From<DockWidget<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(w: DockWidget<'a, Message>) -> Self {
        use std::rc::Rc;

        let DockWidget {
            state,
            content,
            on_action,
            spacing,
            attach_info: drag_hint,
        } = w;

        let on_action = Rc::<dyn Fn(DockAction) -> Message>::from(on_action);
        let a_click = Rc::clone(&on_action);
        let a_resize = Rc::clone(&on_action);
        let a_titlebar = Rc::clone(&on_action);

        let grid = if let Some(panes_state) = state.panes_state().as_ref() {
            pane_grid::PaneGrid::new(panes_state, move |pane, group_data, _maximized| {
                let body = group_data
                    .active()
                    .and_then(|id| content.as_ref().map(|c| c(pane, id.clone())))
                    .unwrap_or_else(|| Element::new(space()));

                let tabs =
                    TabRowWidget::new(group_data, std::convert::identity).title_drag_deadband(10.0);

                let Some(active) = group_data.active() else {
                    return space().into();
                };
                let ctx_menu = ContextMenu::new(
                    Element::new(tabs),
                    Menu::new()
                        .item("Close Active", TabEvent::Close(active.clone()))
                        .item("Close Group", TabEvent::CloseGroup),
                );
                let a_titlebar = Rc::clone(&a_titlebar);
                pane_grid::Content::new(body).title_bar(pane_grid::TitleBar::new(
                    Element::new(ctx_menu)
                        .map(move |msg| (a_titlebar.as_ref())(DockAction::Tab(pane, msg))),
                ))
            })
            .on_click(move |p| (a_click.as_ref())(DockAction::Pane(PaneEvent::Clicked(p))))
            .on_resize(5.0, move |e| {
                (a_resize.as_ref())(DockAction::Pane(PaneEvent::Resized(e)))
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .spacing(spacing)
            .into()
        } else {
            space().into()
        };

        if let Some(split_info) = drag_hint {
            let overlay = PaneHintOverlay {
                state,
                attach_info: split_info,
                spacing,
            };
            iced_widget::stack![grid, Element::new(overlay)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            grid
        }
    }
}

pub struct FloatingDockWidget<'a, Message> {
    group_data: &'a DockGroupData,
    content: Option<FloatContentView<'a, Message>>,
    on_action: Box<dyn Fn(TabEvent) -> Message + 'a>,
    is_attaching: bool,
}

impl<'a, Message> FloatingDockWidget<'a, Message> {
    pub fn new(
        group_data: &'a DockGroupData,
        on_action: impl Fn(TabEvent) -> Message + 'a,
    ) -> Self {
        Self {
            group_data,
            content: None,
            on_action: Box::new(on_action),
            is_attaching: false,
        }
    }

    pub fn content(
        mut self,
        f: impl Fn(DockId) -> Element<'a, Message, Theme, Renderer> + 'a,
    ) -> Self {
        self.content = Some(Box::new(f));
        self
    }

    pub fn is_merging(mut self, attaching: bool) -> Self {
        self.is_attaching = attaching;
        self
    }
}

impl<'a, Message: 'a> From<FloatingDockWidget<'a, Message>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(w: FloatingDockWidget<'a, Message>) -> Self {
        use std::rc::Rc;

        let FloatingDockWidget {
            group_data,
            content,
            on_action,
            is_attaching,
        } = w;

        let on_action: Rc<dyn Fn(TabEvent) -> Message + 'a> = Rc::from(on_action);

        let tab_row = TabRowWidget::new(group_data, move |event| (on_action.as_ref())(event));

        let body = group_data
            .active()
            .and_then(|id| content.map(|c| c(id.clone())))
            .unwrap_or_else(|| Element::new(space()));

        let content = iced_widget::column![Element::from(tab_row), body]
            .width(Length::Fill)
            .height(Length::Fill);

        if is_attaching {
            stack![Element::new(WindowHintOverlay), content].into()
        } else {
            content.into()
        }
    }
}

const ATTACH_HINT_COLOR: iced_core::Color = iced_core::Color {
    r: 0.15,
    g: 0.55,
    b: 1.0,
    a: 0.35,
};

/// Transparent overlay widget drawn on top of `DockWidget` to show where a
/// floating window would re-attach (the pane half closest to the hint cursor).
struct PaneHintOverlay<'a> {
    state: &'a DockState,
    attach_info: AttachInfo,
    spacing: f32,
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
                    pane_grid::Edge::Left => iced_core::Rectangle {
                        x: bounds.x + region.x,
                        y: bounds.y + region.y,
                        width: region.width / 2.0,
                        height: region.height,
                    },
                    pane_grid::Edge::Right => iced_core::Rectangle {
                        x: bounds.x + region.x + region.width / 2.0,
                        y: bounds.y + region.y,
                        width: region.width / 2.0,
                        height: region.height,
                    },
                    pane_grid::Edge::Top => iced_core::Rectangle {
                        x: bounds.x + region.x,
                        y: bounds.y + region.y,
                        width: region.width,
                        height: region.height / 2.0,
                    },
                    pane_grid::Edge::Bottom => iced_core::Rectangle {
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

                iced_core::Rectangle {
                    x: bounds.x + region.x,
                    y: bounds.y + region.y,
                    width: region.width,
                    height: region.height,
                }
            }
            AttachInfo::Initialize => bounds,
        };

        renderer.fill_quad(
            iced_core::renderer::Quad {
                bounds: highlight,
                ..iced_core::renderer::Quad::default()
            },
            iced_core::Background::Color(ATTACH_HINT_COLOR),
        );
    }
}

struct WindowHintOverlay;

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
            iced_core::renderer::Quad {
                bounds: layout.bounds(),
                ..Default::default()
            },
            iced_core::Background::Color(ATTACH_HINT_COLOR),
        );
    }
}
