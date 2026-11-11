use lapiz_utils::wrapper;
use winit::platform::android::activity;

use crate::global::{Global, Globals};

wrapper! {
    /// Global wrapper around the Winit [`activity::AndroidApp`] handle on Android targets.
    #[derive(Debug, Clone)]
    pub AndroidApp : activity::AndroidApp
}

impl Global for AndroidApp {}

/// Extension trait providing convenient access to the [`AndroidApp`] handle stored in [`Globals`].
pub trait AndroidAppExt {
    fn android_app(&self) -> &AndroidApp;
}

impl AndroidAppExt for Globals {
    fn android_app(&self) -> &AndroidApp {
        self.global::<AndroidApp>()
    }
}
