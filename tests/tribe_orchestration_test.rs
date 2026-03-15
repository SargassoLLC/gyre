use gyre::cognitive::memory_stream::namespaces;
use gyre::cognitive::orchestrator::{TribeOrchestrator, WorkerJobStatus, store_worker_result};
use gyre::cognitive::{CognitiveAgent, HermitBox, MemoryEntry, MemoryKind};

// ─── Helper: create a CognitiveAgent in a temp dir with some tribe memories ───

fn setup_chief(tmp: &std::path::Path) -> CognitiveAgent {
    let agent = CognitiveAgent::open(tmp, "chief-alpha").expect("open CognitiveAgent");

    // Seed a tribe-namespace memory so distillation has something to pull
    let ms = agent.context.memory_stream.lock().expect("lock ms");
    ms.add(&MemoryEntry {
        id: uuid::Uuid::new_v4(),
        content: "Previous tribe finding: topic analysis complete.".to_string(),
        kind: MemoryKind::Decision,
        importance: 6.0,
        created_at: chrono::Utc::now(),
        namespace: namespaces::TRIBE.to_string(),
    })
    .expect("add tribe memory");
    drop(ms);

    agent
}

// ─── Test: prepare_job produces valid WorkerJob with tribe context ───────────

#[test]
fn test_prepare_job() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let chief = setup_chief(tmp.path());

    let job = TribeOrchestrator::prepare_job(&chief, "Analyze the competitor landscape");

    // tribe_context should be non-empty (at least the task hint)
    let prompt = job.tribe_context.system_prompt_block();
    assert!(
        !prompt.is_empty(),
        "tribe_context system_prompt_block should not be empty"
    );
    assert!(
        prompt.contains("Analyze the competitor landscape"),
        "should contain the task, got:\n{}",
        prompt
    );

    // worker_system_prompt should contain Worker agent header
    let worker_prompt = TribeOrchestrator::worker_system_prompt(&job);
    assert!(
        worker_prompt.contains("Worker agent"),
        "worker_system_prompt should mention 'Worker agent', got:\n{}",
        worker_prompt
    );

    // Status should be Pending
    assert!(
        matches!(job.status, WorkerJobStatus::Pending),
        "initial status should be Pending"
    );
}

// ─── Test: complete_job + store_worker_result stores tribe memory ────────────

#[test]
fn test_complete_job_stores_memory() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let chief = setup_chief(tmp.path());

    let mut job = TribeOrchestrator::prepare_job(&chief, "Summarize market trends");
    TribeOrchestrator::complete_job(
        &mut job,
        "Market trends indicate growth in AI sector".to_string(),
    );

    store_worker_result(&chief, &job).expect("store_worker_result");

    // Verify: Chief's memory_stream should have a tribe-namespace entry with result
    let ms = chief.context.memory_stream.lock().expect("lock ms");
    let tribe_memories = ms
        .recall_in_namespace(namespaces::TRIBE, 100)
        .expect("recall tribe");

    let has_result = tribe_memories
        .iter()
        .any(|m| m.content.contains("Market trends indicate growth"));
    assert!(
        has_result,
        "tribe namespace should contain the worker result, entries: {:?}",
        tribe_memories
            .iter()
            .map(|m| &m.content)
            .collect::<Vec<_>>()
    );
}

// ─── Test: failed job is not stored ─────────────────────────────────────────

#[test]
fn test_failed_job_not_stored() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let chief = setup_chief(tmp.path());

    let mut job = TribeOrchestrator::prepare_job(&chief, "This will fail");
    TribeOrchestrator::fail_job(&mut job, "timeout".to_string());

    let result = store_worker_result(&chief, &job);
    assert!(
        result.is_err(),
        "store_worker_result should return Err for failed jobs"
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("not Completed"),
        "error should mention not Completed, got: {}",
        err
    );
}

// ─── Test: KG upsert from result text ──────────────────────────────────────

#[test]
fn test_kg_upsert_from_result() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let chief = setup_chief(tmp.path());

    let mut job = TribeOrchestrator::prepare_job(&chief, "Investigate something");
    TribeOrchestrator::complete_job(
        &mut job,
        "CompetitorA launched ProductX targeting SegmentB with PricingC".to_string(),
    );

    store_worker_result(&chief, &job).expect("store_worker_result");

    // Verify: KG should have new entities from words > 5 chars
    let kg = chief.context.knowledge_graph.lock().expect("lock kg");
    let count = kg.entity_count().expect("entity_count");
    assert!(
        count > 0,
        "KG should have entities after storing worker result"
    );

    // Check that at least one of the extracted words is in the KG
    let results = kg.search_by_name("CompetitorA", 5).expect("search");
    assert!(
        !results.is_empty(),
        "KG should contain 'CompetitorA' entity"
    );
}

// ─── Test: worker tribe_context only contains tribe memories ────────────────

#[test]
fn test_worker_cannot_access_personal() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Create agent with both personal and tribe memories
    let hb = HermitBox::open(tmp.path(), "chief-secure").expect("open HermitBox");

    let ms = hb.memory_stream.lock().expect("lock ms");

    // Add a personal memory
    ms.add(&MemoryEntry {
        id: uuid::Uuid::new_v4(),
        content: "SECRET: my personal banking details are XYZ".to_string(),
        kind: MemoryKind::Observation,
        importance: 9.0,
        created_at: chrono::Utc::now(),
        namespace: namespaces::PERSONAL.to_string(),
    })
    .expect("add personal memory");

    // Add a tribe memory
    ms.add(&MemoryEntry {
        id: uuid::Uuid::new_v4(),
        content: "Shared tribe finding: market is growing".to_string(),
        kind: MemoryKind::Decision,
        importance: 6.0,
        created_at: chrono::Utc::now(),
        namespace: namespaces::TRIBE.to_string(),
    })
    .expect("add tribe memory");
    drop(ms);
    drop(hb);

    let agent = CognitiveAgent::open(tmp.path(), "chief-secure").expect("open agent");
    let job = TribeOrchestrator::prepare_job(&agent, "Do some research");

    // The tribe_context memories should only be from tribe namespace
    for mem in &job.tribe_context.memories {
        assert_eq!(
            mem.namespace,
            namespaces::TRIBE,
            "worker tribe_context should only have tribe-namespace memories, found: {} in namespace '{}'",
            mem.content,
            mem.namespace
        );
    }

    // Verify personal data doesn't appear in the worker system prompt
    let worker_prompt = TribeOrchestrator::worker_system_prompt(&job);
    assert!(
        !worker_prompt.contains("SECRET"),
        "worker system prompt must NOT contain personal memories, got:\n{}",
        worker_prompt
    );
    assert!(
        !worker_prompt.contains("banking"),
        "worker system prompt must NOT leak personal data"
    );
}
