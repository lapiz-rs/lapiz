use std::{fmt::Display, ops::RangeInclusive, str::FromStr};

use iced_core::{
    Background, Border, Color, Element, Event, Layout, Length, Pixels, Point, Rectangle, Shell,
    Size, Text, Widget,
    alignment::{Horizontal, Vertical},
    border::{self},
    keyboard::{self, key::Key},
    layout, pointer,
    pointer::mouse,
    renderer,
    text::{Alignment, Ellipsis, LineHeight, Shaping, Wrapping},
    widget::tree::{self, Tree},
};
use iced_widget::{TextInput, text_input};
use num_traits::AsPrimitive;

pub struct SpinSlider<'a, T, Message, Theme = iced_core::Theme>
where
    Theme: Catalog,
{
    range: RangeInclusive<T>,
    value: T,
    step: T,
    precision: usize,
    scale: SliderScale,
    on_change: Option<Box<dyn Fn(T) -> Message + 'a>>,
    on_confirm: Option<Box<dyn Fn(T) -> Message + 'a>>,
    width: Length,
    height: Length,
    size: f32,
    rounded: f32,
    prefix: String,
    suffix: String,
    disabled: bool,
    allow_beyond_range: bool,
    class: <Theme as Catalog>::Class<'a>,
}

impl<'a, T, Message, Theme> SpinSlider<'a, T, Message, Theme>
where
    T: Copy + PartialOrd + Display + FromStr + AsPrimitive<f64>,
    Theme: Catalog,
    f64: AsPrimitive<T>,
{
    pub const DEFAULT_HEIGHT: f32 = 24.0;

    pub fn new(range: RangeInclusive<T>, value: T) -> Self {
        let fractional_step = 0.1f64.as_();
        let step = if fractional_step > 0.0f64.as_() {
            fractional_step
        } else {
            1.0f64.as_()
        };

        Self {
            value,
            range,
            step,
            precision: 2,
            scale: SliderScale::Linear,
            on_change: None,
            on_confirm: None,
            width: Length::Fill,
            height: Length::Fixed(Self::DEFAULT_HEIGHT),
            size: 12.0,
            rounded: 4.0,
            prefix: String::new(),
            suffix: String::new(),
            disabled: false,
            allow_beyond_range: true,
            class: <Theme as Catalog>::default(),
        }
    }

    pub fn new_01(value: T) -> Self {
        Self::new(0.0f64.as_()..=1.0f64.as_(), value)
    }

    pub fn new_percent(value: T) -> Self {
        Self::new(0.0f64.as_()..=100.0f64.as_(), value).precision(0)
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn rounded(mut self, rounded: f32) -> Self {
        self.rounded = rounded;
        self
    }

    pub fn step(mut self, step: T) -> Self {
        self.step = step;
        self
    }

    pub fn precision(mut self, precision: usize) -> Self {
        self.precision = precision;
        let step: T = 10.0_f64.powi(-(precision as i32)).as_();
        if step > 0.0_f64.as_() {
            self.step = step;
        }
        self
    }

    pub fn scale(mut self, scale: SliderScale) -> Self {
        self.scale = scale;
        self
    }

    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    pub fn suffix(mut self, suffix: impl Into<String>) -> Self {
        self.suffix = suffix.into();
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn allow_beyond_range(mut self, allow: bool) -> Self {
        if self.value < *self.range.start() {
            self.value = *self.range.start();
        } else if self.value > *self.range.end() {
            self.value = *self.range.end();
        }
        self.allow_beyond_range = allow;
        self
    }

    pub fn on_change(mut self, on_change: impl Fn(T) -> Message + 'a) -> Self {
        self.on_change = Some(Box::new(on_change));
        self
    }

    pub fn on_confirm(mut self, on_confirm: impl Fn(T) -> Message + 'a) -> Self {
        self.on_confirm = Some(Box::new(on_confirm));
        self
    }

    #[must_use]
    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self
    where
        <Theme as Catalog>::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.class = (Box::new(style) as StyleFn<'a, Theme>).into();
        self
    }

    #[must_use]
    pub fn class(mut self, class: impl Into<<Theme as Catalog>::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }

    fn text_input<'b, Renderer>(
        &'b self,
        value: &'b str,
    ) -> Element<'b, SpinSliderInputMessage, Theme, Renderer>
    where
        Renderer: iced_core::text::Renderer + 'static,
        for<'c> <Theme as text_input::Catalog>::Class<'c>: From<text_input::StyleFn<'c, Theme>>,
    {
        TextInput::new("", value)
            .on_input(SpinSliderInputMessage::Changed)
            .on_submit(SpinSliderInputMessage::Submitted)
            .width(Length::Fill)
            .padding([0, 4])
            .size(self.size)
            .line_height(LineHeight::Relative(1.0))
            .align_x(Horizontal::Center)
            .style(|theme, status| {
                let spin_style = <Theme as Catalog>::style(theme, &self.class, Status::Editing);
                let input_class = <Theme as text_input::Catalog>::default();
                let input_style =
                    <Theme as text_input::Catalog>::style(theme, &input_class, status);
                let text_color = spin_style.text_color.unwrap_or_else(|| {
                    <Theme as text_input::Catalog>::style(
                        theme,
                        &input_class,
                        text_input::Status::Active,
                    )
                    .value
                });
                text_input::Style {
                    background: Color::TRANSPARENT.into(),
                    border: Border::default(),
                    placeholder: text_color,
                    value: text_color,
                    ..input_style
                }
            })
            .into()
    }

    fn step_value(&self, value: T, direction: f64) -> T {
        self.snap_f64(value.as_() + self.step.as_() * direction)
    }

    fn snap(&self, value: T) -> T {
        self.snap_f64(value.as_())
    }

    fn snap_f64(&self, value: f64) -> T {
        let start: f64 = (*self.range.start()).as_();
        let end: f64 = (*self.range.end()).as_();
        let step: f64 = self.step.as_();
        let steps = ((value - start) / step).round();
        let val = start + steps * step;
        if self.allow_beyond_range {
            val
        } else {
            val.clamp(start, end)
        }
        .as_()
    }

    fn value_to_percentage(&self, value: T) -> f32 {
        let value: f64 = value.as_();
        let start: f64 = (*self.range.start()).as_();
        let end: f64 = (*self.range.end()).as_();
        let percentage = match self.scale {
            SliderScale::Linear => (value - start) / (end - start),
            SliderScale::Logarithmic => (value / start).ln() / (end / start).ln(),
        };
        <f64 as AsPrimitive<f32>>::as_(percentage.clamp(0.0, 1.0))
    }

    fn percentage_to_value(&self, percentage: f32) -> T {
        let percentage = percentage.clamp(0.0, 1.0) as f64;
        let start: f64 = (*self.range.start()).as_();
        let end: f64 = (*self.range.end()).as_();
        let value = match self.scale {
            SliderScale::Linear => start + (end - start) * percentage,
            SliderScale::Logarithmic => (end / start).powf(percentage) * start,
        };

        let step: f64 = self.step.as_();
        let steps = ((value - start) / step).round();
        (start + steps * step).clamp(start, end).as_()
    }
}

impl<T, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for SpinSlider<'_, T, Message, Theme>
where
    T: Copy + PartialOrd + Display + FromStr + AsPrimitive<f64> + 'static,
    Theme: Catalog,
    f64: AsPrimitive<T>,
    Renderer: iced_core::Renderer + iced_core::text::Renderer + 'static,
    for<'a> <Theme as text_input::Catalog>::Class<'a>: From<text_input::StyleFn<'a, Theme>>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<SpinSliderTreeState<T>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(SpinSliderTreeState::<T>::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        let state = tree.state.downcast_ref::<SpinSliderTreeState<T>>();
        let value = match &state.interaction {
            SpinSliderState::Editing { value } => value.clone(),
            _ => format!("{:.*}", self.precision, self.value),
        };
        let mut text_input = self.text_input::<Renderer>(&value);
        tree.diff_children(&mut [&mut text_input]);
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
        let size = limits.resolve(self.width, self.height, Size::new(0.0, self.size));
        let button_width = size.height.min(size.width / 3.0);
        let input_size = Size::new((size.width - button_width * 2.0).max(0.0), size.height);
        let state = tree.state.downcast_ref::<SpinSliderTreeState<T>>();
        let value = match &state.interaction {
            SpinSliderState::Editing { value } => value.clone(),
            _ => format!("{:.*}", self.precision, self.value),
        };
        let mut input = self.text_input::<Renderer>(&value);
        let input = input
            .as_widget_mut()
            .layout(
                &mut tree.children[0],
                renderer,
                &layout::Limits::new(input_size, input_size),
            )
            .move_to(Point::new(button_width, 0.0));

        layout::Node::with_children(size, vec![input])
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
        let state = tree.state.downcast_mut::<SpinSliderTreeState<T>>();
        let bounds = layout.bounds();
        let (minus, field, plus) = split_bounds(bounds);

        if let Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
            state.modifiers = *modifiers;
        }

        if self.disabled {
            if matches!(state.interaction, SpinSliderState::Editing { .. }) {
                let mut input = self.text_input::<Renderer>("");
                crate::text_input::unfocus(|operation| {
                    input.as_widget_mut().operate(
                        &mut tree.children[0],
                        layout.child(0),
                        renderer,
                        operation,
                    );
                });
            }
            state.interaction = SpinSliderState::Idle;
            return;
        }

        if let SpinSliderState::Editing { value } = &state.interaction {
            let input_value = value.clone();
            let mut input = self.text_input::<Renderer>(&input_value);
            let mut input_messages = iced_core::shell::Bus::new();
            let mut input_shell = shell.local(&mut input_messages);
            input.as_widget_mut().update(
                &mut tree.children[0],
                event,
                layout.child(0),
                cursor,
                renderer,
                &mut input_shell,
                viewport,
            );

            let event_captured = input_shell.is_event_captured();
            if input_shell.is_event_captured() {
                shell.capture_event();
            }
            shell.request_redraw_at(input_shell.redraw_request());
            if let Some(diff) = input_shell.is_layout_invalid() {
                shell.invalidate_layout_with(diff);
            }
            if input_shell.are_widgets_invalid() {
                shell.invalidate_widgets();
            }
            shell.input_method_mut().merge(input_shell.input_method());
            drop(input_shell);

            for message in input_messages.drain().map(|(message, _)| message) {
                match message {
                    SpinSliderInputMessage::Changed(value) => {
                        state.interaction = SpinSliderState::Editing { value };
                    }
                    SpinSliderInputMessage::Submitted => {
                        let SpinSliderState::Editing { value } = &state.interaction else {
                            continue;
                        };
                        let Ok(value) = value.parse::<T>() else {
                            continue;
                        };
                        let value = self.snap(value);
                        if value != self.value {
                            if let Some(on_change) = &self.on_change {
                                shell.publish((*on_change)(value));
                            }
                            if let Some(on_confirm) = &self.on_confirm {
                                shell.publish((*on_confirm)(value));
                            }
                        }
                        state.interaction = SpinSliderState::Idle;
                    }
                }
            }

            if matches!(
                event,
                Event::Keyboard(keyboard::Event::KeyPressed {
                    key: Key::Named(keyboard::key::Named::Escape),
                    ..
                })
            ) {
                state.interaction = SpinSliderState::Idle;
            }

            if !matches!(state.interaction, SpinSliderState::Editing { .. }) {
                let mut input = self.text_input::<Renderer>("");
                crate::text_input::unfocus(|operation| {
                    input.as_widget_mut().operate(
                        &mut tree.children[0],
                        layout.child(0),
                        renderer,
                        operation,
                    );
                });
            }

            shell.request_redraw();
            if event_captured {
                return;
            }
        }

        if let (SpinSliderState::Editing { value }, Event::Pointer(event)) =
            (&state.interaction, event)
            && event.is_primary_press()
        {
            let position = cursor.position();
            if position.is_some_and(|position| field.contains(position)) {
                shell.capture_event();
                return;
            }

            let edited_value = value.parse::<T>().ok().map(|value| self.snap(value));
            state.interaction = SpinSliderState::Idle;
            {
                let mut input = self.text_input::<Renderer>("");
                crate::text_input::unfocus(|operation| {
                    input.as_widget_mut().operate(
                        &mut tree.children[0],
                        layout.child(0),
                        renderer,
                        operation,
                    );
                });
            }

            if position.is_some_and(|position| minus.contains(position)) {
                {
                    let this = &self;
                    let value = self.step_value(edited_value.unwrap_or(self.value), -1.0);
                    if value != this.value {
                        if let Some(on_change) = &this.on_change {
                            shell.publish((*on_change)(value));
                        }
                        if let Some(on_confirm) = &this.on_confirm {
                            shell.publish((*on_confirm)(value));
                        }
                    }
                };
                state.interaction = SpinSliderState::Pressing {
                    target: PressTarget::Minus,
                };
                shell.capture_event();
            } else if position.is_some_and(|position| plus.contains(position)) {
                {
                    let this = &self;
                    let value = self.step_value(edited_value.unwrap_or(self.value), 1.0);
                    if value != this.value {
                        if let Some(on_change) = &this.on_change {
                            shell.publish((*on_change)(value));
                        }
                        if let Some(on_confirm) = &this.on_confirm {
                            shell.publish((*on_confirm)(value));
                        }
                    }
                };
                state.interaction = SpinSliderState::Pressing {
                    target: PressTarget::Plus,
                };
                shell.capture_event();
            } else if let Some(value) = edited_value
                && value != self.value
            {
                if let Some(on_change) = &self.on_change {
                    shell.publish((*on_change)(value));
                }
                if let Some(on_confirm) = &self.on_confirm {
                    shell.publish((*on_confirm)(value));
                }
            }
            return;
        }

        match event {
            Event::Pointer(event) if event.is_primary_press() => {
                let Some(position) = cursor.position() else {
                    return;
                };

                state.interaction = if minus.contains(position) {
                    let value = self.step_value(self.value, -1.0);
                    if value != self.value {
                        if let Some(on_change) = &self.on_change {
                            shell.publish((*on_change)(value));
                        }
                        if let Some(on_confirm) = &self.on_confirm {
                            shell.publish((*on_confirm)(value));
                        }
                    }
                    SpinSliderState::Pressing {
                        target: PressTarget::Minus,
                    }
                } else if plus.contains(position) {
                    let value = self.step_value(self.value, 1.0);
                    if value != self.value {
                        if let Some(on_change) = &self.on_change {
                            shell.publish((*on_change)(value));
                        }
                        if let Some(on_confirm) = &self.on_confirm {
                            shell.publish((*on_confirm)(value));
                        }
                    }
                    SpinSliderState::Pressing {
                        target: PressTarget::Plus,
                    }
                } else if field.contains(position) {
                    SpinSliderState::Pressing {
                        target: PressTarget::Field,
                    }
                } else {
                    return;
                };

                shell.capture_event();
            }
            Event::Pointer(pointer::Event::PointerMoved { position, .. })
                if matches!(
                    state.interaction,
                    SpinSliderState::Pressing {
                        target: PressTarget::Field
                    } | SpinSliderState::Dragging { .. }
                ) =>
            {
                let value = self.percentage_to_value((position.x - field.x) / field.width);
                state.interaction = SpinSliderState::Dragging { value };
                if value != self.value
                    && let Some(on_change) = &self.on_change
                {
                    shell.publish((*on_change)(value));
                }
                shell.capture_event();
            }
            Event::Pointer(e @ pointer::Event::PointerReleased { .. })
                if e.is_primary_release() =>
            {
                match std::mem::take(&mut state.interaction) {
                    SpinSliderState::Dragging { value } => {
                        if let Some(on_confirm) = &self.on_confirm {
                            shell.publish((*on_confirm)(value));
                        }
                    }
                    SpinSliderState::Pressing {
                        target: PressTarget::Field,
                    } => {
                        state.interaction = SpinSliderState::Editing {
                            value: format!("{:.*}", self.precision, self.value),
                        };
                        let edit_value = format!("{:.*}", self.precision, self.value);
                        let mut input = self.text_input::<Renderer>(&edit_value);
                        crate::text_input::focus_and_select_all(|operation| {
                            input.as_widget_mut().operate(
                                &mut tree.children[0],
                                layout.child(0),
                                renderer,
                                operation,
                            );
                        });
                        shell.invalidate_layout();
                        shell.request_redraw();
                    }
                    SpinSliderState::Pressing { .. } => {}
                    interaction => {
                        state.interaction = interaction;
                        return;
                    }
                }
                shell.request_redraw();
                shell.capture_event();
            }
            Event::Pointer(pointer::Event::WheelScrolled { delta })
                if matches!(state.interaction, SpinSliderState::Idle)
                    && state.modifiers.control()
                    && cursor.is_over(bounds) =>
            {
                let delta = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y,
                };
                let value = self.step_value(self.value, if delta < 0.0 { -1.0 } else { 1.0 });
                if value != self.value {
                    if let Some(on_change) = &self.on_change {
                        shell.publish((*on_change)(value));
                    }
                    if let Some(on_confirm) = &self.on_confirm {
                        shell.publish((*on_confirm)(value));
                    }
                }
                shell.capture_event();
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, .. })
                if matches!(state.interaction, SpinSliderState::Idle) && cursor.is_over(bounds) =>
            {
                match key {
                    Key::Named(keyboard::key::Named::ArrowUp) => {
                        let value = self.step_value(self.value, 1.0);
                        if value != self.value {
                            if let Some(on_change) = &self.on_change {
                                shell.publish((*on_change)(value));
                            }
                            if let Some(on_confirm) = &self.on_confirm {
                                shell.publish((*on_confirm)(value));
                            }
                        }
                        shell.capture_event();
                    }
                    Key::Named(keyboard::key::Named::ArrowDown) => {
                        let value = self.step_value(self.value, -1.0);
                        if value != self.value {
                            if let Some(on_change) = &self.on_change {
                                shell.publish((*on_change)(value));
                            }
                            if let Some(on_confirm) = &self.on_confirm {
                                shell.publish((*on_confirm)(value));
                            }
                        }
                        shell.capture_event();
                    }
                    _ => {}
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
        renderer_style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<SpinSliderTreeState<T>>();
        let bounds = layout.bounds();
        let (minus, field, plus) = split_bounds(bounds);
        let status = if self.disabled {
            Status::Disabled
        } else if matches!(state.interaction, SpinSliderState::Editing { .. }) {
            Status::Editing
        } else if matches!(state.interaction, SpinSliderState::Dragging { .. }) {
            Status::Dragged
        } else if cursor.is_over(bounds) {
            Status::Hovered
        } else {
            Status::Active
        };
        let style = <Theme as Catalog>::style(theme, &self.class, status);

        renderer.fill_quad(
            renderer::Quad {
                bounds: minus,
                border: Border {
                    color: style.border_color,
                    width: 1.0,
                    radius: border::Radius {
                        top_left: self.rounded,
                        bottom_left: self.rounded,
                        ..Default::default()
                    },
                },
                ..Default::default()
            },
            style.button_background,
        );
        renderer.fill_quad(
            renderer::Quad {
                bounds: field,
                border: Border {
                    color: style.border_color,
                    width: 1.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            style.background,
        );
        renderer.fill_quad(
            renderer::Quad {
                bounds: plus,
                border: Border {
                    color: style.border_color,
                    width: 1.0,
                    radius: border::Radius {
                        top_right: self.rounded,
                        bottom_right: self.rounded,
                        ..Default::default()
                    },
                },
                ..Default::default()
            },
            style.button_background,
        );

        let value = match state.interaction {
            SpinSliderState::Idle
            | SpinSliderState::Pressing { .. }
            | SpinSliderState::Editing { .. } => self.value,
            SpinSliderState::Dragging { value } => value,
        };

        let percentage = self.value_to_percentage(value);
        let value_bar = if matches!(state.interaction, SpinSliderState::Editing { .. }) {
            style.value_bar.scale_alpha(0.2)
        } else {
            style.value_bar
        };
        renderer.fill_quad(
            renderer::Quad {
                bounds: Rectangle {
                    width: field.width * percentage,
                    ..field
                },
                ..Default::default()
            },
            value_bar,
        );

        let text_color = style.text_color.unwrap_or(renderer_style.text_color);
        fill_text(renderer, minus, "−", self.size, text_color);
        if let SpinSliderState::Editing { value } = &state.interaction {
            self.text_input::<Renderer>(value).as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                renderer_style,
                layout.child(0),
                cursor,
                viewport,
            );
        } else {
            fill_text(
                renderer,
                field,
                &format!("{}{:.*}{}", self.prefix, self.precision, value, self.suffix),
                self.size,
                text_color,
            );
        }
        fill_text(renderer, plus, "+", self.size, text_color);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if self.disabled || !cursor.is_over(layout.bounds()) {
            return mouse::Interaction::default();
        }
        let state = tree.state.downcast_ref::<SpinSliderTreeState<T>>();
        if let SpinSliderState::Editing { value } = &state.interaction
            && cursor.is_over(layout.child(0).bounds())
        {
            self.text_input::<Renderer>(value)
                .as_widget()
                .mouse_interaction(
                    &tree.children[0],
                    layout.child(0),
                    cursor,
                    viewport,
                    renderer,
                )
        } else if matches!(state.interaction, SpinSliderState::Dragging { .. }) {
            mouse::Interaction::Grabbing
        } else {
            mouse::Interaction::Pointer
        }
    }
}

impl<'a, T, Message, Theme, Renderer> From<SpinSlider<'a, T, Message, Theme>>
    for Element<'a, Message, Theme, Renderer>
where
    T: Copy + PartialOrd + Display + FromStr + AsPrimitive<f64> + 'static,
    f64: AsPrimitive<T>,
    Message: 'a,
    Theme: Catalog + 'a,
    Renderer: iced_core::Renderer + iced_core::text::Renderer + 'static,
    for<'b> <Theme as text_input::Catalog>::Class<'b>: From<text_input::StyleFn<'b, Theme>>,
{
    fn from(widget: SpinSlider<'a, T, Message, Theme>) -> Self {
        Element::new(widget)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderScale {
    Linear,
    Logarithmic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PressTarget {
    Minus,
    Field,
    Plus,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub background: Background,
    pub value_bar: Background,
    pub button_background: Background,
    pub border_color: Color,
    pub text_color: Option<Color>,
}

#[derive(Clone)]
enum SpinSliderInputMessage {
    Changed(String),
    Submitted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Active,
    Hovered,
    Dragged,
    Editing,
    Disabled,
}

#[derive(Debug)]
struct SpinSliderTreeState<T> {
    interaction: SpinSliderState<T>,
    modifiers: keyboard::Modifiers,
}

impl<T> Default for SpinSliderTreeState<T> {
    fn default() -> Self {
        Self {
            interaction: SpinSliderState::Idle,
            modifiers: keyboard::Modifiers::default(),
        }
    }
}

#[derive(Debug, Default)]
enum SpinSliderState<T> {
    #[default]
    Idle,
    Pressing {
        target: PressTarget,
    },
    Dragging {
        value: T,
    },
    Editing {
        value: String,
    },
}

fn split_bounds(bounds: Rectangle) -> (Rectangle, Rectangle, Rectangle) {
    let button_width = bounds.height.min(bounds.width / 3.0);
    let minus = Rectangle {
        width: button_width,
        ..bounds
    };
    let field = Rectangle {
        x: bounds.x + button_width,
        width: bounds.width - button_width * 2.0,
        ..bounds
    };
    let plus = Rectangle {
        x: bounds.x + bounds.width - button_width,
        width: button_width,
        ..bounds
    };
    (minus, field, plus)
}

fn fill_text<Renderer>(
    renderer: &mut Renderer,
    bounds: Rectangle,
    content: &str,
    size: f32,
    color: Color,
) where
    Renderer: iced_core::Renderer + iced_core::text::Renderer,
{
    renderer.fill_text(
        Text {
            content: content.to_owned(),
            bounds: bounds.size(),
            size: Pixels(size),
            line_height: LineHeight::Relative(1.0),
            font: renderer.default_font(),
            align_x: Alignment::Center,
            align_y: Vertical::Center,
            shaping: Shaping::Auto,
            wrapping: Wrapping::None,
            ellipsis: Ellipsis::None,
            hint_factor: None,
        },
        Point::new(bounds.center_x(), bounds.center_y()),
        color,
        bounds,
    );
}

pub trait Catalog: text_input::Catalog + Sized {
    type Class<'a>;

    fn default<'a>() -> <Self as Catalog>::Class<'a>;

    fn style(&self, class: &<Self as Catalog>::Class<'_>, status: Status) -> Style;
}

pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

impl Catalog for iced_core::Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> <Self as Catalog>::Class<'a> {
        Box::new(default)
    }

    fn style(&self, class: &<Self as Catalog>::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

pub fn default(theme: &iced_core::Theme, status: Status) -> Style {
    let palette = theme.palette();
    let value_bar = palette.primary.base.color.scale_alpha(match status {
        Status::Hovered | Status::Editing => 0.22,
        Status::Dragged => 0.28,
        Status::Disabled => 0.08,
        Status::Active => 0.16,
    });
    Style {
        background: palette.background.base.color.into(),
        value_bar: value_bar.into(),
        button_background: theme.palette().background.weakest.color.into(),
        border_color: palette.background.strong.color,
        text_color: None,
    }
}
