use std::{iter, rc::Rc};

use iced_core::{
    Alignment, Element, Event, Layout, Length, Pixels, Rectangle, Shell, Size, Theme, Widget,
    keyboard::{self, Modifiers, key},
    layout,
    pointer::mouse,
    renderer, text,
    widget::{Tree, tree},
    window,
};
use iced_widget::{Action, Component, Container, Row, Text, component::component, container};
use lapiz_input::key::KeySequence;
use lapiz_runtime::Renderer;

use crate::{
    button::{self, Button, Status},
    icon, tag,
};

type StyleFn<'a> = dyn Fn(&Theme) -> container::Style + 'a;

pub struct Kbd<'a, Message> {
    keys: KeySequence,
    width: Length,
    height: Length,
    size: Pixels,
    listenable: bool,
    placeholder: String,
    confirm_on_release: bool,
    check_conflict: Option<Box<dyn Fn(KeySequence) -> bool + 'a>>,
    on_confirm: Option<Box<dyn Fn(KeySequence) -> Message + 'a>>,
    class: Rc<StyleFn<'a>>,
}

impl<'a, Message> Kbd<'a, Message> {
    pub fn new(keys: KeySequence) -> Self {
        Self {
            keys,
            width: Length::Shrink,
            height: Length::Fixed(20.0),
            size: Pixels(12.0),
            listenable: false,
            placeholder: String::new(),
            confirm_on_release: false,
            check_conflict: None,
            on_confirm: None,
            class: Rc::new(default),
        }
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = size.into();
        self
    }

    pub fn listenable(mut self) -> Self {
        self.listenable = true;
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn confirm_on_release(mut self) -> Self {
        self.confirm_on_release = true;
        self
    }

    pub fn check_conflict(mut self, check: impl Fn(KeySequence) -> bool + 'a) -> Self {
        self.check_conflict = Some(Box::new(check));
        self
    }

    pub fn on_confirm(mut self, on_confirm: impl Fn(KeySequence) -> Message + 'a) -> Self {
        self.on_confirm = Some(Box::new(on_confirm));
        self
    }

    pub fn style(mut self, style: impl Fn(&Theme) -> container::Style + 'a) -> Self {
        self.class = Rc::new(style);
        self
    }

    pub fn class(mut self, class: impl Into<<Theme as container::Catalog>::Class<'a>>) -> Self {
        self.class = Rc::from(class.into());
        self
    }

    pub fn accent(self) -> Self {
        self.style(accent)
    }
}

#[derive(Default)]
pub struct State {
    source: Option<KeySequence>,
    confirmed: Option<KeySequence>,
    recording: Recording,
}

#[derive(Default)]
enum Recording {
    #[default]
    Idle,
    Waiting,
    Candidate(KeySequence),
}

#[derive(Clone, Copy)]
pub enum Edit {
    Start,
    Record(KeySequence),
    Confirm,
    Cancel,
}

impl<'a, Message> Component<'a, Message, Theme, Renderer> for Kbd<'a, Message> {
    type State = State;
    type Event = Edit;

    fn diff(&mut self, state: &mut State) {
        if state.source != Some(self.keys) {
            *state = State {
                source: Some(self.keys),
                ..State::default()
            };
        }
        if !self.listenable {
            state.recording = Recording::Idle;
        }
    }

    fn update(&mut self, state: &mut State, event: Edit, _renderer: &Renderer) -> Option<Message> {
        match event {
            Edit::Start if self.listenable => state.recording = Recording::Waiting,
            Edit::Record(keys)
                if self.listenable && !matches!(state.recording, Recording::Idle) =>
            {
                state.recording = Recording::Candidate(keys);
            }
            Edit::Confirm => {
                if let Recording::Candidate(keys) = state.recording
                    && !self
                        .check_conflict
                        .as_ref()
                        .is_some_and(|check| check(keys))
                {
                    state.recording = Recording::Idle;
                    if let Some(confirm) = &self.on_confirm {
                        return Some(confirm(keys));
                    }
                    state.confirmed = Some(keys);
                }
            }
            Edit::Cancel => state.recording = Recording::Idle,
            Edit::Start | Edit::Record(_) => {}
        }
        None
    }

    fn listen(
        &self,
        state: &State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Action<Edit> {
        if matches!(state.recording, Recording::Idle) {
            return Action::none();
        }

        match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                physical_key: key::Physical::Code(code),
                modifiers,
                repeat: false,
                ..
            }) => captured_keys(*code, *modifiers).map_or_else(Action::capture, |keys| {
                Action::publish(Edit::Record(keys)).and_capture()
            }),
            Event::Keyboard(keyboard::Event::KeyReleased { .. })
                if self.confirm_on_release
                    && matches!(state.recording, Recording::Candidate(_)) =>
            {
                Action::publish(Edit::Confirm).and_capture()
            }
            Event::Keyboard(_) => Action::capture(),
            Event::Window(window::Event::Unfocused) => Action::publish(Edit::Cancel),
            Event::Pointer(event) if event.is_primary_press() && !cursor.is_over(bounds) => {
                Action::publish(Edit::Cancel)
            }
            _ => Action::none(),
        }
    }

    fn view(&self, state: &State) -> Element<'a, Edit, Theme, Renderer> {
        let keys = match state.recording {
            Recording::Idle => Some(state.confirmed.unwrap_or(self.keys)),
            Recording::Waiting => None,
            Recording::Candidate(keys) => Some(keys),
        };
        let editing = !matches!(state.recording, Recording::Idle);
        let conflict = matches!(state.recording, Recording::Candidate(keys) if
                self.check_conflict
                    .as_ref()
                    .is_some_and(|check| check(keys))
        );

        let keycap = |label| {
            let class = Rc::clone(&self.class);
            Container::new(
                Text::new(label)
                    .size(self.size)
                    .wrapping(text::Wrapping::None),
            )
            .center_x(Length::Fit.min(10))
            .center_y(self.height)
            .padding([0, 8])
            .style(move |theme| {
                let mut style = class(theme);
                if conflict {
                    style.border.color = theme.palette().danger.base.color;
                    style.border.width = 1.0;
                } else if editing {
                    style.border.color = theme.palette().primary.base.color;
                    style.border.width = 1.0;
                }
                style
            })
        };
        let mut labels = keys
            .into_iter()
            .flat_map(KeySequence::into_codes)
            .filter_map(|code| captured_keys(code, Modifiers::empty()))
            .map(|key| key.to_string());
        let keycaps = Row::new()
            .align_y(Alignment::Center)
            .spacing(6)
            .push(keycap(labels.next().unwrap_or_else(|| {
                if editing {
                    String::new()
                } else {
                    self.placeholder.clone()
                }
            })))
            .extend(labels.map(|label| keycap(label).into()));

        let badge = Badge {
            content: Container::new(keycaps).center_x(self.width),
            clickable: self.listenable && !editing,
        };

        let mut row = Row::new()
            .align_y(Alignment::Center)
            .spacing(2)
            .push(Element::new(badge));
        if self.listenable {
            if !self.width.is_fill() {
                row = row.width(Length::Shrink);
            } else if !self.height.is_fill() {
                row = row.height(Length::Shrink);
            }
        }

        if editing && !self.confirm_on_release {
            let confirm = (keys.is_some() && !conflict).then_some(Edit::Confirm);
            let check = if confirm.is_some() {
                icon::check()
            } else {
                icon::check().muted()
            };
            row = row
                .push(
                    Button::icon(check)
                        .style(action_style)
                        .on_press_maybe(confirm),
                )
                .push(
                    Button::icon(icon::close())
                        .style(button::default)
                        .on_press(Edit::Cancel),
                );
        }
        row.into()
    }
}

fn captured_keys(code: key::Code, modifiers: Modifiers) -> Option<KeySequence> {
    let mut keys = KeySequence::from_codes(iter::once(code)).ok()?;
    if keys.key.is_some() && keys.to_string().is_empty() {
        return None;
    }
    keys.modifiers |= modifiers;
    Some(keys)
}

struct Badge<'a> {
    content: Container<'a, Edit, Theme, Renderer>,
    clickable: bool,
}

impl Badge<'_> {
    fn is_over(&self, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) -> bool {
        self.clickable && cursor.is_over(layout.bounds()) && cursor.is_over(*viewport)
    }
}

impl Widget<Edit, Theme, Renderer> for Badge<'_> {
    fn tag(&self) -> tree::Tag {
        self.content.tag()
    }

    fn state(&self) -> tree::State {
        self.content.state()
    }

    fn size(&self) -> Size<Length> {
        self.content.size()
    }

    fn diff(&mut self, tree: &mut Tree) {
        self.content.diff(tree);
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.layout(tree, renderer, limits)
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        shell: &mut Shell<'_, Edit>,
        viewport: &Rectangle,
    ) {
        if matches!(event, Event::Pointer(event) if event.is_primary_press())
            && !shell.is_event_captured()
            && self.is_over(layout, cursor, viewport)
        {
            shell.publish(Edit::Start);
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if self.is_over(layout, cursor, viewport) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
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
        self.content
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }
}

impl<'a, Message: 'a> From<Kbd<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(kbd: Kbd<'a, Message>) -> Self {
        component(kbd)
    }
}

fn action_style(theme: &Theme, status: Status) -> button::Style {
    match status {
        Status::Disabled => button::Style {
            background: Some(theme.palette().background.base.color.into()),
            ..button::default(theme, Status::Active)
        },
        _ => button::default(theme, status),
    }
}

pub fn default(theme: &Theme) -> container::Style {
    tag::neutral(theme)
}

pub fn accent(theme: &Theme) -> container::Style {
    tag::primary(theme)
}
