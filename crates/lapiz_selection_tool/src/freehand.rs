use std::borrow::Cow;

use bevy_math::Rect;
use glam::Vec2;
use iced_core::{Element, Length, Theme};
use iced_runtime::Task;
use lapiz_canvas::{CanvasAppExt as _, CanvasUndoStackAppExt as _};
use lapiz_i18n::t;
use lapiz_input::{
    key::KeyboardState,
    mouse::{HoverMouseState, PressedMouseState},
};
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_tools::{ToolFunction, ToolId};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{form, icon, label, panel, segmented_control, space};
use lyon::tessellation::FillRule;
use tracing::info;

use crate::render::{
    SelectionOperation, SelectionPreviewLayer, generate_cmd, indices_from_vertices,
};

/// In-progress stroke state for freehand and polygonal lasso selection tools.
struct FreehandSelectionState {
    aabb: Rect,
    points_ps: Vec<Vec2>,
    op: SelectionOperation,
}

/// UI messages for [`FreehandSelectionTool`].
#[derive(Debug, Clone)]
pub enum FreehandSelectionToolMessage {
    FillRuleChanged(FillRule),
}

/// Lasso selection tool that traces a freehand closed path while dragging the pointer.
///
/// `FreehandSelectionTool` implements [`ToolFunction`] to record pixel-space vertices during a drag
/// gesture, render an animated marching-ants preview polyline via [`SelectionPreviewLayer`], and
/// tessellate the closed contour using the configured [`FillRule`] upon release to update the
/// canvas selection mask.
pub struct FreehandSelectionTool {
    fill_rule: FillRule,
    state: Option<FreehandSelectionState>,
}

impl Default for FreehandSelectionTool {
    fn default() -> Self {
        Self {
            fill_rule: FillRule::EvenOdd,
            state: Default::default(),
        }
    }
}

impl ToolFunction for FreehandSelectionTool {
    type Message = FreehandSelectionToolMessage;

    fn id() -> ToolId {
        ToolId::new("freehand_selection_tool".into())
    }

    fn icon() -> icon::Icon<'static> {
        icon::lasso()
    }

    fn begin(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<Self::Message> {
        let Some(canvas) = globals.current_canvas() else {
            return Task::none();
        };

        let point_ps = canvas
            .transform
            .window_to_pixel(Vec2::new(mouse.position.x, mouse.position.y));

        self.state = Some(FreehandSelectionState {
            aabb: Rect {
                min: point_ps,
                max: point_ps,
            },
            points_ps: vec![point_ps],
            op: SelectionOperation::from_modifiers(keyboard.modifiers()),
        });

        Task::none()
    }

    fn update(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<Self::Message> {
        let Some(canvas) = globals.current_canvas() else {
            return Task::none();
        };

        let point_ps = canvas
            .transform
            .window_to_pixel(Vec2::new(mouse.position.x, mouse.position.y));

        let Some(state) = self.state.as_mut() else {
            return Task::none();
        };

        state.points_ps.push(point_ps);
        state.aabb = state.aabb.union_point(point_ps);

        Task::none()
    }

    fn end(
        &mut self,
        _: &KeyboardState,
        _: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<Self::Message> {
        let Some(state) = self.state.take() else {
            return Task::none();
        };

        if state.points_ps.len() < 3 {
            return Task::none();
        }

        let geometry = indices_from_vertices(&state.points_ps, self.fill_rule);
        let cmd = generate_cmd(
            "Freehand Selection".into(),
            &geometry.vertices,
            &geometry.indices,
            state.aabb.as_irect(),
            state.op,
            globals,
        );

        if let Some(cmd) = cmd {
            globals.push_undo_command_to_current(cmd).log_err();
            info!(
                "Freehand select {} points aabb {:?}",
                state.points_ps.len(),
                state.aabb
            );
        }

        Task::none()
    }

    fn handle_message(&mut self, message: Self::Message, _: &mut Globals) -> Task<Self::Message> {
        match message {
            FreehandSelectionToolMessage::FillRuleChanged(fill_rule) => self.fill_rule = fill_rule,
        }

        Task::none()
    }

    fn tool_option_widget<'a>(
        &'a self,
        _: &'a Globals,
    ) -> Option<Element<'a, Self::Message, Theme, Renderer>> {
        let fields = form().push(
            t!("fill_rule"),
            segmented_control()
                .push(
                    label(t!("even_odd")),
                    self.fill_rule == FillRule::EvenOdd,
                    FreehandSelectionToolMessage::FillRuleChanged(FillRule::EvenOdd),
                )
                .push(
                    label(t!("non_zero")),
                    self.fill_rule == FillRule::NonZero,
                    FreehandSelectionToolMessage::FillRuleChanged(FillRule::NonZero),
                ),
        );

        Some(
            panel(fields)
                .transparent()
                .padding(2)
                .width(Length::Fill)
                .into(),
        )
    }

    fn canvas_overlay<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        let Some(state) = &self.state else {
            return space().into();
        };

        let Some(canvas) = globals.current_canvas() else {
            return space().into();
        };

        SelectionPreviewLayer {
            line_vertices_ps: Cow::Borrowed(&state.points_ps),
            canvas_transform: &canvas.transform,
        }
        .into()
    }
}

const POLYGON_CLOSE_DISTANCE: f32 = 10.0;

/// UI messages for [`PolygonSelectionTool`].
#[derive(Debug, Clone)]
pub enum PolygonSelectionToolMessage {
    FillRuleChanged(FillRule),
}

/// Polygonal lasso selection tool that constructs a multi-vertex selection polygon across clicks.
///
/// `PolygonSelectionTool` implements [`ToolFunction`] to append vertices on each click release,
/// previewing the active segment to the hovering cursor via [`SelectionPreviewLayer`]. Clicking
/// within close proximity of the initial vertex closes and tessellates the polygon using the
/// configured [`FillRule`] and commits the resulting selection mask command.
pub struct PolygonSelectionTool {
    fill_rule: FillRule,
    state: Option<FreehandSelectionState>,
    cursor_ps: Vec2,
}

impl Default for PolygonSelectionTool {
    fn default() -> Self {
        Self {
            fill_rule: FillRule::EvenOdd,
            state: Default::default(),
            cursor_ps: Default::default(),
        }
    }
}

impl ToolFunction for PolygonSelectionTool {
    type Message = PolygonSelectionToolMessage;

    fn id() -> ToolId {
        ToolId::new("polygon_selection_tool".into())
    }

    fn icon() -> icon::Icon<'static> {
        icon::poly_lasso()
    }

    fn begin(
        &mut self,
        keyboard: &KeyboardState,
        _: &PressedMouseState,
        _: &mut Globals,
    ) -> Task<Self::Message> {
        if self.state.is_none() {
            self.state = Some(FreehandSelectionState {
                aabb: Rect::EMPTY,
                points_ps: Vec::new(),
                op: SelectionOperation::from_modifiers(keyboard.modifiers()),
            });
        }

        Task::none()
    }

    fn hover(
        &mut self,
        _: &KeyboardState,
        mouse: &HoverMouseState,
        globals: &mut Globals,
    ) -> Task<Self::Message> {
        let Some(canvas) = globals.current_canvas() else {
            return Task::none();
        };

        let point_ss = Vec2::new(mouse.position.x, mouse.position.y);
        self.cursor_ps = canvas.transform.window_to_pixel(point_ss);

        Task::none()
    }

    fn end(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<Self::Message> {
        let Some(canvas) = globals.current_canvas() else {
            return Task::none();
        };

        let point_ss = Vec2::new(mouse.position.x, mouse.position.y);
        let point_ps = canvas.transform.window_to_pixel(point_ss);
        let point_ws = point_ss - canvas.transform.widget_bounds.min;

        let Some(state) = self.state.as_mut() else {
            return Task::none();
        };

        if state.points_ps.len() >= 3 {
            let first_ws = canvas
                .transform
                .pixel_to_widget
                .transform_point2(state.points_ps[0]);

            if first_ws.distance_squared(point_ws) < POLYGON_CLOSE_DISTANCE * POLYGON_CLOSE_DISTANCE
            {
                let geometry = indices_from_vertices(&state.points_ps, self.fill_rule);
                let cmd = generate_cmd(
                    "Polygon Selection".into(),
                    &geometry.vertices,
                    &geometry.indices,
                    state.aabb.as_irect(),
                    state.op,
                    globals,
                );

                if let Some(cmd) = cmd {
                    globals.push_undo_command_to_current(cmd).log_err();
                    info!(
                        "Polygon select {} points aabb {:?}",
                        state.points_ps.len(),
                        state.aabb
                    );
                }

                self.state = None;

                return Task::none();
            }
        }

        state.points_ps.push(point_ps);
        state.aabb = state.aabb.union_point(point_ps);

        Task::none()
    }

    fn handle_message(&mut self, message: Self::Message, _: &mut Globals) -> Task<Self::Message> {
        match message {
            PolygonSelectionToolMessage::FillRuleChanged(fill_rule) => self.fill_rule = fill_rule,
        }

        Task::none()
    }

    fn tool_option_widget<'a>(
        &'a self,
        _: &'a Globals,
    ) -> Option<Element<'a, Self::Message, Theme, Renderer>> {
        let fields = form().push(
            t!("fill_rule"),
            segmented_control()
                .push(
                    label(t!("even_odd")),
                    self.fill_rule == FillRule::EvenOdd,
                    PolygonSelectionToolMessage::FillRuleChanged(FillRule::EvenOdd),
                )
                .push(
                    label(t!("non_zero")),
                    self.fill_rule == FillRule::NonZero,
                    PolygonSelectionToolMessage::FillRuleChanged(FillRule::NonZero),
                ),
        );

        Some(
            panel(fields)
                .transparent()
                .padding(2)
                .width(Length::Fill)
                .into(),
        )
    }

    fn canvas_overlay<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        let Some(state) = &self.state else {
            return space().into();
        };

        let Some(canvas) = globals.current_canvas() else {
            return space().into();
        };

        let line_vertices_ps = state
            .points_ps
            .iter()
            .copied()
            .chain([self.cursor_ps])
            .collect::<Vec<_>>();

        SelectionPreviewLayer {
            line_vertices_ps: Cow::Owned(line_vertices_ps),
            canvas_transform: &canvas.transform,
        }
        .into()
    }
}
