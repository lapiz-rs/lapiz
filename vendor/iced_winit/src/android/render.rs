use crate::android::instance::Loop;
use crate::core::Color;
use crate::core::Size;
use crate::core::renderer::Scale;
use crate::core::theme;
use crate::graphics::{Compositor as _, Viewport, compositor};
use crate::program::Program;

/// Presents the recorded draw commands of the user interface as a single
/// frame of the native surface.
pub(crate) fn present<P>(loop_: &mut Loop<P>, native: &std::sync::Arc<dyn winit::window::Window>)
where
    P: Program,
    P::Theme: theme::Base,
{
    let physical_size = native.surface_size();

    let Some(surface) = loop_.surface.as_mut() else {
        return;
    };

    let Some(compositor) = loop_.compositor.as_mut() else {
        return;
    };

    let Some(renderer) = loop_.renderer.as_mut() else {
        return;
    };

    // The root window fills the whole surface and paints its own
    // background as part of the user interface; the clear color only
    // covers the gaps, if any.
    let viewport = Viewport::with_physical_size(
        Size::new(physical_size.width, physical_size.height),
        Scale {
            window: native.scale_factor() as f32,
            application: 1.0,
        },
    );
    let background = Color::BLACK;

    let span = loop_.manager.root_id().map(crate::debug::present);

    let result = compositor.present(renderer, surface, &viewport, background, || {
        native.pre_present_notify()
    });

    match result {
        Ok(()) => {
            if let Some(span) = span {
                span.finish();
            }
        }
        Err(error) => match error {
            compositor::SurfaceError::OutOfMemory => {
                // This is an unrecoverable error.
                panic!("{error:?}");
            }
            compositor::SurfaceError::Outdated | compositor::SurfaceError::Lost => {
                if let Some(span) = span {
                    span.finish();
                }

                if error == compositor::SurfaceError::Lost {
                    loop_.surface = None;
                    loop_.recreate_surface();
                } else if let (Some(compositor), Some(surface)) =
                    (loop_.compositor.as_mut(), loop_.surface.as_mut())
                {
                    compositor.configure_surface(
                        surface,
                        physical_size.width,
                        physical_size.height,
                    );
                }

                loop_.request_redraw();
            }
            compositor::SurfaceError::Occluded => {
                // Do nothing and wait for the window to become visible
                // again.
            }
            _ => {
                if let Some(span) = span {
                    span.finish();
                }

                log::warn!("Error {error:?} when presenting surface.");

                loop_.request_redraw();
            }
        },
    }
}
