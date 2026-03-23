import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TitleBar } from "../../components/TitleBar/TitleBar";
import { IconResources } from "../../components/icons";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { STRINGS } from "../../constants/strings";
import type { DebugSnapshot } from "../../types";
import { formatRelativeTime } from "../../utils/time";
import { useWindowDragSave } from "../../hooks/useWindowDragSave";
import { useOverlayAnimation } from "../../hooks/useOverlayAnimation";

export function Resources() {
  const [vramUsed, setVramUsed] = useState(0);
  const [vramTotal, setVramTotal] = useState(0);
  const [tokensPerSecond, setTokensPerSecond] = useState(0);
  const [activeModel, setActiveModel] = useState("");
  const [modelDisplayName, setModelDisplayName] = useState("");
  const [totalCompletions, setTotalCompletions] = useState(0);
  const [lastCompletion, setLastCompletion] = useState<string | null>(null);
  const [pinned, setPinned] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [, setTick] = useState(0);

  useWindowDragSave();
  const panelClass = useOverlayAnimation();

  const fetchState = useCallback(() => {
    invoke<DebugSnapshot>("get_debug_snapshot").then((snapshot) => {
      setVramUsed(snapshot.vram.used_mb);
      setVramTotal(snapshot.vram.total_mb);
      setTokensPerSecond(snapshot.inference_stats.tokens_per_second);
      setActiveModel(snapshot.inference_stats.active_model);
      setModelDisplayName(snapshot.inference_stats.model_display_name);
      setTotalCompletions(snapshot.inference_stats.total_completions);
      setLastCompletion(snapshot.inference_stats.last_completion);
    }).catch(() => {});
  }, []);

  useEffect(() => {
    fetchState();
    const interval = setInterval(fetchState, 2000);
    return () => clearInterval(interval);
  }, [fetchState]);

  useTauriEvent("subsystems-reset", () => {
    setVramUsed(0);
    setVramTotal(0);
    setTokensPerSecond(0);
    setActiveModel("");
    setModelDisplayName("");
    setTotalCompletions(0);
    setLastCompletion(null);
  });

  useEffect(() => {
    const interval = setInterval(() => setTick(t => t + 1), 1000);
    return () => clearInterval(interval);
  }, []);

  const vramPercent = vramTotal > 0 ? (vramUsed / vramTotal) * 100 : 0;

  return (
    <div 
      className={`flex flex-col ${collapsed ? '' : 'h-screen'} overflow-hidden text-sm panel-glass ${panelClass}`}
      style={{ background: "var(--bg-panel)", border: "1px solid var(--border)", borderRadius: "var(--radius)" }}
    >
      <TitleBar
        icon={<IconResources size={14} />}
        title={STRINGS.PANEL_RESOURCES}
        pinned={pinned}
        onPinToggle={() => setPinned(!pinned)}
        collapsed={collapsed}
        onCollapseToggle={() => setCollapsed(c => !c)}
      />
      {!collapsed && (
        <div className="flex-1 overflow-y-auto p-3 space-y-4">
          {/* VRAM Section */}
          <div>
            <div className="flex justify-between mb-1">
              <span style={{ color: "var(--text-secondary)", fontSize: 12 }}>VRAM</span>
              <span style={{ color: "var(--text-primary)", fontSize: 12, fontWeight: 600 }}>
                {vramTotal > 0 ? `${vramUsed} / ${vramTotal} MB` : "N/A"}
              </span>
            </div>
            <div style={{ height: 6, background: "rgba(255,255,255,0.08)", borderRadius: 3, overflow: "hidden" }}>
              {vramTotal > 0 && (
              <div style={{
                height: "100%",
                width: `${vramPercent}%`,
                background: vramPercent > 90 ? "var(--status-unavailable)" : vramPercent > 70 ? "var(--status-degraded)" : "var(--status-ready)",
                borderRadius: 3,
                transition: "width 300ms ease",
              }} />
              )}
            </div>
          </div>

          {/* Inference Stats */}
          <div className="space-y-2">
            <div className="flex justify-between">
              <span style={{ color: "var(--text-secondary)", fontSize: 12 }}>Tokens/s</span>
              <span style={{ color: "var(--text-primary)", fontSize: 13, fontWeight: 600, fontFamily: "monospace" }}>
                {tokensPerSecond > 0 ? tokensPerSecond.toFixed(1) : "—"}
              </span>
            </div>
            <div className="flex justify-between">
              <span style={{ color: "var(--text-secondary)", fontSize: 12 }}>Active Model</span>
              <span style={{ color: "var(--text-primary)", fontSize: 12 }} className="truncate ml-2 text-right">
                {modelDisplayName || activeModel || "None loaded"}
              </span>
            </div>
            <div className="flex justify-between">
              <span style={{ color: "var(--text-secondary)", fontSize: 12 }}>Total Completions</span>
              <span style={{ color: "var(--text-primary)", fontSize: 12, fontFamily: "monospace" }}>
                {totalCompletions}
              </span>
            </div>
            {lastCompletion && (
              <div className="flex justify-between">
                <span style={{ color: "var(--text-secondary)", fontSize: 12 }}>Last Completion</span>
                <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
                  {formatRelativeTime(lastCompletion)}
                </span>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
