//! Stage 2: Config State Detection
//!
//! Scan for existing configuration files, databases, API keys,
//! and agent boxes. Report findings and classify as Fresh/Legacy/Existing.

use std::path::PathBuf;

use async_trait::async_trait;

use super::{SetupError, SetupStage, StageOutcome};
use crate::setup::state::{DetectedConfig, InstallType, SetupState};
use crate::setup::ui::SetupUi;

pub struct DetectStage;

#[async_trait]
impl SetupStage for DetectStage {
    fn id(&self) -> &'static str {
        "detect"
    }

    fn name(&self) -> &'static str {
        "Configuration Detection"
    }

    async fn run(&self, state: &mut SetupState, ui: &SetupUi) -> Result<StageOutcome, SetupError> {
        ui.info("Scanning for existing configuration...");

        let gyre_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".gyre");

        let mut detected = DetectedConfig::default();

        // Check for .env file.
        detected.has_env_file = gyre_dir.join(".env").exists();

        // Check for settings.json (legacy).
        detected.has_settings_json = gyre_dir.join("settings.json").exists();

        // Check for config.toml (new format).
        detected.has_config_toml = gyre_dir.join("config.toml").exists();

        // Scan for agent hermit boxes.
        let agents_dir = &state.agents_dir;
        if agents_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(agents_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() && path.join("soul.md").exists() {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            detected.agent_box_names.push(name.to_string());
                        }
                    }
                }
            }
        }

        // Check for existing LLM API key in environment.
        detected.has_llm_key =
            std::env::var("ANTHROPIC_API_KEY").is_ok() || std::env::var("OPENAI_API_KEY").is_ok();

        // Classify installation type.
        detected.install_type = if detected.has_config_toml || detected.has_env_file {
            InstallType::Existing
        } else if detected.has_settings_json {
            InstallType::Legacy
        } else {
            InstallType::Fresh
        };

        // Report findings.
        match detected.install_type {
            InstallType::Fresh => {
                ui.info("No existing configuration found. Starting fresh install.");
            }
            InstallType::Legacy => {
                ui.info("Found legacy settings.json. Will migrate to new format.");
                // Load legacy settings into state.
                let legacy = crate::settings::Settings::load();
                state.settings = legacy;
            }
            InstallType::Existing => {
                ui.info("Found existing configuration.");
                // Load existing settings so later stages can detect what's already configured.
                // Prefer config.toml (new format), fall back to settings.json (legacy).
                if detected.has_config_toml {
                    let toml_path = crate::settings::Settings::default_toml_path();
                    if let Ok(Some(s)) = crate::settings::Settings::load_toml(&toml_path) {
                        state.settings = s;
                    }
                } else {
                    state.settings = crate::settings::Settings::load();
                }
                if detected.has_config_toml {
                    ui.success("  config.toml present");
                }
                if detected.has_env_file {
                    ui.success("  .env present");
                }
            }
        }

        if !detected.agent_box_names.is_empty() {
            ui.success(&format!(
                "  Found {} agent box(es): {}",
                detected.agent_box_names.len(),
                detected.agent_box_names.join(", ")
            ));
        }

        if detected.has_llm_key {
            ui.success("  LLM API key detected in environment");
        }

        state.detected = detected;
        Ok(StageOutcome::Completed)
    }
}
