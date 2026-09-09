use iced_core::Renderer as _;
use iced_core::text::{Paragraph as _, Renderer as _};
use iced_core::{
    Border, Clipboard, Color, Element, Event, Font, Layout, Length, Point, Radians, Rectangle,
    Shadow, Shell, Size, Theme, Vector, Widget, alignment, keyboard, layout, mouse, overlay,
    renderer, svg, text,
    widget::{Tree, tree},
};
use iced_wgpu::Renderer;

const ROOT_PADDING: f32 = 8.0;
const ITEM_HEIGHT: f32 = 24.0;
const ITEM_PADDING_X: f32 = 10.0;
const ICON_SLOT: f32 = 14.0;
const SEPARATOR_HEIGHT: f32 = 9.0;
const PANEL_PADDING: f32 = 4.0;
const LABEL_SIZE: f32 = 12.0;
const SHORTCUT_SIZE: f32 = 10.0;

pub enum Item<Message> {
    Action {
        label: String,
        shortcut: Option<String>,
        message: Message,
        checked: bool,
    },
    Submenu {
        label: String,
        submenu: Menu<Message>,
    },
    Separator,
}

impl<Message> Item<Message> {
    fn height(&self) -> f32 {
        match self {
            Item::Action { .. } | Item::Submenu { .. } => ITEM_HEIGHT,
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
            Item::Submenu { label, .. } => {
                ITEM_PADDING_X * 2.0 + ICON_SLOT + text_width(label, LABEL_SIZE) + 14.0
            }
        }
    }
}

pub struct Menu<Message> {
    items: Vec<Item<Message>>,
    min_width: f32,
}

impl<Message> Menu<Message> {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            min_width: 0.0,
        }
    }

    fn content_width(&self) -> f32 {
        self.items
            .iter()
            .map(Item::width)
            .fold(self.min_width, f32::max)
    }

    fn action(
        mut self,
        label: String,
        shortcut: Option<String>,
        message: Message,
        checked: bool,
    ) -> Self {
        self.items.push(Item::Action {
            label,
            shortcut,
            message,
            checked,
        });
        self
    }

    pub fn item(self, label: impl Into<String>, message: Message) -> Self {
        self.action(label.into(), None, message, false)
    }

    pub fn item_shortcut(self, label: impl Into<String>, shortcut: &str, message: Message) -> Self {
        self.action(label.into(), Some(String::from(shortcut)), message, false)
    }

    pub fn selected_item(self, label: impl Into<String>, message: Message) -> Self {
        self.action(label.into(), None, message, true)
    }

    pub fn selected_item_shortcut(
        self,
        label: impl Into<String>,
        shortcut: &str,
        message: Message,
    ) -> Self {
        self.action(label.into(), Some(String::from(shortcut)), message, true)
    }

    pub fn separator(mut self) -> Self {
        self.items.push(Item::Separator);
        self
    }

    pub fn submenu(mut self, label: impl Into<String>, submenu: Menu<Message>) -> Self {
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

    pub fn get_item_mut(&mut self, name: &str) -> Option<&mut Item<Message>> {
        self.items
            .iter_mut()
            .find(|item| matches!(item, Item::Action { label, .. } | Item::Submenu { label, .. } if label == name))
    }
}

impl<Message> Default for Menu<Message> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MenuBar<Message> {
    labels: Vec<String>,
    menus: Vec<Menu<Message>>,
    width: Length,
    height: Length,
}

impl<Message> MenuBar<Message> {
    pub fn new() -> Self {
        Self {
            labels: Vec::new(),
            menus: Vec::new(),
            width: Length::Shrink,
            height: Length::Fixed(28.0),
        }
    }

    pub fn menu(mut self, label: impl Into<String>, menu: Menu<Message>) -> Self {
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

    pub fn get_menu_mut(&mut self, category: &str) -> Option<&mut Menu<Message>> {
        self.labels
            .iter()
            .position(|name| name == category)
            .and_then(|index| self.menus.get_mut(index))
    }
}

impl<Message> Default for MenuBar<Message> {
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
    }
}

impl<Message> Widget<Message, Theme, Renderer> for MenuBar<Message>
where
    Message: Clone,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<MenuBarState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(MenuBarState::default())
    }

    fn diff(&self, tree: &mut Tree) {
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
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<MenuBarState>();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if state.open.is_empty() =>
            {
                let Some(position) = cursor.position() else {
                    return;
                };
                let Some(index) = root_at(layout, position) else {
                    return;
                };
                state.open = vec![index];
                shell.capture_event();
                shell.invalidate_layout();
                shell.request_redraw();
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if !state.open.is_empty() => {
                if let Some(index) = root_at(layout, *position)
                    && state.open[0] != index
                {
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
        let p = theme.extended_palette();
        for (index, root_layout) in layout.children().enumerate() {
            let bounds = root_layout.bounds();
            let open = state.open.first() == Some(&index);
            let hovered = !open && cursor.is_over(bounds);
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
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<MenuBarState>();
        let Some(position) = cursor.position() else {
            return mouse::Interaction::None;
        };
        if state.open.is_empty() && root_at(layout, position).is_some() {
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
            roots: &self.menus,
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

struct MenuOverlay<'a, Message> {
    roots: &'a [Menu<Message>],
    open: &'a mut Vec<usize>,
    origin: Point,
    root_height: f32,
    viewport: Rectangle,
}

impl<'a, Message> MenuOverlay<'a, Message> {
    fn menu_at(&self, path: &[usize]) -> Option<&Menu<Message>> {
        let mut current = self.roots.get(*path.first()?)?;
        for &index in &path[1..] {
            let Item::Submenu { submenu, .. } = current.items.get(index)? else {
                return None;
            };
            current = submenu;
        }
        Some(current)
    }

    fn resolve_item(&self, level: usize, index: usize) -> Option<&Item<Message>> {
        self.menu_at(&self.open[..level + 1])?.items.get(index)
    }

    fn panels(&self) -> Vec<Panel> {
        let viewport = self.viewport;
        let mut panels = Vec::new();
        let mut anchor = Point::new(self.origin.x, self.origin.y + self.root_height + 1.0);
        let mut level = 0;
        while let Some(menu) = self.menu_at(&self.open[..level + 1]) {
            let width = menu.content_width();
            let height = PANEL_PADDING * 2.0 + menu.items.iter().map(Item::height).sum::<f32>();
            let bounds = Rectangle::new(
                Point::new(
                    anchor.x.min((viewport.x + viewport.width - width).max(0.0)),
                    anchor
                        .y
                        .min((viewport.y + viewport.height - height).max(0.0)),
                ),
                Size::new(width, height),
            );

            let mut rows = Vec::new();
            let mut y = bounds.y + PANEL_PADDING;
            for item in &menu.items {
                let height = item.height();
                rows.push(Rectangle::new(
                    Point::new(bounds.x + 1.0, y),
                    Size::new(bounds.width - 2.0, height),
                ));
                y += height;
            }

            let next = self
                .open
                .get(level + 1)
                .and_then(|&index| rows.get(index).copied());
            panels.push(Panel { bounds, rows });
            match next {
                Some(row) => {
                    let submenu_width = self
                        .menu_at(&self.open[..level + 2])
                        .map(Menu::content_width)
                        .unwrap_or(0.0);
                    anchor = Point::new(
                        if bounds.x + bounds.width + 1.0 + submenu_width
                            > viewport.x + viewport.width
                        {
                            bounds.x - submenu_width - 1.0
                        } else {
                            bounds.x + bounds.width + 1.0
                        },
                        row.y - PANEL_PADDING,
                    );
                    level += 1;
                }
                None => break,
            }
        }
        panels
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

impl<Message> overlay::Overlay<Message, Theme, Renderer> for MenuOverlay<'_, Message>
where
    Message: Clone,
{
    fn layout(&mut self, _renderer: &Renderer, bounds: Size) -> layout::Node {
        layout::Node::new(bounds)
    }

    fn update(
        &mut self,
        event: &Event,
        _layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        match event {
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let panels = self.panels();
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
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let panels = self.panels();
                let hit = cursor
                    .position()
                    .and_then(|position| Self::hit_test(&panels, position));
                match hit {
                    Some((level, index)) => {
                        if let Some(Item::Action { message, .. }) = self.resolve_item(level, index)
                        {
                            shell.publish(message.clone());
                            self.open.clear();
                        }
                    }
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
        _style: &renderer::Style,
        _layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let panels = self.panels();
        let p = theme.extended_palette();
        let hit = cursor
            .position()
            .and_then(|position| Self::hit_test(&panels, position));
        for (level, panel) in panels.iter().enumerate() {
            let menu = self
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
                            label,
                            shortcut,
                            checked,
                            ..
                        } => {
                            draw_entry(
                                renderer,
                                p,
                                panel.bounds,
                                *row,
                                label,
                                hovered == Some(index),
                                *checked,
                                shortcut.as_deref(),
                                false,
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
                                false,
                                None,
                                true,
                            );
                        }
                    }
                }
            };
        }
    }

    fn mouse_interaction(
        &self,
        _layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let panels = self.panels();
        cursor
            .position()
            .and_then(|position| Self::hit_test(&panels, position))
            .map(|_| mouse::Interaction::Pointer)
            .unwrap_or(mouse::Interaction::None)
    }
}

fn draw_entry(
    renderer: &mut Renderer,
    p: &iced_core::theme::palette::Extended,
    panel_bounds: Rectangle,
    row: Rectangle,
    label: &str,
    hovered: bool,
    checked: bool,
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
    if checked {
        draw_svg_icon(
            renderer,
            include_bytes!("../assets/icons/check.svg"),
            icon_color,
            Rectangle::new(
                Point::new(
                    row.x + ITEM_PADDING_X + 2.0,
                    row.y + (row.height - 10.0) / 2.0,
                ),
                Size::new(10.0, 10.0),
            ),
            panel_bounds,
        );
    }
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
    use iced_core::svg::Renderer as _;
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

impl<'a, Message> From<MenuBar<Message>> for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
{
    fn from(value: MenuBar<Message>) -> Self {
        Element::new(value)
    }
}

pub struct ContextMenu<'a, Message> {
    underlay: Element<'a, Message, Theme, Renderer>,
    menu: Menu<Message>,
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
        menu: Menu<Message>,
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

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.underlay)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.underlay]);
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
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if *event == Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right))
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
            _clipboard,
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
            roots: std::slice::from_ref(&self.menu),
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
