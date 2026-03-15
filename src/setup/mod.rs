//! Interactive setup wizard for Gyre.
//!
//! Provides a guided setup experience through 10 stages:
//! 1. Risk acknowledgment
//! 2. Config state detection
//! 3. Flow selection (QuickStart / Advanced)
//! 4. Database connection
//! 5. Security (secrets master key)
//! 6. Auth & model selection
//! 7. Agent creation (multi-agent)
//! 8. Channel & binding setup
//! 9. Gateway & daemon
//! 10. Finalization
//!
//! # Example
//!
//! ```ignore
//! use gyre::setup::{SetupEngine, SetupUi};
//!
//! let ui = SetupUi::new();
//! let engine = SetupEngine::new(ui);
//! engine.run().await?;
//! ```

pub mod config_schema;
pub mod engine;
pub mod stages;
pub mod state;
pub mod ui;

pub use engine::SetupEngine;
pub use stages::{SetupError, SetupStage, StageOutcome};
pub use state::SetupState;
pub use ui::SetupUi;
