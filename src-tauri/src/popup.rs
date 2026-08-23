//! Positions and shows the popup WebviewWindow at (or near) the cursor,
//! clamped so it never renders off-screen. This is where multi-monitor
//! and DPI-scaling edge cases (item #13 in the spec) are handled, because
//! only Win32 knows the true work-area + scale factor of the monitor the
//! cursor is currently on.

use serde::Serialize;
use tauri::{AppHandle, LogicalPosition, Manager, PhysicalPosition, WebviewWindow};
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE};

const POPUP_WIDTH: i32 = 360;
const POPUP_HEIGHT: i32 = 380;
const CURSOR_OFFSET: i32 = 12; // small gap so the popup doesn't sit under the cursor

#[derive(Serialize, Clone)]
pub struct ShowPopupPayload {
    x: i32,
    y: i32,
    #[serde(rename = "monitorWidth")]
    monitor_width: i32,
    #[serde(rename = "monitorHeight")]
    monitor_height: i32,
    #[serde(rename = "scaleFactor")]
    scale_factor: f64,
    #[serde(rename = "triggeredBy")]
    triggered_by: &'static str,
    #[serde(rename = "selectedText")]
    selected_text: Option<String>,
}

fn build_show_popup_payload(
    x: i32,
    y: i32,
    triggered_by: &'static str,
    selected_text: Option<String>,
) -> ShowPopupPayload {
    let work_area = monitor_work_area(x, y);
    ShowPopupPayload {
        x,
        y,
        monitor_width: work_area.2 - work_area.0,
        monitor_height: work_area.3 - work_area.1,
        scale_factor: 1.0, // WebView2/Tauri applies its own DPI scaling to logical px
        triggered_by,
        selected_text,
    }
}

/// Single entry point both the mouse hook and the Ctrl+Space shortcut call
/// to open the popup. Captures whatever text is currently selected in the
/// foreground app (blocks ~120ms — see selection::capture_selected_text),
/// then emits "show-popup" with that text included and positions/shows
/// the window. Consolidating this in one place means both trigger paths
/// automatically pick up any future changes here (e.g. Phase 4's
/// screenshot capture) without needing to stay in sync by hand.
pub fn capture_and_show(app: &AppHandle, x: i32, y: i32, triggered_by: &'static str) {
    use tauri::Emitter;

    let selected_text = crate::selection::capture_selected_text();
    let payload = build_show_popup_payload(x, y, triggered_by, selected_text);
    let _ = app.emit("show-popup", payload);
    position_and_show(app, x, y);
}

/// Returns (left, top, right, bottom) of the work area (excludes taskbar)
/// of the monitor nearest the given physical point.
fn monitor_work_area(x: i32, y: i32) -> (i32, i32, i32, i32) {
    unsafe {
        let pt = POINT { x, y };
        let hmonitor: HMONITOR = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmonitor, &mut info).as_bool() {
            let r = info.rcWork;
            (r.left, r.top, r.right, r.bottom)
        } else {
            // Fallback: assume a generous default work area.
            (0, 0, 1920, 1040)
        }
    }
}

/// Clamps the popup's top-left corner so the full popup stays within the
/// monitor work area, flipping to the opposite side of the cursor when it
/// would otherwise overflow an edge. Takes the popup's *actual current*
/// size (not a fixed constant) since the window is user-resizable — see
/// tauri.conf.json "resizable" and the resize handle in AIPopup.tsx —
/// so a previously-enlarged popup still gets clamped correctly on its
/// next open instead of assuming the original default dimensions.
fn clamp_position(cursor_x: i32, cursor_y: i32, popup_width: i32, popup_height: i32) -> (i32, i32) {
    let (left, top, right, bottom) = monitor_work_area(cursor_x, cursor_y);

    let mut px = cursor_x + CURSOR_OFFSET;
    let mut py = cursor_y + CURSOR_OFFSET;

    if px + popup_width > right {
        px = cursor_x - popup_width - CURSOR_OFFSET; // flip to the left of the cursor
    }
    if py + popup_height > bottom {
        py = cursor_y - popup_height - CURSOR_OFFSET; // flip above the cursor
    }
    // Final safety clamp in case flipping still overflows (tiny/odd monitors).
    px = px.clamp(left, (right - popup_width).max(left));
    py = py.clamp(top, (bottom - popup_height).max(top));

    (px, py)
}

pub fn position_and_show(app: &AppHandle, cursor_x: i32, cursor_y: i32) {
    let Some(window) = app.get_webview_window("popup") else {
        return;
    };

    // Use the window's real current size (it may have been resized by the
    // user via the resize handle) rather than the original defaults, so
    // clamping stays correct after a resize.
    let (popup_width, popup_height) = window
        .outer_size()
        .map(|s| (s.width as i32, s.height as i32))
        .unwrap_or((POPUP_WIDTH, POPUP_HEIGHT));

    let (left, top) = clamp_position(cursor_x, cursor_y, popup_width, popup_height);

    apply_capture_exclusion(&window);

    let _ = window.set_position(PhysicalPosition::new(left, top));
    let _ = window.show();
    // Popups intentionally don't steal focus (spec item: "should not steal
    // focus unnecessarily") — the user is usually mid-task in another app.
    // If they click into the input box, the webview grabs focus naturally.
}

/// Excludes the popup from screen capture (Zoom/Teams/Discord/OBS and most
/// other capture tools): the window still renders normally on the user's
/// own physical display, but the OS compositor (DWM) omits it from any
/// capture surface, so viewers of a shared screen never see it. Requires
/// Windows 10 version 2004+; on older Windows this call harmlessly fails
/// and the popup just remains capturable (no crash, no visible error —
/// there's nothing actionable for the user to do about an OS version).
/// Called on every show rather than once at window creation so it's
/// reapplied even if something (e.g. a future DPI/monitor change) ever
/// resets it.
fn apply_capture_exclusion(window: &WebviewWindow) {
    if let Ok(hwnd) = window.hwnd() {
        unsafe {
            let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        }
    }
}

#[tauri::command]
pub fn hide_popup_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("popup") {
        let _ = window.hide();
    }
}

/// Used by the Ctrl+Space global shortcut (registered in main.rs) to open
/// the popup at the current cursor position without going through the
/// mouse hook at all. Cursor position is read synchronously (cheap), but
/// the actual show — which now includes a ~120ms selection-capture round
/// trip — runs on a spawned thread so the global-shortcut callback never
/// blocks waiting on it.
#[tauri::command]
pub fn show_popup_at_cursor(app: AppHandle) {
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let pt = unsafe {
        let mut pt = POINT::default();
        // GetCursorPos returns Result<()> in this crate version, not a BOOL.
        if GetCursorPos(&mut pt).is_ok() {
            Some(pt)
        } else {
            None
        }
    };

    if let Some(pt) = pt {
        std::thread::spawn(move || {
            capture_and_show(&app, pt.x, pt.y, "shortcut");
        });
    }
}

// Keep LogicalPosition import used (reserved for Phase 5 DPI-aware sizing).
#[allow(dead_code)]
fn _unused(p: LogicalPosition<f64>) -> LogicalPosition<f64> {
    p
}
