# Fingertip AI — Phase 2

Builds on Phase 1's global mouse hook + floating popup by wiring the chat
input up to a real, streaming AI response — powered by Gemini, via the
provider-agnostic `AIProvider` trait so OpenAI/Claude can be added later
without touching the popup or command code.

## What's new in this phase

- `AIProvider` trait (`src-tauri/src/ai/mod.rs`) with a Gemini
  implementation (`src-tauri/src/ai/gemini.rs`) that streams responses via
  Gemini's `streamGenerateContent?alt=sse` endpoint.
- `ask_ai` Tauri command (`src-tauri/src/commands.rs`) — takes the prompt,
  calls the provider on a background async task, and emits `ai-chunk` /
  `ai-done` / `ai-error` events back to the popup as the response streams
  in. The API key is read from `.env`/`.env.local` via `dotenvy` at
  startup and never sent to or readable from the frontend JS.
- Popup input box is now live: type a question, hit Enter, watch the
  response stream in token-by-token with a loading indicator while
  waiting for the first chunk.
- Quick-action buttons (Explain / Rewrite / Translate / Summarize) send a
  templated prompt built around whatever's currently typed. They'll pick
  up real selected-text context in Phase 3.
- Errors (bad/missing API key, network failure, blocked prompt) surface
  inline in the popup instead of failing silently.

## Get a Gemini API key

1. Go to https://aistudio.google.com/apikey
2. Create a key (free tier is enough to test this)
3. Copy it into your `.env.local`:

```bash
cp .env.example .env.local
```

Edit `.env.local`:
```env
AI_PROVIDER=gemini
AI_API_KEY=paste-your-real-key-here
```

**Never commit `.env.local`** — it's already in `.gitignore`.

## Prerequisites (same as Phase 1 — Windows 10/11 only)

1. **Rust** (stable) — https://rustup.rs
2. **Node.js 18+** and npm
3. **WebView2 runtime** — usually preinstalled
4. Microsoft C++ Build Tools ("Desktop development with C++" workload)

## Installation

```bash
npm install
```

## Development

```bash
npm run tauri dev
```

## How to test Phase 2

1. Long-press your mouse's side button (or `Ctrl+Space`) to open the
   popup, same as Phase 1.
2. Type a question — e.g. `"What is Rust's ownership model?"` — and press
   Enter.
3. You should see a brief loading indicator (three bouncing dots), then
   the response should stream in visibly, piece by piece.
4. Try a quick-action button (e.g. type some text, click **Explain**) —
   it should send a templated prompt built from what you typed.
5. Test the error path: temporarily put a garbage value in `AI_API_KEY`
   in `.env.local`, restart `npm run tauri dev`, and confirm you get a
   clear inline error in the popup instead of a silent failure or crash.

## Troubleshooting (Phase 2 additions)

| Symptom | Likely cause / fix |
|---|---|
| "missing or empty API key" error in the popup | `.env.local` wasn't found/loaded, or `AI_API_KEY` is blank. Confirm the file is in `fingertip-ai/` (same level as `package.json`, NOT inside `src-tauri/`), and restart `npm run tauri dev` after editing it — env vars are only read at process startup. |
| "unknown AI_PROVIDER" error | Only `gemini` is implemented in Phase 2. Check `.env.local` doesn't have a typo like `Gemini` with capital G — comparison is case-insensitive so that's fine, but `gemeni` or similar would fail. |
| Response never starts streaming, no error either | Check your internet connection, and that the Gemini API isn't rate-limited/blocked on your network. Also confirm `reqwest`'s TLS feature compiled correctly — a fresh `cargo clean` in `src-tauri/` then re-running `npm run tauri dev` can help if you swapped Rust toolchains recently. |
| "prompt blocked" error | Gemini's safety filters rejected the input. Try rephrasing; this isn't a bug. |
| Popup input box stays disabled after an error | This is currently a known Phase 2 rough edge — the input only re-enables once the message reaches `"done"` or `"error"` status. If it seems stuck, close and reopen the popup as a workaround; will be hardened in a later pass. |

## Next phases (not in this build)

- **Phase 3**: clipboard-based "use selected text?" flow, so quick actions operate on real selected text instead of the typed input box.
- **Phase 4**: manual screenshot capture + vision model call.
- **Phase 5**: system tray, full settings panel (mouse/AI/appearance), configurable shortcut, signed installer.
