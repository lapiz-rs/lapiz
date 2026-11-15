use std::{sync::Arc, time::Duration};

use anyhow::{Context as _, Result};
use futures::stream;
use glam::Vec4;
use iced_core::{Color, Element, Length, Theme, image::Handle};
use iced_futures::Subscription;
use iced_runtime::Task;
use lapiz_assets::{
    AssetAppExt as _,
    asset::{AssetHandle, AssetId},
    loader::AssetSerializer as _,
    store::AssetRegistry,
};
use lapiz_brush::{
    asset::BrushPreset,
    editor::{BrushEditorEngine, BrushEditorEngineId, BrushPresetEntry},
    instance::BrushPresetInstance,
    render::{
        graph::CanvasResources,
        stroke_preview::{create_stroke_preview, predefined_curve_samples},
    },
};
use lapiz_i18n::t;
use lapiz_render::{
    readback::{
        create_readback_buffer_and_schedule_copy_texture, readback_buffer_raw_on_submit_async,
    },
    render_context::RenderContextAppExt as _,
    util::DevicePollExt as _,
};
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_widgets::{
    button, column, flex::Flex, image, label, panel, panel::inset, row, scrollable, text_input,
};
use uuid::Uuid;

use self::controls::Section;
use crate::{asset::PhotoshopBrushSerializer, parameter::PhotoshopBrushPreset};

pub mod controls;

type EditorElement<'a> = Element<'a, PhotoshopBrushEditorMessage, Theme, Renderer>;

pub struct PhotoshopBrushEditor {
    brush: PhotoshopBrushPreset,
    handle: Option<AssetHandle<PhotoshopBrushPreset>>,
    section: Section,
    dirty: bool,
    revision: u64,
    preview: Option<Handle>,
    preview_revision: Option<u64>,
    preview_error: Option<String>,
    status: Option<String>,
}

pub enum PhotoshopBrushEditorMessage {
    SelectSection(Section),
    ToggleSection(Section, bool),
    NameChanged(String),
    Edit(Box<dyn FnOnce(&mut PhotoshopBrushPreset) + Send>),
    Save,
    RefreshPreview(u64),
    PreviewReady(u64, Result<Handle>),
}

impl Default for PhotoshopBrushEditor {
    fn default() -> Self {
        Self {
            brush: PhotoshopBrushPreset::default(),
            handle: None,
            section: Section::BrushTip,
            dirty: false,
            revision: 0,
            preview: None,
            preview_revision: None,
            preview_error: None,
            status: None,
        }
    }
}

impl BrushEditorEngine for PhotoshopBrushEditor {
    type Brush = PhotoshopBrushPreset;
    type Message = PhotoshopBrushEditorMessage;

    fn id() -> BrushEditorEngineId {
        BrushEditorEngineId::new("photoshop_brush_engine")
    }

    fn label() -> String {
        t!("ps_brush_engine")
    }

    fn query_brushes(&self, assets: &AssetRegistry) -> Result<Vec<BrushPresetEntry<Self::Brush>>> {
        assets
            .all_handles_of::<PhotoshopBrushPreset>()?
            .into_iter()
            .map(|handle| {
                Ok(BrushPresetEntry {
                    id: handle.id(),
                    name: handle.get()?.name.clone(),
                })
            })
            .collect()
    }

    fn load_brush(
        &mut self,
        asset: AssetId<Self::Brush>,
        assets: &AssetRegistry,
        _globals: &mut Globals,
    ) -> Result<()> {
        let handle = assets.handle(asset)?;
        let brush = handle.get()?.as_ref().clone();
        self.brush = brush;
        self.handle = Some(handle);
        self.section = Section::BrushTip;
        self.dirty = false;
        self.status = None;
        self.invalidate_preview();
        Ok(())
    }

    fn new_brush(
        &mut self,
        assets: &AssetRegistry,
        globals: &mut Globals,
    ) -> Result<AssetId<Self::Brush>> {
        let bundle = assets
            .bundles()
            .find(|bundle| !bundle.is_readonly())
            .map(|bundle| bundle.metadata().bundle_id)
            .context("A writable asset bundle is required for a new brush")?;
        let path = format!(
            "brush_{}.{}",
            Uuid::new_v4(),
            PhotoshopBrushSerializer::file_extension()
        );
        let id = assets.add_asset(bundle, path, Arc::new(PhotoshopBrushPreset::default()))?;
        self.load_brush(id, assets, globals)?;
        self.dirty = true;
        Ok(id)
    }

    fn compile_brush(&self, _globals: &Globals) -> Result<BrushPreset> {
        self.brush.compile()
    }

    fn view<'a>(&'a self, globals: &'a Globals) -> EditorElement<'a> {
        let name_input = Element::from(
            text_input("", &self.brush.name)
                .on_input(|name| name)
                .width(Length::Fill),
        )
        .map(PhotoshopBrushEditorMessage::NameChanged);
        let toolbar = row![
            name_input,
            button(label(if self.dirty {
                t!("save_dirty")
            } else {
                t!("save")
            }))
            .primary()
            .on_press_maybe(self.dirty.then_some(PhotoshopBrushEditorMessage::Save)),
        ]
        .gap(8.0);

        let sections = Section::ALL.into_iter().map(|section| {
            let toggle = controls::section_toggle(section, &self.brush);
            row![
                toggle,
                button(label(section.label()))
                    .width(Length::Fill)
                    .transparent()
                    .activated(self.section == section)
                    .on_press(PhotoshopBrushEditorMessage::SelectSection(section)),
            ]
            .gap(6.0)
            .into()
        });
        let categories = panel(scrollable(Flex::column(sections).gap(4.0)).height(Length::Fill))
            .padding(8)
            .width(200)
            .height(Length::Fill);

        let mut settings = column![label(self.section.label()).strong()];
        if self.section.enabled(&self.brush) == Some(false) {
            settings = settings.push(label(t!("ps_brush_section_disabled")).muted());
        }
        settings = settings.push(self.view_section(globals.assets()));
        let settings = panel(scrollable(settings.gap(14.0).padding(16)).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill);

        let mut content = column![
            toolbar,
            row![categories, settings].gap(8.0).height(Length::Fill),
            self.view_preview(),
        ]
        .gap(8.0)
        .width(Length::Fill)
        .height(Length::Fill);
        if let Some(status) = &self.status {
            content = content.push(label(status.as_str()).muted());
        }
        content.into()
    }

    fn update(&mut self, message: Self::Message, globals: &mut Globals) -> Task<Self::Message> {
        match message {
            PhotoshopBrushEditorMessage::SelectSection(section) => self.section = section,
            PhotoshopBrushEditorMessage::ToggleSection(section, enabled) => {
                section.set_enabled(&mut self.brush, enabled);
                self.section = section;
                self.dirty = true;
                self.status = None;
                self.invalidate_preview();
            }
            PhotoshopBrushEditorMessage::NameChanged(name) => {
                self.brush.name = name;
                self.dirty = true;
                self.status = None;
            }
            PhotoshopBrushEditorMessage::Edit(edit) => {
                edit(&mut self.brush);
                self.dirty = true;
                self.status = None;
                self.invalidate_preview();
            }
            PhotoshopBrushEditorMessage::Save => {
                self.status = Some(match self.save() {
                    Ok(()) => t!("ps_brush_saved"),
                    Err(error) => format!("{}: {error:#}", t!("ps_brush_save_error")),
                });
            }
            PhotoshopBrushEditorMessage::RefreshPreview(revision) => {
                if revision == self.revision {
                    self.preview_error = None;
                    self.preview_revision = None;

                    match self.render_preview(globals) {
                        Ok(task) => {
                            return task.map(move |result| {
                                PhotoshopBrushEditorMessage::PreviewReady(revision, result)
                            });
                        }
                        Err(error) => self.preview_error = Some(format!("{error:#}")),
                    }
                }
            }
            PhotoshopBrushEditorMessage::PreviewReady(revision, result) => {
                if revision == self.revision {
                    match result {
                        Ok(handle) => {
                            self.preview = Some(handle);
                            self.preview_revision = Some(revision);
                            self.preview_error = None;
                        }
                        Err(error) => self.preview_error = Some(format!("{error:#}")),
                    }
                }
            }
        }
        Task::none()
    }

    fn subscription(&self, _globals: &Globals) -> Subscription<Self::Message> {
        Subscription::run_with(self.revision, |revision| {
            let revision = *revision;
            stream::once(async move {
                smol::Timer::after(Duration::from_millis(200)).await;
                PhotoshopBrushEditorMessage::RefreshPreview(revision)
            })
        })
    }
}

impl PhotoshopBrushEditor {
    fn invalidate_preview(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.preview_error = None;
    }

    fn save(&mut self) -> Result<()> {
        let handle = self
            .handle
            .as_ref()
            .context("A brush asset is required for saving")?;
        handle.update(self.brush.clone())?;
        handle.write()?;
        self.dirty = false;
        Ok(())
    }

    fn view_preview(&self) -> EditorElement<'_> {
        let status = if self.preview_error.is_some() {
            t!("ps_brush_preview_error")
        } else if self.preview_revision != Some(self.revision) {
            t!("ps_brush_preview_pending")
        } else {
            t!("ps_brush_preview")
        };
        let mut content = column![
            row![
                label(status).strong(),
                button(label(t!("ps_brush_refresh")))
                    .on_press(PhotoshopBrushEditorMessage::RefreshPreview(self.revision)),
            ]
            .gap(8.0)
        ];
        if let Some(error) = &self.preview_error {
            content = content.push(scrollable(label(error.as_str())).height(130));
        } else if self.preview_revision == Some(self.revision)
            && let Some(preview) = &self.preview
        {
            content = content.push(
                panel(image(preview.clone()).width(Length::Fill).height(130))
                    .width(Length::Fill)
                    .style(|theme| inset(theme).background(Color::from_rgb8(48, 48, 48))),
            );
        } else {
            content =
                content.push(panel(label(t!("ps_brush_preview_pending")).muted()).height(130));
        }
        panel(content.gap(6.0))
            .inset()
            .padding(10)
            .width(Length::Fill)
            .into()
    }

    fn render_preview(&self, globals: &Globals) -> Result<Task<Result<Handle>>> {
        let brush = &self.brush;
        let preset = brush.compile()?;
        let instance = BrushPresetInstance::new(&preset, globals.assets().clone())?;
        let (width, height) = (512, 128);
        let task = create_stroke_preview(
            &instance,
            &predefined_curve_samples(width, height),
            width,
            height,
            globals,
            &CanvasResources {
                foreground_color: Vec4::ONE,
                background_color: Vec4::ZERO,
            },
        )?;
        let device = globals.render_device().clone();
        let queue = globals.render_queue().clone();

        Ok(task.then(move |texture| {
            let mut encoder = device.create_command_encoder(&Default::default());
            let staging =
                create_readback_buffer_and_schedule_copy_texture(&device, &mut encoder, &texture);
            let mut readback = Some(readback_buffer_raw_on_submit_async(
                &mut encoder,
                &staging,
                ..,
            ));
            let submission = queue.submit([encoder.finish()]);
            let device = device.clone();
            let width = texture.width();
            let height = texture.height();

            Task::future(async move { device.poll_indefinitely_for(submission) }).then(
                move |result| {
                    if let Err(error) = result {
                        return Task::done(Err(error.into()));
                    }
                    let readback = readback.take().expect("preview readback completes once");
                    Task::future(async move {
                        let bytes = readback.into_inner().await??;
                        Ok(Handle::from_rgba(width, height, bytes))
                    })
                },
            )
        }))
    }
}
