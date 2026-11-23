use bevy_math::Rect;
use glam::Vec2;
use iced_core::{
    Element, Event, Layout, Length, Point, Rectangle, Shell, Size, Widget,
    layout::{self, Limits},
    pointer::{self, mouse},
    renderer,
    widget::{Tree, tree},
    window,
};
use iced_wgpu::primitive::Renderer as _;
use lapiz_image::{texel::TexelType, tile::GpuTileStorage};
use moxcms::ColorProfile;

use crate::{Canvas, render::CanvasPrimitive};

/// Interactive UI widget that renders a [`CCanvas`] via [`CanvasPrimitive`] and routes pointer and focus events.
pub struct CanvasWidget<'a, Message> {
    pub is_focusing: bool,
    pub canvas: &'a Canvas,
    pub tile_storage: GpuTileStorage,
    pub on_focus: Box<dyn Fn(Point) -> Message + 'a>,
    pub on_pointer_event: Box<dyn Fn(pointer::Event) -> Message + 'a>,
    pub on_widget_rect_change: Box<dyn Fn(Rect) -> Message>,
    pub color_profile: ColorProfile,
    pub window_id: u64,
    pub monitor_name: String,
}

#[derive(Default)]
struct CanvasState {
    pressed: Option<(Point, pointer::button::Source)>,
}

impl CanvasState {
    fn route(
        &mut self,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<pointer::Event> {
        if matches!(event, Event::Window(window::Event::Unfocused)) {
            return self
                .pressed
                .take()
                .map(|(position, button)| pointer::Event::PointerReleased { position, button });
        }

        let Event::Pointer(event) = event else {
            return None;
        };
        let is_over = match event {
            pointer::Event::PointerEntered { position, .. }
            | pointer::Event::PointerMoved { position, .. }
            | pointer::Event::PointerPressed { position, .. }
            | pointer::Event::PointerReleased { position, .. } => bounds.contains(*position),
            pointer::Event::PointerLeft { .. } | pointer::Event::WheelScrolled { .. } => {
                cursor.land().is_over(bounds)
            }
        };
        if !is_over && self.pressed.is_none() {
            return None;
        }

        match event {
            pointer::Event::PointerPressed { position, button } if event.is_primary_press() => {
                self.pressed = Some((*position, button.clone()));
            }
            pointer::Event::PointerMoved { position, .. } => {
                if let Some((last_position, _)) = &mut self.pressed {
                    *last_position = *position;
                }
            }
            pointer::Event::PointerReleased { .. } if event.is_primary_release() => {
                self.pressed = None;
            }
            _ => {}
        }

        Some(event.clone())
    }
}

impl<Message, Theme> Widget<Message, Theme, lapiz_runtime::Renderer> for CanvasWidget<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<CanvasState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(CanvasState::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _: &mut Tree,
        _: &lapiz_runtime::Renderer,
        limits: &Limits,
    ) -> layout::Node {
        layout::atomic(limits, Length::Fill, Length::Fill)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _: &lapiz_runtime::Renderer,
        shell: &mut Shell<'_, Message>,
        _: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let widget_rect = Rect {
            min: Vec2::new(bounds.x, bounds.y),
            max: Vec2::new(bounds.x + bounds.width, bounds.y + bounds.height),
        };
        if widget_rect != self.canvas.transform.widget_bounds {
            shell.publish((self.on_widget_rect_change)(widget_rect));
        }

        let state = tree.state.downcast_mut::<CanvasState>();
        if state.pressed.is_none() && shell.is_event_captured() {
            return;
        }
        let is_pointer = matches!(event, Event::Pointer(_));
        let Some(event) = state.route(event, bounds, cursor) else {
            return;
        };

        if self.is_focusing {
            shell.publish((self.on_pointer_event)(event.clone()));
            if is_pointer {
                shell.capture_event();
            }
        } else if let pointer::Event::PointerPressed { position, .. } = &event
            && event.is_primary_press()
        {
            shell.publish((self.on_focus)(*position));
            shell.publish((self.on_pointer_event)(event.clone()));
            shell.capture_event();
        }
    }

    fn draw(
        &self,
        _: &Tree,
        renderer: &mut lapiz_runtime::Renderer,
        _: &Theme,
        _: &renderer::Style,
        layout: Layout<'_>,
        _: mouse::Cursor,
        _: &Rectangle,
    ) {
        renderer.draw_primitive(
            layout.bounds(),
            CanvasPrimitive {
                canvas_id: self.canvas.id(),
                image_size: self.canvas.image.size(),
                root_layer: *self.canvas.image.layer_stack().root_id(),
                selection_layer: self.canvas.image.selection_layer(),
                root_texel_type: self.canvas.image.texel_type(),
                selection_texel_type: TexelType::A8,
                transform: self.canvas.transform.clone(),
                tile_storage: self.tile_storage.clone(),
                color_profile: self.color_profile.clone(),
                window_id: self.window_id,
                monitor_name: self.monitor_name.clone(),
            },
        );
    }
}

impl<'a, Message, Theme> From<CanvasWidget<'a, Message>>
    for Element<'a, Message, Theme, lapiz_runtime::Renderer>
where
    Message: 'a,
{
    fn from(canvas: CanvasWidget<'a, Message>) -> Self {
        Element::new(canvas)
    }
}
