//! Basic agent example.
//!
//! Demonstrates the minimal setup to run a Gyre agent: open a cognitive
//! agent from a local folder-world, wire up an LLM provider, and process
//! a single message — seeing the full reasoning pipeline in action.
//!
//! This is the "hello world" for Gyre — start here before exploring
//! the other examples.
//!
//! # Prerequisites
//!
//! Copy `deploy/env.example` to `.env` and fill in at minimum:
//!   - `ANTHROPIC_API_KEY` (or whichever LLM backend you're using)
//!
//! # Usage
//!
//!   cargo run --example basic_agent
//!
//! # What this shows
//!
//!   - Loading config from env
//!   - Opening a CognitiveAgent (hermit box + identity + memory)
//!   - Wiring an LLM provider
//!   - Sending a message and printing the response

use std::path::PathBuf;
use std::sync::Arc;

use gyre::{
    bootstrap::load_gyre_env,
    channels::IncomingMessage,
    cognitive::{CognitiveAgent, CognitiveChannelBridge},
    config::Config,
    llm::{SessionConfig, create_llm_provider, create_session_manager},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load ~/.gyre/.env then ./.env
    load_gyre_env();
    let _ = dotenvy::dotenv();

    // Set RUST_LOG=gyre=debug for verbose output
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "gyre=info".into()))
        .init();

    println!("=== Gyre Basic Agent Example ===\n");

    // ── 1. Load config ────────────────────────────────────────────────────────
    let config = Config::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("Config error: {}", e))?;

    println!("[1/4] Config loaded — LLM: {:?}", config.llm.backend);

    // ── 2. Open cognitive agent ───────────────────────────────────────────────
    //
    // A "folder-world" is the agent's home directory. It contains:
    //   - SOUL.md        (personality & voice)
    //   - MEMORY.md      (summarized long-term memory)
    //   - knowledge.db   (SQLite: knowledge graph + memory stream + axioms)
    //
    // If the directory doesn't exist, a fresh one is created automatically.
    let agent_dir =
        PathBuf::from(std::env::var("GYRE_AGENT_DIR").unwrap_or_else(|_| "./gyre_agent".into()));
    let agent_id = std::env::var("GYRE_AGENT_ID").unwrap_or_else(|_| "default".into());

    let agent = CognitiveAgent::open(&agent_dir, &agent_id)
        .map_err(|e| anyhow::anyhow!("Failed to open agent: {}", e))?;

    println!("[2/4] Agent opened: {agent_id} @ {}", agent_dir.display());

    // Print identity summary if available
    let system_prefix = agent.system_prompt_prefix();
    if !system_prefix.is_empty() {
        println!(
            "  Identity: {} chars of context loaded",
            system_prefix.len()
        );
    } else {
        println!("  Identity: no SOUL.md or MEMORY.md found (fresh agent)");
    }

    // ── 3. Wire up LLM ───────────────────────────────────────────────────────
    let session_manager = create_session_manager(SessionConfig::default()).await;
    let llm = create_llm_provider(&config.llm, &config.resilience, session_manager)?;

    println!("[3/4] LLM: {}", llm.model_name());

    // ── 4. Send a message ─────────────────────────────────────────────────────
    //
    // The CognitiveChannelBridge handles:
    //   - Building the full system prompt (identity + memory + axioms)
    //   - Adding turn-specific context from the knowledge graph
    //   - Sending to the LLM
    //   - Running post-turn memory extraction
    let bridge = CognitiveChannelBridge::new(Arc::new(agent));

    let user_message = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Who are you and what can you help me with?".into());

    println!("[4/4] Sending message...\n");
    println!("User: {user_message}");
    println!("{}", "─".repeat(60));

    let incoming = IncomingMessage::new("example", "user-001", &user_message);

    match bridge.process_message(&incoming, llm.as_ref()).await {
        Ok(response) => {
            println!("\nAgent: {response}");
            println!("{}", "─".repeat(60));
            println!("\n✅ Done. Run with a custom message:");
            println!("  cargo run --example basic_agent -- \"Your question here\"");
        }
        Err(e) => {
            eprintln!("\n❌ Agent error: {e}");
            std::process::exit(1);
        }
    }

    Ok(())
}
