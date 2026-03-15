//! Phase 13 integration tests: TELOS, UOCS, 8 memory kinds, layered recall, learning loop.

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use gyre::cognitive::memory_stream::namespaces;
use gyre::cognitive::{
    AgentIdentityFiles, AutoMemoryConfig, HermitBox, LearningLoop, MemoryEntry, MemoryKind,
    MemoryStream, UocsWriter, extract_memories_from_turn,
};

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn memory_stream_in_memory() -> MemoryStream {
    MemoryStream::new(Path::new(":memory:")).expect("in-memory MemoryStream")
}

// ─── Test: telos directory created ──────────────────────────────────────────

#[test]
fn test_telos_dir_created() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let hb = HermitBox::open(tmp.path(), "telos_agent").expect("open box");

    let telos = hb.telos_dir();
    assert!(telos.is_dir(), "telos/ should exist");

    // All 6 telos files should exist
    for name in &[
        "MISSION.md",
        "GOALS.md",
        "BELIEFS.md",
        "EXPERIENCES.md",
        "BOUNDARIES.md",
        "SKILLS.md",
    ] {
        assert!(telos.join(name).exists(), "telos/{name} should exist");
    }
}

// ─── Test: telos in system prompt ───────────────────────────────────────────

#[test]
fn test_telos_in_system_prompt() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let hb = HermitBox::open(tmp.path(), "telos_prompt").expect("open box");

    // Write mission content
    std::fs::write(hb.telos_dir().join("MISSION.md"), "Protect the realm.\n")
        .expect("write MISSION.md");

    let identity = AgentIdentityFiles::load(&hb);
    assert!(
        identity.mission.contains("Protect the realm"),
        "mission should be loaded, got: {}",
        identity.mission
    );

    let block = identity.system_prompt_block();
    assert!(
        block.contains("## Agent Telos"),
        "system prompt should contain Telos section, got:\n{}",
        block
    );
    assert!(
        block.contains("### Mission"),
        "should contain Mission heading"
    );
    assert!(
        block.contains("Protect the realm"),
        "should contain mission content"
    );
}

// ─── Test: 8 memory kinds ───────────────────────────────────────────────────

#[test]
fn test_8_memory_kinds() {
    let config = AutoMemoryConfig::default();

    // Decision
    let memories = extract_memories_from_turn("We decided to use Rust for performance.", &config);
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].kind, MemoryKind::Decision);

    // Lesson
    let memories = extract_memories_from_turn("I learned that testing early saves time.", &config);
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].kind, MemoryKind::Lesson);

    // Commitment
    let memories = extract_memories_from_turn("I promised to deliver by Friday.", &config);
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].kind, MemoryKind::Commitment);

    // Preference
    let memories = extract_memories_from_turn("The user prefers dark mode.", &config);
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].kind, MemoryKind::Preference);

    // Handoff
    let memories = extract_memories_from_turn("Session ended after finishing the report.", &config);
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].kind, MemoryKind::Handoff);

    // Project
    let memories = extract_memories_from_turn("The project is ahead of schedule.", &config);
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].kind, MemoryKind::Project);

    // Observation (default fallback)
    let memories = extract_memories_from_turn("The weather is nice today.", &config);
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].kind, MemoryKind::Observation);
}

// ─── Test: commitment extraction with correct importance ────────────────────

#[test]
fn test_commitment_extraction() {
    let config = AutoMemoryConfig::default();
    let memories = extract_memories_from_turn("I promised to deliver by Friday.", &config);
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].kind, MemoryKind::Commitment);
    assert!(
        (memories[0].importance - 9.0).abs() < f32::EPSILON,
        "commitment importance should be 9.0, got {}",
        memories[0].importance
    );
}

// ─── Test: dual-write creates markdown files ────────────────────────────────

#[test]
fn test_dual_write() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let uocs = UocsWriter::new(tmp.path());

    let entry = MemoryEntry {
        id: Uuid::new_v4(),
        content: "We chose Postgres over MySQL.".to_string(),
        kind: MemoryKind::Decision,
        importance: 8.5,
        created_at: Utc::now(),
        namespace: namespaces::PERSONAL.to_string(),
    };

    uocs.write_memory(&entry).expect("write_memory");

    // Check that a file exists in memory/decisions/
    let decisions_dir = tmp.path().join("memory").join("decisions");
    assert!(decisions_dir.is_dir(), "decisions/ should exist");

    let files: Vec<_> = std::fs::read_dir(&decisions_dir)
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .collect();
    assert_eq!(files.len(), 1, "should have exactly 1 markdown file");

    let content = std::fs::read_to_string(files[0].path()).expect("read file");
    assert!(
        content.contains("type: decision"),
        "should contain frontmatter type"
    );
    assert!(
        content.contains("We chose Postgres"),
        "should contain memory content"
    );
}

// ─── Test: INDEX.md regeneration ────────────────────────────────────────────

#[test]
fn test_index_regenerated() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let uocs = UocsWriter::new(tmp.path());

    // Add 3 memories of different kinds
    let kinds = [
        (MemoryKind::Decision, "Chose Rust."),
        (MemoryKind::Lesson, "Learned about borrowing."),
        (MemoryKind::Commitment, "Will deliver by Monday."),
    ];

    for (kind, content) in &kinds {
        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            content: content.to_string(),
            kind: kind.clone(),
            importance: 7.0,
            created_at: Utc::now(),
            namespace: namespaces::PERSONAL.to_string(),
        };
        uocs.write_memory(&entry).expect("write_memory");
    }

    uocs.regenerate_index().expect("regenerate_index");

    let index_path = tmp.path().join("memory").join("INDEX.md");
    assert!(index_path.exists(), "INDEX.md should exist");

    let index_content = std::fs::read_to_string(&index_path).expect("read INDEX.md");
    assert!(
        index_content.contains("## Decisions"),
        "INDEX should contain Decisions section"
    );
    assert!(
        index_content.contains("## Lessons"),
        "INDEX should contain Lessons section"
    );
    assert!(
        index_content.contains("## Commitments"),
        "INDEX should contain Commitments section"
    );
}

// ─── Test: recall_layered hits index ────────────────────────────────────────

#[test]
fn test_recall_layered_hits_index() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let hb = HermitBox::open(tmp.path(), "layered_agent").expect("open box");

    // Create UOCS writer
    let uocs = Arc::new(UocsWriter::new(hb.box_dir()));

    // Add a memory to both SQLite and UOCS
    let entry = MemoryEntry {
        id: Uuid::new_v4(),
        content: "Database migration completed successfully.".to_string(),
        kind: MemoryKind::Decision,
        importance: 8.5,
        created_at: Utc::now(),
        namespace: namespaces::PERSONAL.to_string(),
    };

    {
        let ms = hb.memory_stream.lock().expect("lock ms");
        ms.add(&entry).expect("add memory");
    }
    uocs.write_memory(&entry).expect("write_memory");
    uocs.regenerate_index().expect("regenerate_index");

    // Set UOCS on memory stream and recall
    {
        let mut ms = hb.memory_stream.lock().expect("lock ms");
        ms.uocs = Some(Arc::clone(&uocs));
    }

    let results = {
        let ms = hb.memory_stream.lock().expect("lock ms");
        ms.recall_layered("database migration", 5)
            .expect("recall_layered")
    };

    assert!(
        !results.is_empty(),
        "recall_layered should return results for matching query"
    );
    assert!(
        results[0].content.contains("Database migration"),
        "first result should match query, got: {}",
        results[0].content
    );
}

// ─── Test: learning loop updates EXPERIENCES.md ─────────────────────────────

#[test]
fn test_learning_loop_updates_experiences() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let hb = HermitBox::open(tmp.path(), "learning_agent").expect("open box");

    let learning = LearningLoop::new(5);

    let lesson_entry = MemoryEntry {
        id: Uuid::new_v4(),
        content: "Always validate inputs before processing.".to_string(),
        kind: MemoryKind::Lesson,
        importance: 8.0,
        created_at: Utc::now(),
        namespace: namespaces::PERSONAL.to_string(),
    };

    learning.reflect(&hb, &[lesson_entry]).expect("reflect");

    let experiences = hb.read_telos_file("EXPERIENCES.md");
    assert!(
        experiences.contains("Always validate inputs"),
        "EXPERIENCES.md should contain lesson content, got: {}",
        experiences
    );
}

// ─── Test: handoff on session end ───────────────────────────────────────────

#[test]
fn test_handoff_on_session_end() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let hb = HermitBox::open(tmp.path(), "session_end_agent").expect("open box");

    let learning = LearningLoop::new(5);

    let memories = vec![
        MemoryEntry {
            id: Uuid::new_v4(),
            content: "Learned about async patterns.".to_string(),
            kind: MemoryKind::Lesson,
            importance: 8.0,
            created_at: Utc::now(),
            namespace: namespaces::PERSONAL.to_string(),
        },
        MemoryEntry {
            id: Uuid::new_v4(),
            content: "Committed to fix the auth bug by tomorrow.".to_string(),
            kind: MemoryKind::Commitment,
            importance: 9.0,
            created_at: Utc::now(),
            namespace: namespaces::PERSONAL.to_string(),
        },
    ];

    let handoff = learning
        .end_of_session_reflect(&hb, &memories)
        .expect("end_of_session_reflect");

    // Handoff memory should be created
    assert_eq!(handoff.kind, MemoryKind::Handoff);
    assert!((handoff.importance - 9.5).abs() < f32::EPSILON);
    assert!(handoff.content.contains("2 memories stored"));

    // EXPERIENCES.md should contain the lesson
    let experiences = hb.read_telos_file("EXPERIENCES.md");
    assert!(
        experiences.contains("async patterns"),
        "EXPERIENCES.md should contain lesson, got: {}",
        experiences
    );

    // GOALS.md should contain the commitment
    let goals = hb.read_telos_file("GOALS.md");
    assert!(
        goals.contains("fix the auth bug"),
        "GOALS.md should contain commitment, got: {}",
        goals
    );
}
