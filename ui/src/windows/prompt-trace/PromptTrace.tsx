import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TitleBar } from "../../components/TitleBar/TitleBar";
import { IconPromptTrace } from "../../components/icons";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { STRINGS } from "../../constants/strings";
import type { DebugSnapshot, PromptTraceSnapshot } from "../../types";
import { formatRelativeTime } from "../../utils/time";
import { useWindowDragSave } from "../../hooks/useWindowDragSave";
import { useOverlayAnimation } from "../../hooks/useOverlayAnimation";

export function PromptTrace() {
  const [traces, setTraces] = useState<PromptTraceSnapshot[]>([]);
  const [expandedIndex, setExpandedIndex] = useState<number | null>(null);
  const [pinned, setPinned] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [, setTick] = useState(0);

  useWindowDragSave();
  const panelClass = useOverlayAnimation();

  const fetchState = useCallback(() => {
    invoke<DebugSnapshot>("get_debug_snapshot").then((snapshot) => {
      setTraces(snapshot.prompt_traces);
    }).catch(() => {});
  }, []);

  useEffect(() => {
    fetchState();
    const interval = setInterval(fetchState, 2000);
    return () => clearInterval(interval);
  }, [fetchState]);

  useTauriEvent("subsystems-reset", () => {
    setTraces([]);
    setExpandedIndex(null);
  });

  useEffect(() => {
    const interval = setInterval(() => setTick(t => t + 1), 1000);
    return () => clearInterval(interval);
  }, []);

  return (
    <div 
      className={`flex flex-col ${collapsed ? '' : 'h-screen'} overflow-hidden text-sm panel-glass ${panelClass}`}
      style={{ background: "var(--bg-panel)", border: "1px solid var(--border)", borderRadius: "var(--radius)" }}
    >
      <TitleBar
        icon={<IconPromptTrace size={14} />}
        title={STRINGS.PANEL_PROMPT_TRACE}
        pinned={pinned}
        onPinToggle={() => setPinned(!pinned)}
        collapsed={collapsed}
        onCollapseToggle={() => setCollapsed(c => !c)}
      />
      {!collapsed && (
        <div className="flex-1 overflow-y-auto py-1">
          {traces.map((trace, i) => {
            const isExpanded = expandedIndex === i;
            const usagePercent = trace.token_budget > 0
              ? (trace.token_count / trace.token_budget) * 100
              : 0;
            return (
              <div key={`${trace.timestamp}-${i}`} className="mx-2 my-1">
                <div
                  className="px-3 py-2 rounded cursor-pointer transition-colors"
                  style={{ background: isExpanded ? "var(--bg-hover)" : "transparent" }}
                  onClick={() => setExpandedIndex(isExpanded ? null : i)}
                  onMouseEnter={(e) => { if (!isExpanded) e.currentTarget.style.background = "var(--bg-hover)"; }}
                  onMouseLeave={(e) => { if (!isExpanded) e.currentTarget.style.background = "transparent"; }}
                >
                  {/* Header row */}
                  <div className="flex items-center justify-between mb-1">
                    <span style={{ color: "var(--text-primary)", fontSize: 13, fontWeight: 600 }}>
                      {trace.sections.length} sections
                    </span>
                    <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
                      {formatRelativeTime(trace.timestamp)}
                    </span>
                  </div>
                  {/* Context window usage bar */}
                  <div className="flex items-center gap-2">
                    <div style={{ flex: 1, height: 4, background: "rgba(255,255,255,0.08)", borderRadius: 2, overflow: "hidden" }}>
                      <div style={{
                        height: "100%",
                        width: `${usagePercent}%`,
                        background: usagePercent > 90 ? "var(--status-unavailable)" : usagePercent > 70 ? "var(--status-degraded)" : "var(--event-boot)",
                        borderRadius: 2,
                        transition: "width 300ms ease",
                      }} />
                    </div>
                    <span style={{ color: "var(--text-secondary)", fontSize: 11, fontFamily: "monospace", whiteSpace: "nowrap" }}>
                      {trace.token_count}/{trace.token_budget}
                    </span>
                  </div>
                </div>

                {isExpanded && (
                  <div className="px-3 pb-2 space-y-2">
                    {/* Sections */}
                    <div>
                      <div style={{ color: "var(--text-muted)", fontSize: 11, marginBottom: 4 }}>Sections</div>
                      {trace.sections.map((s, si) => (
                        <div key={si} className="flex items-center gap-2 py-0.5">
                          <div style={{ width: 4, height: 4, borderRadius: "50%", background: "var(--event-boot)", flexShrink: 0 }} />
                          <span style={{ color: "var(--text-secondary)", fontSize: 12 }}>{s}</span>
                        </div>
                      ))}
                    </div>
                    {/* TOON preview */}
                    {trace.toon_output_preview && (
                      <div>
                        <div style={{ color: "var(--text-muted)", fontSize: 11, marginBottom: 4 }}>TOON Output</div>
                        <pre style={{
                          color: "var(--text-secondary)", fontSize: 11, fontFamily: "monospace",
                          background: "rgba(0,0,0,0.15)", padding: 8, borderRadius: 4,
                          whiteSpace: "pre-wrap", wordBreak: "break-all", maxHeight: 120, overflow: "auto",
                        }}>
                          {trace.toon_output_preview}
                        </pre>
                      </div>
                    )}
                  </div>
                )}
              </div>
            );
          })}
          {traces.length === 0 && (
            <div className="p-4 text-center text-xs" style={{ color: "var(--text-muted)" }}>
              No prompt traces yet — waiting for prompt-composer events...
            </div>
          )}
        </div>
      )}
    </div>
  );
}
