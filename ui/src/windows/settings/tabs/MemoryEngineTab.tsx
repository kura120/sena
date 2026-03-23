import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SettingsField } from "../../../components/Settings/SettingsField";

interface TabProps {
  onDirtyChange: (dirty: boolean, restartDirty: boolean) => void;
  subsystem: string;
  registerSaveHandler: (handler: () => Promise<void>) => void;
}

export function MemoryEngineTab({ onDirtyChange, subsystem, registerSaveHandler }: TabProps) {
  const [config, setConfig] = useState<Record<string, any> | null>(null);
  const [modified, setModified] = useState<Record<string, any>>({});
  const [restartFields, setRestartFields] = useState<Set<string>>(new Set());

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
         setRestartFields(new Set());
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
    if (restartRequired) {
        setRestartFields(prev => new Set(prev).add(dottedKey));
    }
    onDirtyChange(true, restartRequired || restartFields.size > 0);
  };

  if (!config) return <div className="settings-loading">Loading...</div>;

  return (
    <div className="settings-tab-content">
      <h3 className="settings-section-title">Tiers</h3>
      <SettingsField
        label="Short Term Max Entries"
        value={getValue("tier.short_term.max_entries")}
        onChange={(v) => setValue("tier.short_term.max_entries", v, false)}
        type="input"
        min={32}
        max={1024}
        subsystem={subsystem}
      />
      <SettingsField
        label="Long Term Max Entries"
        value={getValue("tier.long_term.max_entries")}
        onChange={(v) => setValue("tier.long_term.max_entries", v, false)}
        type="input"
        min={1000}
        max={100000}
        subsystem={subsystem}
      />
      <SettingsField
        label="Episodic Max Entries"
        value={getValue("tier.episodic.max_entries")}
        onChange={(v) => setValue("tier.episodic.max_entries", v, false)}
        type="input"
        min={1000}
        max={500000}
        subsystem={subsystem}
      />

      <h3 className="settings-section-title">Decay</h3>
      <SettingsField
        label="Decay Rate"
        value={getValue("decay.rate")}
        onChange={(v) => setValue("decay.rate", v, false)}
        type="slider"
        min={0.5}
        max={1.0}
        step={0.01}
        subsystem={subsystem}
      />
      <SettingsField
        label="Decay Floor"
        value={getValue("decay.floor")}
        onChange={(v) => setValue("decay.floor", v, false)}
        type="slider"
        min={0.0}
        max={0.5}
        step={0.01}
        subsystem={subsystem}
      />

      <h3 className="settings-section-title">Embedder</h3>
      <SettingsField
        label="GPU Layers"
        value={getValue("embedder.gpu_layers")}
        onChange={(v) => setValue("embedder.gpu_layers", v, true)}
        type="input"
        min={0}
        max={999}
        restartRequired
        subsystem={subsystem}
      />

      <h3 className="settings-section-title">Queue</h3>
      <SettingsField
        label="Max Queue Depth"
        value={getValue("queue.max_depth")}
        onChange={(v) => setValue("queue.max_depth", v, false)}
        type="input"
        min={16}
        max={4096}
        subsystem={subsystem}
      />
      <SettingsField
        label="Operation Timeout (ms)"
        value={getValue("queue.operation_timeout_ms")}
        onChange={(v) => setValue("queue.operation_timeout_ms", v, false)}
        type="input"
        min={1000}
        max={60000}
        subsystem={subsystem}
      />
    </div>
  );
}
