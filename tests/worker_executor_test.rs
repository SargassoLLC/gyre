//! Integration tests for WorkerExecutor and TribeOrchestrator::execute().
#![cfg(feature = "test-support")]

use gyre::cognitive::executor::{ExecutorError, WorkerExecutor};
use gyre::cognitive::memory_stream::namespaces;
use gyre::cognitive::orchestrator::{TribeOrchestrator, WorkerJobStatus};
use gyre::cognitive::{CognitiveAgent, MemoryEntry, MemoryKind};
use gyre::llm::MockLlmProvider;

// ─── Helper: create a CognitiveAgent in a temp dir with tribe memories ───────

fn setup_chief(tmp: &std::path::Path) -> CognitiveAgent {
    let agent = CognitiveAgent::open(tmp, "chief-exec").expect("open CognitiveAgent");

    let ms = agent.context.memory_stream.lock().expect("lock ms");
    ms.add(&MemoryEntry {
        id: uuid::Uuid::new_v4(),
        content: "Previous tribe finding: baseline analysis complete.".to_string(),
        kind: MemoryKind::Decision,
        importance: 6.0,
        created_at: chrono::Utc::now(),
        namespace: namespaces::TRIBE.to_string(),
    })
    .expect("add tribe memory");
    drop(ms);

    agent
}

// ─── Test: executor returns Ok on success ────────────────────────────────────

#[tokio::test]
async fn test_executor_success() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let chief = setup_chief(tmp.path());

    let job = TribeOrchestrator::prepare_job(&chief, "Summarize findings");
    let llm = MockLlmProvider::success("Worker result here");

    let result = WorkerExecutor::run(&job, &llm, 10).await;
    assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
    assert_eq!(result.unwrap(), "Worker result here");
}

// ─── Test: executor returns Timeout on slow LLM ─────────────────────────────

#[tokio::test]
async fn test_executor_timeout() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let chief = setup_chief(tmp.path());

    let job = TribeOrchestrator::prepare_job(&chief, "Slow task");
    let llm = MockLlmProvider::slow("response", 5000);

    let result = WorkerExecutor::run(&job, &llm, 1).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, ExecutorError::Timeout),
        "expected Timeout, got: {}",
        err
    );
}

// ─── Test: executor returns LlmError on failure ─────────────────────────────

#[tokio::test]
async fn test_executor_llm_failure() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let chief = setup_chief(tmp.path());

    let job = TribeOrchestrator::prepare_job(&chief, "Failing task");
    let llm = MockLlmProvider::failing();

    let result = WorkerExecutor::run(&job, &llm, 10).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, ExecutorError::LlmError(_)),
        "expected LlmError, got: {}",
        err
    );
}

// ─── Test: TribeOrchestrator::execute full cycle with metrics ────────────────

#[tokio::test]
async fn test_execute_full_cycle() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let chief = setup_chief(tmp.path());
    let llm = MockLlmProvider::success("Full cycle result with details");

    let job = TribeOrchestrator::execute(&chief, "Full cycle task", &llm)
        .await
        .expect("execute should succeed");

    // Verify metrics are populated (mock LLM is instant so duration may be 0)
    assert!(
        job.duration_ms.is_some(),
        "duration_ms should be populated after execute"
    );

    // Verify job completed
    assert!(
        matches!(job.status, WorkerJobStatus::Completed(ref t) if t == "Full cycle result with details"),
        "job should be Completed with the LLM response"
    );

    // Verify tribe memory was stored
    let ms = chief.context.memory_stream.lock().expect("lock ms");
    let tribe_memories = ms
        .recall_in_namespace(namespaces::TRIBE, 100)
        .expect("recall tribe");
    let has_result = tribe_memories
        .iter()
        .any(|m| m.content.contains("Full cycle result"));
    assert!(has_result, "tribe memory should contain the worker result");
}

// ─── Test: execute stores KG entities from multi-word response ───────────────

#[tokio::test]
async fn test_execute_stores_kg_entities() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let chief = setup_chief(tmp.path());
    let llm = MockLlmProvider::success(
        "CompetitorAlpha launched ProductBeta targeting SegmentGamma with PricingDelta",
    );

    TribeOrchestrator::execute(&chief, "Research competitors", &llm)
        .await
        .expect("execute should succeed");

    // Verify KG has entities from the response (words > 5 chars)
    let kg = chief.context.knowledge_graph.lock().expect("lock kg");
    let count = kg.entity_count().expect("entity_count");
    assert!(
        count > 0,
        "KG should have entities after execute with multi-word response"
    );

    // Check specific entity
    let results = kg.search_by_name("CompetitorAlpha", 5).expect("search");
    assert!(
        !results.is_empty(),
        "KG should contain 'CompetitorAlpha' entity"
    );
}
