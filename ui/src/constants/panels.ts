export const PANEL_LABELS = {
  SUBSYSTEM_HEALTH: "subsystem-health",
  EVENT_BUS: "event-bus",
  CHAT: "chat",
  BOOT_TIMELINE: "boot-timeline",
  TOAST: "toast",
  RESOURCES: "resources",
  THOUGHT_STREAM: "thought-stream",
  MEMORY_STATS: "memory-stats",
  PROMPT_TRACE: "prompt-trace",
  CONVERSATION_TIMELINE: "conversation-timeline",
  WIDGET_BAR: "widget-bar",
  SETTINGS: "settings",
  MODEL_PANEL: "model-panel",
} as const;

export const KNOWN_SUBSYSTEMS = [
  "agents", "ctp", "daemon-bus", "inference", "lora-manager",
  "memory-engine", "model-probe", "platform", "prompt-composer",
  "reactive-loop", "soulbox", "ui",
] as const;

/** Expected boot signals in order */
export const EXPECTED_BOOT_SIGNALS = [
  { signal: "DAEMON_BUS_READY", label: "Daemon Bus Ready", required: true },
  { signal: "MEMORY_ENGINE_READY", label: "Memory Engine Ready", required: true },
  { signal: "INFERENCE_READY", label: "Inference Ready", required: true },
  { signal: "MODEL_PROFILE_READY", label: "Model Profile Ready", required: false },
  { signal: "PROMPT_COMPOSER_READY", label: "Prompt Composer Ready", required: true },
  { signal: "REACTIVE_LOOP_READY", label: "Reactive Loop Ready", required: false },
  { signal: "CTP_READY", label: "CTP Ready", required: true },
  { signal: "SENA_READY", label: "Sena Ready", required: true },
] as const;

export const EVENT_MAX_ITEMS = 200;
