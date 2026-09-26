use lapiz_utils::wrapper;
use winit::platform::android::activity;

use crate::{Services, service::Service};

wrapper! {
    #[derive(Debug, Clone)]
    pub AndroidApp : activity::AndroidApp
}

impl Service for AndroidApp {}

pub trait AndroidAppExt {
    fn android_app(&self) -> &AndroidApp;
}

impl AndroidAppExt for Services {
    fn android_app(&self) -> &AndroidApp {
        self.service::<AndroidApp>()
    }
}
