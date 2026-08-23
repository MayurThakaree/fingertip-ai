import { useEffect, useState, useCallback } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type { ShowPopupPayload } from "../types";

/**
 * Subscribes to the "show-popup" event emitted by the Rust global mouse
 * hook (src-tauri/src/mouse_hook.rs) and the "hide-popup" event fired on
 * Escape / click-away / explicit close.
 *
 * Positioning math (screen-edge clamping) is intentionally done in Rust
 * (position_popup_window) because it has the real monitor work-area and
 * DPI info from Win32. This hook just reacts to visibility state.
 */
export function useFingertipEvents() {
  const [visible, setVisible] = useState(false);
  const [lastPayload, setLastPayload] = useState<ShowPopupPayload | null>(null);

  useEffect(() => {
    let unlistenShow: UnlistenFn;
    let unlistenHide: UnlistenFn;

    (async () => {
      unlistenShow = await listen<ShowPopupPayload>("show-popup", (event) => {
        setLastPayload(event.payload);
        setVisible(true);
      });
      unlistenHide = await listen("hide-popup", () => {
        setVisible(false);
      });
    })();

    return () => {
      unlistenShow?.();
      unlistenHide?.();
    };
  }, []);

  const close = useCallback(async () => {
    setVisible(false);
    // Tell Rust to actually hide (not destroy) the popup webview window.
    await invoke("hide_popup_window");
  }, []);

  return { visible, lastPayload, close };
}
