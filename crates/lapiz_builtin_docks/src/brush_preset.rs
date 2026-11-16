use std::sync::LazyLock;

use iced_core::{Element, Length, Theme, window};
use iced_runtime::Task;
use lapiz_assets::AssetAppExt as _;
use lapiz_brush::{
    asset::BrushPreset,
    editor::{BrushEditorEngineId, BrushEngineEditorRegistry, UntypedBrushPresetEntry},
    tool::CurrentBrushPresetExt as _,
};
use lapiz_dock::dock::{Dock, DockId};
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{button, flex::Flex, label, scrollable};

pub static BRUSH_PRESETS_DOCK_ID: LazyLock<DockId> =
    LazyLock::new(|| DockId::new("brush_presets_dock".into()));

/// Workspace [`Dock`] that displays loaded [`BrushPreset`] assets and updates the active brush preset when selected.
pub struct BrushPresetDock {
    selected: Option<usize>,
    brushes: Vec<(BrushEditorEngineId, UntypedBrushPresetEntry)>,
}

/// Messages handled by [`BrushPresetDock`].
#[derive(Clone)]
pub enum BrushPresetDockMessage {
    SelectBrush(usize),
}

impl BrushPresetDock {
    pub fn new(globals: &Globals) -> Self {
        let engines = globals.global::<BrushEngineEditorRegistry>();
        let assets = globals.assets();
        let brushes = engines.all_brushes(assets);
        Self {
            selected: None,
            brushes,
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
        _globals: &'a Globals,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        let buttons = self
            .brushes
            .iter()
            .enumerate()
            .map(|(index, (_, item))| {
                button(label(item.name.clone()))
                    .width(Length::Fill)
                    .activated(Some(index) == self.selected)
                    .on_press(BrushPresetDockMessage::SelectBrush(index))
                    .into()
            })
            .collect::<Vec<Element<'a, _, Theme, Renderer>>>();

        scrollable(Flex::column(buttons).gap(2).padding(4))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn update(&mut self, message: Self::Message, globals: &mut Globals) -> Task<Self::Message> {
        match message {
            BrushPresetDockMessage::SelectBrush(index) => {
                self.selected = Some(index);

                let engines = globals.global::<BrushEngineEditorRegistry>();
                let (engine_id, brush_entry) = &self.brushes[index];
                let mut engine = engines.create(*engine_id).unwrap();
                let assets = globals.assets().clone();
                engine
                    .load_brush(brush_entry.id, &assets, globals)
                    .log_err();
                let Ok(brush) = engine.compile_brush(globals).logged_err() else {
                    return Task::none();
                };
                globals.set_current_brush_preset(&brush).log_err();
                Task::none()
            }
        }
    }
}
