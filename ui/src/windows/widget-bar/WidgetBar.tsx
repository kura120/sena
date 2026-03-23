import { useState, useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import {
  IconSubsystemHealth,
  IconEventBus,
  IconChat,
  IconResources,
  IconThoughtStream,
  IconMemoryStats,
  IconPromptTrace,
  IconConversationTimeline,
  IconSettings,
  IconModel,
} from "../../components/icons";
import { Tooltip } from "../../components/Tooltip/Tooltip";
import { STRINGS } from "../../constants/strings";
import { PANEL_LABELS } from "../../constants/panels";
import type { IconProps } from "../../types";
import "./WidgetBar.css";

interface PanelButton {
  label: string;
  title: string;
  icon: React.FC<IconProps>;
}

const PANEL_BUTTONS: PanelButton[] = [
  { label: PANEL_LABELS.SUBSYSTEM_HEALTH, title: STRINGS.PANEL_SUBSYSTEM_HEALTH, icon: IconSubsystemHealth },
  { label: PANEL_LABELS.EVENT_BUS, title: STRINGS.PANEL_EVENT_BUS, icon: IconEventBus },
  { label: PANEL_LABELS.CHAT, title: STRINGS.PANEL_CHAT, icon: IconChat },
  { label: PANEL_LABELS.RESOURCES, title: STRINGS.PANEL_RESOURCES, icon: IconResources },
  { label: PANEL_LABELS.THOUGHT_STREAM, title: STRINGS.PANEL_THOUGHT_STREAM, icon: IconThoughtStream },
  { label: PANEL_LABELS.MEMORY_STATS, title: STRINGS.PANEL_MEMORY_STATS, icon: IconMemoryStats },
  { label: PANEL_LABELS.PROMPT_TRACE, title: STRINGS.PANEL_PROMPT_TRACE, icon: IconPromptTrace },
  { label: PANEL_LABELS.CONVERSATION_TIMELINE, title: STRINGS.PANEL_CONVERSATION_TIMELINE, icon: IconConversationTimeline },
];

export function WidgetBar() {
  const [animationClass, setAnimationClass] = useState("widget-bar--hidden");
  const [activePanels, setActivePanels] = useState<Record<string, boolean>>({});

  useEffect(() => {
    // Load initial panel states
    invoke<Record<string, boolean>>("get_panel_states")
      .then(setActivePanels)
      .catch((err) => console.error("Failed to load panel states:", err));

    // Listen for state changes
    const unlistenState = listen<{ label: string; is_open: boolean }>("panel-state-changed", (event) => {
      setActivePanels((prev) => ({
        ...prev,
        [event.payload.label]: event.payload.is_open,
      }));
    });

    const unlistenAnimate = listen<{ action: string; delay_ms: number }>("panel-animate", (event) => {
      if (event.payload.action === "show") {
        setAnimationClass("widget-bar--visible");
      } else if (event.payload.action === "hide") {
        setAnimationClass("widget-bar--hidden");
      }
    });

    return () => {
      unlistenState.then((fn) => fn());
      unlistenAnimate.then((fn) => fn());
    };
  }, []);

  const togglePanel = (label: string) => {
    if (activePanels[label]) {
      invoke("hide_panel", { label }).catch((err) => console.error("Hide panel failed:", err));
    } else {
      invoke("show_panel", { label }).catch((err) => console.error("Show panel failed:", err));
    }
  };

  const openSettings = () => {
    invoke("show_settings_panel").catch((err) => console.error("Show settings failed:", err));
  };

  const openModelPanel = () => {
    invoke("show_model_panel").catch((err) => console.error("Show model panel failed:", err));
  };

  return (
    <div className={`widget-bar ${animationClass}`}>
      <div className="widget-bar__panels">
        {PANEL_BUTTONS.map((panel) => {
          const PanelIcon = panel.icon;
          const isActive = activePanels[panel.label] ?? false;
          return (
            <Tooltip key={panel.label} text={panel.title}>
              <button
                className={`widget-bar__button ${isActive ? "widget-bar__button--active" : ""}`}
                type="button"
                onClick={() => togglePanel(panel.label)}
              >
                <PanelIcon size={18} />
              </button>
            </Tooltip>
          );
        })}
      </div>
      <div className="widget-bar__divider" />
      <Tooltip text={STRINGS.WIDGET_BAR_SETTINGS}>
        <button
          className="widget-bar__button"
          type="button"
          onClick={openSettings}
        >
          <IconSettings size={18} />
        </button>
      </Tooltip>
      <div className="widget-bar__divider" />
      <Tooltip text={STRINGS.WIDGET_BAR_MODEL}>
        <button
          className="widget-bar__button"
          type="button"
          onClick={openModelPanel}
        >
          <IconModel size={18} />
        </button>
      </Tooltip>
    </div>
  );
}
