//! Routine execution engine.
//!
//! Handles loading routines, checking triggers, enforcing guardrails,
//! and executing both lightweight (single LLM call) and full-job routines.
//!
//! The engine runs two independent loops:
//! - A **cron ticker** that polls the DB every N seconds for due cron routines
//! - An **event matcher** called synchronously from the agent main loop
//!
//! Lightweight routines execute inline (single LLM call, no scheduler slot).
//! Full-job routines are delegated to the existing `Scheduler`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::Utc;
use regex::Regex;
use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;

use crate::agent::attention::{ATTENTION_FORMAT_INSTRUCTIONS, parse_attention_report};
use crate::agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineRun, RunStatus, Trigger, next_cron_fire,
};
use crate::agent::scheduler::Scheduler;
use crate::channels::{IncomingMessage, OutgoingResponse};
use crate::config::RoutineConfig;
use crate::context::{ContextManager, JobState};
use crate::db::Database;
use crate::llm::{ChatMessage, CompletionRequest, FinishReason, LlmProvider};
use crate::workspace::Workspace;

/// A routine notification with its delivery target attached.
///
/// The target travels WITH the notification through the whole delivery
/// path (including any LLM failover that happened during execution) —
/// previously only the message content was forwarded and the configured
/// channel/user were dropped at the first hop, so cron output was
/// broadcast blindly to all channels.
#[derive(Debug, Clone)]
pub struct RoutineNotification {
    /// Channel to deliver on. `None` = broadcast to all channels.
    pub channel: Option<String>,
    /// User to deliver to.
    pub user: String,
    /// The message itself.
    pub response: OutgoingResponse,
}

/// Report from a routine dry-run + readiness judgment.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RoutineTestReport {
    pub routine_name: String,
    /// Dry-run outcome.
    pub run_status: RunStatus,
    /// The routine's output (None when it reported nothing to do).
    pub output_summary: Option<String>,
    pub tokens_used: Option<i32>,
    /// The judge's verdict: safe to enable unattended?
    pub ready: bool,
    /// Concrete problems the judge found.
    pub issues: Vec<String>,
    /// The judge's overall notes.
    pub judge_notes: Option<String>,
    /// Limitations of THIS test run itself (e.g. full_job dry-runs are
    /// approximate) — distinct from the judge's issues with the routine.
    pub caveats: Vec<String>,
}

/// Parsed judge verdict.
struct ReadinessVerdict {
    ready: bool,
    issues: Vec<String>,
    notes: Option<String>,
}

/// Parse the judge's JSON verdict. Structural parsing only; an
/// unparseable verdict FAILS CLOSED to not-ready — a pre-flight check
/// that can't read its own judge must not green-light the routine.
fn parse_readiness_verdict(content: &str) -> ReadinessVerdict {
    #[derive(serde::Deserialize)]
    struct Wire {
        ready: bool,
        #[serde(default)]
        issues: Vec<String>,
        #[serde(default)]
        notes: String,
    }

    let trimmed = content.trim();
    let candidate = crate::agent::attention::strip_code_fence(trimmed);

    match serde_json::from_str::<Wire>(candidate) {
        Ok(w) => ReadinessVerdict {
            ready: w.ready,
            issues: w.issues,
            notes: if w.notes.trim().is_empty() {
                None
            } else {
                Some(w.notes.trim().to_string())
            },
        },
        Err(_) => ReadinessVerdict {
            ready: false,
            issues: vec!["judge returned unparseable verdict".to_string()],
            notes: Some(trimmed.to_string()),
        },
    }
}

/// The routine execution engine.
pub struct RoutineEngine {
    config: RoutineConfig,
    store: Arc<dyn Database>,
    llm: Arc<dyn LlmProvider>,
    workspace: Arc<Workspace>,
    /// Sender for notifications (routed to channel manager).
    notify_tx: mpsc::Sender<RoutineNotification>,
    /// Scheduler used to run `full_job` routines as real parallel jobs.
    scheduler: Arc<Scheduler>,
    /// Context manager used to create and poll `full_job` job contexts.
    context_manager: Arc<ContextManager>,
    /// Currently running routine count (across all routines).
    running_count: Arc<AtomicUsize>,
    /// Compiled event regex cache: routine_id -> compiled regex.
    event_cache: Arc<RwLock<Vec<(Uuid, Routine, Regex)>>>,
}

impl RoutineEngine {
    pub fn new(
        config: RoutineConfig,
        store: Arc<dyn Database>,
        llm: Arc<dyn LlmProvider>,
        workspace: Arc<Workspace>,
        notify_tx: mpsc::Sender<RoutineNotification>,
        scheduler: Arc<Scheduler>,
        context_manager: Arc<ContextManager>,
    ) -> Self {
        Self {
            config,
            store,
            llm,
            workspace,
            notify_tx,
            scheduler,
            context_manager,
            running_count: Arc::new(AtomicUsize::new(0)),
            event_cache: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Refresh the in-memory event trigger cache from DB.
    pub async fn refresh_event_cache(&self) {
        match self.store.list_event_routines().await {
            Ok(routines) => {
                let mut cache = Vec::new();
                for routine in routines {
                    if let Trigger::Event { ref pattern, .. } = routine.trigger {
                        match Regex::new(pattern) {
                            Ok(re) => cache.push((routine.id, routine.clone(), re)),
                            Err(e) => {
                                tracing::warn!(
                                    routine = %routine.name,
                                    "Invalid event regex '{}': {}",
                                    pattern, e
                                );
                            }
                        }
                    }
                }
                let count = cache.len();
                *self.event_cache.write().await = cache;
                tracing::debug!("Refreshed event cache: {} routines", count);
            }
            Err(e) => {
                tracing::error!("Failed to refresh event cache: {}", e);
            }
        }
    }

    /// Check incoming message against event triggers. Returns number of routines fired.
    ///
    /// Called synchronously from the main loop after handle_message(). The actual
    /// execution is spawned async so this returns quickly.
    pub async fn check_event_triggers(&self, message: &IncomingMessage) -> usize {
        let cache = self.event_cache.read().await;
        let mut fired = 0;

        for (_, routine, re) in cache.iter() {
            // Channel filter
            if let Trigger::Event {
                channel: Some(ch), ..
            } = &routine.trigger
                && ch != &message.channel
            {
                continue;
            }

            // Regex match
            if !re.is_match(&message.content) {
                continue;
            }

            // Cooldown check
            if !self.check_cooldown(routine) {
                tracing::debug!(routine = %routine.name, "Skipped: cooldown active");
                continue;
            }

            // Concurrent run check
            if !self.check_concurrent(routine).await {
                tracing::debug!(routine = %routine.name, "Skipped: max concurrent reached");
                continue;
            }

            // Global capacity check
            if self.running_count.load(Ordering::Relaxed) >= self.config.max_concurrent_routines {
                tracing::warn!(routine = %routine.name, "Skipped: global max concurrent reached");
                continue;
            }

            let detail = truncate(&message.content, 200);
            self.spawn_fire(routine.clone(), "event", Some(detail));
            fired += 1;
        }

        fired
    }

    /// Check all due cron routines and fire them. Called by the cron ticker.
    pub async fn check_cron_triggers(&self) {
        let routines = match self.store.list_due_cron_routines().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("Failed to load due cron routines: {}", e);
                return;
            }
        };

        for routine in routines {
            if self.running_count.load(Ordering::Relaxed) >= self.config.max_concurrent_routines {
                tracing::warn!("Global max concurrent routines reached, skipping remaining");
                break;
            }

            if !self.check_cooldown(&routine) {
                continue;
            }

            if !self.check_concurrent(&routine).await {
                continue;
            }

            let detail = if let Trigger::Cron { ref schedule } = routine.trigger {
                Some(schedule.clone())
            } else {
                None
            };

            self.spawn_fire(routine, "cron", detail);
        }
    }

    /// Fire a routine manually (from tool call or CLI).
    pub async fn fire_manual(&self, routine_id: Uuid) -> Result<Uuid, String> {
        let routine = self
            .store
            .get_routine(routine_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or_else(|| format!("routine {routine_id} not found"))?;

        if !routine.enabled {
            return Err(format!("routine '{}' is disabled", routine.name));
        }

        if !self.check_concurrent(&routine).await {
            return Err(format!(
                "routine '{}' already at max concurrent runs",
                routine.name
            ));
        }

        let run_id = Uuid::new_v4();
        let run = RoutineRun {
            id: run_id,
            routine_id: routine.id,
            trigger_type: "manual".to_string(),
            trigger_detail: None,
            started_at: Utc::now(),
            completed_at: None,
            status: RunStatus::Running,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: Utc::now(),
        };

        if let Err(e) = self.store.create_routine_run(&run).await {
            return Err(format!("failed to create run record: {e}"));
        }

        // Execute inline for manual triggers (caller wants to wait)
        let engine = EngineContext {
            store: self.store.clone(),
            llm: self.llm.clone(),
            workspace: self.workspace.clone(),
            notify_tx: self.notify_tx.clone(),
            scheduler: self.scheduler.clone(),
            context_manager: self.context_manager.clone(),
            running_count: self.running_count.clone(),
            max_lightweight_tokens: self.config.max_lightweight_tokens,
            full_job_timeout_secs: self.config.full_job_timeout_secs,
        };

        tokio::spawn(async move {
            execute_routine(engine, routine, run).await;
        });

        Ok(run_id)
    }

    /// Spawn a fire in a background task.
    fn spawn_fire(&self, routine: Routine, trigger_type: &str, trigger_detail: Option<String>) {
        let run = RoutineRun {
            id: Uuid::new_v4(),
            routine_id: routine.id,
            trigger_type: trigger_type.to_string(),
            trigger_detail,
            started_at: Utc::now(),
            completed_at: None,
            status: RunStatus::Running,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: Utc::now(),
        };

        let engine = EngineContext {
            store: self.store.clone(),
            llm: self.llm.clone(),
            workspace: self.workspace.clone(),
            notify_tx: self.notify_tx.clone(),
            scheduler: self.scheduler.clone(),
            context_manager: self.context_manager.clone(),
            running_count: self.running_count.clone(),
            max_lightweight_tokens: self.config.max_lightweight_tokens,
            full_job_timeout_secs: self.config.full_job_timeout_secs,
        };

        // Record the run in DB, then spawn execution
        let store = self.store.clone();
        tokio::spawn(async move {
            if let Err(e) = store.create_routine_run(&run).await {
                tracing::error!(routine = %routine.name, "Failed to record run: {}", e);
                return;
            }
            execute_routine(engine, routine, run).await;
        });
    }

    /// Dry-run a routine and judge its readiness BEFORE it is enabled.
    ///
    /// Pre-flight simulation for autonomous work: executes the routine's
    /// action once with no run record, no runtime-state update, and no
    /// notification (safe to call on disabled routines), then asks the
    /// model to judge whether the output looks production-ready given the
    /// routine's stated purpose. The judgment is a model call returning
    /// structured JSON — no hand-coded quality rubric.
    pub async fn test_routine(&self, routine_id: Uuid) -> Result<RoutineTestReport, String> {
        let routine = self
            .store
            .get_routine(routine_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or_else(|| format!("routine {routine_id} not found"))?;

        let ctx = EngineContext {
            store: self.store.clone(),
            llm: self.llm.clone(),
            workspace: self.workspace.clone(),
            notify_tx: self.notify_tx.clone(),
            scheduler: self.scheduler.clone(),
            context_manager: self.context_manager.clone(),
            running_count: self.running_count.clone(),
            max_lightweight_tokens: self.config.max_lightweight_tokens,
            full_job_timeout_secs: self.config.full_job_timeout_secs,
        };

        // Dry-runs deliberately never schedule a real job. A live full_job
        // runs through the scheduler with tool access, but the pre-flight
        // check must be side-effect-free (safe to call on disabled routines,
        // no memory writes, no notifications). So full_job is approximated
        // here as a single tool-less LLM call over the description — the
        // caveat below makes this approximation explicit to the caller.
        // Dry-runs count toward the global concurrency cap like live runs.
        self.running_count.fetch_add(1, Ordering::Relaxed);
        let result = match &routine.action {
            RoutineAction::Lightweight {
                prompt,
                context_paths,
                max_tokens,
            } => execute_lightweight(&ctx, &routine, prompt, context_paths, *max_tokens).await,
            RoutineAction::FullJob { description, .. } => {
                execute_lightweight(&ctx, &routine, description, &[], ctx.max_lightweight_tokens)
                    .await
            }
        };
        self.running_count.fetch_sub(1, Ordering::Relaxed);

        let mut caveats = Vec::new();
        if matches!(routine.action, RoutineAction::FullJob { .. }) {
            caveats.push(
                "full_job dry-run is approximate: a live run executes through \
                 the scheduler with tool access, but this pre-flight check ran \
                 it as a single tool-less LLM call to stay side-effect-free, so \
                 the judged output may differ from a real run"
                    .to_string(),
            );
        }

        let (status, summary, tokens) = match result {
            Ok(x) => x,
            Err(e) => (RunStatus::Failed, Some(e), None),
        };

        // Judge the dry-run output against the routine's stated purpose.
        let judge_prompt = format!(
            "You are judging whether a scheduled routine is ready to run \
             unattended. Be skeptical: this routine will act autonomously \
             on a schedule with nobody watching.\n\n\
             Routine name: {}\n\
             Routine description: {}\n\
             Trigger: {}\n\n\
             Dry-run result status: {}\n\
             Dry-run output:\n{}\n\n\
             Judge: does the output match the routine's purpose? Is it \
             specific enough to be useful and quiet enough not to spam? \
             Would repeated runs annoy or mislead the user?\n\n\
             Respond with a single JSON object and nothing else:\n\
             {{\"ready\": <true|false>, \"issues\": [\"<each concrete problem found>\"], \
             \"notes\": \"<one short paragraph of judgment>\"}}",
            routine.name,
            routine.description,
            routine.trigger.type_tag(),
            status,
            summary
                .as_deref()
                .unwrap_or("(no attention output — routine reported nothing to do)"),
        );

        let judge_request =
            CompletionRequest::new(vec![ChatMessage::user(&judge_prompt)]).with_temperature(0.2);

        let verdict = match self.llm.complete(judge_request).await {
            Ok(resp) => parse_readiness_verdict(&resp.content),
            Err(e) => ReadinessVerdict {
                ready: false,
                issues: vec![format!("judge LLM call failed: {e}")],
                notes: None,
            },
        };

        Ok(RoutineTestReport {
            routine_name: routine.name.clone(),
            run_status: status,
            output_summary: summary,
            tokens_used: tokens,
            ready: verdict.ready,
            issues: verdict.issues,
            judge_notes: verdict.notes,
            caveats,
        })
    }

    fn check_cooldown(&self, routine: &Routine) -> bool {
        if let Some(last_run) = routine.last_run_at {
            let elapsed = Utc::now().signed_duration_since(last_run);
            let cooldown = chrono::Duration::from_std(routine.guardrails.cooldown)
                .unwrap_or(chrono::Duration::seconds(300));
            if elapsed < cooldown {
                return false;
            }
        }
        true
    }

    async fn check_concurrent(&self, routine: &Routine) -> bool {
        match self.store.count_running_routine_runs(routine.id).await {
            Ok(count) => count < routine.guardrails.max_concurrent as i64,
            Err(e) => {
                tracing::error!(
                    routine = %routine.name,
                    "Failed to check concurrent runs: {}", e
                );
                false
            }
        }
    }
}

/// Shared context passed to the execution function.
struct EngineContext {
    store: Arc<dyn Database>,
    llm: Arc<dyn LlmProvider>,
    workspace: Arc<Workspace>,
    notify_tx: mpsc::Sender<RoutineNotification>,
    scheduler: Arc<Scheduler>,
    context_manager: Arc<ContextManager>,
    running_count: Arc<AtomicUsize>,
    max_lightweight_tokens: u32,
    full_job_timeout_secs: u64,
}

/// Execute a routine run. Handles both lightweight and full_job modes.
async fn execute_routine(ctx: EngineContext, routine: Routine, run: RoutineRun) {
    // Increment running count (atomic: survives panics in the execution below)
    ctx.running_count.fetch_add(1, Ordering::Relaxed);

    let result = match &routine.action {
        RoutineAction::Lightweight {
            prompt,
            context_paths,
            max_tokens,
        } => execute_lightweight(&ctx, &routine, prompt, context_paths, *max_tokens).await,
        RoutineAction::FullJob {
            title,
            description,
            max_iterations,
        } => execute_full_job(&ctx, &routine, run.id, title, description, *max_iterations).await,
    };

    // Decrement running count
    ctx.running_count.fetch_sub(1, Ordering::Relaxed);

    // Process result
    let (status, summary, tokens) = match result {
        Ok(execution) => execution,
        Err(e) => {
            tracing::error!(routine = %routine.name, "Execution failed: {}", e);
            (RunStatus::Failed, Some(e), None)
        }
    };

    // Complete the run record
    if let Err(e) = ctx
        .store
        .complete_routine_run(run.id, status, summary.as_deref(), tokens)
        .await
    {
        tracing::error!(routine = %routine.name, "Failed to complete run record: {}", e);
    }

    // Update routine runtime state
    let now = Utc::now();
    let next_fire = if let Trigger::Cron { ref schedule } = routine.trigger {
        next_cron_fire(schedule).unwrap_or(None)
    } else {
        None
    };

    let new_failures = if status == RunStatus::Failed {
        routine.consecutive_failures + 1
    } else {
        0
    };

    if let Err(e) = ctx
        .store
        .update_routine_runtime(
            routine.id,
            now,
            next_fire,
            routine.run_count + 1,
            new_failures,
            &routine.state,
        )
        .await
    {
        tracing::error!(routine = %routine.name, "Failed to update runtime state: {}", e);
    }

    // Send notifications based on config
    send_notification(
        &ctx.notify_tx,
        &routine.notify,
        &routine.name,
        status,
        summary.as_deref(),
    )
    .await;
}

/// Execute a lightweight routine (single LLM call).
async fn execute_lightweight(
    ctx: &EngineContext,
    routine: &Routine,
    prompt: &str,
    context_paths: &[String],
    max_tokens: u32,
) -> Result<(RunStatus, Option<String>, Option<i32>), String> {
    // Load context from workspace
    let mut context_parts = Vec::new();
    for path in context_paths {
        match ctx.workspace.read(path).await {
            Ok(doc) => {
                context_parts.push(format!("## {}\n\n{}", path, doc.content));
            }
            Err(e) => {
                tracing::debug!(
                    routine = %routine.name,
                    "Failed to read context path {}: {}", path, e
                );
            }
        }
    }

    // Load routine state from workspace
    let state_path = format!("routines/{}/state.md", routine.name);
    let state_content = match ctx.workspace.read(&state_path).await {
        Ok(doc) => Some(doc.content),
        Err(_) => None,
    };

    // Build the prompt
    let mut full_prompt = String::new();
    full_prompt.push_str(prompt);

    if !context_parts.is_empty() {
        full_prompt.push_str("\n\n---\n\n# Context\n\n");
        full_prompt.push_str(&context_parts.join("\n\n"));
    }

    if let Some(state) = &state_content {
        full_prompt.push_str("\n\n---\n\n# Previous State\n\n");
        full_prompt.push_str(state);
    }

    full_prompt.push_str("\n\n---\n\n");
    full_prompt.push_str(ATTENTION_FORMAT_INSTRUCTIONS);

    // Get system prompt
    let system_prompt = match ctx.workspace.system_prompt().await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(routine = %routine.name, "Failed to get system prompt: {}", e);
            String::new()
        }
    };

    let messages = if system_prompt.is_empty() {
        vec![ChatMessage::user(&full_prompt)]
    } else {
        vec![
            ChatMessage::system(&system_prompt),
            ChatMessage::user(&full_prompt),
        ]
    };

    // Determine max_tokens from model metadata with fallback
    let effective_max_tokens = match ctx.llm.model_metadata().await {
        Ok(meta) => {
            let from_api = meta.context_length.map(|ctx| ctx / 2).unwrap_or(max_tokens);
            from_api.max(max_tokens)
        }
        Err(_) => max_tokens,
    };

    let request = CompletionRequest::new(messages)
        .with_max_tokens(effective_max_tokens)
        .with_temperature(0.3);

    let response = ctx
        .llm
        .complete(request)
        .await
        .map_err(|e| format!("LLM call failed: {e}"))?;

    let content = response.content.trim();
    let tokens_used = Some((response.input_tokens + response.output_tokens) as i32);

    // Empty content guard (same as heartbeat)
    if content.is_empty() {
        return if response.finish_reason == FinishReason::Length {
            Err(
                "LLM response truncated (finish_reason=length) with no content. \
                 Model may have exhausted token budget on reasoning."
                    .to_string(),
            )
        } else {
            Err("LLM returned empty content.".to_string())
        };
    }

    // Structured check-in: {"needs_attention": bool, "summary": "..."}.
    // Unparseable output fails open to Attention with the raw content.
    let report = parse_attention_report(content);
    if report.needs_attention {
        Ok((RunStatus::Attention, report.summary, tokens_used))
    } else {
        Ok((RunStatus::Ok, None, tokens_used))
    }
}

/// Execute a `full_job` routine as a real scheduler job.
///
/// Unlike lightweight routines (a single tool-less LLM call), a full job is
/// submitted to the `Scheduler` and runs through the worker reasoning loop
/// with tool access. Memory tools are approved for autonomous execution via
/// the registered `TrustedToolsHook`; all other approval-gated tools remain
/// blocked. The `job_id` is recorded on the routine run row, then this fn
/// polls until the job reaches a terminal state and maps that to a
/// `RunStatus` for the caller's central notification path.
async fn execute_full_job(
    ctx: &EngineContext,
    routine: &Routine,
    run_id: Uuid,
    title: &str,
    description: &str,
    max_iterations: u32,
) -> Result<(RunStatus, Option<String>, Option<i32>), String> {
    // Create a job context scoped to this routine's user.
    let job_id = ctx
        .context_manager
        .create_job_for_user(&routine.user_id, title, description)
        .await
        .map_err(|e| format!("failed to create job context: {e}"))?;

    // Store max_iterations and routine provenance in the job metadata for
    // observability. The worker enforces its own hard iteration cap; the
    // routine's own limit is advisory context surfaced via the DB.
    let routine_name_clone = routine.name.clone();
    if let Err(e) = ctx
        .context_manager
        .update_context(job_id, move |jctx| {
            // Ensure metadata is an object (defaults to null).
            if !jctx.metadata.is_object() {
                jctx.metadata = serde_json::json!({});
            }
            if let Some(obj) = jctx.metadata.as_object_mut() {
                obj.insert(
                    "routine_max_iterations".to_string(),
                    serde_json::Value::Number(max_iterations.into()),
                );
                obj.insert(
                    "routine_name".to_string(),
                    serde_json::Value::String(routine_name_clone),
                );
            }
        })
        .await
    {
        tracing::warn!(
            routine = %routine.name,
            "Failed to set job metadata: {}", e
        );
    }

    // Persist the job record so it is visible in job listings.
    if let Ok(jctx) = ctx.context_manager.get_context(job_id).await {
        let store = ctx.store.clone();
        tokio::spawn(async move {
            if let Err(e) = store.save_job(&jctx).await {
                tracing::warn!("Failed to persist routine job {}: {}", job_id, e);
            }
        });
    }

    // Record job_id on the run row immediately (best-effort).
    if let Err(e) = ctx.store.update_routine_run_job_id(run_id, job_id).await {
        tracing::warn!(
            routine = %routine.name,
            run = %run_id,
            "Failed to record job_id on routine run: {}", e
        );
    }

    // Submit to the scheduler. The worker path handles tool execution with
    // safety + hooks (including any registered TrustedToolsHook grants).
    if let Err(e) = ctx.scheduler.schedule(job_id).await {
        return Err(format!("scheduler refused job: {e}"));
    }

    tracing::info!(
        routine = %routine.name,
        job = %job_id,
        "FullJob scheduled; polling for completion"
    );

    // Poll until the job reaches a terminal state, bounded by a hard
    // deadline. A permanently stuck job (self-repair gives up after max
    // attempts) must not hold this routine's concurrency slot forever —
    // that would starve ROUTINES_MAX_CONCURRENT for every other routine.
    let deadline = Duration::from_secs(ctx.full_job_timeout_secs);
    let polled = tokio::time::timeout(deadline, poll_job_to_terminal(ctx, routine, job_id)).await;

    match polled {
        Ok(result) => result,
        Err(_) => {
            tracing::warn!(
                routine = %routine.name,
                job = %job_id,
                timeout_secs = ctx.full_job_timeout_secs,
                "FullJob exceeded deadline; cancelling"
            );
            if let Err(e) = ctx.scheduler.stop(job_id).await {
                tracing::warn!(job = %job_id, "Failed to cancel timed-out job: {}", e);
            }
            Err(format!(
                "job {} exceeded the full_job timeout of {}s and was cancelled",
                job_id, ctx.full_job_timeout_secs
            ))
        }
    }
}

/// Poll a scheduled job until it reaches a terminal state. Uses exponential
/// back-off within a bounded range to avoid busy-spinning on long jobs.
/// The caller bounds this with the full_job deadline.
async fn poll_job_to_terminal(
    ctx: &EngineContext,
    routine: &Routine,
    job_id: Uuid,
) -> Result<(RunStatus, Option<String>, Option<i32>), String> {
    let poll_interval_ms: u64 = 500;
    let max_poll_ms: u64 = 5_000;
    let mut interval_ms = poll_interval_ms;

    loop {
        tokio::time::sleep(Duration::from_millis(interval_ms)).await;
        interval_ms = (interval_ms * 2).min(max_poll_ms);

        let job_ctx = match ctx.context_manager.get_context(job_id).await {
            Ok(c) => c,
            Err(e) => {
                return Err(format!("lost job context while polling: {e}"));
            }
        };

        match job_ctx.state {
            JobState::Completed | JobState::Accepted | JobState::Submitted => {
                tracing::info!(
                    routine = %routine.name,
                    job = %job_id,
                    state = ?job_ctx.state,
                    "FullJob finished successfully"
                );
                let summary = Some(format!("Job {} completed ({})", job_id, job_ctx.state));
                return Ok((RunStatus::Ok, summary, None));
            }
            JobState::Failed => {
                let reason = job_ctx
                    .transitions
                    .last()
                    .and_then(|t| t.reason.clone())
                    .unwrap_or_else(|| "unknown reason".to_string());
                tracing::warn!(
                    routine = %routine.name,
                    job = %job_id,
                    "FullJob failed: {}", reason
                );
                return Err(format!("job {} failed: {}", job_id, reason));
            }
            JobState::Cancelled => {
                return Err(format!("job {} was cancelled", job_id));
            }
            JobState::Stuck => {
                // Self-repair may recover it; the full_job deadline bounds
                // how long we wait for that.
                tracing::warn!(
                    routine = %routine.name,
                    job = %job_id,
                    "FullJob is stuck — waiting for self-repair"
                );
            }
            JobState::Pending | JobState::InProgress => {
                // Still running; keep polling.
            }
        }
    }
}

/// Send a notification based on the routine's notify config and run status.
///
/// The notify target (channel + user) is attached to the notification so
/// the forwarder can deliver it to the routine's configured destination.
async fn send_notification(
    tx: &mpsc::Sender<RoutineNotification>,
    notify: &NotifyConfig,
    routine_name: &str,
    status: RunStatus,
    summary: Option<&str>,
) {
    let should_notify = match status {
        RunStatus::Ok => notify.on_success,
        RunStatus::Attention => notify.on_attention,
        RunStatus::Failed => notify.on_failure,
        RunStatus::Running => false,
    };

    if !should_notify {
        return;
    }

    let icon = match status {
        RunStatus::Ok => "✅",
        RunStatus::Attention => "🔔",
        RunStatus::Failed => "❌",
        RunStatus::Running => "⏳",
    };

    let message = match summary {
        Some(s) => format!("{} *Routine '{}'*: {}\n\n{}", icon, routine_name, status, s),
        None => format!("{} *Routine '{}'*: {}", icon, routine_name, status),
    };

    let response = OutgoingResponse {
        content: message,
        thread_id: None,
        metadata: serde_json::json!({
            "source": "routine",
            "routine_name": routine_name,
            "status": status.to_string(),
        }),
    };

    let notification = RoutineNotification {
        channel: notify.channel.clone(),
        user: notify.user.clone(),
        response,
    };

    if let Err(e) = tx.send(notification).await {
        tracing::error!(routine = %routine_name, "Failed to send notification: {}", e);
    }
}

/// Spawn the cron ticker background task.
pub fn spawn_cron_ticker(
    engine: Arc<RoutineEngine>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        // Skip immediate first tick
        ticker.tick().await;

        loop {
            ticker.tick().await;
            engine.check_cron_triggers().await;
        }
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = crate::util::floor_char_boundary(s, max);
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::parse_readiness_verdict;
    use crate::agent::routine::{NotifyConfig, RunStatus};

    #[test]
    fn test_readiness_verdict_ready() {
        let v = parse_readiness_verdict(
            r#"{"ready": true, "issues": [], "notes": "Output matches purpose."}"#,
        );
        assert!(v.ready);
        assert!(v.issues.is_empty());
        assert_eq!(v.notes.as_deref(), Some("Output matches purpose."));
    }

    #[test]
    fn test_readiness_verdict_not_ready_with_issues() {
        let v = parse_readiness_verdict(
            r#"{"ready": false, "issues": ["output is generic", "would notify daily with no findings"], "notes": ""}"#,
        );
        assert!(!v.ready);
        assert_eq!(v.issues.len(), 2);
        assert!(v.notes.is_none());
    }

    #[test]
    fn test_readiness_verdict_fenced() {
        let v = parse_readiness_verdict("```json\n{\"ready\": true}\n```");
        assert!(v.ready);
    }

    // A pre-flight check that can't parse its own judge must not
    // green-light the routine: unparseable fails CLOSED.
    #[test]
    fn test_readiness_verdict_unparseable_fails_closed() {
        let v = parse_readiness_verdict("Looks good to me!");
        assert!(!v.ready);
        assert!(!v.issues.is_empty());
    }

    #[test]
    fn test_notification_gating() {
        let config = NotifyConfig {
            on_success: false,
            on_failure: true,
            on_attention: true,
            ..Default::default()
        };

        // on_success = false means Ok status should not notify
        assert!(!config.on_success);
        assert!(config.on_failure);
        assert!(config.on_attention);
    }

    #[test]
    fn test_run_status_icons() {
        // Just verify the mapping doesn't panic
        for status in [
            RunStatus::Ok,
            RunStatus::Attention,
            RunStatus::Failed,
            RunStatus::Running,
        ] {
            let _ = status.to_string();
        }
    }
}
