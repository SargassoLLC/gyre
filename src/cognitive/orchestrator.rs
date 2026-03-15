//! Tribe orchestration: prepare Worker jobs from a Chief's cognitive context
//! and flow results back into the Chief's memory and knowledge graph.

use chrono::Utc;
use uuid::Uuid;

use crate::cognitive::agent::CognitiveAgent;
use crate::cognitive::auto_memory::sanitize_memory_content;
use crate::cognitive::distillation::{TribeContext, distill_for_worker};
use crate::cognitive::executor::WorkerExecutor;
use crate::cognitive::knowledge_graph::EntityLayer;
use crate::cognitive::memory_stream::{MemoryEntry, MemoryKind, namespaces};
use crate::llm::LlmProvider;

/// Status of a worker job.
pub enum WorkerJobStatus {
    Pending,
    Running,
    Completed(String),
    Failed(String),
}

/// A job prepared by the TribeOrchestrator for a Worker agent.
pub struct WorkerJob {
    pub job_id: String,
    pub task: String,
    pub tribe_context: TribeContext,
    pub status: WorkerJobStatus,
    /// Wall-clock duration of execution in milliseconds.
    pub duration_ms: Option<u64>,
    /// Total tokens consumed (input + output), if available.
    pub tokens_used: Option<u32>,
}

/// Orchestrates tribe delegation: prepares jobs from a Chief's context,
/// generates worker system prompts, and manages job lifecycle.
pub struct TribeOrchestrator {
    pub chief_id: String,
}

impl TribeOrchestrator {
    pub fn new(chief_id: &str) -> Self {
        Self {
            chief_id: chief_id.to_string(),
        }
    }

    /// Prepare a worker job by distilling the Chief's cognitive context for the task.
    pub fn prepare_job(chief: &CognitiveAgent, task: &str) -> WorkerJob {
        let tribe_context = distill_for_worker(&chief.context, task);
        WorkerJob {
            job_id: Uuid::new_v4().to_string(),
            task: task.to_string(),
            tribe_context,
            status: WorkerJobStatus::Pending,
            duration_ms: None,
            tokens_used: None,
        }
    }

    /// Generate the system prompt for a Worker agent executing this job.
    pub fn worker_system_prompt(job: &WorkerJob) -> String {
        let delegated = job.tribe_context.system_prompt_block();
        format!(
            "You are a Worker agent. Complete the following task and return ONLY your result.\n\n{}",
            delegated
        )
    }

    /// Mark a job as completed with a result.
    pub fn complete_job(job: &mut WorkerJob, result: String) {
        job.status = WorkerJobStatus::Completed(result);
    }

    /// Mark a job as failed with an error.
    pub fn fail_job(job: &mut WorkerJob, error: String) {
        job.status = WorkerJobStatus::Failed(error);
    }

    /// Execute a worker job end-to-end: prepare, run against LLM, store result.
    ///
    /// This is the primary high-level entry point for tribe delegation.
    /// It prepares the job, runs it against the LLM provider with a 60-second
    /// timeout, records metrics, stores the result in the Chief's memory and
    /// knowledge graph, and returns the completed job.
    pub async fn execute(
        chief: &CognitiveAgent,
        task: &str,
        llm: &dyn LlmProvider,
    ) -> Result<WorkerJob, String> {
        let start = std::time::Instant::now();
        let mut job = Self::prepare_job(chief, task);

        let result = WorkerExecutor::run(&job, llm, 60)
            .await
            .map_err(|e| e.to_string())?;

        let duration_ms = start.elapsed().as_millis() as u64;
        job.duration_ms = Some(duration_ms);

        Self::complete_job(&mut job, result);
        store_worker_result(chief, &job)?;

        Ok(job)
    }
}

/// Store a completed worker's result back into the Chief's memory and knowledge graph.
///
/// - Only works if `job.status` is `Completed`.
/// - Sanitizes the result through the injection defense layer before storage.
/// - Stores the result as a tribe-namespace memory entry.
/// - Extracts simple entity names from the sanitized result and upserts them into the KG.
pub fn store_worker_result(chief: &CognitiveAgent, job: &WorkerJob) -> Result<(), String> {
    let result_text = match &job.status {
        WorkerJobStatus::Completed(text) => text,
        _ => return Err("Cannot store result: job is not Completed".to_string()),
    };

    // Sanitize the worker result through injection defense before storing.
    // Worker results come from an external LLM and could contain injected content.
    let content_to_store = format!("Worker result for task \"{}\": {}", job.task, result_text);
    let sanitized_content = sanitize_memory_content(&content_to_store)
        .ok_or_else(|| "Worker result rejected by injection defense sanitizer".to_string())?;

    // Store as tribe-namespace memory entry
    let entry = MemoryEntry {
        id: Uuid::new_v4(),
        content: sanitized_content,
        kind: MemoryKind::Observation,
        importance: 7.0,
        created_at: Utc::now(),
        namespace: namespaces::TRIBE.to_string(),
    };

    let ms = chief
        .context
        .memory_stream
        .lock()
        .map_err(|e| format!("memory_stream lock poisoned: {e}"))?;
    ms.add(&entry)
        .map_err(|e| format!("memory write failed: {e}"))?;
    drop(ms);

    // Only scan the first 4 KB of result text for entity extraction.
    // Prevents CPU waste on very large worker outputs.
    const MAX_SCAN_BYTES: usize = 4096;
    let scan_text = if result_text.len() > MAX_SCAN_BYTES {
        let mut end = MAX_SCAN_BYTES;
        while !result_text.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        &result_text[..end]
    } else {
        result_text.as_str()
    };

    // Extract simple entity names: words > 5 chars, capped at 10 unique.
    // Cap prevents KG flooding from large worker results.
    // Entity names are also sanitized to prevent injection via KG entries.
    const MAX_ENTITIES_PER_RESULT: usize = 10;
    const MAX_ENTITY_NAME_BYTES: usize = 128;
    let mut seen = std::collections::HashSet::new();
    let entities: Vec<&str> = scan_text
        .split_whitespace()
        .filter(|w| w.len() > 5 && w.len() <= MAX_ENTITY_NAME_BYTES && seen.insert(*w))
        .filter(|w| {
            w.chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        })
        .take(MAX_ENTITIES_PER_RESULT)
        .collect();

    let kg = chief
        .context
        .knowledge_graph
        .lock()
        .map_err(|e| format!("knowledge_graph lock poisoned: {e}"))?;

    for name in entities {
        let _ = kg.upsert_by_name(name, EntityLayer::Research, "", 5.0);
    }

    Ok(())
}
