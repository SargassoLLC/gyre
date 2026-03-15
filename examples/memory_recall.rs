//! Memory and recall example.
//!
//! Demonstrates Gyre's cognitive memory system: storing facts, observations,
//! and decisions into the memory stream, then recalling them by recency and
//! importance — the same system used during live agent turns to build context.
//!
//! No LLM required — this runs entirely against the local SQLite knowledge base.
//!
//! # Usage
//!
//!   cargo run --example memory_recall
//!
//! # What this shows
//!
//!   - Opening a CognitiveAgent folder-world
//!   - Writing memories (observations, facts, decisions) with importance scores
//!   - Recalling memories by recency + importance ranking
//!   - Namespace scoping (personal vs tribe)
//!   - How the memory stream feeds into agent system prompts

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use gyre::{
    bootstrap::load_gyre_env,
    cognitive::{
        CognitiveAgent,
        memory_stream::{MemoryEntry, MemoryKind, namespaces},
    },
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    load_gyre_env();
    let _ = dotenvy::dotenv();

    println!("=== Gyre Memory & Recall Example ===\n");

    // ── 1. Open agent ─────────────────────────────────────────────────────────
    let agent_dir =
        PathBuf::from(std::env::var("GYRE_AGENT_DIR").unwrap_or_else(|_| "./gyre_agent".into()));
    let agent_id = std::env::var("GYRE_AGENT_ID").unwrap_or_else(|_| "default".into());

    let agent = CognitiveAgent::open(&agent_dir, &agent_id)
        .map_err(|e| anyhow::anyhow!("Failed to open agent: {}", e))?;

    println!("[1/4] Agent: {agent_id} @ {}", agent_dir.display());

    let ms = agent
        .context
        .memory_stream
        .lock()
        .map_err(|_| anyhow::anyhow!("memory stream lock poisoned"))?;

    // ── 2. Count existing memories ────────────────────────────────────────────
    let existing = ms.count();
    println!("[2/4] Existing memories: {existing}\n");

    // ── 3. Write new memories ─────────────────────────────────────────────────
    println!("Writing memories...\n");

    let memories = vec![
        MemoryEntry {
            id: Uuid::new_v4(),
            content: "User prefers concise responses — skip preamble, lead with the answer.".into(),
            kind: MemoryKind::Preference,
            importance: 8.5,
            created_at: Utc::now(),
            namespace: namespaces::PERSONAL.into(),
        },
        MemoryEntry {
            id: Uuid::new_v4(),
            content: "Completed Phase 8 of gyre-rust: Docker packaging, health check, env cleanup."
                .into(),
            kind: MemoryKind::Lesson,
            importance: 7.0,
            created_at: Utc::now(),
            namespace: namespaces::PERSONAL.into(),
        },
        MemoryEntry {
            id: Uuid::new_v4(),
            content: "Decided to use CognitiveChannelBridge as the standard message path — \
                      avoids re-wiring AgentDeps for simple use cases."
                .into(),
            kind: MemoryKind::Decision,
            importance: 6.5,
            created_at: Utc::now(),
            namespace: namespaces::PERSONAL.into(),
        },
        MemoryEntry {
            id: Uuid::new_v4(),
            content: "Tribe alignment: all agents share the sargasso-tribe context pool.".into(),
            kind: MemoryKind::Observation,
            importance: 5.0,
            created_at: Utc::now(),
            namespace: namespaces::TRIBE.into(),
        },
    ];

    for mem in &memories {
        ms.add(mem)?;
        println!(
            "  ✅ [{:?}] importance={:.1} — {}",
            mem.kind,
            mem.importance,
            &mem.content[..mem.content.len().min(60)]
        );
    }

    println!("\nTotal memories after writes: {}", ms.count());

    // ── 4. Recall ─────────────────────────────────────────────────────────────
    println!("\n{}", "─".repeat(60));
    println!("Recalling top 5 by recency + importance:\n");

    let recalled = ms.recall("", 5)?;

    if recalled.is_empty() {
        println!("  (no memories found)");
    } else {
        for (i, entry) in recalled.iter().enumerate() {
            println!(
                "  {}. [{:?}] importance={:.1} namespace={}",
                i + 1,
                entry.kind,
                entry.importance,
                entry.namespace
            );
            println!("     {}", entry.content);
            println!();
        }
    }

    // ── Bonus: layered recall (recency + importance + relevance) ──────────────
    println!("{}", "─".repeat(60));
    println!("Layered recall for query \"Phase 9 examples\":\n");

    let layered = ms.recall_layered("Phase 9 examples", 3)?;
    if layered.is_empty() {
        println!("  (no relevant memories found)");
    } else {
        for (i, entry) in layered.iter().enumerate() {
            println!(
                "  {}. [{:?}] importance={:.1} — {}",
                i + 1,
                entry.kind,
                entry.importance,
                &entry.content[..entry.content.len().min(80)]
            );
        }
    }

    println!("\n✅ Done.");
    println!(
        "\nThese memories persist in: {}/knowledge.db",
        agent_dir.display()
    );
    println!("They'll be loaded automatically on the next agent turn.");

    Ok(())
}
