use std::sync::LazyLock;

use iced_core::{Element, Length, Theme, window};
use iced_runtime::Task;
use lapiz_assets::AssetAppExt as _;
use lapiz_brush::{
    asset::BrushPreset, tool::BrushglobalsExt as _, widget::BrushPresetListDelegate,
};
use lapiz_dock::dock::{Dock, DockId};
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_widgets::{button, flex::Flex, label, scrollable};

pub static BRUSH_PRESETS_DOCK_ID: LazyLock<DockId> =
    LazyLock::new(|| DockId::new("brush_presets_dock".into()));

/// Workspace [`Dock`] that displays loaded [`BrushPreset`] assets and updates the active brush preset when selected.
pub struct BrushPresetDock {
    brushes: BrushPresetListDelegate,
}

/// Messages handled by [`BrushPresetDock`].
#[derive(Clone)]
pub enum BrushPresetDockMessage {
    SelectBrush(usize),
}

impl BrushPresetDock {
    pub fn new(globals: &Globals) -> Self {
        Self {
            brushes: BrushPresetListDelegate::new(
                globals.assets().all_handles_of::<BrushPreset>().unwrap(),
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
        _globals: &'a Globals,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        let buttons = self
            .brushes
            .items()
            .iter()
            .enumerate()
            .map(|(index, item)| {
                button(label(item.name.clone()))
                    .width(Length::Fill)
                    .activated(item.selected)
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
                self.brushes.select(index);
                let handle = self.brushes.get(index).map(|item| item.brush.clone());
                if let Some(handle) = handle {
                    globals.set_current_brush_preset(handle);
                }
                Task::none()
            }
        }
    }
}
