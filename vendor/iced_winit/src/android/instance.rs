use crate::Proxy;
use crate::android::actions;
use crate::android::container::{Window, Windows};
use crate::android::input::{PhysicalBounds, Routed, Router};
use crate::android::manager::{LogicalWindow, Manager, Screen, initial_geometry};
use crate::android::render;
use crate::android::runner::{Control, Event, EventLoopEvent};
use crate::clipboard::Clipboard;
use crate::conversion;
use crate::core;
use crate::core::backend;
use crate::core::pointer::mouse;
use crate::core::renderer;
use crate::core::shell;
use crate::core::theme;
use crate::core::time::Instant;
use crate::core::window::{self, Id};
use crate::core::{Element, Point, Rectangle, Size};
use crate::futures::Runtime;
use crate::futures::futures::StreamExt;
use crate::futures::futures::channel::{mpsc, oneshot};
use crate::futures::subscription;
use crate::graphics::{Compositor as _, Shell, compositor};
use crate::program::{self, Program};
use crate::runtime;
use crate::runtime::Action;
use crate::runtime::user_interface::{self, UserInterface};
use crate::window::Preedit;

use std::borrow::Cow;
use std::mem::ManuallyDrop;
use std::sync::Arc;

pub(crate) type Interface<'a, P> =
    UserInterface<'a, <P as Program>::Message, <P as Program>::Theme, <P as Program>::Renderer>;

/// The non-`program` state of the Android runtime.
pub(crate) struct Loop<P>
where
    P: Program,
    P::Theme: theme::Base,
{
    pub(crate) runtime: Runtime<P::Executor, Proxy<P::Message>, Action<P::Message>>,
    pub(crate) proxy: Proxy<P::Message>,
    pub(crate) control_sender: mpsc::UnboundedSender<Control>,

    pub(crate) display_handle: winit::event_loop::OwnedDisplayHandle,
    pub(crate) backend_settings: backend::Settings,
    pub(crate) renderer_settings: renderer::Settings,
    pub(crate) default_fonts: Vec<Cow<'static, [u8]>>,

    /// The single native window, available between `can_create_surfaces`
    /// and `destroy_surfaces`.
    pub(crate) native: Option<Arc<dyn winit::window::Window>>,
    pub(crate) compositor: Option<<P::Renderer as compositor::Default>::Compositor>,
    pub(crate) renderer: Option<P::Renderer>,
    pub(crate) surface: Option<
        <<P::Renderer as compositor::Default>::Compositor as crate::graphics::Compositor>::Surface,
    >,
    /// The physical size the surface was last configured with.
    pub(crate) surface_size: Size<u32>,

    pub(crate) manager: Manager,
    pub(crate) router: Router,
    pub(crate) modifiers: winit::keyboard::ModifiersState,
    pub(crate) waker: Option<shell::Waker>,

    pub(crate) events: Vec<(Id, core::Event)>,
    pub(crate) messages: shell::Bus<P::Message>,
    pub(crate) clipboard: Clipboard,

    pub(crate) is_window_opening: bool,
    pub(crate) actions: usize,
    pub(crate) system_theme: theme::Mode,

    /// Whether the user interface must be rebuilt before the next frame.
    pub(crate) needs_rebuild: bool,
    pub(crate) redraw_at: Option<Instant>,
    pub(crate) mouse_interaction: mouse::Interaction,
    pub(crate) preedit: Option<Preedit<P::Renderer>>,
    pub(crate) ime_enabled: bool,
}

/// Runs the [`Program`] instance of the application, driving it with the
/// events collected by the [`crate::android::runner`].
///
/// The whole application is driven by a single
/// [`UserInterface`]: its root widget is a [`Windows`] container that lays
/// out one child per logical window. The user interface, the program
/// instance, and the interface cache are locals of this function because
/// the interface borrows the program while [`crate::update`] needs it
/// mutably, which is only expressible in a single function scope.
pub(crate) async fn run<P>(
    mut program: program::Instance<P>,
    runtime: Runtime<P::Executor, Proxy<P::Message>, Action<P::Message>>,
    proxy: Proxy<P::Message>,
    mut event_receiver: mpsc::UnboundedReceiver<Event<Action<P::Message>>>,
    control_sender: mpsc::UnboundedSender<Control>,
    display_handle: winit::event_loop::OwnedDisplayHandle,
    is_daemon: bool,
    backend_settings: backend::Settings,
    renderer_settings: renderer::Settings,
    default_fonts: Vec<Cow<'static, [u8]>>,
    mut system_theme: oneshot::Receiver<theme::Mode>,
) where
    P: Program + 'static,
    P::Theme: theme::Base,
{
    use winit::event::StartCause;
    use winit::event::WindowEvent;

    let system_theme = system_theme.try_recv().ok().flatten().unwrap_or_default();

    log::info!("System theme: {system_theme:?}");

    let mut loop_ = Loop {
        runtime,
        proxy,
        control_sender,
        display_handle,
        backend_settings,
        renderer_settings,
        default_fonts,
        native: None,
        compositor: None,
        renderer: None,
        surface: None,
        surface_size: Size::new(0, 0),
        manager: Manager::new(Screen {
            logical_size: Size::new(0.0, 0.0),
            scale_factor: 1.0,
        }),
        router: Router::new(),
        modifiers: winit::keyboard::ModifiersState::default(),
        waker: None,
        events: Vec::new(),
        messages: shell::Bus::new(),
        clipboard: Clipboard::new(),
        is_window_opening: !is_daemon,
        actions: 0,
        system_theme,
        needs_rebuild: false,
        redraw_at: None,
        mouse_interaction: mouse::Interaction::None,
        preedit: None,
        ime_enabled: false,
    };

    // `ManuallyDrop` mirrors the desktop runtime: the interface borrows
    // the program, so its slot must never run a destructor that could
    // overlap with the mutable borrows of `crate::update`.
    let mut interface = ManuallyDrop::new(None::<Interface<'_, P>>);
    let mut interface_cache = user_interface::Cache::default();

    loop {
        let event = if let Ok(event) = event_receiver.try_recv() {
            Some(event)
        } else {
            event_receiver.next().await
        };

        let Some(event) = event else {
            break;
        };

        match event {
            Event::EventLoopAwakened(EventLoopEvent::NewEvents(cause)) => match cause {
                StartCause::Init => {
                    loop_.request_redraw();
                }
                StartCause::ResumeTimeReached { .. } => {
                    let now = Instant::now();

                    if let Some(redraw_at) = loop_.redraw_at
                        && redraw_at <= now
                    {
                        loop_.request_redraw();
                        loop_.redraw_at = None;
                    }

                    set_control_flow(&mut loop_);
                }
                _ => {}
            },
            Event::EventLoopAwakened(EventLoopEvent::UserEvent(action)) => {
                actions::run(&mut loop_, &program, &mut interface, action);
                loop_.actions += 1;
            }
            Event::EventLoopAwakened(EventLoopEvent::SurfacesCreated { window }) => {
                let size = window.surface_size();

                loop_.native = Some(window);
                update_screen(&mut loop_, size);

                // The native window is recreated on every resume, so the
                // surface must be recreated too. The compositor survives.
                if loop_.compositor.is_some() {
                    loop_.recreate_surface();
                    loop_.request_redraw();
                }
            }
            Event::EventLoopAwakened(EventLoopEvent::SurfacesDestroyed) => {
                loop_.surface = None;
                loop_.native = None;
            }
            Event::EventLoopAwakened(EventLoopEvent::WindowEvent(event)) => {
                match event {
                    WindowEvent::RedrawRequested => {
                        use std::slice;

                        let Some(native) = loop_.native.clone() else {
                            continue;
                        };

                        let physical_size = native.surface_size();

                        if physical_size.width == 0 || physical_size.height == 0 {
                            continue;
                        }

                        // Keep the surface in sync with the native window
                        // size.
                        if loop_.surface_size.width != physical_size.width
                            || loop_.surface_size.height != physical_size.height
                        {
                            if let (Some(compositor), Some(surface)) =
                                (loop_.compositor.as_mut(), loop_.surface.as_mut())
                            {
                                compositor.configure_surface(
                                    surface,
                                    physical_size.width,
                                    physical_size.height,
                                );

                                loop_.surface_size =
                                    Size::new(physical_size.width, physical_size.height);
                            }
                        }

                        if loop_.renderer.is_none() || interface.is_none() {
                            render::present(&mut loop_, &native);

                            continue;
                        }

                        let redraw_event =
                            core::Event::Window(window::Event::RedrawRequested(Instant::now()));

                        let debug_id = loop_.manager.root_id().unwrap_or_else(Id::unique);

                        let mut redraw_count = 0;

                        let state = loop {
                            let message_count = loop_.messages.len();

                            let (state, _) = {
                                let Some(waker) = loop_.waker.clone() else {
                                    break user_interface::State::Outdated;
                                };

                                let Some(current) = interface.as_mut() else {
                                    break user_interface::State::Outdated;
                                };

                                let cursor = cursor(&loop_);

                                let renderer = loop_.renderer.as_mut().expect("Renderer exists");

                                current.update(
                                    &native,
                                    &waker,
                                    slice::from_ref(&redraw_event),
                                    cursor,
                                    renderer,
                                    &mut loop_.messages,
                                )
                            };

                            if message_count == loop_.messages.len() && !state.has_layout_changed()
                            {
                                break state;
                            }

                            if redraw_count >= 2 {
                                log::warn!(
                                    "More than 3 consecutive RedrawRequested events produced layout invalidation"
                                );

                                break state;
                            }

                            redraw_count += 1;

                            if !loop_.messages.is_empty()
                                || matches!(state, user_interface::State::Outdated)
                            {
                                let replaced =
                                    std::mem::replace(&mut interface, ManuallyDrop::new(None));

                                if let Some(current) = ManuallyDrop::into_inner(replaced) {
                                    interface_cache = current.into_cache();
                                }

                                let actions = crate::update(
                                    &mut program,
                                    &mut loop_.runtime,
                                    &mut loop_.messages,
                                );

                                interface = ManuallyDrop::new(build_interface(
                                    &mut loop_,
                                    &program,
                                    std::mem::take(&mut interface_cache),
                                ));

                                if interface.is_some() {
                                    interface_cache = user_interface::Cache::default();
                                }

                                for action in actions {
                                    // Defer window actions to avoid state
                                    // races while redrawing.
                                    if let Action::Window(_) = action {
                                        loop_.proxy.send_action(action);

                                        continue;
                                    }

                                    actions::run(&mut loop_, &program, &mut interface, action);
                                }

                                loop_.request_redraw();

                                if interface.is_none() {
                                    break user_interface::State::Outdated;
                                }
                            }
                        };

                        if let user_interface::State::Updated {
                            redraw_request,
                            input_method,
                            mouse_interaction,
                            clipboard: clipboard_requests,
                            ..
                        } = state
                        {
                            request_redraw_at(&*native, &mut loop_.redraw_at, redraw_request);

                            update_mouse_cursor(
                                native.as_ref(),
                                &mut loop_.mouse_interaction,
                                mouse_interaction,
                            );

                            if let Some(tag) = loop_.manager.root_id() {
                                crate::run_clipboard(
                                    &mut loop_.proxy,
                                    &mut loop_.clipboard,
                                    clipboard_requests,
                                    tag,
                                );
                            }

                            request_input_method(&mut loop_, &program, input_method);
                        }

                        if let Some(tag) = loop_.manager.root_id() {
                            loop_.runtime.broadcast(subscription::Event::Interaction {
                                window: tag,
                                event: redraw_event,
                                status: core::event::Status::Ignored,
                            });
                        }

                        let theme = loop_
                            .manager
                            .root_id()
                            .and_then(|id| program.theme(id))
                            .unwrap_or_else(|| {
                                <P::Theme as theme::Base>::default(loop_.system_theme)
                            });

                        let style = program.style(&theme);
                        let draw_style = renderer::Style {
                            text_color: style.text_color,
                        };

                        let draw_span = crate::debug::draw(debug_id);

                        {
                            let cursor = cursor(&loop_);

                            let Some(current) = interface.as_mut() else {
                                draw_span.finish();

                                render::present(&mut loop_, &native);

                                continue;
                            };

                            let renderer = loop_.renderer.as_mut().expect("Renderer exists");

                            current.draw(renderer, &theme, &draw_style, cursor);

                            if let Some(preedit) = &loop_.preedit {
                                let screen = loop_.manager.screen().logical_size;

                                preedit.draw(
                                    renderer,
                                    draw_style.text_color,
                                    style.background_color,
                                    &Rectangle::new(Point::ORIGIN, screen),
                                );
                            }
                        }

                        draw_span.finish();

                        render::present(&mut loop_, &native);
                    }
                    WindowEvent::SurfaceResized(size) => {
                        update_screen(&mut loop_, size);

                        if let (Some(compositor), Some(surface)) =
                            (loop_.compositor.as_mut(), loop_.surface.as_mut())
                        {
                            compositor.configure_surface(surface, size.width, size.height);
                            loop_.surface_size = Size::new(size.width, size.height);
                        }

                        loop_.request_redraw();
                    }
                    WindowEvent::ScaleFactorChanged { .. } => {
                        if let Some(native) = loop_.native.as_ref() {
                            let size = native.surface_size();

                            update_screen(&mut loop_, size);
                        }
                    }
                    WindowEvent::ThemeChanged(theme) => {
                        let mode = conversion::theme_mode(theme);

                        if mode != loop_.system_theme {
                            loop_.system_theme = mode;

                            loop_
                                .runtime
                                .broadcast(subscription::Event::SystemThemeChanged(mode));
                        }
                    }
                    WindowEvent::ModifiersChanged(modifiers) => {
                        loop_.modifiers = modifiers.state();
                    }
                    WindowEvent::PointerMoved { .. }
                    | WindowEvent::PointerEntered { .. }
                    | WindowEvent::PointerLeft { .. }
                    | WindowEvent::PointerButton { .. }
                    | WindowEvent::MouseWheel { .. } => {
                        pointer_event(&mut loop_, event);
                    }
                    WindowEvent::KeyboardInput { .. }
                    | WindowEvent::Ime(_)
                    | WindowEvent::Focused(_) => {
                        let tag = loop_.router.focused().or_else(|| loop_.manager.root_id());

                        if let Some(core_event) = conversion::window_event(
                            event,
                            loop_.manager.screen().scale_factor,
                            loop_.modifiers,
                        ) {
                            if let Some(tag) = tag {
                                loop_.events.push((tag, core_event));
                            }
                        }
                    }
                    // Android never delivers close or destruction events
                    // per window, and occlusion of the whole app is
                    // reported through `destroy_surfaces`.
                    _ => {}
                }
            }
            Event::EventLoopAwakened(EventLoopEvent::AboutToWait) => {
                if loop_.actions > 0 {
                    loop_.proxy.free_slots(loop_.actions);
                    loop_.actions = 0;
                }

                if loop_.events.is_empty()
                    && loop_.messages.is_empty()
                    && loop_.redraw_at.is_none()
                    && !loop_.needs_rebuild
                {
                    continue;
                }

                let mut uis_stale = false;
                let events = std::mem::take(&mut loop_.events);

                if !events.is_empty() {
                    uis_stale = !deliver_events(&mut loop_, &mut interface, events);
                }

                if !loop_.messages.is_empty() || uis_stale || loop_.needs_rebuild {
                    loop_.needs_rebuild = false;

                    let replaced = std::mem::replace(&mut interface, ManuallyDrop::new(None));

                    if let Some(current) = ManuallyDrop::into_inner(replaced) {
                        interface_cache = current.into_cache();
                    }

                    let actions =
                        crate::update(&mut program, &mut loop_.runtime, &mut loop_.messages);

                    interface = ManuallyDrop::new(build_interface(
                        &mut loop_,
                        &program,
                        std::mem::take(&mut interface_cache),
                    ));

                    if interface.is_some() {
                        interface_cache = user_interface::Cache::default();
                    }

                    for action in actions {
                        actions::run(&mut loop_, &program, &mut interface, action);
                    }

                    loop_.request_redraw();
                }

                set_control_flow(&mut loop_);
            }
            Event::Exit => break,
        }
    }

    let _ = ManuallyDrop::into_inner(interface);
}

/// Feeds the collected events to the user interface, returning whether an
/// up-to-date interface consumed them.
fn deliver_events<'a, P>(
    loop_: &mut Loop<P>,
    interface: &mut ManuallyDrop<Option<Interface<'a, P>>>,
    mut events: Vec<(Id, core::Event)>,
) -> bool
where
    P: Program,
    P::Theme: theme::Base,
{
    let Some(interface) = interface.as_mut() else {
        for (id, event) in events {
            loop_.runtime.broadcast(subscription::Event::Interaction {
                window: id,
                event,
                status: core::event::Status::Ignored,
            });
        }

        return false;
    };

    let cursor = cursor(loop_);

    let Some(native) = loop_.native.clone() else {
        return false;
    };

    let Some(waker) = loop_.waker.clone() else {
        return false;
    };

    let core_events: Vec<core::Event> = events.iter().map(|(_, event)| event.clone()).collect();

    let Some(renderer) = loop_.renderer.as_mut() else {
        return false;
    };

    let (state, statuses) = interface.update(
        &native,
        &waker,
        &core_events,
        cursor,
        renderer,
        &mut loop_.messages,
    );

    let tag = loop_.manager.root_id();

    match state {
        user_interface::State::Updated {
            redraw_request,
            mouse_interaction,
            clipboard: clipboard_requests,
            ..
        } => {
            if let Some(native) = &loop_.native {
                request_redraw_at(&**native, &mut loop_.redraw_at, redraw_request);

                update_mouse_cursor(
                    native.as_ref(),
                    &mut loop_.mouse_interaction,
                    mouse_interaction,
                );
            }

            if let Some(tag) = tag {
                crate::run_clipboard(
                    &mut loop_.proxy,
                    &mut loop_.clipboard,
                    clipboard_requests,
                    tag,
                );
            }
        }
        user_interface::State::Outdated => {}
    }

    let interactions: Vec<_> = events
        .drain(..)
        .zip(statuses)
        .map(|((id, event), status)| (id, event, status))
        .collect();

    for (id, event, status) in interactions {
        loop_.runtime.broadcast(subscription::Event::Interaction {
            window: id,
            event,
            status,
        });
    }

    true
}

/// Builds the user interface of the application: a [`Windows`] container
/// with one child per logical window.
fn build_interface<'a, P>(
    loop_: &mut Loop<P>,
    program: &'a program::Instance<P>,
    cache: user_interface::Cache,
) -> Option<Interface<'a, P>>
where
    P: Program,
    P::Theme: theme::Base,
{
    if loop_.manager.is_empty() {
        return None;
    }

    let Some(renderer) = loop_.renderer.as_mut() else {
        return None;
    };

    let theme = loop_
        .manager
        .root_id()
        .and_then(|id| program.theme(id))
        .unwrap_or_else(|| <P::Theme as theme::Base>::default(loop_.system_theme));

    let style = program.style(&theme);
    let screen = loop_.manager.screen();

    let mut windows: Windows<'a, P::Message, P::Theme, P::Renderer> = Windows::new();

    for id in loop_.manager.z_order().to_vec() {
        let Some(window) = loop_.manager.get(id) else {
            continue;
        };

        if !window.visible {
            continue;
        }

        windows = windows.push(Window {
            id,
            position: window.position,
            size: window.size,
            background: style.background_color,
            content: program.view(id),
        });
    }

    windows = windows.focus(loop_.router.focused());

    let element: Element<'a, P::Message, P::Theme, P::Renderer> = Element::new(windows);

    Some(UserInterface::build(
        element,
        screen.logical_size,
        cache,
        renderer,
    ))
}

/// The cursor of the user interface, from the last known pointer position.
fn cursor<P>(loop_: &Loop<P>) -> mouse::Cursor
where
    P: Program,
    P::Theme: theme::Base,
{
    match loop_.router.cursor() {
        Some(position) => {
            let scale = f64::from(loop_.manager.screen().scale_factor);
            let position = position.to_logical::<f32>(scale);

            mouse::Cursor::Available(Point::new(position.x, position.y))
        }
        None => mouse::Cursor::Unavailable,
    }
}

/// Updates the native cursor from a mouse interaction.
fn update_mouse_cursor(
    native: &dyn winit::window::Window,
    interaction: &mut mouse::Interaction,
    new_interaction: mouse::Interaction,
) {
    if new_interaction != *interaction {
        if let Some(icon) = conversion::mouse_interaction(new_interaction) {
            native.set_cursor(winit::cursor::Cursor::Icon(icon));

            if *interaction == mouse::Interaction::Hidden {
                native.set_cursor_visible(true);
            }
        } else {
            native.set_cursor_visible(false);
        }

        *interaction = new_interaction;
    }
}

/// Requests a redraw according to a [`window::RedrawRequest`], tracking
/// the scheduled instant.
fn request_redraw_at(
    native: &dyn winit::window::Window,
    redraw_at: &mut Option<Instant>,
    redraw_request: window::RedrawRequest,
) {
    match redraw_request {
        window::RedrawRequest::NextFrame => {
            native.request_redraw();
            *redraw_at = None;
        }
        window::RedrawRequest::At(at) => *redraw_at = Some(at),
        window::RedrawRequest::Wait => {}
    }
}

/// Routes a native pointer event.
fn pointer_event<P>(loop_: &mut Loop<P>, event: winit::event::WindowEvent)
where
    P: Program,
    P::Theme: theme::Base,
{
    let scale = loop_
        .native
        .as_ref()
        .map(|native| native.scale_factor())
        .unwrap_or(1.0);

    let result = loop_.router.pointer_event(event);

    match result {
        Routed::Deliver(event) => {
            // A press on the resize border of a resizable window starts
            // an interactive resize session instead of reaching its
            // content, like the resize border of a decorated window.
            if start_edge_resize(loop_, &event) {
                return;
            }

            let tag = loop_
                .router
                .cursor()
                .and_then(|position| {
                    let position = position.to_logical::<f32>(scale);

                    loop_.manager.hit_test(Point::new(position.x, position.y))
                })
                .or_else(|| loop_.manager.root_id())
                .or_else(|| loop_.manager.first_id());

            if let Some(core_event) = conversion::window_event(
                event,
                loop_.manager.screen().scale_factor,
                loop_.modifiers,
            ) {
                if let Some(tag) = tag {
                    // A pointer press focuses the logical window under the
                    // cursor for subsequent keyboard events.
                    if matches!(
                        core_event,
                        core::Event::Pointer(crate::core::pointer::Event::PointerPressed { .. })
                    ) {
                        focus_and_raise(loop_, tag);
                    }

                    loop_.events.push((tag, core_event));
                }
            }
        }
        Routed::Moved {
            id,
            position,
            release,
        } => {
            let position = position.to_logical::<f32>(scale);
            let screen = loop_.manager.screen();

            if let Some(window) = loop_.manager.get_mut(id) {
                window.position =
                    screen.clamp_position(Point::new(position.x, position.y), window.size);

                loop_.events.push((
                    id,
                    core::Event::Window(window::Event::Moved(window.position)),
                ));
            }

            loop_.needs_rebuild = true;
            loop_.request_redraw();

            if let Some(release) = release {
                pointer_event(loop_, release);
            }
        }
        Routed::Resized {
            id,
            bounds,
            release,
        } => {
            let position = bounds.position.to_logical::<f32>(scale);
            let size = bounds.size.to_logical::<f32>(scale);

            if let Some(window) = loop_.manager.get_mut(id) {
                window.position = Point::new(position.x, position.y);
            }

            resize_window(loop_, id, Size::new(size.width, size.height));

            if let Some(release) = release {
                pointer_event(loop_, release);
            }
        }
        Routed::Consumed => {}
    }
}

/// Focuses a window and brings it to the front, like a pointer press on
/// its content does.
///
/// The root window keeps its place below the windows laid out on top of
/// it.
fn focus_and_raise<P>(loop_: &mut Loop<P>, id: Id)
where
    P: Program,
    P::Theme: theme::Base,
{
    loop_.router.focus(id);

    if Some(id) != loop_.manager.root_id() {
        loop_.manager.raise(id);
    }
}

/// Starts an interactive resize session when the given event is a pointer
/// press on the resize border of a resizable logical window, like the
/// resize border of a decorated window on the desktop platforms.
///
/// Returns `true` when the session started, in which case the event is
/// consumed instead of being delivered to the user interface.
fn start_edge_resize<P>(loop_: &mut Loop<P>, event: &winit::event::WindowEvent) -> bool
where
    P: Program,
    P::Theme: theme::Base,
{
    use winit::event::ElementState;

    if !matches!(
        event,
        winit::event::WindowEvent::PointerButton {
            state: ElementState::Pressed,
            ..
        }
    ) {
        return false;
    }

    let scale = loop_
        .native
        .as_ref()
        .map(|native| native.scale_factor())
        .unwrap_or(1.0);

    let Some(cursor) = loop_.router.cursor() else {
        return false;
    };

    let position = cursor.to_logical::<f32>(scale);

    let Some((id, direction)) = loop_
        .manager
        .resize_hit_test(Point::new(position.x, position.y))
    else {
        return false;
    };

    focus_and_raise(loop_, id);

    if !start_resize_session(loop_, id, direction) {
        return false;
    }

    loop_.needs_rebuild = true;
    loop_.request_redraw();

    true
}

/// Starts an interactive resize session for the given window, capturing
/// its current bounds and size constraints.
pub(crate) fn start_resize_session<P>(
    loop_: &mut Loop<P>,
    id: Id,
    direction: window::Direction,
) -> bool
where
    P: Program,
    P::Theme: theme::Base,
{
    let scale = loop_
        .native
        .as_ref()
        .map(|native| native.scale_factor())
        .unwrap_or(1.0);

    let Some(window) = loop_.manager.get(id) else {
        return false;
    };

    let to_physical = |size: Size| {
        winit::dpi::PhysicalSize::new(
            f64::from(size.width) * scale,
            f64::from(size.height) * scale,
        )
    };

    let min_size = window.min_size.map(to_physical);
    let max_size = window.max_size.map(to_physical);
    let bounds = physical_bounds_of(loop_, id);

    loop_
        .router
        .start_drag_resize(id, direction, bounds, min_size, max_size)
}

/// Resizes a logical window to the given logical size, honoring its size
/// constraints.
pub(crate) fn resize_window<P>(loop_: &mut Loop<P>, id: Id, size: Size)
where
    P: Program,
    P::Theme: theme::Base,
{
    let size = loop_.manager.constrain(id, size);

    let Some(window) = loop_.manager.get_mut(id) else {
        return;
    };

    window.size = size;

    loop_
        .events
        .push((id, core::Event::Window(window::Event::Resized(size))));

    loop_.needs_rebuild = true;
    loop_.request_redraw();
}

/// Moves a logical window, keeping it on screen.
pub(crate) fn move_window<P>(loop_: &mut Loop<P>, id: Id, position: Point)
where
    P: Program,
    P::Theme: theme::Base,
{
    let size = loop_.manager.get(id).map(|window| window.size);
    let Some(size) = size else {
        return;
    };

    let screen = loop_.manager.screen();
    let position = screen.clamp_position(position, size);

    let Some(window) = loop_.manager.get_mut(id) else {
        return;
    };

    window.position = position;

    loop_
        .events
        .push((id, core::Event::Window(window::Event::Moved(position))));

    loop_.needs_rebuild = true;
    loop_.request_redraw();
}

/// The physical bounds of a logical window.
pub(crate) fn physical_bounds_of<P>(
    loop_: &Loop<P>,
    id: Id,
) -> PhysicalBounds
where
    P: Program,
    P::Theme: theme::Base,
{
    use winit::dpi::{PhysicalPosition, PhysicalSize};

    let scale = loop_
        .native
        .as_ref()
        .map(|native| native.scale_factor())
        .unwrap_or(1.0);

    loop_.manager.get(id).map_or_else(
        || PhysicalBounds {
            position: PhysicalPosition::new(0.0, 0.0),
            size: PhysicalSize::new(0.0, 0.0),
        },
        |window| PhysicalBounds {
            position: PhysicalPosition::new(
                f64::from(window.position.x) * scale,
                f64::from(window.position.y) * scale,
            ),
            size: PhysicalSize::new(
                f64::from(window.size.width) * scale,
                f64::from(window.size.height) * scale,
            ),
        },
    )
}

/// Maximizes or restores a logical window.
pub(crate) fn maximize_window<P>(loop_: &mut Loop<P>, id: Id, maximized: bool)
where
    P: Program,
    P::Theme: theme::Base,
{
    let screen = loop_.manager.screen().logical_size;

    let Some(window) = loop_.manager.get_mut(id) else {
        return;
    };

    let size = if maximized {
        if window.maximized.is_none() {
            window.maximized = Some(window.bounds());
        }

        window.position = Point::ORIGIN;

        screen
    } else {
        let Some(bounds) = window.maximized.take() else {
            return;
        };

        window.position = bounds.position();

        bounds.size()
    };

    resize_window(loop_, id, size);
}

/// Opens a new logical window.
pub(crate) fn open_window<'a, P>(
    loop_: &mut Loop<P>,
    program: &'a program::Instance<P>,
    interface: &mut ManuallyDrop<Option<Interface<'a, P>>>,
    id: Id,
    settings: window::Settings,
    on_open: oneshot::Sender<Id>,
) where
    P: Program,
    P::Theme: theme::Base,
{
    if loop_.manager.contains(id) {
        log::warn!("Window {id} already exists");

        let _ = on_open.send(id);

        return;
    }

    loop_.ensure_graphics();

    if loop_.native.is_none() {
        log::warn!("Cannot open a window before the surfaces are created");

        return;
    }

    let is_root = loop_.manager.is_empty();
    let (position, size) = initial_geometry(&settings, loop_.manager.screen(), is_root);

    loop_.manager.insert(
        id,
        LogicalWindow {
            position,
            size,
            resizable: settings.resizable,
            min_size: settings.min_size,
            max_size: settings.max_size,
            level: settings.level,
            visible: settings.visible,
            passthrough: false,
            maximized: None,
        },
    );

    loop_.router.focus(id);

    if loop_.waker.is_none() {
        let proxy = loop_.proxy.clone();

        loop_.waker = Some(shell::Waker::new(move || {
            proxy.send_action(Action::Event {
                window: id,
                event: core::Event::Waken,
            });
        }));
    }

    let scale_factor = loop_.manager.screen().scale_factor;

    loop_.events.push((
        id,
        core::Event::Window(window::Event::Opened {
            position: Some(position),
            size,
            scale_factor,
        }),
    ));

    if interface.is_none() {
        **interface = build_interface(loop_, program, user_interface::Cache::default());
    } else {
        loop_.needs_rebuild = true;
    }

    let _ = on_open.send(id);
    loop_.is_window_opening = false;

    loop_.request_redraw();
}

/// Closes a logical window, exiting the application when the last one is
/// gone.
pub(crate) fn close_window<'a, P>(
    loop_: &mut Loop<P>,
    interface: &mut ManuallyDrop<Option<Interface<'a, P>>>,
    id: Id,
) where
    P: Program,
    P::Theme: theme::Base,
{
    loop_.router.forget(id);

    if loop_.manager.remove(id).is_some() {
        loop_
            .events
            .push((id, core::Event::Window(window::Event::Closed)));
    }

    if loop_.manager.is_empty() {
        let dropped = std::mem::replace(&mut *interface, ManuallyDrop::new(None));
        let _ = ManuallyDrop::into_inner(dropped);

        loop_.surface = None;
        loop_.renderer = None;
        loop_.compositor = None;

        if !loop_.is_window_opening {
            let _ = loop_.control_sender.start_send(Control::Exit);
        }
    } else {
        loop_.needs_rebuild = true;
        loop_.request_redraw();
    }
}

/// Updates the screen information from the size of the native window.
fn update_screen<P>(loop_: &mut Loop<P>, physical: winit::dpi::PhysicalSize<u32>)
where
    P: Program,
    P::Theme: theme::Base,
{
    let Some(native) = loop_.native.as_ref() else {
        return;
    };

    let scale_factor = native.scale_factor();
    let logical = physical.to_logical::<f32>(scale_factor);

    loop_.manager.set_screen(
        Size::new(logical.width, logical.height),
        scale_factor as f32,
    );

    loop_.needs_rebuild = true;
    loop_.request_redraw();
}

/// Sets the control flow from the redraw schedule.
fn set_control_flow<P>(loop_: &mut Loop<P>)
where
    P: Program,
    P::Theme: theme::Base,
{
    use winit::event_loop::ControlFlow;

    let flow = match loop_.redraw_at {
        Some(redraw_at) => ControlFlow::WaitUntil(redraw_at),
        None => ControlFlow::Wait,
    };

    let _ = loop_.control_sender.start_send(Control::ChangeFlow(flow));
}

/// Updates the input method state of the application.
///
/// This is a best-effort emulation: `set_ime_allowed` toggles the soft
/// keyboard, while cursor area updates are not supported by the Android
/// backend of `winit` and are ignored.
fn request_input_method<P>(
    loop_: &mut Loop<P>,
    program: &program::Instance<P>,
    input_method: crate::core::InputMethod,
) where
    P: Program,
    P::Theme: theme::Base,
{
    use crate::core::InputMethod;

    let Some(native) = loop_.native.clone() else {
        return;
    };

    match input_method {
        InputMethod::Disabled => {
            #[allow(deprecated, reason = "IME API is deprecated in winit")]
            if loop_.ime_enabled {
                native.set_ime_allowed(false);
                loop_.ime_enabled = false;
            }

            loop_.preedit = None;
        }
        InputMethod::Enabled {
            cursor,
            purpose: _,
            preedit,
        } => {
            #[allow(deprecated, reason = "IME API is deprecated in winit")]
            if !loop_.ime_enabled {
                native.set_ime_allowed(true);
                loop_.ime_enabled = true;
            }

            if let Some(preedit) = preedit {
                if preedit.content.is_empty() {
                    loop_.preedit = None;
                } else {
                    let Some(renderer) = loop_.renderer.as_ref() else {
                        return;
                    };

                    let theme = loop_
                        .manager
                        .root_id()
                        .and_then(|id| program.theme(id))
                        .unwrap_or_else(|| <P::Theme as theme::Base>::default(loop_.system_theme));
                    let style = program.style(&theme);

                    let mut overlay = loop_.preedit.take().unwrap_or_else(Preedit::new);

                    overlay.update(cursor, &preedit, style.background_color, renderer);

                    loop_.preedit = Some(overlay);
                }
            } else {
                loop_.preedit = None;
            }
        }
    }
}

impl<P> Loop<P>
where
    P: Program,
    P::Theme: theme::Base,
{
    pub(crate) fn request_redraw(&self) {
        if let Some(native) = &self.native {
            native.request_redraw();
        }
    }

    pub(crate) fn recreate_surface(&mut self) {
        let Some(native) = self.native.clone() else {
            return;
        };

        let Some(compositor) = self.compositor.as_mut() else {
            return;
        };

        let size = native.surface_size();

        self.surface = Some(compositor.create_surface(native.clone(), size.width, size.height));
        self.surface_size = Size::new(size.width, size.height);
    }

    /// Creates the compositor, renderer, and surface if they are missing.
    pub(crate) fn ensure_graphics(&mut self) {
        if self.compositor.is_none() {
            let Some(native) = self.native.clone() else {
                return;
            };

            let (compositor_sender, mut compositor_receiver) = oneshot::channel();

            let create_compositor = {
                let window = native.clone();
                let backend_settings = self.backend_settings.clone();
                let display_handle = self.display_handle.clone();
                let proxy = self.proxy.clone();
                let default_fonts = self.default_fonts.clone();

                async move {
                    let shell = Shell::new(proxy.clone());

                    let mut new_compositor =
                        <<P::Renderer as compositor::Default>::Compositor as crate::graphics::Compositor>::new(
                            backend_settings,
                            display_handle,
                            window,
                            shell,
                        )
                        .await;

                    if let Ok(compositor) = &mut new_compositor {
                        for font in default_fonts {
                            compositor.load_font(font.clone());
                        }
                    }

                    compositor_sender
                        .send(new_compositor)
                        .ok()
                        .expect("Send compositor");

                    // HACK! Send a proxy event on completion to trigger
                    // a runtime re-poll
                    {
                        let (sender, _receiver) = oneshot::channel();

                        proxy.send_action(Action::Window(runtime::window::Action::GetLatest(
                            sender,
                        )));
                    }
                }
            };

            self.runtime.block_on(create_compositor);

            match compositor_receiver.try_recv() {
                Ok(Some(Ok(new_compositor))) => {
                    let renderer = new_compositor.create_renderer(self.renderer_settings.clone());

                    self.renderer = Some(renderer);
                    self.compositor = Some(new_compositor);
                }
                Ok(Some(Err(error))) => {
                    let _ = self.control_sender.start_send(Control::Crash(error.into()));

                    return;
                }
                // Unreachable: the future that sends the compositor was
                // driven to completion by `block_on` above.
                _ => {
                    log::error!("Compositor creation did not complete");

                    return;
                }
            }
        }

        if self.surface.is_none() {
            self.recreate_surface();
        }
    }
}
