use std::{
    any::{Any, type_name, type_name_of_val},
    collections::{HashMap, hash_map::Entry},
    hash::Hash,
    sync::Arc,
};

use anyhow::{Result, bail};
use iced_core::{Element, Theme, window};
use iced_runtime::{Task, futures::Subscription};
use lapiz_utils::{log_err::LogErr as _, wrapper};
use parse_display::Display;

use crate::global::{Global, Globals};

wrapper! {
    /// Unique static string identifier for a [`WindowView`] type.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display)]
    #[display("{0}")]
    pub WindowViewId : &'static str
}

/// Marker struct representing a window handle namespace.
pub struct Window;

/// A self-contained UI view controller that manages one or more application windows.
///
/// `WindowView` defines the state, rendering, update logic, and window lifecycle for a logical
/// view in the application (such as the main workspace, brush editor, or modal dialogs). Views
/// are registered with the [`WindowViewManager`] via [`Runtime::register_view`](crate::Runtime::register_view)
/// and opened or closed dynamically using [`WindowCommand`]s pushed to the [`WindowCommandBuffer`].
///
/// # Lifecycle
///
/// - **Booting**: [`boot`](WindowView::boot) constructs the view state from optional `BootParams`
///   and returns an initial [`Task`] (typically opening the view's OS window(s)).
/// - **Rendering & Updates**: [`view`](WindowView::view) produces the UI element tree for each
///   associated `window::Id`, while [`update`](WindowView::update) and
///   [`subscription`](WindowView::subscription) handle messages and background event streams.
/// - **Window Tracking**: [`windows`](WindowView::windows) reports all `window::Id`s currently
///   owned by the view. If [`root_window`](WindowView::root_window) returns a window ID and that
///   window is closed by the user, the [`WindowViewManager`] automatically closes the entire view.
/// - **Closing**: [`close`](WindowView::close) consumes the view state and returns a cleanup task.
pub trait WindowView: 'static + Sized {
    type Message: Send + 'static;
    type BootParams;

    fn id() -> WindowViewId;
    fn boot(
        params: Option<Self::BootParams>,
        globals: &mut Globals,
    ) -> Result<(Self, Task<Self::Message>)>;
    fn view<'a>(
        &'a self,
        window: window::Id,
        globals: &'a Globals,
    ) -> impl Into<Element<'a, Self::Message, Theme, crate::Renderer>>;
    fn update(
        &mut self,
        message: Self::Message,
        globals: &mut Globals,
    ) -> impl Into<Task<Self::Message>>;
    fn close(self, globals: &mut Globals) -> Task<()>;
    fn subscription(&self, _globals: &Globals) -> Subscription<Self::Message> {
        Subscription::none()
    }
    fn windows(&self) -> Arc<[window::Id]>;
    fn root_window(&self) -> Option<window::Id> {
        None
    }
}

pub trait ErasedWindowView: 'static {
    fn id(&self) -> WindowViewId;
    fn view<'a>(
        &'a self,
        window: window::Id,
        globals: &'a Globals,
    ) -> Element<'a, Box<dyn Any + Send>, Theme, crate::Renderer>;
    fn update(
        &mut self,
        message: Box<dyn Any + Send>,
        globals: &mut Globals,
    ) -> Task<Box<dyn Any + Send>>;
    fn close(self: Box<Self>, globals: &mut Globals) -> Task<()>;
    fn subscription(&self, globals: &Globals) -> Subscription<Box<dyn Any + Send>>;
    fn windows(&self) -> Arc<[window::Id]>;
    fn root_window(&self) -> Option<window::Id>;
}

impl<T> ErasedWindowView for T
where
    T: WindowView,
{
    fn id(&self) -> WindowViewId {
        <T as WindowView>::id()
    }

    fn view<'a>(
        &'a self,
        window: window::Id,
        globals: &'a Globals,
    ) -> Element<'a, Box<dyn Any + Send>, Theme, crate::Renderer> {
        <T as WindowView>::view(self, window, globals)
            .into()
            .map(|msg| Box::new(msg) as Box<dyn Any + Send>)
    }

    fn update(
        &mut self,
        message: Box<dyn Any + Send>,
        globals: &mut Globals,
    ) -> Task<Box<dyn Any + Send>> {
        let msg = *message
            .downcast::<T::Message>()
            .expect("Cast window message failed");
        <T as WindowView>::update(self, msg, globals)
            .into()
            .map(|msg| Box::new(msg) as Box<dyn Any + Send>)
    }

    fn close(self: Box<Self>, globals: &mut Globals) -> Task<()> {
        <T as WindowView>::close(*self, globals)
    }

    fn subscription(&self, globals: &Globals) -> Subscription<Box<dyn Any + Send>> {
        <T as WindowView>::subscription(self, globals)
            .map(|msg| Box::new(msg) as Box<dyn Any + Send>)
    }

    fn windows(&self) -> Arc<[window::Id]> {
        <T as WindowView>::windows(self)
    }

    fn root_window(&self) -> Option<window::Id> {
        <T as WindowView>::root_window(self)
    }
}

/// Type-erased message targeted at a specific opened [`WindowView`].
#[derive(Debug)]
pub struct ErasedWindowViewMessage {
    view: WindowViewId,
    message: Box<dyn Any + Send>,
}

/// Message routed to the [`WindowViewManager`] to update an active view.
pub enum WindowViewManagerMessage {
    ViewUpdate(ErasedWindowViewMessage),
}

type WindowViewBootFn = Box<
    dyn Fn(
            Option<Box<dyn Any>>,
            &mut Globals,
        ) -> Result<(Box<dyn ErasedWindowView>, Task<ErasedWindowViewMessage>)>
        + 'static,
>;

struct OpenedView {
    windows: Arc<[window::Id]>,
    state: Box<dyn ErasedWindowView>,
}

impl OpenedView {
    fn new(state: Box<dyn ErasedWindowView>) -> Self {
        Self {
            windows: Default::default(),
            state,
        }
    }
}

/// Manager that tracks registered [`WindowView`] factories, active view instances, and window-to-view routing.
///
/// Owned by [`Runtime`](crate::Runtime), `WindowViewManager` boots the designated root view on
/// startup, dispatches `iced` view/update/subscription calls to the appropriate [`WindowView`] for
/// each `window::Id`, and processes lifecycle transitions triggered by [`WindowCommand`]s.
#[derive(Default)]
pub struct WindowViewManager {
    window_to_view: HashMap<window::Id, WindowViewId>,
    registered_views: HashMap<WindowViewId, WindowViewBootFn>,
    opened_views: HashMap<WindowViewId, OpenedView>,
    root_view: Option<WindowViewId>,
}

impl Global for WindowViewManager {}

impl WindowViewManager
where
    Theme: 'static,
{
    pub fn register_view<T: WindowView>(&mut self) {
        self.registered_views.insert(
            T::id(),
            Box::new(|params, globals| {
                let (view, task) = if let Some(params) = params {
                    match params.downcast() {
                        Ok(params) => T::boot(Some(*params), globals)?,
                        Err(params) => {
                            anyhow::bail!(
                                "Invalid params for view: Expected {}, found {}",
                                type_name::<T::BootParams>(),
                                type_name_of_val(params.as_ref())
                            );
                        }
                    }
                } else {
                    T::boot(None, globals)?
                };

                Ok((
                    Box::new(view),
                    task.map(|o| ErasedWindowViewMessage {
                        view: T::id(),
                        message: Box::new(o) as Box<dyn Any + Send>,
                    }),
                ))
            }),
        );
    }

    pub fn set_root_view<T: WindowView>(&mut self) {
        self.root_view = Some(T::id());
    }

    pub fn root_view(&self) -> Option<WindowViewId> {
        self.root_view
    }

    pub fn boot(
        &mut self,
        params: Option<Box<dyn Any>>,
        globals: &mut Globals,
    ) -> Task<WindowViewManagerMessage> {
        self.open_window_view(
            self.root_view.expect("No root view specified."),
            params,
            globals,
        )
        .unwrap()
    }

    pub fn view<'a>(
        &'a self,
        id: window::Id,
        globals: &'a Globals,
    ) -> Option<Element<'a, WindowViewManagerMessage, Theme, crate::Renderer>> {
        let Some(window) = self.window_to_view.get(&id).cloned() else {
            log::error!(
                "Unable to view a window that doesn't have corresponding view: {}",
                id
            );
            return None;
        };

        let Some(view) = self.opened_views.get(&window) else {
            log::error!("Unable to view a window whose view is not opened: {}", id);
            return None;
        };

        Some(view.state.view(id, globals).map(move |msg| {
            WindowViewManagerMessage::ViewUpdate(ErasedWindowViewMessage {
                view: window,
                message: msg,
            })
        }))
    }

    pub fn update(
        &mut self,
        message: WindowViewManagerMessage,
        globals: &mut Globals,
    ) -> Task<WindowViewManagerMessage> {
        match message {
            WindowViewManagerMessage::ViewUpdate(message) => {
                let Some(view) = self.opened_views.get_mut(&message.view) else {
                    log::error!(
                        "Unable to update a view that is not opened: {}",
                        message.view.0
                    );
                    return Task::none();
                };

                let task = view.state.update(message.message, globals).map(move |msg| {
                    WindowViewManagerMessage::ViewUpdate(ErasedWindowViewMessage {
                        view: message.view,
                        message: msg,
                    })
                });

                update_view_windows(message.view, view, &mut self.window_to_view);

                task
            }
        }
    }

    pub fn subscription(&self, globals: &Globals) -> Subscription<WindowViewManagerMessage> {
        Subscription::batch(self.opened_views.iter().map(|(id, view)| {
            view.state
                .subscription(globals)
                .with(*id)
                .map(|(view, message)| {
                    WindowViewManagerMessage::ViewUpdate(ErasedWindowViewMessage { view, message })
                })
        }))
    }

    pub fn open_window_view(
        &mut self,
        view_id: WindowViewId,
        params: Option<Box<dyn Any>>,
        globals: &mut Globals,
    ) -> Result<Task<WindowViewManagerMessage>> {
        if self.opened_views.contains_key(&view_id) {
            bail!("Window view already opened: {}", view_id.0);
        }

        let Some(boot) = self.registered_views.get(&view_id) else {
            bail!(
                "Unable to open a window view that is not registered: {}",
                view_id.0
            );
        };

        let (view_state, task) = boot(params, globals)?;
        let mut view = OpenedView::new(view_state);
        update_view_windows(view_id, &mut view, &mut self.window_to_view);
        self.opened_views.insert(view_id, view);

        Ok(task.map(WindowViewManagerMessage::ViewUpdate))
    }

    pub fn close_window_view(&mut self, view_id: WindowViewId, globals: &mut Globals) -> Task<()> {
        let Some(view) = self.opened_views.remove(&view_id) else {
            return Task::none();
        };

        view.state.close(globals)
    }

    pub fn on_window_closed(&mut self, window: window::Id, globals: &mut Globals) -> Task<()> {
        let Some(view_id) = self.window_to_view.remove(&window) else {
            log::error!(
                "Unable to close a window that doesn't have corresponding view: {}",
                window
            );
            return Task::none();
        };

        let Entry::Occupied(entry) = self.opened_views.entry(view_id) else {
            log::error!("Unable to close a view that is not opened: {}", view_id.0);
            return Task::none();
        };

        if entry.get().state.root_window() == Some(window) {
            log::info!("Root window of view {} closed, closing the view", view_id.0);
            entry.remove().state.close(globals)
        } else {
            Task::none()
        }
    }
}

fn update_view_windows(
    view_id: WindowViewId,
    view: &mut OpenedView,
    window_to_view: &mut HashMap<window::Id, WindowViewId>,
) {
    let new_windows = view.state.windows();
    if view.windows == new_windows {
        return;
    }

    for window in view.windows.iter() {
        window_to_view.remove(window);
    }
    view.windows = new_windows;
    for window in view.windows.iter() {
        window_to_view.insert(*window, view_id);
    }
}

/// Deferred command executed against the [`WindowViewManager`] and [`Globals`].
pub trait WindowCommand: 'static {
    fn execute(
        self: Box<Self>,
        wm: &mut WindowViewManager,
        globals: &mut Globals,
    ) -> Result<Task<WindowViewManagerMessage>>;
}

/// Global queue of [`WindowCommand`]s drained and executed after each application update step.
///
/// Allows views, actions, and tools with access to [`Globals`] to request opening, closing, or
/// toggling [`WindowView`]s without directly borrowing [`WindowViewManager`].
#[derive(Default)]
pub struct WindowCommandBuffer {
    commands: Vec<Box<dyn WindowCommand>>,
}

impl Global for WindowCommandBuffer {}

impl WindowCommandBuffer {
    pub fn push<T: WindowCommand>(&mut self, command: T) {
        self.commands.push(Box::new(command));
    }

    pub fn execute(
        &mut self,
        wm: &mut WindowViewManager,
        globals: &mut Globals,
    ) -> Task<WindowViewManagerMessage> {
        let mut tasks = Vec::new();
        for command in self.commands.drain(..) {
            if let Ok(task) = command.execute(wm, globals).logged_err() {
                tasks.push(task);
            }
        }
        Task::batch(tasks)
    }
}

/// [`WindowCommand`] that opens a registered [`WindowView`] with optional boot parameters.
pub struct OpenWindowViewCommand {
    view_id: WindowViewId,
    open_params: Option<Box<dyn Any>>,
}

impl WindowCommand for OpenWindowViewCommand {
    fn execute(
        self: Box<Self>,
        wm: &mut WindowViewManager,
        globals: &mut Globals,
    ) -> Result<Task<WindowViewManagerMessage>> {
        wm.open_window_view(self.view_id, self.open_params, globals)
    }
}

impl OpenWindowViewCommand {
    pub fn new(view_id: WindowViewId) -> Self {
        Self {
            view_id,
            open_params: None,
        }
    }

    pub fn new_with_params<T: 'static>(view_id: WindowViewId, open_params: T) -> Self {
        Self {
            view_id,
            open_params: Some(Box::new(open_params) as _),
        }
    }
}

/// [`WindowCommand`] that closes a currently opened [`WindowView`].
pub struct CloseWindowViewCommand {
    view_id: WindowViewId,
}

impl WindowCommand for CloseWindowViewCommand {
    fn execute(
        self: Box<Self>,
        wm: &mut WindowViewManager,
        globals: &mut Globals,
    ) -> Result<Task<WindowViewManagerMessage>> {
        Ok(wm.close_window_view(self.view_id, globals).discard())
    }
}

impl CloseWindowViewCommand {
    pub fn new(view_id: WindowViewId) -> Self {
        Self { view_id }
    }
}

/// [`WindowCommand`] that closes a [`WindowView`] if it is open, or opens it if it is closed.
pub struct ToggleWindowViewCommand {
    view_id: WindowViewId,
    open_params: Option<Box<dyn Any>>,
}

impl WindowCommand for ToggleWindowViewCommand {
    fn execute(
        self: Box<Self>,
        wm: &mut WindowViewManager,
        globals: &mut Globals,
    ) -> Result<Task<WindowViewManagerMessage>> {
        if wm.opened_views.contains_key(&self.view_id) {
            Ok(wm.close_window_view(self.view_id, globals).discard())
        } else {
            wm.open_window_view(self.view_id, self.open_params, globals)
        }
    }
}

impl ToggleWindowViewCommand {
    pub fn new(view_id: WindowViewId) -> Self {
        Self {
            view_id,
            open_params: None,
        }
    }

    pub fn new_with_params<T: 'static>(view_id: WindowViewId, open_params: T) -> Self {
        Self {
            view_id,
            open_params: Some(Box::new(open_params) as _),
        }
    }
}

/// [`WindowCommand`] that associates a newly opened sub-window ID with an existing [`WindowViewId`].
pub struct SubWindowOpenedCommand {
    view_id: WindowViewId,
    window: window::Id,
}

impl SubWindowOpenedCommand {
    pub fn new(view_id: WindowViewId, window: window::Id) -> Self {
        Self { view_id, window }
    }
}

impl WindowCommand for SubWindowOpenedCommand {
    fn execute(
        self: Box<Self>,
        wm: &mut WindowViewManager,
        _globals: &mut Globals,
    ) -> Result<Task<WindowViewManagerMessage>> {
        wm.window_to_view.insert(self.window, self.view_id);
        Ok(Task::none())
    }
}
