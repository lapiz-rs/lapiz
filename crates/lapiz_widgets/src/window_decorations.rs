pub fn enable_native_resize(raw_window_id: u64) -> bool {
    imp::enable_native_resize(raw_window_id)
}

#[cfg(not(target_os = "windows"))]
mod imp {
    pub fn enable_native_resize(_raw_window_id: u64) -> bool {
        true
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;

    use windows::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                GWL_STYLE, GetWindowLongPtrW, GetWindowRect, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT,
                HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed, SM_CXPADDEDBORDER,
                SM_CXSIZEFRAME, SM_CYSIZEFRAME, WM_NCDESTROY, WM_NCHITTEST, WS_THICKFRAME,
            },
        },
    };

    pub fn enable_native_resize(raw_window_id: u64) -> bool {
        let window = HWND(raw_window_id as *mut c_void);
        unsafe { SetWindowSubclass(window, Some(window_proc), 1, 0).as_bool() }
    }

    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        subclass_id: usize,
        _ref_data: usize,
    ) -> LRESULT {
        if message == WM_NCHITTEST
            && let Some(hit) = unsafe { resize_hit_test(window, lparam) }
        {
            return LRESULT(hit as isize);
        }

        if message == WM_NCDESTROY {
            unsafe {
                let _ = RemoveWindowSubclass(window, Some(window_proc), subclass_id);
            }
        }

        unsafe { DefSubclassProc(window, message, wparam, lparam) }
    }

    unsafe fn resize_hit_test(window: HWND, lparam: LPARAM) -> Option<u32> {
        let style = unsafe { GetWindowLongPtrW(window, GWL_STYLE) };
        if style & WS_THICKFRAME.0 as isize == 0 || unsafe { IsZoomed(window).as_bool() } {
            return None;
        }

        let mut bounds = RECT::default();
        unsafe { GetWindowRect(window, &mut bounds) }.ok()?;

        let dpi = unsafe { GetDpiForWindow(window) };
        let padded_border = unsafe { GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) };
        let horizontal_border =
            unsafe { GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) } + padded_border;
        let vertical_border =
            unsafe { GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) } + padded_border;

        let x = lparam.0 as u16 as i16 as i32;
        let y = (lparam.0 >> 16) as u16 as i16 as i32;
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
}
