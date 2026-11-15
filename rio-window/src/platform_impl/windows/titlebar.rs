//! Non-client handling for windows with `WindowFlags::MARKER_CUSTOM_TITLEBAR`.

use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HTCAPTION, HTCLIENT, HTCLOSE, HTLEFT, HTMAXBUTTON, HTMINBUTTON, HTRIGHT, HTTOP,
    HTTOPLEFT, HTTOPRIGHT, SC_CLOSE, SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE,
    SM_CXPADDEDBORDER, SM_CYSIZEFRAME,
};

use crate::platform::windows::TitlebarLayout;
use crate::window::CaptionButton;

/// Thickness of the top resize border, which is also how far a maximized window overhangs the
/// monitor at the top.
pub fn top_border_height(scale_factor: f64) -> i32 {
    let dpi = (scale_factor * 96.0).round() as u32;
    unsafe {
        GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi)
            + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
    }
}

/// Refines the `DefWindowProc` result of `WM_NCHITTEST`, whose caption is part of the client area.
pub fn hit_test(
    window: HWND,
    default: u32,
    screen_x: i32,
    screen_y: i32,
    maximized: bool,
    scale_factor: f64,
    layout: &TitlebarLayout,
) -> u32 {
    let mut point = POINT {
        x: screen_x,
        y: screen_y,
    };
    unsafe { ScreenToClient(window, &mut point) };
    let in_top_border = !maximized && point.y < top_border_height(scale_factor);

    match default {
        HTLEFT if in_top_border => return HTTOPLEFT,
        HTRIGHT if in_top_border => return HTTOPRIGHT,
        HTCLIENT => {}
        other => return other,
    }
    if in_top_border {
        return HTTOP;
    }
    if point.y >= layout.height {
        return HTCLIENT;
    }
    if let Some((button, _)) = layout
        .caption_buttons
        .iter()
        .find(|(_, rect)| rect.contains(point.x, point.y))
    {
        return match button {
            CaptionButton::Minimize => HTMINBUTTON,
            CaptionButton::Maximize => HTMAXBUTTON,
            CaptionButton::Close => HTCLOSE,
        };
    }
    if layout
        .client_rects
        .iter()
        .any(|rect| rect.contains(point.x, point.y))
    {
        return HTCLIENT;
    }
    HTCAPTION
}

pub fn caption_button(hit: usize) -> Option<CaptionButton> {
    match hit as u32 {
        HTMINBUTTON => Some(CaptionButton::Minimize),
        HTMAXBUTTON => Some(CaptionButton::Maximize),
        HTCLOSE => Some(CaptionButton::Close),
        _ => None,
    }
}

/// The `WM_SYSCOMMAND` a click on the caption button performs.
pub fn caption_button_command(button: CaptionButton, maximized: bool) -> u32 {
    match button {
        CaptionButton::Minimize => SC_MINIMIZE,
        CaptionButton::Maximize if maximized => SC_RESTORE,
        CaptionButton::Maximize => SC_MAXIMIZE,
        CaptionButton::Close => SC_CLOSE,
    }
}
