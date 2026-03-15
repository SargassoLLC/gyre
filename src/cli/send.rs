//! `gyre send` subcommand — send a task from one agent to another via A2A protocol.

use std::path::PathBuf;

use chrono::Utc;

use crate::cognitive::a2a::{A2AMessage, A2ARouter};
use crate::cognitive::curiosity::sanitize_display_topic;

/// Blocked path prefixes (same validation as cognitive_run / tribe / explore).
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

/// Maximum task length for CLI input. Must match ResearchQueue::MAX_TOPIC_LEN
/// minus room for the `[FROM:agent] ` prefix (max agent_id=64 + "[FROM:] "=8 = 72).
const MAX_CLI_TASK_LEN: usize = 500;

/// Run the send subcommand: send a task between agents via A2A.
pub fn run_send(from: &str, to: &str, base_dir: &PathBuf, task: &str) -> Result<(), String> {
    validate_base_dir(base_dir)?;

    // Enforce task length cap before it enters the queue.
    // The ResearchQueue also truncates at 500 bytes, but by that point the
    // [FROM:agent] prefix has already been prepended, so enforcing here
    // ensures the user's actual task content is not silently eaten.
    if task.len() > MAX_CLI_TASK_LEN {
        return Err(format!(
            "Task too long ({} bytes, max {}). Shorten the task description.",
            task.len(),
            MAX_CLI_TASK_LEN
        ));
    }

    let router = A2ARouter::new(base_dir);
    let msg = A2AMessage {
        from_agent: from.to_string(),
        to_agent: to.to_string(),
        task: task.to_string(),
        priority: 8.0,
        created_at: Utc::now(),
    };

    router.send(&msg)?;

    let display_task = sanitize_display_topic(task);
    let preview = if display_task.len() > 80 {
        format!("{}...", &display_task[..77])
    } else {
        display_task
    };
    println!("Task queued for {}: {}", to, preview);

    Ok(())
}
