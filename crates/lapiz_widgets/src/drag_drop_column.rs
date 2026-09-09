use iced_core::{
    Alignment, Element, Event, Layout, Length, Point, Rectangle, Shell, Size,
    layout::{self, Limits, flex},
    pointer,
    pointer::mouse,
    renderer,
    widget::{Tree, Widget, tree},
};

const DRAG_DEADBAND: f32 = 10.0;

pub struct DragDropInfo<'a> {
    pub dragged_child_bounds: Rectangle,
    pub dragged_index: usize,
    pub column_layout: &'a layout::Layout<'a>,
    pub mouse_position: Point,
}

pub struct DragDropColumn<'a, Message, Theme = iced_core::Theme, Renderer = lapiz_runtime::Renderer>
{
    children: Vec<Element<'a, Message, Theme, Renderer>>,
    spacing: f32,
    width: Length,
    height: Length,
    on_press: Option<Box<dyn for<'b> Fn(DragDropInfo<'b>) -> Message + 'a>>,
    on_drag: Option<Box<dyn for<'b> Fn(DragDropInfo<'b>) -> Message + 'a>>,
    on_drop: Option<Box<dyn for<'b> Fn(DragDropInfo<'b>) -> Message + 'a>>,
}

impl<'a, Message, Theme, Renderer> DragDropColumn<'a, Message, Theme, Renderer>
where
    Renderer: iced_core::Renderer,
{
    pub fn new(children: impl Into<Vec<Element<'a, Message, Theme, Renderer>>>) -> Self {
        Self {
            children: children.into(),
            spacing: 0.0,
            width: Length::Fill,
            height: Length::Fill,
            on_press: None,
            on_drag: None,
            on_drop: None,
        }
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn on_press(mut self, f: impl for<'x> Fn(DragDropInfo<'x>) -> Message + 'a) -> Self {
        self.on_press = Some(Box::new(f));
        self
    }

    pub fn on_drag(mut self, f: impl for<'x> Fn(DragDropInfo<'x>) -> Message + 'a) -> Self {
        self.on_drag = Some(Box::new(f));
        self
    }

    pub fn on_drop(mut self, f: impl for<'x> Fn(DragDropInfo<'x>) -> Message + 'a) -> Self {
        self.on_drop = Some(Box::new(f));
        self
    }
}

#[derive(Debug, Default)]
struct State {
    action: Action,
    index: usize,
    pointer: Point,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum Action {
    #[default]
    Idle,
    Pressing {
        origin: Point,
    },
    Dragging,
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for DragDropColumn<'a, Message, Theme, Renderer>
where
    Renderer: iced_core::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut self.children);
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &Limits) -> layout::Node {
        layout::flex::resolve(
            flex::Axis::Vertical,
            renderer,
            limits,
            self.width,
            self.height,
            iced_core::Padding::ZERO,
            self.spacing,
            Alignment::Start,
            &mut self.children,
            &mut tree.children,
        )
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
        for ((i, child), child_node) in self.children.iter_mut().enumerate().zip(layout.children())
        {
            child.as_widget_mut().update(
                &mut tree.children[i],
                event,
                child_node,
                cursor,
                renderer,
                shell,
                viewport,
            );
            if shell.is_event_captured() {
                return;
            }
        }

        let state = tree.state.downcast_mut::<State>();
        let position = match event {
            Event::Pointer(pointer::Event::PointerMoved { position, .. }) => *position,
            _ => match cursor.land().position() {
                Some(position) => position,
                None if matches!(
                    event,
                    Event::Pointer(pointer::Event::PointerReleased { .. })
                ) && state.action != Action::Idle =>
                {
                    state.pointer
                }
                None => return,
            },
        };
        state.pointer = position;

        match event {
            Event::Pointer(event) if event.is_primary_press() => {
                let Some(index) = layout
                    .children()
                    .enumerate()
                    .find(|(_, child)| child.bounds().contains(position))
                    .map(|(index, _)| index)
                else {
                    return;
                };
                if let Some(on_press) = self.on_press.as_ref()
                    && let Some(info) = make_info(&layout, position, index)
                {
                    shell.publish(on_press(info));
                }
                state.index = index;
                state.action = Action::Pressing { origin: position };
                shell.capture_event();
            }
            Event::Pointer(pointer::Event::PointerMoved { .. }) => match state.action {
                Action::Pressing { origin } if position.distance(origin) > DRAG_DEADBAND => {
                    state.action = Action::Dragging;
                    shell.capture_event();
                    shell.request_redraw();
                    if let Some(on_drag) = self.on_drag.as_ref()
                        && let Some(info) = make_info(&layout, position, state.index)
                    {
                        shell.publish(on_drag(info));
                    }
                }
                Action::Dragging => {
                    shell.capture_event();
                    shell.request_redraw();
                    if let Some(on_drag) = self.on_drag.as_ref()
                        && let Some(info) = make_info(&layout, position, state.index)
                    {
                        shell.publish(on_drag(info));
                    }
                }
                _ => {}
            },
            Event::Pointer(e @ pointer::Event::PointerReleased { .. })
                if e.is_primary_release() =>
            {
                match state.action {
                    Action::Dragging => {
                        shell.capture_event();
                        shell.request_redraw();
                        if let Some(on_drop) = self.on_drop.as_ref()
                            && let Some(info) = make_info(&layout, position, state.index)
                        {
                            shell.publish(on_drop(info));
                        }
                    }
                    Action::Pressing { .. } => {
                        shell.capture_event();
                    }
                    Action::Idle => {}
                }
                state.action = Action::Idle;
            }
            _ => {}
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
        for (i, child) in self.children.iter().enumerate() {
            child.as_widget().draw(
                &tree.children[i],
                renderer,
                theme,
                style,
                layout.children().nth(i).unwrap(),
                cursor,
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
        let state = tree.state.downcast_ref::<State>();
        if state.action == Action::Dragging {
            return mouse::Interaction::Grabbing;
        }
        if cursor.is_over(layout.bounds()) {
            return mouse::Interaction::Pointer;
        }
        mouse::Interaction::default()
    }
}

fn make_info<'a>(
    column_layout: &'a layout::Layout<'a>,
    mouse_position: Point,
    dragged_index: usize,
) -> Option<DragDropInfo<'a>> {
    Some(DragDropInfo {
        dragged_child_bounds: column_layout.child(dragged_index).bounds(),
        mouse_position,
        dragged_index,
        column_layout,
    })
}

impl<'a, Message: 'a, Theme: 'a, Renderer> From<DragDropColumn<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Renderer: iced_core::Renderer + 'a,
{
    fn from(widget: DragDropColumn<'a, Message, Theme, Renderer>) -> Self {
        Element::new(widget)
    }
}
