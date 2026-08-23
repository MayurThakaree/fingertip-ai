//! Rotates across multiple API keys for a single provider, so hitting one
//! key's rate limit doesn't stop the app — it just moves to the next key.
//!
//! Gemini (and most providers) enforce quota per *project*, not per key,
//! so this only actually multiplies your daily capacity if each key comes
//! from a separate project — see the AI_API_KEYS docs in .env.example.
//! Keys from the same project share one quota pool and will all appear
//! "cooled down" together, which is expected, not a bug.
//!
//! State (which keys are currently cooling down, and the round-robin
//! cursor) lives in a process-wide static so it persists across requests
//! — each `ask_ai` call builds a fresh `GeminiProvider`, but they all
//! share the same underlying `KeyPool`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub struct KeyPool {
    keys: Vec<String>,
    /// Parallel to `keys` — Some(instant) means "don't retry this key until
    /// this time has passed". None means the key is currently usable.
    cooldowns: Mutex<Vec<Option<Instant>>>,
    cursor: AtomicUsize,
}

impl KeyPool {
    fn new(keys: Vec<String>) -> Self {
        let len = keys.len();
        Self {
            keys,
            cooldowns: Mutex::new(vec![None; len]),
            cursor: AtomicUsize::new(0),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Returns the next (index, key) that isn't currently cooling down,
    /// advancing the round-robin cursor so repeated calls cycle through
    /// all keys evenly rather than always preferring key 0. Returns None
    /// if every key is currently on cooldown.
    pub fn next_available(&self) -> Option<(usize, String)> {
        let cooldowns = self.cooldowns.lock().unwrap();
        let now = Instant::now();
        let len = self.keys.len();

        for offset in 0..len {
            let idx = (self.cursor.load(Ordering::Relaxed) + offset) % len;
            let still_cooling = cooldowns[idx].map(|until| now < until).unwrap_or(false);
            if !still_cooling {
                self.cursor.store((idx + 1) % len, Ordering::Relaxed);
                return Some((idx, self.keys[idx].clone()));
            }
        }
        None
    }

    /// Marks a key as rate-limited for `cooldown`, so subsequent calls to
    /// `next_available` skip it until that time passes.
    pub fn mark_cooldown(&self, index: usize, cooldown: Duration) {
        let mut cooldowns = self.cooldowns.lock().unwrap();
        if let Some(slot) = cooldowns.get_mut(index) {
            *slot = Some(Instant::now() + cooldown);
        }
    }

    /// How many keys are currently usable (not cooling down) — used to
    /// build a clear "all N keys exhausted" error message. Reserved for a
    /// future settings/status UI (Phase 5); not called yet.
    #[allow(dead_code)]
    pub fn available_count(&self) -> usize {
        let cooldowns = self.cooldowns.lock().unwrap();
        let now = Instant::now();
        cooldowns
            .iter()
            .filter(|c| c.map(|until| now >= until).unwrap_or(true))
            .count()
    }
}

static POOL: OnceLock<KeyPool> = OnceLock::new();

/// Reads AI_API_KEYS (comma-separated, preferred — see .env.example) or
/// falls back to a single AI_API_KEY. Built once and cached: env vars
/// don't change mid-run, but the cooldown *state* inside the pool very
/// much does, so this must stay a shared singleton, not rebuilt per call.
pub fn shared_pool() -> &'static KeyPool {
    POOL.get_or_init(|| {
        let multi = std::env::var("AI_API_KEYS").unwrap_or_default();
        let keys: Vec<String> = if !multi.trim().is_empty() {
            multi
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            std::env::var("AI_API_KEY")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .into_iter()
                .collect()
        };
        KeyPool::new(keys)
    })
}
