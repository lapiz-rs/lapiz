use iced_core::{Border, Element, Length, Padding, Pixels, Theme, alignment};
use lapiz_runtime::Renderer;
use iced_widget::Container;

pub use iced_widget::container::{Catalog, Style, StyleFn};

pub struct Panel<'a, Message> {
    inner: Container<'a, Message, Theme, Renderer>,
    width: Length,
    height: Length,
}

impl<'a, Message> Panel<'a, Message> {
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>) -> Self {
        Self {
            inner: Container::new(content).style(default),
            width: Length::Shrink,
            height: Length::Shrink,
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

    pub fn max_width(mut self, width: impl Into<Pixels>) -> Self {
        self.width = self.width.max(width);
        self
    }

    pub fn max_height(mut self, height: impl Into<Pixels>) -> Self {
        self.height = self.height.max(height);
        self
    }

    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.inner = self.inner.padding(padding);
        self
    }

    pub fn align_x(mut self, alignment: impl Into<alignment::Horizontal>) -> Self {
        self.inner = self.inner.align_x(alignment);
        self
    }

    pub fn align_y(mut self, alignment: impl Into<alignment::Vertical>) -> Self {
        self.inner = self.inner.align_y(alignment);
        self
    }

    pub fn clip(mut self, clip: bool) -> Self {
        self.inner = self.inner.clip(clip);
        self
    }

    pub fn style(mut self, style: impl Fn(&Theme) -> Style + 'a) -> Self {
        self.inner = self.inner.style(style);
        self
    }

    pub fn class(mut self, class: impl Into<<Theme as Catalog>::Class<'a>>) -> Self {
        self.inner = self.inner.class(class);
        self
    }

    pub fn inset(self) -> Self {
        self.style(inset)
    }

    pub fn transparent(self) -> Self {
        self.style(transparent)
    }
}

impl<'a, Message: 'a> From<Panel<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: Panel<'a, Message>) -> Self {
        let Panel {
            mut inner,
            width,
            height,
        } = value;
        inner = inner.width(width).height(height);
        inner.into()
    }
}

pub fn default(theme: &Theme) -> Style {
    let p = theme.palette();
    Style::default()
        .background(p.background.weakest.color)
        .color(p.background.base.text)
        .border(Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.background.strong.color,
        })
}

pub fn inset(theme: &Theme) -> Style {
    let p = theme.palette();
    Style::default()
        .background(p.background.strongest.color)
        .color(p.background.base.text)
        .border(Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.background.strong.color,
        })
}

pub fn transparent(_theme: &Theme) -> Style {
    Style::default()
}
