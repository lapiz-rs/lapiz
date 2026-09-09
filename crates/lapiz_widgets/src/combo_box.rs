use iced_core::{Border, Color, Element, Length, Padding, Pixels, Shadow, Theme, Vector};
pub use iced_widget::combo_box::Catalog;
use iced_widget::{overlay::menu, pick_list, text_input};
use lapiz_runtime::Renderer;

#[derive(Debug, Clone)]
pub struct State<T> {
    combo: iced_widget::combo_box::State<T>,
    options: Vec<T>,
}

impl<T> State<T>
where
    T: std::fmt::Display + Clone,
{
    pub fn new(options: Vec<T>) -> Self {
        Self {
            combo: iced_widget::combo_box::State::new(options.clone()),
            options,
        }
    }

    pub fn options(&self) -> &[T] {
        &self.options
    }

    pub fn push(&mut self, option: T) {
        self.combo.push(option.clone());
        self.options.push(option);
    }
}

enum Inner<'a, T, Message>
where
    T: std::fmt::Display + Clone,
{
    Plain {
        options: Vec<T>,
        selected: Option<T>,
        on_selected: Box<dyn Fn(T) -> Message + 'a>,
    },
    Searchable(Box<iced_widget::ComboBox<'a, T, Message, Theme, Renderer>>),
}

pub struct ComboBox<'a, T, Message>
where
    T: std::fmt::Display + Clone,
{
    inner: Inner<'a, T, Message>,
    placeholder: String,
    width: Length,
    menu_height: Length,
    padding: Padding,
    size: Pixels,
    input_class: <Theme as text_input::Catalog>::Class<'a>,
    menu_class: <Theme as menu::Catalog>::Class<'a>,
}

impl<'a, T, Message> ComboBox<'a, T, Message>
where
    T: std::fmt::Display + Clone,
{
    pub fn new(
        options: impl Into<Vec<T>>,
        selected: Option<T>,
        on_selected: impl Fn(T) -> Message + 'a,
    ) -> Self {
        Self {
            inner: Inner::Plain {
                options: options.into(),
                selected,
                on_selected: Box::new(on_selected),
            },
            placeholder: String::new(),
            width: Length::Shrink,
            menu_height: Length::Shrink,
            padding: Padding::from([5, 8]),
            size: Pixels(12.0),
            input_class: Box::new(crate::text_input::default),
            menu_class: Box::new(menu_style),
        }
    }

    pub fn searchable(
        state: &'a State<T>,
        placeholder: impl Into<String>,
        selected: Option<T>,
        on_selected: impl Fn(T) -> Message + 'static,
    ) -> Self
    where
        T: 'static,
        Message: 'static,
    {
        Self {
            inner: Inner::Searchable(Box::new(iced_widget::ComboBox::new(
                &state.combo,
                placeholder.into(),
                selected.as_ref(),
                on_selected,
            ))),
            placeholder: String::new(),
            width: Length::Shrink,
            menu_height: Length::Shrink,
            padding: Padding::from([5, 8]),
            size: Pixels(12.0),
            input_class: Box::new(crate::text_input::default),
            menu_class: Box::new(menu_style),
        }
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
        self.size = size.into();
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
}

impl<'a, T, Message> From<ComboBox<'a, T, Message>> for Element<'a, Message, Theme, Renderer>
where
    T: std::fmt::Display + Clone + PartialEq + 'a + 'static,
    Message: Clone + 'a,
{
    fn from(value: ComboBox<'a, T, Message>) -> Self {
        match value.inner {
            Inner::Searchable(combo) => (*combo)
                .width(value.width)
                .menu_height(value.menu_height)
                .padding(value.padding)
                .size(value.size)
                .input_class(value.input_class)
                .menu_class(value.menu_class)
                .into(),
            Inner::Plain {
                options,
                selected,
                on_selected,
            } => iced_widget::PickList::new(selected, options, |option: &T| option.to_string())
                .on_select(on_selected)
                .placeholder(value.placeholder)
                .width(value.width)
                .menu_height(value.menu_height)
                .padding(value.padding)
                .text_size(value.size)
                .style(pick_list_style)
                .menu_class(value.menu_class)
                .into(),
        }
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
