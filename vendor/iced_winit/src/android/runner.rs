use crate::Proxy;
use crate::core::backend;
use crate::core::renderer;
use crate::core::theme;
use crate::futures::Executor;
use crate::futures::Runtime;
use crate::futures::futures::channel::{mpsc, oneshot};
use crate::futures::futures::task;
use crate::futures::futures::task::Poll;
use crate::futures::subscription;
use crate::program::{self, Program};
use crate::runtime::{Action, Task};
use crate::{Error, debug};

use std::sync::Arc;

/// An event delivered to the Android runtime.
pub(crate) enum Event<Message: 'static> {
    EventLoopAwakened(EventLoopEvent<Message>),
    Exit,
}

/// An event of the event loop, mirroring the relevant parts of
/// `winit::application` for the single native window of an Android app.
pub(crate) enum EventLoopEvent<Message: 'static> {
    NewEvents(winit::event::StartCause),
    UserEvent(Message),
    WindowEvent(winit::event::WindowEvent),
    SurfacesCreated {
        window: Arc<dyn winit::window::Window>,
    },
    SurfacesDestroyed,
    AboutToWait,
}

/// A control action produced by the runtime.
#[derive(Debug)]
pub(crate) enum Control {
    ChangeFlow(winit::event_loop::ControlFlow),
    Exit,
    Crash(Error),
}

/// Runs a [`Program`] on Android with the provided [`AndroidApp`].
pub fn run<P>(
    program: P,
    android_app: winit::platform::android::activity::AndroidApp,
) -> Result<(), Error>
where
    P: Program + 'static,
    P::Theme: theme::Base,
{
    use winit::event_loop::EventLoop;
    use winit::platform::android::EventLoopBuilderExtAndroid as _;

    let boot_span = debug::boot();
    let settings = program.settings();
    let window_settings = program.window();

    let event_loop = EventLoop::builder()
        .with_android_app(android_app)
        .build()
        .expect("Create event loop");

    let backend_settings = backend::Settings::from(&settings);
    let renderer_settings = renderer::Settings::from(&settings);
    let display_handle = event_loop.owned_display_handle();

    let (proxy, worker, outbox) = Proxy::new(event_loop.create_proxy());

    #[cfg(feature = "debug")]
    {
        let proxy = proxy.clone();

        debug::on_hotpatch(move || {
            proxy.send_action(Action::Reload);
        });
    }

    let mut runtime = {
        let executor = P::Executor::new().map_err(Error::ExecutorCreationFailed)?;
        executor.spawn(worker);

        Runtime::new(executor, proxy.clone())
    };

    let (program, task) = runtime.enter(|| program::Instance::new(program));
    let is_daemon = window_settings.is_none();

    let task = if let Some(window_settings) = window_settings {
        let mut task = Some(task);

        let (_id, open) = crate::runtime::window::open(window_settings);

        open.then(move |_| task.take().unwrap_or_else(Task::none))
    } else {
        task
    };

    if let Some(stream) = crate::runtime::task::into_stream(task) {
        runtime.run(stream);
    }

    runtime.track(subscription::into_recipes(
        runtime.enter(|| program.subscription().map(Action::Output)),
    ));

    let (event_sender, event_receiver) = mpsc::unbounded();
    let (control_sender, control_receiver) = mpsc::unbounded();
    let (system_theme_sender, system_theme_receiver) = oneshot::channel();

    let instance = Box::pin(crate::android::instance::run::<P>(
        program,
        runtime,
        proxy.clone(),
        event_receiver,
        control_sender,
        display_handle,
        is_daemon,
        backend_settings,
        renderer_settings,
        settings.fonts,
        system_theme_receiver,
    ));

    let context = task::Context::from_waker(task::noop_waker_ref());

    struct Runner<Message: 'static, F> {
        instance: std::pin::Pin<Box<F>>,
        context: task::Context<'static>,
        sender: mpsc::UnboundedSender<Event<Action<Message>>>,
        receiver: mpsc::UnboundedReceiver<Control>,
        outbox: mpsc::UnboundedReceiver<Action<Message>>,
        error: Option<Error>,
        system_theme: Option<oneshot::Sender<theme::Mode>>,
        surfaces_ready: bool,
    }

    let runner = Runner {
        instance,
        context,
        sender: event_sender,
        receiver: control_receiver,
        outbox,
        error: None,
        system_theme: Some(system_theme_sender),
        surfaces_ready: false,
    };

    boot_span.finish();

    impl<Message, F> winit::application::ApplicationHandler for Runner<Message, F>
    where
        F: Future<Output = ()>,
    {
        fn resumed(&mut self, event_loop: &dyn winit::event_loop::ActiveEventLoop) {
            if let Some(sender) = self.system_theme.take() {
                let _ = sender.send(
                    event_loop
                        .system_theme()
                        .map(crate::conversion::theme_mode)
                        .unwrap_or_default(),
                );
            }
        }

        fn suspended(&mut self, _event_loop: &dyn winit::event_loop::ActiveEventLoop) {
            // The surface lifecycle is driven by `can_create_surfaces` and
            // `destroy_surfaces`.
        }

        fn can_create_surfaces(&mut self, event_loop: &dyn winit::event_loop::ActiveEventLoop) {
            let window = event_loop
                .create_window(winit::window::WindowAttributes::default())
                .expect("Create native window");

            self.surfaces_ready = true;

            self.process_event(
                event_loop,
                Event::EventLoopAwakened(EventLoopEvent::SurfacesCreated {
                    window: Arc::from(window),
                }),
            );

            self.proxy_wake_up(event_loop);
        }

        fn destroy_surfaces(&mut self, event_loop: &dyn winit::event_loop::ActiveEventLoop) {
            self.process_event(
                event_loop,
                Event::EventLoopAwakened(EventLoopEvent::SurfacesDestroyed),
            );

            self.surfaces_ready = false;
        }

        fn new_events(
            &mut self,
            event_loop: &dyn winit::event_loop::ActiveEventLoop,
            cause: winit::event::StartCause,
        ) {
            self.process_event(
                event_loop,
                Event::EventLoopAwakened(EventLoopEvent::NewEvents(cause)),
            );
        }

        fn window_event(
            &mut self,
            event_loop: &dyn winit::event_loop::ActiveEventLoop,
            _window_id: winit::window::WindowId,
            event: winit::event::WindowEvent,
        ) {
            // Every winit window on Android shares the same id; the events
            // are routed to the logical windows by the runtime.
            self.process_event(
                event_loop,
                Event::EventLoopAwakened(EventLoopEvent::WindowEvent(event)),
            );
        }

        fn proxy_wake_up(&mut self, event_loop: &dyn winit::event_loop::ActiveEventLoop) {
            if !self.surfaces_ready {
                return;
            }

            while let Ok(action) = self.outbox.try_recv() {
                self.process_event(
                    event_loop,
                    Event::EventLoopAwakened(EventLoopEvent::UserEvent(action)),
                );
            }
        }

        fn about_to_wait(&mut self, event_loop: &dyn winit::event_loop::ActiveEventLoop) {
            self.process_event(
                event_loop,
                Event::EventLoopAwakened(EventLoopEvent::AboutToWait),
            );
        }
    }

    impl<Message, F> Runner<Message, F>
    where
        F: Future<Output = ()>,
    {
        fn process_event(
            &mut self,
            event_loop: &dyn winit::event_loop::ActiveEventLoop,
            event: Event<Action<Message>>,
        ) {
            if event_loop.exiting() {
                return;
            }

            self.sender.start_send(event).expect("Send event");

            loop {
                let poll = self.instance.as_mut().poll(&mut self.context);

                match poll {
                    Poll::Pending => match self.receiver.try_recv() {
                        Ok(control) => match control {
                            Control::ChangeFlow(flow) => {
                                use winit::event_loop::ControlFlow;

                                match (event_loop.control_flow(), flow) {
                                    (
                                        ControlFlow::WaitUntil(current),
                                        ControlFlow::WaitUntil(new),
                                    ) if current < new => {}
                                    (ControlFlow::WaitUntil(target), ControlFlow::Wait)
                                        if target > crate::core::time::Instant::now() => {}
                                    _ => {
                                        event_loop.set_control_flow(flow);
                                    }
                                }
                            }
                            Control::Exit => {
                                self.process_event(event_loop, Event::Exit);
                                event_loop.exit();
                                break;
                            }
                            Control::Crash(error) => {
                                self.error = Some(error);
                                event_loop.exit();
                            }
                        },
                        _ => {
                            break;
                        }
                    },
                    Poll::Ready(_) => {
                        event_loop.exit();
                        break;
                    }
                };
            }
        }
    }

    let mut runner = runner;
    let error = runner.error.take();
    let _ = event_loop.run_app(runner);

    error.map(Err).unwrap_or(Ok(()))
}
