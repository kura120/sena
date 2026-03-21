import { useState, useEffect } from "react";
import { TitleBar } from "../../components/TitleBar/TitleBar";
import { IconSubsystemHealth } from "../../components/icons";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { KNOWN_SUBSYSTEMS } from "../../constants/panels";
import { STATUS_COLORS } from "../../constants/colors";
import { STRINGS } from "../../constants/strings";
import { SubsystemStatus } from "../../types";
import { formatRelativeTime } from "../../utils/time";

interface SubsystemData {
  name: string;
  status: string;
  lastChange: Date | null;
}

export function SubsystemHealth() {
  const [subsystems, setSubsystems] = useState<SubsystemData[]>(() => 
    KNOWN_SUBSYSTEMS.map(name => ({
      name,
      status: "Unknown",
      lastChange: null
    }))
  );
  const [pinned, setPinned] = useState(false);
  const [, setTick] = useState(0);

  useTauriEvent<SubsystemStatus>("subsystem-status-updated", (payload) => {
    setSubsystems(prev => prev.map(s => 
      s.name === payload.subsystem 
        ? { ...s, status: payload.status, lastChange: new Date() } 
        : s
    ));
  });

  useEffect(() => {
    const interval = setInterval(() => setTick(t => t + 1), 1000);
    return () => clearInterval(interval);
  }, []);

  return (
    <div 
      className="flex flex-col h-screen overflow-hidden text-sm"
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
      />
      <div className="flex-1 overflow-y-auto py-1">
        {subsystems.map(s => (
          <div 
            key={s.name}
            className="flex items-center px-3 py-2 transition-colors cursor-default"
            onMouseEnter={(e) => e.currentTarget.style.background = "var(--bg-hover)"}
            onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}
          >
            <div 
              className="w-2 h-2 rounded-full mr-3 shrink-0"
              style={{ background: STATUS_COLORS[s.status] || STATUS_COLORS.Unknown }}
            />
            <span 
              className="font-medium mr-auto truncate"
              style={{ color: "var(--text-primary)", fontSize: "13px" }}
            >
              {s.name}
            </span>
            <div className="flex flex-col items-end">
              <span 
                className="text-xs font-medium"
                style={{ color: STATUS_COLORS[s.status] || STATUS_COLORS.Unknown }}
              >
                {s.status}
              </span>
              <span 
                className="text-[11px]"
                style={{ color: "var(--text-muted)" }}
              >
                {s.lastChange ? formatRelativeTime(s.lastChange) : "—"}
              </span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
