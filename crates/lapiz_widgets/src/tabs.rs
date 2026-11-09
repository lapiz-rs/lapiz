use iced_core::{
    Element, Event, Layout, Length, Pixels, Point, Rectangle, Renderer as _, Shell, Size, Theme,
    Vector, Widget, alignment, layout, overlay,
    pointer::{self, mouse},
    renderer,
    text::{
        self, LineHeight, Renderer as _, Shaping, Text,
        paragraph::{self, Plain},
    },
    widget::{self, Tree, tree},
};
use lapiz_runtime::Renderer;

use crate::{
    column,
    menu::{ContextMenu, Menu},
};

pub struct TabbedView<'a, Message> {
    bar: TabBar<'a, Message>,
    pages: Vec<Element<'a, Message, Theme, Renderer>>,
    context_menu: Option<Menu<'a, Message>>,
}

impl<'a, Message: 'a> Default for TabbedView<'a, Message> {
    fn default() -> Self {
        Self {
            bar: TabBar::new(),
            pages: Vec::new(),
            context_menu: None,
        }
    }
}

impl<'a, Message: 'a> TabbedView<'a, Message> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tabs(
        tabs: impl IntoIterator<Item = (String, Element<'a, Message, Theme, Renderer>)>,
        selected: usize,
    ) -> Self {
        let mut this = Self::new();
        for (title, content) in tabs {
            this.bar = this.bar.tab(title);
            this.pages.push(content);
        }
        this.bar = this.bar.selected(selected);
        this
    }

    pub fn tab(
        mut self,
        title: impl Into<String>,
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        self.bar = self.bar.tab(title);
        self.pages.push(content.into());
        self
    }

    pub fn selected(mut self, index: usize) -> Self {
        self.bar = self.bar.selected(index);
        self
    }

    pub fn on_select(mut self, callback: impl Fn(usize) -> Message + 'a) -> Self {
        self.bar = self.bar.on_select(callback);
        self
    }

    pub fn on_reorder(mut self, callback: impl Fn(usize, usize) -> Message + 'a) -> Self {
        self.bar = self.bar.on_reorder(callback);
        self
    }

    pub fn on_detach(mut self, callback: impl Fn(usize) -> Message + 'a) -> Self {
        self.bar = self.bar.on_detach(callback);
        self
    }

    pub fn font_size(mut self, font_size: impl Into<Pixels>) -> Self {
        self.bar = self.bar.font_size(font_size);
        self
    }

    pub fn padding(mut self, padding: f32) -> Self {
        self.bar = self.bar.padding(padding);
        self
    }

    pub fn context_menu(mut self, menu: Menu<'a, Message>) -> Self {
        self.context_menu = Some(menu);
        self
    }
}

impl<'a, Message: Clone + 'a> From<TabbedView<'a, Message>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(value: TabbedView<'a, Message>) -> Self {
        let selected = value.bar.selected;
        let header = if let Some(menu) = value.context_menu {
            Element::new(ContextMenu::new(value.bar, menu))
        } else {
            Element::new(value.bar)
        };

        column![
            header,
            Element::new(TabbedViewWidget {
                pages: value.pages,
                selected,
            })
        ]
        .into()
    }
}

struct TabbedViewWidget<'a, Message> {
    pages: Vec<Element<'a, Message, Theme, Renderer>>,
    selected: Option<usize>,
}

impl<Message> Widget<Message, Theme, Renderer> for TabbedViewWidget<'_, Message> {
    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut self.pages);
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = limits.resolve(Length::Fill, Length::Fill, Size::ZERO);
        let mut nodes = Vec::with_capacity(self.pages.len());
        for (index, page) in self.pages.iter_mut().enumerate() {
            if Some(index) == self.selected {
                nodes.push(page.as_widget_mut().layout(
                    &mut tree.children[index],
                    renderer,
                    &layout::Limits::new(Size::ZERO, size),
                ));
            } else {
                nodes.push(layout::Node::new(Size::ZERO));
            }
        }
        layout::Node::with_children(size, nodes)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let Some(selected) = self.selected else {
            return;
        };

        if let (Some(page), Some(state), Some(layout)) = (
            self.pages.get_mut(selected),
            tree.children.get_mut(selected),
            layout.children().nth(selected),
        ) {
            page.as_widget_mut()
                .update(state, event, layout, cursor, renderer, shell, viewport);
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let Some(selected) = self.selected else {
            return mouse::Interaction::None;
        };

        match (
            self.pages.get(selected),
            tree.children.get(selected),
            layout.children().nth(selected),
        ) {
            (Some(page), Some(state), Some(layout)) => page
                .as_widget()
                .mouse_interaction(state, layout, cursor, viewport, renderer),
            _ => mouse::Interaction::None,
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let Some(selected) = self.selected else {
            return;
        };

        if let (Some(page), Some(state), Some(layout)) = (
            self.pages.get(selected),
            tree.children.get(selected),
            layout.children().nth(selected),
        ) {
            page.as_widget()
                .draw(state, renderer, theme, style, layout, cursor, viewport);
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let Some(selected) = self.selected else {
            return;
        };

        if let (Some(page), Some(state), Some(layout)) = (
            self.pages.get_mut(selected),
            tree.children.get_mut(selected),
            layout.children().nth(selected),
        ) {
            page.as_widget_mut()
                .operate(state, layout, renderer, operation);
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let selected = self.selected?;
        self.pages.get_mut(selected)?.as_widget_mut().overlay(
            tree.children.get_mut(selected)?,
            layout.children().nth(selected)?,
            renderer,
            viewport,
            translation,
        )
    }
}

#[derive(Debug, Default)]
struct TabRowState {
    hovered: Option<usize>,
    action: TabAction,
    labels: Vec<Plain<<Renderer as text::Renderer>::Paragraph>>,
    bounds: Vec<Rectangle>,
}

#[derive(Debug, Default, Clone, Copy)]
enum TabAction {
    #[default]
    Idle,
    Pressing {
        index: usize,
        origin: Point,
    },
    Dragging {
        index: usize,
    },
    TitleDragging {
        origin: Point,
    },
}

const DETACH_DEADBAND_FACTOR: f32 = 0.5;

fn drag_target_index(bounds: Rectangle, tab_bounds: &[Rectangle], cursor: Point) -> Option<usize> {
    let margin = bounds.height * DETACH_DEADBAND_FACTOR;
    if cursor.x < bounds.x - margin
        || cursor.x > bounds.x + bounds.width + bounds.height + margin
        || cursor.y < bounds.y - margin
        || cursor.y > bounds.y + bounds.height * 2.0 + margin
    {
        return None;
    }

    let x = cursor.x - bounds.x;
    tab_bounds
        .iter()
        .position(|tab| x < tab.center_x())
        .or(Some(tab_bounds.len()))
}

fn hit_test(bounds: &[Rectangle], cursor_rel: Point) -> Option<usize> {
    for (i, bounds) in bounds.iter().enumerate() {
        if bounds.contains(cursor_rel) {
            return Some(i);
        }
    }

    None
}

pub struct TabBar<'a, Message> {
    titles: Vec<String>,
    selected: Option<usize>,
    on_select: Option<Box<dyn Fn(usize) -> Message + 'a>>,
    on_reorder: Option<Box<dyn Fn(usize, usize) -> Message + 'a>>,
    on_detach: Option<Box<dyn Fn(usize) -> Message + 'a>>,
    on_title_drag: Option<Box<dyn Fn() -> Message + 'a>>,
    title_drag_threshold: f32,
    tab_drag_threshold: f32,
    font_size: Pixels,
    padding: f32,
}

impl<'a, Message> Default for TabBar<'a, Message> {
    fn default() -> Self {
        Self {
            titles: Vec::new(),
            selected: None,
            on_select: None,
            on_reorder: None,
            on_detach: None,
            on_title_drag: None,
            font_size: Pixels(11.0),
            title_drag_threshold: 10.0,
            tab_drag_threshold: 5.0,
            padding: 6.0,
        }
    }
}

impl<'a, Message: 'a> TabBar<'a, Message> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tabs(tabs: impl IntoIterator<Item = String>, selected: usize) -> Self {
        Self {
            titles: tabs.into_iter().collect(),
            selected: Some(selected),
            ..Self::new()
        }
    }

    pub fn tab(mut self, title: impl Into<String>) -> Self {
        self.titles.push(title.into());
        self
    }

    pub fn selected(mut self, index: usize) -> Self {
        self.selected = Some(index);
        self
    }

    pub fn on_select(mut self, callback: impl Fn(usize) -> Message + 'a) -> Self {
        self.on_select = Some(Box::new(callback));
        self
    }

    pub fn on_reorder(mut self, callback: impl Fn(usize, usize) -> Message + 'a) -> Self {
        self.on_reorder = Some(Box::new(callback));
        self
    }

    pub fn on_detach(mut self, callback: impl Fn(usize) -> Message + 'a) -> Self {
        self.on_detach = Some(Box::new(callback));
        self
    }

    pub fn on_title_drag(mut self, callback: impl Fn() -> Message + 'a) -> Self {
        self.on_title_drag = Some(Box::new(callback));
        self
    }

    pub fn font_size(mut self, font_size: impl Into<Pixels>) -> Self {
        self.font_size = font_size.into();
        self
    }

    pub fn title_drag_threshold(mut self, threshold: f32) -> Self {
        self.title_drag_threshold = threshold;
        self
    }

    pub fn tab_drag_threshold(mut self, threshold: f32) -> Self {
        self.tab_drag_threshold = threshold;
        self
    }

    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    fn height(&self) -> f32 {
        self.font_size.0 + self.padding * 2.0
    }
}

impl<Message> iced_core::Widget<Message, Theme, Renderer> for TabBar<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<TabRowState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(TabRowState::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(self.height()))
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<TabRowState>();
        state.labels.clear();
        state.bounds.clear();
        let height = self.height();
        let available = limits.max().width;

        let mut natural_widths = Vec::with_capacity(self.titles.len());
        for title in &self.titles {
            let p = paragraph::Plain::new(Text {
                content: title.clone(),
                bounds: limits.max(),
                size: self.font_size,
                line_height: LineHeight::Relative(1.0),
                font: renderer.default_font(),
                align_x: text::Alignment::Center,
                align_y: alignment::Vertical::Center,
                shaping: Shaping::Auto,
                wrapping: text::Wrapping::None,
                ellipsis: text::Ellipsis::None,
                hint_factor: None,
            });
            natural_widths.push(p.min_width() + self.padding * 2.0);
            state.labels.push(p);
        }

        let total = natural_widths.iter().sum::<f32>();
        let overflow = total > available;

        let mut x = 0.0;
        for (i, natural) in natural_widths.iter().enumerate() {
            let width = if overflow {
                if i + 1 == natural_widths.len() {
                    (available - x).max(0.0)
                } else {
                    natural * available / total
                }
            } else {
                *natural
            };
            state.bounds.push(Rectangle {
                x,
                y: 0.0,
                width,
                height,
            });
            x += width;
        }
        layout::Node::new(limits.resolve(Length::Fill, Length::Fixed(height), limits.max()))
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<TabRowState>();
        let bounds = layout.bounds();
        let p = theme.palette();

        renderer.fill_quad(
            renderer::Quad {
                bounds,
                ..Default::default()
            },
            p.background.base.color,
        );
        renderer.fill_quad(
            renderer::Quad {
                bounds: Rectangle {
                    y: bounds.y + bounds.height - 1.0,
                    height: 1.0,
                    ..bounds
                },
                ..Default::default()
            },
            p.background.strong.color,
        );

        let drag_target = if matches!(state.action, TabAction::Dragging { .. }) {
            cursor
                .position()
                .and_then(|pos| drag_target_index(bounds, &state.bounds, pos))
        } else {
            None
        };

        for i in 0..self.titles.len() {
            let tab_rect_rel = state.bounds[i];
            let tab_rect = Rectangle {
                x: tab_rect_rel.x + bounds.x,
                y: tab_rect_rel.y + bounds.y,
                ..tab_rect_rel
            };

            let is_selected = self.selected == Some(i);
            let is_hovered = state.hovered == Some(i);
            let (background, text_color) = if is_selected {
                (p.background.weakest.color, p.background.base.text)
            } else if is_hovered {
                (p.primary.weak.color, p.primary.weak.text)
            } else {
                (p.background.base.color, p.background.weak.text)
            };

            renderer.fill_quad(
                renderer::Quad {
                    bounds: tab_rect,
                    ..Default::default()
                },
                background,
            );
            renderer.fill_quad(
                renderer::Quad {
                    bounds: Rectangle {
                        x: tab_rect.x + tab_rect.width - 1.0,
                        width: 1.0,
                        ..tab_rect
                    },
                    ..Default::default()
                },
                p.background.strong.color,
            );
            if is_selected {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            height: 2.0,
                            ..tab_rect
                        },
                        ..Default::default()
                    },
                    p.primary.base.color,
                );
            }

            let mut label = state.labels[i]
                .as_text()
                .with_content(state.labels[i].content().to_string());
            let inner = Rectangle {
                x: tab_rect.x + self.padding,
                width: (tab_rect.width - self.padding * 2.0).max(0.0),
                ..tab_rect
            };
            let position = if state.labels[i].min_width() > inner.width {
                // Keep the start of the title visible instead of cutting both ends.
                label.align_x = text::Alignment::Left;
                Point::new(inner.x, tab_rect.center().y)
            } else {
                tab_rect.center()
            };
            let clip = if label.align_x == text::Alignment::Left {
                inner
            } else {
                tab_rect
            };
            renderer.fill_text(label, position, text_color, clip);
        }

        if let Some(target) = drag_target {
            let x = state
                .bounds
                .get(target)
                .map(|tab| tab.x)
                .unwrap_or_else(|| state.bounds.last().map_or(0.0, |tab| tab.x + tab.width));
            renderer.fill_quad(
                renderer::Quad {
                    bounds: Rectangle {
                        x: bounds.x + x - 1.5,
                        y: bounds.y,
                        width: 3.0,
                        height: bounds.height,
                    },
                    ..Default::default()
                },
                p.primary.base.color,
            );
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let state = tree.state.downcast_mut::<TabRowState>();

        match event {
            Event::Pointer(pointer::Event::PointerMoved { position, .. }) => {
                let hovered = bounds
                    .contains(*position)
                    .then(|| Point::new(position.x - bounds.x, position.y - bounds.y))
                    .and_then(|position| hit_test(&state.bounds, position));
                if state.hovered != hovered {
                    state.hovered = hovered;
                    shell.request_redraw();
                }
                match state.action {
                    TabAction::Idle => {}
                    TabAction::Pressing { index, origin } => {
                        if position.distance(origin) >= self.tab_drag_threshold
                            && (self.on_reorder.is_some() || self.on_detach.is_some())
                        {
                            state.action = TabAction::Dragging { index };
                            shell.request_redraw();
                        }
                        shell.capture_event();
                    }
                    TabAction::Dragging { index } => {
                        shell.capture_event();
                        shell.request_redraw();
                        if drag_target_index(bounds, &state.bounds, *position).is_none()
                            && let Some(callback) = &self.on_detach
                        {
                            shell.publish(callback(index));
                            state.action = TabAction::Idle;
                        }
                    }
                    TabAction::TitleDragging { origin } => {
                        if position.distance(origin) > self.title_drag_threshold {
                            if let Some(callback) = &self.on_title_drag {
                                shell.publish(callback());
                            }
                            state.action = TabAction::Idle;
                        }
                        shell.capture_event();
                    }
                }
            }

            Event::Pointer(event @ pointer::Event::PointerPressed { position, .. })
                if event.is_primary_press() =>
            {
                if !bounds.contains(*position) {
                    return;
                }

                state.action = match hit_test(
                    &state.bounds,
                    Point::new(position.x - bounds.x, position.y - bounds.y),
                ) {
                    Some(index) => TabAction::Pressing {
                        index,
                        origin: *position,
                    },
                    None if self.on_title_drag.is_some() => {
                        TabAction::TitleDragging { origin: *position }
                    }
                    None => TabAction::Idle,
                };
                if !matches!(state.action, TabAction::Idle) {
                    shell.capture_event();
                }
            }

            Event::Pointer(event @ pointer::Event::PointerReleased { position, .. })
                if event.is_primary_release() =>
            {
                match state.action {
                    TabAction::Pressing { index, .. } => {
                        let released_on = bounds
                            .contains(*position)
                            .then(|| Point::new(position.x - bounds.x, position.y - bounds.y))
                            .and_then(|position| hit_test(&state.bounds, position));
                        if released_on == Some(index)
                            && let Some(callback) = &self.on_select
                        {
                            shell.publish(callback(index));
                        }
                    }
                    TabAction::Dragging { index: from } => {
                        if let (Some(to), Some(callback)) = (
                            drag_target_index(bounds, &state.bounds, *position),
                            &self.on_reorder,
                        ) {
                            let to = if to > from { to - 1 } else { to };
                            if to != from {
                                shell.publish(callback(from, to));
                            }
                        }
                    }
                    TabAction::TitleDragging { .. } => {}
                    TabAction::Idle => return,
                }
                state.action = TabAction::Idle;
                shell.capture_event();
                shell.request_redraw();
            }

            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<TabRowState>();
        if matches!(state.action, TabAction::Dragging { .. }) {
            return mouse::Interaction::Grabbing;
        }
        match cursor.position_in(layout.bounds()) {
            Some(position) => match hit_test(&state.bounds, position) {
                Some(_) => mouse::Interaction::Pointer,
                None => mouse::Interaction::default(),
            },
            None => mouse::Interaction::default(),
        }
    }
}

impl<'a, Message: 'a> From<TabBar<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(widget: TabBar<'a, Message>) -> Self {
        Element::new(widget)
    }
}
