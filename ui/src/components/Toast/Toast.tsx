import { useEffect, useState, useCallback } from "react";
import { TOAST_TYPE_COLORS } from "../../constants/colors";
import type { ToastData } from "../../types";
import "./Toast.css";

interface ToastProps {
  toast: ToastData;
  onDismiss: (id: string) => void;
}

export function Toast({ toast, onDismiss }: ToastProps) {
  const [exiting, setExiting] = useState(false);
  const accentColor = TOAST_TYPE_COLORS[toast.toast_type] || TOAST_TYPE_COLORS.info;

  const handleDismiss = useCallback(() => {
    setExiting(true);
    setTimeout(() => onDismiss(toast.id), 180);
  }, [onDismiss, toast.id]);

  useEffect(() => {
    const timer = setTimeout(() => {
      handleDismiss();
    }, toast.dismiss_ms);
    return () => clearTimeout(timer);
  }, [toast.dismiss_ms, handleDismiss]);

  return (
    <div
      className={`toast-card panel-glass ${exiting ? "toast-hiding" : "toast-visible"}`}
      onClick={handleDismiss}
    >
      {/* Left accent bar */}
      <div className="toast-accent" style={{ background: accentColor }} />
      {/* Content */}
      <div className="toast-content">
        <div className="toast-title">{toast.title}</div>
        <div className="toast-message">{toast.message}</div>
      </div>
      {/* Progress bar */}
      <div className="toast-progress-track">
        <div
          className="toast-progress-fill"
          style={{
            background: accentColor,
            animationDuration: `${toast.dismiss_ms}ms`,
          }}
        />
      </div>
    </div>
  );
}
