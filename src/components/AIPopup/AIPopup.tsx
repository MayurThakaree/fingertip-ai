import { useEffect, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark, oneLight } from "react-syntax-highlighter/dist/esm/styles/prism";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useAIChat, type Attachment } from "../../hooks/useAIChat";
import "./AIPopup.css";

type Theme = "dark" | "light";
const THEME_STORAGE_KEY = "fingertip-ai-theme";

// Renders fenced ```lang code blocks with real syntax highlighting
// (matching the language tag Gemini is instructed to include — see
// FORMAT_INSTRUCTION in src-tauri/src/ai/gemini.rs) instead of flat
// monospace text. Inline `code` spans stay as plain <code>.
function MarkdownCodeBlock({
  className,
  children,
  theme,
}: {
  className?: string;
  children?: React.ReactNode;
  theme: Theme;
}) {
  const languageMatch = /language-(\w+)/.exec(className || "");
  if (!languageMatch) {
    return <code className={className}>{children}</code>;
  }
  return (
    <SyntaxHighlighter
      language={languageMatch[1]}
      style={theme === "light" ? oneLight : oneDark}
      PreTag="div"
      customStyle={{
        margin: 0,
        borderRadius: "10px",
        fontSize: "12px",
        background: theme === "light" ? "rgba(0,0,0,0.05)" : "rgba(0,0,0,0.28)",
      }}
      codeTagProps={{ style: { fontFamily: '"SF Mono", "Cascadia Code", Consolas, monospace' } }}
    >
      {String(children).replace(/\n$/, "")}
    </SyntaxHighlighter>
  );
}

interface AIPopupProps {
  visible: boolean;
  onClose: () => void;
  /** Text auto-captured from the foreground app's selection at the moment
   * the popup was triggered (see src-tauri/src/selection.rs). Null/undefined
   * if nothing was selected. */
  selectedText?: string | null;
}

const QUICK_ACTIONS: { label: string; template: (selection: string) => string }[] = [
  { label: "✨ Explain", template: (s) => `Explain this concisely:\n\n${s}` },
  { label: "✍️ Rewrite", template: (s) => `Rewrite this more clearly:\n\n${s}` },
  { label: "🌐 Translate", template: (s) => `Translate this to English:\n\n${s}` },
  { label: "🧠 Summarize", template: (s) => `Summarize this briefly:\n\n${s}` },
];

export default function AIPopup({ visible, onClose, selectedText }: AIPopupProps) {
  const [mounted, setMounted] = useState(visible);
  const [input, setInput] = useState("");
  const [pendingImage, setPendingImage] = useState<Attachment | null>(null);
  const [capturedSelection, setCapturedSelection] = useState<string | null>(null);
  const [capturing, setCapturing] = useState(false);
  const [captureHint, setCaptureHint] = useState<string | null>(null);
  const [theme, setTheme] = useState<Theme>(
    () => (localStorage.getItem(THEME_STORAGE_KEY) as Theme | null) || "dark"
  );
  const [isRecording, setIsRecording] = useState(false);
  const [isTranscribing, setIsTranscribing] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const audioChunksRef = useRef<Blob[]>([]);
  const { messages, send, reset } = useAIChat();

  useEffect(() => {
    localStorage.setItem(THEME_STORAGE_KEY, theme);
  }, [theme]);

  useEffect(() => {
    if (visible) {
      setMounted(true);
      const t = setTimeout(() => inputRef.current?.focus(), 160);

      // A captured selection no longer auto-sends — instead it's staged
      // (chip shown, quick actions emphasized) and the user picks what to
      // do with it (Explain / Rewrite / Translate / Summarize / or just
      // type a specific question). This avoids assuming "Explain" is
      // always what they wanted.
      if (selectedText && selectedText.trim()) {
        setCapturedSelection(selectedText);
      }

      return () => clearTimeout(t);
    } else {
      const t = setTimeout(() => {
        setMounted(false);
        reset();
        setInput("");
        setCapturedSelection(null);
        clearPendingImage();
        stopRecording(/* discard */ true);
      }, 150);
      return () => clearTimeout(t);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible, selectedText]);

  useEffect(() => {
    bodyRef.current?.scrollTo({ top: bodyRef.current.scrollHeight });
  }, [messages]);

  useEffect(() => {
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    function handleClickAway(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        onClose();
      }
    }
    document.addEventListener("keydown", handleKey);
    document.addEventListener("mousedown", handleClickAway);
    return () => {
      document.removeEventListener("keydown", handleKey);
      document.removeEventListener("mousedown", handleClickAway);
    };
  }, [onClose]);

  function clearPendingImage() {
    setPendingImage((prev) => {
      if (prev?.previewUrl) URL.revokeObjectURL(prev.previewUrl);
      return null;
    });
  }

  async function handleFileSelected(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    if (!file.type.startsWith("image/")) return;

    const base64 = await blobToBase64(file);
    clearPendingImage();
    setPendingImage({
      base64,
      mimeType: file.type,
      previewUrl: URL.createObjectURL(file),
      kind: "image",
    });
  }

  function blobToBase64(blob: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const result = reader.result as string;
        resolve(result.split(",")[1] ?? "");
      };
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(blob);
    });
  }

  // --- Voice input ---
  // Records real microphone audio (getUserMedia + MediaRecorder), then
  // sends the audio clip straight to Gemini as an inline attachment (the
  // same mechanism used for images) with an instruction to transcribe and
  // respond — Gemini handles the actual speech-to-text itself, so no
  // separate transcription service or API key is needed.
  async function toggleRecording() {
    if (isRecording) {
      stopRecording(false);
      return;
    }

    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const recorder = new MediaRecorder(stream);
      audioChunksRef.current = [];

      recorder.ondataavailable = (e) => {
        if (e.data.size > 0) audioChunksRef.current.push(e.data);
      };

      recorder.onstop = async () => {
        stream.getTracks().forEach((track) => track.stop());
        const blob = new Blob(audioChunksRef.current, { type: recorder.mimeType || "audio/webm" });
        audioChunksRef.current = [];

        if (blob.size < 500) return; // essentially empty — mic click with no real speech

        setIsTranscribing(true);
        try {
          const base64 = await blobToBase64(blob);
          await send("Transcribe this voice message, then answer it helpfully.", {
            base64,
            mimeType: blob.type.split(";")[0] || "audio/webm",
            kind: "audio",
          });
        } finally {
          setIsTranscribing(false);
        }
      };

      mediaRecorderRef.current = recorder;
      recorder.start();
      setIsRecording(true);
    } catch {
      setCaptureHint("Couldn't access the microphone — check Windows privacy settings for microphone access.");
    }
  }

  function stopRecording(discard: boolean) {
    const recorder = mediaRecorderRef.current;
    if (!recorder) return;
    if (discard) {
      recorder.onstop = () => {
        recorder.stream.getTracks().forEach((track) => track.stop());
      };
    }
    if (recorder.state !== "inactive") recorder.stop();
    mediaRecorderRef.current = null;
    setIsRecording(false);
  }

  // --- Screenshot capture (Phase 4) ---
  // Uses the Screen Capture API (getDisplayMedia) instead of a native
  // Win32 GDI capture — WebView2 supports it natively (it fires its own
  // ScreenCaptureStarting event) and it comes with Windows' own
  // screen/window picker built in, satisfying the spec requirement that
  // capture must be user-triggered and explicit: Windows itself asks the
  // user to choose exactly what to share before anything is captured.
  // Only a single frame is grabbed, then the stream is stopped
  // immediately — this is a one-shot snapshot, never continuous capture.
  const [isCapturingScreen, setIsCapturingScreen] = useState(false);

  async function captureScreenshot() {
    setIsCapturingScreen(true);
    setCaptureHint(null);
    let stream: MediaStream | null = null;
    try {
      stream = await navigator.mediaDevices.getDisplayMedia({
        video: { cursor: "never" } as MediaTrackConstraints,
        audio: false,
      });

      const track = stream.getVideoTracks()[0];
      const video = document.createElement("video");
      video.srcObject = stream;
      await video.play();
      // Give the first real frame a moment to actually paint before
      // grabbing it — capturing immediately on play() can grab a blank
      // frame on some GPUs/drivers.
      await new Promise((r) => setTimeout(r, 150));

      const canvas = document.createElement("canvas");
      canvas.width = video.videoWidth;
      canvas.height = video.videoHeight;
      canvas.getContext("2d")!.drawImage(video, 0, 0);

      track.stop(); // one frame only — stop the capture immediately
      stream.getTracks().forEach((t) => t.stop());
      stream = null;

      const dataUrl = canvas.toDataURL("image/png");
      const base64 = dataUrl.split(",")[1] ?? "";

      clearPendingImage();
      setPendingImage({
        base64,
        mimeType: "image/png",
        previewUrl: dataUrl,
        kind: "image",
      });
      inputRef.current?.focus();
    } catch (err) {
      // User cancelled the picker, or denied permission — not an error
      // worth alarming over, just quietly do nothing on cancel.
      const message = err instanceof Error ? err.message : String(err);
      if (!message.toLowerCase().includes("permission") && !message.toLowerCase().includes("cancel")) {
        setCaptureHint("Screenshot capture failed — try again.");
      }
    } finally {
      stream?.getTracks().forEach((t) => t.stop());
      setIsCapturingScreen(false);
    }
  }

  if (!mounted) return null;

  function handleSubmit() {
    if (!input.trim() && !pendingImage) return;
    const text = capturedSelection
      ? `Context (selected text):\n${capturedSelection}\n\nQuestion: ${input.trim() || "Explain this."}`
      : input.trim() || "Describe this image.";
    send(text, pendingImage);
    setInput("");
    clearPendingImage();
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
  }

  async function runQuickAction(template: (s: string) => string) {
    let base = capturedSelection || input.trim();

    if (!base) {
      setCapturing(true);
      setCaptureHint(null);
      try {
        const live = await invoke<string | null>("capture_selection_now");
        if (live && live.trim()) {
          base = live.trim();
          setCapturedSelection(base);
        }
      } finally {
        setCapturing(false);
      }
    }

    if (!base) {
      setCaptureHint("No text selected — select something, then try again.");
      inputRef.current?.focus();
      return;
    }

    send(template(base), pendingImage);
    setInput("");
    clearPendingImage();
  }

  const isStreaming = messages.some((m) => m.status === "streaming");
  // Quick actions get a visual nudge when there's a fresh, unactioned
  // selection and no conversation has started yet — this is the moment
  // the user is meant to pick Explain/Rewrite/Translate/Summarize.
  const awaitingActionChoice = Boolean(capturedSelection) && messages.length === 0;

  return (
    <div
      className={`ft-popup ${visible ? "ft-popup--in" : "ft-popup--out"} ${theme === "light" ? "ft-popup--light" : ""}`}
      ref={containerRef}
    >
      <div className="ft-popup__header" data-tauri-drag-region>
        <span className="ft-popup__title" data-tauri-drag-region>
          ✨ Fingertip AI
        </span>
        <div className="ft-popup__header-actions">
          <button
            className="ft-popup__theme-toggle"
            onClick={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
            aria-label="Toggle theme"
            title={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"}
          >
            {theme === "dark" ? "☀️" : "🌙"}
          </button>
          <button className="ft-popup__close" onClick={onClose} aria-label="Close">
            ×
          </button>
        </div>
      </div>

      {capturedSelection && (
        <div className="ft-popup__selection-chip">
          <span className="ft-popup__selection-chip-icon">📋</span>
          <span className="ft-popup__selection-chip-text">
            Selected text captured ({capturedSelection.length} chars)
          </span>
          <button onClick={() => setCapturedSelection(null)} aria-label="Stop using selected text">
            ×
          </button>
        </div>
      )}
      {awaitingActionChoice && (
        <p className="ft-popup__action-prompt">What would you like to do with it?</p>
      )}

      <div className="ft-popup__body" ref={bodyRef}>
        {messages.length === 0 ? (
          <p className="ft-popup__placeholder">
            {capturedSelection ? "Pick an action below, or ask a specific question." : "Ask anything..."}
          </p>
        ) : (
          messages.map((m) => (
            <div key={m.id} className={`ft-msg ft-msg--${m.role}`}>
              {m.imagePreviewUrl && m.attachmentKind === "image" && (
                <img className="ft-msg__image" src={m.imagePreviewUrl} alt="attached" />
              )}
              {m.attachmentKind === "audio" && (
                <span className="ft-msg__audio-badge">🎤 Voice message</span>
              )}
              {m.role === "assistant" && m.status === "streaming" && m.text === "" ? (
                <span className="ft-msg__loading">
                  <span className="ft-dot" />
                  <span className="ft-dot" />
                  <span className="ft-dot" />
                </span>
              ) : m.status === "error" ? (
                <span className="ft-msg__error">⚠️ {m.text}</span>
              ) : m.role === "assistant" ? (
                <div className="ft-msg__markdown">
                  <ReactMarkdown
                    components={{ code: (props) => <MarkdownCodeBlock {...props} theme={theme} /> }}
                  >
                    {m.text}
                  </ReactMarkdown>
                </div>
              ) : (
                <span>{m.text}</span>
              )}
            </div>
          ))
        )}
        {isTranscribing && (
          <div className="ft-msg ft-msg--user">
            <span className="ft-msg__audio-badge">🎤 Processing voice…</span>
          </div>
        )}
      </div>

      {pendingImage && (
        <div className="ft-popup__image-preview">
          <img src={pendingImage.previewUrl} alt="Selected" />
          <button onClick={clearPendingImage} aria-label="Remove image">
            ×
          </button>
        </div>
      )}

      {captureHint && <p className="ft-popup__capture-hint">{captureHint}</p>}

      <div className={`ft-popup__quick-actions ${awaitingActionChoice ? "ft-popup__quick-actions--emphasized" : ""}`}>
        {QUICK_ACTIONS.map((qa) => (
          <button
            key={qa.label}
            onClick={() => runQuickAction(qa.template)}
            disabled={isStreaming || capturing}
          >
            {capturing ? "…" : qa.label}
          </button>
        ))}
      </div>

      <div className="ft-popup__input-row">
        <input
          type="file"
          accept="image/*"
          ref={fileInputRef}
          onChange={handleFileSelected}
          style={{ display: "none" }}
        />
        <button
          className="ft-popup__attach"
          aria-label="Attach image"
          onClick={() => fileInputRef.current?.click()}
          disabled={isStreaming}
        >
          🖼️
        </button>
        <button
          className="ft-popup__attach"
          aria-label="Capture screen"
          title="Capture your screen and ask about it"
          onClick={captureScreenshot}
          disabled={isStreaming || isCapturingScreen}
        >
          {isCapturingScreen ? "…" : "📸"}
        </button>
        <input
          ref={inputRef}
          type="text"
          placeholder={capturedSelection ? "Ask about the selected text..." : "Ask anything..."}
          value={input}
          onChange={(e) => {
            setInput(e.target.value);
            if (captureHint) setCaptureHint(null);
          }}
          onKeyDown={handleKeyDown}
          disabled={isStreaming}
        />
        <button
          className={`ft-popup__mic ${isRecording ? "ft-popup__mic--recording" : ""}`}
          aria-label={isRecording ? "Stop recording" : "Voice input"}
          title={isRecording ? "Click to stop and send" : "Click to speak your question"}
          onClick={toggleRecording}
          disabled={isStreaming || isTranscribing}
        >
          {isRecording ? "⏹️" : "🎤"}
        </button>
      </div>

      <div
        className="ft-popup__resize-handle"
        onMouseDown={() => {
          getCurrentWindow()
            .startResizeDragging("SouthEast")
            .catch(() => {
              /* best-effort — older WebView2/Windows combos can reject this */
            });
        }}
        aria-hidden="true"
      />
    </div>
  );
}
