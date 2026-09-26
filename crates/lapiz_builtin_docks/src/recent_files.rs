use std::{
    path::Path,
    sync::{Arc, LazyLock},
};

use iced_core::{Element, Length, image::Handle, text::Ellipsis, window};
use iced_futures::Subscription;
use iced_runtime::Task;
use iced_widget::{Image, container, scrollable};
use lapiz_canvas::recent::{RecentFiles, recent_file_thumbnail_path};
use lapiz_config::Config;
use lapiz_dock::dock::{Dock, DockId};
use lapiz_file_dialog::LocalFile;
use lapiz_image_importer::start_import;
#[cfg(target_os = "android")]
use lapiz_runtime::android::AndroidAppExt as _;
use lapiz_runtime::{Renderer, Services, Theme};
use lapiz_widgets::{button::Button, flex::Flex, label::Label};

pub struct LandingDock {
    config: Config<RecentFiles>,
    files: Arc<RecentFiles>,
}

impl LandingDock {
    #[allow(
        clippy::new_without_default,
        reason = "Default cannot express the semantic of reading config from disk."
    )]
    pub fn new() -> Self {
        let config = Config::<RecentFiles>::read_or_init_or_fallback();
        Self {
            files: config.get(),
            config,
        }
    }
}

pub enum LandingDockMessage {
    OpenFile(usize),
    #[cfg(target_os = "android")]
    Opened(anyhow::Result<LocalFile>),
    RecentFilesChanged,
}

pub static RECENT_FILES_DOCK_ID: LazyLock<DockId> =
    LazyLock::new(|| DockId::new("recent_files_dock".into()));

impl Dock for LandingDock {
    type Message = LandingDockMessage;

    fn id(&self) -> DockId {
        RECENT_FILES_DOCK_ID.clone()
    }

    fn view<'a>(
        &'a self,
        _window_id: window::Id,
        _services: &'a Services,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        scrollable(
            container(
                Flex::row(self.files.files.iter().enumerate().map(|(i, f)| {
                    let file_name = if f.name.is_empty() {
                        Path::new(&f.path_or_uri)
                            .file_stem()
                            .and_then(|name| name.to_str())
                            .unwrap_or("")
                    } else {
                        &f.name
                    };
                    Button::new(
                        Flex::column([
                            Image::new(Handle::from_path(recent_file_thumbnail_path(
                                &f.path_or_uri,
                            )))
                            .into(),
                            // FIXME Ellipsis not working
                            Label::new(file_name)
                                .width(Length::Fill)
                                .height(Length::Fill)
                                .ellipsis(Ellipsis::End)
                                .into(),
                        ])
                        .gap(4.0)
                        .padding(2.0),
                    )
                    .width(100.0)
                    .height(140.0)
                    .on_press(LandingDockMessage::OpenFile(i))
                    .into()
                }))
                .wrap()
                .width(Length::Fill),
            )
            .padding(4.0),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn update(&mut self, message: Self::Message, services: &mut Services) -> Task<Self::Message> {
        match message {
            LandingDockMessage::OpenFile(i) => {
                let Some(file) = self.files.files.get(i) else {
                    return Task::none();
                };

                #[cfg(target_os = "android")]
                {
                    let app = services.android_app().clone();
                    let uri = file.path_or_uri.clone();
                    let name = file.name.clone();
                    Task::future(async move {
                        LandingDockMessage::Opened(LocalFile::open_android(app, uri, name))
                    })
                }

                #[cfg(not(target_os = "android"))]
                {
                    start_import(services, LocalFile::native(file.path_or_uri.clone().into()));
                    Task::none()
                }
            }
            #[cfg(target_os = "android")]
            LandingDockMessage::Opened(result) => {
                match result {
                    Ok(file) => start_import(services, file),
                    Err(error) => log::error!("Unable to reopen document: {error}"),
                }
                Task::none()
            }
            LandingDockMessage::RecentFilesChanged => {
                self.files = self.config.get();
                Task::none()
            }
        }
    }

    fn subscription(&self, _services: &Services) -> Subscription<Self::Message> {
        self.config
            .listen_to()
            .map(|_| LandingDockMessage::RecentFilesChanged)
    }
}
