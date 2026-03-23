import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TitleBar } from "../../components/TitleBar/TitleBar";
import { IconMemoryStats } from "../../components/icons";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { STRINGS } from "../../constants/strings";
import type { DebugSnapshot } from "../../types";
import { formatRelativeTime } from "../../utils/time";
import { useWindowDragSave } from "../../hooks/useWindowDragSave";
import { useOverlayAnimation } from "../../hooks/useOverlayAnimation";

const TIER_COLORS: Record<string, string> = {
  short_term: "var(--event-user)",
  long_term: "var(--event-boot)",
  episodic: "var(--event-memory)",
};

export function MemoryStats() {
  const [shortTerm, setShortTerm] = useState(0);
  const [longTerm, setLongTerm] = useState(0);
  const [episodic, setEpisodic] = useState(0);
  const [lastWrite, setLastWrite] = useState<string | null>(null);
  const [shortTermLastWrite, setShortTermLastWrite] = useState<string | null>(null);
  const [longTermLastWrite, setLongTermLastWrite] = useState<string | null>(null);
  const [episodicLastWrite, setEpisodicLastWrite] = useState<string | null>(null);
  const [pinned, setPinned] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [, setTick] = useState(0);

  useWindowDragSave();
  const panelClass = useOverlayAnimation();

  const fetchState = useCallback(() => {
    invoke<DebugSnapshot>("get_debug_snapshot").then((snapshot) => {
      setShortTerm(snapshot.memory_stats.short_term_count);
      setLongTerm(snapshot.memory_stats.long_term_count);
      setEpisodic(snapshot.memory_stats.episodic_count);
      setLastWrite(snapshot.memory_stats.last_write);
      setShortTermLastWrite(snapshot.memory_stats.short_term_last_write);
      setLongTermLastWrite(snapshot.memory_stats.long_term_last_write);
      setEpisodicLastWrite(snapshot.memory_stats.episodic_last_write);
    }).catch(() => {});
  }, []);

  useEffect(() => {
    fetchState();
    const interval = setInterval(fetchState, 2000);
    return () => clearInterval(interval);
  }, [fetchState]);

  useTauriEvent("subsystems-reset", () => {
    setShortTerm(0);
    setLongTerm(0);
    setEpisodic(0);
    setLastWrite(null);
    setShortTermLastWrite(null);
    setLongTermLastWrite(null);
    setEpisodicLastWrite(null);
  });

  useEffect(() => {
    const interval = setInterval(() => setTick(t => t + 1), 1000);
    return () => clearInterval(interval);
  }, []);

  const total = shortTerm + longTerm + episodic;
  const tiers = [
    { label: "Short-term", count: shortTerm, key: "short_term", lastWrite: shortTermLastWrite },
    { label: "Long-term", count: longTerm, key: "long_term", lastWrite: longTermLastWrite },
    { label: "Episodic", count: episodic, key: "episodic", lastWrite: episodicLastWrite },
  ];

  return (
    <div 
      className={`flex flex-col ${collapsed ? '' : 'h-screen'} overflow-hidden text-sm panel-glass ${panelClass}`}
      style={{ background: "var(--bg-panel)", border: "1px solid var(--border)", borderRadius: "var(--radius)" }}
    >
      <TitleBar
        icon={<IconMemoryStats size={14} />}
        title={STRINGS.PANEL_MEMORY_STATS}
        pinned={pinned}
        onPinToggle={() => setPinned(!pinned)}
        collapsed={collapsed}
        onCollapseToggle={() => setCollapsed(c => !c)}
      />
      {!collapsed && (
        <div className="flex-1 overflow-y-auto p-3 space-y-3">
          {/* Total counter */}
          <div className="text-center" style={{ borderBottom: "1px solid var(--border)", paddingBottom: 8 }}>
            <div style={{ fontSize: 24, fontWeight: 700, fontFamily: "monospace", color: "var(--text-primary)" }}>
              {total}
            </div>
            <div style={{ fontSize: 11, color: "var(--text-muted)" }}>Total Memories</div>
          </div>

          {/* Tier bars */}
          {tiers.map(tier => (
            <div key={tier.key}>
              <div className="flex justify-between mb-1">
                <span style={{ color: "var(--text-secondary)", fontSize: 12 }}>{tier.label}</span>
                <span style={{ color: "var(--text-primary)", fontSize: 12, fontFamily: "monospace", fontWeight: 600 }}>
                  {tier.count}
                </span>
              </div>
              <div style={{ height: 4, background: "rgba(255,255,255,0.08)", borderRadius: 2, overflow: "hidden" }}>
                <div style={{
                  height: "100%",
                  width: total > 0 ? `${(tier.count / total) * 100}%` : "0%",
                  background: TIER_COLORS[tier.key],
                  borderRadius: 2,
                  transition: "width 300ms ease",
                  minWidth: tier.count > 0 ? 4 : 0,
                }} />
              </div>
              {tier.lastWrite && (
                <div className="text-right mt-0.5">
                  <span style={{ color: "var(--text-muted)", fontSize: 10 }}>
                    {formatRelativeTime(tier.lastWrite)}
                  </span>
                </div>
              )}
            </div>
          ))}

          {/* Last write */}
          {lastWrite && (
            <div className="flex justify-between pt-2" style={{ borderTop: "1px solid var(--border)" }}>
              <span style={{ color: "var(--text-secondary)", fontSize: 11 }}>Last Write</span>
              <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
                {formatRelativeTime(lastWrite)}
              </span>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
