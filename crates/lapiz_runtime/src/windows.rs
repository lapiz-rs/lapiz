use std::{
    any::Any,
    collections::{HashMap, hash_map::Entry},
    hash::Hash,
    sync::Arc,
};

use iced_core::{Element, Theme, window};
use iced_runtime::{Task, futures::Subscription};
use lapiz_utils::wrapper;
use parse_display::Display;

use crate::{Services, service::Service};

wrapper! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display)]
    #[display("{0}")]
    pub WindowViewId : &'static str
}

pub struct Window;

pub trait WindowView: 'static + Sized {
    type Message: Send + 'static;

    fn id() -> WindowViewId;
    fn boot(services: &mut Services) -> (Self, Task<Self::Message>);
    fn view<'a>(
        &'a self,
        window: window::Id,
        services: &'a Services,
    ) -> impl Into<Element<'a, Self::Message, Theme, crate::Renderer>>;
    fn update(
        &mut self,
        message: Self::Message,
        services: &mut Services,
    ) -> impl Into<Task<Self::Message>>;
    fn close(self, services: &mut Services) -> Task<()>;
    fn subscription(&self, _services: &Services) -> Subscription<Self::Message> {
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
        services: &'a Services,
    ) -> Element<'a, Box<dyn Any + Send>, Theme, crate::Renderer>;
    fn update(
        &mut self,
        message: Box<dyn Any + Send>,
        services: &mut Services,
    ) -> Task<Box<dyn Any + Send>>;
    fn close(self: Box<Self>, services: &mut Services) -> Task<()>;
    fn subscription(&self, services: &Services) -> Subscription<Box<dyn Any + Send>>;
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
        services: &'a Services,
    ) -> Element<'a, Box<dyn Any + Send>, Theme, crate::Renderer> {
        <T as WindowView>::view(self, window, services)
            .into()
            .map(|msg| Box::new(msg) as Box<dyn Any + Send>)
    }

    fn update(
        &mut self,
        message: Box<dyn Any + Send>,
        services: &mut Services,
    ) -> Task<Box<dyn Any + Send>> {
        let msg = *message
            .downcast::<T::Message>()
            .expect("Cast window message failed");
        <T as WindowView>::update(self, msg, services)
            .into()
            .map(|msg| Box::new(msg) as Box<dyn Any + Send>)
    }

    fn close(self: Box<Self>, services: &mut Services) -> Task<()> {
        <T as WindowView>::close(*self, services)
    }

    fn subscription(&self, services: &Services) -> Subscription<Box<dyn Any + Send>> {
        <T as WindowView>::subscription(self, services)
            .map(|msg| Box::new(msg) as Box<dyn Any + Send>)
    }

    fn windows(&self) -> Arc<[window::Id]> {
        <T as WindowView>::windows(self)
    }

    fn root_window(&self) -> Option<window::Id> {
        <T as WindowView>::root_window(self)
    }
}

#[derive(Debug)]
pub struct ErasedWindowViewMessage {
    view: WindowViewId,
    message: Box<dyn Any + Send>,
}

pub enum WindowViewManagerMessage {
    ViewUpdate(ErasedWindowViewMessage),
}

type WindowViewBootFn = Box<
    dyn Fn(&mut Services) -> (Box<dyn ErasedWindowView>, Task<ErasedWindowViewMessage>)
        + Send
        + Sync
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

#[derive(Default)]
pub struct WindowViewManager {
    window_to_view: HashMap<window::Id, WindowViewId>,
    registered_views: HashMap<WindowViewId, WindowViewBootFn>,
    opened_views: HashMap<WindowViewId, OpenedView>,
    root_view: Option<WindowViewId>,
}

impl Service for WindowViewManager {}

impl WindowViewManager
where
    Theme: 'static,
{
    pub fn register_view<T: WindowView>(&mut self) {
        self.registered_views.insert(
            T::id(),
            Box::new(|runtime| {
                let (view, task) = T::boot(runtime);
                (
                    Box::new(view),
                    task.map(|o| ErasedWindowViewMessage {
                        view: T::id(),
                        message: Box::new(o) as Box<dyn Any + Send>,
                    }),
                )
            }),
        );
    }

    pub fn set_root_view<T: WindowView>(&mut self) {
        self.root_view = Some(T::id());
    }

    pub fn root_view(&self) -> Option<WindowViewId> {
        self.root_view
    }

    pub fn boot(&mut self, services: &mut Services) -> Task<WindowViewManagerMessage> {
        self.open_window_view(self.root_view.expect("No root view specified."), services)
    }

    pub fn view<'a>(
        &'a self,
        id: window::Id,
        services: &'a Services,
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

        Some(view.state.view(id, services).map(move |msg| {
            WindowViewManagerMessage::ViewUpdate(ErasedWindowViewMessage {
                view: window,
                message: msg,
            })
        }))
    }

    pub fn update(
        &mut self,
        message: WindowViewManagerMessage,
        services: &mut Services,
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

                let task = view
                    .state
                    .update(message.message, services)
                    .map(move |msg| {
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

    pub fn subscription(&self, services: &Services) -> Subscription<WindowViewManagerMessage> {
        Subscription::batch(self.opened_views.iter().map(|(id, view)| {
            view.state
                .subscription(services)
                .with(*id)
                .map(|(view, message)| {
                    WindowViewManagerMessage::ViewUpdate(ErasedWindowViewMessage { view, message })
                })
        }))
    }

    pub fn open_window_view(
        &mut self,
        view_id: WindowViewId,
        services: &mut Services,
    ) -> Task<WindowViewManagerMessage> {
        if self.opened_views.contains_key(&view_id) {
            log::warn!("Window view already opened: {}", view_id.0);
            return Task::none();
        }

        let Some(boot) = self.registered_views.get(&view_id) else {
            log::error!(
                "Unable to open a window view that is not registered: {}",
                view_id.0
            );
            return Task::none();
        };

        let (view_state, task) = boot(services);
        let mut view = OpenedView::new(view_state);
        update_view_windows(view_id, &mut view, &mut self.window_to_view);
        self.opened_views.insert(view_id, view);

        task.map(WindowViewManagerMessage::ViewUpdate)
    }

    pub fn close_window_view(
        &mut self,
        view_id: WindowViewId,
        services: &mut Services,
    ) -> Task<()> {
        let Some(view) = self.opened_views.remove(&view_id) else {
            return Task::none();
        };

        view.state.close(services)
    }

    pub fn on_window_closed(&mut self, window: window::Id, services: &mut Services) -> Task<()> {
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
            entry.remove().state.close(services)
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

pub trait WindowCommand: Send + Sync + 'static {
    fn execute(
        self: Box<Self>,
        wm: &mut WindowViewManager,
        services: &mut Services,
    ) -> Option<Task<WindowViewManagerMessage>>;
}

#[derive(Default)]
pub struct WindowCommandBuffer {
    commands: Vec<Box<dyn WindowCommand>>,
}

impl Service for WindowCommandBuffer {}

impl WindowCommandBuffer {
    pub fn push<T: WindowCommand>(&mut self, command: T) {
        self.commands.push(Box::new(command));
    }

    pub fn execute(
        &mut self,
        wm: &mut WindowViewManager,
        services: &mut Services,
    ) -> Task<WindowViewManagerMessage> {
        let mut tasks = Vec::new();
        for command in self.commands.drain(..) {
            if let Some(task) = command.execute(wm, services) {
                tasks.push(task);
            }
        }
        Task::batch(tasks)
    }
}

pub struct OpenWindowViewCommand {
    view_id: WindowViewId,
}

impl WindowCommand for OpenWindowViewCommand {
    fn execute(
        self: Box<Self>,
        wm: &mut WindowViewManager,
        services: &mut Services,
    ) -> Option<Task<WindowViewManagerMessage>> {
        Some(wm.open_window_view(self.view_id, services))
    }
}

impl OpenWindowViewCommand {
    pub fn new(view_id: WindowViewId) -> Self {
        Self { view_id }
    }
}

pub struct CloseWindowViewCommand {
    view_id: WindowViewId,
}

impl WindowCommand for CloseWindowViewCommand {
    fn execute(
        self: Box<Self>,
        wm: &mut WindowViewManager,
        services: &mut Services,
    ) -> Option<Task<WindowViewManagerMessage>> {
        Some(wm.close_window_view(self.view_id, services).discard())
    }
}

impl CloseWindowViewCommand {
    pub fn new(view_id: WindowViewId) -> Self {
        Self { view_id }
    }
}

pub struct ToggleWindowViewCommand {
    view_id: WindowViewId,
}

impl WindowCommand for ToggleWindowViewCommand {
    fn execute(
        self: Box<Self>,
        wm: &mut WindowViewManager,
        services: &mut Services,
    ) -> Option<Task<WindowViewManagerMessage>> {
        if wm.opened_views.contains_key(&self.view_id) {
            Some(wm.close_window_view(self.view_id, services).discard())
        } else {
            Some(wm.open_window_view(self.view_id, services))
        }
    }
}

impl ToggleWindowViewCommand {
    pub fn new(view_id: WindowViewId) -> Self {
        Self { view_id }
    }
}

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
        _services: &mut Services,
    ) -> Option<Task<WindowViewManagerMessage>> {
        wm.window_to_view.insert(self.window, self.view_id);
        None
    }
}
