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
  capabilities?: CapabilityBreakdown;
}

export interface CapabilityItem {
  label: string;
  reason?: string;
}

export interface CapabilityBreakdown {
  granted: CapabilityItem[];
  degraded: CapabilityItem[];
  denied: CapabilityItem[];
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
  pre_thought_text: string | null;
  thought_content: string | null;
  chain_of_thought_supported: boolean;
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

export type ToastType = 'info' | 'success' | 'warning' | 'error';
export interface ToastData {
  id: string;
  toast_type: ToastType;
  title: string;
  message: string;
  dismiss_ms: number;
  dismissible: boolean;
  timestamp: number;
}


export type EventCategory = BusEvent["category"];

export interface SubsystemEntry {
  name: string;
  status: string;
  timestamp: string | null;
  boot_signal_name?: string | null;
  capabilities?: CapabilityBreakdown | null;
}

export interface BusEventSnapshot {
  topic: string;
  source: string;
  payload: string;
  timestamp: string;
}

export interface VramSnapshot {
  used_mb: number;
  total_mb: number;
}

export interface ThoughtSnapshot {
  content: string;
  relevance_score: number;
  timestamp: string;
}

export interface MemoryStatsSnapshot {
  short_term_count: number;
  long_term_count: number;
  episodic_count: number;
  last_write: string | null;
  short_term_last_write: string | null;
  long_term_last_write: string | null;
  episodic_last_write: string | null;
}

export interface ParsedChatMessage {
  id: string;
  pre_thought_text: string | null;
  thought_content: string | null;
  final_response: string;
  chain_of_thought_supported: boolean;
  role: "user" | "assistant";
  timestamp: Date;
  model_id?: string;
  latency_ms?: number;
}

export interface PanelStateMap {
  [label: string]: boolean;
}

export interface InferenceStatsSnapshot {
  active_model: string;
  model_display_name: string;
  tokens_per_second: number;
  total_completions: number;
  last_completion: string | null;
}

export interface PromptTraceSnapshot {
  sections: string[];
  toon_output_preview: string;
  token_count: number;
  token_budget: number;
  timestamp: string;
}

export interface ConversationTurnSnapshot {
  role: string;
  content_preview: string;
  model_id: string;
  latency_ms: number;
  tokens_prompt: number;
  tokens_generated: number;
  timestamp: string;
}

export interface DebugSnapshot {
  subsystems: SubsystemEntry[];
  events: BusEventSnapshot[];
  connected: boolean;
  vram: VramSnapshot;
  thoughts: ThoughtSnapshot[];
  memory_stats: MemoryStatsSnapshot;
  inference_stats: InferenceStatsSnapshot;
  prompt_traces: PromptTraceSnapshot[];
  conversation_turns: ConversationTurnSnapshot[];
}

export interface LocalModel {
  path: string;
  display_name: string;
  filename: string;
  size_gb: number;
  architecture: string;
  quantization: string;
  is_active: boolean;
}

export interface OllamaModel {
  name: string;
  tag: string;
  size_gb: number;
  architecture: string;
  blob_digest: string;
  is_extracted: boolean;
  lora_compatible: boolean;
  chain_of_thought_support: boolean;
}
