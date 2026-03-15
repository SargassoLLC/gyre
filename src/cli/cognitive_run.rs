//! `gyre cognitive-run` subcommand — one-shot or REPL mode with a CognitiveAgent.

use std::path::PathBuf;

use crate::cognitive::CognitiveAgent;
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider};

/// Blocked path prefixes that must not be used as base_dir for HermitBox storage.
/// Prevents device files, kernel filesystems, and other dangerous paths from being
/// passed as `--box` arguments to `gyre cognitive-run`.
const BLOCKED_PATH_PREFIXES: &[&str] = &["/dev", "/proc", "/sys", "/run", "/var/run"];

/// Validate that `base_dir` is a safe, real directory for HermitBox storage.
fn validate_base_dir(base_dir: &PathBuf) -> anyhow::Result<()> {
    // Resolve the path (may not exist yet, so we check parent if needed)
    let check_path = if base_dir.exists() {
        base_dir.canonicalize()?
    } else {
        // If it doesn't exist, check that its parent is a real directory
        let parent = base_dir.parent().unwrap_or(base_dir);
        if !parent.exists() {
            anyhow::bail!("base_dir parent does not exist: {}", parent.display());
        }
        parent.canonicalize()?.join(
            base_dir
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("base_dir has no filename component"))?,
        )
    };

    let path_str = check_path.to_string_lossy();

    // Block dangerous path prefixes
    for prefix in BLOCKED_PATH_PREFIXES {
        if path_str.starts_with(prefix) {
            anyhow::bail!(
                "base_dir '{}' is under blocked prefix '{}' — device/kernel paths are not allowed",
                check_path.display(),
                prefix
            );
        }
    }

    // If the path exists, it must be a directory (not a file, device, symlink-to-device, etc.)
    if base_dir.exists() && !base_dir.is_dir() {
        anyhow::bail!(
            "base_dir '{}' exists but is not a directory",
            base_dir.display()
        );
    }

    Ok(())
}

/// Run a cognitive agent in one-shot or interactive REPL mode.
pub async fn run_cognitive(
    agent_id: &str,
    base_dir: &PathBuf,
    message: Option<&str>,
    verbose: bool,
    llm: &dyn LlmProvider,
) -> anyhow::Result<()> {
    validate_base_dir(base_dir)?;

    let agent = CognitiveAgent::open(base_dir, agent_id)
        .map_err(|e| anyhow::anyhow!("Failed to open CognitiveAgent: {}", e))?;

    let prefix = agent.system_prompt_prefix();
    if verbose && !prefix.is_empty() {
        eprintln!("--- System prompt prefix ---");
        eprintln!("{}", prefix);
        eprintln!("----------------------------");
    }

    if let Some(msg) = message {
        // One-shot mode
        let response = call_llm(llm, &prefix, msg).await?;
        println!("{}", response);
        agent.post_turn(&response);
    } else {
        // Interactive REPL
        eprintln!(
            "[CognitiveAgent '{}' ready. Empty line or Ctrl-C to exit.]",
            agent_id
        );
        let stdin = std::io::stdin();
        let mut line = String::new();
        loop {
            eprint!("> ");
            line.clear();
            let bytes = stdin.read_line(&mut line)?;
            if bytes == 0 || line.trim().is_empty() {
                break;
            }
            let user_input = line.trim();
            match call_llm(llm, &prefix, user_input).await {
                Ok(response) => {
                    println!("{}", response);
                    agent.post_turn(&response);
                }
                Err(e) => {
                    eprintln!("LLM error: {}", e);
                }
            }
        }
    }

    Ok(())
}

async fn call_llm(
    llm: &dyn LlmProvider,
    system_prefix: &str,
    user_message: &str,
) -> anyhow::Result<String> {
    let mut messages = Vec::new();
    if !system_prefix.is_empty() {
        messages.push(ChatMessage::system(system_prefix));
    }
    messages.push(ChatMessage::user(user_message));

    let request = CompletionRequest::new(messages)
        .with_max_tokens(4096)
        .with_temperature(0.7);

    let response = llm
        .complete(request)
        .await
        .map_err(|e| anyhow::anyhow!("LLM completion failed: {}", e))?;

    Ok(response.content)
}
