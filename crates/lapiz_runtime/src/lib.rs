use std::{cell::RefCell, collections::VecDeque, mem, time::Duration};

use iced_core::{
    Element, Length, Widget, layout, pointer::mouse, renderer::Style, widget::Tree, window,
};
use iced_futures::{Subscription, backend::native, event::listen_with};
#[cfg(target_os = "android")]
use iced_graphics::text::font_system;
use iced_runtime::{Task, window::raw_id};
use iced_winit::program::Program;
use parking_lot::deadlock;
#[cfg(target_os = "android")]
use winit::platform::android::activity::AndroidApp;

use crate::{
    global::{FromGlobals, Global, Globals},
    plugin::Plugin,
    renderer::global_render_context,
    windows::{WindowCommandBuffer, WindowView, WindowViewManager, WindowViewManagerMessage},
};

#[cfg(target_os = "android")]
pub mod android;
pub mod event;
pub mod global;
pub mod platform;
pub mod plugin;
pub mod renderer;
pub mod windows;

pub type Renderer = renderer::Renderer;
pub type Theme = iced_core::Theme;

/// Global resource storing the active UI [`Theme`].
pub struct ApplicationTheme(pub Theme);

impl Global for ApplicationTheme {}

/// Lifecycle stage of an [`Application`] during plugin registration and initialization.
pub enum ApplicationState {
    Adding,
    Built,
    Finished,
}

/// Top-level entry point that configures plugins, initializes the [`Runtime`], and drives the event loop.
///
/// `Application` coordinates startup in three phases tracked by [`ApplicationState`]:
///
/// 1. **Registration (`Adding`)**: Plugins are queued via [`add_plugin`](Application::add_plugin).
/// 2. **Building (`Built` -> `Finished`)**: Calling [`build_plugins`](Application::build_plugins)
///    invokes [`Plugin::build`] on every registered plugin in order, followed by [`Plugin::finish`].
/// 3. **Execution**: Calling [`run`](Application::run) hands control to `iced_winit`, booting the
///    root [`WindowView`] and processing window, view, and command buffer updates.
pub struct Application {
    state: ApplicationState,
    // TODO remove this ref cell
    runtime: RefCell<Runtime>,
    plugins: VecDeque<Box<dyn Plugin>>,
}

impl Application {
    #[allow(
        clippy::new_without_default,
        reason = "It's invalid to made up an AndroidApp"
    )]
    pub fn new(#[cfg(target_os = "android")] android_app: AndroidApp) -> Self {
        let mut runtime = Runtime::default();
        runtime
            .add_global_instance(global_render_context())
            .add_global::<WindowCommandBuffer>()
            .add_global_instance(ApplicationTheme(Theme::Dark));

        #[cfg(target_os = "android")]
        runtime.add_global_instance(android::AndroidApp::new(android_app.clone()));

        Self {
            state: ApplicationState::Adding,
            runtime: RefCell::new(runtime),
            plugins: VecDeque::new(),
        }
    }

    pub fn add_plugin<P: Plugin>(&mut self, plugin: P) -> &mut Self {
        if !matches!(self.state, ApplicationState::Adding) {
            panic!("Plugins can only be added in the Adding state");
        }

        self.plugins.push_back(Box::new(plugin));
        self
    }

    pub fn build_plugins(&mut self) {
        let mut plugins = Vec::with_capacity(self.plugins.len());
        while let Some(plugin) = self.plugins.pop_front() {
            plugin.build(&mut self.runtime.borrow_mut());
            plugins.push(plugin);
        }
        self.state = ApplicationState::Built;

        for plugin in plugins {
            plugin.finish(&mut self.runtime.borrow_mut());
        }
        self.state = ApplicationState::Finished;
    }

    pub fn run(self) -> Result<(), iced_winit::Error> {
        if !matches!(self.state, ApplicationState::Finished) {
            panic!("Plugins must be built before running the application");
        }

        #[cfg(target_os = "android")]
        {
            let mut font_system = font_system().write().expect("Font system");
            let db = font_system.raw().db_mut();
            db.load_fonts_dir("/system/fonts");
            db.set_sans_serif_family("Roboto");
            log::info!("Loaded {} Android font faces", db.len());
            drop(font_system);

            let runtime = self.runtime.borrow();
            let android_app = runtime.globals.global::<android::AndroidApp>().0.clone();
            drop(runtime);

            iced_winit::run_android(self, android_app)
        }

        #[cfg(not(target_os = "android"))]
        iced_winit::run(self)
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
        let mut rt = mem::take::<Runtime>(&mut self.runtime.borrow_mut());

        let window_task = rt
            .wm
            .boot(None, &mut rt.globals)
            .map(ApplicationMessage::Window);
        let deadlock_detect_task = Task::future(async {
            loop {
                smol::Timer::after(Duration::from_secs(5)).await;
                let deadlocks = deadlock::check_deadlock();
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
        Some(state.globals.global::<ApplicationTheme>().0.clone())
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        let mut task = match message {
            ApplicationMessage::Window(m) => state
                .wm
                .update(m, &mut state.globals)
                .map(ApplicationMessage::Window),
            ApplicationMessage::WindowOpened(id) => {
                raw_id::<()>(id).map(move |raw_id| ApplicationMessage::WindowRawId(id, raw_id))
            }
            ApplicationMessage::WindowRawId(_, raw_id) => {
                platform::attach_resize_handle(raw_id);
                Task::none()
            }
            ApplicationMessage::WindowClosed(id) => {
                state.wm.on_window_closed(id, &mut state.globals).discard()
            }
        };

        let mut cmd = mem::take(state.globals.global_mut::<WindowCommandBuffer>());
        task = task.chain(cmd.execute(&mut state.wm, &mut state.globals).discard());

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
                _tree: &mut Tree,
                _renderer: &Renderer,
                limits: &layout::Limits,
            ) -> layout::Node {
                layout::atomic(limits, Length::Fill, Length::Fill)
            }

            fn draw(
                &self,
                _tree: &Tree,
                _renderer: &mut Renderer,
                _theme: &Theme,
                _style: &Style,
                _layout: iced_core::Layout<'_>,
                _cursor: mouse::Cursor,
                _viewport: &iced_core::Rectangle,
            ) {
            }
        }

        state
            .wm
            .view(window, &state.globals)
            .map(|e| e.map(ApplicationMessage::Window))
            .unwrap_or_else(|| Element::new(DummyWidget))
    }

    fn subscription(&self, state: &Self::State) -> Subscription<Self::Message> {
        let windows = state
            .wm
            .subscription(&state.globals)
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

/// Core application state holding [`Globals`] and the [`WindowViewManager`].
///
/// During startup, [`Plugin`]s mutate `Runtime` to insert [`Global`] resources and register
/// [`WindowView`] types. Once the application boots, `Runtime` serves as the `iced` program
/// state, routing window events and view messages through the [`WindowViewManager`].
#[derive(Default)]
pub struct Runtime {
    globals: Globals,
    wm: WindowViewManager,
}

impl Runtime {
    pub fn add_global<T: Global + FromGlobals>(&mut self) -> &mut Self {
        let instance = T::from_globals(&self.globals);
        self.add_global_instance(instance);
        self
    }

    pub fn add_global_instance<T: Global>(&mut self, value: T) -> &mut Self {
        self.globals.insert_global(value);
        self
    }

    pub fn register_view<T: WindowView>(&mut self) -> &mut Self {
        self.wm.register_view::<T>();
        self
    }

    pub fn globals(&self) -> &Globals {
        &self.globals
    }

    pub fn globals_mut(&mut self) -> &mut Globals {
        &mut self.globals
    }

    pub fn window_manager(&self) -> &WindowViewManager {
        &self.wm
    }

    pub fn window_manager_mut(&mut self) -> &mut WindowViewManager {
        &mut self.wm
    }
}

/// Top-level message routed by [`Application`] between the windowing backend and [`WindowViewManager`].
pub enum ApplicationMessage {
    Window(WindowViewManagerMessage),
    WindowOpened(window::Id),
    WindowRawId(window::Id, u64),
    WindowClosed(window::Id),
}
