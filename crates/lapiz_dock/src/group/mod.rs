use indexmap::IndexSet;
use lapiz_utils::wrapper;
use parse_display::Display;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::dock::DockId;

pub mod tab_row;

wrapper! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
    pub DockGroupId : Uuid
}

#[derive(Debug, Clone, Serialize)]
pub struct DockGroupData {
    id: DockGroupId,
    docks: IndexSet<DockId>,
    active: Option<DockId>,
}

impl DockGroupData {
    pub fn new(dock: DockId) -> Self {
        Self {
            id: DockGroupId::new(Uuid::new_v4()),
            docks: IndexSet::from([dock.clone()]),
            active: Some(dock),
        }
    }

    pub fn empty() -> Self {
        Self {
            id: DockGroupId::new(Uuid::new_v4()),
            docks: IndexSet::default(),
            active: None,
        }
    }

    pub fn add_dock(&mut self, dock_id: DockId) {
        self.docks.insert(dock_id.clone());
        if self.active.is_none() {
            self.active = Some(dock_id);
        }
    }

    pub fn remove_dock(&mut self, dock_id: &DockId) {
        if self.active.as_ref().is_some_and(|active| active == dock_id) {
            let (index, _) = self.docks.shift_remove_full(dock_id).unwrap();
            if self.docks.is_empty() {
                self.active = None;
                return;
            }

            self.active = self.docks.get_index(index.saturating_sub(1)).cloned();
        } else {
            self.docks.shift_remove(dock_id);
        }
    }

    pub fn set_active(&mut self, dock_id: DockId) {
        if self.docks.contains(&dock_id) {
            self.active = Some(dock_id);
        }
    }

    pub fn reorder(&mut self, dock_id: DockId, index: usize) {
        self.docks.shift_insert(index, dock_id);
    }

    pub fn active(&self) -> Option<&DockId> {
        self.active.as_ref()
    }

    pub fn is_empty(&self) -> bool {
        self.docks.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &DockId> {
        self.docks.iter()
    }

    pub fn len(&self) -> usize {
        self.docks.len()
    }

    pub fn extend(&mut self, other: DockGroupData) {
        self.docks.extend(other.docks);
    }

    pub fn id(&self) -> &DockGroupId {
        &self.id
    }
}
