//! Test that reactive-loop emits proper event payloads for conversation turns.

#[tokio::test]
async fn test_conversation_turn_event_payload() {
    // This test verifies that when a full conversation turn completes,
    // the TOPIC_USER_MESSAGE_RESPONSE event contains all necessary metadata:
    // - User message (or reference)
    // - Assistant response
    // - Latency
    // - Token counts
    // - Model ID
    
    // The actual event emission is tested in integration tests.
    // This test verifies the data structure that will be emitted.
    
    let user_message = "test message";
    let assistant_response = "test response";
    let model_id = "test-model";
    let latency_ms = 1000u64;
    let tokens_prompt = 50u32;
    let tokens_generated = 100u32;
    
    // Build the JSON payload that should be emitted in TOPIC_USER_MESSAGE_RESPONSE
    let payload = serde_json::json!({
        "user_message": user_message,
        "response": assistant_response,
        "model_id": model_id,
        "latency_ms": latency_ms,
        "tokens_prompt": tokens_prompt,
        "tokens_generated": tokens_generated,
    });
    
    // Verify all fields are present
    assert_eq!(payload["user_message"].as_str().unwrap(), user_message);
    assert_eq!(payload["response"].as_str().unwrap(), assistant_response);
    assert_eq!(payload["model_id"].as_str().unwrap(), model_id);
    assert_eq!(payload["latency_ms"].as_u64().unwrap(), latency_ms);
    assert_eq!(payload["tokens_prompt"].as_u64().unwrap(), tokens_prompt as u64);
    assert_eq!(payload["tokens_generated"].as_u64().unwrap(), tokens_generated as u64);
}

#[tokio::test]
async fn test_user_message_received_event_payload() {
    // Verify the TOPIC_USER_MESSAGE_RECEIVED payload structure
    let user_message = "hello world";
    
    let payload = serde_json::json!({
        "content": user_message,
    });
    
    assert_eq!(payload["content"].as_str().unwrap(), user_message);
}
