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
}

pub struct Menu<Message> {
    items: Vec<Item<Message>>,
    width: f32,
}

impl<Message> Menu<Message> {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            width: 180.0,
        }
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

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
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
    roots: Vec<(String, Menu<Message>)>,
    width: Length,
    height: Length,
}

impl<Message> MenuBar<Message> {
    pub fn new() -> Self {
        Self {
            roots: Vec::new(),
            width: Length::Shrink,
            height: Length::Fixed(28.0),
        }
    }

    pub fn menu(mut self, label: impl Into<String>, menu: Menu<Message>) -> Self {
        self.roots.push((label.into(), menu));
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
        self.roots
            .iter_mut()
            .find(|(name, _)| name == category)
            .map(|(_, menu)| menu)
    }

    fn menu_at(&self, path: &[usize]) -> Option<&Menu<Message>> {
        let mut current = &self.roots.get(*path.first()?)?.1;
        for &index in &path[1..] {
            let Item::Submenu { submenu, .. } = current.items.get(index)? else {
                return None;
            };
            current = submenu;
        }
        Some(current)
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

fn text_spec(content: String, size: f32) -> text::Text<String, Font> {
    text::Text {
        content,
        font: Font::DEFAULT,
        size: size.into(),
        line_height: text::LineHeight::default(),
        bounds: Size::new(f32::MAX, f32::MAX),
        align_x: text::Alignment::Left,
        align_y: alignment::Vertical::Top,
        shaping: text::Shaping::Basic,
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
        let labels = self
            .roots
            .iter()
            .map(|(label, _)| label.clone())
            .collect::<Vec<_>>();
        if state.labels != labels {
            state.labels = labels;
            state.label_widths = state
                .labels
                .iter()
                .map(|label| {
                    let paragraph =
                        <Renderer as text::Renderer>::Paragraph::with_text(text::Text {
                            content: label.as_str(),
                            font: Font::DEFAULT,
                            size: LABEL_SIZE.into(),
                            line_height: text::LineHeight::default(),
                            bounds: Size::new(f32::MAX, f32::MAX),
                            align_x: text::Alignment::Left,
                            align_y: alignment::Vertical::Top,
                            shaping: text::Shaping::Basic,
                            wrapping: text::Wrapping::None,
                        });
                    paragraph.min_bounds().width
                })
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
            let mut text = text_spec(state.labels[index].clone(), LABEL_SIZE);
            text.bounds = bounds.size();
            text.align_y = alignment::Vertical::Center;
            text.align_x = text::Alignment::Center;
            renderer.fill_text(
                text,
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
            state,
            menu_bar: self,
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
    state: &'a mut MenuBarState,
    menu_bar: &'a MenuBar<Message>,
    origin: Point,
    root_height: f32,
    viewport: Rectangle,
}

impl<'a, Message> MenuOverlay<'a, Message> {
    fn resolve_item(&self, level: usize, index: usize) -> Option<&Item<Message>> {
        self.menu_bar
            .menu_at(&self.state.open[..level + 1])?
            .items
            .get(index)
    }

    fn panels(&self) -> Vec<Panel> {
        let viewport = self.viewport;
        let mut panels = Vec::new();
        let mut anchor = Point::new(self.origin.x, self.origin.y + self.root_height + 1.0);
        let mut level = 0;
        while let Some(menu) = self.menu_bar.menu_at(&self.state.open[..level + 1]) {
            let height = PANEL_PADDING * 2.0 + menu.items.iter().map(Item::height).sum::<f32>();
            let width = menu.width;
            let bounds = Rectangle::new(
                Point::new(
                    anchor.x.min((viewport.x + viewport.width - width).max(0.0)),
                    anchor
                        .y
                        .min((viewport.y + viewport.height - height).max(0.0)),
                ),
                Size::new(width, height),
            );
            let rows = {
                let mut rows = Vec::new();
                let mut y = bounds.y + PANEL_PADDING;
                for item in &menu.items {
                    let height = item.height();
                    rows.push(Rectangle::new(
                        Point::new(bounds.x + 1.0, y),
                        Size::new(width - 2.0, height),
                    ));
                    y += height;
                }
                rows
            };
            let next = self
                .state
                .open
                .get(level + 1)
                .and_then(|&index| rows.get(index).copied());
            panels.push(Panel { bounds, rows });
            match next {
                Some(row) => {
                    let submenu_width = self
                        .menu_bar
                        .menu_at(&self.state.open[..level + 2])
                        .map(|menu| menu.width)
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
                        if self.state.open.get(level + 1) != Some(&index)
                            || self.state.open.len() > level + 2
                        {
                            self.state.open.truncate(level + 1);
                            self.state.open.push(index);
                            true
                        } else {
                            false
                        }
                    } else if self.state.open.len() > level + 1 {
                        self.state.open.truncate(level + 1);
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
                            self.state.open.clear();
                        }
                    }
                    None => {
                        self.state.open.clear();
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
                self.state.open.clear();
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
                .menu_bar
                .menu_at(&self.state.open[..level + 1])
                .expect("open path resolves during layout");
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
                let item = &menu.items[index];
                match item {
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
                        let hovered = hit == Some((level, index));
                        draw_entry(
                            renderer,
                            p,
                            panel.bounds,
                            *row,
                            label,
                            hovered,
                            *checked,
                            shortcut.as_deref(),
                            false,
                        );
                    }
                    Item::Submenu { label, .. } => {
                        let opened = level + 1 < self.state.open.len()
                            && self.state.open[level + 1] == index;
                        let hovered = opened || hit == Some((level, index));
                        draw_entry(
                            renderer,
                            p,
                            panel.bounds,
                            *row,
                            label,
                            hovered,
                            false,
                            None,
                            true,
                        );
                    }
                }
            }
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
    let mut label_text = text_spec(label.to_owned(), LABEL_SIZE);
    label_text.bounds = Size::new(row.width - ITEM_PADDING_X * 2.0 - ICON_SLOT, row.height);
    label_text.align_y = alignment::Vertical::Center;
    renderer.fill_text(
        label_text,
        Point::new(row.x + ITEM_PADDING_X + ICON_SLOT, row.y + row.height / 2.0),
        text_color,
        panel_bounds,
    );
    if let Some(shortcut) = shortcut {
        let mut shortcut_text = text_spec(shortcut.to_owned(), SHORTCUT_SIZE);
        shortcut_text.bounds = Size::new(row.width - ITEM_PADDING_X * 2.0 - ICON_SLOT, row.height);
        shortcut_text.align_x = text::Alignment::Right;
        shortcut_text.align_y = alignment::Vertical::Center;
        renderer.fill_text(
            shortcut_text,
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

pub struct MenuPanel<Message> {
    menu: Menu<Message>,
}

impl<Message> MenuPanel<Message> {
    pub fn new(menu: Menu<Message>) -> Self {
        Self { menu }
    }

    fn height(&self) -> f32 {
        PANEL_PADDING * 2.0 + self.menu.items.iter().map(Item::height).sum::<f32>()
    }

    fn row_at(&self, bounds: Rectangle, position: Point) -> Option<usize> {
        if !bounds.contains(position) {
            return None;
        }
        let mut y = bounds.y + PANEL_PADDING;
        for (index, item) in self.menu.items.iter().enumerate() {
            let height = item.height();
            if position.y >= y && position.y < y + height {
                return Some(index);
            }
            y += height;
        }
        None
    }
}

impl<Message: Clone> Widget<Message, Theme, Renderer> for MenuPanel<Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.menu.width), Length::Shrink)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let intrinsic = Size::new(self.menu.width, self.height());
        layout::Node::new(limits.resolve(Length::Fixed(self.menu.width), Length::Shrink, intrinsic))
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event
            && let Some(position) = cursor.position()
            && let Some(index) = self.row_at(layout.bounds(), position)
            && let Some(Item::Action { message, .. }) = self.menu.items.get(index)
        {
            shell.publish(message.clone());
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let p = theme.extended_palette();
        renderer.fill_quad(
            renderer::Quad {
                bounds,
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
        let hovered = cursor
            .position()
            .and_then(|position| self.row_at(bounds, position));
        let mut y = bounds.y + PANEL_PADDING;
        for (index, item) in self.menu.items.iter().enumerate() {
            let height = item.height();
            let row = Rectangle::new(
                Point::new(bounds.x + 1.0, y),
                Size::new(bounds.width - 2.0, height),
            );
            match item {
                Item::Separator => {
                    let mid = row.y + row.height / 2.0;
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle::new(
                                Point::new(row.x + ITEM_PADDING_X, mid),
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
                        bounds,
                        row,
                        label,
                        hovered == Some(index),
                        *checked,
                        shortcut.as_deref(),
                        false,
                    );
                }
                Item::Submenu { label, .. } => {
                    draw_entry(renderer, p, bounds, row, label, false, false, None, true);
                }
            }
            y += height;
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
        match cursor
            .position()
            .and_then(|position| self.row_at(layout.bounds(), position))
        {
            Some(_) => mouse::Interaction::Pointer,
            None => mouse::Interaction::None,
        }
    }
}

impl<'a, Message: Clone + 'a> From<MenuPanel<Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: MenuPanel<Message>) -> Self {
        Element::new(value)
    }
}
