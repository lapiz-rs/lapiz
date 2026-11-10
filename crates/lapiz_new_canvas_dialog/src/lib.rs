use std::sync::Arc;

use anyhow::Result;
use glam::UVec2;
use iced_core::{Element, Length, Size, window};
use iced_runtime::{
    Task,
    window::{close, drag, minimize, open, toggle_maximize},
};
use lapiz_canvas::{CCanvas, CanvasAppExt as _};
use lapiz_i18n::t;
use lapiz_image::{
    CImage,
    layer::properties::builtin::LayerTexelTypePropertyExt as _,
    tile::{GpuLayerInfo, GpuTileStorage, TileStorageAppExt as _},
};
use lapiz_lazuli::LazuliArchive;
use lapiz_runtime::{
    Renderer, Runtime, Theme,
    global::Globals,
    plugin::Plugin,
    windows::{WindowView, WindowViewId},
};
use lapiz_widgets::{button, column, form, label, panel, row, space, spin_box, title_bar};
use moxcms::ColorProfile;

lapiz_i18n::define_i18n!("new_canvas_dialog");

pub const NEW_CANVAS_DIALOG_VIEW_ID: &str = "new_canvas_dialog";
const MAX_DIMENSION: u32 = i32::MAX as u32;

pub struct NewCanvasDialogPlugin;

impl Plugin for NewCanvasDialogPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();
        app.register_view::<NewCanvasDialog>();
    }
}

pub struct NewCanvasDialog {
    window: window::Id,
    width: u32,
    height: u32,
    error: Option<String>,
}

#[derive(Clone)]
pub enum NewCanvasDialogMessage {
    WidthChanged(u32),
    HeightChanged(u32),
    Create,
    Close,
    Drag,
    Minimize,
    Maximize,
}

impl WindowView for NewCanvasDialog {
    type Message = NewCanvasDialogMessage;
    type BootParams = ();

    fn id() -> WindowViewId {
        WindowViewId::new(NEW_CANVAS_DIALOG_VIEW_ID)
    }

    fn boot(
        _params: Option<Self::BootParams>,
        _globals: &mut Globals,
    ) -> Result<(Self, Task<Self::Message>)> {
        let (window, task) = open(window::Settings {
            decorations: false,
            size: Size::new(360.0, 280.0),
            min_size: Some(Size::new(360.0, 280.0)),
            #[cfg(target_os = "windows")]
            platform_specific: window::settings::PlatformSpecific {
                corner_preference: window::settings::platform::CornerPreference::DoNotRound,
                ..Default::default()
            },
            ..Default::default()
        });

        Ok((
            Self {
                window,
                width: 1920,
                height: 1080,
                error: None,
            },
            task.discard(),
        ))
    }

    fn view<'a>(
        &'a self,
        _: window::Id,
        _: &'a Globals,
    ) -> impl Into<Element<'a, Self::Message, Theme, Renderer>> {
        let title = title_bar(label(t!("new_canvas_title")).window_title())
            .on_close(NewCanvasDialogMessage::Close)
            .on_drag(NewCanvasDialogMessage::Drag)
            .on_minimize(NewCanvasDialogMessage::Minimize)
            .on_maximize(NewCanvasDialogMessage::Maximize);
        let dimensions = form()
            .push(
                t!("new_canvas_width"),
                spin_box(
                    &self.width,
                    1..=MAX_DIMENSION,
                    NewCanvasDialogMessage::WidthChanged,
                )
                .width(Length::Fill),
            )
            .push(
                t!("new_canvas_height"),
                spin_box(
                    &self.height,
                    1..=MAX_DIMENSION,
                    NewCanvasDialogMessage::HeightChanged,
                )
                .width(Length::Fill),
            );
        let mut content = column![dimensions]
            .gap(8)
            .width(Length::Fill)
            .height(Length::Fill);
        if let Some(error) = &self.error {
            content = content.push(label(error.as_str()));
        }
        let footer = row![
            space().width(Length::Fill),
            button(label(t!("cancel"))).on_press(NewCanvasDialogMessage::Close),
            button(label(t!("new_canvas_create")))
                .primary()
                .on_press(NewCanvasDialogMessage::Create),
        ]
        .gap(8)
        .width(Length::Fill);

        let body = column![content, footer]
            .gap(12)
            .padding(16)
            .width(Length::Fill)
            .height(Length::Fill);

        panel(
            column![title, body]
                .width(Length::Fill)
                .height(Length::Fill),
        )
    }

    fn update(
        &mut self,
        message: Self::Message,
        globals: &mut Globals,
    ) -> impl Into<Task<Self::Message>> {
        match message {
            NewCanvasDialogMessage::WidthChanged(width) => {
                self.width = width.clamp(1, MAX_DIMENSION);
                Task::none()
            }
            NewCanvasDialogMessage::HeightChanged(height) => {
                self.height = height.clamp(1, MAX_DIMENSION);
                Task::none()
            }
            NewCanvasDialogMessage::Create => match Self::create_canvas(self.width, self.height) {
                Ok(canvas) => {
                    let tiles = globals.tile_storage();
                    for layer in canvas.image.layer_stack().iter_layers() {
                        if let Some(texel_type) = layer.properties().get_texel_type() {
                            tiles.declare_layer(*layer.id(), GpuLayerInfo { texel_type });
                        }
                    }

                    globals.add_canvas(canvas);
                    close(self.window)
                }
                Err(error) => {
                    self.error = Some(t!("new_canvas_error", error = error.to_string()));
                    Task::none()
                }
            },
            NewCanvasDialogMessage::Close => close(self.window),
            NewCanvasDialogMessage::Drag => drag(self.window),
            NewCanvasDialogMessage::Minimize => minimize(self.window, true),
            NewCanvasDialogMessage::Maximize => toggle_maximize(self.window),
        }
    }

    fn close(self, _: &mut Globals) -> Task<()> {
        close(self.window)
    }

    fn windows(&self) -> Arc<[window::Id]> {
        [self.window].into()
    }

    fn root_window(&self) -> Option<window::Id> {
        Some(self.window)
    }
}

impl NewCanvasDialog {
    fn create_canvas(width: u32, height: u32) -> Result<CCanvas> {
        let image = CImage::new(UVec2::new(width, height), ColorProfile::new_srgb());
        let archive = LazuliArchive::new_in_memory()?;
        Ok(CCanvas::new(None, image, archive))
    }
}
