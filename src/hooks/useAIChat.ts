import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  text: string;
  status: "streaming" | "done" | "error";
  imagePreviewUrl?: string; // shown on the user's own message bubble only
  attachmentKind?: "image" | "audio";
}

export interface Attachment {
  /** base64 payload with no "data:...;base64," prefix — that's stripped
   * before it's sent to Rust, which forwards it to Gemini as-is. */
  base64: string;
  mimeType: string;
  /** object URL for a local thumbnail preview — image attachments only;
   * audio attachments render as a badge instead (see AIPopup.tsx). */
  previewUrl?: string;
  kind: "image" | "audio";
}

interface ChunkPayload {
  requestId: string;
  text: string;
}
interface DonePayload {
  requestId: string;
}
interface ErrorPayload {
  requestId: string;
  message: string;
}

/**
 * Owns the popup's conversation state for the current session (cleared
 * when the popup closes — persistent history across sessions is a later
 * phase). Talks to the Rust `ask_ai` command and reconciles streamed
 * "ai-chunk" / "ai-done" / "ai-error" events against the in-flight
 * requestId, so a stale response from a previous question can't clobber
 * a newer one if the user fires off a second question quickly.
 */
export function useAIChat() {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const activeRequestId = useRef<string | null>(null);

  useEffect(() => {
    let unlistenChunk: UnlistenFn;
    let unlistenDone: UnlistenFn;
    let unlistenError: UnlistenFn;

    (async () => {
      unlistenChunk = await listen<ChunkPayload>("ai-chunk", (event) => {
        const { requestId, text } = event.payload;
        if (requestId !== activeRequestId.current) return; // stale response
        setMessages((prev) =>
          prev.map((m) => (m.id === requestId ? { ...m, text: m.text + text } : m))
        );
      });

      unlistenDone = await listen<DonePayload>("ai-done", (event) => {
        if (event.payload.requestId !== activeRequestId.current) return;
        setMessages((prev) =>
          prev.map((m) =>
            m.id === event.payload.requestId ? { ...m, status: "done" as const } : m
          )
        );
      });

      unlistenError = await listen<ErrorPayload>("ai-error", (event) => {
        if (event.payload.requestId !== activeRequestId.current) return;
        setMessages((prev) =>
          prev.map((m) =>
            m.id === event.payload.requestId
              ? { ...m, status: "error" as const, text: event.payload.message }
              : m
          )
        );
      });
    })();

    return () => {
      unlistenChunk?.();
      unlistenDone?.();
      unlistenError?.();
    };
  }, []);

  const send = useCallback(async (prompt: string, attachment?: Attachment | null) => {
    const trimmed = prompt.trim();
    if (!trimmed) return;

    const requestId = crypto.randomUUID();
    activeRequestId.current = requestId;

    setMessages((prev) => [
      ...prev,
      {
        id: `${requestId}-user`,
        role: "user",
        text: trimmed,
        status: "done",
        imagePreviewUrl: attachment?.previewUrl,
        attachmentKind: attachment?.kind,
      },
      { id: requestId, role: "assistant", text: "", status: "streaming" },
    ]);

    try {
      await invoke("ask_ai", {
        requestId,
        prompt: trimmed,
        imageBase64: attachment?.base64 ?? null,
        imageMimeType: attachment?.mimeType ?? null,
      });
    } catch (err) {
      // invoke() rejects if the command itself returns Err(...) — the Rust
      // side already emitted "ai-error" in that case, so this just guards
      // against an unexpected invoke-level failure (e.g. IPC issue).
      setMessages((prev) =>
        prev.map((m) =>
          m.id === requestId
            ? { ...m, status: "error" as const, text: String(err) }
            : m
        )
      );
    }
  }, []);

  const reset = useCallback(() => {
    activeRequestId.current = null;
    setMessages([]);
  }, []);

  return { messages, send, reset };
}
