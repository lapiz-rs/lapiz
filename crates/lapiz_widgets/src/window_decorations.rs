use iced_core::{
    Element, Layout, Length, Rectangle, Size, Theme, Widget, layout, mouse, renderer, widget::Tree,
};
use iced_wgpu::Renderer;

#[derive(Clone, Debug, Default)]
pub struct WindowDecorations {
    #[cfg(target_os = "windows")]
    inner: imp::Inner,
}

impl WindowDecorations {
    pub fn attach(&self, raw_window_id: u64) -> bool {
        imp::attach(self, raw_window_id)
    }

    pub fn caption_region<Message>(&self) -> Element<'_, Message, Theme, Renderer> {
        Element::new(CaptionRegion {
            window_decorations: self.clone(),
        })
    }
}

struct CaptionRegion {
    window_decorations: WindowDecorations,
}

impl<Message> Widget<Message, Theme, Renderer> for CaptionRegion {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, Length::Fill, Length::Fill)
    }

    fn draw(
        &self,
        _tree: &Tree,
        _renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        imp::set_caption_region(&self.window_decorations, layout.bounds());
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use iced_core::Rectangle;

    use super::WindowDecorations;

    pub fn attach(_window_decorations: &WindowDecorations, _raw_window_id: u64) -> bool {
        true
    }

    pub fn set_caption_region(_window_decorations: &WindowDecorations, _bounds: Rectangle) {}
}

#[cfg(target_os = "windows")]
mod imp {
    use std::{cell::Cell, ffi::c_void, rc::Rc};

    use iced_core::{Point, Rectangle};
    use windows::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::ScreenToClient,
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                GWL_STYLE, GetWindowLongPtrW, GetWindowRect, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT,
                HTCAPTION, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed,
                SM_CXPADDEDBORDER, SM_CXSIZEFRAME, SM_CYSIZEFRAME, WM_NCDESTROY, WM_NCHITTEST,
                WS_THICKFRAME,
            },
        },
    };

    use super::WindowDecorations;

    pub(super) type Inner = Rc<HitTestState>;

    #[derive(Debug, Default)]
    pub(super) struct HitTestState {
        caption_region: Cell<Option<Rectangle>>,
    }

    pub fn attach(window_decorations: &WindowDecorations, raw_window_id: u64) -> bool {
        let window = HWND(raw_window_id as *mut c_void);
        let state = std::rc::Rc::into_raw(window_decorations.inner.clone()) as usize;
        let attached = unsafe { SetWindowSubclass(window, Some(window_proc), 1, state).as_bool() };

        if !attached {
            unsafe {
                drop(Rc::from_raw(state as *const HitTestState));
            }
        }

        attached
    }

    pub fn set_caption_region(window_decorations: &WindowDecorations, bounds: Rectangle) {
        window_decorations.inner.caption_region.set(Some(bounds));
    }

    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        subclass_id: usize,
        ref_data: usize,
    ) -> LRESULT {
        if message == WM_NCHITTEST {
            let x = lparam.0 as u16 as i16 as i32;
            let y = (lparam.0 >> 16) as u16 as i16 as i32;

            if let Some(hit) = unsafe { resize_hit_test(window, x, y) } {
                return LRESULT(hit as isize);
            }

            let state = unsafe { &*(ref_data as *const HitTestState) };
            if unsafe { is_caption(window, x, y, state) } {
                return LRESULT(HTCAPTION as isize);
            }
        }

        if message == WM_NCDESTROY {
            unsafe {
                let _ = RemoveWindowSubclass(window, Some(window_proc), subclass_id);
                drop(Rc::from_raw(ref_data as *const HitTestState));
            }
        }

        unsafe { DefSubclassProc(window, message, wparam, lparam) }
    }

    unsafe fn resize_hit_test(window: HWND, x: i32, y: i32) -> Option<u32> {
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

    unsafe fn is_caption(window: HWND, x: i32, y: i32, state: &HitTestState) -> bool {
        let mut point = POINT { x, y };
        if !unsafe { ScreenToClient(window, &mut point) }.as_bool() {
            return false;
        }

        let scale = unsafe { GetDpiForWindow(window) } as f32 / 96.0;
        let point = Point::new(point.x as f32 / scale, point.y as f32 / scale);
        state
            .caption_region
            .get()
            .is_some_and(|bounds| bounds.contains(point))
    }
}
