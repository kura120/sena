import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TitleBar } from "../../components/TitleBar/TitleBar";
import { IconThoughtStream } from "../../components/icons";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { STRINGS } from "../../constants/strings";
import type { DebugSnapshot, ThoughtSnapshot } from "../../types";
import { formatRelativeTime } from "../../utils/time";
import { useWindowDragSave } from "../../hooks/useWindowDragSave";
import { useOverlayAnimation } from "../../hooks/useOverlayAnimation";

export function ThoughtStream() {
  const [thoughts, setThoughts] = useState<ThoughtSnapshot[]>([]);
  const [pinned, setPinned] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [, setTick] = useState(0);

  useWindowDragSave();
  const panelClass = useOverlayAnimation();

  const fetchState = useCallback(() => {
    invoke<DebugSnapshot>("get_debug_snapshot").then((snapshot) => {
      setThoughts(snapshot.thoughts);
    }).catch(() => {});
  }, []);

  useEffect(() => {
    fetchState();
    const interval = setInterval(fetchState, 2000);
    return () => clearInterval(interval);
  }, [fetchState]);

  useTauriEvent("subsystems-reset", () => {
    setThoughts([]);
  });

  useEffect(() => {
    const interval = setInterval(() => setTick(t => t + 1), 1000);
    return () => clearInterval(interval);
  }, []);

  function scoreColor(score: number): string {
    if (score >= 0.8) return "var(--status-ready)";
    if (score >= 0.5) return "var(--status-degraded)";
    return "var(--text-muted)";
  }

  return (
    <div 
      className={`flex flex-col ${collapsed ? '' : 'h-screen'} overflow-hidden text-sm panel-glass ${panelClass}`}
      style={{ background: "var(--bg-panel)", border: "1px solid var(--border)", borderRadius: "var(--radius)" }}
    >
      <TitleBar
        icon={<IconThoughtStream size={14} />}
        title={STRINGS.PANEL_THOUGHT_STREAM}
        pinned={pinned}
        onPinToggle={() => setPinned(!pinned)}
        collapsed={collapsed}
        onCollapseToggle={() => setCollapsed(c => !c)}
      />
      {!collapsed && (
        <div className="flex-1 overflow-y-auto py-1">
          {thoughts.map((t, i) => (
            <div 
              key={`${t.timestamp}-${i}`}
              className="mx-2 my-1 px-3 py-2 rounded transition-colors"
              style={{ background: "transparent" }}
              onMouseEnter={(e) => e.currentTarget.style.background = "var(--bg-hover)"}
              onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}
            >
              <div className="flex items-center justify-between mb-1">
                <div className="flex items-center gap-2">
                  <div style={{
                    width: 8, height: 8, borderRadius: "50%",
                    background: scoreColor(t.relevance_score),
                  }} />
                  <span style={{ color: "var(--text-muted)", fontSize: 11, fontFamily: "monospace" }}>
                    {t.relevance_score.toFixed(2)}
                  </span>
                </div>
                <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
                  {formatRelativeTime(t.timestamp)}
                </span>
              </div>
              <div style={{ color: "var(--text-primary)", fontSize: 13, lineHeight: "1.4" }}>
                {t.content}
              </div>
            </div>
          ))}
          {thoughts.length === 0 && (
            <div className="p-4 text-center text-xs" style={{ color: "var(--text-muted)" }}>
              No thoughts surfaced yet — CTP is processing...
            </div>
          )}
        </div>
      )}
    </div>
  );
}
