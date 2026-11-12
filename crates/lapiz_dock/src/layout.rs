use std::collections::HashSet;

use iced_core::{Point, Size};
use iced_runtime::Task;
use lapiz_config::Configuration;
use lapiz_runtime::global::Globals;
use lapiz_widgets::pane_grid;
use serde::{Deserialize, Serialize};

use crate::{DockManager, DockMessage, dock::DockId, group::DockGroupData, state::DockState};

/// Serializable workspace layout configuration capturing the main window's split pane hierarchy and detached floating dock groups.
///
/// `DockLayout` implements [`Configuration`] (`dock_layout.toml`) and is used by [`DockManager::layout_snapshot`]
/// and [`DockManager::restore_layout`] to persist and restore dock arrangements across sessions.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DockLayout {
    main: Option<LayoutNode>,
    detached: Vec<FloatingGroup>,
}

impl Configuration for DockLayout {
    const NAME: &'static str = "dock_layout.toml";
    const DEFAULT: &'static str = include_str!("../../../default_config/dock_layout.toml");
}

/// Recursive node in the serialized main window pane tree.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LayoutNode {
    Split {
        axis: LayoutAxis,
        ratio: f32,
        a: Box<Self>,
        b: Box<Self>,
    },
    Pane {
        docks: Vec<DockId>,
        active: Option<DockId>,
    },
}

/// Split axis orientation for a serialized [`LayoutNode::Split`].
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum LayoutAxis {
    Horizontal,
    Vertical,
}

/// Serialized state and window geometry of a detached floating dock group.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct FloatingGroup {
    docks: Vec<DockId>,
    active: Option<DockId>,
    position: [f32; 2],
    size: [f32; 2],
}

impl LayoutNode {
    fn snapshot(node: &pane_grid::Node, panes: &pane_grid::State<DockGroupData>) -> Self {
        match node {
            pane_grid::Node::Split {
                axis, ratio, a, b, ..
            } => Self::Split {
                axis: match axis {
                    pane_grid::Axis::Horizontal => LayoutAxis::Horizontal,
                    pane_grid::Axis::Vertical => LayoutAxis::Vertical,
                },
                ratio: *ratio,
                a: Box::new(Self::snapshot(a, panes)),
                b: Box::new(Self::snapshot(b, panes)),
            },
            pane_grid::Node::Pane(pane) => {
                let group = panes.get(*pane).expect("layout pane must have a group");
                Self::Pane {
                    docks: group.iter().cloned().collect(),
                    active: group.active().cloned(),
                }
            }
        }
    }

    fn restore(
        &self,
        registered: &HashSet<DockId>,
        seen: &mut HashSet<DockId>,
        opened: &mut Vec<DockId>,
    ) -> Option<pane_grid::Configuration<DockGroupData>> {
        match self {
            Self::Pane { docks, active } => {
                let group = restore_group(docks, active.as_ref(), registered, seen, opened)?;
                Some(pane_grid::Configuration::Pane(group))
            }
            Self::Split { axis, ratio, a, b } => {
                let a = a.restore(registered, seen, opened);
                let b = b.restore(registered, seen, opened);
                match (a, b) {
                    (Some(a), Some(b)) => Some(pane_grid::Configuration::Split {
                        axis: match axis {
                            LayoutAxis::Horizontal => pane_grid::Axis::Horizontal,
                            LayoutAxis::Vertical => pane_grid::Axis::Vertical,
                        },
                        ratio: if ratio.is_finite() {
                            ratio.clamp(0.1, 0.9)
                        } else {
                            0.5
                        },
                        a: Box::new(a),
                        b: Box::new(b),
                    }),
                    (Some(node), None) | (None, Some(node)) => Some(node),
                    (None, None) => None,
                }
            }
        }
    }
}

fn restore_group(
    docks: &[DockId],
    active: Option<&DockId>,
    registered: &HashSet<DockId>,
    seen: &mut HashSet<DockId>,
    opened: &mut Vec<DockId>,
) -> Option<DockGroupData> {
    let mut ids = docks
        .iter()
        .filter(|id| registered.contains(*id) && seen.insert((*id).clone()));

    let first = ids.next()?.clone();
    opened.push(first.clone());

    let mut group = DockGroupData::new(first);
    for id in ids {
        group.add_dock(id.clone());
        opened.push(id.clone());
    }
    if let Some(active) = active {
        group.set_active(active.clone());
    }

    Some(group)
}

impl DockManager {
    pub fn layout_snapshot(&self) -> DockLayout {
        let main = self
            .main_window
            .layout
            .panes_state()
            .map(|panes| LayoutNode::snapshot(panes.layout(), panes));
        let detached = self
            .detached
            .values()
            .filter_map(|info| {
                let group = info.layout.single_group()?;
                Some(FloatingGroup {
                    docks: group.iter().cloned().collect(),
                    active: group.active().cloned(),
                    position: [info.position.x, info.position.y],
                    size: [info.size.width, info.size.height],
                })
            })
            .collect();
        DockLayout { main, detached }
    }

    pub fn restore_layout(
        &mut self,
        layout: &DockLayout,
        globals: &mut Globals,
    ) -> Option<Task<DockMessage>> {
        let registered = self.docks.keys().cloned().collect::<HashSet<_>>();
        let mut seen = HashSet::new();
        let mut opened = Vec::new();
        let main = layout
            .main
            .as_ref()
            .and_then(|node| node.restore(&registered, &mut seen, &mut opened));
        let detached = layout
            .detached
            .iter()
            .filter_map(|floating| {
                let [x, y] = floating.position;
                let [width, height] = floating.size;
                if ![x, y, width, height].iter().all(|v| v.is_finite())
                    || width <= 0.0
                    || height <= 0.0
                {
                    return None;
                }
                let group = restore_group(
                    &floating.docks,
                    floating.active.as_ref(),
                    &registered,
                    &mut seen,
                    &mut opened,
                )?;
                Some((group, Point::new(x, y), Size::new(width, height)))
            })
            .collect::<Vec<_>>();

        if main.is_none() && detached.is_empty() {
            return None;
        }

        if let Some(main) = main {
            self.main_window.layout = DockState::from_configuration(main);
        }
        let mut tasks = Vec::new();
        for (group, position, size) in detached {
            let (_, task) = self.open_detached_group(group, position, size, false);
            tasks.push(task.map(|(id, raw)| DockMessage::RawWindowGet(id, raw)));
        }

        tasks.extend(opened.into_iter().map(|id| self.on_open_task(globals, id)));

        Some(Task::batch(tasks))
    }
}
