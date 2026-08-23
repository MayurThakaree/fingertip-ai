// Prevents an extra console window from appearing on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod commands;
mod mouse_hook;
mod popup;
mod selection;

fn main() {
    // Cargo's working directory when Tauri launches the backend is
    // src-tauri/, NOT the project root (where package.json lives) — so we
    // check both locations for .env / .env.local. This makes the env file
    // work whether it was placed next to package.json (as instructed) or
    // accidentally next to Cargo.toml.
    load_env_from_both_locations();

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let handle = app.handle().clone();

            // Phase 1 core loop: install the global low-level mouse hook on
            // its own thread. It runs independent of window focus, which is
            // the whole point — the user can be in Chrome/VS Code/a game and
            // still trigger the popup.
            mouse_hook::start(handle.clone());

            // Ctrl+Space fallback trigger (spec item #11). Registered here so
            // it's live as soon as the app starts, same lifetime as the mouse
            // hook.
            #[cfg(desktop)]
            {
                use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};
                let shortcut = Shortcut::new(Some(Modifiers::CONTROL), Code::Space);
                let app_for_shortcut = handle.clone();
                app.global_shortcut().on_shortcut(shortcut, move |_app, _shortcut, _event| {
                    popup::show_popup_at_cursor(app_for_shortcut.clone());
                })?;
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            popup::hide_popup_window,
            popup::show_popup_at_cursor,
            commands::ask_ai,
            commands::capture_selection_now,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Fingertip AI");
}

/// Tries, in order: .env / .env.local in the current working directory
/// (src-tauri/ during `cargo run`, or next to the .exe in a release
/// build), then the same two files one directory up (the project root,
/// where the README tells the user to create .env.local). First match
/// for each filename wins; missing files are silently skipped — ask_ai
/// surfaces a clear error later if no key was ever found. Never logs the
/// actual key contents.
fn load_env_from_both_locations() {
    use std::path::PathBuf;

    let mut candidates: Vec<PathBuf> = vec![
        PathBuf::from(".env.local"),
        PathBuf::from(".env"),
        PathBuf::from("../.env.local"),
        PathBuf::from("../.env"),
    ];

    // Also check relative to the compiled binary's own location, which
    // covers `tauri build` output where the working directory can differ
    // from both of the above.
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            candidates.push(exe_dir.join(".env.local"));
            candidates.push(exe_dir.join(".env"));
        }
    }

    for path in candidates {
        if path.exists() {
            let _ = dotenvy::from_path(&path);
        }
    }
}
