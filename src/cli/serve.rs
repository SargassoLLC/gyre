//! `gyre serve` subcommand — always-on CognitiveAgent with multi-surface channels.
//!
//! With `--web`, starts the full web gateway (same as the main agent's gateway).
//! With `--telegram`, loads the Telegram WASM channel.
//! Without surface flags, falls back to a simple REPL driven by `CognitiveChannelBridge`.

use std::path::PathBuf;
use std::sync::Arc;

use crate::agent::{Agent, AgentDeps, SessionManager};
use crate::channels::{ChannelManager, GatewayChannel, IncomingMessage, ReplChannel};
use crate::cli::license::print_startup_license_info;
use crate::cognitive::agent::CognitiveAgent;
use crate::cognitive::channel_bridge::CognitiveChannelBridge;
use crate::cognitive::curiosity::{CuriosityConfig, CuriosityEngine, start_curiosity_loop};
use crate::config::{Config, GatewayConfig};
use crate::context::ContextManager;
use crate::hooks::HookRegistry;
use crate::licensing::validator::load_and_validate;
use crate::llm::{LlmProvider, SessionConfig, create_llm_provider, create_session_manager};
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;

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

/// Attempt to load an LLM provider from the environment configuration.
async fn try_load_llm() -> Option<(Arc<dyn LlmProvider>, Config)> {
    let config = Config::from_env().await.ok()?;
    let session_config = SessionConfig {
        auth_base_url: String::new(),
        session_path: crate::llm::session::default_session_path(),
    };
    let session = create_session_manager(session_config).await;
    let llm = create_llm_provider(&config.llm, &config.resilience, session).ok()?;
    Some((llm, config))
}

/// Run the serve subcommand.
///
/// When `web` or `telegram` is true, creates a full `Agent` with the cognitive layer
/// wired into `AgentDeps`, reusing the main agent's channel infrastructure.
/// Otherwise, falls back to a lightweight REPL driven by `CognitiveChannelBridge`.
pub async fn run_serve(
    agent_id: &str,
    base_dir: &PathBuf,
    no_curiosity: bool,
    curiosity_interval: u64,
    web: bool,
    telegram: bool,
    port: u16,
) -> Result<(), String> {
    validate_base_dir(base_dir)?;

    // ── License check ───────────────────────────────────────────────────────
    // Validate license on startup; print tier info / grace warnings.
    // Non-blocking: if the server is unreachable, falls back to local cache.
    let license_status: crate::licensing::LicenseStatus = load_and_validate().await;
    print_startup_license_info(&license_status).await;
    let license_gates = license_status.feature_gates();

    // Gate curiosity engine on license
    if !no_curiosity && !license_gates.curiosity_engine {
        eprintln!(
            "[license] Curiosity engine requires Standard tier or higher. \
             Upgrade at https://gyre.ai/pricing"
        );
    }
    // ─────────────────────────────────────────────────────────────────────────

    let cog_agent = CognitiveAgent::open(base_dir, agent_id)?;
    let cog_agent = Arc::new(cog_agent);

    let (llm, config) = match try_load_llm().await {
        Some(pair) => pair,
        None => {
            return Err("No LLM provider configured. Set LLM env vars to run serve.".to_string());
        }
    };

    // Determine if we need the full agent or the lightweight bridge
    let use_full_agent = web || telegram;

    if use_full_agent {
        run_serve_full_agent(
            agent_id,
            cog_agent.clone(),
            llm.clone(),
            config,
            web,
            telegram,
            port,
            no_curiosity,
            curiosity_interval,
        )
        .await
    } else {
        run_serve_repl(
            agent_id,
            cog_agent.clone(),
            llm.clone(),
            no_curiosity,
            curiosity_interval,
        )
        .await
    }
}

/// Full agent mode: creates a real `Agent` with cognitive layer wired into AgentDeps.
async fn run_serve_full_agent(
    agent_id: &str,
    cog_agent: Arc<CognitiveAgent>,
    llm: Arc<dyn LlmProvider>,
    config: Config,
    web: bool,
    _telegram: bool,
    port: u16,
    no_curiosity: bool,
    curiosity_interval: u64,
) -> Result<(), String> {
    let safety = Arc::new(SafetyLayer::new(&config.safety));
    let tools = Arc::new(ToolRegistry::new());
    tools.register_builtin_tools();

    let hooks = Arc::new(HookRegistry::new());
    let session_manager = Arc::new(SessionManager::new().with_hooks(hooks.clone()));
    let context_manager = Arc::new(ContextManager::new(config.agent.max_parallel_jobs));

    let cost_guard = Arc::new(crate::agent::cost_guard::CostGuard::new(
        crate::agent::cost_guard::CostGuardConfig::default(),
    ));

    // Wire cognitive layer into AgentDeps
    let deps = AgentDeps {
        store: None,
        llm: llm.clone(),
        cheap_llm: None,
        safety,
        tools: tools.clone(),
        workspace: None,
        extension_manager: None,
        skill_registry: None,
        skills_config: config.skills.clone(),
        hooks: hooks.clone(),
        cost_guard,
        cognitive: Some(Arc::new(cog_agent.context.clone())),
        identity: Some(cog_agent.identity.clone()),
    };

    // Set up channels
    let mut channels = ChannelManager::new();

    // Always add REPL for interactive use
    let repl = ReplChannel::new();
    repl.suppress_banner();
    channels.add(Box::new(repl));

    // Web gateway
    if web {
        let gw_config = GatewayConfig {
            host: "127.0.0.1".to_string(),
            port,
            auth_token: None,
            user_id: "default".to_string(),
        };
        let gw = GatewayChannel::new(gw_config.clone())
            .with_session_manager(Arc::clone(&session_manager))
            .with_tool_registry(Arc::clone(&tools))
            .with_llm_provider(llm.clone());

        eprintln!(
            "[Serve] Web gateway enabled on http://{}:{}/",
            gw_config.host, gw_config.port
        );
        eprintln!("[Serve] Auth token: {}", gw.auth_token());
        channels.add(Box::new(gw));
    }

    // Telegram WASM channel (if the channel binary exists)
    if _telegram {
        eprintln!(
            "[Serve] Telegram channel requested but WASM channel loading in serve \
             is not yet supported. Use the main `gyre` command with WASM_CHANNELS_ENABLED=true."
        );
    }

    // Spawn curiosity loop
    if !no_curiosity {
        spawn_curiosity(&cog_agent, llm.clone(), curiosity_interval).await;
    }

    let mut agent_config = config.agent.clone();
    agent_config.name = agent_id.to_string();

    let agent = Agent::new(
        agent_config,
        deps,
        channels,
        None,
        None,
        Some(context_manager),
        Some(session_manager),
    );

    eprintln!(
        "[Serve] Agent '{}' ready (multi-surface). Type messages, Ctrl-C to exit.",
        agent_id
    );

    agent.run().await.map_err(|e| e.to_string())
}

/// Lightweight REPL mode: uses CognitiveChannelBridge directly.
async fn run_serve_repl(
    agent_id: &str,
    agent: Arc<CognitiveAgent>,
    llm: Arc<dyn LlmProvider>,
    no_curiosity: bool,
    curiosity_interval: u64,
) -> Result<(), String> {
    let bridge = CognitiveChannelBridge::new(Arc::clone(&agent));

    if !no_curiosity {
        spawn_curiosity(&agent, llm.clone(), curiosity_interval).await;
    }

    eprintln!(
        "[Serve] Agent '{}' ready. Type messages, empty line or Ctrl-C to exit.",
        agent_id
    );
    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        eprint!("> ");
        line.clear();
        let bytes = stdin
            .read_line(&mut line)
            .map_err(|e| format!("stdin read failed: {e}"))?;
        if bytes == 0 || line.trim().is_empty() {
            break;
        }
        let user_input = line.trim();

        let msg = IncomingMessage::new("repl", "local_user", user_input);
        match bridge.process_message(&msg, llm.as_ref()).await {
            Ok(response) => {
                println!("{}", response);
            }
            Err(e) => {
                eprintln!("Error: {}", e);
            }
        }
    }

    // Graceful shutdown: save memory snapshot
    let memory_summary = format!(
        "Session ended at {}. Agent '{}' served via REPL.",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
        agent_id
    );
    match agent.hermit_box.write_memory_summary(&memory_summary) {
        Ok(()) => eprintln!("Agent {} shutting down. Memory saved.", agent_id),
        Err(e) => eprintln!(
            "Agent {} shutting down. Failed to save memory: {}",
            agent_id, e
        ),
    }

    Ok(())
}

/// Spawn the background curiosity engine.
async fn spawn_curiosity(
    agent: &Arc<CognitiveAgent>,
    llm: Arc<dyn LlmProvider>,
    curiosity_interval: u64,
) {
    match CuriosityEngine::open_for_agent(&agent.hermit_box) {
        Ok(engine) => {
            let config = CuriosityConfig {
                cycle_interval_secs: curiosity_interval,
                ..CuriosityConfig::default()
            };
            let engine = Arc::new(CuriosityEngine {
                queue: engine.queue,
                config,
            });
            start_curiosity_loop(engine, Arc::clone(agent), Arc::clone(&llm)).await;
            eprintln!(
                "[Serve] Background curiosity enabled (interval: {}s)",
                curiosity_interval
            );
        }
        Err(e) => {
            eprintln!("[Serve] Warning: failed to start curiosity engine: {e}");
        }
    }
}
