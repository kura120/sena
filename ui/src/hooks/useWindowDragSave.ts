import { useEffect, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";

/**
 * Save window position to persistent store after drag ends.
 * Debounces to avoid excessive writes during drag movement.
 */
export function useWindowDragSave(): void {
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const currentWindow = getCurrentWindow();
    let cancelled = false;

    const unlisten = currentWindow.onMoved(() => {
      if (cancelled) return;

      // Debounce: only save after movement settles
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(async () => {
        if (cancelled) return;
        try {
          const pos = await currentWindow.outerPosition();
          const size = await currentWindow.innerSize();
          await invoke("save_window_position", {
            label: currentWindow.label,
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
          });
        } catch {
          // Position save is non-critical — silently ignore errors
        }
      }, 300);
    });

    return () => {
      cancelled = true;
      if (timerRef.current) clearTimeout(timerRef.current);
      unlisten.then((fn) => fn());
    };
  }, []);
}
