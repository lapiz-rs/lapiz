#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "Win32 FFI in this function is sequenced and checked locally"
)]

use std::ffi::c_void;

use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFOEXW, MonitorFromWindow},
    UI::{
        HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
        Shell::{DefSubclassProc, SetWindowSubclass},
        WindowsAndMessaging::{
            GWL_STYLE, GWLP_HWNDPARENT, GetWindowLongPtrW, GetWindowRect, HTBOTTOM, HTBOTTOMLEFT,
            HTBOTTOMRIGHT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed,
            SM_CXPADDEDBORDER, SM_CXSIZEFRAME, SM_CYSIZEFRAME, SWP_FRAMECHANGED, SWP_NOACTIVATE,
            SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, WM_NCHITTEST,
            WS_MAXIMIZEBOX, WS_THICKFRAME,
        },
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

pub fn attach_resize_handle(raw_window_id: u64) -> bool {
    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _subclass_id: usize,
        _ref_data: usize,
    ) -> LRESULT {
        if message == WM_NCHITTEST {
            let x = lparam.0 as u16 as i16 as i32;
            let y = (lparam.0 >> 16) as u16 as i16 as i32;

            if let Some(hit) = resize_hit_test(window, x, y) {
                return LRESULT(hit as isize);
            }
        }

        unsafe { DefSubclassProc(window, message, wparam, lparam) }
    }

    fn resize_hit_test(window: HWND, x: i32, y: i32) -> Option<u32> {
        let style = unsafe { GetWindowLongPtrW(window, GWL_STYLE) };
        let zoomed = unsafe { IsZoomed(window).as_bool() };
        if style & WS_THICKFRAME.0 as isize == 0 || zoomed {
            return None;
        }

        let mut bounds = RECT::default();
        unsafe { GetWindowRect(window, &mut bounds) }.ok()?;

        let dpi = unsafe { GetDpiForWindow(window) };
        let padded_border = unsafe { GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) };
        let frame_x = unsafe { GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) };
        let frame_y = unsafe { GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) };
        let horizontal_border = frame_x + padded_border;
        let vertical_border = frame_y + padded_border;

        let left = x < bounds.left + horizontal_border;
        let right = x >= bounds.right - horizontal_border;
        let top = y < bounds.top + vertical_border;
        let bottom = y >= bounds.bottom - vertical_border;

        match (left, right, top, bottom) {
            (true, _, true, _) => Some(HTTOPLEFT),
            (_, true, true, _) => Some(HTTOPRIGHT),
            (true, _, _, true) => Some(HTBOTTOMLEFT),
            (_, true, _, true) => Some(HTBOTTOMRIGHT),
            (true, ..) => Some(HTLEFT),
            (_, true, ..) => Some(HTRIGHT),
            (_, _, true, _) => Some(HTTOP),
            (_, _, _, true) => Some(HTBOTTOM),
            _ => None,
        }
    }

    let window = HWND(raw_window_id as *mut c_void);
    unsafe { SetWindowSubclass(window, Some(window_proc), 1, 0).as_bool() }
}
