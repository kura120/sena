import { useEffect, useRef } from "react";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

/**
 * Subscribe to a Tauri backend event. The callback is stable across re-renders
 * if wrapped with useCallback. Automatically unsubscribes on unmount.
 */
export function useTauriEvent<T>(eventName: string, handler: (payload: T) => void): void {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;

    listen<T>(eventName, (event) => {
      if (!cancelled) {
        handlerRef.current(event.payload);
      }
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlisten = fn;
      }
    });

    return () => {
      cancelled = true;
      if (unlisten) {
        unlisten();
      }
    };
  }, [eventName]);
}
