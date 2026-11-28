use anyhow::{Context as _, Result};
use iced_core::{Element, Length, Theme};
use iced_runtime::Task;
use lapiz_assets::{AssetAppExt as _, asset::AssetId, store::AssetRegistry};
use lapiz_brush::{
    asset::BrushPreset,
    engine::{BrushEngine, BrushEngineId, BrushPresetEntry},
    widget::BrushPreviewCompiled,
};
use lapiz_i18n::t;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_widgets::{button, column, flex::Flex, label, panel, row, scrollable, text_input};

use crate::{
    asset::PhotoshopBrushSerializer, editor::controls::Section, parameter::PhotoshopBrushPreset,
};

pub mod controls;

impl PhotoshopBrushEngine {
    fn view_preview<'a>(&'a self, globals: &'a Globals) -> EditorElement<'a> {
        let preview = match self.compile_brush(globals) {
            Ok(brush) => EditorElement::from(
                BrushPreviewCompiled::new(brush, self.asset_id.map(AssetId::into_untyped), globals)
                    .generation(self.revision),
            ),
            Err(error) => EditorElement::from(label(format!("{error:#}")).muted()),
        };
        panel(preview)
            .outline()
            .padding(8)
            .width(Length::Fill)
            .height(200)
            .into()
    }
}

type EditorElement<'a> = Element<'a, PhotoshopBrushEngineMessage, Theme, Renderer>;

pub struct PhotoshopBrushEngine {
    brush: PhotoshopBrushPreset,
    asset_id: Option<AssetId<PhotoshopBrushPreset>>,
    section: Section,
    dirty: bool,
    revision: u64,
    status: Option<String>,
}

pub enum PhotoshopBrushEngineMessage {
    SelectSection(Section),
    ToggleSection(Section, bool),
    NameChanged(String),
    Edit(Box<dyn FnOnce(&mut PhotoshopBrushPreset) + Send>),
}

impl BrushEngine for PhotoshopBrushEngine {
    type Brush = PhotoshopBrushPreset;
    type Message = PhotoshopBrushEngineMessage;
    type BrushSerializer = PhotoshopBrushSerializer;

    fn id() -> BrushEngineId {
        BrushEngineId::new("photoshop_brush_engine")
    }

    fn label() -> String {
        t!("photoshop_brush_engine")
    }

    fn query_brushes(assets: &AssetRegistry) -> Result<Vec<BrushPresetEntry<Self::Brush>>> {
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
        brush: &Self::Brush,
        asset_id: Option<AssetId<Self::Brush>>,
        _assets: &AssetRegistry,
    ) -> Result<Self> {
        Ok(Self {
            brush: brush.clone(),
            asset_id,
            section: Section::BrushTip,
            dirty: false,
            revision: 0,
            status: None,
        })
    }

    fn new_brush(name: String, _assets: &AssetRegistry) -> Result<Self::Brush> {
        Ok(PhotoshopBrushPreset {
            name,
            ..Default::default()
        })
    }

    fn brush_asset_id(&self) -> Option<AssetId<PhotoshopBrushPreset>> {
        self.asset_id
    }

    fn compile_brush(&self, _globals: &Globals) -> Result<BrushPreset> {
        self.brush.compile()
    }

    fn editor_view<'a>(&'a self, globals: &'a Globals) -> EditorElement<'a> {
        let name_input = Element::from(
            text_input("", &self.brush.name)
                .on_input(|name| name)
                .width(Length::Fill),
        )
        .map(PhotoshopBrushEngineMessage::NameChanged);
        let toolbar = row![name_input].gap(8.0);

        let sections = Section::ALL.into_iter().map(|section| {
            let toggle = controls::section_toggle(section, &self.brush);
            row![
                toggle,
                button(label(section.label()))
                    .width(Length::Fill)
                    .activated(self.section == section)
                    .on_press(PhotoshopBrushEngineMessage::SelectSection(section)),
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
            settings = settings.push(label(t!("photoshop_brush_section_disabled")).muted());
        }
        settings = settings.push(self.view_section(globals.assets()));
        let settings = panel(scrollable(settings.gap(14.0).padding(16)).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill);

        let mut content = column![
            toolbar,
            row![categories, settings].gap(8.0).height(Length::Fill),
            self.view_preview(globals),
        ]
        .gap(8.0)
        .width(Length::Fill)
        .height(Length::Fill);
        if let Some(status) = &self.status {
            content = content.push(label(status.as_str()).muted());
        }
        content.into()
    }

    fn editor_update(
        &mut self,
        message: Self::Message,
        _globals: &mut Globals,
    ) -> Task<Self::Message> {
        match message {
            PhotoshopBrushEngineMessage::SelectSection(section) => self.section = section,
            PhotoshopBrushEngineMessage::ToggleSection(section, enabled) => {
                section.set_enabled(&mut self.brush, enabled);
                self.section = section;
                self.dirty = true;
                self.status = None;
                self.revision = self.revision.wrapping_add(1);
            }
            PhotoshopBrushEngineMessage::NameChanged(name) => {
                self.brush.name = name;
                self.dirty = true;
                self.status = None;
            }
            PhotoshopBrushEngineMessage::Edit(edit) => {
                edit(&mut self.brush);
                self.dirty = true;
                self.status = None;
                self.revision = self.revision.wrapping_add(1);
            }
        }
        Task::none()
    }

    fn editor_dirty(&self) -> bool {
        self.dirty
    }

    fn editor_save(&mut self, globals: &Globals) -> Result<()> {
        let asset_id = self
            .asset_id
            .context("Cannot save brush without asset ID")?;
        let assets = globals.assets();
        let handle = assets.handle(asset_id)?;
        handle.update(self.brush.clone())?;
        handle.write()?;
        self.dirty = false;
        Ok(())
    }
}
