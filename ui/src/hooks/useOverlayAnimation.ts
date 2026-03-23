import { useState, useEffect, useRef, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";

interface PanelAnimatePayload {
  action: "show" | "hide";
  delay_ms: number;
}

/**
 * Hook that listens for panel-animate events from the Rust backend
 * and returns the CSS class to apply for open/close animations.
 *
 * Returns a className string: "panel-root", "panel-root panel-visible",
 * or "panel-root panel-hiding".
 */
export function useOverlayAnimation(): string {
  const [animClass, setAnimClass] = useState("panel-root panel-visible");
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const cleanup = useCallback(() => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  useEffect(() => {
    let cancelled = false;

    const unlistenPromise = listen<PanelAnimatePayload>("panel-animate", (event) => {
      if (cancelled) return;

      const { action, delay_ms } = event.payload;
      cleanup();

      if (action === "show") {
        // Staggered show: apply visible class after delay
        timerRef.current = setTimeout(() => {
          if (!cancelled) setAnimClass("panel-root panel-visible");
        }, delay_ms);
      } else if (action === "hide") {
        // Simultaneous hide: apply hiding class immediately
        setAnimClass("panel-root panel-hiding");
      }
    });

    return () => {
      cancelled = true;
      cleanup();
      unlistenPromise.then((fn) => fn());
    };
  }, [cleanup]);

  return animClass;
}
