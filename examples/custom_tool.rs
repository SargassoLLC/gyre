//! Custom tool example.
//!
//! Demonstrates how to implement a custom tool and register it with the
//! ToolRegistry so the agent can invoke it during reasoning.
//!
//! Tools are how the agent interacts with the outside world: fetching data,
//! running computations, calling APIs, reading files, etc. Any Rust struct
//! that implements the `Tool` trait can be registered.
//!
//! This example builds two tools:
//!
//!   1. `word_count` — counts words in a string (synchronous, trivial)
//!   2. `weather_stub` — simulates a weather API lookup (shows async + params)
//!
//! Then runs the agent with these tools available, so the LLM can call them
//! when responding to a question that requires them.
//!
//! # Prerequisites
//!
//!   - `ANTHROPIC_API_KEY` (or configured LLM backend)
//!
//! # Usage
//!
//!   cargo run --example custom_tool

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use gyre::{
    bootstrap::load_gyre_env,
    channels::IncomingMessage,
    cognitive::{CognitiveAgent, CognitiveChannelBridge},
    config::Config,
    context::JobContext,
    llm::{SessionConfig, create_llm_provider, create_session_manager},
    tools::{Tool, ToolError, ToolOutput, ToolRegistry, require_str},
};

// ── Tool 1: word_count ────────────────────────────────────────────────────────
//
// A minimal synchronous tool. No external calls — just pure computation.

struct WordCountTool;

#[async_trait]
impl Tool for WordCountTool {
    fn name(&self) -> &str {
        "word_count"
    }

    fn description(&self) -> &str {
        "Count the number of words in a given text string."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The text to count words in."
                }
            },
            "required": ["text"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let text = require_str(&params, "text")?;
        let count = text.split_whitespace().count();

        Ok(ToolOutput::success(
            json!({ "word_count": count, "text_length": text.len() }),
            start.elapsed(),
        ))
    }

    // No external calls — sanitization not needed
    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ── Tool 2: weather_stub ──────────────────────────────────────────────────────
//
// A simulated async tool that mimics an external API call.
// Replace the stub logic with a real HTTP call to wire it up for real.

struct WeatherStubTool;

#[async_trait]
impl Tool for WeatherStubTool {
    fn name(&self) -> &str {
        "get_weather"
    }

    fn description(&self) -> &str {
        "Get the current weather for a given city. \
         Returns temperature (°F), conditions, and humidity."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "string",
                    "description": "City name to get weather for (e.g. 'Beaufort, SC')."
                }
            },
            "required": ["city"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let city = require_str(&params, "city")?;

        // Stub: simulate network latency
        tokio::time::sleep(Duration::from_millis(50)).await;

        // In production, replace with: reqwest::get(...).await?
        let result = match city.to_lowercase().as_str() {
            c if c.contains("beaufort") => json!({
                "city": city,
                "temp_f": 72,
                "conditions": "Partly cloudy",
                "humidity_pct": 68,
                "wind_mph": 12,
            }),
            c if c.contains("savannah") => json!({
                "city": city,
                "temp_f": 75,
                "conditions": "Sunny",
                "humidity_pct": 61,
                "wind_mph": 8,
            }),
            _ => json!({
                "city": city,
                "temp_f": 65,
                "conditions": "Unknown",
                "humidity_pct": 50,
                "wind_mph": 10,
                "note": "Stub data — city not in local dataset"
            }),
        };

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // real weather API would return true
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    load_gyre_env();
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "gyre=info".into()))
        .init();

    println!("=== Gyre Custom Tool Example ===\n");

    // ── 1. Config + LLM ───────────────────────────────────────────────────────
    let config = Config::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("Config error: {}", e))?;

    let session_mgr = create_session_manager(SessionConfig::default()).await;
    let llm = create_llm_provider(&config.llm, &config.resilience, session_mgr)?;

    println!("[1/4] LLM: {}", llm.model_name());

    // ── 2. Register custom tools ──────────────────────────────────────────────
    let registry = Arc::new(ToolRegistry::new());
    registry.register(Arc::new(WordCountTool)).await;
    registry.register(Arc::new(WeatherStubTool)).await;

    // Optionally add Gyre's builtin tools alongside your custom ones
    // registry.register_builtin_tools();

    let tool_names = registry.list().await;
    println!("[2/4] Tools registered: {}", tool_names.join(", "));

    // ── 3. Open agent ─────────────────────────────────────────────────────────
    let agent_dir =
        PathBuf::from(std::env::var("GYRE_AGENT_DIR").unwrap_or_else(|_| "./gyre_agent".into()));
    let agent_id = std::env::var("GYRE_AGENT_ID").unwrap_or_else(|_| "default".into());

    let agent = Arc::new(
        CognitiveAgent::open(&agent_dir, &agent_id)
            .map_err(|e| anyhow::anyhow!("Failed to open agent: {}", e))?,
    );
    let bridge = Arc::new(CognitiveChannelBridge::new(Arc::clone(&agent)));

    println!("[3/4] Agent: {agent_id}");

    // ── 4. Run a query that exercises the tools ───────────────────────────────
    //
    // The LLM will see these tools in its system prompt and can call them
    // when the user's question requires external data or computation.
    let queries = vec![
        "How many words are in this sentence: 'The quick brown fox jumps over the lazy dog'?",
        "What's the weather like in Beaufort, SC right now?",
    ];

    println!("[4/4] Running queries...\n");
    println!("{}", "─".repeat(60));

    for query in queries {
        println!("User: {query}");

        let msg = IncomingMessage::new("example", "user-001", query);
        match bridge.process_message(&msg, llm.as_ref()).await {
            Ok(response) => println!("Agent: {response}\n"),
            Err(e) => eprintln!("Error: {e}\n"),
        }
        println!("{}", "─".repeat(60));
    }

    println!("\n✅ Done.");
    println!("\nTo add a new tool: implement the Tool trait, then call registry.register().");

    Ok(())
}
