use iced_core::Renderer as _;
use iced_core::{
    Background, Border, Clipboard, Color, Element, Event, Layout, Length, Padding, Pixels, Point,
    Rectangle, Shell, Size, Theme, Vector, Widget, keyboard, layout, mouse, overlay, renderer,
    widget::{Operation, Tree},
};
use iced_wgpu::Renderer;
use iced_widget::container;
use taffy::prelude::{
    AlignItems, AvailableSpace, Dimension, Display, FlexDirection, FlexWrap, JustifyContent,
    LengthPercentage, TaffyAuto, TaffyTree,
};

use crate::callback::{Callback, CallbackWith, publish, publish_with};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Active,
    Hovered,
}

pub type Style = container::Style;
pub type StyleFn<'a> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

pub trait Catalog {
    type Class<'a>;

    fn default<'a>() -> Self::Class<'a>;

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style;
}

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(transparent)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

pub struct Flex<'a, Message> {
    children: Vec<Element<'a, Message, Theme, Renderer>>,
    taffy_style: taffy::Style,
    direction: FlexDirection,
    width: Length,
    height: Length,
    padding: Padding,
    gap: f32,
    clip: bool,
    press: Callback<'a, Message>,
    release: Callback<'a, Message>,
    key_event: CallbackWith<'a, keyboard::Event, Message>,
    class: <Theme as Catalog>::Class<'a>,
}

impl<'a, Message> Flex<'a, Message> {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            taffy_style: taffy::Style::default(),
            direction: FlexDirection::Row,
            width: Length::Shrink,
            height: Length::Shrink,
            padding: Padding::ZERO,
            gap: 0.0,
            clip: false,
            press: Callback::Empty,
            release: Callback::Empty,
            key_event: None,
            class: <Theme as Catalog>::default(),
        }
    }

    pub fn row(children: impl IntoIterator<Item = Element<'a, Message, Theme, Renderer>>) -> Self {
        Self::new()
            .direction(FlexDirection::Row)
            .align_items(AlignItems::Center)
            .extend(children)
    }

    pub fn column(
        children: impl IntoIterator<Item = Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        Self::new()
            .direction(FlexDirection::Column)
            .extend(children)
    }

    pub fn push(mut self, child: impl Into<Element<'a, Message, Theme, Renderer>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn extend(
        mut self,
        children: impl IntoIterator<Item = Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        self.children.extend(children);
        self
    }

    fn direction(mut self, direction: FlexDirection) -> Self {
        self.direction = direction;
        self
    }

    fn align_items(mut self, alignment: AlignItems) -> Self {
        self.taffy_style.align_items = Some(alignment);
        self
    }

    fn justify_content(mut self, justification: JustifyContent) -> Self {
        self.taffy_style.justify_content = Some(justification);
        self
    }

    pub fn space_between(self) -> Self {
        self.justify_content(JustifyContent::SpaceBetween)
    }

    pub fn space_evenly(self) -> Self {
        self.justify_content(JustifyContent::SpaceEvenly)
    }

    pub fn wrap(mut self) -> Self {
        self.taffy_style.flex_wrap = FlexWrap::Wrap;
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

    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    pub fn gap(mut self, gap: impl Into<Pixels>) -> Self {
        self.gap = gap.into().0;
        self
    }

    pub fn clip(mut self, clip: bool) -> Self {
        self.clip = clip;
        self
    }

    crate::callback_methods!(press);
    crate::callback_methods!(release);
    crate::callback_methods!(key_event, keyboard::Event);

    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self {
        self.class = Box::new(style);
        self
    }

    pub fn class(mut self, class: impl Into<<Theme as Catalog>::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }

    pub fn panel(self) -> Self {
        self.style(panel)
    }

    pub fn surface(self) -> Self {
        self.style(surface)
    }

    pub fn transparent(self) -> Self {
        self.style(transparent)
    }
}

impl<Message> Default for Flex<'_, Message> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Flex<'_, Message> {
    fn children(&self) -> Vec<Tree> {
        self.children.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.children);
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let horizontal = self.is_horizontal();
        let measured = self.measure_children(tree, renderer, limits);
        let intrinsic = self.intrinsic_size(&measured, horizontal);
        let resolved = limits.resolve(self.width, self.height, intrinsic);

        let measure_height =
            self.taffy_style.flex_wrap == FlexWrap::Wrap && matches!(self.height, Length::Shrink);

        let mut taffy = TaffyTree::<()>::new();
        let leaves = self.taffy_leaves(&mut taffy, &measured, horizontal);
        let root = taffy
            .new_with_children(self.root_style(resolved, measure_height), &leaves)
            .unwrap();
        let available = taffy::Size {
            width: AvailableSpace::Definite(resolved.width),
            height: if measure_height {
                AvailableSpace::MaxContent
            } else {
                AvailableSpace::Definite(resolved.height)
            },
        };
        taffy.compute_layout(root, available).unwrap();

        let size = if measure_height {
            Size::new(resolved.width, taffy.layout(root).unwrap().size.height)
        } else {
            resolved
        };
        let children = self.place_children(tree, renderer, &taffy, &leaves);
        layout::Node::with_children(size, children)
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
            for ((child, state), layout) in self
                .children
                .iter_mut()
                .zip(&mut tree.children)
                .zip(layout.children())
            {
                child
                    .as_widget_mut()
                    .operate(state, layout, renderer, operation);
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
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        for ((child, state), layout) in self
            .children
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
        {
            child.as_widget_mut().update(
                state, event, layout, cursor, renderer, clipboard, shell, viewport,
            );
        }
        if shell.is_event_captured() || !cursor.is_over(layout.bounds()) {
            return;
        }
        let message = match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                publish(&mut self.press)
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                publish(&mut self.release)
            }
            Event::Keyboard(event) => publish_with(&mut self.key_event, event.clone()),
            _ => None,
        };
        if let Some(message) = message {
            shell.publish(message);
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let own = if self.press.is_set() && cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        };
        self.children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((child, state), layout)| {
                child
                    .as_widget()
                    .mouse_interaction(state, layout, cursor, viewport, renderer)
            })
            .fold(mouse::Interaction::None, |acc, interaction| {
                acc.max(interaction)
            })
            .max(own)
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
        let bounds = layout.bounds();
        let style = theme.style(
            &self.class,
            if cursor.is_over(bounds) {
                Status::Hovered
            } else {
                Status::Active
            },
        );
        if style.background.is_some() || style.border.width > 0.0 || style.shadow.color.a > 0.0 {
            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    border: style.border,
                    shadow: style.shadow,
                    snap: style.snap,
                },
                style
                    .background
                    .unwrap_or(Background::Color(Color::TRANSPARENT)),
            );
        }
        let clipped = bounds.intersection(viewport).unwrap_or(*viewport);
        let viewport = if self.clip { &clipped } else { viewport };
        let renderer_style = renderer::Style {
            text_color: style.text_color.unwrap_or(renderer_style.text_color),
        };
        for ((child, state), layout) in self
            .children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
        {
            if layout.bounds().intersects(viewport) {
                child.as_widget().draw(
                    state,
                    renderer,
                    theme,
                    &renderer_style,
                    layout,
                    cursor,
                    viewport,
                );
            }
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
        overlay::from_children(
            &mut self.children,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<Message> Flex<'_, Message> {
    fn is_horizontal(&self) -> bool {
        matches!(
            self.direction,
            FlexDirection::Row | FlexDirection::RowReverse
        )
    }

    fn measure_children(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> Vec<layout::Node> {
        let loose = limits.loose();
        self.children
            .iter_mut()
            .zip(&mut tree.children)
            .map(|(child, state)| child.as_widget_mut().layout(state, renderer, &loose))
            .collect()
    }

    fn intrinsic_size(&self, measured: &[layout::Node], horizontal: bool) -> Size {
        let mut intrinsic = Size::from(self.padding);
        for node in measured {
            let size = node.size();
            if horizontal {
                intrinsic.width += size.width;
                intrinsic.height = intrinsic.height.max(size.height + self.padding.y());
            } else {
                intrinsic.height += size.height;
                intrinsic.width = intrinsic.width.max(size.width + self.padding.x());
            }
        }
        let gaps = self.gap * measured.len().saturating_sub(1) as f32;
        if horizontal {
            intrinsic.width += gaps;
        } else {
            intrinsic.height += gaps;
        }
        intrinsic
    }

    fn taffy_leaves(
        &self,
        taffy: &mut TaffyTree<()>,
        measured: &[layout::Node],
        horizontal: bool,
    ) -> Vec<taffy::NodeId> {
        self.children
            .iter()
            .zip(measured)
            .map(|(child, node)| {
                let style = self.leaf_style(child.as_widget().size(), node.size(), horizontal);
                taffy.new_leaf(style).unwrap()
            })
            .collect()
    }

    fn leaf_style(
        &self,
        requested: Size<Length>,
        measured: Size,
        horizontal: bool,
    ) -> taffy::Style {
        let main = if horizontal {
            requested.width
        } else {
            requested.height
        };
        let cross = if horizontal {
            requested.height
        } else {
            requested.width
        };
        let mut style = taffy::Style {
            size: taffy::Size {
                width: Dimension::length(measured.width),
                height: Dimension::length(measured.height),
            },
            ..taffy::Style::default()
        };
        if main.is_fill() {
            style.flex_grow = main.fill_factor() as f32;
            if horizontal {
                style.size.width = Dimension::AUTO;
            } else {
                style.size.height = Dimension::AUTO;
            }
        } else {
            style.flex_shrink = 0.0;
        }
        if cross.is_fill() {
            if horizontal {
                style.size.height = Dimension::percent(1.0);
            } else {
                style.size.width = Dimension::percent(1.0);
            }
        }
        style
    }

    fn root_style(&self, resolved: Size, measure_height: bool) -> taffy::Style {
        let mut style = self.taffy_style.clone();
        style.display = Display::Flex;
        style.flex_direction = self.direction;
        style.size = taffy::Size {
            width: Dimension::length(resolved.width),
            height: if measure_height {
                Dimension::AUTO
            } else {
                Dimension::length(resolved.height)
            },
        };
        style.padding = taffy::Rect {
            left: LengthPercentage::length(self.padding.left),
            right: LengthPercentage::length(self.padding.right),
            top: LengthPercentage::length(self.padding.top),
            bottom: LengthPercentage::length(self.padding.bottom),
        };
        style.gap = taffy::Size {
            width: LengthPercentage::length(self.gap),
            height: LengthPercentage::length(self.gap),
        };
        style
    }

    fn place_children(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        taffy: &TaffyTree<()>,
        leaves: &[taffy::NodeId],
    ) -> Vec<layout::Node> {
        self.children
            .iter_mut()
            .zip(&mut tree.children)
            .zip(leaves.iter().copied())
            .map(|((child, state), leaf)| {
                let result = *taffy.layout(leaf).unwrap();
                let size = Size::new(result.size.width, result.size.height);
                let exact = layout::Limits::new(size, size);
                child
                    .as_widget_mut()
                    .layout(state, renderer, &exact)
                    .move_to(Point::new(result.location.x, result.location.y))
            })
            .collect()
    }
}

impl<'a, Message: 'a> From<Flex<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: Flex<'a, Message>) -> Self {
        Element::new(value)
    }
}

pub fn transparent(_theme: &Theme, _status: Status) -> Style {
    Style::default()
}

pub fn panel(theme: &Theme, _status: Status) -> Style {
    let p = theme.extended_palette();
    Style::default()
        .background(p.background.base.color)
        .color(p.background.base.text)
        .border(Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.background.strong.color,
        })
}

pub fn surface(theme: &Theme, status: Status) -> Style {
    let p = theme.extended_palette();
    Style::default()
        .background(if status == Status::Hovered {
            p.background.weak.color
        } else {
            p.background.weakest.color
        })
        .color(p.background.base.text)
        .border(Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.background.strong.color,
        })
}
