#[cfg(target_os = "android")]
#[path = "android.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

pub fn get_window_monitor_name(window: u64) -> String {
    imp::get_window_monitor_name(window)
}

pub fn set_window_parent(parent: u64, child: u64) {
    imp::set_window_parent(parent, child);
}

pub fn disable_window_snap(window: u64) {
    imp::disable_window_snap(window);
}

pub fn attach_resize_handle(window: u64) -> bool {
    imp::attach_resize_handle(window)
}
