use gyre::cognitive::CognitiveAgent;

// ─── Test: CognitiveAgent::open succeeds ──────────────────────────────────

#[test]
fn test_cognitive_agent_open() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let agent = CognitiveAgent::open(tmp.path(), "test_agent").expect("open CognitiveAgent");

    assert_eq!(agent.agent_id(), "test_agent");

    // Identity should be populated (empty strings since no files written yet)
    assert!(agent.identity.soul.is_empty());
    assert!(agent.identity.user_context.is_empty());
    assert!(agent.identity.memory_summary.is_empty());

    // Context subsystems should be accessible
    let ms = agent
        .context
        .memory_stream
        .lock()
        .expect("lock memory_stream");
    let recalled = ms.recall("", 10).expect("recall");
    assert!(recalled.is_empty(), "fresh agent should have no memories");
}

// ─── Test: system_prompt_prefix contains soul content ─────────────────────

#[test]
fn test_cognitive_agent_system_prompt() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // First, create the box directory so we can write soul.md
    let hb = gyre::cognitive::HermitBox::open(tmp.path(), "soul_agent").expect("open HermitBox");
    std::fs::write(hb.soul_path(), "I am a focused research agent.\n").expect("write soul.md");
    drop(hb);

    // Now open as CognitiveAgent (which reloads identity from disk)
    let agent = CognitiveAgent::open(tmp.path(), "soul_agent").expect("open CognitiveAgent");

    let prefix = agent.system_prompt_prefix();
    assert!(
        prefix.contains("focused research agent"),
        "system_prompt_prefix should contain soul content, got:\n{}",
        prefix
    );
    assert!(
        prefix.contains("## Agent Identity"),
        "should have identity header, got:\n{}",
        prefix
    );
}

// ─── Test: post_turn stores memories ──────────────────────────────────────

#[test]
fn test_cognitive_agent_post_turn() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let agent = CognitiveAgent::open(tmp.path(), "post_turn_agent").expect("open CognitiveAgent");

    // Before post_turn, memory stream is empty
    {
        let ms = agent.context.memory_stream.lock().expect("lock");
        let before = ms.recall("", 100).expect("recall before");
        assert!(before.is_empty(), "should start empty");
    }

    // post_turn with a planning-type message
    agent.post_turn("I will implement Phase 7 of the cognitive layer.");

    // After post_turn, should have at least one memory
    {
        let ms = agent.context.memory_stream.lock().expect("lock");
        let after = ms.recall("", 100).expect("recall after");
        assert!(
            !after.is_empty(),
            "should have at least one memory after post_turn"
        );
        assert!(
            after.iter().any(|m| m.content.contains("Phase 7")),
            "should contain the planning intent, entries: {:?}",
            after.iter().map(|m| &m.content).collect::<Vec<_>>()
        );
    }
}

// ─── Test: memory persists across open/close ─────────────────────────────

#[test]
fn test_cognitive_agent_memory_persists() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Open agent, post a turn, drop it
    {
        let agent = CognitiveAgent::open(tmp.path(), "persist_agent").expect("open CognitiveAgent");
        agent.post_turn("I will implement Phase 7 of the cognitive layer.");

        // Verify memory was stored
        let ms = agent.context.memory_stream.lock().expect("lock");
        let recalled = ms.recall("", 100).expect("recall");
        assert!(!recalled.is_empty(), "should have memories before closing");
    }

    // Reopen the same agent and verify memories are still there
    {
        let agent =
            CognitiveAgent::open(tmp.path(), "persist_agent").expect("reopen CognitiveAgent");
        let ms = agent.context.memory_stream.lock().expect("lock");
        let recalled = ms.recall("", 5).expect("recall after reopen");
        assert!(
            !recalled.is_empty(),
            "memories should persist across open/close"
        );
        assert!(
            recalled.iter().any(|m| m.content.contains("Phase 7")),
            "should find the previously stored memory"
        );
    }
}

// ─── Test: rejects bad agent IDs ─────────────────────────────────────────

#[test]
fn test_cognitive_agent_rejects_bad_id() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Path traversal
    let result = CognitiveAgent::open(tmp.path(), "../evil");
    assert!(result.is_err(), "should reject path traversal in agent_id");
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("expected error"),
    };
    assert!(
        err.contains(".."),
        "error should mention '..', got: {}",
        err
    );

    // Slash in agent_id
    let result = CognitiveAgent::open(tmp.path(), "foo/bar");
    assert!(result.is_err(), "should reject slash in agent_id");

    // Empty agent_id
    let result = CognitiveAgent::open(tmp.path(), "");
    assert!(result.is_err(), "should reject empty agent_id");

    // Too long agent_id (>64 chars)
    let long_id = "a".repeat(65);
    let result = CognitiveAgent::open(tmp.path(), &long_id);
    assert!(
        result.is_err(),
        "should reject agent_id longer than 64 chars"
    );
}
