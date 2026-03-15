//! Setup stage trait and registry.
//!
//! Each setup stage implements the `SetupStage` trait and is registered
//! in the stage registry. The `SetupEngine` iterates over stages in order.

use std::fmt;

use async_trait::async_trait;

use super::state::SetupState;
use super::ui::SetupUi;

/// Outcome of running a setup stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageOutcome {
    /// Stage completed successfully.
    Completed,
    /// Stage was skipped (already satisfied or QuickStart).
    Skipped,
    /// User requested going back to a previous stage.
    GoBack,
    /// Stage should be retried (transient failure).
    Retry,
}

/// Error during a setup stage.
#[derive(Debug)]
pub enum SetupError {
    /// User cancelled the setup.
    Cancelled,
    /// I/O error (terminal, filesystem, network).
    Io(std::io::Error),
    /// Database error.
    Database(String),
    /// Configuration validation error.
    Validation(String),
    /// Generic error with context.
    Other(anyhow::Error),
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetupError::Cancelled => write!(f, "Setup cancelled by user"),
            SetupError::Io(e) => write!(f, "I/O error: {}", e),
            SetupError::Database(e) => write!(f, "Database error: {}", e),
            SetupError::Validation(e) => write!(f, "Validation error: {}", e),
            SetupError::Other(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for SetupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SetupError::Io(e) => Some(e),
            SetupError::Other(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SetupError {
    fn from(e: std::io::Error) -> Self {
        SetupError::Io(e)
    }
}

impl From<anyhow::Error> for SetupError {
    fn from(e: anyhow::Error) -> Self {
        SetupError::Other(e)
    }
}

/// Trait for a single setup stage.
///
/// Each stage has a unique ID, display name, and a `run` method that
/// performs the interactive setup and mutates the shared `SetupState`.
#[async_trait]
pub trait SetupStage: Send + Sync {
    /// Unique identifier for this stage (e.g., "database", "security").
    fn id(&self) -> &'static str;

    /// Human-readable name shown in the step header.
    fn name(&self) -> &'static str;

    /// Whether this stage can be skipped in QuickStart mode.
    fn skippable_in_quickstart(&self) -> bool {
        false
    }

    /// Check if this stage's requirements are already satisfied.
    ///
    /// When `true`, the engine may skip this stage automatically
    /// (showing "already configured" to the user).
    async fn is_satisfied(&self, _state: &SetupState) -> bool {
        false
    }

    /// Run the stage, interacting with the user via `ui` and
    /// mutating the shared `state`.
    async fn run(&self, state: &mut SetupState, ui: &SetupUi) -> Result<StageOutcome, SetupError>;
}

/// Build the ordered list of all setup stages.
pub fn all_stages() -> Vec<Box<dyn SetupStage>> {
    vec![
        Box::new(s01_risk_ack::RiskAckStage),
        Box::new(s02_detect::DetectStage),
        Box::new(s03_flow::FlowStage),
        Box::new(s04_database::DatabaseStage),
        Box::new(s05_security::SecurityStage),
        Box::new(s06_auth_model::AuthModelStage),
        Box::new(s07_agents::AgentsStage),
        Box::new(s08_channels::ChannelsStage),
        Box::new(s09_gateway::GatewayStage),
        Box::new(s10_finalize::FinalizeStage),
    ]
}

pub mod s01_risk_ack;
pub mod s02_detect;
pub mod s03_flow;
pub mod s04_database;
pub mod s05_security;
pub mod s06_auth_model;
pub mod s07_agents;
pub mod s08_channels;
pub mod s09_gateway;
pub mod s10_finalize;
