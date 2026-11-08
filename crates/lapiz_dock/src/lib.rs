use std::{
    any::Any,
    collections::HashMap,
    fmt, iter,
    time::{Duration, Instant},
};

use dock::{DockAction, DockId, TabEvent};
use group::DockGroupData;
use iced_core::{Element, Length, Point, Size, Theme, Vector, pointer, widget::Void, window};
use iced_futures::{Subscription, event::listen_with};
use iced_runtime::{
    Task,
    window::{close, drag, open, raw_id},
};
use lapiz_runtime::{
    Renderer,
    global::{Global, Globals},
    platform,
};
use lapiz_widgets::{column, context_menu, menu, pane_grid, space, stack, tab_bar};
use state::DockState;

use crate::{
    dock::{Dock, ErasedDock, PaneEvent, PaneHintOverlay, WindowHintOverlay},
    group::{DockGroupId, GroupWindowInfo},
};

pub mod dock;
pub mod group;
pub mod layout;
pub mod state;

const ATTACH_DWELL: Duration = Duration::from_millis(200);
const MERGE_DISTANCE: f32 = 30.0;
const PANE_SPACING: f32 = 2.0;
const PANE_MIN_SIZE: f32 = 50.0;

#[derive(Default)]
pub struct DockRegistry {
    inner: HashMap<DockId, Box<dyn ErasedDock>>,
}

impl DockRegistry {
    pub fn register<T: Dock>(&mut self, dock: T) {
        self.inner.insert(dock.id(), Box::new(dock));
    }

    pub fn build(self, main_window: window::Id) -> (DockManager, Task<DockMessage>) {
        let (mut manager, task) = DockManager::new(main_window);

        for dock in self.inner.into_values() {
            manager.register_dock_boxed(dock);
        }

        (manager, task)
    }
}

impl Global for DockRegistry {}

pub struct DockManager {
    main_window: GroupWindowInfo,
    main_layout_size: Option<Size>,
    detached: HashMap<window::Id, GroupWindowInfo>,
    docks: HashMap<DockId, Box<dyn ErasedDock>>,
    cursor_pos: Option<(window::Id, Point)>,
    sub_windows: HashMap<window::Id, DockId>,
}

impl DockManager {
    fn new(main_window: window::Id) -> (Self, Task<DockMessage>) {
        let this = Self {
            main_window: GroupWindowInfo {
                id: main_window,
                raw_id: None,
                position: Point::ORIGIN,
                size: Size::ZERO,
                layout: DockState::default(),
                dragging_cursor_relative: None,
                last_overlap: None,
            },
            main_layout_size: None,
            docks: HashMap::new(),
            detached: HashMap::new(),
            cursor_pos: None,
            sub_windows: HashMap::new(),
        };

        let task =
            raw_id::<()>(main_window).map(move |raw| DockMessage::RawWindowGet(main_window, raw));

        (this, task)
    }

    pub fn register_dock<T: Dock>(&mut self, dock: T) {
        self.docks.insert(dock.id(), Box::new(dock));
    }

    pub fn registered_docks(&self) -> impl Iterator<Item = &dyn ErasedDock> + '_ {
        self.docks.values().map(Box::as_ref)
    }

    pub fn is_dock_open(&self, id: &DockId) -> bool {
        self.main_window.layout.dock_in_group(id).is_some()
            || self
                .detached
                .values()
                .any(|window| window.layout.dock_in_group(id).is_some())
    }

    fn register_dock_boxed(&mut self, dock: Box<dyn ErasedDock>) {
        self.docks.insert(dock.id(), dock);
    }

    pub fn unregister_dock(&mut self, dock_id: &DockId) -> Task<()> {
        let task = self.close_dock(dock_id);
        self.docks.remove(dock_id);
        self.sub_windows.retain(|_, id| id != dock_id);
        task
    }

    pub fn close_dock(&mut self, dock_id: &DockId) -> Task<()> {
        let panes = &mut self.main_window.layout;
        let empty = panes
            .panes_state_mut()
            .map(|state| {
                state
                    .iter_mut()
                    .filter_map(|(pane, group)| {
                        if group.iter().any(|id| id == dock_id) {
                            group.remove_dock(dock_id);
                            group.is_empty().then_some(*pane)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for pane in empty {
            panes.close(pane);
        }

        let mut windows_to_close = Vec::new();
        self.detached.retain(|id, info| {
            let Some(group) = info.layout.single_group_mut() else {
                return true;
            };
            group.remove_dock(dock_id);
            if group.is_empty() {
                windows_to_close.push(*id);
                false
            } else {
                true
            }
        });
        Task::batch(windows_to_close.into_iter().map(close))
    }

    pub fn open_dock(&mut self, globals: &mut Globals, dock_id: DockId) -> Task<DockMessage> {
        if !self.docks.contains_key(&dock_id) {
            log::warn!("Dock not registered: {}", dock_id);
            return Task::none();
        }
        self.main_window.layout.open(dock_id.clone());

        self.on_open_task(globals, dock_id)
    }

    pub fn open_dock_in_group(
        &mut self,
        globals: &mut Globals,
        dock_id: DockId,
        target: &DockGroupId,
    ) -> Task<DockMessage> {
        if !self.docks.contains_key(&dock_id) {
            log::warn!("Dock not registered: {}", dock_id);
            return Task::none();
        }
        let opened = iter::once(&mut self.main_window)
            .chain(self.detached.values_mut())
            .any(|window| {
                window
                    .layout
                    .open_in_group(target, dock_id.clone())
                    .is_some()
            });
        if opened {
            self.on_open_task(globals, dock_id)
        } else {
            self.open_dock(globals, dock_id)
        }
    }

    fn on_open_task(&mut self, globals: &mut Globals, dock_id: DockId) -> Task<DockMessage> {
        if let Some(dock) = self.docks.get_mut(&dock_id) {
            dock.on_open(globals)
                .map(move |message| DockMessage::Dock(dock_id.clone(), message))
        } else {
            Task::none()
        }
    }

    pub fn open_dock_split(
        &mut self,
        globals: &mut Globals,
        dock_id: DockId,
        target: &DockGroupId,
        edge: pane_grid::Edge,
        ratio: f32,
    ) -> Task<DockMessage> {
        if !self.docks.contains_key(&dock_id) {
            log::warn!("Dock not registered: {}", dock_id);
            return Task::none();
        }
        if self
            .main_window
            .layout
            .open_split(target, edge, ratio, dock_id.clone())
            .is_none()
        {
            return self.open_dock(globals, dock_id);
        }

        self.on_open_task(globals, dock_id)
    }

    fn on_dock_action(&mut self, globals: &mut Globals, action: DockAction) -> Task<DockMessage> {
        if matches!(action, DockAction::Tab(_, TabEvent::Detach(_)))
            && self.screen_cursor_pos().is_none()
        {
            return Task::none();
        }
        match action {
            DockAction::Pane(event) => self.main_window.layout.update(event),
            DockAction::Tab(pane, tab_event) => {
                let Some(pane_state) = self.main_window.layout.panes_state_mut() else {
                    return Task::none();
                };

                match tab_event {
                    TabEvent::Select(dock_id) => {
                        if let Some(group) = pane_state.get_mut(pane) {
                            group.set_active(dock_id);
                        }
                    }
                    TabEvent::Close(dock_id) => {
                        if let Some(group) = pane_state.get_mut(pane) {
                            group.remove_dock(&dock_id);
                            if group.is_empty() {
                                self.main_window.layout.close(pane);
                            }
                        }

                        if let Some(dock) = self.docks.get_mut(&dock_id) {
                            return dock
                                .on_close(globals)
                                .map(move |m| DockMessage::Dock(dock_id.clone(), m));
                        }
                    }
                    TabEvent::Reorder { from, to } => {
                        if let Some(group) = pane_state.get_mut(pane) {
                            let dock_id = group.iter().nth(from).cloned();
                            if let Some(dock_id) = dock_id {
                                group.reorder(dock_id, to);
                            }
                        }
                    }
                    TabEvent::Detach(dock_id) => {
                        if let Some(group) = pane_state.get_mut(pane) {
                            group.remove_dock(&dock_id);
                            if group.is_empty() {
                                self.main_window.layout.close(pane);
                            }

                            return match self.detach_group(DockGroupData::new(dock_id)) {
                                Some((_, task)) => {
                                    task.map(|m| DockMessage::RawWindowGet(m.0, m.1))
                                }
                                None => Task::none(),
                            };
                        }
                    }
                    TabEvent::TitleBarDrag => {
                        return self.detach(pane);
                    }
                    TabEvent::CloseGroup => {
                        let Some(group) = self.main_window.layout.close(pane) else {
                            log::error!(
                                "Failed to close pane, the pane cannot be found: {:?}",
                                pane
                            );
                            return Task::none();
                        };
                        let mut tasks = Vec::with_capacity(group.len());
                        for dock_id in group.iter() {
                            let dock_id = dock_id.clone();
                            let Some(dock) = self.docks.get_mut(&dock_id) else {
                                continue;
                            };

                            let task = dock
                                .on_close(globals)
                                .map(move |m| DockMessage::Dock(dock_id.clone(), m));
                            tasks.push(task);
                        }

                        return Task::batch(tasks);
                    }
                }
            }
        }

        Task::none()
    }

    fn on_cursor_moved(&mut self, window: window::Id, pos: Point) {
        self.cursor_pos = Some((window, pos));
    }

    fn on_float_window_drag_end(&mut self) -> Task<DockMessage> {
        let mut try_attach_or_merge = None;
        for (id, info) in &mut self.detached {
            if info.dragging_cursor_relative.is_none() {
                continue;
            }

            info.dragging_cursor_relative = None;
            try_attach_or_merge = Some((*id, info.last_overlap.take()));
        }

        let Some((src_window, Some((overlap_dst_window, overlap_since, _)))) = try_attach_or_merge
        else {
            return Task::done(DockMessage::RedrawRequested);
        };

        if overlap_since.elapsed() > ATTACH_DWELL {
            if overlap_dst_window == self.main_window.id {
                return self.attach_to_main(src_window).discard();
            } else {
                return self
                    .merge_floating(src_window, overlap_dst_window)
                    .discard();
            }
        }

        Task::none()
    }

    fn on_window_event(&mut self, id: window::Id, event: window::Event) {
        match event {
            window::Event::Opened { position, size, .. } => {
                if id == self.main_window.id {
                    self.main_window.position = position.unwrap_or(Point::ORIGIN);
                    self.main_window.size = size;
                }
            }
            window::Event::Moved(pos) => {
                if id == self.main_window.id {
                    self.main_window.position = pos;
                } else if let Some(info) = self.detached.get_mut(&id) {
                    info.position = pos;

                    if info
                        .last_overlap
                        .is_none_or(|(_, _, p)| p.distance(pos) > 10.0)
                    {
                        let mut next_dst = None;
                        if let Some(dst_id) = self.floating_merge_info(id) {
                            next_dst = Some(dst_id);
                        } else if self.is_over_main_window(id) {
                            next_dst.get_or_insert(self.main_window.id);
                        }

                        self.detached.get_mut(&id).unwrap().last_overlap =
                            next_dst.map(|dst| (dst, Instant::now(), pos));
                    }
                }
            }
            window::Event::Resized(size) => {
                if id == self.main_window.id {
                    self.main_window.size = size;
                } else if let Some(info) = self.detached.get_mut(&id) {
                    info.size = size;
                }
            }
            window::Event::Closed => {
                self.detached.remove(&id);
            }
            _ => {}
        }
    }

    fn on_float_action(
        &mut self,
        id: window::Id,
        tab_event: TabEvent,
        globals: &mut Globals,
    ) -> Task<DockMessage> {
        if matches!(tab_event, TabEvent::Detach(_)) && self.screen_cursor_pos().is_none() {
            return Task::none();
        }
        let Some(info) = self.detached.get_mut(&id) else {
            return Task::none();
        };
        let Some(group) = info.layout.single_group_mut() else {
            return Task::none();
        };

        match tab_event {
            TabEvent::Select(dock_id) => {
                group.set_active(dock_id);
            }
            TabEvent::Close(dock_id) => {
                group.remove_dock(&dock_id);
                let close_window = if group.is_empty() {
                    self.detached.remove(&id);
                    close::<()>(id).discard()
                } else {
                    Task::none()
                };
                let close_dock = self
                    .docks
                    .get_mut(&dock_id)
                    .map_or_else(Task::none, |dock| {
                        dock.on_close(globals)
                            .map(move |m| DockMessage::Dock(dock_id.clone(), m))
                    });
                return Task::batch([close_window, close_dock]);
            }
            TabEvent::Reorder { from, to } => {
                let dock_id = group.iter().nth(from).cloned();
                if let Some(d) = dock_id {
                    group.reorder(d, to);
                }
            }
            TabEvent::Detach(dock_id) => {
                if group.len() == 1 {
                    // Equivalent to dragging the window
                    let Some(cursor_pos) = self.screen_cursor_pos() else {
                        return Task::none();
                    };

                    let info = self.detached.get_mut(&id).unwrap();
                    info.dragging_cursor_relative = Some(Vector::new(
                        cursor_pos.x - info.position.x,
                        cursor_pos.y - info.position.y,
                    ));
                    return drag(id);
                } else {
                    group.remove_dock(&dock_id);
                    return match self.detach_group(DockGroupData::new(dock_id)) {
                        Some((_, task)) => task.map(|m| DockMessage::RawWindowGet(m.0, m.1)),
                        None => Task::none(),
                    };
                }
            }
            TabEvent::TitleBarDrag => {
                let Some(cursor_pos) = self.screen_cursor_pos() else {
                    return Task::none();
                };

                let info = self.detached.get_mut(&id).unwrap();
                info.dragging_cursor_relative = Some(Vector::new(
                    cursor_pos.x - info.position.x,
                    cursor_pos.y - info.position.y,
                ));
                return drag(id);
            }
            TabEvent::CloseGroup => {
                let dock_ids = group.iter().cloned().collect::<Vec<_>>();
                self.detached.remove(&id);
                let tasks = dock_ids.into_iter().filter_map(|dock_id| {
                    let dock = self.docks.get_mut(&dock_id)?;
                    Some(
                        dock.on_close(globals)
                            .map(move |m| DockMessage::Dock(dock_id.clone(), m)),
                    )
                });
                return Task::batch(tasks).chain(close::<()>(id).discard());
            }
        }

        Task::none()
    }

    fn attach_to_main(&mut self, id: window::Id) -> Task<()> {
        let Some(attach) = self.main_attach_info(id) else {
            return Task::none();
        };

        let Some(group) = self
            .detached
            .get(&id)
            .and_then(|info| info.layout.single_group())
            .cloned()
        else {
            return Task::none();
        };
        let attached = match attach {
            AttachInfo::Split { pane, result_edge } => self
                .main_window
                .layout
                .split(pane, result_edge, group)
                .is_some(),
            AttachInfo::Merge { pane } => {
                if let Some(target) = self
                    .main_window
                    .layout
                    .panes_state_mut()
                    .and_then(|st| st.get_mut(pane))
                {
                    target.extend(group);
                    true
                } else {
                    false
                }
            }
            AttachInfo::Initialize => {
                self.main_window.layout.open_group(group);
                true
            }
        };
        if !attached {
            return Task::none();
        }
        self.detached.remove(&id);
        close(id)
    }

    fn merge_floating(&mut self, src: window::Id, dst: window::Id) -> Task<()> {
        if src == dst {
            return Task::none();
        }
        let Some(group) = self
            .detached
            .get(&src)
            .and_then(|info| info.layout.single_group())
            .cloned()
        else {
            return Task::none();
        };
        let Some(target) = self
            .detached
            .get_mut(&dst)
            .and_then(|info| info.layout.single_group_mut())
        else {
            return Task::none();
        };
        target.extend(group);
        self.detached.remove(&src);
        close(src)
    }

    fn detach(&mut self, pane: pane_grid::Pane) -> Task<DockMessage> {
        if self.screen_cursor_pos().is_none() {
            return Task::none();
        }
        let Some(group) = self.main_window.layout.close(pane) else {
            log::error!(
                "Failed to detach pane, the pane cannot be found: {:?}",
                pane
            );
            return Task::none();
        };

        if let Some((_, task)) = self.detach_group(group) {
            task.map(|m| DockMessage::RawWindowGet(m.0, m.1))
        } else {
            log::error!(
                "Failed to detach pane, the window cannot be spawned: {:?}",
                pane
            );
            Task::none()
        }
    }

    fn detach_group(
        &mut self,
        group: DockGroupData,
    ) -> Option<(window::Id, Task<(window::Id, u64)>)> {
        let position = self.screen_cursor_pos()?;
        Some(self.open_detached_group(group, position, Size::new(400.0, 350.0), true))
    }

    fn open_detached_group(
        &mut self,
        group: DockGroupData,
        position: Point,
        size: Size,
        dragging: bool,
    ) -> (window::Id, Task<(window::Id, u64)>) {
        let (window_id, open_task) = open(window::Settings {
            decorations: false,
            position: window::Position::Specific(position),
            size,
            #[cfg(target_os = "windows")]
            platform_specific: window::settings::PlatformSpecific {
                skip_taskbar: true,
                corner_preference: window::settings::platform::CornerPreference::DoNotRound,
                ..Default::default()
            },
            ..Default::default()
        });
        self.detached.insert(
            window_id,
            GroupWindowInfo {
                id: window_id,
                raw_id: None,
                layout: DockState::single(group),
                position,
                size,
                dragging_cursor_relative: dragging.then_some(Vector::ZERO),
                last_overlap: None,
            },
        );

        (
            window_id,
            open_task.then(move |id| raw_id::<()>(id).map(move |raw| (id, raw))),
        )
    }

    fn screen_cursor_pos(&self) -> Option<Point> {
        let (window, cursor) = self.cursor_pos?;

        if window == self.main_window.id {
            Some(Point::new(
                self.main_window.position.x + cursor.x,
                self.main_window.position.y + cursor.y,
            ))
        } else {
            self.detached
                .get(&window)
                .map(|info| Point::new(info.position.x + cursor.x, info.position.y + cursor.y))
        }
    }

    fn is_over_main_window(&self, id: window::Id) -> bool {
        if id == self.main_window.id {
            return true;
        }

        if let Some(info) = self.detached.get(&id) {
            return overlaps(
                info.position,
                info.size,
                self.main_window.position,
                self.main_window.size,
            );
        }

        false
    }

    pub fn main_window(&self) -> window::Id {
        self.main_window.id
    }

    fn detached_window(&self, id: window::Id) -> Option<&GroupWindowInfo> {
        self.detached.get(&id)
    }

    pub fn windows(&self) -> impl Iterator<Item = window::Id> + '_ {
        iter::once(self.main_window.id)
            .chain(self.detached.keys().copied())
            .chain(self.sub_windows())
    }

    fn sub_windows(&self) -> impl Iterator<Item = window::Id> {
        self.sub_windows.keys().copied()
    }

    pub fn close(self) -> Task<()> {
        let mut task = Task::none();
        for id in self.detached.keys() {
            task = task.chain(close(*id));
        }
        task.chain(close(self.main_window.id))
    }

    pub fn group_of(&self, dock: &DockId) -> Option<DockGroupId> {
        iter::once(&self.main_window)
            .chain(self.detached.values())
            .find_map(|window| window.layout.dock_in_group(dock).map(|group| *group.id()))
    }

    pub fn contains_group(&self, target: &DockGroupId) -> bool {
        iter::once(&self.main_window)
            .chain(self.detached.values())
            .any(|window| {
                window
                    .layout
                    .panes_state()
                    .is_some_and(|panes| panes.iter().any(|(_, group)| group.id() == target))
            })
    }

    pub fn largest_group(&self) -> Option<DockGroupId> {
        iter::once(&self.main_window)
            .chain(self.detached.values())
            .flat_map(|window| {
                window.layout.panes_state().into_iter().flat_map(|panes| {
                    panes
                        .layout()
                        .pane_regions(
                            PANE_SPACING,
                            PANE_MIN_SIZE,
                            if window.id == self.main_window.id {
                                self.main_layout_size.unwrap_or(window.size)
                            } else {
                                window.size
                            },
                        )
                        .into_iter()
                        .map(|(pane, region)| {
                            let group = panes.get(pane).expect("layout pane must have a group");
                            (*group.id(), region.width * region.height)
                        })
                })
            })
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(group, _)| group)
    }

    fn main_attach_info(&self, window: window::Id) -> Option<AttachInfo> {
        let info = self.detached.get(&window)?;
        let relative_window_pos = Point::new(
            info.position.x + info.size.width / 2.0 - self.main_window.position.x,
            info.position.y + info.size.height / 2.0 - self.main_window.position.y,
        );

        let Some(node) = self.main_window.layout.panes_state().map(|st| st.layout()) else {
            let rel_cx = self.main_window.size.width / 2.0;
            let rel_cy = self.main_window.size.height / 2.0;

            if (relative_window_pos.x - rel_cx).abs() < self.main_window.size.width / 4.0
                && (relative_window_pos.y - rel_cy).abs() < self.main_window.size.height / 4.0
            {
                return Some(AttachInfo::Initialize);
            } else {
                return None;
            }
        };
        let regions = node.pane_regions(PANE_SPACING, 0.0, self.main_window.size);

        regions
            .iter()
            .find(|(_, r)| r.contains(relative_window_pos))
            .map(|(&pane, r)| {
                let cx = r.x + r.width / 2.0;
                let cy = r.y + r.height / 2.0;

                if (relative_window_pos.x - cx).abs() < r.width / 4.0
                    && (relative_window_pos.y - cy).abs() < r.height / 4.0
                {
                    AttachInfo::Merge { pane }
                } else {
                    let edge = if (relative_window_pos.y - cy).abs()
                        > (relative_window_pos.x - cx).abs()
                    {
                        if relative_window_pos.y < cy {
                            pane_grid::Edge::Top
                        } else {
                            pane_grid::Edge::Bottom
                        }
                    } else {
                        if relative_window_pos.x < cx {
                            pane_grid::Edge::Left
                        } else {
                            pane_grid::Edge::Right
                        }
                    };

                    AttachInfo::Split {
                        pane,
                        result_edge: edge,
                    }
                }
            })
    }

    fn floating_merge_info(&self, src_window: window::Id) -> Option<window::Id> {
        let info = self.detached_window(src_window)?;
        let src_center = Point::new(
            info.position.x + info.size.width / 2.0,
            info.position.y + info.size.height / 2.0,
        );

        for (dst_id, dst_window) in &self.detached {
            if *dst_id == src_window {
                continue;
            }

            let dst_center = Point::new(
                dst_window.position.x + dst_window.size.width / 2.0,
                dst_window.position.y + dst_window.size.height / 2.0,
            );

            if src_center.distance(dst_center) < MERGE_DISTANCE {
                return Some(*dst_id);
            }
        }

        None
    }

    fn current_attach_or_merge_info(&self) -> Option<AttachOrMergeInfo> {
        let dragging = self
            .detached
            .values()
            .find(|info| info.dragging_cursor_relative.is_some())?;

        let src_id = dragging.id;
        let dst_id = dragging.last_overlap?.0;
        if dst_id == self.main_window.id {
            self.main_attach_info(src_id).map(AttachOrMergeInfo::Attach)
        } else {
            Some(AttachOrMergeInfo::Merge { dst: dst_id })
        }
    }

    pub fn view<'a>(
        &'a self,
        window_id: window::Id,
        globals: &'a Globals,
    ) -> Option<Element<'a, DockMessage, Theme, Renderer>> {
        if window_id == self.main_window.id {
            let grid = self.main_window.layout.panes_state().map_or_else(
                || Element::new(Void),
                |panes| {
                    pane_grid::PaneGrid::new(panes, move |pane, group, _| {
                        let body = group.active().map_or_else(
                            || Element::new(space()),
                            |id| {
                                self.docks[id].view(window_id, globals).map({
                                    let id = id.clone();
                                    move |m| DockMessage::Dock(id.clone(), m)
                                })
                            },
                        );

                        let ids = group.iter().cloned().collect::<Vec<_>>();
                        let selected = group
                            .active()
                            .and_then(|id| ids.iter().position(|item| item == id))
                            .unwrap_or(0);
                        let select_ids = ids.clone();
                        let detach_ids = ids.clone();

                        let tabs =
                            tab_bar(ids.iter().map(|id| self.docks[id].display_name()), selected)
                                .on_select(move |index| TabEvent::Select(select_ids[index].clone()))
                                .on_reorder(|from, to| TabEvent::Reorder { from, to })
                                .on_detach(move |index| TabEvent::Detach(detach_ids[index].clone()))
                                .on_title_drag(|| TabEvent::TitleBarDrag);

                        let Some(active) = group.active() else {
                            return pane_grid::Content::new(Element::new(space()));
                        };

                        let tabs = context_menu(
                            tabs,
                            menu()
                                .item("Close Active", TabEvent::Close(active.clone()))
                                .item("Close Group", TabEvent::CloseGroup),
                        );
                        pane_grid::Content::new(body).title_bar(pane_grid::TitleBar::new(
                            Element::new(tabs)
                                .map(move |event| DockMessage::Main(DockAction::Tab(pane, event))),
                        ))
                    })
                    .on_click(|pane| DockMessage::Main(DockAction::Pane(PaneEvent::Clicked(pane))))
                    .on_resize(5.0, |event| {
                        DockMessage::Main(DockAction::Pane(PaneEvent::Resized(event)))
                    })
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .spacing(PANE_SPACING)
                    .min_size(PANE_MIN_SIZE)
                    .into()
                },
            );

            if let Some(AttachOrMergeInfo::Attach(attach_info)) =
                self.current_attach_or_merge_info()
            {
                Some(
                    stack![
                        grid,
                        Element::new(PaneHintOverlay {
                            state: &self.main_window.layout,
                            attach_info,
                            spacing: PANE_SPACING,
                        })
                    ]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                )
            } else {
                Some(grid)
            }
        } else if let Some(info) = self.detached_window(window_id) {
            let group = info.layout.single_group()?;
            let ids = group.iter().cloned().collect::<Vec<_>>();
            let selected = group
                .active()
                .and_then(|id| ids.iter().position(|item| item == id))
                .unwrap_or(0);
            let select_ids = ids.clone();
            let detach_ids = ids.clone();

            let tabs = tab_bar(ids.iter().map(|id| self.docks[id].display_name()), selected)
                .on_select(move |index| TabEvent::Select(select_ids[index].clone()))
                .on_reorder(|from, to| TabEvent::Reorder { from, to })
                .on_detach(move |index| TabEvent::Detach(detach_ids[index].clone()))
                .on_title_drag(|| TabEvent::TitleBarDrag);
            let active = group.active()?;
            let tabs = context_menu(
                tabs,
                menu()
                    .item("Close Active", TabEvent::Close(active.clone()))
                    .item("Close Group", TabEvent::CloseGroup),
            );
            let body = self.docks[active].view(window_id, globals).map({
                let active = active.clone();
                move |m| DockMessage::Dock(active.clone(), m)
            });
            let content = Element::from(
                column![
                    Element::new(tabs).map(move |action| DockMessage::Float {
                        id: window_id,
                        action,
                    }),
                    body
                ]
                .width(Length::Fill)
                .height(Length::Fill),
            );
            if matches!(self.current_attach_or_merge_info(), Some(AttachOrMergeInfo::Merge { dst }) if dst == window_id)
            {
                Some(stack![Element::new(WindowHintOverlay), content].into())
            } else {
                Some(content)
            }
        } else if let Some(dock_id) = self.sub_windows.get(&window_id)
            && let Some(dock) = self.docks.get(dock_id)
        {
            Some(
                dock.view(window_id, globals)
                    .map(move |m| DockMessage::Dock(dock_id.clone(), m)),
            )
        } else {
            None
        }
    }

    pub fn update(&mut self, action: DockMessage, globals: &mut Globals) -> Task<DockMessage> {
        let task = match action {
            DockMessage::Main(dock_action) => self.on_dock_action(globals, dock_action),
            DockMessage::Float { id, action } => self.on_float_action(id, action, globals),
            DockMessage::Dock(dock_id, msg) => {
                if let Some(dock) = self.docks.get_mut(&dock_id) {
                    dock.update(msg, globals)
                        .map(move |m| DockMessage::Dock(dock_id.clone(), m))
                } else {
                    Task::none()
                }
            }
            DockMessage::RawWindowGet(id, raw_id) => {
                if id == self.main_window.id {
                    self.main_window.raw_id = Some(raw_id);
                    for info in self.detached.values() {
                        if let Some(detached_raw_id) = info.raw_id {
                            platform::set_window_parent(raw_id, detached_raw_id);
                        }
                    }
                    Task::none()
                } else if let Some(info) = self.detached.get_mut(&id) {
                    info.raw_id = Some(raw_id);
                    platform::disable_window_snap(raw_id);

                    if let Some(main_raw_id) = self.main_window.raw_id {
                        platform::set_window_parent(main_raw_id, raw_id);
                    }

                    if info.dragging_cursor_relative.is_some() {
                        drag(id)
                    } else {
                        Task::none()
                    }
                } else {
                    Task::none()
                }
            }
            DockMessage::MainLayoutResized(size) => {
                self.main_layout_size = Some(size);
                return Task::none();
            }
            DockMessage::WindowEvent(id, event) => {
                self.on_window_event(id, event);
                return Task::none();
            }
            DockMessage::CursorMoved(id, position) => {
                self.on_cursor_moved(id, position);
                return Task::none();
            }
            DockMessage::PointerReleased => return self.on_float_window_drag_end(),
            DockMessage::RedrawRequested => return Task::none(),
        };

        self.sub_windows.clear();
        for (dock_id, dock) in &self.docks {
            for sub_window in dock.sub_windows() {
                self.sub_windows.insert(sub_window, dock_id.clone());
            }
        }

        task
    }

    pub fn subscription(&self, globals: &Globals) -> Subscription<DockMessage> {
        let events = listen_with(|event, _status, window| match event {
            iced_core::Event::Window(event) => Some(DockMessage::WindowEvent(window, event)),
            iced_core::Event::Pointer(pointer::Event::PointerMoved { position, .. }) => {
                Some(DockMessage::CursorMoved(window, position))
            }
            iced_core::Event::Pointer(event) if event.is_primary_release() => {
                Some(DockMessage::PointerReleased)
            }
            _ => None,
        });

        Subscription::batch(
            iter::once(events).chain(self.docks.iter().map(|(id, dock)| {
                dock.subscription(globals)
                    .with(id.clone())
                    .map(|(dock, message)| DockMessage::Dock(dock, message))
            })),
        )
    }
}

fn overlaps(pos_a: Point, size_a: Size, pos_b: Point, size_b: Size) -> bool {
    pos_a.x < pos_b.x + size_b.width
        && pos_a.x + size_a.width > pos_b.x
        && pos_a.y < pos_b.y + size_b.height
        && pos_a.y + size_a.height > pos_b.y
}

pub enum DockMessage {
    Main(DockAction),
    Float { id: window::Id, action: TabEvent },
    Dock(DockId, Box<dyn Any + Send>),
    RawWindowGet(window::Id, u64),
    MainLayoutResized(Size),
    WindowEvent(window::Id, window::Event),
    CursorMoved(window::Id, Point),
    PointerReleased,
    RedrawRequested,
}

impl fmt::Debug for DockMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Main(arg0) => f.debug_tuple("Main").field(arg0).finish(),
            Self::Float { id, action } => f
                .debug_struct("Float")
                .field("id", id)
                .field("action", action)
                .finish(),
            Self::Dock(arg0, _) => f.debug_tuple("Dock").field(arg0).finish(),
            Self::RawWindowGet(id, raw_id) => f
                .debug_struct("RawWindowGet")
                .field("id", id)
                .field("raw_id", raw_id)
                .finish(),
            Self::MainLayoutResized(size) => {
                f.debug_tuple("MainLayoutResized").field(size).finish()
            }
            Self::WindowEvent(id, event) => {
                f.debug_tuple("WindowEvent").field(id).field(event).finish()
            }
            Self::CursorMoved(id, point) => {
                f.debug_tuple("CursorMoved").field(id).field(point).finish()
            }
            Self::PointerReleased => f.debug_tuple("PointerReleased").finish(),
            Self::RedrawRequested => f.debug_tuple("RedrawRequested").finish(),
        }
    }
}

#[derive(Debug, Clone)]
enum AttachOrMergeInfo {
    Attach(AttachInfo),
    Merge { dst: window::Id },
}

#[derive(Debug, Clone)]
pub(crate) enum AttachInfo {
    Split {
        pane: pane_grid::Pane,
        result_edge: pane_grid::Edge,
    },
    Merge {
        pane: pane_grid::Pane,
    },
    Initialize,
}
