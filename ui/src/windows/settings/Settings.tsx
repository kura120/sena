import { useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./Settings.css";

import { OverlayTab } from "./tabs/OverlayTab";
import { AppearanceTab } from "./tabs/AppearanceTab";
import { InferenceTab } from "./tabs/InferenceTab";
import { MemoryEngineTab } from "./tabs/MemoryEngineTab";
import { CtpTab } from "./tabs/CtpTab";
import { PromptComposerTab } from "./tabs/PromptComposerTab";
import { DaemonBusTab } from "./tabs/DaemonBusTab";
import { LoggingTab } from "./tabs/LoggingTab";

export function Settings() {
  const [activeTab, setActiveTab] = useState("overlay");
  const [dirty, setDirty] = useState(false);
  const [restartDirty, setRestartDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  
  // Handlers for the current active tab
  const [currentSaveHandler, setCurrentSaveHandler] = useState<(() => Promise<void>) | null>(null);

  const tabs = [
    { id: "overlay", label: "Overlay", subsystem: "ui" },
    { id: "appearance", label: "Appearance", subsystem: "ui" },
    { id: "inference", label: "Inference", subsystem: "inference" },
    { id: "memory", label: "Memory", subsystem: "memory-engine" },
    { id: "ctp", label: "CTP", subsystem: "ctp" },
    { id: "prompt", label: "Prompt", subsystem: "prompt-composer" },
    { id: "daemon", label: "Daemon Bus", subsystem: "daemon-bus" },
    { id: "logging", label: "Logging", subsystem: null },
  ];

  const handleDirtyChange = useCallback((isDirty: boolean, isRestartDirty: boolean) => {
      setDirty(isDirty);
      setRestartDirty(isRestartDirty);
  }, []);

  const handleTabChange = (tabId: string) => {
      if (dirty) {
          if (!confirm("You have unsaved changes. Discard them?")) {
              return;
          }
      }
      setActiveTab(tabId);
      setDirty(false);
      setRestartDirty(false);
      setCurrentSaveHandler(null);
  };

  const handleRegisterSaveHandler = useCallback((handler: () => Promise<void>) => {
      setCurrentSaveHandler(() => handler);
  }, []);

  const handleSave = async (restart: boolean) => {
      if (!currentSaveHandler) return;
      
      setSaving(true);
      try {
          await currentSaveHandler();
          
          if (restart && restartDirty) {
              const tab = tabs.find(t => t.id === activeTab);
              if (tab && tab.subsystem) {
                  await invoke("restart_subsystem", { subsystem: tab.subsystem });
              }
          }
      } catch (e) {
          console.error("Failed to save settings:", e);
          alert("Failed to save settings: " + e);
      } finally {
          setSaving(false);
      }
  };

  const handleClose = () => {
      getCurrentWindow().hide();
  };

  const renderActiveTab = () => {
      const commonProps = {
          onDirtyChange: handleDirtyChange,
          registerSaveHandler: handleRegisterSaveHandler
      };

      switch (activeTab) {
          case "overlay": return <OverlayTab {...commonProps} subsystem="ui" />;
          case "appearance": return <AppearanceTab {...commonProps} subsystem="ui" />;
          case "inference": return <InferenceTab {...commonProps} subsystem="inference" />;
          case "memory": return <MemoryEngineTab {...commonProps} subsystem="memory-engine" />;
          case "ctp": return <CtpTab {...commonProps} subsystem="ctp" />;
          case "prompt": return <PromptComposerTab {...commonProps} subsystem="prompt-composer" />;
          case "daemon": return <DaemonBusTab {...commonProps} subsystem="daemon-bus" />;
          case "logging": return <LoggingTab {...commonProps} subsystem={null} />; 
          default: return <div>Unknown tab</div>;
      }
  };

  const currentTabInfo = tabs.find(t => t.id === activeTab);

  return (
    <div className="settings-container">
      <div className="settings-header-bar">
        <div className="settings-tabs-header">
            {tabs.map((tab) => (
            <button
                key={tab.id}
                className={`settings-tab-button ${activeTab === tab.id ? "active" : ""}`}
                onClick={() => handleTabChange(tab.id)}
            >
                {tab.label}
            </button>
            ))}
        </div>
        <button className="settings-close-button" onClick={handleClose} title="Close">
            ×
        </button>
      </div>

      <div className="settings-content-area">
        {renderActiveTab()}
      </div>

      <div className="settings-footer">
        {dirty && (
            <>
                {restartDirty && currentTabInfo?.subsystem && (
                    <button 
                        className="settings-btn settings-btn-primary" 
                        onClick={() => handleSave(true)}
                        disabled={saving}
                    >
                        {saving ? "Saving..." : `Save & Restart ${currentTabInfo.subsystem}`}
                    </button>
                )}
                <button 
                    className="settings-btn settings-btn-secondary" 
                    onClick={() => handleSave(false)}
                    disabled={saving}
                >
                    {saving ? "Saving..." : (restartDirty ? "Save Only" : "Save")}
                </button>
            </>
        )}
        {!dirty && (
            <button className="settings-btn settings-btn-secondary" disabled>
                Up to date
            </button>
        )}
      </div>
    </div>
  );
}
