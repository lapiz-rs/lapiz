use crate::Runtime;

/// Modular unit of application configuration that registers globals, views, and services on [`Runtime`].
///
/// Plugins are added to an [`Application`](crate::Application) and initialized in a two-phase
/// lifecycle when [`Application::build_plugins`](crate::Application::build_plugins) is called:
///
/// 1. [`build`](Plugin::build): Called in registration order on every plugin to insert initial
///    [`Global`](crate::global::Global) registries, [`WindowView`](crate::windows::WindowView)s,
///    and extension items.
/// 2. [`finish`](Plugin::finish): Called in registration order after all plugins have completed
///    `build`, allowing plugins to finalize systems that depend on registrations from other plugins.
pub trait Plugin: 'static {
    fn build(&self, app: &mut Runtime);
    fn finish(&self, _app: &mut Runtime) {}
}
