use std::{fmt::Display, ops::RangeBounds, str::FromStr};

use iced_core::{
    Border, Element, Event, Layout, Length, Point, Radians, Rectangle, Renderer as _, Shell, Size,
    Theme, Widget, keyboard, layout, pointer,
    pointer::mouse,
    renderer, svg,
    svg::Renderer as _,
    widget::{Operation, Tree, tree},
};
use iced_widget::text_input;
use lapiz_runtime::Renderer;

use crate::{
    callback::{CallbackWith, publish_with},
    text_input as text_input_ops,
};

const STEPPER_WIDTH: f32 = 16.0;
const DEFAULT_WIDTH: f32 = 80.0;
const ICON_SIZE: f32 = 7.0;

#[derive(Clone)]
enum InternalMessage {
    Changed(String),
}

pub struct SpinBox<'a, T, Message> {
    value: T,
    min: Option<T>,
    max: Option<T>,
    step: T,
    text: String,
    content: text_input::TextInput<'a, InternalMessage, Theme, Renderer>,
    on_change: CallbackWith<'a, T, Message>,
    width: Length,
}

impl<'a, T, Message> SpinBox<'a, T, Message>
where
    T: Copy + num_traits::Num + PartialOrd + Display + FromStr,
{
    pub fn new(
        value: &T,
        bounds: impl RangeBounds<T>,
        on_change: impl Fn(T) -> Message + 'a,
    ) -> Self {
        let bound = |bound: std::ops::Bound<&T>| match bound {
            std::ops::Bound::Included(value) | std::ops::Bound::Excluded(value) => Some(*value),
            std::ops::Bound::Unbounded => None,
        };
        let text = value.to_string();
        Self {
            value: *value,
            min: bound(bounds.start_bound()),
            max: bound(bounds.end_bound()),
            step: num_traits::One::one(),
            content: Self::text_input(text.clone()),
            on_change: Some(Box::new(on_change)),
            text,
            width: Length::Fixed(DEFAULT_WIDTH),
        }
    }

    fn text_input(text: String) -> text_input::TextInput<'a, InternalMessage, Theme, Renderer> {
        text_input::TextInput::new("", text)
            .on_input(InternalMessage::Changed)
            .size(12.0)
            .padding([3.0, 6.0])
            .style(crate::text_input::default)
    }

    pub fn step(mut self, step: T) -> Self {
        self.step = step;
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    fn clamp(&self, value: T) -> T {
        let mut value = value;
        if let Some(min) = self.min
            && value < min
        {
            value = min;
        }
        if let Some(max) = self.max
            && value > max
        {
            value = max;
        }
        value
    }

    fn in_bounds(&self, value: &T) -> bool {
        self.min.is_none_or(|min| *value >= min) && self.max.is_none_or(|max| *value <= max)
    }

    fn accepts(&self, text: &str) -> bool {
        if text.is_empty() || text == "-" && self.min.is_none_or(|min| min < T::zero()) {
            return true;
        }
        match T::from_str(text) {
            Ok(value) => self.in_bounds(&value),
            Err(_) => false,
        }
    }

    fn step_value(&mut self, up: bool, shell: &mut Shell<'_, Message>) {
        let target = if up {
            self.value + self.step
        } else {
            self.value - self.step
        };
        let target = self.clamp(target);
        if target != self.value {
            self.value = target;
            if let Some(message) = publish_with(&mut self.on_change, target) {
                shell.publish(message);
            }
        }
    }

    fn forward_content(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, InternalMessage>,
        viewport: &Rectangle,
    ) {
        let content_layout = layout.children().next().expect("content layout");
        self.content.update(
            &mut tree.children[0],
            event,
            content_layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
    }
}

fn stepper_at(bounds: Rectangle, position: Point) -> Option<bool> {
    if !bounds.contains(position) || position.x < bounds.x + bounds.width - STEPPER_WIDTH {
        return None;
    }
    Some(position.y < bounds.y + bounds.height / 2.0)
}

#[derive(Default)]
struct SpinBoxTreeState {
    input_focused: bool,
}

impl<'a, T, Message> Widget<Message, Theme, Renderer> for SpinBox<'a, T, Message>
where
    T: Copy + num_traits::Num + PartialOrd + Display + FromStr + 'a,
    Message: 'a,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<SpinBoxTreeState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(SpinBoxTreeState::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children_custom(
            std::slice::from_mut(&mut self.content),
            |tree, content| content.diff(tree),
            |content| Tree {
                tag: content.tag(),
                state: content.state(),
                children: Vec::new(),
            },
        );
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(self.width);
        let content = self
            .content
            .layout(&mut tree.children[0], renderer, &limits);
        layout::Node::with_children(content.size(), vec![content])
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
        let bounds = layout.bounds();
        let stepper = cursor
            .position()
            .and_then(|position| stepper_at(bounds, position));

        let mut messages = iced_core::shell::Bus::new();
        let mut sub_shell = shell.local(&mut messages);

        match event {
            Event::Pointer(event) if event.is_primary_press() && stepper.is_some() => {
                self.step_value(stepper == Some(true), shell);
                shell.capture_event();
                shell.request_redraw();
            }
            Event::Pointer(pointer::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => {
                match delta {
                    mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => {
                        self.step_value(y.is_sign_positive(), shell);
                    }
                }
                shell.capture_event();
                shell.request_redraw();
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, text, .. }) => {
                let is_focused = {
                    let probe = &mut self.content;
                    text_input_ops::is_focused(|operation| {
                        probe.operate(
                            &mut tree.children[0],
                            layout.children().next().expect("content layout"),
                            renderer,
                            operation,
                        )
                    })
                };
                if !is_focused {
                    return;
                }

                match key {
                    keyboard::Key::Named(keyboard::key::Named::ArrowUp) if text.is_none() => {
                        self.step_value(true, shell);
                        shell.capture_event();
                        shell.request_redraw();
                        return;
                    }
                    keyboard::Key::Named(keyboard::key::Named::ArrowDown) if text.is_none() => {
                        self.step_value(false, shell);
                        shell.capture_event();
                        shell.request_redraw();
                        return;
                    }
                    _ => {}
                }
                self.forward_content(
                    tree,
                    event,
                    layout,
                    cursor,
                    renderer,
                    &mut sub_shell,
                    viewport,
                );
            }
            _ => self.forward_content(
                tree,
                event,
                layout,
                cursor,
                renderer,
                &mut sub_shell,
                viewport,
            ),
        }

        {
            let probe = &mut self.content;
            let is_focused = text_input_ops::is_focused(|operation| {
                probe.operate(
                    &mut tree.children[0],
                    layout.children().next().expect("content layout"),
                    renderer,
                    operation,
                )
            });
            tree.state.downcast_mut::<SpinBoxTreeState>().input_focused = is_focused;
        }

        shell.request_redraw_at(sub_shell.redraw_request());
        if sub_shell.is_event_captured() {
            shell.capture_event();
        }
        if let Some(diff) = sub_shell.is_layout_invalid() {
            shell.invalidate_layout_with(diff);
        }
        if sub_shell.are_widgets_invalid() {
            shell.invalidate_widgets();
        }
        for message in messages.drain().map(|(message, _)| message) {
            match message {
                InternalMessage::Changed(text) => {
                    if self.accepts(&text) {
                        self.text = text;
                    }
                    self.content = Self::text_input(self.text.clone());
                    shell.invalidate_layout();

                    if let Ok(value) = T::from_str(&self.text)
                        && self.in_bounds(&value)
                        && value != self.value
                    {
                        self.value = value;
                        if let Some(message) = publish_with(&mut self.on_change, value) {
                            shell.publish(message);
                        }
                    }
                }
            }
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let content_layout = layout.children().next().expect("content layout");
        self.content
            .operate(&mut tree.children[0], content_layout, renderer, operation);
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let content_layout = layout.children().next().expect("content layout");
        self.content.draw(
            &tree.children[0],
            renderer,
            theme,
            &renderer::Style {
                text_color: Default::default(),
            },
            content_layout,
            cursor,
            viewport,
        );

        let p = theme.palette();
        let focused = tree.state.downcast_ref::<SpinBoxTreeState>().input_focused;
        let border_color = if focused {
            p.primary.base.color
        } else {
            p.background.strong.color
        };
        let stepper_bounds = Rectangle::new(
            Point::new(bounds.x + bounds.width - STEPPER_WIDTH, bounds.y),
            Size::new(STEPPER_WIDTH, bounds.height),
        );
        renderer.fill_quad(
            renderer::Quad {
                bounds: stepper_bounds,
                border: Border {
                    radius: 0.0.into(),
                    width: 1.0,
                    color: border_color,
                },
                ..renderer::Quad::default()
            },
            p.background.base.color,
        );

        let stepper = cursor
            .position()
            .and_then(|position| stepper_at(bounds, position));
        for (cell, bytes, hovered) in [
            (
                Rectangle::new(
                    stepper_bounds.position(),
                    Size::new(STEPPER_WIDTH, stepper_bounds.height / 2.0),
                ),
                include_bytes!("../assets/icons/chevron_up.svg") as &'static [u8],
                stepper == Some(true),
            ),
            (
                Rectangle::new(
                    Point::new(
                        stepper_bounds.x,
                        stepper_bounds.y + stepper_bounds.height / 2.0,
                    ),
                    Size::new(STEPPER_WIDTH, stepper_bounds.height / 2.0),
                ),
                include_bytes!("../assets/icons/chevron_down.svg") as &'static [u8],
                stepper == Some(false),
            ),
        ] {
            if hovered {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: cell,
                        ..renderer::Quad::default()
                    },
                    p.primary.weak.color,
                );
            }
            renderer.draw_svg(
                svg::Svg {
                    handle: svg::Handle::from_memory(bytes),
                    color: Some(if hovered {
                        p.primary.strong.color
                    } else {
                        p.background.weak.text
                    }),
                    rotation: Radians(0.0),
                    opacity: 1.0,
                },
                Rectangle::new(
                    Point::new(
                        cell.x + (cell.width - ICON_SIZE) / 2.0,
                        cell.y + (cell.height - ICON_SIZE) / 2.0,
                    ),
                    Size::new(ICON_SIZE, ICON_SIZE),
                ),
                cell,
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
        if cursor
            .position()
            .is_some_and(|position| stepper_at(layout.bounds(), position).is_some())
        {
            mouse::Interaction::Pointer
        } else if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::None
        }
    }
}

impl<'a, T, Message> From<SpinBox<'a, T, Message>> for Element<'a, Message, Theme, Renderer>
where
    T: Copy + num_traits::Num + PartialOrd + Display + FromStr + 'a,
    Message: 'a,
{
    fn from(value: SpinBox<'a, T, Message>) -> Self {
        Element::new(value)
    }
}
