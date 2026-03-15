//! Stage 1: Risk Acknowledgment
//!
//! Display security warning (system access, no sandbox by default).
//! Require explicit confirmation before proceeding.

use async_trait::async_trait;

use super::{SetupError, SetupStage, StageOutcome};
use crate::setup::state::SetupState;
use crate::setup::ui::SetupUi;

pub struct RiskAckStage;

#[async_trait]
impl SetupStage for RiskAckStage {
    fn id(&self) -> &'static str {
        "risk_ack"
    }

    fn name(&self) -> &'static str {
        "Risk Acknowledgment"
    }

    async fn run(&self, _state: &mut SetupState, ui: &SetupUi) -> Result<StageOutcome, SetupError> {
        ui.info("Gyre is a personal AI assistant that can execute commands,");
        ui.info("access files, and interact with external services on your behalf.");
        ui.blank();
        ui.info("By proceeding, you acknowledge that:");
        ui.info("  - Gyre may execute shell commands with your user permissions");
        ui.info("  - Tools and channels may access the network");
        ui.info("  - WASM sandboxing limits untrusted code, but built-in tools run natively");
        ui.info("  - You are responsible for reviewing tool approvals");
        ui.blank();

        let accepted = ui.confirm("Do you understand and wish to proceed?", true)?;
        if accepted {
            Ok(StageOutcome::Completed)
        } else {
            Err(SetupError::Cancelled)
        }
    }
}
