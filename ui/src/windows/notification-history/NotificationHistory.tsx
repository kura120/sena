import { useState, useEffect, useCallback } from "react";
import { TitleBar } from "../../components/TitleBar/TitleBar";
import { IconBell } from "../../components/icons";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { TOAST_TYPE_COLORS } from "../../constants/colors";
import { STRINGS } from "../../constants/strings";
import { formatRelativeTime } from "../../utils/time";
import type { ToastData } from "../../types";

const HISTORY_MAX = 50;
const STORE_KEY = "notification-history";

// Lazy-loaded store access
async function getStore() {
  const { load } = await import("@tauri-apps/plugin-store");
  return load("notification-history.json", { defaults: { [STORE_KEY]: [] } });
}

export function NotificationHistory() {
  const [history, setHistory] = useState<ToastData[]>([]);
  const [pinned, setPinned] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [, setTick] = useState(0);

  // Load persisted history on mount
  useEffect(() => {
    getStore().then(async (store) => {
      const saved = await store.get<ToastData[]>(STORE_KEY);
      if (saved && Array.isArray(saved)) {
        setHistory(saved);
      }
    }).catch(() => { /* store not available */ });
  }, []);

  // Persist history when it changes
  const persistHistory = useCallback(async (items: ToastData[]) => {
    try {
      const store = await getStore();
      await store.set(STORE_KEY, items);
    } catch {
      // store not available
    }
  }, []);

  // Listen for new toasts
  useTauriEvent<ToastData>("toast", (payload) => {
    setHistory((prev) => {
      const next = [payload, ...prev].slice(0, HISTORY_MAX);
      persistHistory(next);
      return next;
    });
  });

  // Tick for relative time updates
  useEffect(() => {
    const interval = setInterval(() => setTick((t) => t + 1), 10000);
    return () => clearInterval(interval);
  }, []);

  const handleClearAll = useCallback(() => {
    setHistory([]);
    persistHistory([]);
  }, [persistHistory]);

  return (
    <div
      className={`flex flex-col ${collapsed ? '' : 'h-screen'} overflow-hidden text-sm`}
      style={{
        background: "var(--bg-panel)",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius)",
      }}
    >
      <TitleBar
        icon={<IconBell size={14} />}
        title={STRINGS.TOAST_NOTIFICATION_HISTORY}
        pinned={pinned}
        onPinToggle={() => setPinned(!pinned)}
        collapsed={collapsed}
        onCollapseToggle={() => setCollapsed(c => !c)}
      />
      {!collapsed && (
      <>
      <div className="flex items-center justify-end px-3 py-1.5">
        <button
          onClick={handleClearAll}
          className="text-[11px] px-2 py-0.5 rounded transition-colors"
          style={{ color: "var(--text-muted)" }}
          onMouseEnter={(e) => {
            e.currentTarget.style.background = "var(--bg-hover)";
            e.currentTarget.style.color = "var(--text-secondary)";
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.background = "transparent";
            e.currentTarget.style.color = "var(--text-muted)";
          }}
        >
          {STRINGS.TOAST_CLEAR_ALL}
        </button>
      </div>
      <div className="flex-1 overflow-y-auto">
        {history.length === 0 ? (
          <div
            className="flex items-center justify-center h-full"
            style={{ color: "var(--text-muted)" }}
          >
            <span className="text-xs">{STRINGS.TOAST_NO_NOTIFICATIONS}</span>
          </div>
        ) : (
          history.map((item) => (
            <div
              key={item.id}
              className="flex items-start gap-2.5 px-3 py-2 transition-colors"
              onMouseEnter={(e) =>
                (e.currentTarget.style.background = "var(--bg-hover)")
              }
              onMouseLeave={(e) =>
                (e.currentTarget.style.background = "transparent")
              }
            >
              <div
                className="w-2 h-2 rounded-full mt-1.5 shrink-0"
                style={{
                  background:
                    TOAST_TYPE_COLORS[item.toast_type] ||
                    TOAST_TYPE_COLORS.info,
                }}
              />
              <div className="flex-1 min-w-0">
                <div className="flex items-baseline gap-2">
                  <span
                    className="text-xs font-medium truncate"
                    style={{ color: "var(--text-primary)" }}
                  >
                    {item.title}
                  </span>
                  <span
                    className="text-[10px] shrink-0 ml-auto"
                    style={{ color: "var(--text-muted)" }}
                  >
                    {formatRelativeTime(new Date(item.timestamp))}
                  </span>
                </div>
                <div
                  className="text-[11px] truncate"
                  style={{ color: "var(--text-secondary)" }}
                >
                  {item.message}
                </div>
              </div>
            </div>
          ))
        )}
      </div>
      </>
      )}
    </div>
  );
}

