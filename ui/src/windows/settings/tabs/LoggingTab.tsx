import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SettingsField } from "../../../components/Settings/SettingsField";
import { STRINGS } from "../../../constants/strings";

interface TabProps {
  onDirtyChange: (dirty: boolean, restartDirty: boolean) => void;
  subsystem: string | null;
  registerSaveHandler: (handler: () => Promise<void>) => void;
}

const SUBSYSTEMS = [
    "daemon-bus",
    "inference",
    "memory-engine",
    "ctp",
    "prompt-composer",
    "reactive-loop",
    "ui"
];

const LOG_LEVELS = [
    { label: "Trace", value: "trace" },
    { label: "Debug", value: "debug" },
    { label: "Info", value: "info" },
    { label: "Warn", value: "warn" },
    { label: "Error", value: "error" },
];

export function LoggingTab({ onDirtyChange, registerSaveHandler }: TabProps) {
  // Map subsystem name -> config object
  const [configs, setConfigs] = useState<Record<string, any>>({});
  const [loading, setLoading] = useState(true);
  const [modified, setModified] = useState<Record<string, Record<string, any>>>({});
  const [verboseHealth, setVerboseHealth] = useState(false);

  useEffect(() => {
    // Load subsystem configs
    Promise.all(SUBSYSTEMS.map(s => 
        invoke("read_subsystem_config", { subsystem: s })
            .then(data => ({ name: s, data }))
            .catch(e => {
                console.error(`Failed to load config for ${s}`, e);
                return { name: s, data: null };
            })
    )).then(results => {
        const newConfigs: Record<string, any> = {};
        results.forEach(r => {
            if (r.data) newConfigs[r.name] = r.data;
        });
        setConfigs(newConfigs);
        setLoading(false);
    });

    // Load verbose health setting
    invoke<any>("get_overlay_setting", { key: "verbose_health" })
      .then((val) => setVerboseHealth(!!val))
      .catch(() => { /* default false */ });
  }, []);

  useEffect(() => {
    registerSaveHandler(async () => {
        if (Object.keys(modified).length === 0) return;
        
        for (const [sub, mods] of Object.entries(modified)) {
             await invoke("write_subsystem_config", { subsystem: sub, values: mods });
        }
        
        setModified({});
        onDirtyChange(false, false);
        
        // Refresh all
        const results = await Promise.all(SUBSYSTEMS.map(s => invoke("read_subsystem_config", { subsystem: s }).then(d => ({name: s, data: d}))));
        const newConfigs: Record<string, any> = {};
        results.forEach(r => {
            if (r.data) newConfigs[r.name] = r.data;
        });
        setConfigs(newConfigs);
    });
  }, [modified, registerSaveHandler, onDirtyChange]);


  const getValue = (subsystem: string, dottedKey: string) => {
    if (modified[subsystem] && dottedKey in modified[subsystem]) {
        return modified[subsystem][dottedKey];
    }
    const config = configs[subsystem];
    if (!config) return undefined;
    
    // Simple traversal
    const parts = dottedKey.split('.');
    let val: any = config;
    for (const p of parts) {
      if (val == null) return undefined;
      val = val[p];
    }
    return val;
  };

  const setValue = (subsystem: string, dottedKey: string, value: any) => {
    setModified(prev => {
        const subMod = prev[subsystem] || {};
        return {
            ...prev,
            [subsystem]: {
                ...subMod,
                [dottedKey]: value
            }
        };
    });
    
    // Always restart required for logging
    onDirtyChange(true, true);
  };

  if (loading) return <div className="settings-loading">Loading...</div>;

  const handleVerboseToggle = (checked: boolean) => {
    setVerboseHealth(checked);
    invoke("set_overlay_setting", { key: "verbose_health", value: checked })
      .catch((e: unknown) => console.error("Failed to save verbose_health setting:", e));
  };

  return (
    <div className="settings-tab-content">
      <h3 className="settings-section-title">{STRINGS.VERBOSE_HEALTH_LABEL}</h3>
      <SettingsField
        label={STRINGS.VERBOSE_HEALTH_LABEL}
        description={STRINGS.VERBOSE_HEALTH_DESC}
        value={verboseHealth}
        onChange={handleVerboseToggle}
        type="toggle"
      />

      <h3 className="settings-section-title" style={{ marginTop: 24 }}>Subsystem Logging Levels</h3>
      <div style={{ marginBottom: 16, fontSize: '13px', color: 'var(--text-tertiary)' }}>
          All logging level changes require a restart of the respective subsystem.
      </div>
      
      {SUBSYSTEMS.map(sub => (
          <SettingsField
            key={sub}
            label={sub.charAt(0).toUpperCase() + sub.slice(1)} // Capitalize
            value={getValue(sub, "logging.level") || "info"} // Fallback to info
            onChange={(v) => setValue(sub, "logging.level", v)}
            type="dropdown"
            options={LOG_LEVELS}
            restartRequired
            subsystem={sub}
            description={`Current: ${configs[sub]?.logging?.level || 'unknown'}`}
          />
      ))}
    </div>
  );
}
