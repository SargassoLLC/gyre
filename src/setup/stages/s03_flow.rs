//! Stage 3: Flow Selection
//!
//! QuickStart (~3 min): libSQL, auto keychain, single agent, TUI only.
//! Advanced (~10 min): full control over every setting.

use async_trait::async_trait;

use super::{SetupError, SetupStage, StageOutcome};
use crate::setup::state::SetupState;
use crate::setup::ui::SetupUi;

pub struct FlowStage;

#[async_trait]
impl SetupStage for FlowStage {
    fn id(&self) -> &'static str {
        "flow"
    }

    fn name(&self) -> &'static str {
        "Setup Mode"
    }

    async fn run(&self, state: &mut SetupState, ui: &SetupUi) -> Result<StageOutcome, SetupError> {
        // If QuickStart was pre-selected via --quick, skip the prompt.
        if state.quickstart {
            ui.info("QuickStart mode selected via --quick flag.");
            return Ok(StageOutcome::Completed);
        }

        let choice = ui.select_one(
            "Choose setup mode",
            &[
                "QuickStart  (~3 min) — sensible defaults, get running fast",
                "Advanced    (~10 min) — full control over every setting",
            ],
        )?;

        state.quickstart = choice == 0;

        if state.quickstart {
            ui.success("QuickStart mode — using sensible defaults where possible.");
        } else {
            ui.success("Advanced mode — you'll configure each setting.");
        }

        Ok(StageOutcome::Completed)
    }
}
