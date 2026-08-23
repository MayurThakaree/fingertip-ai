// Payload emitted by the Rust mouse hook when a long-press is detected.
export interface ShowPopupPayload {
  x: number;
  y: number;
  monitorWidth: number;
  monitorHeight: number;
  scaleFactor: number;
  triggeredBy: "mouse" | "shortcut";
  // Text auto-captured from whatever was selected in the foreground app
  // at trigger time (simulated Ctrl+C, then the clipboard is restored —
  // see src-tauri/src/selection.rs). Absent if nothing was selected.
  selectedText?: string | null;
}

export interface PopupPosition {
  left: number;
  top: number;
}
