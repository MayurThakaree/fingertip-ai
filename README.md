# Fingertip AI

An AI agent that appears instantly anywhere on your desktop. Open a lightweight popup using a mouse side button or keyboard shortcut, ask questions, rewrite text, summarize content, translate text, and receive streaming AI responses in real time.

Built with **Rust**, **Tauri**, **TypeScript**, and **Google Gemini**.

---

# Features

## Core Features

* Global mouse hook support
* Floating desktop AI popup
* Keyboard shortcut support (`Ctrl + Space`)
* Real-time streaming AI responses
* Secure backend API handling
* Fast and lightweight Tauri architecture

## AI Features

* Explain text
* Rewrite content
* Translate text
* Summarize information
* Streaming token-by-token responses
* Inline error handling

---

# Phase 2 Overview

Phase 2 extends the popup system introduced in Phase 1 by connecting the interface to a real AI backend.

### Added in Phase 2

* Provider-agnostic `AIProvider` trait
* Gemini AI implementation
* Streaming responses using Gemini SSE endpoint
* Tauri `ask_ai` command
* Background async processing
* Frontend event streaming:

  * `ai-chunk`
  * `ai-done`
  * `ai-error`
* Secure environment variable loading
* Quick action prompts
* Loading indicators
* User-friendly error messages

---

# Architecture

```text
User Input
     │
     ▼
Popup UI (Frontend)
     │
     ▼
Tauri Command (ask_ai)
     │
     ▼
AIProvider Trait
     │
     ▼
Gemini Provider
     │
     ▼
Gemini API
     │
     ▼
Streaming Response
     │
     ▼
Popup UI
```

---

# Technology Stack

* Rust
* Tauri
* TypeScript
* HTML
* CSS
* Gemini API
* Reqwest
* Tokio
* dotenvy

---

# Getting a Gemini API Key

Create a Gemini API key from:

[Google AI Studio API Keys](https://aistudio.google.com/apikey?utm_source=chatgpt.com)

Copy the key into `.env.local`.

```bash
cp .env.example .env.local
```

Edit `.env.local`:

```env
AI_PROVIDER=gemini
AI_API_KEY=your-api-key
```

> Never commit `.env.local` to GitHub.

---

# Prerequisites

Windows 10 or Windows 11

Required software:

1. Rust (Stable)
2. Node.js 18+
3. npm
4. Microsoft Edge WebView2 Runtime
5. Microsoft C++ Build Tools

Downloads:

* [Rust Installer](https://rustup.rs?utm_source=chatgpt.com)
* [Node.js](https://nodejs.org?utm_source=chatgpt.com)

---

# Installation

```bash
npm install
```

---

# Running in Development Mode

```bash
npm run tauri dev
```

---

# Testing

### Ask a Question

1. Open the popup using:

   * Mouse side button
   * `Ctrl + Space`

2. Enter a question:

```text
What is Rust's ownership model?
```

3. Press Enter.

4. Watch the response stream live.

---

### Test Quick Actions

Type text and select:

* Explain
* Rewrite
* Translate
* Summarize

The selected action will automatically create a prompt and send it to Gemini.

---

### Test Error Handling

Replace your API key with an invalid value:

```env
AI_API_KEY=invalid-key
```

Restart the application:

```bash
npm run tauri dev
```

You should see an error message displayed inside the popup.

---

# Troubleshooting

| Problem                | Solution                                            |
| ---------------------- | --------------------------------------------------- |
| Missing API key error  | Verify `.env.local` exists and contains a valid key |
| Unknown AI provider    | Ensure `AI_PROVIDER=gemini`                         |
| Response never streams | Check internet connection and Gemini availability   |
| Prompt blocked         | Gemini safety filters rejected the request          |
| Popup stays disabled   | Close and reopen the popup                          |

---

# Project Structure

```text
fingertip-ai/
│
├── src/
├── src-tauri/
│   ├── src/
│   │   ├── ai/
│   │   │   ├── mod.rs
│   │   │   └── gemini.rs
│   │   ├── commands.rs
│   │   └── main.rs
│   │
│   └── Cargo.toml
│
├── .env.example
├── package.json
└── README.md
```

---



# Security

* API keys remain on the backend
* Environment variables are never exposed to the frontend
* `.env.local` is excluded from Git tracking
* No AI credentials are stored in frontend code

---

# Future Vision

Fingertip AI aims to become a universal desktop AI assistant that can understand selected text, screenshots, documents, and on-screen content from any application while remaining fast, lightweight, and privacy-conscious.

---

# License

MIT License

Copyright (c) 2026 Mayur Thakare
