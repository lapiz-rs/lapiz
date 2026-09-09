use iced_core::{Element, Length, Theme};
use lapiz_runtime::Renderer;

pub use iced_widget::scrollable::{Catalog, Direction, Scrollbar, Status, Style, StyleFn};

pub struct Scrollable<'a, Message> {
    inner: iced_widget::Scrollable<'a, Message, Theme, Renderer>,
}

impl<'a, Message> Scrollable<'a, Message> {
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>) -> Self {
        Self {
            inner: iced_widget::Scrollable::new(content).style(default),
        }
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.inner = self.inner.width(width);
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.inner = self.inner.height(height);
        self
    }

    pub fn direction(mut self, direction: impl Into<Direction>) -> Self {
        self.inner = self.inner.direction(direction);
        self
    }

    pub fn horizontal(mut self) -> Self {
        self.inner = self.inner.horizontal();
        self
    }

    pub fn anchor_top(mut self) -> Self {
        self.inner = self.inner.anchor_top();
        self
    }

    pub fn anchor_bottom(mut self) -> Self {
        self.inner = self.inner.anchor_bottom();
        self
    }

    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self {
        self.inner = self.inner.style(style);
        self
    }

    pub fn class(mut self, class: impl Into<<Theme as Catalog>::Class<'a>>) -> Self {
        self.inner = self.inner.class(class);
        self
    }
}

impl<'a, Message: 'a> From<Scrollable<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: Scrollable<'a, Message>) -> Self {
        value.inner.into()
    }
}

pub fn default(theme: &Theme, status: Status) -> Style {
    let p = theme.palette();
    let active = matches!(status, Status::Dragged { .. });
    let mut style = iced_widget::scrollable::default(theme, status);
    style.vertical_rail.background = Some(p.background.base.color.into());
    style.horizontal_rail.background = Some(p.background.base.color.into());
    style.vertical_rail.scroller.background = if active {
        p.primary.base.color.into()
    } else {
        p.background.strong.color.into()
    };
    style.horizontal_rail.scroller.background = style.vertical_rail.scroller.background;
    style.gap = None;
    style
}
