use windows::Win32::{
    Foundation::HWND,
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFOEXW, MonitorFromWindow},
    UI::WindowsAndMessaging::{
        GWL_STYLE, GWLP_HWNDPARENT, GetWindowLongPtrW, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, WS_MAXIMIZEBOX,
    },
};

pub fn get_window_monitor_name(raw_window_id: u64) -> String {
    let window = HWND(raw_window_id as *mut std::ffi::c_void);
    let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return String::new();
    }

    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let succeeded = unsafe { GetMonitorInfoW(monitor, &mut info as *mut _ as *mut _) };
    if !succeeded.as_bool() {
        return String::new();
    }

    let length = info
        .szDevice
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(info.szDevice.len());
    String::from_utf16_lossy(&info.szDevice[..length])
}

pub fn set_window_parent(parent: u64, child: u64) {
    unsafe {
        SetWindowLongPtrW(
            HWND(child as *mut std::ffi::c_void),
            GWLP_HWNDPARENT,
            parent as isize,
        );
    }
}

pub fn disable_window_snap(raw_window_id: u64) {
    let window = HWND(raw_window_id as *mut std::ffi::c_void);
    unsafe {
        let style = GetWindowLongPtrW(window, GWL_STYLE);
        SetWindowLongPtrW(window, GWL_STYLE, style & !(WS_MAXIMIZEBOX.0 as isize));
        let _ = SetWindowPos(
            window,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}
