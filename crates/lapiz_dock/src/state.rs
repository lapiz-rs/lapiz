use lapiz_widgets::pane_grid;

use crate::{
    dock::{DockId, PaneEvent},
    group::{DockGroupData, DockGroupId},
};

/// Manages the [`pane_grid::State`] of [`DockGroupData`] tab groups within a dock window.
///
/// The main window uses a `DockState` with splits enabled (`allow_splits: true`), whereas detached
/// floating windows use [`DockState::single`] with splits disabled so each floating window hosts a single tab group.
#[derive(Debug)]
pub struct DockState {
    panes: Option<pane_grid::State<DockGroupData>>,
    allow_splits: bool,
}

impl Default for DockState {
    fn default() -> Self {
        Self {
            panes: None,
            allow_splits: true,
        }
    }
}

impl DockState {
    pub fn from_configuration(config: pane_grid::Configuration<DockGroupData>) -> Self {
        Self {
            panes: Some(pane_grid::State::with_configuration(config)),
            allow_splits: true,
        }
    }

    pub fn single(group: DockGroupData) -> Self {
        Self {
            panes: Some(pane_grid::State::new(group).0),
            allow_splits: false,
        }
    }

    pub fn single_group(&self) -> Option<&DockGroupData> {
        let panes = self.panes.as_ref()?;
        if panes.len() != 1 {
            return None;
        }
        panes.iter().next().map(|(_, group)| group)
    }

    pub fn single_group_mut(&mut self) -> Option<&mut DockGroupData> {
        let panes = self.panes.as_mut()?;
        if panes.len() != 1 {
            return None;
        }
        panes.iter_mut().next().map(|(_, group)| group)
    }

    pub fn open(&mut self, dock: DockId) -> pane_grid::Pane {
        if let Some(state) = self.panes.as_mut() {
            let (pane, group) = state.iter_mut().next().unwrap();
            group.add_dock(dock.clone());
            *pane
        } else {
            let group = DockGroupData::new(dock);
            let (state, pane) = pane_grid::State::new(group);
            self.panes = Some(state);
            pane
        }
    }

    pub fn open_in_group(&mut self, target: &DockGroupId, dock: DockId) -> Option<pane_grid::Pane> {
        let panes = self.panes.as_mut()?;
        let (pane, group) = panes.iter_mut().find(|(_, group)| group.id() == target)?;
        group.add_dock(dock.clone());
        group.set_active(dock);
        Some(*pane)
    }

    pub fn open_split(
        &mut self,
        target: &DockGroupId,
        result_edge: pane_grid::Edge,
        ratio: f32,
        dock: DockId,
    ) -> Option<pane_grid::Pane> {
        if !self.allow_splits {
            return None;
        }

        let panes = self.panes.as_mut()?;
        let target_pane = panes
            .iter()
            .find(|(_, group)| group.id() == target)
            .map(|(pane, _)| *pane)?;
        let group = DockGroupData::new(dock.clone());
        let axis = match result_edge {
            pane_grid::Edge::Top | pane_grid::Edge::Bottom => pane_grid::Axis::Horizontal,
            pane_grid::Edge::Left | pane_grid::Edge::Right => pane_grid::Axis::Vertical,
        };
        let (new_pane, split) = panes.split(axis, target_pane, group)?;
        if matches!(result_edge, pane_grid::Edge::Top | pane_grid::Edge::Left) {
            panes.swap(target_pane, new_pane);
        }
        panes.resize(split, ratio.clamp(0.1, 0.9));
        Some(new_pane)
    }

    pub fn open_group(&mut self, group: DockGroupData) -> pane_grid::Pane {
        if let Some(state) = self.panes.as_mut() {
            let (pane, target) = state.iter_mut().next().unwrap();
            target.extend(group);
            *pane
        } else {
            let (state, pane) = pane_grid::State::new(group);
            self.panes = Some(state);
            pane
        }
    }

    pub fn split(
        &mut self,
        pane: pane_grid::Pane,
        result_edge: pane_grid::Edge,
        new_group: DockGroupData,
    ) -> Option<pane_grid::Pane> {
        if !self.allow_splits {
            return None;
        }

        let panes = self.panes.as_mut()?;
        let (new_pane, _) = panes.split(
            match result_edge {
                pane_grid::Edge::Top => pane_grid::Axis::Horizontal,
                pane_grid::Edge::Bottom => pane_grid::Axis::Horizontal,
                pane_grid::Edge::Left => pane_grid::Axis::Vertical,
                pane_grid::Edge::Right => pane_grid::Axis::Vertical,
            },
            pane,
            new_group,
        )?;

        match result_edge {
            pane_grid::Edge::Top | pane_grid::Edge::Left => {
                panes.swap(pane, new_pane);
            }
            pane_grid::Edge::Bottom | pane_grid::Edge::Right => {}
        };

        Some(pane)
    }

    pub fn close(&mut self, pane: pane_grid::Pane) -> Option<DockGroupData> {
        let panes = self.panes.as_mut()?;
        if let Some((state, _)) = panes.close(pane) {
            Some(state)
        } else if panes.len() == 1 {
            let state = panes.panes.remove(&pane);
            self.panes = None;
            state
        } else {
            None
        }
    }

    pub fn update(&mut self, action: PaneEvent) {
        match action {
            PaneEvent::Clicked(_) => {}
            PaneEvent::Resized(event) => {
                if let Some(panes) = self.panes.as_mut() {
                    panes.resize(event.split, event.ratio);
                }
            }
        }
    }

    pub fn panes_state(&self) -> Option<&pane_grid::State<DockGroupData>> {
        self.panes.as_ref()
    }

    pub fn panes_state_mut(&mut self) -> Option<&mut pane_grid::State<DockGroupData>> {
        self.panes.as_mut()
    }

    pub fn dock_in_group(&self, dock: &DockId) -> Option<&DockGroupData> {
        let panes = self.panes.as_ref()?;
        let (_, group) = panes
            .iter()
            .find(|(_, group)| group.iter().any(|id| dock == id))?;
        Some(group)
    }
}
