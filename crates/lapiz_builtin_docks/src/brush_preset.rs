use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
};

use anyhow::Result;
use iced_core::{Border, Element, Length, Theme, window};
use iced_runtime::Task;
use lapiz_assets::{
    AssetAppExt as _,
    asset::UntypedAssetId,
    index_db::{AssetIndexDb, UntypedAssetFilter},
    tag::{Tag, TagId},
};
use lapiz_brush::{
    engine::{BrushEngineId, BrushEngineRegistryExt as _},
    tool::CurrentBrushPresetExt as _,
};
use lapiz_dock::dock::{Dock, DockId};
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{
    asset_hierarchy::{AssetHierarchy, AssetHierarchyNode},
    button, label, scrollable,
};

pub static BRUSH_PRESETS_DOCK_ID: LazyLock<DockId> =
    LazyLock::new(|| DockId::new("brush_presets_dock".into()));

#[derive(Clone)]
struct BrushRow {
    engine_id: BrushEngineId,
    asset_id: UntypedAssetId,
    name: String,
}

/// Workspace [`Dock`] that browses brush presets by tag and selects the active preset.
pub struct BrushPresetDock {
    selected: Option<UntypedAssetId>,
    expanded_tags: HashSet<TagId>,
    hierarchy: AssetHierarchy<TagId, BrushRow>,
}

/// Messages handled by [`BrushPresetDock`].
#[derive(Clone)]
pub enum BrushPresetDockMessage {
    ToggleTag(TagId),
    SelectBrush {
        engine_id: BrushEngineId,
        asset_id: UntypedAssetId,
    },
}

impl BrushPresetDock {
    pub fn new(globals: &Globals) -> Self {
        let assets = globals.assets();
        let brushes = globals
            .brush_engine_registry()
            .all_brushes(assets)
            .into_iter()
            .map(|(engine_id, entry)| BrushRow {
                engine_id,
                asset_id: entry.id,
                name: entry.name,
            })
            .collect::<Vec<_>>();
        let brushes_by_id = brushes
            .iter()
            .map(|brush| (brush.asset_id, brush))
            .collect();
        let index = assets.index_db();
        let mut tagged_brushes = HashSet::new();
        let read_tree = index.get_root_tags().and_then(|tags| {
            let mut roots = Vec::new();
            for tag in tags {
                if let Some(node) = read_tag(index, tag, &brushes_by_id, &mut tagged_brushes)? {
                    roots.push(node);
                }
            }
            Ok(roots)
        });
        let mut roots = read_tree.logged_err().unwrap_or_else(|_| {
            tagged_brushes.clear();
            Vec::new()
        });
        roots.extend(
            brushes
                .into_iter()
                .filter(|brush| !tagged_brushes.contains(&brush.asset_id))
                .map(AssetHierarchyNode::Asset),
        );
        Self {
            selected: None,
            expanded_tags: HashSet::new(),
            hierarchy: AssetHierarchy { roots },
        }
    }
}

fn read_tag(
    index: &AssetIndexDb,
    tag: Tag,
    brushes: &HashMap<UntypedAssetId, &BrushRow>,
    tagged_brushes: &mut HashSet<UntypedAssetId>,
) -> Result<Option<AssetHierarchyNode<TagId, BrushRow>>> {
    let mut children = Vec::new();
    for child in index.get_child_tags(&tag.id)? {
        if let Some(node) = read_tag(index, child, brushes, tagged_brushes)? {
            children.push(node);
        }
    }
    for asset in index.get_assets(UntypedAssetFilter {
        tag: Some(tag.id),
        ..Default::default()
    })? {
        if let Some(&brush) = brushes.get(&asset.asset_id) {
            tagged_brushes.insert(brush.asset_id);
            children.push(AssetHierarchyNode::Asset(brush.clone()));
        }
    }
    Ok((!children.is_empty()).then_some(AssetHierarchyNode::Tag {
        id: tag.id,
        name: tag.name,
        children,
    }))
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
        let tree = self.hierarchy.view(
            &self.expanded_tags,
            BrushPresetDockMessage::ToggleTag,
            |brush| {
                let selected = Some(brush.asset_id) == self.selected;
                button(label(brush.name.as_str()).width(Length::Fill))
                    .width(Length::Fill)
                    .style(move |theme, status| {
                        if selected {
                            button::Style {
                                border: Border::default(),
                                ..button::activated_style(theme, status)
                            }
                        } else {
                            button::transparent(theme, status)
                        }
                    })
                    .on_press(BrushPresetDockMessage::SelectBrush {
                        engine_id: brush.engine_id,
                        asset_id: brush.asset_id,
                    })
                    .into()
            },
        );
        scrollable(tree)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn update(&mut self, message: Self::Message, globals: &mut Globals) -> Task<Self::Message> {
        match message {
            BrushPresetDockMessage::ToggleTag(id) => {
                if !self.expanded_tags.remove(&id) {
                    self.expanded_tags.insert(id);
                }
            }
            BrushPresetDockMessage::SelectBrush {
                engine_id,
                asset_id,
            } => {
                let result = globals
                    .brush_engine_registry()
                    .get(&engine_id)
                    .and_then(|factory| factory.load_brush_from_assets(asset_id, globals.assets()))
                    .and_then(|engine| engine.compile_brush(globals))
                    .and_then(|brush| globals.set_current_brush_preset(&brush, Some(asset_id)));
                if result.logged_err().is_ok() {
                    self.selected = Some(asset_id);
                }
            }
        }
        Task::none()
    }
}
