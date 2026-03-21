export interface IconProps {
  size?: number;
  className?: string;
}

export interface SubsystemStatus {
  subsystem: string;
  status: "Ready" | "Degraded" | "Unknown" | "Unavailable";
}

export interface BootSignalEvent {
  signal: string;
  required: boolean;
  timestamp: string;
  subsystem: string;
}

export interface BusEvent {
  topic: string;
  source: string;
  payload: string;
  category: "boot" | "error" | "memory" | "ctp" | "user" | "default";
  timestamp: string;
}

export interface ChatMessage {
  id: string;
  content: string;
  role: "user" | "assistant";
  timestamp: Date;
}

export interface SendMessageResponse {
  response: string;
  model_id: string;
  latency_ms: number;
}

export interface OverlayConfigResponse {
  toggle_key: string;
  panels: PanelConfig[];
}

export interface PanelConfig {
  label: string;
  title: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

export type EventCategory = BusEvent["category"];
