import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { IconClose, IconPin, IconPinActive, IconChevronRight, IconChevronDown } from "../icons";

interface TitleBarProps {
  icon: React.ReactNode;
  title: string;
  pinned: boolean;
  onPinToggle: () => void;
  collapsed: boolean;
  onCollapseToggle: () => void;
  extraActions?: React.ReactNode;
}

export function TitleBar({ icon, title, pinned, onPinToggle, collapsed, onCollapseToggle, extraActions }: TitleBarProps) {
  const handleClose = () => {
    const windowLabel = getCurrentWindow().label;
    invoke("hide_panel", { label: windowLabel }).catch(() => {
      // Fallback: hide directly if command fails (e.g. window not in panel registry)
      getCurrentWindow().hide();
    });
  };

  const handleDragStart = (e: React.MouseEvent) => {
    // Only drag from the title bar background, not from buttons
    if ((e.target as HTMLElement).closest("button")) return;
    getCurrentWindow().startDragging().catch(() => {
      // Fallback: rely on data-tauri-drag-region attribute
    });
  };

  return (
    <div
      data-tauri-drag-region
      onMouseDown={handleDragStart}
      className="flex items-center h-[34px] px-2 select-none shrink-0 cursor-grab active:cursor-grabbing"
      style={{ background: "var(--bg-titlebar)", borderBottom: collapsed ? "none" : "1px solid var(--border)" }}
    >
      <button
        onClick={(e) => { e.stopPropagation(); onCollapseToggle(); }}
        className="p-0.5 rounded transition-colors mr-1"
        style={{ color: "var(--text-muted)" }}
        onMouseEnter={(e) => e.currentTarget.style.background = "var(--bg-hover)"}
        onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}
      >
        {collapsed ? <IconChevronRight size={12} /> : <IconChevronDown size={12} />}
      </button>
      <div className="flex items-center gap-1.5 pointer-events-none" style={{ color: "var(--text-secondary)" }}>
        {icon}
        <span className="text-xs font-medium" style={{ color: "var(--text-primary)" }}>{title}</span>
      </div>
      <div className="ml-auto flex items-center gap-0.5">
        {!collapsed && extraActions}
        <button
          onClick={(e) => { e.stopPropagation(); onPinToggle(); }}
          className="p-1 rounded transition-colors"
          style={{ color: pinned ? "var(--text-primary)" : "var(--text-muted)" }}
          onMouseEnter={(e) => e.currentTarget.style.background = "var(--bg-hover)"}
          onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}
        >
          {pinned ? <IconPinActive size={14} /> : <IconPin size={14} />}
        </button>
        <button
          onClick={(e) => { e.stopPropagation(); handleClose(); }}
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

