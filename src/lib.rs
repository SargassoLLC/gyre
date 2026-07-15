//! Gyre Agentic Worker Framework
//!
//! An LLM-powered autonomous agent that operates on the Gyre platform.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────────┐
//! │                              User Interaction Layer                              │
//! │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐                         │
//! │  │   CLI    │  │  Slack   │  │ Telegram │  │   HTTP   │                         │
//! │  └────┬─────┘  └────┬─────┘  └────┬─────┘  └────┬─────┘                         │
//! │       └─────────────┴────────────┬┴─────────────┘                               │
//! └──────────────────────────────────┼──────────────────────────────────────────────┘
//!                                    ▼
//! ┌──────────────────────────────────────────────────────────────────────────────────┐
//! │                              Main Agent Loop                                      │
//! │  ┌────────────────┐  ┌────────────────┐  ┌────────────────┐                      │
//! │  │ Message Router │──│  LLM Reasoning │──│ Action Executor│                      │
//! │  └────────────────┘  └───────┬────────┘  └───────┬────────┘                      │
//! │         ▲                    │                   │                               │
//! │         │         ┌──────────┴───────────────────┴──────────┐                    │
//! │         │         ▼                                         ▼                    │
//! │  ┌──────┴─────────────┐                         ┌───────────────────────┐        │
//! │  │   Safety Layer     │                         │    Self-Repair        │        │
//! │  │ - Input sanitizer  │                         │ - Stuck job detection │        │
//! │  │ - Injection defense│                         │ - Tool fixer          │        │
//! │  └────────────────────┘                         └───────────────────────┘        │
//! └──────────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Features
//!
//! - **Multi-channel interaction** - CLI, Slack, Telegram, HTTP webhooks
//! - **Parallel job execution** - Run multiple jobs with isolated contexts
//! - **Pluggable tools** - MCP, 3rd party services, dynamic tools
//! - **Self-repair** - Detect and fix stuck jobs and broken tools
//! - **Prompt injection defense** - Sanitize all external data
//! - **Continuous learning** - Improve estimates from historical data

pub mod agent;
pub mod boot_screen;
pub mod bootstrap;
pub mod channels;
pub mod cli;
pub mod cognitive;
pub mod config;
pub mod context;
pub mod db;
pub mod error;
pub mod estimation;
pub mod evaluation;
pub mod extensions;
pub mod history;
pub mod hooks;
pub mod licensing;
pub mod llm;
pub mod observability;
pub mod orchestrator;
pub mod pairing;
pub mod safety;
pub mod sandbox;
pub mod secrets;
pub mod service;
pub mod settings;
pub mod setup;
pub mod skills;
pub mod template;
pub mod tools;
pub mod tracing_fmt;
pub mod tunnel;
pub mod util;
pub mod worker;
pub mod workspace;

pub use config::Config;
pub use error::{Error, Result};

/// Process-global mutex for tests that spawn child processes.
///
/// tokio installs a **process-wide** SIGCHLD handler that manages a shared
/// child-process registry. When concurrent test threads each spawn and
/// wait on tokio child processes, the signal handler races with the
/// per-thread `waitpid()` calls, which can trigger internal assertions and
/// cause the test binary to abort with SIGABRT.
///
/// Any test that spawns a `tokio::process::Child` (via `tokio::process::Command`
/// or shells the `ShellTool`) must hold this lock for the duration of the
/// child's lifetime (spawn → wait/kill).
///
/// Usage:
/// ```ignore
/// let _guard = crate::test_helpers::PROC_MUTEX.lock().unwrap();
/// ```
#[cfg(test)]
pub mod test_helpers {
    use std::sync::Mutex;

    pub static PROC_MUTEX: Mutex<()> = Mutex::new(());
}

/// Re-export commonly used types.
pub mod prelude {
    pub use crate::channels::{Channel, IncomingMessage, MessageStream};
    pub use crate::config::Config;
    pub use crate::context::{JobContext, JobState};
    pub use crate::error::{Error, Result};
    pub use crate::llm::LlmProvider;
    pub use crate::safety::{SanitizedOutput, Sanitizer};
    pub use crate::tools::{Tool, ToolOutput, ToolRegistry};
    pub use crate::workspace::{MemoryDocument, Workspace};
}
