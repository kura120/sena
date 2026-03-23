import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SettingsField } from "../../../components/Settings/SettingsField";

interface TabProps {
  onDirtyChange: (dirty: boolean, restartDirty: boolean) => void;
  subsystem: string;
  registerSaveHandler: (handler: () => Promise<void>) => void;
}

export function PromptComposerTab({ onDirtyChange, subsystem, registerSaveHandler }: TabProps) {
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
    onDirtyChange(true, restartRequired);
  };

  if (!config) return <div className="settings-loading">Loading...</div>;

  return (
    <div className="settings-tab-content">
      <h3 className="settings-section-title">Context Window</h3>
      <SettingsField
        label="ESU Savings Threshold"
        value={getValue("context_window.esu_savings_threshold")}
        onChange={(v) => setValue("context_window.esu_savings_threshold", v, false)}
        type="slider"
        min={0.0}
        max={0.5}
        step={0.01}
        subsystem={subsystem}
      />
      <SettingsField
        label="Tokens Per Char Estimate"
        value={getValue("context_window.tokens_per_char_estimate")}
        onChange={(v) => setValue("context_window.tokens_per_char_estimate", v, false)}
        type="slider"
        min={0.1}
        max={0.5}
        step={0.01}
        subsystem={subsystem}
      />

      <h3 className="settings-section-title">Sacred</h3>
      <SettingsField
        label="Sacred Fields"
        value={getValue("sacred.sacred_fields")}
        onChange={() => {}}
        type="readonly"
        subsystem={subsystem}
      />

      <h3 className="settings-section-title">Response Format</h3>
      <SettingsField
        label="System Instruction"
        value={getValue("response_format.system_instruction")}
        onChange={() => {}}
        type="readonly"
        subsystem={subsystem}
      />
    </div>
  );
}
