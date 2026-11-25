use std::any::Any;

use iced_core::{
    Border, Element, Event, Layout, Point, Rectangle, Renderer as _, Shell, Size, Theme, Vector,
    Widget, keyboard, layout, overlay, pointer,
    pointer::mouse,
    renderer,
    widget::{Id, Operation, Tree, operation::Scrollable, tree},
    window,
};
use lapiz_runtime::Renderer;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DropZone {
    Before,
    Into,
    After,
}

#[derive(Clone, Copy)]
pub enum DropMode {
    Insert,
    Into,
    Tree,
}

impl DropMode {
    fn zone(self, bounds: Rectangle, point: Point) -> DropZone {
        let fraction = (point.y - bounds.y) / bounds.height.max(1.0);
        match self {
            Self::Into => DropZone::Into,
            Self::Insert if fraction < 0.5 => DropZone::Before,
            Self::Insert => DropZone::After,
            Self::Tree if fraction < 0.25 => DropZone::Before,
            Self::Tree if fraction > 0.75 => DropZone::After,
            Self::Tree => DropZone::Into,
        }
    }
}

#[derive(Clone)]
struct RegionData<T> {
    id: T,
    draggable: bool,
    mode: DropMode,
}

pub struct DragRegion<'a, T, Message> {
    content: Element<'a, Message, Theme, Renderer>,
    data: RegionData<T>,
}

impl<'a, T, Message> DragRegion<'a, T, Message> {
    pub fn new(id: T, content: impl Into<Element<'a, Message, Theme, Renderer>>) -> Self {
        Self {
            content: content.into(),
            data: RegionData {
                id,
                draggable: true,
                mode: DropMode::Insert,
            },
        }
    }

    pub fn draggable(mut self, draggable: bool) -> Self {
        self.data.draggable = draggable;
        self
    }

    pub fn mode(mut self, mode: DropMode) -> Self {
        self.data.mode = mode;
        self
    }
}

impl<T: Clone + 'static, Message> Widget<Message, Theme, Renderer> for DragRegion<'_, T, Message> {
    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut [&mut self.content]);
    }
    fn size(&self) -> Size<iced_core::Length> {
        self.content.as_widget().size()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.custom(None, layout.bounds(), &mut self.data);
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
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
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
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
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
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
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, T: Clone + 'static, Message: 'a> From<DragRegion<'a, T, Message>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(value: DragRegion<'a, T, Message>) -> Self {
        Element::new(value)
    }
}

type Accept<'a, T> = Box<dyn Fn(&T, &T) -> bool + 'a>;

pub struct DragDrop<'a, T, Message> {
    content: Element<'a, Message, Theme, Renderer>,
    accept: Accept<'a, T>,
    on_drop: Box<dyn Fn(T, T, DropZone) -> Message + 'a>,
}

impl<'a, T, Message> DragDrop<'a, T, Message> {
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        accept: impl Fn(&T, &T) -> bool + 'a,
        on_drop: impl Fn(T, T, DropZone) -> Message + 'a,
    ) -> Self {
        Self {
            content: content.into(),
            accept: Box::new(accept),
            on_drop: Box::new(on_drop),
        }
    }
}

struct State<T> {
    pressed: Option<(T, Point)>,
    dragging: bool,
    target: Option<(T, DropZone, Rectangle)>,
}

impl<T> Default for State<T> {
    fn default() -> Self {
        Self {
            pressed: None,
            dragging: false,
            target: None,
        }
    }
}

struct HitTest<T> {
    point: Point,
    translation: Vector,
    clip: Rectangle,
    scroll: Option<(Vector, Rectangle)>,
    hits: Vec<(RegionData<T>, Rectangle)>,
}

impl<T: Clone + Send + 'static> Operation for HitTest<T> {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        let (translation, clip) = (self.translation, self.clip);
        if let Some((offset, bounds)) = self.scroll.take() {
            self.clip = self
                .clip
                .intersection(&(bounds - self.translation))
                .unwrap_or_default();
            self.translation += offset;
        }
        operate(self);
        self.translation = translation;
        self.clip = clip;
    }
    fn scrollable(
        &mut self,
        _: Option<&Id>,
        bounds: Rectangle,
        _: Rectangle,
        translation: Vector,
        _: &mut dyn Scrollable,
    ) {
        self.scroll = Some((translation, bounds));
    }
    fn custom(&mut self, _: Option<&Id>, bounds: Rectangle, state: &mut dyn Any) {
        if let Some(data) = state.downcast_ref::<RegionData<T>>() {
            let bounds = bounds - self.translation;
            if bounds.contains(self.point) && self.clip.contains(self.point) {
                self.hits.push((data.clone(), bounds));
            }
        }
    }
}

impl<T: Clone + Send + 'static, Message> Widget<Message, Theme, Renderer>
    for DragDrop<'_, T, Message>
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<T>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::<T>::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut [&mut self.content]);
    }

    fn size(&self) -> Size<iced_core::Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
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
        let cancel = matches!(
            event,
            Event::Window(window::Event::Unfocused)
                | Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(keyboard::key::Named::Escape),
                    ..
                })
        );
        if cancel {
            *tree.state.downcast_mut::<State<T>>() = State::default();
            shell.request_redraw();
        }
        let position = match event {
            Event::Pointer(
                pointer::Event::PointerMoved { position, .. }
                | pointer::Event::PointerPressed { position, .. }
                | pointer::Event::PointerReleased { position, .. },
            ) => Some(*position),
            _ => None,
        };
        if let Some(point) = position {
            let mut hit = HitTest::<T> {
                point,
                translation: Vector::ZERO,
                clip: *viewport,
                scroll: None,
                hits: Vec::new(),
            };
            self.content
                .as_widget_mut()
                .operate(&mut tree.children[0], layout, renderer, &mut hit);
            let state = tree.state.downcast_mut::<State<T>>();
            match event {
                Event::Pointer(e) if e.is_primary_press() => {
                    state.pressed = hit
                        .hits
                        .iter()
                        .rev()
                        .find(|(data, _)| data.draggable)
                        .map(|(data, _)| (data.id.clone(), point));
                }
                Event::Pointer(pointer::Event::PointerMoved { .. }) => {
                    if let Some((source, origin)) = &state.pressed
                        && (state.dragging || point.distance(*origin) > 8.0)
                    {
                        state.dragging = true;
                        state.target = hit
                            .hits
                            .into_iter()
                            .rev()
                            .find(|(data, _)| (self.accept)(source, &data.id))
                            .map(|(data, bounds)| (data.id, data.mode.zone(bounds, point), bounds));
                        shell.capture_event();
                        shell.request_redraw();
                        return;
                    }
                }
                Event::Pointer(e) if e.is_primary_release() => {
                    let dragging = state.dragging;
                    if dragging
                        && let Some((source, _)) = state.pressed.take()
                        && let Some((data, bounds)) = hit
                            .hits
                            .into_iter()
                            .rev()
                            .find(|(data, _)| (self.accept)(&source, &data.id))
                    {
                        shell.publish((self.on_drop)(
                            source,
                            data.id,
                            data.mode.zone(bounds, point),
                        ));
                    }
                    *state = State::default();
                    if dragging {
                        self.content.as_widget_mut().update(
                            &mut tree.children[0],
                            event,
                            layout,
                            mouse::Cursor::Unavailable,
                            renderer,
                            shell,
                            viewport,
                        );
                        shell.capture_event();
                        shell.request_redraw();
                        return;
                    }
                }
                _ => {}
            }
        }

        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
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
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );

        if let Some((_, zone, bounds)) = &tree.state.downcast_ref::<State<T>>().target {
            let color = theme.palette().primary.base.color;
            let bounds = match zone {
                DropZone::Into => *bounds,
                DropZone::Before => Rectangle {
                    height: 3.0,
                    ..*bounds
                },
                DropZone::After => Rectangle {
                    y: bounds.y + bounds.height - 3.0,
                    height: 3.0,
                    ..*bounds
                },
            };
            renderer.with_layer(*viewport, |renderer| {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds,
                        border: Border {
                            color,
                            width: 2.0,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    color.scale_alpha(0.2),
                )
            });
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
        if tree.state.downcast_ref::<State<T>>().dragging {
            mouse::Interaction::Grabbing
        } else {
            self.content.as_widget().mouse_interaction(
                &tree.children[0],
                layout,
                cursor,
                viewport,
                renderer,
            )
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
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, T: Clone + Send + 'static, Message: 'a> From<DragDrop<'a, T, Message>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(value: DragDrop<'a, T, Message>) -> Self {
        Element::new(value)
    }
}

#[cfg(test)]
mod tests {
    use crate::drag_drop::*;

    #[test]
    fn drop_zones_distinguish_insertion_from_parenting() {
        let bounds = Rectangle {
            x: 0.0,
            y: 20.0,
            width: 100.0,
            height: 40.0,
        };
        for (y, zone) in [
            (22.0, DropZone::Before),
            (40.0, DropZone::Into),
            (58.0, DropZone::After),
        ] {
            assert_eq!(DropMode::Tree.zone(bounds, Point::new(5.0, y)), zone);
        }
        assert_eq!(
            DropMode::Insert.zone(bounds, Point::new(5.0, 30.0)),
            DropZone::Before
        );
        assert_eq!(
            DropMode::Insert.zone(bounds, Point::new(5.0, 50.0)),
            DropZone::After
        );
        assert_eq!(
            DropMode::Into.zone(bounds, Point::new(5.0, 22.0)),
            DropZone::Into
        );
    }
}
