use std::fmt::Display;

use iced_core::{
    Border, Color, Element, Event, Layout, Length, Padding, Pixels, Point, Rectangle,
    Renderer as _, Shadow, Shell, Size, Vector, Widget, alignment, keyboard, layout, overlay,
    pointer::{self, mouse},
    renderer,
    text::{self, Paragraph as _, Renderer as _, editor, input},
    widget::{Operation, Tree, operation::Focusable as _, tree},
    window,
};
use iced_widget::{Scrollable, Text, overlay::menu, pick_list, text_input};
use lapiz_runtime::{Renderer, Theme};

use crate::text_input::default;

/// A dropdown selection widget with optional type-to-filter search and custom item views.
///
/// `ComboBox` displays the currently selected option (or a placeholder when none is selected)
/// alongside a dropdown arrow indicator. Clicking the widget opens a floating [`Scrollable`]
/// menu overlay positioned below or above the control depending on available viewport space.
///
/// # Features
///
/// - **Custom rendering**: Construct with [`ComboBox::new`] for [`Display`] types or
///   [`ComboBox::new_with`] to supply a custom element view closure for each option.
/// - **Search filtering**: Enable [`searchable`](Self::searchable) to turn the open control into
///   a focused text input that filters visible options as the user types.
/// - **Keyboard & wheel navigation**: Supports `ArrowUp`/`ArrowDown` to cycle options, `Enter` to
///   confirm, `Escape` to close, and `Ctrl`/`Cmd` + mouse wheel scrolling when closed.
pub struct ComboBox<'a, T, Message> {
    options: Vec<(T, Element<'a, Message, Theme, Renderer>)>,
    selected: Option<(T, Element<'a, Message, Theme, Renderer>)>,
    on_selected: Box<dyn Fn(T) -> Message + 'a>,
    view: Box<dyn Fn(&T, Pixels) -> Element<'a, Message, Theme, Renderer> + 'a>,
    matches: Option<fn(&T, &str) -> bool>,
    placeholder: String,
    width: Length,
    menu_height: Length,
    padding: Padding,
    size: Pixels,
    input_class: <Theme as text_input::Catalog>::Class<'a>,
    menu_class: <Theme as menu::Catalog>::Class<'a>,
}

impl<'a, T, Message: 'a> ComboBox<'a, T, Message> {
    pub fn new(
        options: impl Into<Vec<T>>,
        selected: Option<T>,
        on_selected: impl Fn(T) -> Message + 'a,
    ) -> Self
    where
        T: Display,
    {
        Self::with_view(options, selected, on_selected, |option, size| {
            Text::new(option.to_string()).size(size).into()
        })
    }

    pub fn new_with(
        options: impl Into<Vec<T>>,
        selected: Option<T>,
        on_selected: impl Fn(T) -> Message + 'a,
        display: impl Fn(&T) -> Element<'a, Message, Theme, Renderer> + 'a,
    ) -> Self {
        Self::with_view(options, selected, on_selected, move |option, _size| {
            display(option)
        })
    }

    pub fn searchable(mut self, searchable: bool) -> Self
    where
        T: for<'s> PartialEq<&'s str>,
    {
        self.matches = searchable.then_some(|option, query| option == &query);
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn menu_height(mut self, height: impl Into<Length>) -> Self {
        self.menu_height = height.into();
        self
    }

    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        let size = size.into();
        if self.size != size {
            self.size = size;
            for (option, element) in self.selected.iter_mut().chain(&mut self.options) {
                *element = (self.view)(option, size);
            }
        }
        self
    }

    pub fn input_style(
        mut self,
        style: impl Fn(&Theme, text_input::Status) -> text_input::Style + 'a,
    ) -> Self {
        self.input_class = Box::new(style);
        self
    }

    pub fn menu_style(mut self, style: impl Fn(&Theme) -> menu::Style + 'a) -> Self {
        self.menu_class = Box::new(style);
        self
    }

    pub fn input_class(
        mut self,
        class: impl Into<<Theme as text_input::Catalog>::Class<'a>>,
    ) -> Self {
        self.input_class = class.into();
        self
    }

    pub fn menu_class(mut self, class: impl Into<<Theme as menu::Catalog>::Class<'a>>) -> Self {
        self.menu_class = class.into();
        self
    }

    fn with_view(
        options: impl Into<Vec<T>>,
        selected: Option<T>,
        on_selected: impl Fn(T) -> Message + 'a,
        view: impl Fn(&T, Pixels) -> Element<'a, Message, Theme, Renderer> + 'a,
    ) -> Self {
        let size = Pixels(12.0);
        let options = options
            .into()
            .into_iter()
            .map(|option| {
                let element = view(&option, size);
                (option, element)
            })
            .collect();
        let selected = selected.map(|option| {
            let element = view(&option, size);
            (option, element)
        });

        Self {
            options,
            selected,
            on_selected: Box::new(on_selected),
            view: Box::new(view),
            matches: None,
            placeholder: String::new(),
            width: Length::Shrink,
            menu_height: Length::Shrink,
            padding: Padding::from([5, 8]),
            size,
            input_class: Box::new(default),
            menu_class: Box::new(menu_style),
        }
    }

    fn visible_options(&self, query: &str) -> Vec<usize> {
        self.options
            .iter()
            .enumerate()
            .filter_map(|(index, (option, _))| {
                (query.is_empty() || self.matches.is_none_or(|matches| matches(option, query)))
                    .then_some(index)
            })
            .collect()
    }
}

impl<'a, T: Clone + PartialEq + 'a, Message: 'a> From<ComboBox<'a, T, Message>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(combo: ComboBox<'a, T, Message>) -> Self {
        Element::new(combo)
    }
}

/// Internal widget state for [`ComboBox`].
#[derive(Default)]
struct State {
    input: text::Input<Renderer>,
    is_open: bool,
    hovered_option: Option<usize>,
    modifiers: keyboard::Modifiers,
    menu: Option<Tree>,
    last_status: Option<pick_list::Status>,
}

impl<'a, T: Clone + PartialEq, Message: 'a> Widget<Message, Theme, Renderer>
    for ComboBox<'a, T, Message>
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        let mut children = self
            .selected
            .iter_mut()
            .chain(&mut self.options)
            .map(|(_, element)| element)
            .collect::<Vec<_>>();
        tree.diff_children(&mut children);
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
        let limits = limits.width(self.width).height(Length::Shrink);
        let inner_limits = limits.shrink(self.padding);
        let handle_width = self.size.0 + self.padding.left;
        let content_limits = inner_limits.shrink(Size::new(handle_width, 0.0));
        let line_height = text::LineHeight::default().to_absolute(self.size).0;
        let mut intrinsic = Size::new(0.0, line_height);
        if let Some((_, element)) = &mut self.selected {
            intrinsic = element
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, &content_limits)
                .size();
            intrinsic.height = intrinsic.height.max(line_height);
        } else {
            let paragraph = <Renderer as text::Renderer>::Paragraph::with_text(text::Text {
                content: &self.placeholder,
                bounds: content_limits.max(),
                size: self.size,
                line_height: text::LineHeight::default(),
                font: renderer.default_font(),
                align_x: text::Alignment::Default,
                align_y: alignment::Vertical::Center,
                shaping: text::Shaping::Advanced,
                wrapping: text::Wrapping::None,
                ellipsis: text::Ellipsis::None,
                hint_factor: renderer.hint_factor(),
            });
            intrinsic.width = paragraph.min_width();
        }
        if self.width == Length::Shrink {
            let offset = usize::from(self.selected.is_some());
            for ((_, element), tree) in self.options.iter_mut().zip(&mut tree.children[offset..]) {
                let width = element
                    .as_widget_mut()
                    .layout(tree, renderer, &content_limits)
                    .size()
                    .width;
                intrinsic.width = intrinsic.width.max(width);
            }
        }
        intrinsic.width += handle_width;
        let inner_size = inner_limits.resolve(self.width, Length::Shrink, intrinsic);
        let size = inner_size.expand(self.padding);
        let mut children = Vec::new();
        if let Some((_, element)) = &mut self.selected {
            let limits = layout::Limits::new(
                Size::ZERO,
                Size::new(
                    (inner_size.width - handle_width).max(0.0),
                    inner_size.height,
                ),
            );
            let node = element
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, &limits);
            let position = Point::new(
                self.padding.left,
                self.padding.top + (inner_size.height - node.size().height) / 2.0,
            );
            children.push(node.move_to(position));
        }
        let state = tree.state.downcast_mut::<State>();
        if self.matches.is_some() {
            state.input.layout(
                renderer,
                &limits,
                input::Layout {
                    width: Length::Fixed(size.width),
                    height: Length::Fixed(size.height),
                    padding: self.padding,
                    placeholder: &self.placeholder,
                    font: None,
                    size: Some(self.size),
                    line_height: text::LineHeight::default(),
                    alignment: text::Alignment::Default,
                    multiline: None,
                    is_secure: false,
                },
            );
        }
        layout::Node::with_children(size, children)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();
        if self.matches.is_some() && state.is_open {
            operation.focusable(None, layout.bounds(), &mut state.input);
            operation.text_input(None, layout.bounds(), &mut state.input);
        } else if let Some((_, element)) = &mut self.selected {
            element.as_widget_mut().operate(
                &mut tree.children[0],
                layout.children().next().unwrap(),
                renderer,
                operation,
            );
        }
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
        let state = tree.state.downcast_mut::<State>();
        let searching = self.matches.is_some();
        if searching && state.is_open {
            if state
                .input
                .update(
                    event,
                    layout.bounds(),
                    cursor,
                    shell,
                    editor::Binding::from_key_press,
                )
                .is_some()
            {
                state.hovered_option = self.visible_options(&state.input.value()).first().copied();
                state.menu = None;
                shell.invalidate_layout();
                shell.request_redraw();
            }
            if !state.input.is_focused() {
                state.is_open = false;
                state.input.overwrite("");
                shell.invalidate_layout();
                shell.request_redraw();
            }
            if let Event::Window(window::Event::RedrawRequested(_)) = event {
                shell.request_input_method(
                    &state
                        .input
                        .input_method(layout.bounds().shrink(self.padding).position()),
                );
            }
        }
        match event {
            Event::Pointer(event)
                if event.is_primary_press()
                    && (!state.is_open && cursor.is_over(layout.bounds())
                        || state.is_open && (!searching || !cursor.is_over(layout.bounds()))) =>
            {
                state.is_open = !state.is_open;
                state.input.overwrite("");
                if state.is_open {
                    if searching {
                        state.input.focus();
                    }
                    state.menu = None;
                    state.hovered_option = self
                        .options
                        .iter()
                        .position(|(option, _)| {
                            Some(option) == self.selected.as_ref().map(|(value, _)| value)
                        })
                        .or_else(|| (!self.options.is_empty()).then_some(0));
                } else {
                    state.input.unfocus();
                }
                shell.invalidate_layout();
                shell.capture_event();
                shell.request_redraw();
            }
            Event::Pointer(pointer::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { y, .. },
            }) if state.modifiers.command()
                && cursor.is_over(layout.bounds())
                && !state.is_open =>
            {
                let selected = self.options.iter().position(|(option, _)| {
                    Some(option) == self.selected.as_ref().map(|(value, _)| value)
                });
                let next = if *y < 0.0 {
                    selected.map_or(Some(0), |index| index.checked_add(1))
                } else if *y > 0.0 {
                    selected.map_or(self.options.len().checked_sub(1), |index| {
                        index.checked_sub(1)
                    })
                } else {
                    None
                };
                if let Some((option, _)) = next.and_then(|index| self.options.get(index)) {
                    shell.publish((self.on_selected)(option.clone()));
                }
                shell.capture_event();
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(key),
                ..
            }) if state.is_open
                && matches!(
                    key,
                    keyboard::key::Named::Escape
                        | keyboard::key::Named::Enter
                        | keyboard::key::Named::ArrowDown
                        | keyboard::key::Named::ArrowUp
                ) =>
            {
                let visible = self.visible_options(&state.input.value());
                match key {
                    keyboard::key::Named::Escape => {
                        state.is_open = false;
                        state.input.unfocus();
                        state.input.overwrite("");
                    }
                    keyboard::key::Named::Enter => {
                        if let Some(index) =
                            state.hovered_option.filter(|index| visible.contains(index))
                        {
                            shell.publish((self.on_selected)(self.options[index].0.clone()));
                            state.is_open = false;
                            state.input.unfocus();
                            state.input.overwrite("");
                        }
                    }
                    keyboard::key::Named::ArrowDown | keyboard::key::Named::ArrowUp => {
                        let current = visible
                            .iter()
                            .position(|index| Some(*index) == state.hovered_option);
                        let next = if *key == keyboard::key::Named::ArrowDown {
                            current.map_or(0, |index| (index + 1) % visible.len())
                        } else {
                            current
                                .unwrap_or(0)
                                .checked_sub(1)
                                .unwrap_or(visible.len().saturating_sub(1))
                        };
                        state.hovered_option = visible.get(next).copied();
                    }
                    _ => {}
                }
                shell.capture_event();
                shell.invalidate_layout();
                shell.request_redraw();
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
            }
            _ => {}
        }
        let is_hovered = cursor.is_over(layout.bounds());
        let status = if state.is_open {
            pick_list::Status::Opened { is_hovered }
        } else if is_hovered {
            pick_list::Status::Hovered
        } else {
            pick_list::Status::Active
        };
        if let Event::Window(window::Event::RedrawRequested(_)) = event {
            state.last_status = Some(status);
        } else if state.last_status.is_some_and(|last| last != status) {
            shell.request_redraw();
        }
        if !(shell.is_event_captured() || searching && state.is_open)
            && let Some((_, element)) = &mut self.selected
        {
            element.as_widget_mut().update(
                &mut tree.children[0],
                event,
                layout.children().next().unwrap(),
                cursor,
                renderer,
                shell,
                viewport,
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
        if cursor.is_over(layout.bounds()) {
            if self.matches.is_some() && tree.state.downcast_ref::<State>().is_open {
                mouse::Interaction::Text
            } else {
                mouse::Interaction::Pointer
            }
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _defaults: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        if self.matches.is_some() && state.is_open {
            let style = text_input::Catalog::style(
                theme,
                &self.input_class,
                text_input::Status::Focused {
                    is_hovered: cursor.is_over(layout.bounds()),
                },
            );
            renderer.fill_quad(
                renderer::Quad {
                    bounds: layout.bounds(),
                    border: style.border,
                    ..renderer::Quad::default()
                },
                style.background,
            );
            state.input.draw(
                renderer,
                layout.bounds(),
                *viewport,
                input::Style {
                    value: style.value,
                    selection: style.selection,
                    placeholder: style.placeholder,
                },
            );
            return;
        }
        let style = pick_list_style(
            theme,
            state.last_status.unwrap_or(pick_list::Status::Active),
        );
        renderer.fill_quad(
            renderer::Quad {
                bounds: layout.bounds(),
                border: style.border,
                ..renderer::Quad::default()
            },
            style.background,
        );
        let bounds = layout.bounds();
        let line_height = text::LineHeight::default();
        let text = text::Text {
            content: String::new(),
            size: self.size,
            line_height,
            font: Renderer::ICON_FONT,
            bounds: Size::new(self.size.0, line_height.to_absolute(self.size).0),
            align_x: text::Alignment::Right,
            align_y: alignment::Vertical::Center,
            shaping: text::Shaping::Advanced,
            wrapping: text::Wrapping::None,
            ellipsis: text::Ellipsis::None,
            hint_factor: renderer.hint_factor(),
        };

        renderer.fill_text(
            text::Text {
                content: Renderer::ARROW_DOWN_ICON.to_string(),
                ..text
            },
            Point::new(
                bounds.x + bounds.width - self.padding.right,
                bounds.center_y(),
            ),
            style.handle_color,
            *viewport,
        );

        if let Some((_, element)) = &self.selected {
            element.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                &renderer::Style {
                    text_color: style.text_color,
                },
                layout.children().next().unwrap(),
                cursor,
                viewport,
            );
        } else {
            renderer.fill_text(
                text::Text {
                    content: self.placeholder.clone(),
                    font: renderer.default_font(),
                    bounds: Size::new(
                        (bounds.width - self.padding.x() - self.size.0 - self.padding.left)
                            .max(0.0),
                        text.bounds.height,
                    ),
                    align_x: text::Alignment::Default,
                    ..text
                },
                Point::new(bounds.x + self.padding.left, bounds.center_y()),
                style.placeholder_color,
                *viewport,
            );
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
        let state = tree.state.downcast_mut::<State>();
        if !state.is_open {
            return self.selected.as_mut().and_then(|(_, element)| {
                element.as_widget_mut().overlay(
                    &mut tree.children[0],
                    layout.children().next().unwrap(),
                    renderer,
                    viewport,
                    translation,
                )
            });
        }

        let visible = self.visible_options(&state.input.value());
        let on_selected = &self.on_selected;
        let is_open = &mut state.is_open;
        let input = &mut state.input;
        let list = List {
            options: &mut self.options,
            visible,
            hovered_option: &mut state.hovered_option,
            on_selected: Box::new(move |option| {
                *is_open = false;
                input.unfocus();
                input.overwrite("");
                on_selected(option)
            }),
            padding: self.padding,
            class: &self.menu_class,
        };
        let mut list = Scrollable::new(Element::new(list)).height(self.menu_height);
        let tree = state.menu.get_or_insert_with(Tree::empty);
        tree.diff(&mut list as &mut dyn Widget<_, _, _>);
        Some(overlay::Element::new(Box::new(MenuOverlay {
            position: layout.position() + translation,
            target_height: layout.bounds().height,
            width: layout.bounds().width,
            list,
            tree,
            class: &self.menu_class,
        })))
    }
}

/// Internal list widget that lays out and renders filtered [`ComboBox`] options inside the overlay.
struct List<'a, 'b, T, Message> {
    options: &'b mut [(T, Element<'a, Message, Theme, Renderer>)],
    visible: Vec<usize>,
    hovered_option: &'b mut Option<usize>,
    on_selected: Box<dyn FnMut(T) -> Message + 'b>,
    padding: Padding,
    class: &'b <Theme as menu::Catalog>::Class<'a>,
}

impl<T: Clone, Message> Widget<Message, Theme, Renderer> for List<'_, '_, T, Message> {
    fn diff(&mut self, tree: &mut Tree) {
        let mut children = self
            .options
            .iter_mut()
            .map(|(_, element)| element)
            .collect::<Vec<_>>();
        tree.diff_children(&mut children);
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(Length::Fill);
        let width = limits.max().width;
        let mut height = 0.0;
        let children = self
            .visible
            .iter()
            .map(|&index| {
                let limits =
                    layout::Limits::new(Size::new(width, 0.0), Size::new(width, f32::INFINITY))
                        .height(Length::Shrink)
                        .shrink(self.padding);
                let node = self.options[index].1.as_widget_mut().layout(
                    &mut tree.children[index],
                    renderer,
                    &limits,
                );
                let size = node.size().expand(self.padding);
                let row = layout::Node::with_children(
                    size,
                    vec![node.move_to(Point::new(self.padding.left, self.padding.top))],
                );
                let position = Point::new(0.0, height);
                height += size.height;
                row.move_to(position)
            })
            .collect();

        layout::Node::with_children(Size::new(width, height), children)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            for (&index, layout) in self.visible.iter().zip(layout.children()) {
                self.options[index].1.as_widget_mut().operate(
                    &mut tree.children[index],
                    layout.children().next().unwrap(),
                    renderer,
                    operation,
                );
            }
        });
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
        let hovered = cursor
            .position()
            .filter(|position| viewport.contains(*position))
            .and_then(|position| {
                layout
                    .children()
                    .position(|item| {
                        Rectangle {
                            x: layout.bounds().x,
                            width: layout.bounds().width,
                            ..item.bounds()
                        }
                        .contains(position)
                    })
                    .map(|index| self.visible[index])
            });

        if matches!(
            event,
            Event::Pointer(
                pointer::Event::PointerMoved { .. }
                    | pointer::Event::PointerEntered { .. }
                    | pointer::Event::PointerLeft { .. }
            )
        ) && hovered != *self.hovered_option
        {
            *self.hovered_option = hovered;
            shell.request_redraw();
        }

        if let Event::Pointer(event) = event
            && event.is_primary_press()
            && cursor.is_over(*viewport)
            && let Some(index) = hovered
        {
            shell.publish((self.on_selected)(self.options[index].0.clone()));
            shell.invalidate_layout();
            shell.capture_event();
            shell.request_redraw();
            return;
        }

        for (&index, layout) in self.visible.iter().zip(layout.children()) {
            self.options[index].1.as_widget_mut().update(
                &mut tree.children[index],
                event,
                layout.children().next().unwrap(),
                cursor,
                renderer,
                shell,
                viewport,
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
        if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _defaults: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let style = menu::Catalog::style(theme, self.class);

        for (&index, item_layout) in self.visible.iter().zip(layout.children()) {
            let item = &self.options[index].1;
            let tree = &tree.children[index];
            let bounds = Rectangle {
                x: layout.bounds().x,
                width: layout.bounds().width,
                ..item_layout.bounds()
            };
            if !bounds.intersects(viewport) {
                continue;
            }

            let selected = *self.hovered_option == Some(index);
            if selected {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            x: bounds.x + style.border.width,
                            width: (bounds.width - style.border.width * 2.0).max(0.0),
                            ..bounds
                        },
                        ..renderer::Quad::default()
                    },
                    style.selected_background,
                );
            }
            item.as_widget().draw(
                tree,
                renderer,
                theme,
                &renderer::Style {
                    text_color: if selected {
                        style.selected_text_color
                    } else {
                        style.text_color
                    },
                },
                item_layout.children().next().unwrap(),
                cursor,
                viewport,
            );
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
        let mut layouts = layout.children();
        let overlays = self
            .options
            .iter_mut()
            .zip(&mut tree.children)
            .enumerate()
            .filter(|(index, _)| self.visible.binary_search(index).is_ok())
            .filter_map(|(_, ((_, item), tree))| {
                item.as_widget_mut().overlay(
                    tree,
                    layouts.next().unwrap().children().next().unwrap(),
                    renderer,
                    viewport,
                    translation,
                )
            })
            .collect::<Vec<_>>();
        (!overlays.is_empty()).then(|| overlay::Group::with_children(overlays).overlay())
    }
}

/// Floating dropdown overlay for an open [`ComboBox`].
struct MenuOverlay<'a: 'b, 'b, Message> {
    position: Point,
    target_height: f32,
    width: f32,
    list: Scrollable<'b, Message, Theme, Renderer>,
    tree: &'b mut Tree,
    class: &'b <Theme as menu::Catalog>::Class<'a>,
}

impl<Message> overlay::Overlay<Message, Theme, Renderer> for MenuOverlay<'_, '_, Message> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let below = bounds.height - (self.position.y + self.target_height);
        let above = self.position.y;
        let limits = layout::Limits::new(
            Size::ZERO,
            Size::new(
                (bounds.width - self.position.x).max(0.0),
                below.max(above).max(0.0),
            ),
        )
        .width(self.width);
        let node = self.list.layout(self.tree, renderer, &limits);
        let height = node.size().height;
        node.move_to(if height <= below || below >= above {
            self.position + Vector::new(0.0, self.target_height)
        } else {
            self.position - Vector::new(0.0, height)
        })
    }

    fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.list.operate(self.tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        self.list.update(
            self.tree,
            event,
            layout,
            cursor,
            renderer,
            shell,
            &layout.bounds(),
        );
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.list
            .mouse_interaction(self.tree, layout, cursor, &layout.bounds(), renderer)
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let style = menu::Catalog::style(theme, self.class);
        renderer.fill_quad(
            renderer::Quad {
                bounds: layout.bounds(),
                border: style.border,
                shadow: style.shadow,
                ..renderer::Quad::default()
            },
            style.background,
        );
        self.list.draw(
            self.tree,
            renderer,
            theme,
            defaults,
            layout,
            cursor,
            &layout.bounds(),
        );
    }

    fn overlay<'b>(
        &'b mut self,
        layout: Layout<'b>,
        renderer: &Renderer,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.list
            .overlay(self.tree, layout, renderer, &layout.bounds(), Vector::ZERO)
    }
}

pub fn menu_style(theme: &Theme) -> menu::Style {
    let p = theme.palette();
    menu::Style {
        background: p.background.weakest.color.into(),
        border: Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.background.strong.color,
        },
        text_color: p.background.base.text,
        selected_text_color: p.primary.weak.text,
        selected_background: p.primary.weak.color.into(),
        shadow: Shadow {
            color: Color::BLACK.scale_alpha(0.25),
            offset: Vector::new(3.0, 3.0),
            blur_radius: 0.0,
        },
    }
}

pub fn pick_list_style(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let p = theme.palette();
    let highlighted = !matches!(status, pick_list::Status::Active);
    pick_list::Style {
        text_color: p.background.base.text,
        placeholder_color: p.background.weak.text,
        handle_color: p.background.weak.text,
        background: p.background.base.color.into(),
        border: Border {
            radius: 0.0.into(),
            width: 1.0,
            color: if highlighted {
                p.primary.base.color
            } else {
                p.background.strong.color
            },
        },
    }
}
