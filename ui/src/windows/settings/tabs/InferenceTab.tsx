import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SettingsField } from "../../../components/Settings/SettingsField";

interface TabProps {
  onDirtyChange: (dirty: boolean, restartDirty: boolean) => void;
  subsystem: string;
  registerSaveHandler: (handler: () => Promise<void>) => void;
}

export function InferenceTab({ onDirtyChange, subsystem, registerSaveHandler }: TabProps) {
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
      <h3 className="settings-section-title">gRPC</h3>
      <SettingsField
        label="Listen Port"
        description="Port inference gRPC server listens on."
        value={getValue("grpc.listen_port")}
        onChange={(v) => setValue("grpc.listen_port", v, true)}
        type="input"
        min={1024}
        max={65535}
        restartRequired
        subsystem={subsystem}
      />

      <h3 className="settings-section-title">Model</h3>
      <SettingsField
        label="Model Path"
        description="Path to the GGUF model file. Managed by Model Panel."
        value={getValue("model.model_path")}
        onChange={() => {}}
        type="readonly"
        restartRequired
        subsystem={subsystem}
      />
      <SettingsField
        label="GPU Layers"
        description="Number of layers to offload to GPU."
        value={getValue("model.gpu_layers")}
        onChange={(v) => setValue("model.gpu_layers", v, true)}
        type="input"
        min={0}
        max={999}
        restartRequired
        subsystem={subsystem}
      />
      <SettingsField
        label="Context Length"
        description="Maximum context length in tokens."
        value={getValue("model.context_length")}
        onChange={(v) => setValue("model.context_length", v, true)}
        type="input"
        min={512}
        max={32768}
        restartRequired
        subsystem={subsystem}
      />
      <SettingsField
        label="VRAM Budget (MB)"
        description="Maximum VRAM to use."
        value={getValue("model.vram_budget_mb")}
        onChange={(v) => setValue("model.vram_budget_mb", v, true)}
        type="input"
        min={512}
        max={24576}
        restartRequired
        subsystem={subsystem}
      />

      <h3 className="settings-section-title">Runtime</h3>
      <SettingsField
        label="Request Timeout"
        description="Timeout for inference requests."
        value={getValue("runtime.request_timeout_ms")}
        onChange={(v) => setValue("runtime.request_timeout_ms", v, false)}
        type="slider"
        min={10000}
        max={300000}
        step={1000}
        displayTransform={(val) => `${(val / 1000).toFixed(1)}s`}
        subsystem={subsystem}
      />
      <SettingsField
        label="Request Queue Max Depth"
        description="Maximum number of queued requests."
        value={getValue("runtime.request_queue_max_depth")}
        onChange={(v) => setValue("runtime.request_queue_max_depth", v, false)}
        type="input"
        min={1}
        max={256}
        subsystem={subsystem}
      />
    </div>
  );
}
