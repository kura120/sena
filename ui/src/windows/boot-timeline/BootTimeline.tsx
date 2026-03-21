import { useState, useMemo } from "react";
import { TitleBar } from "../../components/TitleBar/TitleBar";
import { IconBootTimeline } from "../../components/icons";
import { STRINGS } from "../../constants/strings";
import { EXPECTED_BOOT_SIGNALS } from "../../constants/panels";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { formatDuration } from "../../utils/time";
import type { BootSignalEvent } from "../../types";

interface ReceivedSignal {
  signal: string;
  timestamp: Date;
  subsystem: string;
}

export function BootTimeline() {
  const [pinned, setPinned] = useState(false);
  const [receivedSignals, setReceivedSignals] = useState<Map<string, ReceivedSignal>>(new Map());

  useTauriEvent<BootSignalEvent>("boot-signal-received", (payload) => {
    setReceivedSignals(prev => {
      const next = new Map(prev);
      next.set(payload.signal, {
        signal: payload.signal,
        timestamp: new Date(payload.timestamp),
        subsystem: payload.subsystem,
      });
      return next;
    });
  });

  const processedSignals = useMemo(() => {
    // Sort received to compute deltas correctly based on time
    const receivedList = Array.from(receivedSignals.values()).sort((a, b) => a.timestamp.getTime() - b.timestamp.getTime());
    
    // Map signal name to its previous signal for delta calculation
    const signalDeltas = new Map<string, number>();
    
    receivedList.forEach((current, index) => {
      if (index > 0) {
        const prev = receivedList[index - 1];
        if (prev) {
          signalDeltas.set(current.signal, current.timestamp.getTime() - prev.timestamp.getTime());
        }
      } else {
        signalDeltas.set(current.signal, 0);
      }

    });

    return EXPECTED_BOOT_SIGNALS.map(expected => {
      const received = receivedSignals.get(expected.signal);
      return {
        ...expected,
        received,
        delta: received ? signalDeltas.get(expected.signal) : undefined
      };
    });
  }, [receivedSignals]);

  const allRequiredReceived = EXPECTED_BOOT_SIGNALS
    .filter(s => s.required)
    .every(s => receivedSignals.has(s.signal));

  const totalDuration = useMemo(() => {
    if (!allRequiredReceived) return null;
    const timestamps = Array.from(receivedSignals.values()).map(s => s.timestamp.getTime());
    if (timestamps.length === 0) return null;
    const start = Math.min(...timestamps);
    const end = Math.max(...timestamps);
    return end - start;
  }, [receivedSignals, allRequiredReceived]);

  return (
    <div 
      className="flex flex-col h-full overflow-hidden border rounded-lg shadow-xl"
      style={{
        background: "var(--bg-panel)",
        borderColor: "var(--border)",
        borderRadius: "var(--radius)",
      }}
    >
      <TitleBar 
        icon={<IconBootTimeline size={14} />} 
        title={STRINGS.PANEL_BOOT_TIMELINE}
        pinned={pinned}
        onPinToggle={() => setPinned(!pinned)} 
      />
      
      <div className="flex-1 overflow-y-auto p-2 scrollbar-thin">
        <div className="flex flex-col gap-1">
          {processedSignals.map((item) => (
            <div 
              key={item.signal}
              className={`flex flex-col py-1.5 px-3 rounded transition-colors ${item.signal === 'SENA_READY' ? 'mt-2 border border-dashed border-white/10' : 'hover:bg-white/5'}`}
            >
              <div className="flex items-center gap-3">
                {/* Status Dot */}
                <div 
                  className="w-2.5 h-2.5 rounded-full shrink-0 transition-all duration-300"
                  style={{
                    background: item.received 
                      ? "var(--status-ready)" 
                      : "var(--status-unknown)",
                    boxShadow: item.received ? "0 0 8px var(--status-ready)" : "none"
                  }}
                />
                
                {/* Signal Name */}
                <span className="text-[13px] font-medium grow truncate" style={{ color: "var(--text-primary)" }}>
                  {item.label}
                </span>

                {/* Badge */}
                <span 
                  className="text-[10px] font-semibold px-1.5 py-px rounded border"
                  style={{
                    color: "var(--text-muted)",
                    borderColor: "var(--border)",
                  }}
                >
                  {item.required ? STRINGS.BADGE_REQUIRED : STRINGS.BADGE_OPTIONAL}
                </span>

                {/* Time */}
                {item.received && (
                  <span className="text-[11px] font-mono" style={{ color: "var(--text-secondary)" }}>
                    {item.received.timestamp.toLocaleTimeString([], { hour12: false })}
                  </span>
                )}
              </div>

              {/* Delta Line */}
              {item.received && item.delta !== undefined && (
                <div className="flex items-center ml-[22px] mt-0.5">
                  <span className="text-[11px]" style={{ color: "var(--text-muted)" }}>
                    {item.delta > 0 ? `+${formatDuration(item.delta)}` : (item.delta === 0 ? "+0ms" : "")}
                  </span>
                </div>
              )}
            </div>
          ))}
        </div>

        {/* Total Duration Footer */}
        {allRequiredReceived && totalDuration !== null && (
          <div 
            className="mt-4 pt-3 border-t flex justify-between items-center px-3 pb-2 animate-fade-in"
            style={{ borderColor: "var(--border)" }}
          >
            <span className="text-xs" style={{ color: "var(--text-muted)" }}>
              {STRINGS.BOOT_TOTAL_DURATION}
            </span>
            <span className="text-sm font-bold font-mono" style={{ color: "var(--status-ready)" }}>
              {formatDuration(totalDuration)}
            </span>
          </div>
        )}
      </div>
    </div>
  );
}
