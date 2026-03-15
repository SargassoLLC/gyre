//! Multi-channel example.
//!
//! Demonstrates how Gyre's ChannelManager merges messages from multiple
//! sources into a single unified stream — the same architecture used in
//! production when Telegram, CLI, and HTTP all feed the same agent.
//!
//! This example wires up two channels simultaneously:
//!
//!   1. **ReplChannel** — stdin/stdout interactive terminal
//!   2. **Inject channel** — programmatic background messages (cron jobs,
//!      monitors, health checks — anything that needs to talk to the agent
//!      without being a UI channel)
//!
//! All messages, regardless of source, flow through the same agent loop.
//! The agent doesn't know (or care) which channel sent what.
//!
//! # Usage
//!
//!   cargo run --example multi_channel
//!
//! Type messages in the terminal. A background task will inject an automated
//! message every 30 seconds to show the inject channel in action.
//!
//! Ctrl+C to exit.
//!
//! # What this shows
//!
//!   - Building a ChannelManager with multiple channels
//!   - Using inject_sender for programmatic/background messages
//!   - The unified MessageStream that feeds the agent loop
//!   - How to extend to real channels (Telegram, HTTP) — same pattern

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::time;

use gyre::{
    bootstrap::load_gyre_env,
    channels::ReplChannel,
    channels::{ChannelManager, IncomingMessage},
    cognitive::{CognitiveAgent, CognitiveChannelBridge},
    config::Config,
    llm::{SessionConfig, create_llm_provider, create_session_manager},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    load_gyre_env();
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "gyre=info".into()))
        .init();

    println!("=== Gyre Multi-Channel Example ===\n");

    // ── 1. Config + LLM ───────────────────────────────────────────────────────
    let config = Config::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("Config error: {}", e))?;

    let session_mgr = create_session_manager(SessionConfig::default()).await;
    let llm = create_llm_provider(&config.llm, &config.resilience, session_mgr)?;

    println!("[1/3] LLM: {}", llm.model_name());

    // ── 2. Open cognitive agent ───────────────────────────────────────────────
    let agent_dir =
        PathBuf::from(std::env::var("GYRE_AGENT_DIR").unwrap_or_else(|_| "./gyre_agent".into()));
    let agent_id = std::env::var("GYRE_AGENT_ID").unwrap_or_else(|_| "default".into());

    let agent = Arc::new(
        CognitiveAgent::open(&agent_dir, &agent_id)
            .map_err(|e| anyhow::anyhow!("Failed to open agent: {}", e))?,
    );
    let bridge = Arc::new(CognitiveChannelBridge::new(Arc::clone(&agent)));

    println!("[2/3] Agent: {agent_id}");

    // ── 3. Build ChannelManager with multiple sources ─────────────────────────
    //
    // Each channel implements the Channel trait and produces IncomingMessages.
    // ChannelManager merges them into one stream via select_all.
    let mut manager = ChannelManager::new();

    // Channel 1: REPL — reads from stdin, writes responses to stdout
    manager.add(Box::new(ReplChannel::new()));

    // Channel 2: Inject — a programmatic sender any background task can use.
    //   Use cases: cron jobs, health monitors, system alerts, scheduler.
    let inject_tx = manager.inject_sender();

    println!("[3/3] Channels ready: repl + inject\n");

    // ── Background task: inject a message every 30 seconds ───────────────────
    //
    // In production this would be a heartbeat runner, routine engine, or
    // job monitor sending status updates into the agent loop.
    let inject_tx_bg = inject_tx.clone();
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(30));
        interval.tick().await; // skip the first immediate tick

        let mut count = 0u32;
        loop {
            interval.tick().await;
            count += 1;

            let msg = IncomingMessage::new(
                "inject",
                "system",
                &format!("[Automated check #{count}] Summarize what we've talked about so far."),
            );

            if inject_tx_bg.send(msg).await.is_err() {
                break; // channel closed, agent loop exited
            }
        }
    });

    // ── Message loop ──────────────────────────────────────────────────────────
    //
    // In a full Agent::run() setup this is handled internally.
    // Here we drive it manually to keep the example transparent.
    println!("Ready. Type a message (Ctrl+C to exit).");
    println!("A background task will inject a message every 30 seconds.\n");
    println!("{}", "─".repeat(60));

    use futures::StreamExt;
    let mut stream = manager.start_all().await?;

    while let Some(msg) = stream.next().await {
        let channel = msg.channel.clone();
        let user = msg.user_id.clone();
        let content = msg.content.clone();

        // Show which channel the message came from
        if channel == "inject" {
            println!("\n[inject → agent] {content}");
        }

        let bridge = Arc::clone(&bridge);
        let llm = Arc::clone(&llm);

        // Process in a spawned task so channels don't block each other
        tokio::spawn(async move {
            match bridge.process_message(&msg, llm.as_ref()).await {
                Ok(response) => {
                    println!(
                        "\n[{channel}:{user}] → {}",
                        &content[..content.len().min(60)]
                    );
                    println!("Agent: {response}\n");
                    print!("You: ");
                }
                Err(e) => {
                    eprintln!("[{channel}] error: {e}");
                }
            }
        });
    }

    println!("\n✅ Exited.");
    Ok(())
}
