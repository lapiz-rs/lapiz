#[cfg(target_os = "android")]
#[path = "main.rs"]
mod app;

#[cfg(target_os = "android")]
use winit::platform::android::activity::AndroidApp;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    app::run(android_app);
}
