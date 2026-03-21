import { getCurrentWindow } from "@tauri-apps/api/window";
import { IconClose, IconPin, IconPinActive } from "../icons";

interface TitleBarProps {
  icon: React.ReactNode;
  title: string;
  pinned: boolean;
  onPinToggle: () => void;
}

export function TitleBar({ icon, title, pinned, onPinToggle }: TitleBarProps) {
  const handleClose = () => {
    getCurrentWindow().hide();
  };

  return (
    <div
      data-tauri-drag-region
      className="flex items-center h-[34px] px-2 select-none shrink-0"
      style={{ background: "var(--bg-titlebar)", borderBottom: "1px solid var(--border)" }}
    >
      <div className="flex items-center gap-1.5 pointer-events-none" style={{ color: "var(--text-secondary)" }}>
        {icon}
        <span className="text-xs font-medium" style={{ color: "var(--text-primary)" }}>{title}</span>
      </div>
      <div className="ml-auto flex items-center gap-0.5">
        <button
          onClick={onPinToggle}
          className="p-1 rounded transition-colors"
          style={{ color: pinned ? "var(--text-primary)" : "var(--text-muted)" }}
          onMouseEnter={(e) => e.currentTarget.style.background = "var(--bg-hover)"}
          onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}
        >
          {pinned ? <IconPinActive size={14} /> : <IconPin size={14} />}
        </button>
        <button
          onClick={handleClose}
          className="p-1 rounded transition-colors"
          style={{ color: "var(--text-muted)" }}
          onMouseEnter={(e) => e.currentTarget.style.background = "var(--bg-hover)"}
          onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}
        >
          <IconClose size={14} />
        </button>
      </div>
    </div>
  );
}
