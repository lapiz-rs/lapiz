use lapiz_assets::asset::AssetHandle;
use lapiz_shader_graph::graph::function::{GraphFunction, GraphFunctionId};

use crate::asset::BrushPreset;

/// List entry representing a selectable [`BrushPreset`] asset in UI lists.
pub struct BrushPresetListItem {
    pub brush: AssetHandle<BrushPreset>,
    pub name: String,
    pub selected: bool,
}

impl BrushPresetListItem {
    pub fn new(brush: AssetHandle<BrushPreset>) -> Option<Self> {
        let name = brush.get().ok()?.metadata.name.clone();
        Some(Self {
            brush,
            name,
            selected: false,
        })
    }
}

/// Selection and list model managing a collection of [`BrushPresetListItem`]s.
pub struct BrushPresetListDelegate {
    items: Vec<BrushPresetListItem>,
    selected: Option<usize>,
}

impl BrushPresetListDelegate {
    pub fn new(brushes: Vec<AssetHandle<BrushPreset>>) -> Self {
        Self {
            items: brushes
                .into_iter()
                .filter_map(BrushPresetListItem::new)
                .collect(),
            selected: None,
        }
    }

    pub fn get(&self, index: usize) -> Option<&BrushPresetListItem> {
        self.items.get(index)
    }

    pub fn items(&self) -> &[BrushPresetListItem] {
        &self.items
    }

    pub fn select(&mut self, index: usize) {
        assert!(index < self.items.len(), "Brush index should exist");
        if let Some(previous) = self.selected {
            self.items[previous].selected = false;
        }
        self.items[index].selected = true;
        self.selected = Some(index);
    }

    pub fn push(&mut self, brush: AssetHandle<BrushPreset>) -> usize {
        if let Some(item) = BrushPresetListItem::new(brush) {
            self.items.push(item);
        }
        self.items.len().saturating_sub(1)
    }
}

/// List entry representing a selectable shader [`GraphFunction`] in UI lists.
pub struct BrushFunctionItem {
    pub id: GraphFunctionId,
    pub name: String,
    pub selected: bool,
}

impl BrushFunctionItem {
    pub fn new(id: GraphFunctionId, name: String) -> Self {
        Self {
            id,
            name,
            selected: false,
        }
    }
}

/// Selection and list model managing a collection of [`BrushFunctionItem`]s.
pub struct BrushFunctionListDelegate {
    items: Vec<BrushFunctionItem>,
    selected: Option<usize>,
}

impl BrushFunctionListDelegate {
    pub fn new<'a>(functions: impl IntoIterator<Item = &'a GraphFunction>) -> Self {
        Self {
            items: functions
                .into_iter()
                .map(|function| BrushFunctionItem::new(function.id, function.name.clone()))
                .collect(),
            selected: None,
        }
    }

    pub fn get(&self, index: usize) -> Option<&BrushFunctionItem> {
        self.items.get(index)
    }

    pub fn items(&self) -> &[BrushFunctionItem] {
        &self.items
    }

    pub fn select(&mut self, index: usize) {
        assert!(index < self.items.len(), "Function index should exist");
        if let Some(previous) = self.selected {
            self.items[previous].selected = false;
        }
        self.items[index].selected = true;
        self.selected = Some(index);
    }

    pub fn push(&mut self, function: &GraphFunction) -> usize {
        self.items
            .push(BrushFunctionItem::new(function.id, function.name.clone()));
        self.items.len() - 1
    }
}
