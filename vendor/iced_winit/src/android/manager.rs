use crate::core::window::{self, Id, Level, Settings};
use crate::core::{Point, Rectangle, Size};

/// The logical size and scale factor of the native window.
#[derive(Debug, Clone, Copy)]
pub struct Screen {
    /// The logical size of the native window.
    pub logical_size: Size,
    /// The scale factor reported by the native window.
    pub scale_factor: f32,
}

impl Screen {
    /// Clamps a logical window position so that the window stays fully
    /// visible on the screen.
    pub fn clamp_position(&self, position: Point, size: Size) -> Point {
        Point::new(
            position
                .x
                .clamp(0.0, (self.logical_size.width - size.width).max(0.0)),
            position
                .y
                .clamp(0.0, (self.logical_size.height - size.height).max(0.0)),
        )
    }
}

/// A logical window: a rectangular region of the single native window.
pub struct LogicalWindow {
    pub position: Point,
    pub size: Size,
    pub resizable: bool,
    pub min_size: Option<Size>,
    pub max_size: Option<Size>,
    pub level: Level,
    pub visible: bool,
    pub passthrough: bool,

    /// The bounds saved before maximizing, if the window is maximized.
    pub maximized: Option<Rectangle>,
}

impl LogicalWindow {
    /// The logical bounds of the window, in the coordinate space of the
    /// native window.
    pub fn bounds(&self) -> Rectangle {
        Rectangle::new(self.position, self.size)
    }

    /// Returns `true` if the given logical position (in native window
    /// coordinates) is inside the window.
    pub fn contains(&self, position: Point) -> bool {
        self.visible && !self.passthrough && self.bounds().contains(position)
    }
}

/// Keeps track of the logical windows of the application.
pub struct Manager {
    entries: std::collections::BTreeMap<Id, LogicalWindow>,
    /// The z-order of the windows, from bottom to top.
    order: Vec<Id>,
    /// The sequence numbers used to keep the z-order stable per level.
    sequence: rustc_hash::FxHashMap<Id, u64>,
    next_sequence: u64,
    root: Option<Id>,
    screen: Screen,
}

impl Manager {
    pub fn new(screen: Screen) -> Self {
        Self {
            entries: std::collections::BTreeMap::new(),
            order: Vec::new(),
            sequence: rustc_hash::FxHashMap::default(),
            next_sequence: 0,
            root: None,
            screen,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, id: Id) -> bool {
        self.entries.contains_key(&id)
    }

    pub fn root_id(&self) -> Option<Id> {
        self.root
    }

    /// Returns the oldest window, in creation order.
    pub fn first_id(&self) -> Option<Id> {
        self.entries.first_key_value().map(|(id, _)| *id)
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// Updates the logical size of the native window, resizing the root
    /// window if needed.
    pub fn set_screen(&mut self, logical_size: Size, scale_factor: f32) {
        let changed = self.screen.logical_size != logical_size;

        self.screen = Screen {
            logical_size,
            scale_factor,
        };

        if !changed {
            return;
        }

        if let Some(root) = self.root {
            if let Some(window) = self.entries.get_mut(&root) {
                window.position = Point::ORIGIN;
                window.size = logical_size;
            }
        }

        self.clamp_positions();
    }

    pub fn insert(&mut self, id: Id, window: LogicalWindow) {
        if self.entries.is_empty() {
            self.root = Some(id);
        }

        let _ = self.sequence.insert(id, self.next_sequence);
        self.next_sequence += 1;
        self.order.push(id);
        self.sort_order();

        let _ = self.entries.insert(id, window);
    }

    pub fn remove(&mut self, id: Id) -> Option<LogicalWindow> {
        let window = self.entries.remove(&id)?;
        let _ = self.sequence.remove(&id);
        self.order.retain(|entry| *entry != id);

        if self.root == Some(id) {
            self.root = None;
        }

        Some(window)
    }

    pub fn get(&self, id: Id) -> Option<&LogicalWindow> {
        self.entries.get(&id)
    }

    pub fn get_mut(&mut self, id: Id) -> Option<&mut LogicalWindow> {
        self.entries.get_mut(&id)
    }

    /// Returns the z-order of the windows, from bottom to top.
    pub fn z_order(&self) -> &[Id] {
        &self.order
    }

    /// Brings a window to the front of its level.
    pub fn raise(&mut self, id: Id) {
        if !self.sequence.contains_key(&id) {
            return;
        }

        let sequence = self.next_sequence;
        self.next_sequence += 1;
        let _=self.sequence.insert(id, sequence);
        self.sort_order();
    }

    pub fn set_level(&mut self, id: Id, level: Level) {
        if let Some(window) = self.get_mut(id) {
            window.level = level;
        }

        self.sort_order();
    }

    fn sort_order(&mut self) {
        let level_of = |window: &LogicalWindow| match window.level {
            Level::AlwaysOnBottom => 0,
            Level::Normal => 1,
            Level::AlwaysOnTop => 2,
        };
        let sequence = |id: Id| self.sequence.get(&id).copied().unwrap_or(u64::MAX);

        self.order.sort_by_key(|id| {
            let level = self.entries.get(id).map(level_of).unwrap_or(1);

            (level, sequence(*id))
        });
    }

    /// Returns the id of the topmost window containing the given logical
    /// position, if any.
    pub fn hit_test(&self, position: Point) -> Option<Id> {
        self.order.iter().rev().find_map(|id| {
            let window = self.entries.get(id)?;

            window.contains(position).then_some(*id)
        })
    }

    /// Returns the topmost window whose resize border contains the given
    /// logical position, together with the direction to resize it in.
    ///
    /// The root window—which always covers the whole native window—and
    /// maximized windows are never resizable through their borders.
    pub fn resize_hit_test(&self, position: Point) -> Option<(Id, window::Direction)> {
        let id = self.hit_test(position)?;
        let window = self.entries.get(&id)?;

        if !window.resizable || Some(id) == self.root || window.maximized.is_some() {
            return None;
        }

        const RESIZE_BORDER: f32 = 10.0;

        let bounds = window.bounds();
        let left = position.x < bounds.x + RESIZE_BORDER;
        let right = position.x >= bounds.x + bounds.width - RESIZE_BORDER;
        let top = position.y < bounds.y + RESIZE_BORDER;
        let bottom = position.y >= bounds.y + bounds.height - RESIZE_BORDER;

        let direction = match (left, right, top, bottom) {
            (true, _, true, _) => Some(window::Direction::NorthWest),
            (_, true, true, _) => Some(window::Direction::NorthEast),
            (true, _, _, true) => Some(window::Direction::SouthWest),
            (_, true, _, true) => Some(window::Direction::SouthEast),
            (true, ..) => Some(window::Direction::West),
            (_, true, ..) => Some(window::Direction::East),
            (_, _, true, _) => Some(window::Direction::North),
            (_, _, _, true) => Some(window::Direction::South),
            _ => None,
        };

        direction.map(|d| (id, d))
    }

    /// Clamps the position of every window so that it stays fully visible
    /// on the screen.
    pub fn clamp_positions(&mut self) {
        let screen = self.screen;

        for window in self.entries.values_mut() {
            window.position = screen.clamp_position(window.position, window.size);
        }
    }

    /// Applies the size constraints of a window to the given logical size.
    pub fn constrain(&self, id: Id, mut size: Size) -> Size {
        let Some(window) = self.entries.get(&id) else {
            return size;
        };

        if let Some(min_size) = window.min_size {
            size.width = size.width.max(min_size.width);
            size.height = size.height.max(min_size.height);
        }

        if let Some(max_size) = window.max_size {
            size.width = size.width.min(max_size.width);
            size.height = size.height.min(max_size.height);
        }

        size
    }
}

/// Computes the initial geometry of a logical window from its [`Settings`].
pub fn initial_geometry(settings: &Settings, screen: Screen, is_root: bool) -> (Point, Size) {
    if is_root {
        // The root window always covers the whole native window.
        return (Point::ORIGIN, screen.logical_size);
    }

    let size = Size::new(
        settings.size.width.min(screen.logical_size.width),
        settings.size.height.min(screen.logical_size.height),
    );

    let position = match settings.position {
        window::Position::Default | window::Position::Centered => Point::new(
            (screen.logical_size.width - size.width) / 2.0,
            (screen.logical_size.height - size.height) / 2.0,
        ),
        window::Position::Specific(position) => position,
        window::Position::SpecificWith(position) => position(size, screen.logical_size),
    };

    (screen.clamp_position(position, size), size)
}
