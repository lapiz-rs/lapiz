use std::{fs::File, io::BufReader, sync::Arc};

use anyhow::{Context as _, Result, ensure};
use glam::UVec2;
use iced_core::{
    ContentFit, Element, Length, Size,
    clipboard::{Content, Error, Kind},
    image::Handle,
    window,
};
use iced_runtime::{
    Task, clipboard,
    window::{close, drag, minimize, open, toggle_maximize},
};
use image::{DynamicImage, RgbaImage};
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
use lapiz_widgets::{
    button, column, form, image::Image, label, panel, row, space, spin_box, tab_bar, title_bar,
};
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
    tab: CanvasTab,
    clipboard_image: Option<ClipboardImage>,
    reading_clipboard: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum CanvasTab {
    NewCanvas,
    Clipboard,
}

struct ClipboardImage {
    pixels: RgbaImage,
    preview: Handle,
    profile: ColorProfile,
}

impl ClipboardImage {
    fn from_content(content: &Content) -> Result<Self> {
        match content {
            Content::Image(image) => {
                let Size { width, height } = image.size;
                ensure!(
                    (1..=MAX_DIMENSION).contains(&width) && (1..=MAX_DIMENSION).contains(&height),
                    "Invalid clipboard image dimensions"
                );
                let expected_len = u64::from(width) * u64::from(height) * 4;
                ensure!(
                    image.rgba.len() as u64 == expected_len,
                    "Invalid clipboard image pixels"
                );
                let pixels = RgbaImage::from_raw(width, height, image.rgba.to_vec())
                    .context("Invalid clipboard image pixels")?;
                let preview = Handle::from_rgba(width, height, image.rgba.clone());

                return Ok(Self {
                    pixels,
                    preview,
                    profile: ColorProfile::new_srgb(),
                });
            }
            Content::Files(paths) => {
                for path in paths {
                    let Ok(file) = File::open(path) else {
                        continue;
                    };
                    if let Ok((image, profile)) =
                        CImage::load_image_with_profile(BufReader::new(file))
                    {
                        let pixels = image.into_rgba8();
                        let (width, height) = pixels.dimensions();
                        ensure!(
                            (1..=MAX_DIMENSION).contains(&width)
                                && (1..=MAX_DIMENSION).contains(&height),
                            "Invalid clipboard image dimensions"
                        );
                        let preview = Handle::from_rgba(width, height, pixels.as_raw().clone());

                        return Ok(Self {
                            pixels,
                            preview,
                            profile,
                        });
                    }
                }
            }
            _ => {}
        }

        anyhow::bail!("Expected clipboard image")
    }
}

#[derive(Clone)]
pub enum NewCanvasDialogMessage {
    TabSelected(usize),
    ClipboardRead(Result<Arc<Content>, Error>),
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
            size: Size::new(360.0, 400.0),
            min_size: Some(Size::new(360.0, 400.0)),
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
                tab: CanvasTab::NewCanvas,
                clipboard_image: None,
                reading_clipboard: false,
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
        let tabs = tab_bar(
            [t!("new_canvas_tab"), t!("clipboard_tab")],
            self.tab as usize,
        )
        .on_select(NewCanvasDialogMessage::TabSelected);
        let dimensions = if self.tab == CanvasTab::Clipboard {
            let size = self
                .clipboard_image
                .as_ref()
                .map(|image| image.pixels.dimensions());
            form()
                .push(
                    t!("new_canvas_width"),
                    label(size.map_or_else(|| "—".into(), |(width, _)| width.to_string())),
                )
                .push(
                    t!("new_canvas_height"),
                    label(size.map_or_else(|| "—".into(), |(_, height)| height.to_string())),
                )
        } else {
            form()
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
                )
        };
        let mut content = column![dimensions]
            .gap(8)
            .width(Length::Fill)
            .height(Length::Fill);
        if self.tab == CanvasTab::Clipboard {
            if let Some(image) = &self.clipboard_image {
                content = content.push(
                    Image::new(image.preview.clone())
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .content_fit(ContentFit::Contain),
                );
            } else if self.reading_clipboard {
                content = content.push(label(t!("clipboard_loading")));
            }
        }
        if let Some(error) = &self.error {
            content = content.push(label(error.as_str()));
        }
        let footer = row![
            space().width(Length::Fill),
            button(label(t!("cancel"))).on_press(NewCanvasDialogMessage::Close),
            button(label(t!("new_canvas_create")))
                .primary()
                .on_press_maybe(
                    (self.tab == CanvasTab::NewCanvas || self.clipboard_image.is_some())
                        .then_some(NewCanvasDialogMessage::Create),
                ),
        ]
        .gap(8)
        .width(Length::Fill);

        let body = column![tabs, content, footer]
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
            NewCanvasDialogMessage::TabSelected(index) => {
                self.tab = if index == CanvasTab::Clipboard as usize {
                    CanvasTab::Clipboard
                } else {
                    CanvasTab::NewCanvas
                };
                self.error = None;

                if self.tab == CanvasTab::Clipboard {
                    self.clipboard_image = None;
                    self.reading_clipboard = true;
                    clipboard::read(Kind::Image)
                        .then(|result| match result {
                            Err(Error::ContentNotAvailable) => clipboard::read(Kind::Files),
                            result => Task::done(result),
                        })
                        .map(NewCanvasDialogMessage::ClipboardRead)
                } else {
                    Task::none()
                }
            }
            NewCanvasDialogMessage::ClipboardRead(result) => {
                if self.tab == CanvasTab::Clipboard {
                    self.reading_clipboard = false;
                    match result {
                        Ok(content) => match ClipboardImage::from_content(&content) {
                            Ok(image) => self.clipboard_image = Some(image),
                            Err(_) => self.error = Some(t!("clipboard_invalid_image")),
                        },
                        Err(Error::ContentNotAvailable) => self.error = Some(t!("clipboard_empty")),
                        Err(_) => self.error = Some(t!("clipboard_read_error")),
                    }
                }
                Task::none()
            }
            NewCanvasDialogMessage::WidthChanged(width) => {
                self.width = width.clamp(1, MAX_DIMENSION);
                Task::none()
            }
            NewCanvasDialogMessage::HeightChanged(height) => {
                self.height = height.clamp(1, MAX_DIMENSION);
                Task::none()
            }
            NewCanvasDialogMessage::Create => {
                let this = &self;
                let tiles = globals.tile_storage();
                let archive = match LazuliArchive::new_in_memory() {
                    Ok(ok) => ok,
                    Err(err) => {
                        self.error = Some(t!("new_canvas_error", error = err.to_string()));
                        return Task::none();
                    }
                };
                let profile = ColorProfile::new_srgb();
                let image = match this.tab {
                    CanvasTab::NewCanvas => {
                        CImage::new(UVec2::new(this.width, this.height), profile)
                    }
                    CanvasTab::Clipboard => {
                        let image = this.clipboard_image.as_ref().unwrap();
                        CImage::from_image(
                            DynamicImage::ImageRgba8(image.pixels.clone()),
                            image.profile.clone(),
                            tiles,
                        )
                    }
                };

                let canvas = CCanvas::new(None, image, archive);
                let tiles = globals.tile_storage();
                for layer in canvas.image.layer_stack().iter_layers() {
                    if let Some(texel_type) = layer.properties().get_texel_type() {
                        tiles.declare_layer(*layer.id(), GpuLayerInfo { texel_type });
                    }
                }

                globals.add_canvas(canvas);
                close(self.window)
            }
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
