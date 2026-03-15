//! Integration tests for the Curiosity Engine (Phase 10).
#![cfg(feature = "test-support")]

use chrono::Utc;
use gyre::cognitive::curiosity::{
    CuriosityConfig, CuriosityEngine, KnowledgeGapDetector, ResearchQueue, TaskStatus,
};
use gyre::cognitive::knowledge_graph::EntityLayer;
use gyre::cognitive::memory_stream::namespaces;
use gyre::cognitive::{CognitiveAgent, KgEntity, KnowledgeGraph, MemoryEntry, MemoryKind};
use gyre::llm::MockLlmProvider;

// ─── Helper: create a CognitiveAgent in a temp dir ───────────────────────────

fn setup_chief(tmp: &std::path::Path) -> CognitiveAgent {
    let agent = CognitiveAgent::open(tmp, "chief-curiosity").expect("open CognitiveAgent");

    let ms = agent.context.memory_stream.lock().expect("lock ms");
    ms.add(&MemoryEntry {
        id: uuid::Uuid::new_v4(),
        content: "Tribe baseline: curiosity engine testing.".to_string(),
        kind: MemoryKind::Decision,
        importance: 6.0,
        created_at: Utc::now(),
        namespace: namespaces::TRIBE.to_string(),
    })
    .expect("add tribe memory");
    drop(ms);

    agent
}

// ─── Test 1: push 3 tasks with different priorities, pop returns highest ─────

#[test]
fn test_research_queue_push_pop() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let queue = ResearchQueue::new(&tmp.path().join("q.db")).expect("new queue");

    queue
        .push("low-priority-topic", 2.0, "test")
        .expect("push low");
    queue
        .push("high-priority-topic", 9.0, "test")
        .expect("push high");
    queue
        .push("medium-priority-topic", 5.0, "test")
        .expect("push medium");

    let next = queue.pop_next().expect("pop").expect("should have task");
    assert_eq!(next.topic, "high-priority-topic");
    assert!((next.priority - 9.0).abs() < f32::EPSILON);
    assert_eq!(next.status, TaskStatus::InProgress);
}

// ─── Test 2: push same topic twice, pending_count still == 1 ────────────────

#[test]
fn test_queue_dedup() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let queue = ResearchQueue::new(&tmp.path().join("q.db")).expect("new queue");

    queue
        .push("duplicate-topic", 5.0, "manual")
        .expect("push 1");
    queue
        .push("duplicate-topic", 8.0, "manual")
        .expect("push 2");

    assert_eq!(queue.pending_count().expect("count"), 1);
}

// ─── Test 3: mark 20 tasks done, run_cycle skips with daily limit ───────────

#[tokio::test]
async fn test_daily_limit() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let chief = setup_chief(tmp.path());

    let engine = CuriosityEngine::new(
        &tmp.path().join("curiosity_q.db"),
        CuriosityConfig {
            max_daily_tasks: 20,
            max_concurrent: 2,
            cycle_interval_secs: 3600,
        },
    )
    .expect("new engine");

    // Mark 20 tasks done to hit the daily limit
    for i in 0..20 {
        let id = engine
            .queue
            .push(&format!("daily-task-{i}"), 5.0, "test")
            .expect("push");
        let _ = engine.queue.pop_next();
        engine.queue.mark_done(id, Utc::now()).expect("mark_done");
    }

    // Now add a task that should not be processed
    engine
        .queue
        .push("should-not-run", 9.0, "test")
        .expect("push");

    let llm = MockLlmProvider::success("should not be called");
    let report = engine.run_cycle(&chief, &llm).await.expect("run_cycle");

    assert!(
        report.skipped_daily_limit,
        "cycle should skip due to daily limit"
    );
    assert!(report.task_processed.is_none());
}

// ─── Test 4: gap detector finds isolated entity ─────────────────────────────

#[test]
fn test_gap_detector_finds_isolated() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let kg = KnowledgeGraph::new(&tmp.path().join("kg.db")).expect("new KG");

    // Add high-importance entity with 0 edges
    let entity = KgEntity {
        id: uuid::Uuid::new_v4(),
        name: "IsolatedConcept".to_string(),
        layer: EntityLayer::Research,
        summary: "An important but isolated concept".to_string(),
        importance: 8.0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    kg.upsert_entity(&entity).expect("upsert");

    // Add low-importance entity (should NOT appear)
    let low = KgEntity {
        id: uuid::Uuid::new_v4(),
        name: "LowImportance".to_string(),
        layer: EntityLayer::Research,
        summary: "Not important".to_string(),
        importance: 3.0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    kg.upsert_entity(&low).expect("upsert low");

    let report = KnowledgeGapDetector::scan(&kg).expect("scan");
    assert!(
        !report.isolated_entities.is_empty(),
        "should find at least one isolated entity"
    );
    assert!(
        report
            .isolated_entities
            .iter()
            .any(|e| e.name == "IsolatedConcept"),
        "should find IsolatedConcept"
    );
    assert!(
        !report
            .isolated_entities
            .iter()
            .any(|e| e.name == "LowImportance"),
        "should NOT find LowImportance"
    );
}

// ─── Test 5: full cycle with MockLlmProvider ────────────────────────────────

#[tokio::test]
async fn test_full_cycle() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let chief = setup_chief(tmp.path());

    let engine = CuriosityEngine::new(
        &tmp.path().join("curiosity_q.db"),
        CuriosityConfig::default(),
    )
    .expect("new engine");

    // Seed the queue with a task
    engine
        .queue
        .push("Research quantum computing trends", 8.0, "manual")
        .expect("push");

    let llm = MockLlmProvider::success(
        "Quantum computing is advancing rapidly with multiple breakthroughs",
    );

    let report = engine.run_cycle(&chief, &llm).await.expect("run_cycle");

    assert!(!report.skipped_daily_limit);
    assert!(
        report.task_processed.is_some(),
        "should have processed a task"
    );

    let task = report.task_processed.as_ref().expect("task");
    assert_eq!(task.status, TaskStatus::Done);
    assert!(task.topic.contains("quantum computing"));
}

// ─── Test 6: manual task via explore CLI path ───────────────────────────────

#[test]
fn test_explore_add_manual() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let queue = ResearchQueue::new(&tmp.path().join("q.db")).expect("new queue");

    queue
        .push("Manual research about Rust async patterns", 8.0, "manual")
        .expect("push");

    let tasks = queue.peek(10).expect("peek");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].source, "manual");
    assert!((tasks[0].priority - 8.0).abs() < f32::EPSILON);
    assert!(tasks[0].topic.contains("Rust async"));
}
