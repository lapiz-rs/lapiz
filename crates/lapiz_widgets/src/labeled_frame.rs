use iced_core::{
    Border, Element, Layout, Length, Point, Rectangle, Renderer as _, Size, Theme, Vector, Widget,
    layout, overlay,
    pointer::mouse,
    renderer,
    widget::{Operation, Tree},
};
use lapiz_runtime::Renderer;

const BORDER: f32 = 1.0;
const PADDING: f32 = 12.0;
const TITLE_SPACING: f32 = 8.0;

pub struct LabeledFrame<'a, Message> {
    title: Element<'a, Message, Theme, Renderer>,
    content: Element<'a, Message, Theme, Renderer>,
    width: Length,
    height: Length,
}

impl<'a, Message> LabeledFrame<'a, Message> {
    pub fn new(
        title: impl Into<Element<'a, Message, Theme, Renderer>>,
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        Self {
            title: title.into(),
            content: content.into(),
            width: Length::Fit,
            height: Length::Fit,
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
}

impl<Message> Widget<Message, Theme, Renderer> for LabeledFrame<'_, Message> {
    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut [&mut self.title, &mut self.content]);
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
        let loose = limits.loose();
        let title = self
            .title
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &loose);
        let content_limits = loose.shrink(Size::new(
            (PADDING + BORDER) * 2.0,
            (PADDING + BORDER) * 2.0 + title.size().height / 2.0,
        ));
        let content =
            self.content
                .as_widget_mut()
                .layout(&mut tree.children[1], renderer, &content_limits);
        let intrinsic = Size::new(
            content.size().width.max(title.size().width) + (PADDING + BORDER) * 2.0,
            content.size().height + (PADDING + BORDER) * 2.0 + title.size().height / 2.0,
        );
        let size = limits.resolve(self.width, self.height, intrinsic);
        let title_height = title.size().height;
        let title = title.move_to(Point::new(
            BORDER + PADDING / 2.0,
            BORDER - title_height / 2.0,
        ));
        let content = content.move_to(Point::new(
            BORDER + PADDING,
            BORDER + title_height / 2.0 + PADDING / 2.0,
        ));
        layout::Node::with_children(size, vec![title, content])
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        let mut children = layout.children();
        let title_layout = children.next().expect("title layout");
        let content_layout = children.next().expect("content layout");
        operation.traverse(&mut |operation| {
            self.title.as_widget_mut().operate(
                &mut tree.children[0],
                title_layout,
                renderer,
                operation,
            );
            self.content.as_widget_mut().operate(
                &mut tree.children[1],
                content_layout,
                renderer,
                operation,
            );
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced_core::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut iced_core::Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let mut children = layout.children();
        let title_layout = children.next().expect("title layout");
        let content_layout = children.next().expect("content layout");
        self.title.as_widget_mut().update(
            &mut tree.children[0],
            event,
            title_layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
        self.content.as_widget_mut().update(
            &mut tree.children[1],
            event,
            content_layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &iced_core::renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let p = theme.palette();
        let bounds = layout.bounds();
        renderer.fill_quad(
            renderer::Quad {
                bounds,
                border: Border {
                    radius: 0.0.into(),
                    width: BORDER,
                    color: p.background.strong.color,
                },
                ..renderer::Quad::default()
            },
            iced_core::Color::TRANSPARENT,
        );
        let mut children = layout.children();
        let title_layout = children.next().expect("title layout");
        let title_bounds = title_layout.bounds();
        let backing = Rectangle::new(
            Point::new(title_bounds.x - TITLE_SPACING / 2.0, title_bounds.y),
            Size::new(title_bounds.width + TITLE_SPACING, title_bounds.height),
        );
        renderer.fill_quad(
            renderer::Quad {
                bounds: backing,
                ..renderer::Quad::default()
            },
            p.background.weakest.color,
        );
        self.title.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            title_layout,
            cursor,
            viewport,
        );
        self.content.as_widget().draw(
            &tree.children[1],
            renderer,
            theme,
            style,
            children.next().expect("content layout"),
            cursor,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let mut children = layout.children();
        let title_layout = children.next().expect("title layout");
        let content_layout = children.next().expect("content layout");
        self.title
            .as_widget()
            .mouse_interaction(&tree.children[0], title_layout, cursor, viewport, renderer)
            .max(self.content.as_widget().mouse_interaction(
                &tree.children[1],
                content_layout,
                cursor,
                viewport,
                renderer,
            ))
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let [title_tree, content_tree] = tree.children.as_mut_slice() else {
            return None;
        };
        let mut layouts = layout.children();
        let title = self.title.as_widget_mut().overlay(
            title_tree,
            layouts.next()?,
            renderer,
            viewport,
            translation,
        );
        let content = self.content.as_widget_mut().overlay(
            content_tree,
            layouts.next()?,
            renderer,
            viewport,
            translation,
        );
        let children = [title, content].into_iter().flatten().collect::<Vec<_>>();
        (!children.is_empty()).then(|| overlay::Group::with_children(children).overlay())
    }
}

impl<'a, Message: 'a> From<LabeledFrame<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: LabeledFrame<'a, Message>) -> Self {
        Element::new(value)
    }
}
