import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { TitleBar } from "../../components/TitleBar/TitleBar";
import { IconSubsystemHealth, IconBell, IconReboot, IconChevronRight, IconChevronDown } from "../../components/icons";
import { CapabilityDetail } from "../../components/CapabilityDetail/CapabilityDetail";
import { CapabilityChips } from "../../components/CapabilityChips/CapabilityChips";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { KNOWN_SUBSYSTEMS } from "../../constants/panels";
import { STATUS_COLORS } from "../../constants/colors";
import { STRINGS } from "../../constants/strings";
import type { DebugSnapshot, CapabilityBreakdown, BootSignalEvent } from "../../types";
import { formatRelativeTime } from "../../utils/time";
import { useWindowDragSave } from "../../hooks/useWindowDragSave";
import { useOverlayAnimation } from "../../hooks/useOverlayAnimation";

interface SubsystemData {
  name: string;
  status: string;
  lastChange: Date | null;
  bootSignalName?: string | null;
  capabilities?: CapabilityBreakdown | null;
}

export function SubsystemHealth() {
  const [subsystems, setSubsystems] = useState<SubsystemData[]>(() => 
    KNOWN_SUBSYSTEMS.map(name => ({
      name,
      status: "Unknown",
      lastChange: null,
      bootSignalName: null,
      capabilities: null,
    }))
  );
  const [pinned, setPinned] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [, setTick] = useState(0);
  const [verbose, setVerbose] = useState(false);

  useWindowDragSave();
  const panelClass = useOverlayAnimation();

  // Load verbose setting on mount
  useEffect(() => {
    invoke<boolean>("get_overlay_setting", { key: "verbose_health" })
      .then((val) => setVerbose(!!val))
      .catch(() => { /* default false */ });
  }, []);

  // Listen for verbose setting changes from the Settings panel
  useTauriEvent<{ key: string; value: unknown }>("overlay-setting-changed", (event) => {
    if (event.key === "verbose_health") {
      setVerbose(!!event.value);
    }
  });

  // Fetch accumulated state from Rust DebugState
  const fetchState = useCallback(() => {
    invoke<DebugSnapshot>("get_debug_snapshot").then((snapshot) => {
      setSubsystems(prev => prev.map(s => {
        const match = snapshot.subsystems.find(e => e.name === s.name);
        if (match) {
          return {
            ...s,
            status: match.status,
            lastChange: match.timestamp ? new Date(match.timestamp) : s.lastChange,
            bootSignalName: match.boot_signal_name || s.bootSignalName,
            capabilities: match.capabilities || s.capabilities,
          };
        }
        return s;
      }));
    }).catch(() => { /* backend not ready yet */ });
  }, []);

  // Poll every 2s to pick up new state
  useEffect(() => {
    fetchState();
    const interval = setInterval(fetchState, 2000);
    return () => clearInterval(interval);
  }, [fetchState]);

  // Listen for subsystems-reset event (emitted during reboot)
  useTauriEvent("subsystems-reset", () => {
    setSubsystems(
      KNOWN_SUBSYSTEMS.map(name => ({
        name,
        status: "Unknown",
        lastChange: null,
        bootSignalName: null,
        capabilities: null,
      }))
    );
    setExpanded(new Set());
  });

  // Listen for boot-signal-received to update signal names and capabilities in real time
  useTauriEvent<BootSignalEvent>("boot-signal-received", (event) => {
    setSubsystems(prev => prev.map(s => {
      if (s.name === event.subsystem) {
        return {
          ...s,
          bootSignalName: event.signal,
          capabilities: event.capabilities || s.capabilities,
        };
      }
      return s;
    }));
  });

  // Tick for relative time display updates
  useEffect(() => {
    const interval = setInterval(() => setTick(t => t + 1), 1000);
    return () => clearInterval(interval);
  }, []);

  const handleReboot = useCallback(() => {
    invoke("reboot_daemon_bus").catch((error: unknown) => {
      console.error("Failed to reboot daemon-bus:", error);
    });
  }, []);

  const handleShowHistory = useCallback(() => {
    invoke("show_notification_history").catch((error: unknown) => {
      console.error("Failed to show notification history:", error);
    });
  }, []);

  const toggleExpanded = useCallback((name: string) => {
    setExpanded(prev => {
      const next = new Set(prev);
      if (next.has(name)) {
        next.delete(name);
      } else {
        next.add(name);
      }
      return next;
    });
  }, []);

  const getCapabilities = useCallback((s: SubsystemData): CapabilityBreakdown | null => {
    // If real capabilities are available, return them in verbose mode
    if (s.capabilities) {
      if (verbose) {
        return s.capabilities;
      }
      // In non-verbose mode, provide a summary
      const hasDegraded = s.capabilities.degraded.length > 0;
      if (hasDegraded) {
        const summary = STRINGS.VERBOSE_DEGRADED_SUMMARY.replace(
          "{count}",
          String(s.capabilities.degraded.length),
        );
        return {
          granted: [],
          degraded: [{ label: summary }],
          denied: [],
        };
      }
      return {
        granted: [{ label: STRINGS.CAPABILITY_ALL_OPERATIONAL }],
        degraded: [],
        denied: [],
      };
    }

    // Generate default capabilities based on status
    if (s.status === "Ready") {
      return {
        granted: [{ label: STRINGS.CAPABILITY_ALL_OPERATIONAL }],
        degraded: [],
        denied: [],
      };
    } else if (s.status === "Unknown") {
      return {
        granted: [],
        degraded: [],
        denied: [{ label: STRINGS.CAPABILITY_NOT_STARTED }],
      };
    } else if (s.status === "Degraded") {
      return {
        granted: [],
        degraded: [{ label: STRINGS.STATUS_DEGRADED }],
        denied: [],
      };
    }

    return null;
  }, [verbose]);

  const extraActions = (
    <>
      <button
        onClick={handleShowHistory}
        className="p-1 rounded transition-colors"
        style={{ color: "var(--text-muted)" }}
        onMouseEnter={(e) => e.currentTarget.style.background = "var(--bg-hover)"}
        onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}
        title="Notification History"
      >
        <IconBell size={14} />
      </button>
      <button
        onClick={handleReboot}
        className="p-1 rounded transition-colors"
        style={{ color: "var(--text-muted)" }}
        onMouseEnter={(e) => e.currentTarget.style.background = "var(--bg-hover)"}
        onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}
        title="Reboot daemon-bus"
      >
        <IconReboot size={14} />
      </button>
    </>
  );

  return (
    <div 
      className={`flex flex-col ${collapsed ? '' : 'h-screen'} overflow-hidden text-sm panel-glass ${panelClass}`}
      style={{
        background: "var(--bg-panel)",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius)"
      }}
    >
      <TitleBar 
        icon={<IconSubsystemHealth size={14} />} 
        title={STRINGS.PANEL_SUBSYSTEM_HEALTH} 
        pinned={pinned} 
        onPinToggle={() => setPinned(!pinned)}
        collapsed={collapsed}
        onCollapseToggle={() => setCollapsed(c => !c)}
        extraActions={extraActions}
      />
      {!collapsed && (
      <div className="flex-1 overflow-y-auto py-1">
        {subsystems.map(s => {
          const isExpanded = expanded.has(s.name);
          const capabilities = getCapabilities(s);
          const ChevronIcon = isExpanded ? IconChevronDown : IconChevronRight;

          // Derive display status: if real capabilities have degraded items, show Degraded
          const hasDegradedCaps = s.capabilities && s.capabilities.degraded.length > 0;
          const displayStatus = (s.status === "Ready" && hasDegradedCaps)
            ? STRINGS.STATUS_DEGRADED
            : s.status;

          return (
            <div key={s.name}>
              <div 
                className="flex items-center px-3 py-2 transition-colors cursor-pointer"
                onMouseEnter={(e) => e.currentTarget.style.background = "var(--bg-hover)"}
                onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}
                onClick={() => toggleExpanded(s.name)}
              >
                <div 
                  className="w-2 h-2 rounded-full mr-3 shrink-0"
                  style={{ background: STATUS_COLORS[displayStatus] || STATUS_COLORS.Unknown }}
                />
                <span 
                  className="font-medium truncate"
                  style={{ color: "var(--text-primary)", fontSize: "13px" }}
                >
                  {s.name}
                </span>
                <div className="flex items-center gap-2 ml-auto shrink-0">
                  <div className="flex flex-col items-end">
                    <span 
                      className="text-xs font-medium"
                      style={{ color: STATUS_COLORS[displayStatus] || STATUS_COLORS.Unknown }}
                    >
                      {displayStatus}
                    </span>
                    <span 
                      className="text-[11px]"
                      style={{ color: "var(--text-muted)" }}
                    >
                      {s.lastChange ? formatRelativeTime(s.lastChange) : "—"}
                    </span>
                  </div>
                  {s.bootSignalName && (
                    <span 
                      className="text-[11px] ml-2"
                      style={{ color: "var(--text-muted)" }}
                    >
                      {s.bootSignalName}
                    </span>
                  )}                  <ChevronIcon 
                    size={16}
                    className="opacity-60 transition-opacity"
                  />
                </div>
              </div>
              {isExpanded && capabilities && (
                verbose
                  ? <CapabilityChips capabilities={capabilities} />
                  : <CapabilityDetail capabilities={capabilities} />
              )}
            </div>
          );
        })}
      </div>
      )}
    </div>
  );
}
