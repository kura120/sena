import { useEffect, useState, useCallback } from "react";
import { TOAST_TYPE_COLORS } from "../../constants/colors";
import type { ToastData } from "../../types";

interface ToastProps {
  toast: ToastData;
  onDismiss: (id: string) => void;
}

export function Toast({ toast, onDismiss }: ToastProps) {
  const [exiting, setExiting] = useState(false);
  const accentColor = TOAST_TYPE_COLORS[toast.toast_type] || TOAST_TYPE_COLORS.info;

  const handleDismiss = useCallback(() => {
    setExiting(true);
    setTimeout(() => onDismiss(toast.id), 150);
  }, [onDismiss, toast.id]);

  useEffect(() => {
    const timer = setTimeout(() => {
      handleDismiss();
    }, toast.dismiss_ms);
    return () => clearTimeout(timer);
  }, [toast.dismiss_ms, handleDismiss]);

  return (
    <div
      className={exiting ? "toast-exiting" : "toast-entering"}
      onClick={handleDismiss}
      style={{
        width: 320,
        minHeight: 72,
        background: "var(--bg-panel)",
        backdropFilter: "blur(12px)",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius)",
        display: "flex",
        overflow: "hidden",
        cursor: "pointer",
        position: "relative",
      }}
    >
      {/* Left accent bar */}
      <div
        style={{
          width: 4,
          background: accentColor,
          flexShrink: 0,
        }}
      />
      {/* Content */}
      <div style={{ flex: 1, padding: "10px 12px", minWidth: 0 }}>
        <div
          style={{
            fontSize: 13,
            fontWeight: 600,
            color: "var(--text-primary)",
            marginBottom: 2,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {toast.title}
        </div>
        <div
          style={{
            fontSize: 12,
            color: "var(--text-secondary)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            display: "-webkit-box",
            WebkitLineClamp: 2,
            WebkitBoxOrient: "vertical" as const,
          }}
        >
          {toast.message}
        </div>
      </div>
      {/* Progress bar */}
      <div
        style={{
          position: "absolute",
          bottom: 0,
          left: 4,
          right: 0,
          height: 3,
          background: "transparent",
          overflow: "hidden",
          borderRadius: "0 0 var(--radius) 0",
        }}
      >
        <div
          style={{
            height: "100%",
            background: accentColor,
            opacity: 0.4,
            animation: `toast-progress ${toast.dismiss_ms}ms linear forwards`,
          }}
        />
      </div>
    </div>
  );
}
