//! Global low-level mouse hook (WH_MOUSE_LL).
//!
//! Windows requires low-level hooks to run on a thread that pumps a Win32
//! message loop, so this runs on its own dedicated OS thread — separate
//! from Tauri's event loop — and communicates back via a channel + a
//! Tauri AppHandle captured at spawn time.
//!
//! Long-press algorithm (same regardless of which button is configured):
//!   BUTTONDOWN (matching configured trigger) -> start a timer thread.
//!   If BUTTONUP for that trigger arrives before LONG_PRESS_DURATION,
//!     cancel the timer (treat as a normal click, do nothing).
//!   If the timer fires first, emit "show-popup" with the cursor position
//!     captured at press time (not release time — the user's hand may have
//!     drifted slightly during the hold, and anchoring to the original
//!     press point feels more predictable).
//!
//! Trigger button: not every mouse has side (X) buttons — many basic mice
//! only have left/right/middle — so the trigger is configurable between
//! XBUTTON1, XBUTTON2, and MIDDLE (scroll-wheel click). Default is MIDDLE
//! since it's present on virtually every mouse; side buttons remain
//! available for mice that have them (see MouseHookConfig::default and
//! the Phase 5 settings panel this will eventually feed).
//!
//! The hook proc itself must be fast and non-blocking (Windows will
//! silently unregister slow hooks), so all it does is post to an atomic
//! state + spawn a short-lived timer thread; no I/O or Tauri calls happen
//! directly inside the hook callback.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tauri::AppHandle;
use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, HHOOK, MSG, MSLLHOOKSTRUCT, WH_MOUSE_LL, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_XBUTTONDOWN, WM_XBUTTONUP,
};

/// Which physical button triggers the popup. Stored as a plain u32 in the
/// atomic config (0 = Middle, 1 = XButton1, 2 = XButton2) so it can be
/// swapped at runtime later (Phase 5 settings) without restarting the hook.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TriggerButton {
    Middle,
    XButton1,
    XButton2,
}

impl TriggerButton {
    fn as_code(self) -> u32 {
        match self {
            TriggerButton::Middle => 0,
            TriggerButton::XButton1 => 1,
            TriggerButton::XButton2 => 2,
        }
    }
    fn from_code(code: u32) -> Self {
        match code {
            1 => TriggerButton::XButton1,
            2 => TriggerButton::XButton2,
            _ => TriggerButton::Middle,
        }
    }
}

/// Runtime-configurable settings shared with the hook thread via atomics
/// so the settings panel (Phase 5) can update them without restarting
/// the hook.
pub struct MouseHookConfig {
    pub long_press_ms: AtomicU32,
    pub trigger_button: AtomicU32, // see TriggerButton::as_code/from_code
    pub enabled: AtomicBool,
}

impl Default for MouseHookConfig {
    fn default() -> Self {
        Self {
            long_press_ms: AtomicU32::new(700),
            // Middle-click is the safest default: every mouse has one,
            // unlike side buttons which many basic/laptop mice lack.
            trigger_button: AtomicU32::new(TriggerButton::Middle.as_code()),
            enabled: AtomicBool::new(true),
        }
    }
}

static CONFIG: OnceLock<Arc<MouseHookConfig>> = OnceLock::new();
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
static HOOK_HANDLE: Mutex<Option<isize>> = Mutex::new(None);

// Press-tracking state, valid for a single button-down/up cycle at a time.
static PRESS_ACTIVE: AtomicBool = AtomicBool::new(false);
static PRESS_X: AtomicI32 = AtomicI32::new(0);
static PRESS_Y: AtomicI32 = AtomicI32::new(0);
/// Monotonically increasing token: incremented on every button-up so a
/// timer thread from a *previous* press can recognize it was cancelled.
static PRESS_TOKEN: AtomicU32 = AtomicU32::new(0);

pub fn config() -> Arc<MouseHookConfig> {
    CONFIG.get_or_init(|| Arc::new(MouseHookConfig::default())).clone()
}

/// Spawns the hook thread. Call once from main().
pub fn start(app_handle: AppHandle) {
    let _ = APP_HANDLE.set(app_handle);
    config(); // ensure initialized

    std::thread::spawn(|| unsafe {
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(low_level_mouse_proc), None, 0)
            .expect("failed to install WH_MOUSE_LL hook");
        *HOOK_HANDLE.lock().unwrap() = Some(hook.0 as isize);

        // Required message pump for a low-level hook to receive callbacks.
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = UnhookWindowsHookEx(hook);
    });
}

/// Stops and unregisters the hook. Not called yet in Phase 2 — wired up to
/// the tray "Pause" menu item in Phase 5.
#[allow(dead_code)]
pub fn stop() {
    if let Some(raw) = HOOK_HANDLE.lock().unwrap().take() {
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(raw as *mut _));
        }
    }
}

unsafe extern "system" fn low_level_mouse_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        let cfg = config();
        if cfg.enabled.load(Ordering::Relaxed) {
            let msg = wparam.0 as u32;
            let trigger = TriggerButton::from_code(cfg.trigger_button.load(Ordering::Relaxed));
            let data = *(lparam.0 as *const MSLLHOOKSTRUCT);

            match trigger {
                TriggerButton::Middle => {
                    if msg == WM_MBUTTONDOWN {
                        handle_button_down(data.pt, cfg.long_press_ms.load(Ordering::Relaxed));
                    } else if msg == WM_MBUTTONUP {
                        handle_button_up();
                    }
                }
                TriggerButton::XButton1 | TriggerButton::XButton2 => {
                    if msg == WM_XBUTTONDOWN || msg == WM_XBUTTONUP {
                        // High word of mouseData encodes which X button (1 or 2).
                        let button = ((data.mouseData >> 16) & 0xFFFF) as u32;
                        let wanted = if trigger == TriggerButton::XButton1 { 1 } else { 2 };
                        if button == wanted {
                            if msg == WM_XBUTTONDOWN {
                                handle_button_down(
                                    data.pt,
                                    cfg.long_press_ms.load(Ordering::Relaxed),
                                );
                            } else {
                                handle_button_up();
                            }
                        }
                    }
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

fn handle_button_down(pt: POINT, long_press_ms: u32) {
    PRESS_ACTIVE.store(true, Ordering::Relaxed);
    PRESS_X.store(pt.x, Ordering::Relaxed);
    PRESS_Y.store(pt.y, Ordering::Relaxed);
    let my_token = PRESS_TOKEN.load(Ordering::Relaxed);

    // Spawn a lightweight timer thread rather than blocking the hook.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(long_press_ms as u64));

        // Only fire if this is still the same press (not released/cancelled,
        // and no newer press has started) and it's still marked active.
        if PRESS_ACTIVE.load(Ordering::Relaxed) && PRESS_TOKEN.load(Ordering::Relaxed) == my_token
        {
            PRESS_ACTIVE.store(false, Ordering::Relaxed);
            let x = PRESS_X.load(Ordering::Relaxed);
            let y = PRESS_Y.load(Ordering::Relaxed);
            emit_show_popup(x, y);
        }
    });
}

fn handle_button_up() {
    // Cancel any pending long-press timer for this press.
    PRESS_ACTIVE.store(false, Ordering::Relaxed);
    PRESS_TOKEN.fetch_add(1, Ordering::Relaxed);
}

fn emit_show_popup(x: i32, y: i32) {
    if let Some(app) = APP_HANDLE.get() {
        // Already running on the spawned long-press timer thread (see
        // handle_button_down below), so the ~120ms selection-capture
        // round trip inside capture_and_show doesn't block the hook.
        crate::popup::capture_and_show(app, x, y, "mouse");
    }
}
