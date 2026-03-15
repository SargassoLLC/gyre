//! End-to-end integration test: message in → auto_memory → response.
//!
//! Proves the full cognitive loop works without a database, channels, or real LLM.
//! Uses CognitiveAgent + MockLlmProvider + tempdir.

use gyre::cognitive::{CognitiveAgent, HermitBox, MemoryKind};

// ─── Helper: create an agent with a soul.md written ──────────────────────

fn setup_agent(base_dir: &std::path::Path, agent_id: &str) -> CognitiveAgent {
    // Create HermitBox first so we can write identity files
    let hb = HermitBox::open(base_dir, agent_id).expect("open HermitBox");
    std::fs::write(
        hb.soul_path(),
        "I am Gyre, a secure personal AI assistant.\n",
    )
    .expect("write soul.md");
    drop(hb);

    // Open as CognitiveAgent (reloads identity from disk)
    CognitiveAgent::open(base_dir, agent_id).expect("open CognitiveAgent")
}

// ─── Layer 1: User message → post_turn → memory stored ──────────────────

#[test]
fn e2e_message_stores_memory_via_post_turn() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let agent = setup_agent(tmp.path(), "e2e_agent");

    // Verify clean slate
    {
        let ms = agent.context.memory_stream.lock().expect("lock");
        assert!(
            ms.recall("", 100).expect("recall").is_empty(),
            "should start with no memories"
        );
    }

    // Simulate: user says "Remember that the deployment deadline is March 30th"
    // Agent (mock) responds with a commitment-pattern sentence so auto_memory extracts it.
    let assistant_response =
        "Understood. I committed to tracking the deployment deadline of March 30th.";

    agent.post_turn(assistant_response);

    // Verify memory was stored
    let memories = {
        let ms = agent.context.memory_stream.lock().expect("lock");
        ms.recall("", 100).expect("recall after post_turn")
    };

    assert!(
        !memories.is_empty(),
        "post_turn should have stored at least one memory"
    );
    assert!(
        memories.iter().any(|m| m.content.contains("March 30th")),
        "stored memory should contain the deadline, got: {:?}",
        memories.iter().map(|m| &m.content).collect::<Vec<_>>()
    );
    assert!(
        memories
            .iter()
            .any(|m| matches!(m.kind, MemoryKind::Commitment)),
        "memory kind should be Commitment (triggered by 'committed to')"
    );
}

// ─── Layer 2: Recalled memory contains relevant content ──────────────────

#[test]
fn e2e_recalled_memory_contains_original_content() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let agent = setup_agent(tmp.path(), "recall_agent");

    // Store a decision memory
    agent.post_turn("We decided to use PostgreSQL for the production database.");

    // Recall and verify content fidelity
    let memories = {
        let ms = agent.context.memory_stream.lock().expect("lock");
        ms.recall("PostgreSQL", 10).expect("recall")
    };

    assert_eq!(memories.len(), 1, "should have exactly one memory");
    assert!(
        memories[0].content.contains("PostgreSQL"),
        "recalled content should contain 'PostgreSQL'"
    );
    assert!(
        memories[0].content.contains("production database"),
        "recalled content should contain 'production database'"
    );
    assert!(matches!(memories[0].kind, MemoryKind::Decision));
}

// ─── Layer 3: Two turns accumulate memories ──────────────────────────────

#[test]
fn e2e_two_turns_accumulate_memories() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let agent = setup_agent(tmp.path(), "multi_turn_agent");

    // Turn 1: user asks about deadline, agent responds with commitment
    let turn1_response = "I committed to tracking the deployment deadline of March 30th.";
    agent.post_turn(turn1_response);

    let count_after_turn1 = {
        let ms = agent.context.memory_stream.lock().expect("lock");
        ms.recall("", 100).expect("recall").len()
    };
    assert!(
        count_after_turn1 >= 1,
        "should have at least 1 memory after turn 1"
    );

    // Turn 2: user asks when deadline is, agent responds with a decision
    let turn2_response =
        "We decided to move the deadline to April 5th based on the new requirements.";
    agent.post_turn(turn2_response);

    let memories_after_turn2 = {
        let ms = agent.context.memory_stream.lock().expect("lock");
        ms.recall("", 100).expect("recall")
    };

    assert!(
        memories_after_turn2.len() > count_after_turn1,
        "turn 2 should add more memories (had {}, now {})",
        count_after_turn1,
        memories_after_turn2.len()
    );

    // Both turns' content should be present
    let all_content: String = memories_after_turn2
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    assert!(
        all_content.contains("March 30th"),
        "should still have turn 1 content"
    );
    assert!(
        all_content.contains("April 5th"),
        "should have turn 2 content"
    );
}

// ─── Layer 4: System prompt includes identity ────────────────────────────

#[test]
fn e2e_system_prompt_includes_identity() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let agent = setup_agent(tmp.path(), "prompt_agent");

    let prefix = agent.system_prompt_prefix();

    assert!(
        prefix.contains("secure personal AI assistant"),
        "system prompt should include soul content, got:\n{}",
        prefix
    );
    assert!(
        prefix.contains("## Agent Identity"),
        "system prompt should have identity header"
    );
}

// ─── Layer 5: Memory persists across agent restarts ──────────────────────

#[test]
fn e2e_memory_survives_agent_restart() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // First session: store a memory
    {
        let agent = setup_agent(tmp.path(), "restart_agent");
        agent.post_turn("I promised to deliver the security audit by Friday.");
    }

    // Second session: reopen and verify memory is still there
    {
        let agent =
            CognitiveAgent::open(tmp.path(), "restart_agent").expect("reopen CognitiveAgent");

        let memories = {
            let ms = agent.context.memory_stream.lock().expect("lock");
            ms.recall("", 100).expect("recall after restart")
        };

        assert!(
            !memories.is_empty(),
            "memories should survive agent restart"
        );
        assert!(
            memories
                .iter()
                .any(|m| m.content.contains("security audit")),
            "should find the previously stored memory"
        );
    }
}

// ─── Layer 6: Full loop — identity + message + memory + recall ───────────

#[test]
fn e2e_full_cognitive_loop() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let agent = setup_agent(tmp.path(), "full_loop_agent");

    // 1. Verify identity loads into system prompt
    let prefix = agent.system_prompt_prefix();
    assert!(prefix.contains("Gyre"), "identity should be loaded");

    // 2. Simulate turn 1: user message about deadline
    //    Mock assistant response contains commitment keyword
    agent.post_turn(
        "I committed to ensuring the deployment deadline of March 30th is tracked and met.",
    );

    // 3. Verify memory was extracted and stored
    {
        let ms = agent.context.memory_stream.lock().expect("lock");
        let recalled = ms.recall("deployment deadline", 10).expect("recall");
        assert!(
            recalled.iter().any(|m| m.content.contains("March 30th")),
            "should recall the deadline from turn 1"
        );
    }

    // 4. Simulate turn 2: user asks about deadline, agent confirms
    agent.post_turn(
        "We decided to keep the March 30th deadline. The team selected a phased rollout strategy.",
    );

    // 5. Verify both memories are present
    {
        let ms = agent.context.memory_stream.lock().expect("lock");
        let all = ms.recall("", 100).expect("recall all");
        assert!(
            all.len() >= 2,
            "should have memories from both turns, got {}",
            all.len()
        );

        let content_joined: String = all
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join(" | ");

        assert!(
            content_joined.contains("March 30th"),
            "should contain deadline info across turns"
        );
        assert!(
            content_joined.contains("decided") || content_joined.contains("selected"),
            "should contain decision from turn 2"
        );
    }
}
