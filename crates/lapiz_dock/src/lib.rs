use std::{
    any::Any,
    collections::HashMap,
    time::{Duration, Instant},
};

use dock::{DockAction, DockId, TabEvent};
use group::DockGroupData;
use iced_core::{Element, Point, Size, Theme, Vector, window};
use iced_futures::Subscription;
use iced_runtime::Task;
use iced_wgpu::Renderer;
use iced_widget::pane_grid;
use lapiz_runtime::Services;
use lapiz_widgets::window_decorations::WindowDecorations;
use state::DockState;

use crate::{
    dock::{Dock, DockWidget, ErasedDock, FloatingDockWidget},
    group::DockGroupId,
};

pub mod dock;
pub mod group;
pub mod state;
pub mod style;

const ATTACH_DWELL: Duration = Duration::from_millis(200);
const MERGE_DISTANCE: f32 = 30.0;
const _FLOATING_WINDOW_SNAP_DISTANCE: f32 = 10.0;

pub struct DockManager {
    main_window: GroupWindowInfo,
    dock_state: DockState,
    detached: HashMap<window::Id, GroupWindowInfo>,
    docks: HashMap<DockId, Box<dyn ErasedDock>>,
    cursor_pos: Option<(window::Id, Point)>,
    sub_windows: HashMap<window::Id, DockId>,
}

impl DockManager {
    pub fn new(main_window: window::Id) -> (Self, Task<DockMessage>) {
        let this = Self {
            main_window: GroupWindowInfo {
                id: main_window,
                raw_id: None,
                window_decorations: WindowDecorations::default(),
                position: Point::ORIGIN,
                size: Size::ZERO,
                group: DockGroupData::empty(),
                dragging_cursor_relative: None,
                last_overlap: None,
            },
            dock_state: DockState::default(),
            docks: HashMap::new(),
            detached: HashMap::new(),
            cursor_pos: None,
            sub_windows: HashMap::new(),
        };

        let task = iced_runtime::window::raw_id::<()>(main_window)
            .map(move |raw| DockMessage::RawWindowGet(main_window, raw));

        (this, task)
    }

    pub fn register_dock<T: Dock>(&mut self, dock: T) {
        self.docks.insert(dock.id(), Box::new(dock));
    }

    pub fn register_dock_boxed(&mut self, dock: Box<dyn ErasedDock>) {
        self.docks.insert(dock.id(), dock);
    }

    pub fn unregister_dock(&mut self, dock_id: &DockId) {
        self.docks.remove(dock_id);
    }

    pub fn open_dock(&mut self, dock_id: DockId) -> Task<DockMessage> {
        self.dock_state.open(dock_id.clone());

        self.on_open_task(dock_id)
    }

    pub fn open_dock_in_group(
        &mut self,
        dock_id: DockId,
        target: &DockGroupId,
    ) -> Task<DockMessage> {
        if self
            .dock_state
            .open_in_group(target, dock_id.clone())
            .is_none()
        {
            return self.open_dock(dock_id);
        }

        self.on_open_task(dock_id)
    }

    fn on_open_task(&mut self, dock_id: DockId) -> Task<DockMessage> {
        if let Some(dock) = self.docks.get_mut(&dock_id) {
            dock.on_open()
                .map(move |message| DockMessage::Dock(dock_id.clone(), message))
        } else {
            Task::none()
        }
    }

    pub fn open_dock_split(
        &mut self,
        dock_id: DockId,
        target: &DockGroupId,
        edge: pane_grid::Edge,
        ratio: f32,
    ) -> Task<DockMessage> {
        if self
            .dock_state
            .open_split(target, edge, ratio, dock_id.clone())
            .is_none()
        {
            return self.open_dock(dock_id);
        }

        self.on_open_task(dock_id)
    }

    pub fn on_dock_action(&mut self, action: DockAction) -> Task<DockMessage> {
        match action {
            DockAction::Pane(event) => self.dock_state.update(event),
            DockAction::Tab(pane, tab_event) => {
                let Some(pane_state) = self.dock_state.panes_state_mut() else {
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
                                pane_state.close(pane);
                            }
                        }

                        if let Some(dock) = self.docks.get_mut(&dock_id) {
                            return dock
                                .on_close()
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
                                pane_state.close(pane);
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
                        let Some((group, _)) = pane_state.close(pane) else {
                            log::error!(
                                "Failed to close pane, the pane cannot be found or it's the last pane: {:?}",
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
                                .on_close()
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

    pub fn on_cursor_moved(&mut self, window: window::Id, pos: Point) -> Task<DockMessage> {
        self.cursor_pos = Some((window, pos));

        // TODO Implement window snapping if possible
        //      Currently, if we are using manually implemented dragging, it may cause problem
        //      when the cursor is moving too fast and it goes into inner widget, then the event is
        //      captured by that widget, and stuck.
        //      Also, the drop target window won't update and show indicator.
        //      So although snapping will work if you uncomment the code below and stop the native
        //      window drag in `TabEvent::TitleBarDrag`, it may cause bad user experience.

        // let Some(cursor_pos) = self.screen_cursor_pos() else {
        //     return Task::none();
        // };

        // for window in self.detached.values() {
        //     let Some(p) = window.dragging_cursor_relative else {
        //         continue;
        //     };

        //     let mut pos = cursor_pos - p;
        //     for another in self.detached.values() {
        //         if another.id == window.id {
        //             continue;
        //         }

        //         pos = snap(pos, window.size, another.position, another.size);
        //     }

        //     return iced_runtime::window::move_to(window.id, pos);
        // }

        Task::none()
    }

    pub fn on_float_window_drag_end(&mut self) -> Task<DockMessage> {
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

    pub fn on_window_event(&mut self, id: window::Id, event: window::Event) -> Task<()> {
        match event {
            window::Event::Opened { position, size } => {
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
        Task::none()
    }

    pub fn on_float_action(&mut self, id: window::Id, tab_event: TabEvent) -> Task<DockMessage> {
        let Some(info) = self.detached.get_mut(&id) else {
            return Task::none();
        };

        match tab_event {
            TabEvent::Select(dock_id) => {
                info.group.set_active(dock_id);
            }
            TabEvent::Close(dock_id) => {
                info.group.remove_dock(&dock_id);
                if info.group.is_empty() {
                    self.detached.remove(&id);
                    return iced_runtime::window::close(id);
                }
            }
            TabEvent::Reorder { from, to } => {
                let dock_id = info.group.iter().nth(from);
                if let Some(d) = dock_id {
                    info.group.reorder(d.clone(), to);
                }
            }
            TabEvent::Detach(dock_id) => {
                if info.group.len() == 1 {
                    // Equivalent to dragging the window
                    let Some(cursor_pos) = self.screen_cursor_pos() else {
                        return Task::none();
                    };

                    let info = self.detached.get_mut(&id).unwrap();
                    info.dragging_cursor_relative = Some(Vector::new(
                        cursor_pos.x - info.position.x,
                        cursor_pos.y - info.position.y,
                    ));
                    return iced_runtime::window::drag(id);
                } else {
                    info.group.remove_dock(&dock_id);
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
                return iced_runtime::window::drag(id);
            }
            TabEvent::CloseGroup => {
                self.detached.remove(&id);
                return iced_runtime::window::close(id);
            }
        }

        Task::none()
    }

    pub fn attach_to_main(&mut self, id: window::Id) -> Task<()> {
        let Some(attach) = self.main_attach_info(id) else {
            return Task::none();
        };

        let Some(info) = self.detached.remove(&id) else {
            return Task::none();
        };

        match attach {
            AttachInfo::Split { pane, result_edge } => {
                self.dock_state.split(pane, result_edge, info.group);
            }
            AttachInfo::Merge { pane } => {
                if let Some(group) = self
                    .dock_state
                    .panes_state_mut()
                    .and_then(|st| st.get_mut(pane))
                {
                    for dock in info.group.iter() {
                        group.add_dock(dock.clone());
                    }
                }
            }
            AttachInfo::Initialize => {
                self.dock_state.open_group(info.group);
            }
        }

        iced_runtime::window::close(id)
    }

    pub fn merge_floating(&mut self, src: window::Id, dst: window::Id) -> Task<()> {
        let Some(src_info) = self.detached.remove(&src) else {
            return Task::none();
        };
        let Some(dst_info) = self.detached.get_mut(&dst) else {
            return Task::none();
        };

        for dock in src_info.group.iter() {
            dst_info.group.add_dock(dock.clone());
        }

        iced_runtime::window::close(src)
    }

    fn detach(&mut self, pane: pane_grid::Pane) -> Task<DockMessage> {
        let Some(group) = self.dock_state.close(pane) else {
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
        let window_size = Size::new(400.0, 350.0);
        let (window_id, open_task) = iced_runtime::window::open(window::Settings {
            decorations: false,
            position: window::Position::Specific(self.screen_cursor_pos()?),
            size: window_size,
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
                window_decorations: WindowDecorations::default(),
                group,
                position: self.screen_cursor_pos()?,
                size: window_size,
                dragging_cursor_relative: Some(Vector::ZERO),
                last_overlap: None,
            },
        );

        Some((
            window_id,
            open_task
                .then(move |id| iced_runtime::window::raw_id::<()>(id).map(move |raw| (id, raw))),
        ))
    }

    pub fn screen_cursor_pos(&self) -> Option<Point> {
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

    pub fn is_over_main_window(&self, id: window::Id) -> bool {
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

    pub fn main_window(&self) -> &GroupWindowInfo {
        &self.main_window
    }

    pub fn detached_window(&self, id: window::Id) -> Option<&GroupWindowInfo> {
        self.detached.get(&id)
    }

    pub fn window_infos(&self) -> impl Iterator<Item = &GroupWindowInfo> {
        std::iter::once(&self.main_window).chain(self.detached.values())
    }

    pub fn window_info(&self, id: window::Id) -> Option<&GroupWindowInfo> {
        if id == self.main_window.id {
            Some(&self.main_window)
        } else {
            self.detached.get(&id)
        }
    }

    pub fn sub_windows(&self) -> impl Iterator<Item = window::Id> {
        self.sub_windows.keys().copied()
    }

    pub fn close(self) -> Task<()> {
        let mut task = Task::none();
        for id in self.detached.keys() {
            task = task.chain(iced_runtime::window::close(*id));
        }
        task.chain(iced_runtime::window::close(self.main_window.id))
    }

    pub fn dock_state(&self) -> &DockState {
        &self.dock_state
    }

    pub fn main_attach_info(&self, window: window::Id) -> Option<AttachInfo> {
        const SPACING: f32 = 2.0;

        let info = self.detached.get(&window)?;
        let relative_window_pos = Point::new(
            info.position.x + info.size.width / 2.0 - self.main_window.position.x,
            info.position.y + info.size.height / 2.0 - self.main_window.position.y,
        );

        let Some(node) = self.dock_state.panes_state().map(|st| st.layout()) else {
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
        let regions = node.pane_regions(SPACING, 0.0, self.main_window.size);

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

    pub fn floating_merge_info(&self, src_window: window::Id) -> Option<window::Id> {
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

    pub fn current_attach_or_merge_info(&self) -> Option<AttachOrMergeInfo> {
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
        services: &'a Services,
    ) -> Option<Element<'a, DockMessage, Theme, Renderer>> {
        if window_id == self.main_window.id {
            let dock_w =
                DockWidget::new(&self.dock_state, DockMessage::Main).content(move |_, dock_id| {
                    let dock = self
                        .docks
                        .get(&dock_id)
                        .unwrap_or_else(|| panic!("Dock not found: {}", dock_id));
                    dock.view(window_id, services)
                        .map(move |m| DockMessage::Dock(dock_id.clone(), m))
                });

            if let Some(AttachOrMergeInfo::Attach(attach)) = self.current_attach_or_merge_info() {
                Some(dock_w.attach_info(attach).into())
            } else {
                Some(dock_w.into())
            }
        } else if let Some(info) = self.detached_window(window_id) {
            Some(
                FloatingDockWidget::new(&info.group, move |action| DockMessage::Float {
                    id: window_id,
                    action,
                })
                .content(move |dock_id| {
                    let dock = self
                        .docks
                        .get(&dock_id)
                        .unwrap_or_else(|| panic!("Dock not found: {}", dock_id));
                    dock.view(window_id, services)
                        .map(move |m| DockMessage::Dock(dock_id.clone(), m))
                })
                .is_merging(match self.current_attach_or_merge_info() {
                    Some(AttachOrMergeInfo::Merge { dst }) => dst == window_id,
                    _ => false,
                })
                .into(),
            )
        } else if let Some(dock_id) = self.sub_windows.get(&window_id)
            && let Some(dock) = self.docks.get(dock_id)
        {
            Some(
                dock.view(window_id, services)
                    .map(move |m| DockMessage::Dock(dock_id.clone(), m)),
            )
        } else {
            None
        }
    }

    pub fn update(&mut self, action: DockMessage, services: &mut Services) -> Task<DockMessage> {
        let task = match action {
            DockMessage::Main(dock_action) => self.on_dock_action(dock_action),
            DockMessage::Float { id, action } => self.on_float_action(id, action),
            DockMessage::Dock(dock_id, msg) => {
                if let Some(dock) = self.docks.get_mut(&dock_id) {
                    dock.update(msg, services)
                        .map(move |m| DockMessage::Dock(dock_id.clone(), m))
                } else {
                    Task::none()
                }
            }
            DockMessage::RawWindowGet(id, raw_id) => {
                let window_decorations = if id == self.main_window.id {
                    &self.main_window.window_decorations
                } else if let Some(info) = self.detached.get(&id) {
                    &info.window_decorations
                } else {
                    return Task::none();
                };
                if !window_decorations.attach(raw_id) {
                    log::error!("Failed to attach native window decorations for {id:?}");
                }

                if id == self.main_window.id {
                    self.main_window.raw_id = Some(raw_id);
                    Task::none()
                } else if let Some(info) = self.detached.get_mut(&id) {
                    info.raw_id = Some(raw_id);
                    lapiz_runtime::platform::disable_window_snap(raw_id);

                    let Some(main_raw_id) = self.main_window.raw_id else {
                        log::error!("Main window raw ID is not available. This should not happen.");
                        return Task::none();
                    };
                    lapiz_runtime::platform::set_window_parent(main_raw_id, raw_id);

                    if info.dragging_cursor_relative.is_some() {
                        iced_runtime::window::drag(id)
                    } else {
                        Task::none()
                    }
                } else {
                    Task::none()
                }
            }
            DockMessage::RedrawRequested => Task::none(),
        };

        self.sub_windows.clear();
        for (dock_id, dock) in &self.docks {
            for sub_window in dock.sub_windows() {
                self.sub_windows.insert(sub_window, dock_id.clone());
            }
        }

        task
    }

    pub fn subscription(&self, services: &Services) -> Subscription<DockMessage> {
        Subscription::batch(self.docks.iter().map(|(id, dock)| {
            dock.subscription(services)
                .with(id.clone())
                .map(|(dock, message)| DockMessage::Dock(dock, message))
        }))
    }
}

fn overlaps(pos_a: Point, size_a: Size, pos_b: Point, size_b: Size) -> bool {
    pos_a.x < pos_b.x + size_b.width
        && pos_a.x + size_a.width > pos_b.x
        && pos_a.y < pos_b.y + size_b.height
        && pos_a.y + size_a.height > pos_b.y
}

fn _snap(pos_a: Point, size_a: Size, pos_b: Point, size_b: Size) -> Point {
    let mut result = pos_a;

    // snap left to left
    if (pos_a.x - pos_b.x).abs() < _FLOATING_WINDOW_SNAP_DISTANCE {
        result.x = pos_b.x;
    }
    // snap left to right
    else if (pos_a.x - (pos_b.x + size_b.width)).abs() < _FLOATING_WINDOW_SNAP_DISTANCE {
        result.x = pos_b.x + size_b.width;
    }

    // snap right to right
    if (pos_a.x + size_a.width - (pos_b.x + size_b.width)).abs() < _FLOATING_WINDOW_SNAP_DISTANCE {
        result.x = pos_b.x + size_b.width - size_a.width;
    }
    // snap right to left
    else if (pos_a.x + size_a.width - pos_b.x).abs() < _FLOATING_WINDOW_SNAP_DISTANCE {
        result.x = pos_b.x - size_a.width;
    }

    // snap top to top
    if (pos_a.y - pos_b.y).abs() < _FLOATING_WINDOW_SNAP_DISTANCE {
        result.y = pos_b.y;
    }
    // snap top to bottom
    else if (pos_a.y - (pos_b.y + size_b.height)).abs() < _FLOATING_WINDOW_SNAP_DISTANCE {
        result.y = pos_b.y + size_b.height;
    }

    // snap bottom to bottom
    if (pos_a.y + size_a.height - (pos_b.y + size_b.height)).abs() < _FLOATING_WINDOW_SNAP_DISTANCE
    {
        result.y = pos_b.y + size_b.height - size_a.height;
    }
    // snap bottom to top
    else if (pos_a.y + size_a.height - pos_b.y).abs() < _FLOATING_WINDOW_SNAP_DISTANCE {
        result.y = pos_b.y - size_a.height;
    }

    result
}

pub enum DockMessage {
    Main(DockAction),
    Float { id: window::Id, action: TabEvent },
    Dock(DockId, Box<dyn Any + Send>),
    RawWindowGet(window::Id, u64),
    RedrawRequested,
}

impl std::fmt::Debug for DockMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
            Self::RedrawRequested => f.debug_tuple("RedrawRequested").finish(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GroupWindowInfo {
    pub id: window::Id,
    pub raw_id: Option<u64>,
    pub window_decorations: WindowDecorations,
    pub position: Point,
    pub size: Size,
    pub group: DockGroupData,
    pub dragging_cursor_relative: Option<Vector>,
    pub last_overlap: Option<(window::Id, std::time::Instant, Point)>,
}

#[derive(Debug, Clone)]
pub enum AttachOrMergeInfo {
    Attach(AttachInfo),
    Merge { dst: window::Id },
}

#[derive(Debug, Clone)]
pub enum AttachInfo {
    Split {
        pane: pane_grid::Pane,
        result_edge: pane_grid::Edge,
    },
    Merge {
        pane: pane_grid::Pane,
    },
    Initialize,
}
