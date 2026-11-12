use bevy_math::Rect;
use glam::{Mat3, Vec2};

/// Affine transformation and viewport bounds mapping between image pixel coordinates, widget coordinates, and window coordinates.
#[derive(Default, Debug, Clone)]
pub struct CanvasTransform {
    pub widget_bounds: Rect,
    pub pixel_to_widget: Mat3,
}

impl CanvasTransform {
    pub fn translate(&mut self, delta: Vec2) {
        let translation = Mat3::from_translation(delta);
        self.pixel_to_widget = translation * self.pixel_to_widget;
    }

    pub fn rotate_around(&mut self, angle: f32, center_ws: Vec2) {
        let new_mat = Mat3::from_translation(center_ws)
            * Mat3::from_angle(angle)
            * Mat3::from_translation(-center_ws)
            * self.pixel_to_widget;
        self.pixel_to_widget = new_mat;
    }

    pub fn scale_around(&mut self, scale_factor: f32, center_ws: Vec2) {
        let new_mat = Mat3::from_translation(center_ws)
            * Mat3::from_scale(Vec2::splat(scale_factor))
            * Mat3::from_translation(-center_ws)
            * self.pixel_to_widget;
        self.pixel_to_widget = new_mat;
    }

    pub fn translated(mut self, delta: Vec2) -> Self {
        self.translate(delta);
        self
    }

    pub fn rotated_around(mut self, angle: f32, center_ws: Vec2) -> Self {
        self.rotate_around(angle, center_ws);
        self
    }

    pub fn scaled_around(mut self, scale_factor: f32, center_ws: Vec2) -> Self {
        self.scale_around(scale_factor, center_ws);
        self
    }

    pub fn pixel_to_window(&self, point: Vec2) -> Vec2 {
        let widget = self.pixel_to_widget.transform_point2(point);
        widget + self.widget_bounds.min
    }

    pub fn window_to_widget(&self, point: Vec2) -> Vec2 {
        point - self.widget_bounds.min
    }

    pub fn window_to_in_widget(&self, point: Vec2) -> Option<Vec2> {
        if self.widget_bounds.contains(point) {
            Some(point - self.widget_bounds.min)
        } else {
            None
        }
    }

    pub fn window_to_pixel(&self, point: Vec2) -> Vec2 {
        let widget = self.window_to_widget(point);
        self.pixel_to_widget.inverse().transform_point2(widget)
    }

    pub fn window_to_in_pixel(&self, point: Vec2) -> Option<Vec2> {
        let widget = self.window_to_in_widget(point)?;
        Some(self.pixel_to_widget.inverse().transform_point2(widget))
    }

    pub fn zoom(&self) -> f32 {
        self.pixel_to_widget.x_axis.truncate().length()
    }

    pub fn rotation(&self) -> f32 {
        self.pixel_to_widget
            .x_axis
            .y
            .atan2(self.pixel_to_widget.x_axis.x)
    }
}
