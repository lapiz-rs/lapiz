use std::sync::Arc;

use glam::Vec2;
use iced_core::{
    Color, Element, Event, Layout, Length, Point, Rectangle, Renderer as _, Shell, Size, Theme,
    Vector, Widget,
    layout::{self, Limits},
    pointer,
    pointer::mouse,
    renderer::{self},
    widget::{
        Tree,
        tree::{self, Tag},
    },
};
use iced_runtime::Task;
use iced_wgpu::graphics::geometry;
use iced_wgpu::primitive;
use iced_widget::canvas::{Frame, Path, Stroke};
use lapiz_runtime::Renderer;
use lapiz_runtime::Services;

use crate::{
    ColorSelectorMessage, ColorSelectorState,
    render::{GradientDrawPrimitive, SurfaceDrawData},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceTarget {
    Plane(usize),
    Bar(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActiveSelection {
    Plane(usize),
    Ring(usize),
    Bar(usize),
}

impl ActiveSelection {
    pub(crate) fn surface_target(self) -> SurfaceTarget {
        match self {
            ActiveSelection::Plane(index) | ActiveSelection::Ring(index) => {
                SurfaceTarget::Plane(index)
            }
            ActiveSelection::Bar(index) => SurfaceTarget::Bar(index),
        }
    }
}

fn remap_normalized(value: f32, range: Vec2) -> f32 {
    range.x + (range.y - range.x) * value
}

impl ColorSelectorState {
    pub(crate) fn start_plane_selection(
        &mut self,
        index: usize,
        position: Point,
        services: &Services,
    ) -> Task<ColorSelectorMessage> {
        let config = &self.presets[self.selected_preset].planes[index];
        let Some(bounds) = self.planes.get(index).map(|plane| plane.bounds) else {
            return Task::none();
        };
        let size = bounds.width;
        if size <= 0.0 {
            return Task::none();
        }

        let texture_uv = Vec2::new(
            (position.x - bounds.x) / size,
            1.0 - (position.y - bounds.y) / size,
        );
        let radius = (texture_uv - Vec2::splat(0.5)).length();
        let antialias_width = 1.0 / size;
        let outer_radius = 0.5 - antialias_width;
        let inner_radius = (outer_radius - config.primary_channel_ring_width / size).max(0.0);

        self.active_selection = if config.show_primary_channel_ring
            && radius >= inner_radius - antialias_width
            && radius <= outer_radius + antialias_width
        {
            Some(ActiveSelection::Ring(index))
        } else if self
            .plane_uv_from_window_position(index, position)
            .is_some_and(|(_, inside)| inside)
        {
            Some(ActiveSelection::Plane(index))
        } else {
            None
        };

        self.update_active_selection(position, services)
    }

    pub(crate) fn start_bar_selection(
        &mut self,
        index: usize,
        position: Point,
        services: &Services,
    ) -> Task<ColorSelectorMessage> {
        if index >= self.bars.len() {
            return Task::none();
        }
        self.active_selection = Some(ActiveSelection::Bar(index));
        self.update_active_selection(position, services)
    }

    pub(crate) fn update_active_selection(
        &mut self,
        position: Point,
        services: &Services,
    ) -> Task<ColorSelectorMessage> {
        let Some(selection) = self.active_selection else {
            return Task::none();
        };

        match selection {
            ActiveSelection::Plane(index) => {
                let config = &self.presets[self.selected_preset].planes[index];
                let Some((uv, _)) = self.plane_uv_from_window_position(index, position) else {
                    return Task::none();
                };
                let (x_range, y_range) = self.plane_normalized_ranges(index);
                let uv = Vec2::new(
                    remap_normalized(uv.x, x_range),
                    remap_normalized(uv.y, y_range),
                );
                let mut channels = config.model.channels(self.color, &self.profile);
                let ranges = config.model.channel_ranges();
                let mut variable_index = 0;
                let variable_channels = self.plane_variable_channels(index, config);
                for channel in 0..3 {
                    if variable_channels & (1 << channel) != 0 {
                        channels[channel] = ranges[channel].x
                            + (ranges[channel].y - ranges[channel].x) * uv[variable_index];
                        variable_index += 1;
                    }
                }
                self.color = config.model.color_from_channels(channels);
            }
            ActiveSelection::Ring(index) => {
                let config = &self.presets[self.selected_preset].planes[index];
                let Some(bounds) = self.planes.get(index).map(|plane| plane.bounds) else {
                    return Task::none();
                };

                let size = bounds.width;
                let centered = Vec2::new(
                    (position.x - bounds.x) / size - 0.5,
                    0.5 - (position.y - bounds.y) / size,
                );
                if centered.length_squared() <= f32::EPSILON {
                    return Task::none();
                }
                let mut angle = centered.y.atan2(centered.x) + config.ring_rotation;
                if config.reversed_ring {
                    angle = -angle;
                }
                let factor = (angle / std::f32::consts::TAU).rem_euclid(1.0);
                let channel = self.plane_primary_channel(index, config) as usize;
                let mut channels = config.model.channels(self.color, &self.profile);
                let range = config.model.channel_ranges()[channel];
                channels[channel] = range.x + (range.y - range.x) * factor;
                self.color = config.model.color_from_channels(channels);
            }
            ActiveSelection::Bar(index) => {
                let config = &self.presets[self.selected_preset].bars[index];
                let Some(bounds) = self.bars.get(index).map(|bar| bar.bounds) else {
                    return Task::none();
                };

                let factor = ((position.x - bounds.x) / bounds.width).clamp(0.0, 1.0);
                let channel = config.channel as usize;
                let mut channels = config.model.channels(self.color, &self.profile);
                let range = config.model.channel_ranges()[channel];
                channels[channel] = range.x + (range.y - range.x) * factor;
                self.color = config.model.color_from_channels(channels);
            }
        }

        Task::batch([
            self.refresh_clip_bounds(services),
            Task::done(ColorSelectorMessage::Changed(self.color())),
        ])
    }

    pub(crate) fn finish_active_selection(
        &mut self,
        target: SurfaceTarget,
        position: Point,
        services: &Services,
    ) -> Task<ColorSelectorMessage> {
        if self.active_selection.map(|s| s.surface_target()) != Some(target) {
            return Task::none();
        }
        let task = self.update_active_selection(position, services);
        self.active_selection = None;
        Task::batch([
            task,
            Task::done(ColorSelectorMessage::Confirmed(self.color())),
        ])
    }
}

pub(crate) struct GradientSurface {
    surface_target: SurfaceTarget,
    data: Option<Arc<SurfaceDrawData>>,
    plane_indicator: Option<Vec2>,
    ring_indicator: Option<Vec2>,
    bar_indicator: Option<f32>,
    indicator_color: Color,
    square: bool,
    max_width: f32,
    width: Length,
    height: Length,
    cur_bounds: Rectangle,
}

impl GradientSurface {
    pub(crate) fn plane(
        index: usize,
        data: Option<Arc<SurfaceDrawData>>,
        max_width: f32,
        cur_bounds: Rectangle,
    ) -> Self {
        Self {
            surface_target: SurfaceTarget::Plane(index),
            data,
            plane_indicator: None,
            ring_indicator: None,
            bar_indicator: None,
            indicator_color: Color::WHITE,
            square: true,
            max_width,
            width: Length::FillPortion(1),
            height: Length::Fill,
            cur_bounds,
        }
    }

    pub(crate) fn bar(
        index: usize,
        data: Option<Arc<SurfaceDrawData>>,
        height: f32,
        cur_bounds: Rectangle,
    ) -> Self {
        Self {
            surface_target: SurfaceTarget::Bar(index),
            data,
            plane_indicator: None,
            ring_indicator: None,
            bar_indicator: None,
            indicator_color: Color::WHITE,
            square: false,
            max_width: f32::INFINITY,
            width: Length::Fill,
            height: Length::Fixed(height),
            cur_bounds,
        }
    }

    pub(crate) fn plane_indicator(mut self, position: Option<Vec2>) -> Self {
        self.plane_indicator = position;
        self
    }

    pub(crate) fn ring_indicator(mut self, position: Option<Vec2>) -> Self {
        self.ring_indicator = position;
        self
    }

    pub(crate) fn bar_indicator(mut self, position: f32) -> Self {
        self.bar_indicator = Some(position);
        self
    }

    pub(crate) fn indicator_color(mut self, color: Color) -> Self {
        self.indicator_color = color;
        self
    }
}

impl Widget<ColorSelectorMessage, Theme, Renderer> for GradientSurface {
    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &Renderer, limits: &Limits) -> layout::Node {
        let limits = limits.width(Length::Fit.max(self.max_width));
        let size = limits.resolve(self.width, self.height, Size::ZERO);
        let size = if self.square {
            let side = size.width.min(size.height).max(1.0);
            Size::new(side, side)
        } else {
            size
        };
        layout::Node::new(size)
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        shell: &mut Shell<'_, ColorSelectorMessage>,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        if self.cur_bounds != bounds {
            shell.publish(ColorSelectorMessage::SurfaceBoundsChanged(
                self.surface_target,
                bounds,
            ));
        }

        let Some(position) = cursor.position_over(bounds) else {
            return;
        };

        match event {
            Event::Pointer(event) if event.is_primary_press() => {
                shell.publish(ColorSelectorMessage::SurfacePress(
                    self.surface_target,
                    position,
                ));
                shell.capture_event();
            }
            Event::Pointer(event) if event.is_primary_release() => {
                shell.publish(ColorSelectorMessage::SurfaceRelease);
                shell.capture_event();
            }
            Event::Pointer(pointer::Event::PointerMoved { .. }) => {
                shell.publish(ColorSelectorMessage::SurfaceMove(position));
                shell.capture_event();
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        if let Some(data) = &self.data {
            primitive::Renderer::draw_primitive(
                renderer,
                bounds,
                GradientDrawPrimitive { data: data.clone() },
            );
        }

        renderer.with_layer(bounds, |renderer| {
            renderer.with_translation(Vector::new(bounds.x, bounds.y), |renderer| {
                let mut frame = Frame::new(renderer, bounds.size());

                let path = Path::new(|builder| {
                    if let Some(position) = self.plane_indicator {
                        builder.circle(
                            Point::new(position.x * bounds.width, position.y * bounds.height),
                            3.0,
                        );
                    }
                    if let Some(position) = self.ring_indicator {
                        builder.circle(
                            Point::new(position.x * bounds.width, position.y * bounds.height),
                            3.0,
                        );
                    }
                    if let Some(fraction) = self.bar_indicator {
                        let width = 4.0;
                        let x = fraction * bounds.width;

                        builder.rectangle(
                            Point::new(x - width * 0.5, 0.0),
                            Size::new(width, bounds.height),
                        );
                    }
                });

                frame.stroke(
                    &path,
                    Stroke {
                        style: self.indicator_color.into(),
                        width: 1.0,
                        ..Default::default()
                    },
                );

                geometry::Renderer::draw_geometry(renderer, frame.into_geometry());
            });
        });
    }
}

pub(crate) struct PlaneRow {
    surfaces: Vec<GradientSurface>,
    spacing: f32,
    max_cell_size: f32,
}

impl PlaneRow {
    pub(crate) fn new(surfaces: Vec<GradientSurface>) -> Self {
        Self {
            surfaces,
            spacing: 5.0,
            max_cell_size: f32::INFINITY,
        }
    }

    pub(crate) fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub(crate) fn max_cell_size(mut self, max_cell_size: f32) -> Self {
        self.max_cell_size = max_cell_size;
        self
    }
}

impl Widget<ColorSelectorMessage, Theme, Renderer> for PlaneRow {
    fn tag(&self) -> tree::Tag {
        self.surfaces
            .first()
            .map_or(Tag::stateless(), |surface| surface.tag())
    }

    fn state(&self) -> tree::State {
        self.surfaces
            .first()
            .map_or(tree::State::new(()), |surface| surface.state())
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children_custom(
            &mut self.surfaces,
            |tree, surface| surface.diff(tree),
            |surface| Tree {
                tag: surface.tag(),
                state: surface.state(),
                children: Vec::new(),
            },
        );
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fit)
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &Limits) -> layout::Node {
        let count = self.surfaces.len().max(1);
        let spacing = if count > 1 {
            self.spacing * (count - 1) as f32
        } else {
            0.0
        };
        let width = limits.max().width;
        let cell = ((width - spacing) / count as f32)
            .min(self.max_cell_size)
            .max(1.0);
        let surface_size = (cell - 5.0).max(1.0);

        let mut nodes: Vec<layout::Node> = Vec::with_capacity(count);
        let mut height: f32 = 0.0;
        for (surface, child_tree) in self.surfaces.iter_mut().zip(tree.children.iter_mut()) {
            let child_limits = Limits::new(Size::ZERO, Size::new(surface_size, f32::INFINITY));
            let node = surface.layout(child_tree, renderer, &child_limits);
            height = height.max(node.size().height);
            nodes.push(node);
        }

        let mut main = 2.5;
        for node in nodes.iter_mut() {
            node.move_to_mut(Point::new(main, 0.0));
            main += node.size().width + self.spacing;
        }

        layout::Node::with_children(Size::new(width, height), nodes)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, ColorSelectorMessage>,
        viewport: &Rectangle,
    ) {
        for ((surface, child_tree), child_layout) in self
            .surfaces
            .iter_mut()
            .zip(tree.children.iter_mut())
            .zip(layout.children())
        {
            surface.update(
                child_tree,
                event,
                child_layout,
                cursor,
                renderer,
                shell,
                viewport,
            );
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
        for (surface, (child_tree, child_layout)) in self
            .surfaces
            .iter()
            .zip(tree.children.iter().zip(layout.children()))
        {
            surface.draw(
                child_tree,
                renderer,
                theme,
                style,
                child_layout,
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
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.surfaces
            .iter()
            .zip(tree.children.iter().zip(layout.children()))
            .map(|(surface, (child_tree, child_layout))| {
                surface.mouse_interaction(child_tree, child_layout, cursor, viewport, renderer)
            })
            .fold(mouse::Interaction::default(), |a, b| a.max(b))
    }
}

impl<'a> From<PlaneRow> for Element<'a, ColorSelectorMessage, Theme, Renderer> {
    fn from(row: PlaneRow) -> Self {
        Element::new(row)
    }
}

impl<'a> From<GradientSurface> for Element<'a, ColorSelectorMessage, Theme, Renderer> {
    fn from(surface: GradientSurface) -> Self {
        Element::new(surface)
    }
}
