//! Integration tests for Phase 11: CognitiveChannelBridge + A2A protocol.
#![cfg(feature = "test-support")]

use std::sync::Arc;

use gyre::channels::IncomingMessage;
use gyre::cognitive::a2a::{A2AMessage, A2ARouter};
use gyre::cognitive::agent::CognitiveAgent;
use gyre::cognitive::channel_bridge::CognitiveChannelBridge;
use gyre::llm::MockLlmProvider;

// ─── CognitiveChannelBridge tests ────────────────────────────────────────────

#[tokio::test]
async fn test_bridge_process_message() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let agent = CognitiveAgent::open(tmp.path(), "bridge_test").expect("open agent");
    let agent = Arc::new(agent);

    let llm = MockLlmProvider::success(
        "I will research the topic thoroughly. This is a comprehensive response with enough \
         detail to be meaningful and trigger memory storage for future recall.",
    );

    let bridge = CognitiveChannelBridge::new(Arc::clone(&agent));

    let msg = IncomingMessage::new("test_channel", "user1", "Tell me about Rust ownership");

    let response = bridge.process_message(&msg, &llm).await.expect("process");

    // Response should be non-empty (MockLlmProvider returns configured text)
    assert!(
        !response.is_empty(),
        "bridge should return non-empty response"
    );
    assert!(
        response.contains("research the topic"),
        "response should contain mock LLM output"
    );

    // Memory should have been stored (response > 100 chars triggers user message storage too)
    let ms = agent.context.memory_stream.lock().expect("lock");
    let memories = ms.recall("", 10).expect("recall");
    assert!(
        !memories.is_empty(),
        "bridge should store memories after processing"
    );
}

#[tokio::test]
async fn test_bridge_post_turn_stores_memory() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let agent = CognitiveAgent::open(tmp.path(), "post_turn_test").expect("open agent");
    let agent = Arc::new(agent);

    let bridge = CognitiveChannelBridge::new(Arc::clone(&agent));

    // Long response triggers both assistant memory extraction and user message storage
    let long_response = "I will implement the new feature. ".repeat(10);
    bridge.post_turn("What should we build next?", &long_response);

    let ms = agent.context.memory_stream.lock().expect("lock");
    let memories = ms.recall("", 20).expect("recall");
    assert!(
        !memories.is_empty(),
        "post_turn should store memories for long responses"
    );
}

#[test]
fn test_cognitive_bridge_system_prompt() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Create agent with soul.md
    let hb = gyre::cognitive::HermitBox::open(tmp.path(), "prompt_agent").expect("open HermitBox");
    std::fs::write(hb.soul_path(), "I am a helpful assistant.\n").expect("write soul.md");
    drop(hb);

    let agent = CognitiveAgent::open(tmp.path(), "prompt_agent").expect("open agent");
    let agent = Arc::new(agent);

    let bridge = CognitiveChannelBridge::new(agent);

    let prompt = bridge.prepare_system_prompt();
    assert!(
        prompt.contains("helpful assistant"),
        "system prompt should contain soul content when soul.md exists, got:\n{}",
        prompt
    );
    assert!(
        prompt.contains("## Agent Identity"),
        "system prompt should contain identity header"
    );
}

// ─── A2A protocol tests ─────────────────────────────────────────────────────

#[test]
fn test_a2a_send_queues_task() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Create sender and receiver agent boxes
    let _sender = CognitiveAgent::open(tmp.path(), "alice").expect("open sender");
    let _receiver = CognitiveAgent::open(tmp.path(), "bob").expect("open receiver");

    let router = A2ARouter::new(tmp.path());
    let msg = A2AMessage {
        from_agent: "alice".to_string(),
        to_agent: "bob".to_string(),
        task: "Prepare the quarterly report".to_string(),
        priority: 8.0,
        created_at: chrono::Utc::now(),
    };

    router.send(&msg).expect("send should succeed");

    // Verify task appears in bob's queue with source='a2a'
    let pending = router.pending_messages("bob").expect("pending_messages");
    assert_eq!(pending.len(), 1, "should have one A2A task");
    assert!(
        pending[0].topic.contains("[FROM:alice]"),
        "task should have sender attribution, got: {}",
        pending[0].topic
    );
    assert!(
        pending[0].topic.contains("quarterly report"),
        "task should contain the original task text"
    );
    assert_eq!(pending[0].source, "a2a", "source should be 'a2a'");
}

#[test]
fn test_a2a_invalid_agent() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Only create sender, not receiver
    let _sender = CognitiveAgent::open(tmp.path(), "alice").expect("open sender");

    let router = A2ARouter::new(tmp.path());
    let msg = A2AMessage {
        from_agent: "alice".to_string(),
        to_agent: "nonexistent".to_string(),
        task: "This should fail".to_string(),
        priority: 5.0,
        created_at: chrono::Utc::now(),
    };

    let result = router.send(&msg);
    assert!(result.is_err(), "sending to non-existent agent should fail");
}

#[test]
fn test_a2a_pending_messages_filters_source() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let receiver = CognitiveAgent::open(tmp.path(), "charlie").expect("open receiver");
    let _sender = CognitiveAgent::open(tmp.path(), "dana").expect("open sender");

    // Add a manual task directly to charlie's queue (not via A2A)
    let engine = gyre::cognitive::curiosity::CuriosityEngine::open_for_agent(&receiver.hermit_box)
        .expect("open engine");
    engine
        .queue
        .push("manual research topic", 5.0, "manual")
        .expect("push manual");

    // Send an A2A task
    let router = A2ARouter::new(tmp.path());
    let msg = A2AMessage {
        from_agent: "dana".to_string(),
        to_agent: "charlie".to_string(),
        task: "A2A specific task".to_string(),
        priority: 7.0,
        created_at: chrono::Utc::now(),
    };
    router.send(&msg).expect("send");

    // pending_messages should only return the A2A task
    let a2a_tasks = router.pending_messages("charlie").expect("pending");
    assert_eq!(a2a_tasks.len(), 1, "should only return A2A tasks");
    assert_eq!(a2a_tasks[0].source, "a2a");
    assert!(a2a_tasks[0].topic.contains("[FROM:dana]"));
}
