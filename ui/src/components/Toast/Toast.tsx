import { useEffect, useState } from "react";

interface ToastProps {
  hotkeyLabel: string;
  onDismiss: () => void;
}

export function Toast({ hotkeyLabel, onDismiss }: ToastProps) {
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    // Fade in
    requestAnimationFrame(() => setVisible(true));
    // Auto-dismiss after 4 seconds
    const timer = setTimeout(() => {
      setVisible(false);
      setTimeout(onDismiss, 300); // Wait for fade-out animation
    }, 4000);
    return () => clearTimeout(timer);
  }, [onDismiss]);

  return (
    <div
      className="flex items-center gap-2 px-4 h-full text-xs transition-opacity duration-300"
      style={{
        opacity: visible ? 1 : 0,
        background: "var(--bg-panel)",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius)",
        color: "var(--text-secondary)",
      }}
    >
      <span>Press</span>
      <kbd
        className="px-1.5 py-0.5 rounded text-[11px] font-mono font-semibold"
        style={{
          background: "var(--bg-hover)",
          border: "1px solid var(--border)",
          color: "var(--text-primary)",
        }}
      >
        {hotkeyLabel}
      </kbd>
      <span>to open Sena debug overlay</span>
    </div>
  );
}
