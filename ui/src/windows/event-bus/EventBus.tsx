import { useState, useEffect, useRef } from "react";
import { TitleBar } from "../../components/TitleBar/TitleBar";
import { IconEventBus, IconChevronRight, IconChevronDown } from "../../components/icons";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { EVENT_MAX_ITEMS } from "../../constants/panels";
import { CATEGORY_COLORS } from "../../constants/colors";
import { STRINGS } from "../../constants/strings";
import { BusEvent } from "../../types";
import { formatRelativeTime } from "../../utils/time";

function parsePayload(payload: string): Array<[string, string]> {
  try {
    const obj = JSON.parse(payload);
    if (typeof obj === "object" && obj !== null) {
      return Object.entries(obj).map(([k, v]) => [k, typeof v === 'object' ? JSON.stringify(v) : String(v)]);
    }
  } catch { /* not JSON */ }
  return [["raw", payload]];
}

export function EventBus() {
  const [events, setEvents] = useState<BusEvent[]>([]);
  const [pinned, setPinned] = useState(false);
  const [expandedIndex, setExpandedIndex] = useState<number | null>(null);
  const [autoScroll, setAutoScroll] = useState(true);
  const [, setTick] = useState(0); // Force re-render for relative times
  const listRef = useRef<HTMLDivElement>(null);

  useTauriEvent<BusEvent>("bus-event", (event) => {
    setEvents(prev => {
      const newEvents = [...prev, event];
      if (newEvents.length > EVENT_MAX_ITEMS) {
        return newEvents.slice(newEvents.length - EVENT_MAX_ITEMS);
      }
      return newEvents;
    });
  });

  // Auto-tick for relative times
  useEffect(() => {
    const interval = setInterval(() => setTick(t => t + 1), 1000);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    if (autoScroll && listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [events, autoScroll]);

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
        icon={<IconEventBus size={14} />} 
        title={STRINGS.PANEL_EVENT_BUS} 
        pinned={pinned} 
        onPinToggle={() => setPinned(!pinned)} 
      />
      <div 
        ref={listRef}
        className="flex-1 overflow-y-auto py-1 scroll-smooth"
        onMouseEnter={() => setAutoScroll(false)}
        onMouseLeave={() => setAutoScroll(true)}
      >
        {events.map((e, i) => {
          const isExpanded = expandedIndex === i;
          return (
            <div 
              key={`${e.timestamp}-${i}`} 
              className="group"
            >
              <div
                 className="flex items-center mx-2 px-2 py-2 cursor-pointer transition-colors border-l-[3px] my-1"
                 style={{ 
                   borderColor: CATEGORY_COLORS[e.category] || CATEGORY_COLORS.default,
                   background: isExpanded ? "var(--bg-hover)" : "transparent"
                 }}
                 onClick={() => setExpandedIndex(isExpanded ? null : i)}
                 onMouseEnter={(evt) => { if(!isExpanded) evt.currentTarget.style.background = "var(--bg-hover)"; }}
                 onMouseLeave={(evt) => { if(!isExpanded) evt.currentTarget.style.background = "transparent"; }}
              >
                <div 
                  className="w-[52px] shrink-0 font-mono text-[11px] whitespace-nowrap"
                  style={{ color: "var(--text-muted)" }}
                >
                  {formatRelativeTime(e.timestamp)}
                </div>
                <div 
                  className="flex-1 min-w-0 font-medium truncate px-2"
                  style={{ color: "var(--text-primary)", fontSize: "13px" }}
                >
                  {e.topic}
                </div>
                <div 
                  className="truncate text-[11px] mr-2"
                  style={{ color: "var(--text-secondary)" }}
                >
                  {e.source}
                </div>
                <div style={{ color: "var(--text-muted)" }}>
                   {isExpanded ? <IconChevronDown size={14} /> : <IconChevronRight size={14} />}
                </div>
              </div>
              
              {isExpanded && (
                <div 
                  className="mx-3 mb-2 px-3 py-2 rounded text-[12px] font-mono overflow-x-auto"
                  style={{ background: "rgba(0,0,0,0.1)", color: "var(--text-secondary)" }}
                >
                  {parsePayload(e.payload).map(([k, v], idx) => (
                    <div key={idx} className="whitespace-pre-wrap break-all">
                      <span className="opacity-70">{k}:</span> {v}
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        })}
        {events.length === 0 && (
            <div className="p-4 text-center text-xs" style={{ color: "var(--text-muted)" }}>
                Waiting for events...
            </div>
        )}
      </div>
    </div>
  );
}
