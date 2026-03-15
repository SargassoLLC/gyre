//! `gyre tribe` subcommand — prepare and execute a Worker job from a Chief's
//! cognitive context, using a real LLM provider when available.

use std::path::PathBuf;
use std::sync::Arc;

use crate::cognitive::CognitiveAgent;
use crate::cognitive::orchestrator::{TribeOrchestrator, WorkerJobStatus, store_worker_result};
use crate::config::Config;
use crate::llm::{SessionConfig, create_llm_provider, create_session_manager};

/// Blocked path prefixes (same validation as cognitive_run).
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

/// Run the tribe orchestration command.
pub async fn run_tribe(chief_id: &str, base_dir: &PathBuf, task: &str) -> Result<(), String> {
    validate_base_dir(base_dir)?;

    // ── License gate: tribe requires Standard tier or higher ─────────────────
    {
        use crate::licensing::gates::check_tribe_enabled;
        use crate::licensing::validator::load_and_validate;
        let status = load_and_validate().await;
        let gates = status.feature_gates();
        if let Err(e) = check_tribe_enabled(&gates) {
            return Err(format!(
                "[license] {} Run `gyre license activate <KEY>` to unlock.",
                e
            ));
        }
    }
    // ─────────────────────────────────────────────────────────────────────────

    let agent = CognitiveAgent::open(base_dir, chief_id)?;

    // Try to load a real LLM provider
    match try_load_llm().await {
        Some(llm) => {
            println!("[Tribe] Executing worker job with LLM provider...");
            let job = TribeOrchestrator::execute(&agent, task, llm.as_ref()).await?;

            let result_text = match &job.status {
                WorkerJobStatus::Completed(text) => text.as_str(),
                _ => "(no result)",
            };
            println!("[Tribe] Worker job completed: {}", job.job_id);
            println!("[Tribe] Duration: {}ms", job.duration_ms.unwrap_or(0));
            if let Some(tokens) = job.tokens_used {
                println!("[Tribe] Tokens used: {}", tokens);
            }
            println!("[Tribe] Result:\n{}", result_text);
            println!("[Tribe] Result stored in Chief memory (tribe namespace).");
        }
        None => {
            eprintln!(
                "[Tribe] Configure LLM provider in gyre config to use real Workers. \
                 Falling back to simulation."
            );

            let job = TribeOrchestrator::prepare_job(&agent, task);
            println!(
                "Worker system prompt:\n{}",
                TribeOrchestrator::worker_system_prompt(&job)
            );
            println!("[Tribe] Worker job prepared: {}", job.job_id);

            let simulated_result = format!("Simulated result for: {}", task);
            let mut job = job;
            TribeOrchestrator::complete_job(&mut job, simulated_result);
            store_worker_result(&agent, &job)?;
            println!("[Tribe] Result stored in Chief memory (tribe namespace).");
        }
    }

    Ok(())
}
