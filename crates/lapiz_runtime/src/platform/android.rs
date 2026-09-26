pub fn get_window_monitor_name(_window: u64) -> String {
    "Android".to_owned()
}

pub fn set_window_parent(_parent: u64, _child: u64) {}

pub fn disable_window_snap(_window: u64) {}

// This is handled by multi window emulator in our modified iced_winit
pub fn attach_resize_handle(_raw_window_id: u64) -> bool {
    true
}
