import { Toast } from "./Toast";
import type { ToastData } from "../../types";

const MAX_VISIBLE = 5;

interface ToastStackProps {
  toasts: ToastData[];
  onDismiss: (id: string) => void;
}

export function ToastStack({ toasts, onDismiss }: ToastStackProps) {
  const visibleToasts = toasts.slice(0, MAX_VISIBLE);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: 8,
        padding: 8,
        pointerEvents: "none",
      }}
    >
      {visibleToasts.map((toast) => (
        <div key={toast.id} style={{ pointerEvents: "auto" }}>
          <Toast toast={toast} onDismiss={onDismiss} />
        </div>
      ))}
    </div>
  );
}
