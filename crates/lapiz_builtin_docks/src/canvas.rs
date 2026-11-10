use std::{
    cell::RefCell,
    fs::{self, File},
};

use bevy_math::{IRect, Rect, UVec2};
use iced_core::{Element, Point, Theme, pointer, widget::Void, window};
use iced_futures::Subscription;
use iced_runtime::{
    Task,
    window::{events, raw_id},
};
use image::{ImageEncoder as _, codecs::png::PngEncoder};
use lapiz_canvas::{
    CanvasAppExt as _, CanvasId, CanvasManager, CanvasToolProxyAppExt as _,
    event::{CanvasRemoved, CanvasUpdated},
    recent::recent_file_thumbnail_path,
    widget::canvas::CanvasWidget,
};
use lapiz_dock::dock::{Dock, DockId};
use lapiz_i18n::t;
use lapiz_image::{
    composite::{BlendFunctionRegistry, ImageCompositor, LayerPreviewOverriders},
    tile::{GpuTileStorage, TileStorageAppExt as _},
};
use lapiz_input::{
    key::KeyboardState,
    mouse::{HoverMouseState, PressedMouseState},
};
use lapiz_render::render_context::RenderContextAppExt as _;
use lapiz_runtime::{
    Renderer, event::Event as _, global::Globals, platform::get_window_monitor_name,
};
use lapiz_tools::ErasedToolFunctionMessage;
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::stack;

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

    fn update_thumbnail(&self, globals: &Globals) -> Task<CanvasDockMessage> {
        let Some(canvas) = globals.canvas(&self.canvas) else {
            return Task::none();
        };

        let Some(file) = canvas.local_file() else {
            return Task::none();
        };

        let root_id = *canvas.image.layer_stack().root_id();
        let tiles = globals.tile_storage().clone();
        let source = file.source();
        let image_rect = canvas.image.image_pixel_rect();
        let canvas_id = self.canvas;
        let color_profile = canvas.image.profile().clone();

        Task::future(async move {
            let root_layer = tiles.get_layer(root_id).unwrap();
            let Ok(result) = root_layer
                .generate_thumbnail(UVec2::splat(256), image_rect, false)
                .await
                .logged_err()
            else {
                return;
            };

            let path = recent_file_thumbnail_path(&source);
            if let Some(parent) = path.parent()
                && fs::create_dir_all(parent).logged_err().is_err()
            {
                return;
            }

            let Ok(mut file) = File::create(&path).logged_err() else {
                return;
            };
            let Ok(encoded_profile) = color_profile.encode() else {
                return;
            };
            let mut encoder = PngEncoder::new(&mut file);
            if encoder
                .set_icc_profile(encoded_profile)
                .logged_err()
                .is_err()
            {
                return;
            }
            if encoder
                .write_image(
                    result.as_bytes(),
                    result.width(),
                    result.height(),
                    result.color().into(),
                )
                .logged_err()
                .is_err()
            {
                return;
            }

            log::info!(
                "Saved thumbnail for canvas {} to {}",
                canvas_id,
                path.display()
            );
        })
        .discard()
    }

    fn request_composite(&mut self, globals: &mut Globals, dirty_tiles: Option<IRect>) {
        globals.update_global::<LayerPreviewOverriders, _>(|overriders, globals| {
            let Some(canvas) = globals.canvas(&self.canvas) else {
                return;
            };
            let tiles = globals.tile_storage();
            let blend_functions = globals.global::<BlendFunctionRegistry>();
            let device = globals.render_device();
            let queue = globals.render_queue();
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
        globals: &'a Globals,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        let canvas_manager = globals.global::<CanvasManager>();
        self.window_id.replace(window_id);

        let (Some(canvas), Some(window_id), Some(monitor_name)) = (
            canvas_manager.get(&self.canvas),
            self.raw_window_id,
            self.monitor_name.clone(),
        ) else {
            return Void.into();
        };

        let canvas_overlay = globals.tool_proxy(&self.canvas).map(|proxy| {
            proxy
                .canvas_overlay(globals)
                .map(CanvasDockMessage::ToolFunctionMessage)
        });

        let canvas = CanvasWidget {
            is_focusing: canvas_manager.current_id() == Some(self.canvas),
            canvas,
            tile_storage: globals.global::<GpuTileStorage>().clone(),
            on_focus: Box::new(CanvasDockMessage::CanvasFocus),
            on_pointer_event: Box::new(CanvasDockMessage::PointerEvent),
            on_widget_rect_change: Box::new(CanvasDockMessage::WidgetRectChange),
            // TODO wrap in arc?
            color_profile: canvas.image.profile().clone(),
            window_id,
            monitor_name,
        };

        stack!(canvas, canvas_overlay).clip(true).into()
    }

    fn update(&mut self, message: Self::Message, globals: &mut Globals) -> Task<Self::Message> {
        match message {
            CanvasDockMessage::CanvasUpdated(dirty_tiles) => {
                self.request_composite(globals, dirty_tiles);

                Task::none()
            }
            CanvasDockMessage::PointerEvent(event) => {
                if globals.current_canvas_id() != Some(self.canvas) {
                    return Task::none();
                }

                globals
                    .update_tool_proxy(&self.canvas, |tool_proxy, globals| {
                        let keyboard_state = globals.global::<KeyboardState>().clone();

                        match event {
                            pointer::Event::PointerPressed { position, button }
                                if event.is_primary_press() =>
                            {
                                self.is_mouse_pressed = true;
                                self.cursor_position = position;
                                tool_proxy.mouse_pressed(
                                    &keyboard_state,
                                    &PressedMouseState::from_button(position, button),
                                    globals,
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
                                    globals,
                                )
                            }
                            pointer::Event::PointerMoved { position, source } => {
                                self.cursor_position = position;
                                if self.is_mouse_pressed {
                                    tool_proxy.mouse_moved_pressing(
                                        &keyboard_state,
                                        &PressedMouseState::from_pointer(position, source),
                                        globals,
                                    )
                                } else {
                                    tool_proxy.mouse_moved_hovering(
                                        &keyboard_state,
                                        &HoverMouseState::from_pointer(position, source),
                                        globals,
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
                globals
                    .global_mut::<CanvasManager>()
                    .set_current(self.canvas);
                Task::none()
            }
            CanvasDockMessage::WidgetRectChange(rect) => {
                let canvas_manager = globals.global_mut::<CanvasManager>();
                if let Some(canvas) = canvas_manager.get_mut(&self.canvas) {
                    canvas.transform.widget_bounds = rect;
                }
                Task::none()
            }
            CanvasDockMessage::ToolFunctionMessage(message) => globals
                .update_tool_proxy(&self.canvas, |tool_proxy, globals| {
                    tool_proxy.handle_message(message, globals)
                })
                .unwrap_or_else(Task::none)
                .map(CanvasDockMessage::ToolFunctionMessage),
            CanvasDockMessage::WindowMoved => {
                raw_id::<()>(*self.window_id.borrow()).map(CanvasDockMessage::RawWindowIdUpdate)
            }
            CanvasDockMessage::RawWindowIdUpdate(id) => {
                self.raw_window_id = Some(id);
                self.monitor_name = Some(get_window_monitor_name(id));
                Task::none()
            }
        }
    }

    fn on_open(&mut self, globals: &mut Globals) -> Task<Self::Message> {
        self.request_composite(globals, None);

        Task::batch([
            Task::done(CanvasDockMessage::WindowMoved),
            self.update_thumbnail(globals),
        ])
    }

    fn on_close(&mut self, globals: &mut Globals) -> Task<Self::Message> {
        CanvasRemoved::broadcast(CanvasRemoved { id: self.canvas });

        self.request_composite(globals, None);
        self.update_thumbnail(globals)
    }

    fn subscription(&self, globals: &Globals) -> Subscription<Self::Message> {
        let cur_window = *self.window_id.borrow();

        let canvas_update = CanvasUpdated::listen_to()
            .map(|e| CanvasDockMessage::CanvasUpdated(Some(e.dirty_tiles)));
        let window_moved =
            events()
                .with(cur_window)
                .filter_map(|(cur_window, (window_id, event))| {
                    if matches!(event, window::Event::Moved(_)) && cur_window == window_id {
                        Some(CanvasDockMessage::WindowMoved)
                    } else {
                        None
                    }
                });
        let tool = globals
            .tool_proxy(&self.canvas)
            .and_then(|tool_proxy| tool_proxy.subscription())
            .unwrap_or_else(Subscription::none)
            .map(CanvasDockMessage::ToolFunctionMessage);

        Subscription::batch([canvas_update, window_moved, tool])
    }
}
