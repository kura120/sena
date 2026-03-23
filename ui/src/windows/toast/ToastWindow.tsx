import { useState, useCallback, useEffect } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { ToastStack } from "../../components/Toast/ToastStack";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import type { ToastData } from "../../types";

const MAX_VISIBLE = 5;
const TOAST_CARD_HEIGHT = 80;
const TOAST_GAP = 8;
const CONTAINER_PADDING = 8;
const WINDOW_WIDTH = 340;

function calculateWindowHeight(count: number): number {
  if (count === 0) return 1;
  const visible = Math.min(count, MAX_VISIBLE);
  return CONTAINER_PADDING * 2 + visible * TOAST_CARD_HEIGHT + (visible - 1) * TOAST_GAP;
}

export function ToastWindow() {
  const [toasts, setToasts] = useState<ToastData[]>([]);

  useTauriEvent<ToastData>("toast", (payload) => {
    setToasts((prev) => {
      const next = [payload, ...prev];
      if (next.length > MAX_VISIBLE) {
        return next.slice(0, MAX_VISIBLE);
      }
      return next;
    });
  });

  const handleDismiss = useCallback((id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  useEffect(() => {
    const win = getCurrentWindow();
    const height = calculateWindowHeight(toasts.length);
    const hasToasts = toasts.length > 0;

    win.setSize(new LogicalSize(WINDOW_WIDTH, height)).catch((e: unknown) => {
      console.error("Failed to resize toast window:", e);
    });
    // When empty, make window click-through so it never intercepts input.
    // When toasts are present, disable click-through so they are dismissible.
    win.setIgnoreCursorEvents(!hasToasts).catch((e: unknown) => {
      console.error("Failed to toggle toast click-through:", e);
    });
  }, [toasts.length]);

  return <ToastStack toasts={toasts} onDismiss={handleDismiss} />;
}
