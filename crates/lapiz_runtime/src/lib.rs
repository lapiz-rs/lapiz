#![expect(
    clippy::pub_use,
    reason = "Event derive expansions use the root __private path"
)]

use std::{
    any::{Any, TypeId},
    cell::{Ref, RefCell, RefMut},
    collections::{HashMap, VecDeque},
};

use iced_core::{Element, Length, Widget, window};
use iced_futures::{Subscription, backend::native, event::listen_with};
use iced_runtime::{Task, window::raw_id};
use iced_winit::program::Program;

use crate::{
    plugin::Plugin,
    service::{FromServices, Service},
    windows::{WindowCommandBuffer, WindowViewManager, WindowViewManagerMessage},
};

pub mod event;
#[doc(hidden)]
pub use event::__private;
#[cfg(target_os = "android")]
pub mod android;
pub mod platform;
pub mod plugin;
pub mod renderer;
pub mod service;
pub mod windows;

pub type Renderer = renderer::Renderer;
pub type Theme = iced_core::Theme;

pub struct ApplicationTheme(pub Theme);

impl Service for ApplicationTheme {}

pub enum ApplicationState {
    Adding,
    Built,
    Finished,
}

pub struct Application {
    state: ApplicationState,
    // TODO remove this ref cell
    runtime: RefCell<Runtime>,
    plugins: VecDeque<Box<dyn Plugin>>,
}

impl Application {
    pub fn add_plugin<P: Plugin>(&mut self, plugin: P) -> &mut Self {
        if !matches!(self.state, ApplicationState::Adding) {
            panic!("Plugins can only be added in the Adding state");
        }

        self.plugins.push_back(Box::new(plugin));
        self
    }

    pub fn add_service<T: Service + FromServices>(&mut self) -> &mut Self {
        self.runtime.borrow_mut().add_service::<T>();
        self
    }

    pub fn add_service_instance<T: Service>(&mut self, service: T) -> &mut Self {
        self.runtime.borrow_mut().add_service_instance(service);
        self
    }

    pub fn runtime(&self) -> Ref<'_, Runtime> {
        self.runtime.borrow()
    }

    pub fn runtime_mut(&mut self) -> RefMut<'_, Runtime> {
        self.runtime.borrow_mut()
    }

    pub fn build_plugins(&mut self) {
        let mut plugins = Vec::with_capacity(self.plugins.len());
        while let Some(plugin) = self.plugins.pop_front() {
            plugin.build(self);
            plugins.push(plugin);
        }
        self.state = ApplicationState::Built;

        for plugin in plugins {
            plugin.finish(self);
        }
        self.state = ApplicationState::Finished;
    }

    pub fn run(
        self,
        #[cfg(target_os = "android")] android_app: winit::platform::android::activity::AndroidApp,
    ) -> Result<(), iced_winit::Error> {
        if !matches!(self.state, ApplicationState::Finished) {
            panic!("Plugins must be built before running the application");
        }

        #[cfg(target_os = "android")]
        {
            let mut font_system = iced_graphics::text::font_system()
                .write()
                .expect("Font system");
            let db = font_system.raw().db_mut();
            db.load_fonts_dir("/system/fonts");
            db.set_sans_serif_family("Roboto");
            log::info!("Loaded {} Android font faces", db.len());
            drop(font_system);

            iced_winit::run_android(self, android_app)
        }

        #[cfg(not(target_os = "android"))]
        iced_winit::run(self)
    }
}

impl Default for Application {
    fn default() -> Self {
        let mut runtime = Runtime::default();
        runtime.add_service_instance(ApplicationTheme(Theme::Dark));
        Self {
            state: ApplicationState::Adding,
            runtime: RefCell::new(runtime),
            plugins: VecDeque::new(),
        }
    }
}

impl Program for Application {
    type State = Runtime;

    type Message = ApplicationMessage;

    type Theme = Theme;

    type Renderer = renderer::Renderer;

    type Executor = native::smol::Executor;

    fn name() -> &'static str {
        "Lapiz Runtime"
    }

    fn settings(&self) -> iced_core::Settings {
        Default::default()
    }

    fn window(&self) -> Option<window::Settings> {
        None
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        let mut rt = std::mem::take::<Runtime>(&mut self.runtime.borrow_mut());

        let window_task = rt
            .wm
            .boot(None, &mut rt.services)
            .map(ApplicationMessage::Window);
        let deadlock_detect_task = Task::future(async {
            loop {
                smol::Timer::after(std::time::Duration::from_secs(5)).await;
                let deadlocks = parking_lot::deadlock::check_deadlock();
                for (i_dl, threads) in deadlocks.into_iter().enumerate() {
                    log::error!("#{} Deadlock detected", i_dl);

                    for (it, t) in threads.into_iter().enumerate() {
                        log::error!("Thread {}:", it);
                        log::error!("{:#?}", t.backtrace());
                    }
                }
            }
        });
        (
            rt,
            Task::batch([window_task, deadlock_detect_task.discard()]),
        )
    }

    fn theme(&self, state: &Self::State, _window: window::Id) -> Option<Self::Theme> {
        Some(state.services.service::<ApplicationTheme>().0.clone())
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        let mut task = match message {
            ApplicationMessage::Window(m) => state
                .wm
                .update(m, &mut state.services)
                .map(ApplicationMessage::Window),
            ApplicationMessage::WindowOpened(id) => {
                raw_id::<()>(id).map(move |raw_id| ApplicationMessage::WindowRawId(id, raw_id))
            }
            ApplicationMessage::WindowRawId(_, raw_id) => {
                platform::attach_resize_handle(raw_id);
                Task::none()
            }
            ApplicationMessage::WindowClosed(id) => {
                state.wm.on_window_closed(id, &mut state.services).discard()
            }
        };

        let mut cmd = std::mem::take(state.services.service_mut::<WindowCommandBuffer>());
        task = task.chain(cmd.execute(&mut state.wm, &mut state.services).discard());

        task
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        window: window::Id,
    ) -> Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        struct DummyWidget;
        impl Widget<ApplicationMessage, Theme, Renderer> for DummyWidget {
            fn size(&self) -> iced_core::Size<iced_core::Length> {
                iced_core::Size::new(iced_core::Length::Fill, iced_core::Length::Fill)
            }

            fn layout(
                &mut self,
                _tree: &mut iced_core::widget::Tree,
                _renderer: &Renderer,
                limits: &iced_core::layout::Limits,
            ) -> iced_core::layout::Node {
                iced_core::layout::atomic(limits, Length::Fill, Length::Fill)
            }

            fn draw(
                &self,
                _tree: &iced_core::widget::Tree,
                _renderer: &mut Renderer,
                _theme: &Theme,
                _style: &iced_core::renderer::Style,
                _layout: iced_core::Layout<'_>,
                _cursor: iced_core::pointer::mouse::Cursor,
                _viewport: &iced_core::Rectangle,
            ) {
            }
        }

        state
            .wm
            .view(window, &state.services)
            .map(|e| e.map(ApplicationMessage::Window))
            .unwrap_or_else(|| Element::new(DummyWidget))
    }

    fn subscription(&self, state: &Self::State) -> Subscription<Self::Message> {
        let windows = state
            .wm
            .subscription(&state.services)
            .map(ApplicationMessage::Window);
        let external = listen_with(|event, _, window_id| match event {
            iced_core::Event::Window(event) => match event {
                window::Event::Opened { .. } => Some(ApplicationMessage::WindowOpened(window_id)),
                window::Event::Closed => Some(ApplicationMessage::WindowClosed(window_id)),
                _ => None,
            },
            _ => None,
        });

        Subscription::batch([windows, external])
    }
}

#[derive(Default)]
pub struct Runtime {
    services: Services,
    wm: WindowViewManager,
}

impl Runtime {
    pub fn add_service<T: Service + FromServices>(&mut self) -> &mut Self {
        let instance = T::from_services(&self.services);
        self.add_service_instance(instance);
        self
    }

    pub fn add_service_instance<T: Service>(&mut self, service: T) -> &mut Self {
        self.services
            .services
            .insert(TypeId::of::<T>(), Box::new(service));
        self
    }

    pub fn services(&self) -> &Services {
        &self.services
    }

    pub fn services_mut(&mut self) -> &mut Services {
        &mut self.services
    }

    pub fn window_manager(&self) -> &WindowViewManager {
        &self.wm
    }

    pub fn window_manager_mut(&mut self) -> &mut WindowViewManager {
        &mut self.wm
    }
}

pub enum ApplicationMessage {
    Window(WindowViewManagerMessage),
    WindowOpened(window::Id),
    WindowRawId(window::Id, u64),
    WindowClosed(window::Id),
}

#[derive(Default)]
pub struct Services {
    services: HashMap<TypeId, Box<dyn Any>>,
}

impl Services {
    pub fn service<T: Service>(&self) -> &T {
        self.services
            .get(&TypeId::of::<T>())
            .unwrap_or_else(|| panic!("Service of type {} not found", std::any::type_name::<T>()))
            .downcast_ref()
            .unwrap_or_else(|| {
                panic!(
                    "Service of type {} has wrong type. This should not happen.",
                    std::any::type_name::<T>()
                )
            })
    }

    pub fn service_mut<T: Service>(&mut self) -> &mut T {
        self.services
            .get_mut(&TypeId::of::<T>())
            .unwrap_or_else(|| panic!("Service of type {} not found", std::any::type_name::<T>()))
            .downcast_mut()
            .unwrap_or_else(|| {
                panic!(
                    "Service of type {} has wrong type. This should not happen.",
                    std::any::type_name::<T>()
                )
            })
    }

    pub fn has_service<T: Service>(&self) -> bool {
        self.services.contains_key(&TypeId::of::<T>())
    }

    pub fn get_service<T: Service>(&self) -> Option<&T> {
        self.services
            .get(&TypeId::of::<T>())
            .and_then(|service| service.downcast_ref())
    }

    pub fn get_service_mut<T: Service>(&mut self) -> Option<&mut T> {
        self.services
            .get_mut(&TypeId::of::<T>())
            .and_then(|service| service.downcast_mut())
    }

    pub fn remove_service<T: Service>(&mut self) -> T {
        let s = self
            .services
            .remove(&TypeId::of::<T>())
            .unwrap_or_else(|| panic!("Service of type {} not found", std::any::type_name::<T>()));

        match s.downcast() {
            Ok(s) => *s,
            Err(_) => {
                panic!(
                    "Service of type {} has wrong type. This should not happen.",
                    std::any::type_name::<T>()
                )
            }
        }
    }

    pub fn try_remove_service<T: Service>(&mut self) -> Option<T> {
        let s = self.services.remove(&TypeId::of::<T>())?;

        match s.downcast() {
            Ok(s) => Some(*s),
            Err(_) => {
                panic!(
                    "Service of type {} has wrong type. This should not happen.",
                    std::any::type_name::<T>()
                )
            }
        }
    }

    pub fn insert_service<T: Service>(&mut self, service: T) {
        self.services.insert(TypeId::of::<T>(), Box::new(service));
    }

    pub fn service_scope<T: Service, O>(&mut self, f: impl FnOnce(&mut T, &mut Self) -> O) -> O {
        let mut s = self.remove_service::<T>();
        let result = f(&mut s, self);
        self.insert_service(s);
        result
    }

    pub fn try_service_scope<T: Service, O>(
        &mut self,
        f: impl FnOnce(&mut T, &mut Self) -> O,
    ) -> Option<O> {
        let mut s = self.try_remove_service::<T>()?;
        let result = f(&mut s, self);
        self.insert_service(s);
        Some(result)
    }
}
