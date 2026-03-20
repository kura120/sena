#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub id: String,
    pub content: String,
    pub is_user: bool,
    pub timestamp: String,
    pub latency_ms: Option<u64>,
    pub assembly_trace: Option<AssemblyTraceData>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssemblyTraceData {
    pub token_count: u32,
    pub token_budget: u32,
    pub included_tiers: Vec<String>,
    pub dropped_tiers: Vec<String>,
    pub encoding_used: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationTurn {
    pub request_id: String,
    pub user_message_preview: String,
    pub latency_ms: u64,
    pub timestamp: String,
}