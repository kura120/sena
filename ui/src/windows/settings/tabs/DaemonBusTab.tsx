import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SettingsField } from "../../../components/Settings/SettingsField";

interface TabProps {
  onDirtyChange: (dirty: boolean, restartDirty: boolean) => void;
  subsystem: string;
  registerSaveHandler: (handler: () => Promise<void>) => void;
}

export function DaemonBusTab({ onDirtyChange, subsystem, registerSaveHandler }: TabProps) {
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
      <h3 className="settings-section-title">Bus</h3>
      <SettingsField
        label="Channel Capacity"
        value={getValue("bus.channel_capacity")}
        onChange={(v) => setValue("bus.channel_capacity", v, true)}
        type="input"
        min={64}
        max={4096}
        restartRequired
        subsystem={subsystem}
      />

      <h3 className="settings-section-title">Supervisor</h3>
      <SettingsField
        label="Max Retries"
        value={getValue("supervisor.max_retries")}
        onChange={(v) => setValue("supervisor.max_retries", v, true)}
        type="input"
        min={1}
        max={5}
        restartRequired
        subsystem={subsystem}
      />
      <SettingsField
        label="Backoff (ms)"
        value={getValue("supervisor.backoff_ms")}
        onChange={(v) => setValue("supervisor.backoff_ms", v, true)}
        type="array-input"
        restartRequired
        subsystem={subsystem}
      />

      <h3 className="settings-section-title">Arbitration</h3>
      <SettingsField
        label="Max Escalation Duration"
        value={getValue("arbitration.max_escalation_duration_ms")}
        onChange={(v) => setValue("arbitration.max_escalation_duration_ms", v, false)}
        type="input"
        min={1000}
        max={120000}
        subsystem={subsystem}
      />
      <SettingsField
        label="Default Escalation Duration"
        value={getValue("arbitration.default_escalation_duration_ms")}
        onChange={(v) => setValue("arbitration.default_escalation_duration_ms", v, false)}
        type="input"
        min={1000}
        max={60000}
        subsystem={subsystem}
      />

      <h3 className="settings-section-title">Watchdog</h3>
      <SettingsField
        label="Default Task Timeout"
        value={getValue("watchdog.default_task_timeout_ms")}
        onChange={(v) => setValue("watchdog.default_task_timeout_ms", v, false)}
        type="input"
        min={1000}
        max={120000}
        subsystem={subsystem}
      />
       <SettingsField
        label="Max Task Timeout"
        value={getValue("watchdog.max_task_timeout_ms")}
        onChange={(v) => setValue("watchdog.max_task_timeout_ms", v, false)}
        type="input"
        min={1000}
        max={300000}
        subsystem={subsystem}
      />
    </div>
  );
}
