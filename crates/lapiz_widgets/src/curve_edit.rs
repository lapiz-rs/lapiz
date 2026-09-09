use glam::Vec2;
use iced_core::{
    Background, Border, Color, Element, Event, Layout, Length, Point, Rectangle, Renderer as _,
    Shell, Size, Theme, Widget,
    keyboard::{self, key},
    layout, pointer,
    pointer::mouse::{self, Cursor},
    renderer::{self, Quad},
    widget::{self, tree},
};
use iced_graphics::geometry::{Frame, Path, Renderer as _, Stroke};
use lapiz_math::curve::CubicCurve;
use lapiz_runtime::Renderer;

use crate::callback::{CallbackWith, publish_with};

const MIN_POINT_GAP: f32 = 0.001;

pub struct CurveEdit<'a, Message> {
    curve: CubicCurve,
    change: CallbackWith<'a, CubicCurve, Message>,
    release: CallbackWith<'a, CubicCurve, Message>,
    width: Length,
    height: Length,
    grid_resolution: usize,
    curve_resolution: usize,
    control_point_radius: f32,
    curve_stroke_width: f32,
    grid_stroke_width: f32,
    control_point_stroke_width: f32,
    class: <Theme as Catalog>::Class<'a>,
}

impl<'a, Message> CurveEdit<'a, Message> {
    pub fn new(curve: CubicCurve) -> Self {
        Self {
            curve,
            change: None,
            release: None,
            width: Length::Fill,
            height: Length::Fill,
            grid_resolution: 4,
            curve_resolution: 128,
            control_point_radius: 4.0,
            curve_stroke_width: 1.5,
            grid_stroke_width: 0.5,
            control_point_stroke_width: 1.0,
            class: <Theme as Catalog>::default(),
        }
    }

    crate::callback_methods!(change, CubicCurve);
    crate::callback_methods!(release, CubicCurve);

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    #[must_use]
    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self {
        self.class = Box::new(style);
        self
    }

    #[must_use]
    pub fn class(mut self, class: impl Into<<Theme as Catalog>::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }

    pub fn grid_resolution(mut self, resolution: usize) -> Self {
        self.grid_resolution = resolution;
        self
    }

    pub fn curve_resolution(mut self, resolution: usize) -> Self {
        self.curve_resolution = resolution;
        self
    }

    pub fn control_point_radius(mut self, radius: f32) -> Self {
        self.control_point_radius = radius;
        self
    }

    pub fn curve_stroke_width(mut self, width: f32) -> Self {
        self.curve_stroke_width = width;
        self
    }

    pub fn grid_stroke_width(mut self, width: f32) -> Self {
        self.grid_stroke_width = width;
        self
    }
}

impl<Message> Widget<Message, Theme, Renderer> for CurveEdit<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<CurveEditState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(CurveEditState::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(
        &mut self,
        _tree: &mut widget::Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, self.width, self.height)
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: Cursor,
        _renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<CurveEditState>();
        let bounds = layout.bounds();

        match event {
            Event::Pointer(event) if event.is_primary_press() => {
                let Some(position) = cursor.position_in(bounds) else {
                    return;
                };

                let normalized = local_to_normalized(position, bounds.size());
                let closest = self
                    .curve
                    .control_points()
                    .iter()
                    .enumerate()
                    .filter_map(|(index, point)| {
                        let screen = normalized_to_local(*point, bounds.size());
                        let distance = (screen - Vec2::new(position.x, position.y)).length();
                        (distance <= self.control_point_radius * 2.0).then_some((index, distance))
                    })
                    .min_by(|left, right| left.1.total_cmp(&right.1))
                    .map(|(index, _)| index);

                if let Some(index) = closest {
                    state.selected_index = Some(index);
                    state.dragging = true;
                    state.drag_curve = None;
                } else if self.change.is_some() {
                    let mut points = self.curve.control_points().to_vec();
                    let index = points.partition_point(|point| point.x < normalized.x);
                    let min = index
                        .checked_sub(1)
                        .map_or(0.0, |previous| points[previous].x + MIN_POINT_GAP);
                    let max = points.get(index).map_or(1.0, |next| next.x - MIN_POINT_GAP);
                    points.insert(index, Vec2::new(normalized.x.clamp(min, max), normalized.y));
                    let curve = CubicCurve::new(points);
                    state.selected_index = Some(index);
                    state.dragging = true;
                    state.drag_curve = Some(curve.clone());
                    if let Some(message) = publish_with(&mut self.change, curve) {
                        shell.publish(message);
                    }
                }

                shell.request_redraw();
                shell.capture_event();
            }
            Event::Pointer(pointer::Event::PointerMoved { position, .. }) if state.dragging => {
                let index = state
                    .selected_index
                    .expect("dragging without a selected point");
                let mut points = self.curve.control_points().to_vec();

                let normalized = screen_to_normalized(*position, bounds);
                let min = index
                    .checked_sub(1)
                    .map_or(0.0, |previous| points[previous].x + MIN_POINT_GAP);
                let max = points
                    .get(index + 1)
                    .map_or(1.0, |next| next.x - MIN_POINT_GAP);
                points[index] = Vec2::new(normalized.x.clamp(min, max), normalized.y);

                let curve = CubicCurve::new(points);
                state.drag_curve = Some(curve.clone());
                if let Some(message) = publish_with(&mut self.change, curve) {
                    shell.publish(message);
                }
                shell.capture_event();
            }
            Event::Pointer(e @ pointer::Event::PointerReleased { .. })
                if e.is_primary_release() && state.dragging =>
            {
                state.dragging = false;
                let curve = state
                    .drag_curve
                    .take()
                    .unwrap_or_else(|| self.curve.clone());
                if let Some(message) = publish_with(&mut self.release, curve) {
                    shell.publish(message);
                }
                shell.capture_event();
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(key::Named::Delete),
                ..
            }) if cursor.is_over(bounds) => {
                let Some(index) = state.selected_index else {
                    return;
                };
                if self.curve.control_points().len() <= 2 {
                    return;
                }

                let mut points = self.curve.control_points().to_vec();
                points.remove(index);
                state.selected_index = None;
                state.dragging = false;
                state.drag_curve = None;
                if let Some(message) = publish_with(&mut self.change, CubicCurve::new(points)) {
                    shell.publish(message);
                }
                shell.capture_event();
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _renderer_style: &renderer::Style,
        layout: Layout<'_>,
        cursor: Cursor,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let state = tree.state.downcast_ref::<CurveEditState>();
        let status = if state.dragging {
            Status::Dragged
        } else if cursor.is_over(bounds) {
            Status::Hovered
        } else {
            Status::Active
        };
        let style = theme.style(&self.class, status);

        renderer.fill_quad(
            Quad {
                bounds,
                border: style.border,
                ..Default::default()
            },
            style.background,
        );

        let mut frame = Frame::new(renderer, bounds.size());

        for index in 1..self.grid_resolution {
            let fraction = index as f32 / self.grid_resolution as f32;
            let x = fraction * bounds.width;
            let y = fraction * bounds.height;
            let vertical = Path::new(|builder| {
                builder.move_to(Point::new(x, 0.0));
                builder.line_to(Point::new(x, bounds.height));
            });
            let horizontal = Path::new(|builder| {
                builder.move_to(Point::new(0.0, y));
                builder.line_to(Point::new(bounds.width, y));
            });
            let stroke = Stroke {
                style: style.grid.into(),
                width: self.grid_stroke_width,
                ..Default::default()
            };
            frame.stroke(&vertical, stroke);
            frame.stroke(&horizontal, stroke);
        }

        let sampled = self.curve.subdivide(self.curve_resolution);
        let curve = Path::new(|builder| {
            let first = normalized_to_local(sampled[0], bounds.size());
            builder.move_to(Point::new(first.x, first.y));
            for point in &sampled[1..] {
                let point = normalized_to_local(*point, bounds.size());
                builder.line_to(Point::new(point.x, point.y));
            }
        });
        frame.stroke(
            &curve,
            Stroke {
                style: style.curve.into(),
                width: self.curve_stroke_width,
                ..Default::default()
            },
        );

        for (index, point) in self.curve.control_points().iter().enumerate() {
            let center = normalized_to_local(*point, bounds.size());
            let radius = self.control_point_radius;
            let origin = Point::new(center.x - radius, center.y - radius);
            let point_size = Size::new(radius * 2.0, radius * 2.0);
            if state.selected_index == Some(index) {
                frame.fill_rectangle(origin, point_size, style.selected_control_point);
            } else {
                frame.stroke_rectangle(
                    origin,
                    point_size,
                    Stroke {
                        style: style.control_point.into(),
                        width: self.control_point_stroke_width,
                        ..Default::default()
                    },
                );
            }
        }

        renderer.with_translation(iced_core::Vector::new(bounds.x, bounds.y), |renderer| {
            renderer.draw_geometry(frame.into_geometry())
        });
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if tree.state.downcast_ref::<CurveEditState>().dragging {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

impl<'a, Message: 'a> From<CurveEdit<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(widget: CurveEdit<'a, Message>) -> Self {
        Element::new(widget)
    }
}

#[derive(Default)]
struct CurveEditState {
    selected_index: Option<usize>,
    dragging: bool,
    drag_curve: Option<CubicCurve>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Active,
    Hovered,
    Dragged,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub background: Background,
    pub border: Border,
    pub grid: Color,
    pub curve: Color,
    pub control_point: Color,
    pub selected_control_point: Color,
}

pub trait Catalog: Sized {
    type Class<'a>;

    fn default<'a>() -> Self::Class<'a>;

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style;
}

pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

impl Catalog for iced_core::Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(default)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

pub fn default(theme: &iced_core::Theme, _status: Status) -> Style {
    let palette = theme.palette();
    Style {
        background: palette.background.base.color.into(),
        border: Border {
            color: palette.background.strong.color,
            width: 1.0,
            ..Default::default()
        },
        grid: palette.background.strong.color,
        curve: palette.primary.base.color,
        control_point: palette.primary.base.color,
        selected_control_point: palette.primary.strong.color,
    }
}

fn local_to_normalized(local: Point, size: Size) -> Vec2 {
    Vec2::new(
        (local.x / size.width).clamp(0.0, 1.0),
        (1.0 - local.y / size.height).clamp(0.0, 1.0),
    )
}

fn screen_to_normalized(screen: Point, bounds: Rectangle) -> Vec2 {
    local_to_normalized(
        Point::new(screen.x - bounds.x, screen.y - bounds.y),
        bounds.size(),
    )
}

fn normalized_to_local(point: Vec2, size: Size) -> Vec2 {
    Vec2::new(point.x * size.width, (1.0 - point.y) * size.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_between_local_and_normalized_coordinates() {
        let size = Size::new(200.0, 100.0);
        let normalized = local_to_normalized(Point::new(50.0, 75.0), size);
        assert_eq!(normalized, Vec2::new(0.25, 0.25));
        assert_eq!(normalized_to_local(normalized, size), Vec2::new(50.0, 75.0));
    }

    #[test]
    fn converts_screen_coordinates_relative_to_bounds() {
        let bounds = Rectangle::new(Point::new(100.0, 50.0), Size::new(200.0, 100.0));
        assert_eq!(
            screen_to_normalized(Point::new(150.0, 125.0), bounds),
            Vec2::new(0.25, 0.25)
        );
    }
}
