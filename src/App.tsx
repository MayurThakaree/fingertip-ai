import AIPopup from "./components/AIPopup/AIPopup";
import { useFingertipEvents } from "./hooks/useFingertipEvents";

/**
 * This whole App is rendered inside the dedicated, borderless, always-on-top
 * "popup" WebviewWindow (see src-tauri/src/main.rs). There is no separate
 * "main window" UI for Phase 1 — the tray + settings window come in Phase 5.
 */
export default function App() {
  const { visible, close, lastPayload } = useFingertipEvents();
  return (
    <AIPopup visible={visible} onClose={close} selectedText={lastPayload?.selectedText ?? null} />
  );
}
