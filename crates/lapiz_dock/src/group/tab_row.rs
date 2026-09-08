use iced_core::{
    Element, Event, Layout, Length, Pixels, Point, Rectangle, Renderer as _, Shell, Size, Theme,
    alignment,
    clipboard::Clipboard,
    layout, mouse, renderer,
    text::{self, LineHeight, Renderer as _, Shaping, paragraph},
    widget::{Tree, tree},
};
use iced_wgpu::Renderer;

use crate::{
    dock::{DockId, TabEvent},
    group::DockGroupData,
};

#[derive(Debug, Default, Clone, Copy)]
enum TabAction {
    #[default]
    Idle,
    Pressing {
        index: usize,
    },
    Dragging {
        index: usize,
    },
    TitleDragging {
        origin: Point,
    },
}

#[derive(Debug, Default)]
struct TabRowState {
    action: TabAction,
    hovered: Option<usize>,
    labels: Vec<paragraph::Plain<<Renderer as iced_core::text::Renderer>::Paragraph>>,
    bounds: Vec<Rectangle>,
}

fn hit_test(bounds: &[Rectangle], cursor_rel: Point) -> Option<usize> {
    for (i, bounds) in bounds.iter().enumerate() {
        if bounds.contains(cursor_rel) {
            return Some(i);
        }
    }

    None
}

const DETACH_DEADBAND_FACTOR: f32 = 0.5;

fn drag_target_index(bounds: Rectangle, tab_bounds: &[Rectangle], cursor: Point) -> Option<usize> {
    let detach_min = Point {
        x: bounds.x - bounds.height * DETACH_DEADBAND_FACTOR,
        y: bounds.y - bounds.height * DETACH_DEADBAND_FACTOR,
    };
    let detach_max = Point {
        x: bounds.x + bounds.width + bounds.height * (1.0 + DETACH_DEADBAND_FACTOR),
        y: bounds.y + bounds.height + bounds.height * (1.0 + DETACH_DEADBAND_FACTOR),
    };
    if cursor.x < detach_min.x
        || cursor.x > detach_max.x
        || cursor.y < detach_min.y
        || cursor.y > detach_max.y
    {
        return None;
    }

    let rel_x = cursor.x - bounds.x;
    for (i, label_bounds) in tab_bounds.iter().enumerate() {
        if rel_x < label_bounds.center_x() {
            return Some(i);
        }
    }

    Some(tab_bounds.len())
}

pub struct TabRowWidget<'a, Message> {
    group_data: &'a DockGroupData,
    font_size: Pixels,
    padding: f32,
    on_action: Box<dyn Fn(TabEvent) -> Message + 'a>,
    title_drag_deadband: f32,
    title_of: Box<dyn Fn(&DockId) -> String + 'a>,
}

impl<'a, Message> TabRowWidget<'a, Message> {
    pub fn new(
        group_data: &'a DockGroupData,
        on_action: impl Fn(TabEvent) -> Message + 'a,
    ) -> Self {
        Self {
            group_data,
            font_size: Pixels(11.0),
            padding: 7.0,
            on_action: Box::new(on_action),
            title_drag_deadband: 0.0,
            title_of: Box::new(|id| id.to_string()),
        }
    }

    pub fn title_drag_deadband(mut self, factor: f32) -> Self {
        self.title_drag_deadband = factor;
        self
    }

    pub fn title_of(mut self, f: impl Fn(&DockId) -> String + 'a) -> Self {
        self.title_of = Box::new(f);
        self
    }

    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    pub fn font_size(mut self, font_size: Pixels) -> Self {
        self.font_size = font_size;
        self
    }
}

impl<Message> iced_core::Widget<Message, Theme, Renderer> for TabRowWidget<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<TabRowState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(TabRowState::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![]
    }

    fn size(&self) -> Size<Length> {
        Size::new(
            Length::Fill,
            Length::Fixed(self.font_size.0 + self.padding * 2.0),
        )
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
        let height = self.font_size.0 + self.padding * 2.0;
        let available = limits.max().width;

        let mut natural_widths = Vec::with_capacity(self.group_data.len());
        for dock in &self.group_data.docks {
            let p = paragraph::Plain::new(iced_core::text::Text {
                content: (self.title_of)(dock),
                bounds: limits.max(),
                size: self.font_size,
                line_height: LineHeight::Relative(1.0),
                font: renderer.default_font(),
                align_x: text::Alignment::Center,
                align_y: alignment::Vertical::Center,
                shaping: Shaping::Auto,
                wrapping: text::Wrapping::None,
            });
            natural_widths.push(p.min_width() + self.padding * 2.0);
            state.labels.push(p);
        }

        let total: f32 = natural_widths.iter().sum();
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
        layout::Node::new(limits.resolve(
            Length::Fill,
            Length::Fixed(self.font_size.0 + self.padding * 2.0),
            limits.max(),
        ))
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
        let dock_style = crate::style::default_style(theme);
        let ts = &dock_style.tab_bar;
        renderer.fill_quad(
            renderer::Quad {
                bounds,
                ..Default::default()
            },
            ts.background,
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
            ts.inactive_tab.border.color,
        );

        let active = self.group_data.active();
        let drag_target = if let TabAction::Dragging { index: _ } = state.action {
            cursor
                .position()
                .and_then(|p| drag_target_index(bounds, &state.bounds, p))
        } else {
            None
        };

        for (i, dock_id) in self.group_data.iter().enumerate() {
            let tab_rect_rel = state.bounds[i];
            let tab_rect = Rectangle {
                x: tab_rect_rel.x + bounds.x,
                y: tab_rect_rel.y + bounds.y,
                ..tab_rect_rel
            };

            let is_active = active == Some(dock_id);
            let is_hovered = state.hovered == Some(i);
            let tab_style = if is_active {
                &ts.active_tab
            } else if is_hovered {
                &ts.hovered_tab
            } else {
                &ts.inactive_tab
            };

            renderer.fill_quad(
                renderer::Quad {
                    bounds: tab_rect,
                    border: tab_style.border,
                    ..Default::default()
                },
                tab_style.background,
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
                ts.inactive_tab.border.color,
            );
            if is_active {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            height: 2.0,
                            ..tab_rect
                        },
                        ..Default::default()
                    },
                    tab_style.border.color,
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
                // TODO use `Ellipsis` once iced 0.15 lands.
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
            renderer.fill_text(label, position, tab_style.text_color, clip);
        }

        // Drop indicator during tab drag
        if let Some(target) = drag_target {
            let rel_x = if let Some(bounds) = state.bounds.get(target) {
                bounds.x
            } else {
                let last = state.bounds.last().unwrap();
                last.x + last.width
            };
            let x = bounds.x + rel_x;

            let indicator = Rectangle {
                x: x - 1.5,
                y: bounds.y,
                width: 3.0,
                height: self.font_size.0 + self.padding * 2.0,
            };
            renderer.fill_quad(
                renderer::Quad {
                    bounds: indicator,
                    ..Default::default()
                },
                iced_core::Background::Color(iced_core::Color::from_rgb(0.3, 0.6, 1.0)),
            );
        }
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
        let bounds = layout.bounds();
        let state = tree.state.downcast_mut::<TabRowState>();

        match event {
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                // Update hover
                state.hovered = cursor
                    .position_in(bounds)
                    .and_then(|pos| hit_test(&state.bounds, pos));

                match state.action {
                    TabAction::Idle => {}
                    TabAction::Pressing { index } => {
                        state.action = TabAction::Dragging { index };
                        shell.capture_event();
                    }
                    TabAction::Dragging { index } => {
                        shell.request_redraw();
                        shell.capture_event();

                        if drag_target_index(bounds, &state.bounds, *position).is_none() {
                            let id = self.group_data.iter().nth(index).unwrap();
                            shell.publish((self.on_action)(TabEvent::Detach(id.clone())));
                            state.action = TabAction::Idle;
                        }
                    }
                    TabAction::TitleDragging { origin } => {
                        if let Some(pos) = cursor.position()
                            && pos.distance(origin) > self.title_drag_deadband
                        {
                            shell.publish((self.on_action)(TabEvent::TitleBarDrag));
                            state.action = TabAction::Idle;
                        }

                        shell.capture_event();
                    }
                }
            }

            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if !cursor.is_over(layout.bounds()) {
                    return;
                }

                match hit_test(&state.bounds, cursor.position_in(layout.bounds()).unwrap()) {
                    Some(i) => {
                        state.action = TabAction::Pressing { index: i };
                        shell.capture_event();
                    }
                    None => {
                        state.action = TabAction::TitleDragging {
                            origin: cursor.position().unwrap(),
                        };
                        shell.capture_event();
                    }
                }
            }

            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                match state.action {
                    TabAction::Pressing { index: i } => {
                        if let Some(dock_id) = self.group_data.iter().nth(i) {
                            shell.publish((self.on_action)(TabEvent::Select(dock_id.clone())));
                        }
                        state.action = TabAction::Idle;
                        shell.capture_event();
                    }
                    TabAction::Dragging { index: from } => {
                        if let Some(pos) = cursor.position()
                            && let Some(to) = drag_target_index(bounds, &state.bounds, pos)
                        {
                            let to = if to > from { to - 1 } else { to };
                            if to != from {
                                shell.publish((self.on_action)(TabEvent::Reorder { from, to }));
                            }
                        }

                        state.action = TabAction::Idle;
                        shell.capture_event();
                    }
                    TabAction::TitleDragging { .. } => {
                        state.action = TabAction::Idle;
                        shell.capture_event();
                    }
                    TabAction::Idle => {}
                }

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
        if let TabAction::Dragging { .. } = state.action {
            return mouse::Interaction::Grabbing;
        }
        match cursor.position_in(layout.bounds()) {
            Some(pos) => match hit_test(&state.bounds, pos) {
                Some(_) => mouse::Interaction::Pointer,
                None => mouse::Interaction::Grab,
            },
            None => mouse::Interaction::default(),
        }
    }
}

impl<'a, Message: 'a> From<TabRowWidget<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(w: TabRowWidget<'a, Message>) -> Self {
        Element::new(w)
    }
}
