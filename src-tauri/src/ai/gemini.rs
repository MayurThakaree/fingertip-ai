//! Gemini implementation of `AIProvider`.
//!
//! Calls Gemini's `streamGenerateContent` endpoint with `alt=sse`, which
//! returns a Server-Sent-Events stream: a sequence of `data: {...}\n\n`
//! frames, each a JSON chunk of the response. We parse those frames
//! incrementally as bytes arrive over the network (not after the whole
//! response is buffered), extract the text delta from each one, and hand
//! it to the `on_chunk` callback so the popup can render it as it types.
//!
//! Image support: if an `ImageAttachment` is passed, it's sent as an
//! additional `inline_data` part alongside the text prompt — Gemini's
//! vision-capable models (the default flash model included) accept
//! image+text in the same `contents` array with no separate endpoint.
//!
//! Multi-key rotation: on a 429 (rate limit) response — checked BEFORE
//! any streaming/on_chunk calls happen, so a rotation never produces a
//! half-shown answer — this marks the current key on cooldown (using
//! Google's own suggested retry delay when present) via the shared
//! KeyPool and immediately retries with the next available key. Only
//! once every key is cooling down does this surface an error. Any
//! non-429 error is NOT retried with another key (a bad prompt or
//! network failure will fail the same way on every key).

use super::key_pool::KeyPool;
use super::{AIError, AIProvider, ImageAttachment};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const DEFAULT_MODEL: &str = "gemini-3.6-flash";

// Prepended to every prompt so responses come back as well-structured
// Markdown (headings, bullet lists, numbered steps, code blocks) instead
// of a single unbroken paragraph — the popup renders Markdown, so this
// directly controls how "organized" answers look. Also caps length —
// the popup is a small floating panel, not a document viewer, so a
// default "explain everything exhaustively" answer overflows it badly.
const FORMAT_INSTRUCTION: &str = "Answer concisely — a few short paragraphs or a focused list at most, not an exhaustive essay. Skip preamble like \"Sure, here's...\" and get straight to the point. Format as clean Markdown: bold labels or headings for distinct sections, bullet/numbered lists for steps, and fenced code blocks (with the correct language tag, e.g. ```python, ```javascript) for ANY code, command, or file content — never inline code in a sentence when it's more than a few words. If the question is fundamentally about code, prioritize showing the code itself over describing it in prose.\n\n---\n\n";

/// Fallback cooldown when Google's error response doesn't include a
/// parseable retryDelay (rare, but don't loop forever if so).
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(60);

pub struct GeminiProvider {
    model: String,
    client: reqwest::Client,
}

impl GeminiProvider {
    pub fn new() -> Self {
        let model = std::env::var("AI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        Self {
            model,
            client: reqwest::Client::new(),
        }
    }
}

/// Outcome of a single request attempt with one specific key — lets the
/// caller decide whether to rotate to the next key (RateLimited only) or
/// give up immediately (every other error).
enum AttemptOutcome {
    Success,
    RateLimited(Duration),
    Failed(AIError),
}

/// Parses Google's `retryDelay` field (e.g. "30s", "30.58s") into a
/// Duration. Falls back to DEFAULT_COOLDOWN if the format is unexpected —
/// this is a cooldown heuristic, not something worth failing hard over.
fn parse_retry_delay(raw: &str) -> Duration {
    raw.trim_end_matches('s')
        .parse::<f64>()
        .map(Duration::from_secs_f64)
        .unwrap_or(DEFAULT_COOLDOWN)
}

/// Turns a non-2xx HTTP response into a short, human-readable AIError
/// (used for the final "every key exhausted" style messages) — separate
/// from the 429-detection in `attempt_request`, which needs the raw
/// retryDelay rather than a pre-formatted string.
fn build_http_error(status: reqwest::StatusCode, body: &str) -> AIError {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or(body)
        .to_string();
    AIError::ProviderError(format!("HTTP {status}: {message}"))
}

#[derive(Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(rename = "generationConfig")]
    generation_config: GeminiGenerationConfig,
}

#[derive(Serialize)]
struct GeminiContent {
    parts: Vec<GeminiPart>,
}

// Gemini's `parts` array is polymorphic: either a plain text part or an
// inline-image part. Representing both as one enum with `#[serde(untagged)]`
// lets serde pick the right shape per-variant when serializing.
#[derive(Serialize, Clone)]
#[serde(untagged)]
enum GeminiPart {
    Text { text: String },
    InlineImage { #[serde(rename = "inlineData")] inline_data: GeminiInlineData },
}

#[derive(Serialize, Clone)]
struct GeminiInlineData {
    #[serde(rename = "mimeType")]
    mime_type: String,
    data: String,
}

#[derive(Serialize)]
struct GeminiGenerationConfig {
    temperature: f32,
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: u32,
}

// Only the fields we actually read from each SSE chunk.
#[derive(Deserialize)]
struct GeminiStreamChunk {
    candidates: Option<Vec<GeminiCandidate>>,
    #[serde(rename = "promptFeedback")]
    prompt_feedback: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: Option<GeminiContentResponse>,
}

#[derive(Deserialize)]
struct GeminiContentResponse {
    parts: Option<Vec<GeminiPartResponse>>,
}

#[derive(Deserialize)]
struct GeminiPartResponse {
    text: Option<String>,
}

impl GeminiProvider {
    /// One attempt with one specific API key. Checks the response status
    /// BEFORE touching the streaming body, so a 429 is detected and can
    /// trigger key rotation without ever calling on_chunk — the caller
    /// only sees a Success after the full stream is genuinely consumed.
    async fn attempt_request(
        &self,
        api_key: &str,
        parts: &[GeminiPart],
        on_chunk: &(dyn Fn(String) + Send + Sync),
    ) -> AttemptOutcome {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse",
            self.model
        );

        let body = GeminiRequest {
            contents: vec![GeminiContent { parts: parts.to_vec() }],
            generation_config: GeminiGenerationConfig {
                temperature: std::env::var("AI_TEMPERATURE")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0.7),
                // Hard backstop on response length — the FORMAT_INSTRUCTION
                // asks the model to be concise, but this caps it even if
                // it ignores that. ~600 tokens is roughly a medium-length
                // paragraph-or-two answer, appropriate for a small floating
                // popup rather than a full document. Overridable via env
                // for anyone who wants longer answers back.
                max_output_tokens: std::env::var("AI_MAX_OUTPUT_TOKENS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(600),
            },
        };

        let response = match self
            .client
            .post(&url)
            .header("x-goog-api-key", api_key)
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return AttemptOutcome::Failed(AIError::Network(e.to_string())),
        };

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();

            if status.as_u16() == 429 {
                let parsed: Option<serde_json::Value> = serde_json::from_str(&text).ok();
                let retry_delay = parsed
                    .as_ref()
                    .and_then(|v| v.get("error"))
                    .and_then(|e| e.get("details"))
                    .and_then(|d| d.as_array())
                    .and_then(|arr| {
                        arr.iter()
                            .find_map(|detail| detail.get("retryDelay").and_then(|d| d.as_str()))
                    })
                    .map(parse_retry_delay)
                    .unwrap_or(DEFAULT_COOLDOWN);
                return AttemptOutcome::RateLimited(retry_delay);
            }

            return AttemptOutcome::Failed(build_http_error(status, &text));
        }

        // SSE frames are separated by a blank line. Some servers send
        // "\n\n", others send "\r\n\r\n" — normalizing CRLF to LF as bytes
        // arrive means both work, and the response body can still split a
        // frame across multiple network chunks, so we buffer text and only
        // parse once we see a complete frame boundary.
        let mut byte_stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut emitted_any_text = false;

        while let Some(chunk_result) = byte_stream.next().await {
            let bytes = match chunk_result {
                Ok(b) => b,
                Err(e) => return AttemptOutcome::Failed(AIError::Network(e.to_string())),
            };
            let decoded = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
            buffer.push_str(&decoded);

            while let Some(frame_end) = buffer.find("\n\n") {
                let frame = buffer[..frame_end].to_string();
                buffer.drain(..frame_end + 2);

                for line in frame.lines() {
                    let Some(data) = line.strip_prefix("data: ") else {
                        continue;
                    };
                    if data.trim() == "[DONE]" {
                        continue;
                    }

                    let parsed: GeminiStreamChunk = match serde_json::from_str(data) {
                        Ok(p) => p,
                        Err(e) => return AttemptOutcome::Failed(AIError::ParseError(e.to_string())),
                    };

                    if let Some(feedback) = parsed.prompt_feedback {
                        return AttemptOutcome::Failed(AIError::ProviderError(format!(
                            "prompt blocked: {feedback}"
                        )));
                    }

                    if let Some(candidates) = parsed.candidates {
                        for candidate in candidates {
                            if let Some(content) = candidate.content {
                                if let Some(resp_parts) = content.parts {
                                    for part in resp_parts {
                                        if let Some(text) = part.text {
                                            if !text.is_empty() {
                                                emitted_any_text = true;
                                                on_chunk(text);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if !emitted_any_text {
            return AttemptOutcome::Failed(AIError::ParseError(
                "stream ended with no readable response text".to_string(),
            ));
        }

        AttemptOutcome::Success
    }
}

#[async_trait]
impl AIProvider for GeminiProvider {
    async fn stream_response(
        &self,
        prompt: &str,
        image: Option<ImageAttachment>,
        on_chunk: Box<dyn Fn(String) + Send + Sync>,
    ) -> Result<(), AIError> {
        let mut parts = vec![GeminiPart::Text {
            text: format!("{FORMAT_INSTRUCTION}{prompt}"),
        }];
        if let Some(img) = image {
            parts.push(GeminiPart::InlineImage {
                inline_data: GeminiInlineData {
                    mime_type: img.mime_type,
                    data: img.base64_data,
                },
            });
        }

        let pool: &KeyPool = super::key_pool::shared_pool();

        loop {
            let Some((index, key)) = pool.next_available() else {
                return Err(AIError::AllKeysExhausted { total: pool.len() });
            };

            match self.attempt_request(&key, &parts, on_chunk.as_ref()).await {
                AttemptOutcome::Success => return Ok(()),
                AttemptOutcome::RateLimited(cooldown) => {
                    eprintln!(
                        "[gemini] key #{index} rate-limited, cooling down for {:.0}s, trying next key ({} total configured)",
                        cooldown.as_secs_f64(),
                        pool.len()
                    );
                    pool.mark_cooldown(index, cooldown);
                    continue;
                }
                AttemptOutcome::Failed(e) => return Err(e),
            }
        }
    }
}
