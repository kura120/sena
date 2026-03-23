import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SettingsField } from "../../../components/Settings/SettingsField";

interface TabProps {
  onDirtyChange: (dirty: boolean, restartDirty: boolean) => void;
  subsystem: string;
  // Parent can pass a ref or callback to get the modified data for saving
  registerSaveHandler: (handler: () => Promise<void>) => void;
}

// Since the parent manages saving, we need to expose the modified state.
// The user prompt didn't specify how saving works, only the UI.
// But "Save & Restart" implies the parent does the saving.
// The parent `Settings` component will likely need to collect data from the active tab.
// I'll add `registerSaveHandler` to the props definition used in my files, 
// and update `Settings.tsx` to use it.
// Wait, the user didn't ask for `registerSaveHandler`. 
// "Replace the minimal Settings panel... with a full tabbed config editor"
// I need to implement the saving logic.
// The `Settings` component has a "Save" button. It needs to know what to save.
// I will implement a way for the tab to expose its changes.

// Let's stick to the user's pattern but refine the `setValue` part to track restart requirements.
// And I'll need `useEffect` in the tab to notify parent of changes.

export function OverlayTab({ onDirtyChange, subsystem, registerSaveHandler }: TabProps) {
  const [config, setConfig] = useState<Record<string, any> | null>(null);
  const [modified, setModified] = useState<Record<string, any>>({});
  const [restartFields, setRestartFields] = useState<Set<string>>(new Set());

  useEffect(() => {
    invoke("read_subsystem_config", { subsystem })
      .then((data) => setConfig(data as Record<string, any>))
      .catch(console.error);
  }, [subsystem]);

  // Register save handler
  useEffect(() => {
     registerSaveHandler(async () => {
         if (Object.keys(modified).length === 0) return;
         await invoke("write_subsystem_config", { subsystem, values: modified });
         setModified({});
         setRestartFields(new Set());
         onDirtyChange(false, false);
         // Refresh config
         const data = await invoke("read_subsystem_config", { subsystem });
         setConfig(data as Record<string, any>);
     });
  }, [modified, subsystem, registerSaveHandler, onDirtyChange]);

  const getValue = (dottedKey: string) => {
    if (dottedKey in modified) return modified[dottedKey];
    if (!config) return undefined;
    const parts = dottedKey.split('.');
    let val: any = config;
    for (const p of parts) {
      if (val == null) return undefined;
      val = val[p];
    }
    return val;
  };

  const setValue = (dottedKey: string, value: any, restartRequired: boolean) => {
    setModified(prev => {
        const next = { ...prev, [dottedKey]: value };
        return next;
    });
    
    setRestartFields(prev => {
        const next = new Set(prev);
        if (restartRequired) next.add(dottedKey);
        // If we reverted the value? We don't check for revert here for simplicity.
        return next;
    });
    
    // Defer notification slightly to ensure state is updated? 
    // Actually, setting state is async. 
    // onDirtyChange should be called based on the *result*.
    // Since I can't see the result immediately, I'll calculate it.
    const willBeModified = true; // Simplified: any change is modification
    const willRestart = restartRequired || restartFields.size > 0;
    onDirtyChange(willBeModified, willRestart);
  };

  if (!config) return <div className="settings-loading">Loading...</div>;

  return (
    <div className="settings-tab-content">
      <h3 className="settings-section-title">Overlay</h3>
      <SettingsField
        label="Overlay Toggle Key"
        description="Global hotkey to toggle the overlay."
        value={getValue("overlay.toggle_key")}
        onChange={() => {}} 
        type="readonly"
        subsystem={subsystem}
      />
      
      <SettingsField
        label="Overlay Enabled"
        description="Enable or disable the overlay features entirely."
        value={getValue("overlay.enabled")}
        onChange={(v) => setValue("overlay.enabled", v, false)}
        type="toggle"
        subsystem={subsystem}
      />
    </div>
  );
}
