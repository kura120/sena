import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SettingsField } from "../../../components/Settings/SettingsField";

interface TabProps {
  onDirtyChange: (dirty: boolean, restartDirty: boolean) => void;
  subsystem: string;
  registerSaveHandler: (handler: () => Promise<void>) => void;
}

export function CtpTab({ onDirtyChange, subsystem, registerSaveHandler }: TabProps) {
  const [config, setConfig] = useState<Record<string, any> | null>(null);
  const [modified, setModified] = useState<Record<string, any>>({});

  useEffect(() => {
    invoke("read_subsystem_config", { subsystem })
      .then((data) => setConfig(data as Record<string, any>))
      .catch(console.error);
  }, [subsystem]);

  useEffect(() => {
     registerSaveHandler(async () => {
         if (Object.keys(modified).length === 0) return;
         await invoke("write_subsystem_config", { subsystem, values: modified });
         setModified({});
         onDirtyChange(false, false);
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
    setModified(prev => ({ ...prev, [dottedKey]: value }));
    onDirtyChange(true, restartRequired); // No restart fields in CTP tab based on instructions
  };

  if (!config) return <div className="settings-loading">Loading...</div>;

  return (
    <div className="settings-tab-content">
      <h3 className="settings-section-title">Default Weights</h3>
      <SettingsField
        label="Urgency"
        value={getValue("default_weights.urgency")}
        onChange={(v) => setValue("default_weights.urgency", v, false)}
        type="slider"
        min={0.0}
        max={1.0}
        step={0.1}
        subsystem={subsystem}
      />
      <SettingsField
        label="Emotional Resonance"
        value={getValue("default_weights.emotional_resonance")}
        onChange={(v) => setValue("default_weights.emotional_resonance", v, false)}
        type="slider"
        min={0.0}
        max={1.0}
        step={0.1}
        subsystem={subsystem}
      />
      <SettingsField
        label="Novelty"
        value={getValue("default_weights.novelty")}
        onChange={(v) => setValue("default_weights.novelty", v, false)}
        type="slider"
        min={0.0}
        max={1.0}
        step={0.1}
        subsystem={subsystem}
      />
      <SettingsField
        label="Recurrence"
        value={getValue("default_weights.recurrence")}
        onChange={(v) => setValue("default_weights.recurrence", v, false)}
        type="slider"
        min={0.0}
        max={1.0}
        step={0.1}
        subsystem={subsystem}
      />
      <SettingsField
        label="Idle Curiosity"
        value={getValue("default_weights.idle_curiosity")}
        onChange={(v) => setValue("default_weights.idle_curiosity", v, false)}
        type="slider"
        min={0.0}
        max={1.0}
        step={0.1}
        subsystem={subsystem}
      />

      <h3 className="settings-section-title">Surface Thresholds</h3>
      <SettingsField
        label="User Active"
        value={getValue("surface_thresholds.user_active")}
        onChange={(v) => setValue("surface_thresholds.user_active", v, false)}
        type="slider"
        min={0.0}
        max={1.0}
        step={0.05}
        subsystem={subsystem}
      />
      <SettingsField
        label="Idle (2 min)"
        value={getValue("surface_thresholds.idle_2min")}
        onChange={(v) => setValue("surface_thresholds.idle_2min", v, false)}
        type="slider"
        min={0.0}
        max={1.0}
        step={0.05}
        subsystem={subsystem}
      />
      <SettingsField
        label="Idle (10 min)"
        value={getValue("surface_thresholds.idle_10min")}
        onChange={(v) => setValue("surface_thresholds.idle_10min", v, false)}
        type="slider"
        min={0.0}
        max={1.0}
        step={0.05}
        subsystem={subsystem}
      />

      <h3 className="settings-section-title">Queue</h3>
      <SettingsField
        label="Max Depth"
        value={getValue("queue.max_depth")}
        onChange={(v) => setValue("queue.max_depth", v, false)}
        type="input"
        min={10}
        max={500}
        subsystem={subsystem}
      />
    </div>
  );
}
