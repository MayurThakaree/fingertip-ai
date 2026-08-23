//! Captures whatever text is currently selected in the foreground app
//! (e.g. a browser, VS Code, a PDF viewer) without the user needing to
//! copy or paste anything themselves.
//!
//! There's no direct Win32 "give me the current selection" API that works
//! across arbitrary apps, so this uses the same trick most "AI everywhere"
//! tools (Raycast, TextSniper, etc.) use: simulate Ctrl+C via SendInput,
//! then read whatever landed on the clipboard. The user's *actual*
//! clipboard content is saved beforehand and restored immediately after,
//! so from their perspective nothing about their clipboard ever changed —
//! this matches the spec requirement to never silently modify or destroy
//! clipboard contents.
//!
//! Because our own popup window never takes focus (see popup::position_and_show),
//! Ctrl+C is simulated while the *previously* focused app still has focus,
//! so the keystroke reaches whatever the user had selected text in.
//!
//! Every step logs to stderr (visible in the `npm run tauri dev` terminal)
//! prefixed with "[selection]" — this whole module was previously a silent
//! black box (every failure was swallowed via .ok()), which made it
//! impossible to tell *why* a capture attempt came back empty. Keep this
//! logging in place until the mechanism is confirmed reliable.

use std::thread;
use std::time::Duration;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL,
};

const VK_C: VIRTUAL_KEY = VIRTUAL_KEY(0x43);

/// Sends one key event and logs how many events Windows actually accepted
/// (SendInput returns the count it successfully inserted into the input
/// stream — 0 means it was rejected, e.g. by UIPI blocking input into a
/// higher-privilege foreground window).
fn send_key(vk: VIRTUAL_KEY, key_up: bool) {
    let flags = if key_up {
        KEYEVENTF_KEYUP
    } else {
        KEYBD_EVENT_FLAGS(0)
    };
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inserted = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    if inserted == 0 {
        eprintln!(
            "[selection] SendInput REJECTED key vk={:#x} key_up={} — likely blocked by UIPI (foreground app may be running elevated/as Administrator while Fingertip AI is not)",
            vk.0, key_up
        );
    }
}

/// Blocks for ~150ms (the Ctrl+C round-trip) — call this from a background
/// thread, never from the mouse-hook callback or the Tauri main thread.
/// Returns `None` if nothing was selected (clipboard content is unchanged
/// after the simulated copy) or if clipboard access fails for any reason.
pub fn capture_selected_text() -> Option<String> {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[selection] failed to open clipboard: {e}");
            return None;
        }
    };

    let previous_text = match clipboard.get_text() {
        Ok(t) => Some(t),
        Err(e) => {
            eprintln!("[selection] no pre-existing clipboard text (or read failed): {e}");
            None
        }
    };
    eprintln!(
        "[selection] clipboard before capture: {:?} chars",
        previous_text.as_ref().map(|t| t.len())
    );

    eprintln!("[selection] simulating Ctrl+C...");
    send_key(VK_CONTROL, false);
    send_key(VK_C, false);
    send_key(VK_C, true);
    send_key(VK_CONTROL, true);

    thread::sleep(Duration::from_millis(150));

    let captured_text = match clipboard.get_text() {
        Ok(t) => Some(t),
        Err(e) => {
            eprintln!("[selection] clipboard read after Ctrl+C failed: {e}");
            None
        }
    };
    eprintln!(
        "[selection] clipboard after capture: {:?} chars",
        captured_text.as_ref().map(|t| t.len())
    );

    // Restore the user's original clipboard exactly, whether that was some
    // text or nothing at all.
    match &previous_text {
        Some(text) => {
            if let Err(e) = clipboard.set_text(text.clone()) {
                eprintln!("[selection] failed to restore original clipboard: {e}");
            }
        }
        None => {
            let _ = clipboard.clear();
        }
    }

    match captured_text {
        Some(text) if Some(&text) != previous_text.as_ref() && !text.trim().is_empty() => {
            eprintln!(
                "[selection] SUCCESS — captured {} chars of new selection",
                text.len()
            );
            Some(text)
        }
        Some(_) => {
            eprintln!(
                "[selection] clipboard unchanged after Ctrl+C — nothing was selected, or the Ctrl+C keystroke never reached the target app"
            );
            None
        }
        None => {
            eprintln!("[selection] no text on clipboard after Ctrl+C");
            None
        }
    }
}
