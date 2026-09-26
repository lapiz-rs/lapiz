use crate::android::instance::{
    Interface, Loop, close_window, maximize_window, move_window, open_window, physical_bounds_of,
    resize_window, start_resize_session,
};
use crate::core::Renderer as _;
use crate::core::Size;
use crate::core::theme;
use crate::core::window::Mode;
use crate::futures::subscription;
use crate::graphics::{Compositor as _, Shell, compositor};
use crate::program::{self, Program};

use crate::runtime::Action;
use crate::runtime::backend;
use crate::runtime::clipboard;
use crate::runtime::font;
use crate::runtime::image;
use crate::runtime::system;
use crate::runtime::window;
use std::mem::ManuallyDrop;

/// Runs an [`Action`] produced by the [`Program`] against the Android
/// runtime.
pub(crate) fn run<'a, P>(
    loop_: &mut Loop<P>,
    program: &'a program::Instance<P>,
    interface: &mut ManuallyDrop<Option<Interface<'a, P>>>,
    action: Action<P::Message>,
) where
    P: Program,
    P::Theme: theme::Base,
{
    match action {
        Action::Output(message) => {
            let _ = loop_.messages.push(message);
        }
        Action::Clipboard(action) => match action {
            clipboard::Action::Read { kind, channel } => {
                loop_.clipboard.read(kind, move |result| {
                    let _ = channel.send(result);
                });
            }
            clipboard::Action::Write { content, channel } => {
                loop_.clipboard.write(content, move |result| {
                    let _ = channel.send(result);
                });
            }
        },
        Action::Window(action) => {
            run_window_action(loop_, program, interface, action);
        }
        Action::System(action) => match action {
            system::Action::GetInformation(_channel) => {
                #[cfg(feature = "sysinfo")]
                if let Some(compositor) = loop_.compositor.as_ref() {
                    let graphics_info = compositor.information();

                    std::thread::spawn(move || {
                        let information = crate::system_information(graphics_info);

                        let _ = _channel.send(information);
                    });
                }
            }
            system::Action::GetTheme(channel) => {
                let _ = channel.send(loop_.system_theme);
            }
            system::Action::NotifyTheme(mode) => {
                if mode != loop_.system_theme {
                    loop_.system_theme = mode;

                    loop_
                        .runtime
                        .broadcast(subscription::Event::SystemThemeChanged(mode));
                }
            }
        },
        Action::Font(action) => match action {
            font::Action::Load { bytes, channel } => {
                if let Some(compositor) = loop_.compositor.as_mut() {
                    let result = compositor.load_font(bytes.clone());

                    let _ = channel.send(result);
                }
            }
            font::Action::List { channel } => {
                if let Some(compositor) = loop_.compositor.as_mut() {
                    let fonts = compositor.list_fonts();

                    let _ = channel.send(fonts);
                }
            }
            font::Action::SetDefaults { font, text_size } => {
                loop_.renderer_settings.default_font = font;
                loop_.renderer_settings.default_text_size = text_size;

                let Some(compositor) = loop_.compositor.as_mut() else {
                    return;
                };

                // Recreate the renderer; the interface is rebuilt with the
                // new defaults on the next `AboutToWait`.
                loop_.renderer = Some(compositor.create_renderer(loop_.renderer_settings.clone()));

                loop_.needs_rebuild = true;
                loop_.request_redraw();
            }
        },
        Action::Widget(operation) => {
            use crate::core::widget::operation;

            let mut current_operation = Some(operation);

            while let Some(mut operation) = current_operation.take() {
                if let Some(interface) = interface.as_mut() {
                    if let Some(renderer) = loop_.renderer.as_mut() {
                        interface.operate(renderer, operation.as_mut());
                    }
                }

                match operation.finish() {
                    operation::Outcome::None => {}
                    operation::Outcome::Some(()) => {}
                    operation::Outcome::Chain(next) => {
                        current_operation = Some(next);
                    }
                }
            }

            loop_.request_redraw();
        }
        Action::Image(action) => match action {
            image::Action::Allocate(handle, sender) => {
                if let Some(renderer) = loop_.renderer.as_mut() {
                    renderer.allocate_image(&handle, move |allocation| {
                        let _ = sender.send(allocation);
                    });
                }
            }
        },
        Action::Backend(action) => match action {
            backend::Action::Configure(settings, sender) => {
                let shell = Shell::new(loop_.proxy.clone());

                let Some(native) = loop_.native.clone() else {
                    return;
                };

                let mut new_compositor = match loop_.runtime.block_on(
                    <<P::Renderer as compositor::Default>::Compositor as crate::graphics::Compositor>::new(
                        settings,
                        loop_.display_handle.clone(),
                        native.clone(),
                        shell,
                    ),
                ) {
                    Ok(compositor) => compositor,
                    Err(error) => {
                        let _ = sender.send(Err(error));

                        return;
                    }
                };

                crate::graphics::cache::invalidate_all();

                let renderer = new_compositor.create_renderer(loop_.renderer_settings.clone());

                let size = native.surface_size();
                let surface =
                    new_compositor.create_surface(native.clone(), size.width, size.height);

                loop_.renderer = Some(renderer);
                loop_.surface = Some(surface);
                loop_.surface_size = Size::new(size.width, size.height);
                loop_.compositor = Some(new_compositor);

                let _ = sender.send(Ok(()));

                loop_.needs_rebuild = true;
                loop_.request_redraw();
            }
        },
        Action::Event { window, event } => {
            loop_.events.push((window, event));
        }
        Action::Tick => {
            if let Some(renderer) = loop_.renderer.as_mut() {
                renderer.tick();
            }
        }
        Action::Reload => {
            loop_.needs_rebuild = true;
            loop_.request_redraw();
        }
        Action::Exit => {
            let _ = loop_
                .control_sender
                .start_send(crate::android::runner::Control::Exit);
        }
    }
}

/// Runs a [`window::Action`] against the logical windows.
fn run_window_action<'a, P>(
    loop_: &mut Loop<P>,
    program: &'a program::Instance<P>,
    interface: &mut ManuallyDrop<Option<Interface<'a, P>>>,
    action: window::Action,
) where
    P: Program,
    P::Theme: theme::Base,
{
    let _ = program;

    match action {
        window::Action::Open(id, settings, channel) => {
            open_window(loop_, program, interface, id, settings, channel);
        }
        window::Action::Close(id) => {
            close_window(loop_, interface, id);
        }
        window::Action::GetOldest(channel) => {
            let id = loop_.manager.first_id();

            let _ = channel.send(id);
        }
        window::Action::GetLatest(channel) => {
            let id = loop_
                .manager
                .z_order()
                .last()
                .copied()
                .or_else(|| loop_.manager.first_id());

            let _ = channel.send(id);
        }
        window::Action::Drag(id) => {
            let bounds = physical_bounds_of(loop_, id);

            if !loop_.router.start_drag(id, bounds) {
                log::warn!("window::drag ignored: no known pointer position");
            }
        }
        window::Action::DragResize(id, direction) => {
            if !start_resize_session(loop_, id, direction) {
                log::warn!("window::drag_resize ignored: no known pointer position");
            }
        }
        window::Action::Resize(id, size) => {
            resize_window(loop_, id, size);
        }
        window::Action::SetMinSize(id, size) => {
            if let Some(window) = loop_.manager.get_mut(id) {
                window.min_size = size;

                let size = window.size;

                resize_window(loop_, id, size);
            }
        }
        window::Action::SetMaxSize(id, size) => {
            if let Some(window) = loop_.manager.get_mut(id) {
                window.max_size = size;

                let size = window.size;

                resize_window(loop_, id, size);
            }
        }
        window::Action::SetResizeIncrements(_id, _increments) => {
            log::warn!("window::set_resize_increments is not supported on Android");
        }
        window::Action::SetResizable(id, resizable) => {
            if let Some(window) = loop_.manager.get_mut(id) {
                window.resizable = resizable;
            }
        }
        window::Action::GetSize(id, channel) => {
            if let Some(window) = loop_.manager.get(id) {
                let _ = channel.send(Size::new(window.size.width, window.size.height));
            }
        }
        window::Action::GetMaximized(id, channel) => {
            if let Some(window) = loop_.manager.get(id) {
                let _ = channel.send(window.maximized.is_some());
            }
        }
        window::Action::Maximize(id, maximized) => {
            maximize_window(loop_, id, maximized);
        }
        window::Action::GetMinimized(_id, channel) => {
            log::warn!("window::minimize is not supported on Android");

            let _ = channel.send(Some(false));
        }
        window::Action::Minimize(_id, _minimized) => {
            log::warn!("window::minimize is not supported on Android");
        }
        window::Action::GetPosition(id, channel) => {
            if let Some(window) = loop_.manager.get(id) {
                let _ = channel.send(Some(window.position));
            }
        }
        window::Action::GetScaleFactor(_id, channel) => {
            let _ = channel.send(loop_.manager.screen().scale_factor);
        }
        window::Action::Move(id, position) => {
            move_window(loop_, id, position);
        }
        window::Action::SetMode(id, mode) => {
            if let Some(window) = loop_.manager.get_mut(id) {
                match mode {
                    Mode::Hidden => {
                        window.visible = false;
                    }
                    Mode::Windowed | Mode::Fullscreen => {
                        window.visible = true;
                    }
                }

                loop_.needs_rebuild = true;
                loop_.request_redraw();
            }
        }
        window::Action::SetIcon(_id, _icon) => {
            log::warn!("window::set_icon is not supported on Android");
        }
        window::Action::GetMode(id, channel) => {
            if let Some(window) = loop_.manager.get(id) {
                let mode = if window.visible {
                    Mode::Windowed
                } else {
                    Mode::Hidden
                };

                let _ = channel.send(mode);
            }
        }
        window::Action::ToggleMaximize(id) => {
            let maximized = loop_
                .manager
                .get(id)
                .is_some_and(|window| window.maximized.is_some());

            maximize_window(loop_, id, !maximized);
        }
        window::Action::ToggleDecorations(_id) => {
            log::warn!("window::toggle_decorations is not supported on Android");
        }
        window::Action::RequestUserAttention(_id, _attention) => {
            log::warn!("window::request_user_attention is not supported on Android");
        }
        window::Action::GainFocus(id) => {
            loop_.router.focus(id);
            loop_.manager.raise(id);

            loop_.needs_rebuild = true;
            loop_.request_redraw();
        }
        window::Action::SetLevel(id, level) => {
            loop_.manager.set_level(id, level);

            loop_.needs_rebuild = true;
            loop_.request_redraw();
        }
        window::Action::ShowSystemMenu(_id) => {
            log::warn!("window::show_system_menu is not supported on Android");
        }
        window::Action::GetRawId(_id, channel) => {
            if let Some(native) = &loop_.native {
                let _ = channel.send(native.id().into_raw() as u64);
            }
        }
        window::Action::Run(_id, f) => {
            if let Some(native) = &loop_.native {
                f(native);
            }
        }
        window::Action::Screenshot(_id, _channel) => {
            log::warn!("window::screenshot is not supported on Android");
        }
        window::Action::EnableMousePassthrough(id) => {
            if let Some(window) = loop_.manager.get_mut(id) {
                window.passthrough = true;
            }
        }
        window::Action::DisableMousePassthrough(id) => {
            if let Some(window) = loop_.manager.get_mut(id) {
                window.passthrough = false;
            }
        }
        window::Action::GetMonitorSize(_id, channel) => {
            let size = loop_.manager.screen().logical_size;

            let _ = channel.send(Some(Size::new(size.width, size.height)));
        }
        window::Action::SetAllowAutomaticTabbing(_enabled) => {
            log::warn!("window::allow_automatic_tabbing is not supported on Android");
        }
        window::Action::RedrawAll => {
            loop_.request_redraw();
        }
        window::Action::RelayoutAll => {
            loop_.needs_rebuild = true;
            loop_.request_redraw();
        }
    }
}
