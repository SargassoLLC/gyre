//! SetupEngine — orchestrator for the 10-stage setup wizard.
//!
//! Runs stages sequentially with skip/back support, tracks metadata,
//! and supports `--reconfigure <stage>` for re-running individual stages.

use super::stages::{self, SetupError, StageOutcome};
use super::state::{SetupState, StageStatus};
use super::ui::SetupUi;

/// Orchestrates the multi-stage setup flow.
pub struct SetupEngine {
    ui: SetupUi,
    state: SetupState,
    /// If set, only run the stage with this ID.
    reconfigure_stage: Option<String>,
    /// Skip the risk acknowledgment stage.
    skip_risk_ack: bool,
}

impl SetupEngine {
    /// Create a new engine with an interactive UI.
    pub fn new(ui: SetupUi) -> Self {
        Self {
            ui,
            state: SetupState::new(),
            reconfigure_stage: None,
            skip_risk_ack: false,
        }
    }

    /// Set QuickStart mode.
    pub fn with_quickstart(mut self, quickstart: bool) -> Self {
        self.state.quickstart = quickstart;
        self
    }

    /// Reconfigure only a single stage.
    pub fn with_reconfigure(mut self, stage_id: Option<String>) -> Self {
        self.reconfigure_stage = stage_id;
        self
    }

    /// Skip the risk acknowledgment stage.
    pub fn with_skip_risk_ack(mut self, skip: bool) -> Self {
        self.skip_risk_ack = skip;
        self
    }

    /// Set the agents directory.
    pub fn with_agents_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.state.agents_dir = dir;
        self
    }

    /// Run the full setup flow (or a single stage if reconfiguring).
    pub async fn run(mut self) -> Result<(), SetupError> {
        let all = stages::all_stages();
        let total = all.len();

        self.ui.header("Gyre Setup");

        // Single-stage reconfiguration mode.
        if let Some(ref stage_id) = self.reconfigure_stage {
            let stage = all
                .iter()
                .find(|s| s.id() == stage_id.as_str())
                .ok_or_else(|| {
                    SetupError::Validation(format!(
                        "Unknown stage '{}'. Available: {}",
                        stage_id,
                        all.iter().map(|s| s.id()).collect::<Vec<_>>().join(", ")
                    ))
                })?;

            self.ui
                .info(&format!("Reconfiguring stage: {}", stage.name()));
            self.ui.blank();
            stage.run(&mut self.state, &self.ui).await?;
            self.state.mark_completed(stage.id());
            self.ui.success("Reconfiguration complete.");
            return Ok(());
        }

        // Full setup flow.
        let mut idx = 0;
        while idx < total {
            let stage = &all[idx];

            // Skip risk ack if --skip-risk-ack was passed.
            if stage.id() == "risk_ack" && self.skip_risk_ack {
                self.state.mark_skipped(stage.id());
                idx += 1;
                continue;
            }

            // Skip stages already satisfied.
            if stage.is_satisfied(&self.state).await {
                self.ui
                    .success(&format!("{} — already configured", stage.name()));
                self.state.mark_skipped(stage.id());
                idx += 1;
                continue;
            }

            // Skip QuickStart-skippable stages when in QuickStart mode.
            if self.state.quickstart && stage.skippable_in_quickstart() {
                self.state.mark_skipped(stage.id());
                idx += 1;
                continue;
            }

            self.ui.step(idx + 1, total, stage.name());

            match stage.run(&mut self.state, &self.ui).await? {
                StageOutcome::Completed => {
                    self.state.mark_completed(stage.id());
                    idx += 1;
                }
                StageOutcome::Skipped => {
                    self.state.mark_skipped(stage.id());
                    idx += 1;
                }
                StageOutcome::GoBack => {
                    if idx > 0 {
                        idx -= 1;
                    }
                }
                StageOutcome::Retry => {
                    // Stay on the same stage.
                }
            }

            self.ui.blank();
        }

        // Final status.
        let completed = self
            .state
            .stage_status
            .values()
            .filter(|s| **s == StageStatus::Completed)
            .count();
        let skipped = self
            .state
            .stage_status
            .values()
            .filter(|s| **s == StageStatus::Skipped)
            .count();

        self.ui.header("Setup Complete");
        self.ui.success(&format!(
            "{} stages completed, {} skipped",
            completed, skipped
        ));

        Ok(())
    }
}
