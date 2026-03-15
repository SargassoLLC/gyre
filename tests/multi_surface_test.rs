//! Phase 12: Multi-surface UI integration tests.
//!
//! Tests that identity, cognitive context, and agent status are correctly
//! wired through the turn pipeline.

use std::sync::Arc;

use gyre::channels::StatusUpdate;
use gyre::cognitive::agent::CognitiveAgent;
use gyre::cognitive::{
    CognitiveContext, HermitBox, format_cognitive_prefix, prepare_cognitive_context,
};

// ─── Test: identity block appears in system prompt ─────────────────────────

#[test]
fn test_identity_in_system_prompt() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Create HermitBox with soul.md
    let hb = HermitBox::open(tmp.path(), "id_agent").expect("open HermitBox");
    std::fs::write(hb.soul_path(), "I am a multi-surface agent.\n").expect("write soul.md");
    drop(hb);

    // Open CognitiveAgent, which reloads identity from disk
    let agent = CognitiveAgent::open(tmp.path(), "id_agent").expect("open agent");

    // AgentIdentityFiles should contain the soul content
    let block = agent.identity.system_prompt_block();
    assert!(
        block.contains("multi-surface agent"),
        "system_prompt_block should include soul content, got:\n{}",
        block
    );
    assert!(
        block.contains("## Agent Identity"),
        "should have identity header"
    );

    // Identity should be cloneable (required for AgentDeps)
    let cloned = agent.identity.clone();
    assert_eq!(
        cloned.system_prompt_block(),
        block,
        "cloned identity should produce the same prompt"
    );
}

// ─── Test: cognitive context in turn pipeline ─────────────────────────────

#[test]
fn test_cognitive_in_turn() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let agent = CognitiveAgent::open(tmp.path(), "cog_turn_agent").expect("open agent");

    // Store some memories first
    agent.post_turn("I will implement the multi-surface pipeline for Gyre.");

    // Now prepare cognitive context for a related message
    let ctx = prepare_cognitive_context(&agent.context, "Tell me about the pipeline");
    let formatted = format_cognitive_prefix(&ctx);

    // Should contain the memory we stored
    assert!(
        !formatted.is_empty(),
        "cognitive prefix should not be empty when memories exist"
    );
    assert!(
        formatted.contains("## Cognitive Context"),
        "should have cognitive context header, got:\n{}",
        formatted
    );
    assert!(
        formatted.contains("Recent Memories"),
        "should have memories section"
    );

    // CognitiveContext should be cloneable (all Arc-wrapped)
    let cloned_ctx = agent.context.clone();
    let ctx2 = prepare_cognitive_context(&cloned_ctx, "pipeline");
    let formatted2 = format_cognitive_prefix(&ctx2);
    assert!(
        !formatted2.is_empty(),
        "cloned context should produce same cognitive prefix"
    );
}

// ─── Test: agent status summary ───────────────────────────────────────────

#[test]
fn test_cognitive_agent_status() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let agent = CognitiveAgent::open(tmp.path(), "status_agent").expect("open agent");

    // Before any memories
    let summary = agent.status_summary();
    assert!(
        summary.contains("agent:status_agent"),
        "summary should contain agent_id, got: {}",
        summary
    );
    assert!(
        summary.contains("memories:0"),
        "fresh agent should have 0 memories, got: {}",
        summary
    );
    assert!(
        summary.contains("queue:0"),
        "fresh agent should have 0 queue items, got: {}",
        summary
    );

    // After some memories
    agent.post_turn("I will optimize the rendering pipeline for the multi-surface UI.");
    let summary_after = agent.status_summary();
    assert!(
        !summary_after.contains("memories:0"),
        "after post_turn, memories should be > 0, got: {}",
        summary_after
    );
}

// ─── Test: StatusUpdate::AgentStatus variant ──────────────────────────────

#[test]
fn test_agent_status_variant() {
    let status = StatusUpdate::AgentStatus {
        agent_id: "kimi".to_string(),
        memory_count: 42,
        queue_depth: 3,
    };

    // Verify the variant can be constructed and pattern-matched
    match &status {
        StatusUpdate::AgentStatus {
            agent_id,
            memory_count,
            queue_depth,
        } => {
            assert_eq!(agent_id, "kimi");
            assert_eq!(*memory_count, 42);
            assert_eq!(*queue_depth, 3);
        }
        _ => panic!("expected AgentStatus variant"),
    }
}

// ─── Test: CognitiveContext from HermitBox ─────────────────────────────────

#[test]
fn test_cognitive_context_from_hermit_box() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let hb = HermitBox::open(tmp.path(), "ctx_test").expect("open HermitBox");

    let ctx = CognitiveContext::from_hermit_box(&hb);

    // Should be able to access memory_stream through the context
    let ms = ctx.memory_stream.lock().expect("lock memory_stream");
    let count = ms.count();
    assert_eq!(count, 0, "fresh context should have 0 memories");
}

// ─── Test: serve validation rejects bad paths ─────────────────────────────

#[test]
fn test_serve_rejects_blocked_paths() {
    // The validate_base_dir function is private, but we can verify
    // through the CognitiveAgent::open which also validates.
    // Test that system paths are rejected at the HermitBox level.
    let result = HermitBox::open(std::path::Path::new("/dev"), "test");
    assert!(result.is_err(), "opening HermitBox under /dev should fail");
}

// ─── Test: REPL fallback path (lightweight mode) ──────────────────────────

#[test]
fn test_repl_fallback_agent_creation() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Verify CognitiveAgent can be created and used for REPL mode
    let agent = CognitiveAgent::open(tmp.path(), "repl_agent").expect("open agent");
    let agent = Arc::new(agent);

    // CognitiveChannelBridge should work with the agent
    let bridge = gyre::cognitive::CognitiveChannelBridge::new(Arc::clone(&agent));

    // Fresh agent with no soul.md should produce empty system prompt
    let prompt = bridge.prepare_system_prompt();
    assert!(
        prompt.is_empty(),
        "fresh agent with no identity files should produce empty prompt, got:\n{}",
        prompt
    );

    // After writing a soul.md and reopening, prompt should be non-empty
    let hb = HermitBox::open(tmp.path(), "repl_agent").expect("reopen HermitBox");
    std::fs::write(hb.soul_path(), "I am a REPL test agent.\n").expect("write soul.md");
    drop(hb);

    let agent2 = CognitiveAgent::open(tmp.path(), "repl_agent").expect("reopen agent");
    let agent2 = Arc::new(agent2);
    let bridge2 = gyre::cognitive::CognitiveChannelBridge::new(agent2);
    let prompt2 = bridge2.prepare_system_prompt();
    assert!(
        prompt2.contains("REPL test agent"),
        "after writing soul.md, prompt should contain soul content, got:\n{}",
        prompt2
    );
}

// ─── Test: memory_stream.count() ──────────────────────────────────────────

#[test]
fn test_memory_stream_count() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let agent = CognitiveAgent::open(tmp.path(), "count_agent").expect("open agent");

    {
        let ms = agent.context.memory_stream.lock().expect("lock");
        assert_eq!(ms.count(), 0, "fresh memory stream should have count 0");
    }

    // Store a memory via post_turn
    agent.post_turn("I will design the architecture for multi-surface channels.");

    {
        let ms = agent.context.memory_stream.lock().expect("lock");
        assert!(
            ms.count() > 0,
            "after post_turn, count should be > 0, got: {}",
            ms.count()
        );
    }
}
