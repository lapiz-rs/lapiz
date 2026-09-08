use glam::Vec2;
use iced_runtime::Task;
use lapiz_input::{key::KeyboardState, mouse::PressedMouseState};
use lapiz_math::number::AngleDifference;
use lapiz_runtime::Services;
use lapiz_tools::{ToolFunction, ToolId};
use lapiz_widgets::icon;

use crate::{CanvasAppExt, control::CanvasTransform};

#[derive(Default)]
pub struct PanTool {
    start_pos: Vec2,
    original_transform: CanvasTransform,
}

impl ToolFunction for PanTool {
    type Message = ();

    fn id() -> ToolId {
        ToolId::new("pan_tool".into())
    }

    fn icon() -> icon::Icon<'static> {
        icon::hand()
    }

    fn begin(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let Some(canvas) = services.current_canvas() else {
            return Task::none();
        };

        self.start_pos = Vec2::new(mouse.position.x, mouse.position.y);
        self.original_transform = canvas.transform.clone();
        Task::none()
    }

    fn update(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let Some(canvas) = services.current_canvas_mut() else {
            return Task::none();
        };

        let delta = Vec2::new(mouse.position.x, mouse.position.y) - self.start_pos;
        canvas.transform = self.original_transform.clone().translated(delta);

        Task::none()
    }
}

#[derive(Default)]
pub struct RotateTool {
    center: Vec2,
    initial_angle: f32,
    original_transform: CanvasTransform,
}

impl ToolFunction for RotateTool {
    type Message = ();

    fn id() -> ToolId {
        ToolId::new("rotate_tool".into())
    }

    fn icon() -> icon::Icon<'static> {
        icon::refresh()
    }

    fn begin(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let Some(canvas) = services.current_canvas() else {
            return Task::none();
        };

        self.center = canvas.transform.widget_bounds.size() * 0.5;
        let offset = self.center - Vec2::new(mouse.position.x, mouse.position.y);
        self.initial_angle = offset.y.atan2(offset.x);
        self.original_transform = canvas.transform.clone();
        Task::none()
    }

    fn update(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let Some(canvas) = services.current_canvas_mut() else {
            return Task::none();
        };

        let offset = self.center - Vec2::new(mouse.position.x, mouse.position.y);
        let current_angle = offset.y.atan2(offset.x);
        canvas.transform = self.original_transform.clone().rotated_around(
            current_angle.angle_difference(self.initial_angle),
            self.center,
        );

        Task::none()
    }
}

#[derive(Default)]
pub struct ZoomTool {
    start_pos: Vec2,
    original_transform: CanvasTransform,
}

impl ToolFunction for ZoomTool {
    type Message = ();

    fn id() -> ToolId {
        ToolId::new("zoom_tool".into())
    }

    fn icon() -> icon::Icon<'static> {
        icon::zoom()
    }

    fn begin(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let Some(canvas) = services.current_canvas() else {
            return Task::none();
        };

        self.start_pos = Vec2::new(mouse.position.x, mouse.position.y);
        self.original_transform = canvas.transform.clone();
        Task::none()
    }

    fn update(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let Some(canvas) = services.current_canvas_mut() else {
            return Task::none();
        };

        let position = Vec2::new(mouse.position.x, mouse.position.y);
        let delta = position.y - self.start_pos.y;
        let factor = -delta / self.original_transform.widget_bounds.size().y + 1.0;
        canvas.transform = self
            .original_transform
            .clone()
            .scaled_around(factor, self.start_pos);

        Task::none()
    }
}
