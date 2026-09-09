use std::{cell::RefCell, sync::LazyLock};

use bevy_math::{IRect, Rect};
use iced::{
    Element, Length, Size, Subscription, Task, Theme,
    event::listen_with,
    keyboard::{self, Modifiers},
    pointer,
    widget::Space,
    window,
};
use iced_core::Point;
use iced_widget::{space, stack};
use lapiz_assets::AssetAppExt;
use lapiz_brush::{asset::BrushPreset, tool::BrushServicesExt, widget::BrushPresetListDelegate};
use lapiz_canvas::{
    CanvasAppExt, CanvasId, CanvasManager, CanvasToolProxyAppExt, CanvasUndoStackAppExt,
    command::{LayerPropertyChangeCommand, MoveLayersCommand},
    event::{CanvasRemoved, CanvasUpdated},
    widget::{
        canvas::CanvasWidget,
        layer_stack::{DropInfo, LayerStackMessage, LayerStackView},
    },
};
use lapiz_color::{
    BackgroundColorChanged, Color, ForegroundBackgroundColorExt, ForegroundColorChanged,
    model::rgb::Rgb,
};
use lapiz_color_selector::{
    ColorModel, ColorSelector, ColorSelectorMessage, ColorSelectorState, GradientPlaneShape,
    config::{
        ColorSelectorConfig, ColorSelectorConfigEditorState, ColorSelectorConfigMessage,
        GradientBarConfig, GradientPlaneConfig, GradientPlaneFlipAxis,
    },
};
use lapiz_dock::dock::{Dock, DockId};
use lapiz_i18n::t;
use lapiz_image::{
    composite::{BlendFunctionRegistry, ImageCompositor, LayerPreviewOverriders},
    layer::{
        LayerId,
        properties::{LayerProperties, NamePropertyExt},
    },
    tile::{GpuTileStorage, TileStorageAppExt},
};
use lapiz_input::{
    key::KeyboardState,
    mouse::{HoverMouseState, PressedMouseState},
};
use lapiz_render::render_context::RenderContextAppExt;
use lapiz_runtime::{Renderer, Services, event::Event};
use lapiz_tools::{
    ErasedToolFunctionMessage, ToolFunctionRegistry, ToolId, manifest::ToolBoxManifest,
};
use lapiz_utils::log_err::LogErr;
use lapiz_widgets::{
    button::{self, Button},
    divider::Divider,
    flex::Flex,
    icon,
    label::Label,
    panel::Panel,
    scrollable::Scrollable,
    tooltip::{Position, Tooltip},
};
use moxcms::ColorProfile;

#[derive(Clone)]
pub enum ColorSelectorDockMessage {
    RawWindowId(u64),
    WindowMoved,
    ColorSelector(ColorSelectorMessage),
    ConfigEditor(ColorSelectorConfigMessage),
    OpenSettings,
    SettingsWindowClosed,
    ForegroundColorChanged(ForegroundColorChanged),
    BackgroundColorChanged(BackgroundColorChanged),
}

pub static COLOR_SELECTOR_DOCK_ID: LazyLock<DockId> =
    LazyLock::new(|| DockId::new("color_selector_dock".into()));

pub struct ColorSelectorDock {
    selector: ColorSelectorState,
    config_editor: ColorSelectorConfigEditorState,
    window_id: RefCell<window::Id>,
    settings_window_id: Option<window::Id>,

    last_color: Color,
    is_foreground_color: bool,
}

impl ColorSelectorDock {
    pub fn new(services: &Services) -> Self {
        let configs = vec![ColorSelectorConfig {
            name: "RGB".to_string(),
            max_plane_size: 512,
            max_planes_per_row: 2,
            planes: vec![
                GradientPlaneConfig {
                    model: ColorModel::Rgb,
                    shape: GradientPlaneShape::Square,
                    variable_channels: 0b110,
                    flip_axis: GradientPlaneFlipAxis::empty(),
                    rotation: 0.0,
                    show_primary_channel_ring: false,
                    primary_channel_ring_width: 20.0,
                    ring_bar_saturated_hue_channel: false,
                    ring_rotation: 0.0,
                    reversed_ring: false,
                },
                GradientPlaneConfig {
                    model: ColorModel::OkLab,
                    shape: GradientPlaneShape::Square,
                    variable_channels: 0b110,
                    flip_axis: GradientPlaneFlipAxis::empty(),
                    rotation: 0.0,
                    show_primary_channel_ring: true,
                    primary_channel_ring_width: 20.0,
                    ring_bar_saturated_hue_channel: true,
                    ring_rotation: std::f32::consts::FRAC_PI_2,
                    reversed_ring: false,
                },
            ],
            bars: vec![
                GradientBarConfig {
                    model: ColorModel::Rgb,
                    channel: 0,
                    bar_height: 20.0,
                    show_channel_label: true,
                    show_precise_spin_box: true,
                    show_primary_channel_lock: true,
                },
                GradientBarConfig {
                    model: ColorModel::Rgb,
                    channel: 1,
                    bar_height: 20.0,
                    show_channel_label: true,
                    show_precise_spin_box: false,
                    show_primary_channel_lock: true,
                },
                GradientBarConfig {
                    model: ColorModel::Rgb,
                    channel: 2,
                    bar_height: 20.0,
                    show_channel_label: false,
                    show_precise_spin_box: true,
                    show_primary_channel_lock: true,
                },
                GradientBarConfig {
                    model: ColorModel::Hsv,
                    channel: 0,
                    bar_height: 20.0,
                    show_channel_label: true,
                    show_precise_spin_box: true,
                    show_primary_channel_lock: false,
                },
            ],
            out_of_gamut_color: Rgb::new(0.5, 0.5, 0.5),
            use_out_of_gamut_color: true,
            clip_to_gamut: true,
        }];

        Self {
            selector: ColorSelectorState::new(
                Color::Rgb(Rgb::new(0.0, 0.0, 0.0)),
                ColorProfile::new_srgb(),
                configs.clone(),
                0,
                services,
            ),
            config_editor: ColorSelectorConfigEditorState::new(configs, Some(0)),
            window_id: RefCell::new(window::Id::unique()),
            settings_window_id: None,
            last_color: **services.foreground_color(),
            is_foreground_color: true,
        }
    }
}

impl Dock for ColorSelectorDock {
    type Message = ColorSelectorDockMessage;

    fn id(&self) -> DockId {
        COLOR_SELECTOR_DOCK_ID.clone()
    }

    fn view<'a>(
        &'a self,
        window_id: window::Id,
        _services: &'a Services,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        self.window_id.replace(window_id);
        let content = if self.settings_window_id == Some(window_id) {
            self.config_editor
                .view()
                .map(ColorSelectorDockMessage::ConfigEditor)
        } else {
            Flex::column([
                Scrollable::new(ColorSelector::new(
                    &self.selector,
                    ColorSelectorDockMessage::ColorSelector,
                ))
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
                Button::new(Label::new(t!("settings")))
                    .width(Length::Fill)
                    .on_press(ColorSelectorDockMessage::OpenSettings)
                    .into(),
            ])
            .gap(4)
            .height(Length::Fill)
            .into()
        };

        Panel::new(content)
            .padding(4)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn update(&mut self, message: Self::Message, services: &mut Services) -> Task<Self::Message> {
        match message {
            ColorSelectorDockMessage::WindowMoved => {
                let window_id = *self.window_id.borrow();
                window::raw_id::<()>(window_id).map(ColorSelectorDockMessage::RawWindowId)
            }
            ColorSelectorDockMessage::RawWindowId(id) => self
                .selector
                .set_output_profile(id, services)
                .map(ColorSelectorDockMessage::ColorSelector),
            ColorSelectorDockMessage::ColorSelector(ColorSelectorMessage::Confirmed(color)) => {
                if self.is_foreground_color {
                    **services.foreground_color_mut() = color;
                    ForegroundColorChanged::broadcast(ForegroundColorChanged::new(
                        self.last_color,
                        color,
                    ));
                } else {
                    **services.background_color_mut() = color;
                    BackgroundColorChanged::broadcast(BackgroundColorChanged::new(
                        self.last_color,
                        color,
                    ));
                }

                Task::none()
            }
            ColorSelectorDockMessage::ColorSelector(m) => self
                .selector
                .update(m, services)
                .map(ColorSelectorDockMessage::ColorSelector),
            ColorSelectorDockMessage::OpenSettings => {
                if let Some(id) = self.settings_window_id {
                    window::gain_focus(id)
                } else {
                    let (id, task) = window::open(window::Settings {
                        size: Size {
                            width: 700.0,
                            height: 900.0,
                        },
                        ..Default::default()
                    });
                    self.settings_window_id = Some(id);
                    self.config_editor = ColorSelectorConfigEditorState::new(
                        self.selector.configs().to_vec(),
                        Some(0),
                    );
                    task.discard()
                }
            }
            ColorSelectorDockMessage::SettingsWindowClosed => {
                self.settings_window_id = None;
                Task::none()
            }
            ColorSelectorDockMessage::ConfigEditor(ColorSelectorConfigMessage::Cancelled) => {
                if let Some(id) = self.settings_window_id {
                    window::close(id)
                } else {
                    Task::none()
                }
            }
            ColorSelectorDockMessage::ConfigEditor(ColorSelectorConfigMessage::Confirmed) => self
                .selector
                .set_configs(self.config_editor.configs().to_vec(), services)
                .map(ColorSelectorDockMessage::ColorSelector),
            ColorSelectorDockMessage::ConfigEditor(m) => {
                self.config_editor.update(m);
                Task::none()
            }
            ColorSelectorDockMessage::ForegroundColorChanged(event) => {
                if !self.is_foreground_color {
                    return Task::none();
                }

                dbg!(event.new);
                self.last_color = event.new;
                self.selector
                    .set_color(event.new, services)
                    .map(ColorSelectorDockMessage::ColorSelector)
            }
            ColorSelectorDockMessage::BackgroundColorChanged(event) => {
                if self.is_foreground_color {
                    return Task::none();
                }

                dbg!(event.new);
                self.last_color = event.new;
                self.selector
                    .set_color(event.new, services)
                    .map(ColorSelectorDockMessage::ColorSelector)
            }
        }
    }

    fn on_open(&mut self) -> Task<Self::Message> {
        Task::done(ColorSelectorDockMessage::WindowMoved)
    }

    fn subscription(&self, _services: &Services) -> Subscription<Self::Message> {
        let cur_window = *self.window_id.borrow();

        let window_moved =
            window::events()
                .with(cur_window)
                .filter_map(|(cur_window, (window_id, event))| {
                    if matches!(event, window::Event::Moved(_)) && cur_window == window_id {
                        Some(ColorSelectorDockMessage::WindowMoved)
                    } else {
                        None
                    }
                });

        let settings_window_closed = window::events().with(self.settings_window_id).filter_map(
            |(settings_window_id, (window_id, event))| {
                if matches!(event, window::Event::Closed) && Some(window_id) == settings_window_id {
                    Some(ColorSelectorDockMessage::SettingsWindowClosed)
                } else {
                    None
                }
            },
        );

        let foreground_color_changed = ForegroundColorChanged::listen_to()
            .map(ColorSelectorDockMessage::ForegroundColorChanged);
        let background_color_changed = BackgroundColorChanged::listen_to()
            .map(ColorSelectorDockMessage::BackgroundColorChanged);

        Subscription::batch([
            window_moved,
            settings_window_closed,
            foreground_color_changed,
            background_color_changed,
        ])
    }

    fn sub_windows(&self) -> Vec<window::Id> {
        if let Some(id) = self.settings_window_id {
            vec![id]
        } else {
            Vec::new()
        }
    }
}

pub static LAYER_DOCK_ID: LazyLock<DockId> = LazyLock::new(|| DockId::new("layer_dock".into()));

pub struct LayersDock {
    renaming_layer: Option<LayerId>,
    rename_value: String,
    drop_preview: Option<DropInfo>,
}

impl LayersDock {
    pub fn new() -> Self {
        Self {
            renaming_layer: None,
            rename_value: String::new(),
            drop_preview: None,
        }
    }

    fn push_property_change(
        services: &mut Services,
        layer_id: LayerId,
        apply: impl FnOnce(&mut LayerProperties),
    ) {
        let Some(canvas_id) = services.current_canvas_id() else {
            return;
        };
        let cmd = services.update_canvas(&canvas_id, |canvas, _services| {
            let layer = canvas.image.layer_stack().get_layer(&layer_id)?;
            let old = layer.properties().clone();
            let new = {
                let mut props = old.clone();
                apply(&mut props);
                props
            };
            Some(LayerPropertyChangeCommand {
                canvas: canvas_id,
                layer_id,
                old,
                new,
            })
        });
        if let Some(cmd) = cmd.flatten() {
            services.push_undo_command(&canvas_id, cmd).log_err();
        }
    }
}

#[derive(Debug, Clone)]
pub enum LayersDockMessage {
    Layer(LayerStackMessage),
    EscapePressed,
}

impl Dock for LayersDock {
    type Message = LayersDockMessage;

    fn id(&self) -> DockId {
        LAYER_DOCK_ID.clone()
    }

    fn view<'a>(
        &'a self,
        _window_id: window::Id,
        services: &'a Services,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        let Some(canvas) = services.current_canvas() else {
            return Space::new().into();
        };
        let blend_functions = services.service::<BlendFunctionRegistry>();
        let tile_storage = services.tile_storage();
        LayerStackView::new(
            canvas,
            blend_functions,
            tile_storage,
            self.renaming_layer,
            &self.rename_value,
            self.drop_preview.clone(),
            &|m| LayersDockMessage::Layer(m),
        )
        .into()
    }

    fn update(&mut self, message: Self::Message, services: &mut Services) -> Task<Self::Message> {
        match message {
            LayersDockMessage::EscapePressed => {
                if self.renaming_layer.is_some() {
                    self.renaming_layer = None;
                    self.rename_value.clear();
                }
            }
            LayersDockMessage::Layer(LayerStackMessage::LayerPropertyChanged(command)) => {
                let canvas_id = command.canvas;
                services.push_undo_command(&canvas_id, command).log_err();
            }
            LayersDockMessage::Layer(LayerStackMessage::DropPreview(drop_preview)) => {
                self.drop_preview = drop_preview;
            }
            LayersDockMessage::Layer(LayerStackMessage::SelectLayer(layer_id)) => {
                let Some(canvas_id) = services.current_canvas_id() else {
                    return Task::none();
                };
                let modifiers = services.service::<KeyboardState>().modifiers();
                services.update_canvas(&canvas_id, |canvas, _| {
                    if modifiers.contains(Modifiers::CTRL) {
                        canvas.toggle_layer_selection_and_active(layer_id);
                    } else if modifiers.contains(Modifiers::SHIFT) {
                        let active_layer = canvas.active_layer_id();
                        if layer_id == active_layer {
                            return;
                        }
                        let tree = canvas
                            .image
                            .layer_stack()
                            .iter_layers_dfs_display_order_without_root()
                            .map(|(n, _)| *n.id())
                            .collect::<Vec<_>>();
                        let mut on_select = false;
                        for layer in tree {
                            if on_select {
                                canvas.select_layer(layer);
                            }
                            if layer == layer_id || layer == active_layer {
                                on_select = !on_select;
                            }
                        }
                        canvas.set_active_layer(layer_id);
                    } else if !canvas.selected_layer_ids().contains(&layer_id) {
                        canvas.set_active_layer_and_clear_select(layer_id);
                    } else {
                        canvas.set_active_layer(layer_id);
                    }
                });
            }
            LayersDockMessage::Layer(LayerStackMessage::MoveLayers {
                layer_ids,
                new_parent,
                new_position,
            }) => {
                self.drop_preview = None;
                let Some(canvas_id) = services.current_canvas_id() else {
                    return Task::none();
                };
                let cmd = services.update_canvas(&canvas_id, |canvas, _services| {
                    let dragged = layer_ids.first()?;
                    let original_parent = canvas
                        .image
                        .layer_stack()
                        .get_layer(dragged)
                        .and_then(|n| n.parent().copied())?;
                    let original_index = canvas
                        .image
                        .layer_stack()
                        .get_layer(&original_parent)
                        .and_then(|p| p.child_index(dragged))
                        .unwrap_or(0);
                    let resolved_index = canvas
                        .image
                        .layer_stack()
                        .get_layer(&new_parent)
                        .and_then(|p| p.resolve_index(new_position));
                    if let Some(resolved_index) = resolved_index
                        && original_parent == new_parent
                        && original_index == resolved_index
                    {
                        return None;
                    }
                    Some(MoveLayersCommand::new(
                        canvas,
                        layer_ids.iter().copied(),
                        new_parent,
                        new_position,
                    ))
                });
                if let Some(cmd) = cmd.flatten() {
                    services.push_undo_command(&canvas_id, cmd).log_err();
                }
            }
            LayersDockMessage::Layer(LayerStackMessage::RenameLayer(layer_id)) => {
                let name = services.current_canvas().and_then(|canvas| {
                    canvas
                        .image
                        .layer_stack()
                        .get_layer(&layer_id)
                        .and_then(|layer| layer.properties().get_name())
                        .map(ToOwned::to_owned)
                });
                if let Some(name) = name {
                    self.renaming_layer = Some(layer_id);
                    self.rename_value = name;
                }
            }
            LayersDockMessage::Layer(LayerStackMessage::RenameChanged(value)) => {
                if self.renaming_layer.is_some() {
                    self.rename_value = value;
                }
            }
            LayersDockMessage::Layer(LayerStackMessage::RenameCommit(layer_id)) => {
                if self.renaming_layer != Some(layer_id) {
                    return Task::none();
                }
                let name = std::mem::take(&mut self.rename_value);
                self.renaming_layer = None;
                Self::push_property_change(services, layer_id, move |props| {
                    props.set_name(name);
                });
            }
        }
        Task::none()
    }

    fn subscription(&self, _services: &Services) -> Subscription<Self::Message> {
        listen_with(|event, _status, _window| match event {
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            }) => Some(LayersDockMessage::EscapePressed),
            _ => None,
        })
    }
}

pub static TOOL_OPTIONS_DOCK_ID: LazyLock<DockId> =
    LazyLock::new(|| DockId::new("tool_options_dock".into()));
pub static TOOL_BOX_DOCK_ID: LazyLock<DockId> =
    LazyLock::new(|| DockId::new("tool_box_dock".into()));
pub static BRUSH_PRESETS_DOCK_ID: LazyLock<DockId> =
    LazyLock::new(|| DockId::new("brush_presets_dock".into()));

pub fn construct_canvas_dock_id(canvas: CanvasId) -> String {
    format!("canvas_{}", canvas)
}

pub struct CanvasDock {
    canvas: CanvasId,

    is_mouse_pressed: bool,
    compositor: ImageCompositor,
    cursor_position: Point,

    window_id: RefCell<window::Id>,
    raw_window_id: Option<u64>,
    monitor_name: Option<String>,
}

impl CanvasDock {
    pub fn new(canvas: CanvasId, window_id: window::Id) -> Self {
        Self {
            canvas,
            is_mouse_pressed: false,
            compositor: ImageCompositor::default(),
            cursor_position: Point::default(),
            window_id: RefCell::new(window_id),
            raw_window_id: None,
            monitor_name: None,
        }
    }
}

pub enum CanvasDockMessage {
    WindowMoved,
    CanvasUpdated(Option<IRect>),
    CanvasFocus(Point),
    PointerEvent(pointer::Event),
    WidgetRectChange(Rect),
    ToolFunctionMessage(ErasedToolFunctionMessage),
    RawWindowIdUpdate(u64),
}

impl Dock for CanvasDock {
    type Message = CanvasDockMessage;

    fn id(&self) -> DockId {
        DockId::new(construct_canvas_dock_id(self.canvas).into())
    }

    fn display_name(&self) -> String {
        t!("canvas_dock", name = self.canvas.to_string())
    }

    fn view<'a>(
        &'a self,
        window_id: window::Id,
        services: &'a Services,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        let canvas_manager = services.service::<CanvasManager>();
        self.window_id.replace(window_id);

        let (Some(canvas), Some(window_id), Some(monitor_name)) = (
            canvas_manager.get(&self.canvas),
            self.raw_window_id,
            self.monitor_name.clone(),
        ) else {
            return Space::new().into();
        };

        let canvas_overlay = services.tool_proxy(&self.canvas).map(|proxy| {
            proxy
                .canvas_overlay(services)
                .map(CanvasDockMessage::ToolFunctionMessage)
        });

        let canvas = CanvasWidget {
            is_focusing: canvas_manager.current_id() == Some(self.canvas),
            canvas,
            tile_storage: services.service::<GpuTileStorage>().clone(),
            on_focus: Box::new(CanvasDockMessage::CanvasFocus),
            on_pointer_event: Box::new(CanvasDockMessage::PointerEvent),
            on_widget_rect_change: Box::new(CanvasDockMessage::WidgetRectChange),
            // TODO wrap in arc?
            color_profile: canvas.image.profile().clone(),
            window_id,
            monitor_name,
        };

        stack!(canvas, canvas_overlay).into()
    }

    fn update(&mut self, message: Self::Message, services: &mut Services) -> Task<Self::Message> {
        match message {
            CanvasDockMessage::CanvasUpdated(dirty_tiles) => {
                services.service_scope::<LayerPreviewOverriders, _>(|overriders, services| {
                    let Some(canvas) = services.canvas(&self.canvas) else {
                        return;
                    };
                    let tiles = services.tile_storage();
                    let blend_functions = services.service::<BlendFunctionRegistry>();
                    let device = services.render_device();
                    let queue = services.render_queue();
                    self.compositor.create_cache(
                        overriders,
                        &canvas.image,
                        tiles,
                        blend_functions,
                        device,
                        queue,
                    );
                    self.compositor.composite(
                        overriders,
                        dirty_tiles.unwrap_or_else(|| canvas.image.image_tile_rect()),
                        &canvas.image,
                        tiles,
                        device,
                        queue,
                    );
                });

                Task::none()
            }
            CanvasDockMessage::PointerEvent(event) => {
                if services.current_canvas_id() != Some(self.canvas) {
                    return Task::none();
                }

                services
                    .update_tool_proxy(&self.canvas, |tool_proxy, services| {
                        let keyboard_state = services.service::<KeyboardState>().clone();

                        match event {
                            pointer::Event::PointerPressed { position, button }
                                if event.is_primary_press() =>
                            {
                                self.is_mouse_pressed = true;
                                self.cursor_position = position;
                                tool_proxy.mouse_pressed(
                                    &keyboard_state,
                                    &PressedMouseState::from_button(position, button),
                                    services,
                                )
                            }
                            pointer::Event::PointerReleased { position, button }
                                if event.is_primary_release() =>
                            {
                                self.is_mouse_pressed = false;
                                self.cursor_position = position;
                                tool_proxy.mouse_released(
                                    &keyboard_state,
                                    &PressedMouseState::from_button(position, button),
                                    services,
                                )
                            }
                            pointer::Event::PointerMoved { position, source } => {
                                self.cursor_position = position;
                                if self.is_mouse_pressed {
                                    tool_proxy.mouse_moved_pressing(
                                        &keyboard_state,
                                        &PressedMouseState::from_pointer(position, source),
                                        services,
                                    )
                                } else {
                                    tool_proxy.mouse_moved_hovering(
                                        &keyboard_state,
                                        &HoverMouseState::from_pointer(position, source),
                                        services,
                                    )
                                }
                            }
                            _ => Task::none(),
                        }
                    })
                    .unwrap_or_else(Task::none)
                    .map(CanvasDockMessage::ToolFunctionMessage)
            }
            CanvasDockMessage::CanvasFocus(cursor_pos) => {
                self.cursor_position = cursor_pos;
                services
                    .service_mut::<CanvasManager>()
                    .set_current(self.canvas);
                Task::none()
            }
            CanvasDockMessage::WidgetRectChange(rect) => {
                let canvas_manager = services.service_mut::<CanvasManager>();
                if let Some(canvas) = canvas_manager.get_mut(&self.canvas) {
                    canvas.transform.widget_bounds = rect;
                }
                Task::none()
            }
            CanvasDockMessage::ToolFunctionMessage(message) => services
                .update_tool_proxy(&self.canvas, |tool_proxy, services| {
                    tool_proxy.handle_message(message, services)
                })
                .unwrap_or_else(Task::none)
                .map(CanvasDockMessage::ToolFunctionMessage),
            CanvasDockMessage::WindowMoved => window::raw_id::<()>(*self.window_id.borrow())
                .map(CanvasDockMessage::RawWindowIdUpdate),
            CanvasDockMessage::RawWindowIdUpdate(id) => {
                self.raw_window_id = Some(id);
                self.monitor_name = Some(lapiz_runtime::platform::get_window_monitor_name(id));
                Task::none()
            }
        }
    }

    fn on_open(&mut self) -> Task<Self::Message> {
        Task::batch([
            Task::done(CanvasDockMessage::WindowMoved),
            Task::done(CanvasDockMessage::CanvasUpdated(None)),
        ])
    }

    fn on_close(&mut self) -> Task<Self::Message> {
        CanvasRemoved::broadcast(CanvasRemoved { id: self.canvas });

        Task::none()
    }

    fn subscription(&self, services: &Services) -> Subscription<Self::Message> {
        let cur_window = *self.window_id.borrow();

        let canvas_update = CanvasUpdated::listen_to()
            .map(|e| CanvasDockMessage::CanvasUpdated(Some(e.dirty_tiles)));
        let window_moved =
            window::events()
                .with(cur_window)
                .filter_map(|(cur_window, (window_id, event))| {
                    if matches!(event, window::Event::Moved(_)) && cur_window == window_id {
                        Some(CanvasDockMessage::WindowMoved)
                    } else {
                        None
                    }
                });
        let tool = services
            .tool_proxy(&self.canvas)
            .and_then(|tool_proxy| tool_proxy.subscription())
            .unwrap_or_else(Subscription::none)
            .map(CanvasDockMessage::ToolFunctionMessage);

        Subscription::batch([canvas_update, window_moved, tool])
    }
}

pub struct ToolOptionsDock;

pub enum ToolOptionsDockMessage {
    ToolFunction(ErasedToolFunctionMessage),
}

impl ToolOptionsDock {
    pub fn new(_: &Services) -> Self {
        Self
    }
}

impl Dock for ToolOptionsDock {
    type Message = ToolOptionsDockMessage;

    fn id(&self) -> DockId {
        TOOL_OPTIONS_DOCK_ID.clone()
    }

    fn view<'a>(
        &'a self,
        _window_id: window::Id,
        services: &'a Services,
    ) -> Element<'a, Self::Message, Theme, lapiz_runtime::Renderer> {
        let Some(tool_proxy) = services.current_tool_proxy() else {
            return space().into();
        };

        let Some(widget) = tool_proxy.tool_option_widget(services) else {
            return Panel::new(Label::new(t!("no_tool_options")).muted())
                .padding(8)
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        };

        Panel::new(Scrollable::new(
            widget.map(ToolOptionsDockMessage::ToolFunction),
        ))
        .padding(4)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn update(&mut self, message: Self::Message, services: &mut Services) -> Task<Self::Message> {
        match message {
            ToolOptionsDockMessage::ToolFunction(message) => services
                .update_current_tool_proxy(|tool_proxy, services| {
                    tool_proxy.handle_message(message, services)
                })
                .unwrap_or_else(Task::none)
                .map(ToolOptionsDockMessage::ToolFunction),
        }
    }
}

pub struct ToolBoxDock {
    manifest: ToolBoxManifest,
}

pub enum ToolBoxDockMessage {
    Switch(ToolId),
    ToolFunction(ErasedToolFunctionMessage),
}

impl ToolBoxDock {
    pub fn new() -> Self {
        // TODO: move to a proper config directory once the app has one.
        let manifest = match std::fs::read_to_string("assets/tool_box_manifest.toml") {
            Ok(content) => match toml::from_str(&content) {
                Ok(manifest) => manifest,
                Err(error) => {
                    log::error!("Failed to parse tool box manifest: {error}");
                    ToolBoxManifest::default()
                }
            },
            Err(error) => {
                log::error!("Failed to read tool box manifest: {error}");
                ToolBoxManifest::default()
            }
        };
        Self { manifest }
    }
}

impl Dock for ToolBoxDock {
    type Message = ToolBoxDockMessage;

    fn id(&self) -> DockId {
        TOOL_BOX_DOCK_ID.clone()
    }

    fn view<'a>(
        &'a self,
        _window_id: window::Id,
        services: &'a Services,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        let active_tool = services
            .current_tool_proxy()
            .and_then(|proxy| proxy.current_tool());
        let tool_button = |tool: &'a ToolId| {
            let selected = active_tool == Some(tool);
            let glyph = services
                .service::<ToolFunctionRegistry>()
                .icon(tool)
                .unwrap_or_else(icon::info)
                .size(12)
                .style(move |theme, _| {
                    let p = theme.palette();
                    icon::Style {
                        color: Some(if selected {
                            p.primary.base.text
                        } else {
                            p.background.weak.text
                        }),
                    }
                });
            let button = Button::new(glyph)
                .width(28)
                .height(28)
                .padding(8)
                .style(move |theme, status| {
                    let p = theme.palette();
                    let hovered =
                        matches!(status, button::Status::Hovered | button::Status::Pressed);
                    button::Style {
                        background: Some(
                            if selected {
                                p.primary.base.color
                            } else if hovered {
                                p.primary.weak.color
                            } else {
                                iced::Color::TRANSPARENT
                            }
                            .into(),
                        ),
                        text_color: if selected {
                            p.primary.base.text
                        } else {
                            p.background.weak.text
                        },
                        border: iced::Border {
                            radius: 0.0.into(),
                            width: if selected { 1.0 } else { 0.0 },
                            color: p.primary.base.color,
                        },
                        ..Default::default()
                    }
                })
                .on_press(ToolBoxDockMessage::Switch(tool.clone()));
            Tooltip::new(button, Label::new(t!(tool)), Position::Right).into()
        };
        let separator = || {
            Flex::row([Divider::horizontal(1).into()])
                .height(7)
                .padding([3, 0])
                .into()
        };
        let mut items = Vec::new();
        for group in &self.manifest.groups {
            if !items.is_empty() {
                items.push(separator());
            }
            let buttons: Vec<_> = group.tools.iter().map(tool_button).collect();
            items.push(
                Flex::row(buttons)
                    .wrap()
                    .space_evenly()
                    .gap(1)
                    .width(Length::Fill)
                    .into(),
            );
        }
        let content = Flex::column(items).width(Length::Fill).gap(0).padding(4);

        Scrollable::new(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn update(&mut self, message: Self::Message, services: &mut Services) -> Task<Self::Message> {
        match message {
            ToolBoxDockMessage::Switch(tool) => services
                .update_current_tool_proxy(|proxy, services| proxy.switch_tool(tool, services))
                .unwrap_or_else(Task::none)
                .map(ToolBoxDockMessage::ToolFunction),
            ToolBoxDockMessage::ToolFunction(message) => services
                .update_current_tool_proxy(|proxy, services| {
                    proxy.handle_message(message, services)
                })
                .unwrap_or_else(Task::none)
                .map(ToolBoxDockMessage::ToolFunction),
        }
    }
}

pub struct BrushPresetDock {
    brushes: BrushPresetListDelegate,
}

#[derive(Clone)]
pub enum BrushPresetDockMessage {
    SelectBrush(usize),
}

impl BrushPresetDock {
    pub fn new(services: &Services) -> Self {
        Self {
            brushes: BrushPresetListDelegate::new(
                services.assets().all_handles_of::<BrushPreset>().unwrap(),
            ),
        }
    }
}

impl Dock for BrushPresetDock {
    type Message = BrushPresetDockMessage;

    fn id(&self) -> DockId {
        BRUSH_PRESETS_DOCK_ID.clone()
    }

    fn view<'a>(
        &'a self,
        _window_id: window::Id,
        _services: &'a Services,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        let buttons = self
            .brushes
            .items()
            .iter()
            .enumerate()
            .map(|(index, item)| {
                Button::new(Label::new(item.name.clone()))
                    .width(Length::Fill)
                    .activated(item.selected)
                    .on_press(BrushPresetDockMessage::SelectBrush(index))
                    .into()
            })
            .collect::<Vec<Element<'a, _, Theme, Renderer>>>();

        Scrollable::new(Flex::column(buttons).gap(2).padding(4))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn update(&mut self, message: Self::Message, services: &mut Services) -> Task<Self::Message> {
        match message {
            BrushPresetDockMessage::SelectBrush(index) => {
                self.brushes.select(index);
                let handle = self.brushes.get(index).map(|item| item.brush.clone());
                if let Some(handle) = handle {
                    services.set_current_brush_preset(handle);
                }
                Task::none()
            }
        }
    }
}
