//! Provider-agnostic AI abstraction.
//!
//! Every concrete provider (Gemini now; OpenAI/Claude later) implements
//! `AIProvider::stream_response`, taking a prompt and a channel-like
//! callback it invokes once per chunk of text as it arrives. The Tauri
//! command in `commands.rs` only knows about this trait — it never talks
//! to a specific vendor's HTTP API directly, so adding OpenAIProvider or
//! ClaudeProvider later is a matter of adding a new file here plus one
//! line in `provider_from_env`, not touching the popup/command code.

pub mod gemini;
pub mod key_pool;

use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum AIError {
    #[error("missing or empty API key — set AI_API_KEY (or AI_API_KEYS for multiple) in .env")]
    MissingApiKey,
    #[error("network error contacting AI provider: {0}")]
    Network(String),
    #[error("AI provider returned an error: {0}")]
    ProviderError(String),
    #[error("failed to parse AI provider response: {0}")]
    ParseError(String),
    #[error("all {total} configured API key(s) are currently rate-limited — wait a bit and try again, or add more keys to AI_API_KEYS (see .env.example)")]
    AllKeysExhausted { total: usize },
}

/// An image attached to a prompt (e.g. from the popup's image-upload
/// button). `data` is raw base64 (no "data:image/png;base64," prefix —
/// that gets stripped on the frontend before this reaches Rust).
pub struct ImageAttachment {
    pub mime_type: String,
    pub base64_data: String,
}

#[async_trait]
pub trait AIProvider: Send + Sync {
    /// Streams a response for `prompt` (optionally with an attached
    /// image, for vision-capable models), invoking `on_chunk` once per
    /// piece of text as it arrives from the provider. Returns once the
    /// stream is fully consumed (or errors out partway through).
    async fn stream_response(
        &self,
        prompt: &str,
        image: Option<ImageAttachment>,
        on_chunk: Box<dyn Fn(String) + Send + Sync>,
    ) -> Result<(), AIError>;
}

/// Reads AI_PROVIDER from the environment and constructs the matching
/// provider, backed by the shared multi-key rotation pool (see
/// key_pool.rs). Defaults to Gemini if unset, since that's what ships in
/// Phase 2. Add new `match` arms here as new providers are implemented —
/// nothing else in the app needs to change.
pub fn provider_from_env() -> Result<Box<dyn AIProvider>, AIError> {
    let provider_name =
        std::env::var("AI_PROVIDER").unwrap_or_else(|_| "gemini".to_string());

    if key_pool::shared_pool().is_empty() {
        return Err(AIError::MissingApiKey);
    }

    match provider_name.to_lowercase().as_str() {
        "gemini" => Ok(Box::new(gemini::GeminiProvider::new())),
        other => Err(AIError::ProviderError(format!(
            "unknown AI_PROVIDER '{other}' — only 'gemini' is implemented in Phase 2"
        ))),
    }
}
