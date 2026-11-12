use lapiz_runtime::{Runtime, plugin::Plugin};

use crate::key::KeyboardState;

pub mod key;
pub mod mouse;

/// A [`Plugin`] that registers [`KeyboardState`] into the application's global state.
pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut Runtime) {
        app.add_global::<KeyboardState>();
    }

    fn finish(&self, _app: &mut Runtime) {}
}
