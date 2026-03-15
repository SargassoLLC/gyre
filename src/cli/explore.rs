//! `gyre explore` subcommand — curiosity engine queue management and cycle execution.

use std::path::PathBuf;
use std::sync::Arc;

use crate::cognitive::CognitiveAgent;
use crate::cognitive::curiosity::{CuriosityEngine, TaskStatus};
use crate::config::Config;
use crate::llm::{SessionConfig, create_llm_provider, create_session_manager};

/// Blocked path prefixes (same validation as cognitive_run / tribe).
const BLOCKED_PATH_PREFIXES: &[&str] = &["/dev", "/proc", "/sys", "/run", "/var/run"];

fn validate_base_dir(base_dir: &PathBuf) -> Result<(), String> {
    let check_path = if base_dir.exists() {
        base_dir
            .canonicalize()
            .map_err(|e| format!("cannot canonicalize base_dir: {e}"))?
    } else {
        let parent = base_dir.parent().unwrap_or(base_dir);
        if !parent.exists() {
            return Err(format!(
                "base_dir parent does not exist: {}",
                parent.display()
            ));
        }
        parent
            .canonicalize()
            .map_err(|e| format!("cannot canonicalize parent: {e}"))?
            .join(
                base_dir
                    .file_name()
                    .ok_or_else(|| "base_dir has no filename component".to_string())?,
            )
    };

    let path_str = check_path.to_string_lossy();
    for prefix in BLOCKED_PATH_PREFIXES {
        if path_str.starts_with(prefix) {
            return Err(format!(
                "base_dir '{}' is under blocked prefix '{}'",
                check_path.display(),
                prefix
            ));
        }
    }

    if base_dir.exists() && !base_dir.is_dir() {
        return Err(format!(
            "base_dir '{}' exists but is not a directory",
            base_dir.display()
        ));
    }

    Ok(())
}

/// Attempt to load an LLM provider from the environment configuration.
async fn try_load_llm() -> Option<Arc<dyn crate::llm::LlmProvider>> {
    let config = Config::from_env().await.ok()?;
    let session_config = SessionConfig {
        auth_base_url: String::new(),
        session_path: crate::llm::session::default_session_path(),
    };
    let session = create_session_manager(session_config).await;
    create_llm_provider(&config.llm, &config.resilience, session).ok()
}

/// Run the explore subcommand.
pub async fn run_explore(
    agent_id: &str,
    base_dir: &PathBuf,
    show_queue: bool,
    add_topic: Option<&str>,
    cycles: u32,
) -> Result<(), String> {
    validate_base_dir(base_dir)?;

    // ── License gate: curiosity engine requires Standard tier or higher ───────
    {
        use crate::licensing::gates::check_curiosity_enabled;
        use crate::licensing::validator::load_and_validate;
        let status = load_and_validate().await;
        let gates = status.feature_gates();
        if let Err(e) = check_curiosity_enabled(&gates) {
            return Err(format!(
                "[license] {} Run `gyre license activate <KEY>` to unlock.",
                e
            ));
        }
    }
    // ─────────────────────────────────────────────────────────────────────────

    let agent = CognitiveAgent::open(base_dir, agent_id)?;
    let engine = CuriosityEngine::open_for_agent(&agent.hermit_box)
        .map_err(|e| format!("Failed to open CuriosityEngine: {e}"))?;

    // --add <topic>: push a manual task
    if let Some(topic) = add_topic {
        let id = engine
            .queue
            .push(topic, 8.0, "manual")
            .map_err(|e| format!("Failed to push task: {e}"))?;
        let display_topic = crate::cognitive::curiosity::sanitize_display_topic(topic);
        println!(
            "[Explore] Added manual task: id={}, topic=\"{}\"",
            id, display_topic
        );
        return Ok(());
    }

    // --queue: show pending tasks
    if show_queue {
        let tasks = engine
            .queue
            .peek(20)
            .map_err(|e| format!("Failed to peek queue: {e}"))?;

        if tasks.is_empty() {
            println!("[Explore] Queue is empty.");
        } else {
            println!(
                "{:<38} {:<8} {:<24} {}",
                "ID", "Priority", "Source", "Topic"
            );
            println!("{}", "-".repeat(90));
            for task in &tasks {
                let display_topic =
                    crate::cognitive::curiosity::sanitize_display_topic(&task.topic);
                println!(
                    "{:<38} {:<8.1} {:<24} {}",
                    task.id, task.priority, task.source, display_topic
                );
            }
            println!(
                "\n{} pending task(s)",
                engine
                    .queue
                    .pending_count()
                    .map_err(|e| format!("count: {e}"))?
            );
        }
        return Ok(());
    }

    // Default: run N cycles
    let llm = match try_load_llm().await {
        Some(llm) => llm,
        None => {
            return Err(
                "No LLM provider configured. Set LLM env vars to run curiosity cycles.".to_string(),
            );
        }
    };

    for i in 1..=cycles {
        println!("[Explore] Running cycle {i}/{cycles}...");
        let report = engine.run_cycle(&agent, llm.as_ref()).await?;

        if report.skipped_daily_limit {
            println!("[Explore] Skipped: daily limit reached");
            break;
        }

        println!("[Explore] Gaps detected: {}", report.gaps_detected);
        println!("[Explore] Tasks enqueued: {}", report.tasks_enqueued);

        if let Some(ref task) = report.task_processed {
            let status_str = match task.status {
                TaskStatus::Done => "completed",
                TaskStatus::Failed => "FAILED",
                _ => "in progress",
            };
            let display_topic = crate::cognitive::curiosity::sanitize_display_topic(&task.topic);
            println!(
                "[Explore] Task processed: \"{}\" [{}]",
                display_topic, status_str
            );
            if let Some(ref reason) = task.failure_reason {
                println!("[Explore]   Failure reason: {}", reason);
            }
        } else {
            println!("[Explore] No pending task to process.");
        }

        println!("[Explore] Entities added: {}", report.entities_added);
        println!("[Explore] Memories stored: {}", report.memories_stored);

        if i < cycles {
            println!();
        }
    }

    Ok(())
}
