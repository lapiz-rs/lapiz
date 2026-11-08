use std::{cell::Cell, slice};

use iced_core::{
    Border, Color, Element, Event, Font, Layout, Length, Point, Radians, Rectangle, Renderer as _,
    Shadow, Shell, Size, Theme, Vector, Widget, alignment, keyboard, layout, overlay, pointer,
    pointer::mouse,
    renderer,
    shell::Bus,
    svg::{self, Renderer as _},
    text,
    text::{Paragraph as _, Renderer as _},
    theme::palette::Palette,
    widget::{self, Tree, tree},
};
use lapiz_runtime::Renderer;

const ROOT_PADDING: f32 = 8.0;
const ITEM_HEIGHT: f32 = 24.0;
const ITEM_PADDING_X: f32 = 10.0;
const ICON_SLOT: f32 = 14.0;
const SEPARATOR_HEIGHT: f32 = 9.0;
const PANEL_PADDING: f32 = 4.0;
const LABEL_SIZE: f32 = 12.0;
const SHORTCUT_SIZE: f32 = 10.0;

pub enum Item<'a, Message> {
    Action {
        label: String,
        shortcut: Option<String>,
        message: Message,
    },
    Element {
        label: String,
        element: Element<'a, Message, Theme, Renderer>,
        close_on_message: bool,
    },
    Submenu {
        label: String,
        submenu: Menu<'a, Message>,
    },
    Separator,
}

impl<Message> Item<'_, Message> {
    fn height(&self) -> f32 {
        match self {
            Item::Action { .. } | Item::Element { .. } | Item::Submenu { .. } => ITEM_HEIGHT,
            Item::Separator => SEPARATOR_HEIGHT,
        }
    }

    fn width(&self) -> f32 {
        match self {
            Item::Separator => 0.0,
            Item::Action {
                label, shortcut, ..
            } => {
                ITEM_PADDING_X * 2.0
                    + ICON_SLOT
                    + text_width(label, LABEL_SIZE)
                    + shortcut
                        .as_ref()
                        .map_or(0.0, |s| text_width(s, SHORTCUT_SIZE) + ITEM_PADDING_X)
            }
            Item::Element { .. } => 0.0,
            Item::Submenu { label, .. } => {
                ITEM_PADDING_X * 2.0 + ICON_SLOT + text_width(label, LABEL_SIZE) + 14.0
            }
        }
    }
}

pub struct Menu<'a, Message> {
    items: Vec<Item<'a, Message>>,
    min_width: f32,
}

impl<'a, Message> Menu<'a, Message> {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            min_width: 0.0,
        }
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.children.resize_with(self.items.len(), Tree::empty);
        for (item, tree) in self.items.iter_mut().zip(&mut tree.children) {
            match item {
                Item::Element { element, .. } => tree.diff(element.as_widget_mut()),
                Item::Submenu { submenu, .. } => {
                    if tree.tag != tree::Tag::stateless() {
                        *tree = Tree::empty();
                    }
                    submenu.diff(tree);
                }
                _ => *tree = Tree::empty(),
            }
        }
    }

    fn overlays<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        path: &[usize],
        panels: &mut impl Iterator<Item = Layout<'b>>,
        renderer: &Renderer,
        viewport: &Rectangle,
    ) -> Vec<overlay::Element<'b, (Message, bool), Theme, Renderer>>
    where
        Message: 'b,
    {
        let Some(panel) = panels.next() else {
            return Vec::new();
        };
        let mut overlays = Vec::new();
        for (index, ((item, tree), row)) in self
            .items
            .iter_mut()
            .zip(&mut tree.children)
            .zip(panel.children())
            .enumerate()
        {
            match item {
                Item::Element {
                    element,
                    close_on_message,
                    ..
                } => {
                    if let Some(overlay) = element.as_widget_mut().overlay(
                        tree,
                        row.children().next().expect("element layout"),
                        renderer,
                        viewport,
                        Vector::ZERO,
                    ) {
                        overlays.push(if *close_on_message {
                            overlay.map(&|message| (message, true))
                        } else {
                            overlay.map(&|message| (message, false))
                        });
                    }
                }
                Item::Submenu { submenu, .. } if path.first() == Some(&index) => {
                    overlays.extend(submenu.overlays(tree, &path[1..], panels, renderer, viewport));
                }
                _ => {}
            }
        }
        overlays
    }

    fn content_width(&self) -> f32 {
        self.items
            .iter()
            .map(Item::width)
            .fold(self.min_width, f32::max)
    }

    pub fn item(
        mut self,
        label: impl Into<String>,
        shortcut: Option<&str>,
        message: Message,
    ) -> Self {
        self.items.push(Item::Action {
            label: label.into(),
            shortcut: shortcut.map(String::from),
            message,
        });
        self
    }

    pub fn element(
        mut self,
        label: impl Into<String>,
        element: impl Into<Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        self.items.push(Item::Element {
            label: label.into(),
            element: element.into(),
            close_on_message: false,
        });
        self
    }

    pub fn oneshot_element(
        mut self,
        label: impl Into<String>,
        element: impl Into<Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        self.items.push(Item::Element {
            label: label.into(),
            element: element.into(),
            close_on_message: true,
        });
        self
    }

    pub fn separator(mut self) -> Self {
        self.items.push(Item::Separator);
        self
    }

    pub fn submenu(mut self, label: impl Into<String>, submenu: Menu<'a, Message>) -> Self {
        self.items.push(Item::Submenu {
            label: label.into(),
            submenu,
        });
        self
    }

    pub fn min_width(mut self, min_width: f32) -> Self {
        self.min_width = min_width;
        self
    }

    pub fn get_item_mut(&mut self, name: &str) -> Option<&mut Item<'a, Message>> {
        self.items
            .iter_mut()
            .find(|item| matches!(item, Item::Action { label, .. } | Item::Element { label, .. } | Item::Submenu { label, .. } if label == name))
    }
}

impl<Message> Default for Menu<'_, Message> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MenuBar<'a, Message> {
    labels: Vec<String>,
    menus: Vec<Menu<'a, Message>>,
    width: Length,
    height: Length,
}

impl<'a, Message> MenuBar<'a, Message> {
    pub fn new() -> Self {
        Self {
            labels: Vec::new(),
            menus: Vec::new(),
            width: Length::Fit,
            height: Length::Fixed(28.0),
        }
    }

    pub fn menu(mut self, label: impl Into<String>, menu: Menu<'a, Message>) -> Self {
        self.labels.push(label.into());
        self.menus.push(menu);
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    pub fn get_menu_mut(&mut self, category: &str) -> Option<&mut Menu<'a, Message>> {
        self.labels
            .iter()
            .position(|name| name == category)
            .and_then(|index| self.menus.get_mut(index))
    }
}

impl<Message> Default for MenuBar<'_, Message> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
pub struct MenuBarState {
    open: Vec<usize>,
    labels: Vec<String>,
    label_widths: Vec<f32>,
}

fn text_width(content: &str, size: f32) -> f32 {
    let paragraph = <Renderer as text::Renderer>::Paragraph::with_text(text::Text {
        content,
        font: Font::DEFAULT,
        size: size.into(),
        line_height: text::LineHeight::default(),
        bounds: Size::new(f32::MAX, f32::MAX),
        align_x: text::Alignment::Left,
        align_y: alignment::Vertical::Top,
        shaping: text::Shaping::Auto,
        wrapping: text::Wrapping::None,
        ellipsis: text::Ellipsis::None,
        hint_factor: None,
    });
    paragraph.min_bounds().width
}

fn text_spec(renderer: &Renderer, content: String, size: f32) -> text::Text<String, Font> {
    text::Text {
        content,
        font: renderer.default_font(),
        size: size.into(),
        line_height: text::LineHeight::default(),
        bounds: Size::new(f32::MAX, f32::MAX),
        align_x: text::Alignment::Left,
        align_y: alignment::Vertical::Top,
        shaping: text::Shaping::Auto,
        wrapping: text::Wrapping::None,
        ellipsis: text::Ellipsis::None,
        hint_factor: None,
    }
}

impl<Message> Widget<Message, Theme, Renderer> for MenuBar<'_, Message>
where
    Message: Clone,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<MenuBarState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(MenuBarState::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.children.resize_with(self.menus.len(), Tree::empty);
        for (menu, tree) in self.menus.iter_mut().zip(&mut tree.children) {
            menu.diff(tree);
        }

        let state = tree.state.downcast_mut::<MenuBarState>();
        let labels = self.labels.clone();
        if state.labels != labels {
            state.labels = labels;
            state.label_widths = state
                .labels
                .iter()
                .map(|label| text_width(label, LABEL_SIZE))
                .collect();
        }
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_ref::<MenuBarState>();
        let intrinsic = Size::new(
            state
                .label_widths
                .iter()
                .map(|width| width + ROOT_PADDING * 2.0)
                .sum(),
            0.0,
        );
        let size = limits.resolve(self.width, self.height, intrinsic);
        let mut x = 0.0;
        let children = state
            .label_widths
            .iter()
            .map(|width| {
                let node = layout::Node::new(Size::new(width + ROOT_PADDING * 2.0, size.height))
                    .move_to(Point::new(x, 0.0));
                x += width + ROOT_PADDING * 2.0;
                node
            })
            .collect();
        layout::Node::with_children(size, children)
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
        let state = tree.state.downcast_mut::<MenuBarState>();
        match event {
            Event::Pointer(event @ pointer::Event::PointerPressed { position, .. })
                if event.is_primary_press() =>
            {
                let Some(index) = root_at(layout, *position) else {
                    return;
                };
                if state.open.first() == Some(&index) {
                    state.open.clear();
                } else {
                    state.open = vec![index];
                }
                shell.capture_event();
                shell.invalidate_layout();
                shell.request_redraw();
            }
            Event::Pointer(pointer::Event::PointerMoved { position, .. })
                if !state.open.is_empty() =>
            {
                if let Some(index) = root_at(layout, *position)
                    && state.open[0] != index
                {
                    state.open.truncate(1);
                    state.open[0] = index;
                    shell.invalidate_layout();
                    shell.request_redraw();
                }
            }
            _ => {}
        }
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
        let state = tree.state.downcast_ref::<MenuBarState>();
        let p = theme.palette();
        for (index, root_layout) in layout.children().enumerate() {
            let bounds = root_layout.bounds();
            let open = state.open.first() == Some(&index);
            let hovered = state.open.is_empty() && cursor.is_over(bounds);
            if open || hovered {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds,
                        ..renderer::Quad::default()
                    },
                    if open {
                        p.primary.base.color
                    } else {
                        Color {
                            a: 0.12,
                            ..p.primary.base.color
                        }
                    },
                );
            }

            renderer.fill_text(
                text::Text {
                    bounds: bounds.size(),
                    align_y: alignment::Vertical::Center,
                    align_x: text::Alignment::Center,
                    ..text_spec(renderer, state.labels[index].clone(), LABEL_SIZE)
                },
                bounds.center(),
                if open {
                    p.primary.base.text
                } else {
                    p.background.base.text
                },
                bounds,
            );
        }
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let Some(position) = cursor.position() else {
            return mouse::Interaction::None;
        };
        if root_at(layout, position).is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        _renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let state = tree.state.downcast_mut::<MenuBarState>();
        if state.open.is_empty() {
            return None;
        }
        let root_layout = layout.children().nth(*state.open.first()?)?;
        let bounds = root_layout.bounds();
        Some(overlay::Element::new(Box::new(MenuOverlay {
            roots: &mut self.menus,
            trees: &mut tree.children,
            bar_bounds: Some(Rectangle::new(
                layout.bounds().position() + translation,
                layout.bounds().size(),
            )),
            open: &mut state.open,
            origin: bounds.position() + translation,
            root_height: bounds.height,
            viewport: *viewport,
        })))
    }
}

fn root_at(layout: Layout<'_>, position: Point) -> Option<usize> {
    layout
        .children()
        .enumerate()
        .find(|(_, root)| root.bounds().contains(position))
        .map(|(index, _)| index)
}

struct Panel {
    bounds: Rectangle,
    rows: Vec<Rectangle>,
}

struct MenuOverlay<'a, 'b, Message> {
    roots: &'a mut [Menu<'b, Message>],
    trees: &'a mut [Tree],
    bar_bounds: Option<Rectangle>,
    open: &'a mut Vec<usize>,
    origin: Point,
    root_height: f32,
    viewport: Rectangle,
}

impl<'a, 'b, Message> MenuOverlay<'a, 'b, Message> {
    fn menu_at(&self, path: &[usize]) -> Option<(&Menu<'b, Message>, &Tree)> {
        let mut current = self.roots.get(*path.first()?)?;
        let mut tree = self.trees.get(*path.first()?)?;
        for &index in &path[1..] {
            let Item::Submenu { submenu, .. } = current.items.get(index)? else {
                return None;
            };
            current = submenu;
            tree = tree.children.get(index)?;
        }
        Some((current, tree))
    }

    fn menu_at_mut(&mut self, level: usize) -> Option<(&mut Menu<'b, Message>, &mut Tree)> {
        let path = self.open.get(..level + 1)?;
        let mut menu = self.roots.get_mut(*path.first()?)?;
        let mut tree = self.trees.get_mut(*path.first()?)?;
        for &index in &path[1..] {
            let Item::Submenu { submenu, .. } = menu.items.get_mut(index)? else {
                return None;
            };
            menu = submenu;
            tree = tree.children.get_mut(index)?;
        }
        Some((menu, tree))
    }

    fn resolve_item(&self, level: usize, index: usize) -> Option<&Item<'b, Message>> {
        self.menu_at(&self.open[..level + 1])?.0.items.get(index)
    }

    fn panels(layout: Layout<'_>) -> Vec<Panel> {
        layout
            .children()
            .map(|panel| Panel {
                bounds: panel.bounds(),
                rows: panel.children().map(|row| row.bounds()).collect(),
            })
            .collect()
    }

    fn hit_test(panels: &[Panel], position: Point) -> Option<(usize, usize)> {
        for level in (0..panels.len()).rev() {
            let panel = &panels[level];
            if panel.bounds.contains(position) {
                for (index, row) in panel.rows.iter().enumerate() {
                    if row.contains(position) {
                        return Some((level, index));
                    }
                }
            }
        }
        None
    }
}

impl<Message> overlay::Overlay<Message, Theme, Renderer> for MenuOverlay<'_, '_, Message>
where
    Message: Clone,
{
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let viewport = self.viewport;
        let mut panels = Vec::<layout::Node>::new();
        for level in 0..self.open.len() {
            let Some((menu, tree)) = self.menu_at_mut(level) else {
                break;
            };
            let max_width = (viewport.width - ITEM_PADDING_X * 2.0 - 2.0).max(0.0);
            let limits = layout::Limits::with_compression(
                Size::ZERO,
                Size::new(max_width, viewport.height),
                Size::new(true, true),
            );
            let mut width = menu.content_width();
            for (item, tree) in menu.items.iter_mut().zip(&mut tree.children) {
                if let Item::Element { element, .. } = item {
                    let node = element.as_widget_mut().layout(tree, renderer, &limits);
                    width = width.max(node.size().width + ITEM_PADDING_X * 2.0 + 2.0);
                }
            }
            let width = width.min(viewport.width);
            let limits = layout::Limits::new(
                Size::ZERO,
                Size::new(
                    (width - ITEM_PADDING_X * 2.0 - 2.0).max(0.0),
                    viewport.height,
                ),
            );
            let mut rows = Vec::new();
            let mut y = PANEL_PADDING;
            for (item, tree) in menu.items.iter_mut().zip(&mut tree.children) {
                let mut height = item.height();
                let children = if let Item::Element { element, .. } = item {
                    let node = element.as_widget_mut().layout(tree, renderer, &limits);
                    height = height.max(node.size().height);
                    let offset = (height - node.size().height) / 2.0;
                    vec![node.move_to(Point::new(ITEM_PADDING_X, offset))]
                } else {
                    Vec::new()
                };
                rows.push(
                    layout::Node::with_children(
                        Size::new((width - 2.0).max(0.0), height),
                        children,
                    )
                    .move_to(Point::new(1.0, y)),
                );
                y += height;
            }
            let height = y + PANEL_PADDING;
            let anchor = if let Some(parent) = panels.last() {
                let parent_bounds = parent.bounds();
                let row = parent.children()[self.open[level]].bounds();
                Point::new(
                    if parent_bounds.x + parent_bounds.width + 1.0 + width
                        > viewport.x + viewport.width
                    {
                        parent_bounds.x - width - 1.0
                    } else {
                        parent_bounds.x + parent_bounds.width + 1.0
                    },
                    parent_bounds.y + row.y - PANEL_PADDING,
                )
            } else {
                Point::new(self.origin.x, self.origin.y + self.root_height + 1.0)
            };
            let position = Point::new(
                anchor.x.clamp(
                    viewport.x,
                    (viewport.x + viewport.width - width).max(viewport.x),
                ),
                anchor.y.clamp(
                    viewport.y,
                    (viewport.y + viewport.height - height).max(viewport.y),
                ),
            );
            panels.push(
                layout::Node::with_children(Size::new(width, height), rows).move_to(position),
            );
        }
        layout::Node::with_children(bounds, panels)
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        let panels = Self::panels(layout);
        let hit = cursor
            .position()
            .and_then(|position| Self::hit_test(&panels, position));
        let mut close = false;
        for (level, panel) in layout.children().enumerate().rev() {
            let Some((menu, tree)) = self.menu_at_mut(level) else {
                continue;
            };
            for (index, ((item, tree), row)) in menu
                .items
                .iter_mut()
                .zip(&mut tree.children)
                .zip(panel.children())
                .enumerate()
            {
                if let Item::Element {
                    element,
                    close_on_message,
                    ..
                } = item
                {
                    let mut bus = Bus::default();
                    let mut local = shell.local(&mut bus);
                    element.as_widget_mut().update(
                        tree,
                        event,
                        row.children().next().expect("element layout"),
                        if hit == Some((level, index)) {
                            cursor
                        } else {
                            mouse::Cursor::Unavailable
                        },
                        renderer,
                        &mut local,
                        &panel.bounds(),
                    );
                    close |= *close_on_message && !local.is_empty();
                    shell.merge(local, |message| message);
                }
            }
        }
        if close {
            self.open.clear();
            shell.capture_event();
            shell.invalidate_layout();
            shell.request_redraw();
            return;
        }

        match event {
            Event::Pointer(pointer::Event::PointerMoved { position, .. }) => {
                if let Some((level, index)) = Self::hit_test(&panels, *position) {
                    let opens_submenu =
                        matches!(self.resolve_item(level, index), Some(Item::Submenu { .. }));
                    let changed = if opens_submenu {
                        if self.open.get(level + 1) != Some(&index) || self.open.len() > level + 2 {
                            self.open.truncate(level + 1);
                            self.open.push(index);
                            true
                        } else {
                            false
                        }
                    } else if self.open.len() > level + 1 {
                        self.open.truncate(level + 1);
                        true
                    } else {
                        false
                    };
                    if changed {
                        shell.invalidate_layout();
                        shell.request_redraw();
                    }
                }
            }
            Event::Pointer(event @ pointer::Event::PointerPressed { position, .. })
                if event.is_primary_press() =>
            {
                if self
                    .bar_bounds
                    .is_some_and(|bounds| bounds.contains(*position))
                {
                    return;
                }

                let hit = Self::hit_test(&panels, *position);
                match hit {
                    Some((level, index)) => match self.resolve_item(level, index) {
                        Some(Item::Action { message, .. }) => {
                            shell.publish(message.clone());
                            self.open.clear();
                        }
                        Some(Item::Submenu { .. }) => {
                            self.open.truncate(level + 1);
                            self.open.push(index);
                        }
                        _ => {}
                    },
                    None if panels.iter().any(|panel| panel.bounds.contains(*position)) => {}
                    None => {
                        self.open.clear();
                    }
                }
                shell.capture_event();
                shell.invalidate_layout();
                shell.request_redraw();
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            }) => {
                self.open.clear();
                shell.capture_event();
                shell.invalidate_layout();
                shell.request_redraw();
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let panels = Self::panels(layout);
        let p = theme.palette();
        let hit = cursor
            .position()
            .and_then(|position| Self::hit_test(&panels, position));
        for (level, panel) in panels.iter().enumerate() {
            let (menu, tree) = self
                .menu_at(&self.open[..level + 1])
                .expect("open path resolves during layout");
            {
                let hovered = hit.filter(|(l, _)| *l == level).map(|(_, index)| index);
                let opened_submenu = self.open.get(level + 1).copied();
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: panel.bounds,
                        border: Border {
                            radius: 0.0.into(),
                            width: 1.0,
                            color: p.background.strong.color,
                        },
                        shadow: Shadow {
                            color: Color::BLACK.scale_alpha(0.25),
                            offset: Vector::new(3.0, 3.0),
                            blur_radius: 0.0,
                        },
                        ..renderer::Quad::default()
                    },
                    p.background.weakest.color,
                );
                for (index, row) in panel.rows.iter().enumerate() {
                    match &menu.items[index] {
                        Item::Separator => {
                            let y = row.y + row.height / 2.0;
                            renderer.fill_quad(
                                renderer::Quad {
                                    bounds: Rectangle::new(
                                        Point::new(row.x + ITEM_PADDING_X, y),
                                        Size::new(row.width - ITEM_PADDING_X * 2.0, 1.0),
                                    ),
                                    ..renderer::Quad::default()
                                },
                                p.background.strong.color,
                            );
                        }
                        Item::Action {
                            label, shortcut, ..
                        } => {
                            draw_entry(
                                renderer,
                                p,
                                panel.bounds,
                                *row,
                                label,
                                hovered == Some(index),
                                shortcut.as_deref(),
                                false,
                            );
                        }
                        Item::Element { element, .. } => {
                            let child_layout = layout
                                .children()
                                .nth(level)
                                .expect("panel layout")
                                .children()
                                .nth(index)
                                .expect("row layout")
                                .children()
                                .next()
                                .expect("element layout");
                            element.as_widget().draw(
                                &tree.children[index],
                                renderer,
                                theme,
                                style,
                                child_layout,
                                if hit == Some((level, index)) {
                                    cursor
                                } else {
                                    mouse::Cursor::Unavailable
                                },
                                &panel.bounds,
                            );
                        }
                        Item::Submenu { label, .. } => {
                            draw_entry(
                                renderer,
                                p,
                                panel.bounds,
                                *row,
                                label,
                                hovered == Some(index) || opened_submenu == Some(index),
                                None,
                                true,
                            );
                        }
                    }
                }
            };
        }
    }

    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        for (level, panel) in layout.children().enumerate() {
            let Some((menu, tree)) = self.menu_at_mut(level) else {
                continue;
            };
            operation.container(None, panel.bounds());
            operation.traverse(&mut |operation| {
                for ((item, tree), row) in menu
                    .items
                    .iter_mut()
                    .zip(&mut tree.children)
                    .zip(panel.children())
                {
                    if let Item::Element { element, .. } = item {
                        element.as_widget_mut().operate(
                            tree,
                            row.children().next().expect("element layout"),
                            renderer,
                            operation,
                        );
                    }
                }
            });
        }
    }

    fn overlay<'b>(
        &'b mut self,
        layout: Layout<'b>,
        renderer: &Renderer,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let root = *self.open.first()?;
        let overlays = self.roots.get_mut(root)?.overlays(
            self.trees.get_mut(root)?,
            &self.open[1..],
            &mut layout.children(),
            renderer,
            &self.viewport,
        );
        if overlays.is_empty() {
            return None;
        }
        Some(overlay::Element::new(Box::new(MenuElementOverlay {
            content: overlay::Group::with_children(overlays).overlay(),
            open: self.open,
        })))
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let panels = Self::panels(layout);
        if let Some((level, index)) = cursor
            .position()
            .and_then(|position| Self::hit_test(&panels, position))
            && let Some((menu, tree)) = self.menu_at(&self.open[..level + 1])
            && let Item::Element { element, .. } = &menu.items[index]
        {
            return element.as_widget().mouse_interaction(
                &tree.children[index],
                layout
                    .children()
                    .nth(level)
                    .expect("panel layout")
                    .children()
                    .nth(index)
                    .expect("row layout")
                    .children()
                    .next()
                    .expect("element layout"),
                cursor,
                &panels[level].bounds,
                renderer,
            );
        }
        cursor
            .position()
            .and_then(|position| Self::hit_test(&panels, position))
            .map(|_| mouse::Interaction::Pointer)
            .unwrap_or(mouse::Interaction::None)
    }
}

struct MenuElementOverlay<'a, Message> {
    content: overlay::Element<'a, (Message, bool), Theme, Renderer>,
    open: &'a mut Vec<usize>,
}

impl<Message> overlay::Overlay<Message, Theme, Renderer> for MenuElementOverlay<'_, Message> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        self.content.as_overlay_mut().layout(renderer, bounds)
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.content
            .as_overlay()
            .draw(renderer, theme, style, layout, cursor);
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        let mut bus = Bus::default();
        let mut local = shell.local(&mut bus);
        self.content
            .as_overlay_mut()
            .update(event, layout, cursor, renderer, &mut local);
        let close = Cell::new(false);
        shell.merge(local, |(message, close_on_message)| {
            close.set(close.get() || close_on_message);
            message
        });
        if close.get() {
            self.open.clear();
            shell.capture_event();
            shell.invalidate_layout();
            shell.request_redraw();
        }
    }

    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_overlay_mut()
            .operate(layout, renderer, operation);
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_overlay()
            .mouse_interaction(layout, cursor, renderer)
    }

    fn overlay<'a>(
        &'a mut self,
        layout: Layout<'a>,
        renderer: &Renderer,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        self.content
            .as_overlay_mut()
            .overlay(layout, renderer)
            .map(|content| {
                overlay::Element::new(Box::new(MenuElementOverlay {
                    content,
                    open: self.open,
                }))
            })
    }

    fn index(&self) -> f32 {
        self.content.as_overlay().index()
    }
}

fn draw_entry(
    renderer: &mut Renderer,
    p: &Palette,
    panel_bounds: Rectangle,
    row: Rectangle,
    label: &str,
    hovered: bool,
    shortcut: Option<&str>,
    chevron: bool,
) {
    if hovered {
        renderer.fill_quad(
            renderer::Quad {
                bounds: Rectangle::new(
                    Point::new(row.x + PANEL_PADDING - 1.0, row.y),
                    Size::new(row.width - PANEL_PADDING * 2.0 + 2.0, row.height),
                ),
                ..renderer::Quad::default()
            },
            p.primary.base.color,
        );
    }
    let text_color = if hovered {
        p.primary.base.text
    } else {
        p.background.base.text
    };
    let icon_color = if hovered {
        p.primary.base.text
    } else {
        p.background.weak.text
    };
    renderer.fill_text(
        text::Text {
            bounds: Size::new(row.width - ITEM_PADDING_X * 2.0 - ICON_SLOT, row.height),
            align_y: alignment::Vertical::Center,
            ..text_spec(renderer, label.to_owned(), LABEL_SIZE)
        },
        Point::new(row.x + ITEM_PADDING_X + ICON_SLOT, row.y + row.height / 2.0),
        text_color,
        panel_bounds,
    );
    if let Some(shortcut) = shortcut {
        renderer.fill_text(
            text::Text {
                bounds: Size::new(row.width - ITEM_PADDING_X * 2.0 - ICON_SLOT, row.height),
                align_x: text::Alignment::Right,
                align_y: alignment::Vertical::Center,
                ..text_spec(renderer, shortcut.to_owned(), SHORTCUT_SIZE)
            },
            Point::new(row.x + row.width - ITEM_PADDING_X, row.y + row.height / 2.0),
            if hovered {
                text_color
            } else {
                p.background.weak.text
            },
            panel_bounds,
        );
    }
    if chevron {
        draw_svg_icon(
            renderer,
            include_bytes!("../assets/icons/chevron_right.svg"),
            icon_color,
            Rectangle::new(
                Point::new(
                    row.x + row.width - ITEM_PADDING_X - 4.0,
                    row.y + (row.height - 10.0) / 2.0,
                ),
                Size::new(10.0, 10.0),
            ),
            panel_bounds,
        );
    }
}

fn draw_svg_icon(
    renderer: &mut Renderer,
    bytes: &'static [u8],
    color: Color,
    bounds: Rectangle,
    clip_bounds: Rectangle,
) {
    renderer.draw_svg(
        svg::Svg {
            handle: svg::Handle::from_memory(bytes),
            color: Some(color),
            rotation: Radians(0.0),
            opacity: 1.0,
        },
        bounds,
        clip_bounds,
    );
}

impl<'a, Message> From<MenuBar<'a, Message>> for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
{
    fn from(value: MenuBar<'a, Message>) -> Self {
        Element::new(value)
    }
}

pub struct ContextMenu<'a, Message> {
    underlay: Element<'a, Message, Theme, Renderer>,
    menu: Menu<'a, Message>,
}

#[derive(Default)]
struct ContextMenuState {
    show: bool,
    cursor_position: Point,
    open: Vec<usize>,
}

impl<'a, Message> ContextMenu<'a, Message> {
    pub fn new(
        underlay: impl Into<Element<'a, Message, Theme, Renderer>>,
        menu: Menu<'a, Message>,
    ) -> Self {
        Self {
            underlay: underlay.into(),
            menu,
        }
    }
}

impl<Message: Clone> Widget<Message, Theme, Renderer> for ContextMenu<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<ContextMenuState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(ContextMenuState::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.children.resize_with(2, Tree::empty);
        tree.children[0].diff(self.underlay.as_widget_mut());
        self.menu.diff(&mut tree.children[1]);
    }

    fn size(&self) -> Size<Length> {
        self.underlay.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.underlay
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let Event::Pointer(event) = event
            && event.is_secondary_press()
            && cursor.is_over(layout.bounds())
        {
            let state = tree.state.downcast_mut::<ContextMenuState>();
            if !state.show || state.open.is_empty() {
                state.show = true;
                state.open = vec![0];
                state.cursor_position = cursor.position().unwrap_or_default();
            } else {
                state.show = false;
                state.open.clear();
            }
            shell.capture_event();
            shell.invalidate_layout();
            shell.request_redraw();
        }

        self.underlay.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            _renderer,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.underlay.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
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
        self.underlay.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let state = tree.state.downcast_mut::<ContextMenuState>();
        if !state.show || state.open.is_empty() {
            return self.underlay.as_widget_mut().overlay(
                &mut tree.children[0],
                layout,
                renderer,
                viewport,
                translation,
            );
        }

        let origin = state.cursor_position;
        Some(overlay::Element::new(Box::new(MenuOverlay {
            roots: slice::from_mut(&mut self.menu),
            trees: &mut tree.children[1..],
            bar_bounds: None,
            open: &mut state.open,
            origin: origin + translation,
            root_height: 0.0,
            viewport: *viewport,
        })))
    }
}

impl<'a, Message: Clone + 'a> From<ContextMenu<'a, Message>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(value: ContextMenu<'a, Message>) -> Self {
        Element::new(value)
    }
}
