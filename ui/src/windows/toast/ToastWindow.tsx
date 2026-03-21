import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Toast } from "../../components/Toast/Toast";
import type { OverlayConfigResponse } from "../../types";

export function ToastWindow() {
  const [hotkeyLabel, setHotkeyLabel] = useState("F12");

  useEffect(() => {
    invoke<OverlayConfigResponse>("get_overlay_config")
      .then((config) => setHotkeyLabel(config.toggle_key))
      .catch(() => { /* use default */ });
  }, []);

  const handleDismiss = () => {
    getCurrentWindow().hide();
  };

  return <Toast hotkeyLabel={hotkeyLabel} onDismiss={handleDismiss} />;
}
