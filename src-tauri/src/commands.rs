//! Frontend-facing commands for Phase 2 chat.
//!
//! `ask_ai` is invoked by the popup's input box. It runs the actual HTTP
//! call on a background async task (so it never blocks the Tauri event
//! loop or the mouse hook thread) and streams progress back to the
//! *same* popup window via three events the frontend listens for:
//!   - "ai-chunk": { requestId, text }  — one per piece of streamed text
//!   - "ai-done":  { requestId }        — stream finished successfully
//!   - "ai-error": { requestId, message } — something went wrong
//!
//! `requestId` lets the frontend ignore stale events if the user closes
//! the popup and reopens it with a new question before the old request
//! finishes.

use crate::ai;
use crate::ai::ImageAttachment;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Clone)]
struct ChunkPayload {
    #[serde(rename = "requestId")]
    request_id: String,
    text: String,
}

#[derive(Serialize, Clone)]
struct DonePayload {
    #[serde(rename = "requestId")]
    request_id: String,
}

#[derive(Serialize, Clone)]
struct ErrorPayload {
    #[serde(rename = "requestId")]
    request_id: String,
    message: String,
}

#[tauri::command]
pub async fn ask_ai(
    app: AppHandle,
    request_id: String,
    prompt: String,
    // Optional attached image, base64-encoded on the frontend (no data URI
    // prefix — that's stripped before invoke). Both fields must be present
    // together or both absent.
    image_base64: Option<String>,
    image_mime_type: Option<String>,
) -> Result<(), String> {
    if prompt.trim().is_empty() {
        return Err("empty prompt".into());
    }

    let image = match (image_base64, image_mime_type) {
        (Some(data), Some(mime_type)) => Some(ImageAttachment {
            mime_type,
            base64_data: data,
        }),
        _ => None,
    };

    // Building the provider per-request (cheap — just reads env vars) keeps
    // this command decoupled from app startup state, and means a user who
    // fixes a bad API key in .env doesn't need to restart the whole app
    // once Phase 5's settings panel can rewrite it.
    let provider = match ai::provider_from_env() {
        Ok(p) => p,
        Err(e) => {
            emit_error(&app, &request_id, &e.to_string());
            return Err(e.to_string());
        }
    };

    let app_for_chunks = app.clone();
    let request_id_for_chunks = request_id.clone();

    let on_chunk = Box::new(move |text: String| {
        let _ = app_for_chunks.emit(
            "ai-chunk",
            ChunkPayload {
                request_id: request_id_for_chunks.clone(),
                text,
            },
        );
    });

    match provider.stream_response(&prompt, image, on_chunk).await {
        Ok(()) => {
            let _ = app.emit(
                "ai-done",
                DonePayload {
                    request_id: request_id.clone(),
                },
            );
            Ok(())
        }
        Err(e) => {
            emit_error(&app, &request_id, &e.to_string());
            Err(e.to_string())
        }
    }
}

fn emit_error(app: &AppHandle, request_id: &str, message: &str) {
    let _ = app.emit(
        "ai-error",
        ErrorPayload {
            request_id: request_id.to_string(),
            message: message.to_string(),
        },
    );
}

/// On-demand selection capture, used by the quick-action buttons when the
/// popup opened without a selection already captured (e.g. the trigger
/// fired before text was selected, or the earlier capture missed it).
/// Blocking (~120ms clipboard round trip) — this is an `async` command so
/// Tauri runs it off the main thread automatically, keeping the UI
/// responsive while it waits.
#[tauri::command]
pub async fn capture_selection_now() -> Option<String> {
    crate::selection::capture_selected_text()
}
