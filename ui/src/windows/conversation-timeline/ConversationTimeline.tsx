import { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TitleBar } from "../../components/TitleBar/TitleBar";
import { IconConversationTimeline } from "../../components/icons";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { STRINGS } from "../../constants/strings";
import type { DebugSnapshot, ConversationTurnSnapshot } from "../../types";
import { formatRelativeTime } from "../../utils/time";
import { useWindowDragSave } from "../../hooks/useWindowDragSave";
import { useOverlayAnimation } from "../../hooks/useOverlayAnimation";

export function ConversationTimeline() {
  const [turns, setTurns] = useState<ConversationTurnSnapshot[]>([]);
  const [pinned, setPinned] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [autoScroll, setAutoScroll] = useState(true);
  const [, setTick] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  useWindowDragSave();
  const panelClass = useOverlayAnimation();

  const fetchState = useCallback(() => {
    invoke<DebugSnapshot>("get_debug_snapshot").then((snapshot) => {
      setTurns(snapshot.conversation_turns);
    }).catch(() => {});
  }, []);

  useEffect(() => {
    fetchState();
    const interval = setInterval(fetchState, 2000);
    return () => clearInterval(interval);
  }, [fetchState]);

  useTauriEvent("subsystems-reset", () => {
    setTurns([]);
  });

  useEffect(() => {
    const interval = setInterval(() => setTick(t => t + 1), 1000);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    if (autoScroll && listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [turns, autoScroll]);

  return (
    <div 
      className={`flex flex-col ${collapsed ? '' : 'h-screen'} overflow-hidden text-sm panel-glass ${panelClass}`}
      style={{ background: "var(--bg-panel)", border: "1px solid var(--border)", borderRadius: "var(--radius)" }}
    >
      <TitleBar
        icon={<IconConversationTimeline size={14} />}
        title={STRINGS.PANEL_CONVERSATION_TIMELINE}
        pinned={pinned}
        onPinToggle={() => setPinned(!pinned)}
        collapsed={collapsed}
        onCollapseToggle={() => setCollapsed(c => !c)}
      />
      {!collapsed && (
        <div
          ref={listRef}
          className="flex-1 overflow-y-auto py-1 scroll-smooth"
          onMouseEnter={() => setAutoScroll(false)}
          onMouseLeave={() => setAutoScroll(true)}
        >
          {turns.map((turn, i) => {
            const isUser = turn.role === "user";
            return (
              <div key={`${turn.timestamp}-${i}`} className="mx-2 my-1 px-3 py-2 border-l-[3px]"
                style={{ borderColor: isUser ? "var(--event-user)" : "var(--event-boot)" }}
              >
                <div className="flex items-center justify-between mb-1">
                  <span style={{ color: isUser ? "var(--event-user)" : "var(--event-boot)", fontSize: 11, fontWeight: 600, textTransform: "uppercase" }}>
                    {turn.role}
                  </span>
                  <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
                    {formatRelativeTime(turn.timestamp)}
                  </span>
                </div>
                <div style={{ color: "var(--text-primary)", fontSize: 13, lineHeight: "1.4", marginBottom: 4 }}>
                  {turn.content_preview}
                </div>
                {!isUser && (
                  <div className="flex gap-3 flex-wrap" style={{ fontSize: 11, color: "var(--text-muted)" }}>
                    {turn.model_id && <span>model: {turn.model_id}</span>}
                    {turn.latency_ms > 0 && <span>{turn.latency_ms}ms</span>}
                    {turn.tokens_generated > 0 && (
                      <span>{turn.tokens_prompt}→{turn.tokens_generated} tok</span>
                    )}
                  </div>
                )}
              </div>
            );
          })}
          {turns.length === 0 && (
            <div className="p-4 text-center text-xs" style={{ color: "var(--text-muted)" }}>
              No conversation turns yet — start chatting to see the timeline...
            </div>
          )}
        </div>
      )}
    </div>
  );
}
