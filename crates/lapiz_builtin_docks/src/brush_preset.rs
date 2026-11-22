use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
};

use anyhow::Result;
use iced_core::{Color, Element, Length, Theme, text::IntoFragment, window};
use iced_runtime::Task;
use lapiz_assets::{AssetAppExt as _, asset::UntypedAssetId, store::AssetRegistry, tag::TagId};
use lapiz_brush::{
    engine::{BrushEngineId, BrushEngineRegistryExt as _, UntypedBrushPresetEntry},
    tool::CurrentBrushPresetExt as _,
    widget::UntypedBrushPreview,
};
use lapiz_dock::dock::{Dock, DockId};
use lapiz_i18n::t;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{
    button, column, container, icon, icon_button, label, row, scrollable, space,
    tree::{Tree, TreeNode},
};

pub static BRUSH_PRESETS_DOCK_ID: LazyLock<DockId> =
    LazyLock::new(|| DockId::new("brush_presets_dock".into()));

#[derive(Default)]
pub struct BrushPresetDock {
    selected: Option<UntypedAssetId>,
    brushes: Vec<(BrushEngineId, UntypedBrushPresetEntry)>,
    groups: BrushGroups,
    collapsed: HashSet<BrushGroupId>,
    preview_generation: u64,
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub enum BrushGroupId {
    Tag(Vec<TagId>),
    Untagged,
}

#[derive(Clone, Eq, Hash, PartialEq)]
enum BrushRowId {
    Group(BrushGroupId),
    Preset(BrushGroupId, UntypedAssetId),
}

#[derive(Default)]
struct BrushGroups {
    tags: Vec<BrushTagGroup>,
    untagged: Vec<usize>,
}

struct BrushTagGroup {
    tag: TagId,
    name: String,
    children: Vec<BrushTagGroup>,
    brushes: Vec<usize>,
}

#[derive(Clone)]
pub enum BrushPresetDockMessage {
    SelectBrush(UntypedAssetId),
    ToggleGroup(BrushGroupId),
    Refresh,
}

impl BrushGroups {
    fn read(
        assets: &AssetRegistry,
        brushes: &[(BrushEngineId, UntypedBrushPresetEntry)],
    ) -> Result<Self> {
        let db = assets.index_db();
        let tags = assets
            .all_tags()?
            .into_iter()
            .filter(|tag| !tag.is_type_tag())
            .collect::<Vec<_>>();
        let names = tags
            .iter()
            .map(|tag| (tag.id, tag.name.as_str()))
            .collect::<HashMap<_, _>>();
        let mut members = HashMap::<TagId, Vec<usize>>::new();
        let mut untagged = Vec::new();
        for (index, (_, brush)) in brushes.iter().enumerate() {
            let mut tagged = false;
            for id in db
                .get_asset_tags(&brush.id)?
                .into_iter()
                .filter(|id| names.contains_key(id))
            {
                members.entry(id).or_default().push(index);
                tagged = true;
            }
            if !tagged {
                untagged.push(index);
            }
        }

        let mut children = HashMap::new();
        let mut nested = HashSet::new();
        for tag in &tags {
            let ids = db
                .get_child_tags(&tag.id)?
                .into_iter()
                .filter(|child| names.contains_key(&child.id))
                .map(|child| child.id)
                .collect::<Vec<_>>();
            nested.extend(ids.iter().copied());
            children.insert(tag.id, ids);
        }

        let tags = tags
            .iter()
            .filter(|tag| !nested.contains(&tag.id))
            .filter_map(|tag| BrushTagGroup::from_tag(tag.id, &names, &children, &members))
            .collect();
        Ok(Self { tags, untagged })
    }
}

impl BrushTagGroup {
    fn from_tag(
        tag: TagId,
        names: &HashMap<TagId, &str>,
        children: &HashMap<TagId, Vec<TagId>>,
        members: &HashMap<TagId, Vec<usize>>,
    ) -> Option<Self> {
        let subgroups = children
            .get(&tag)
            .into_iter()
            .flatten()
            .filter_map(|&id| Self::from_tag(id, names, children, members))
            .collect::<Vec<_>>();
        let brushes = members.get(&tag).map_or(&[][..], Vec::as_slice);
        if brushes.is_empty() && subgroups.is_empty() {
            return None;
        }
        Some(Self {
            tag,
            name: names[&tag].to_owned(),
            children: subgroups,
            brushes: brushes.to_vec(),
        })
    }
}

impl BrushPresetDock {
    pub fn new(globals: &Globals) -> Self {
        let mut dock = Self::default();
        dock.refresh(globals).log_err();
        dock
    }

    fn refresh(&mut self, globals: &Globals) -> Result<()> {
        let assets = globals.assets();
        let brushes = globals.brush_engine_registry().all_brushes(assets);
        let groups = BrushGroups::read(assets, &brushes)?;
        self.brushes = brushes;
        self.groups = groups;
        self.preview_generation += 1;
        Ok(())
    }

    fn tag_node<'a>(
        &'a self,
        group: &'a BrushTagGroup,
        parent: &[TagId],
        globals: &'a Globals,
    ) -> TreeNode<'a, BrushPresetDockMessage, BrushRowId> {
        let mut path = parent.to_vec();
        path.push(group.tag);
        let id = BrushGroupId::Tag(path.clone());
        self.group_node(id.clone(), group.name.as_str(), || {
            group
                .children
                .iter()
                .map(|group| self.tag_node(group, &path, globals))
                .chain(
                    group
                        .brushes
                        .iter()
                        .map(|&index| self.brush_node(index, &id, globals)),
                )
                .collect()
        })
    }

    fn group_node<'a>(
        &'a self,
        id: BrushGroupId,
        name: impl IntoFragment<'a>,
        children: impl FnOnce() -> Vec<TreeNode<'a, BrushPresetDockMessage, BrushRowId>>,
    ) -> TreeNode<'a, BrushPresetDockMessage, BrushRowId> {
        let collapsed = self.collapsed.contains(&id);
        let arrow = if collapsed {
            icon::chevron_right()
        } else {
            icon::chevron_down()
        };
        TreeNode::Branch {
            id: BrushRowId::Group(id.clone()),
            label: button(row![arrow.size(12), label(name)].gap(4.0))
                .width(Length::Fill)
                .on_press(BrushPresetDockMessage::ToggleGroup(id))
                .into(),
            children: if collapsed { Vec::new() } else { children() },
        }
    }

    fn brush_node<'a>(
        &'a self,
        index: usize,
        group: &BrushGroupId,
        globals: &'a Globals,
    ) -> TreeNode<'a, BrushPresetDockMessage, BrushRowId> {
        let (engine_id, item) = &self.brushes[index];
        let preview = container(UntypedBrushPreview::new(
            *engine_id,
            item.id,
            self.preview_generation,
            globals,
        ))
        .width(Length::Fill)
        .height(40);

        TreeNode::Leaf(
            BrushRowId::Preset(group.clone(), item.id),
            button(row![label(item.name.as_str()), preview].gap(6))
                .width(Length::Fill)
                .height(Length::Fit)
                .activated(Some(item.id) == self.selected)
                .on_press(BrushPresetDockMessage::SelectBrush(item.id))
                .into(),
        )
    }
}

impl Dock for BrushPresetDock {
    type Message = BrushPresetDockMessage;

    fn id(&self) -> DockId {
        BRUSH_PRESETS_DOCK_ID.clone()
    }

    fn on_open(&mut self, globals: &mut Globals) -> Task<Self::Message> {
        self.refresh(globals).log_err();
        Task::none()
    }

    fn view<'a>(
        &'a self,
        _window_id: window::Id,
        globals: &'a Globals,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        let mut nodes = self
            .groups
            .tags
            .iter()
            .map(|group| self.tag_node(group, &[], globals))
            .collect::<Vec<_>>();
        if !self.groups.untagged.is_empty() {
            nodes.push(self.group_node(
                BrushGroupId::Untagged,
                t!("brush_presets_untagged"),
                || {
                    self.groups
                        .untagged
                        .iter()
                        .map(|&index| self.brush_node(index, &BrushGroupId::Untagged, globals))
                        .collect()
                },
            ));
        }

        column![
            row![
                space().width(Length::Fill),
                icon_button(icon::refresh()).on_press(BrushPresetDockMessage::Refresh)
            ],
            scrollable(Tree::new(nodes).spacing(2.0))
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .gap(4)
        .padding(4)
        .height(Length::Fill)
        .into()
    }

    fn update(&mut self, message: Self::Message, globals: &mut Globals) -> Task<Self::Message> {
        match message {
            BrushPresetDockMessage::Refresh => self.refresh(globals).log_err(),
            BrushPresetDockMessage::ToggleGroup(id) => {
                if !self.collapsed.remove(&id) {
                    self.collapsed.insert(id);
                }
            }
            BrushPresetDockMessage::SelectBrush(id) => {
                let Some((engine_id, _)) = self.brushes.iter().find(|(_, brush)| brush.id == id)
                else {
                    return Task::none();
                };
                let factory = globals
                    .brush_engine_registry()
                    .get(engine_id)
                    .expect("Brush engine not found");
                let assets = globals.assets().clone();
                let Ok(engine) = factory.load_brush_from_assets(id, &assets).logged_err() else {
                    return Task::none();
                };
                let Ok(brush) = engine.compile_brush(globals).logged_err() else {
                    return Task::none();
                };
                if globals
                    .set_current_brush_preset(&brush, Some(id))
                    .logged_err()
                    .is_ok()
                {
                    self.selected = Some(id);
                }
            }
        }
        Task::none()
    }
}
