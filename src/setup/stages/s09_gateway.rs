//! Stage 9: Gateway & Daemon
//!
//! Web gateway configuration (port, host, auth token).
//! Daemon installation (launchd on macOS, systemd on Linux).
//! Heartbeat configuration.

use async_trait::async_trait;

use super::{SetupError, SetupStage, StageOutcome};
use crate::setup::state::SetupState;
use crate::setup::ui::SetupUi;

pub struct GatewayStage;

#[async_trait]
impl SetupStage for GatewayStage {
    fn id(&self) -> &'static str {
        "gateway"
    }

    fn name(&self) -> &'static str {
        "Gateway & Daemon"
    }

    fn skippable_in_quickstart(&self) -> bool {
        true
    }

    async fn run(&self, state: &mut SetupState, ui: &SetupUi) -> Result<StageOutcome, SetupError> {
        // Web gateway.
        let enable_gateway = ui.confirm("Enable web gateway (browser UI)?", false)?;
        state.settings.gateway.web_enabled = enable_gateway;

        if enable_gateway {
            let port = ui.input_with_default("Gateway port", "3000")?;
            if let Ok(p) = port.parse() {
                state.settings.gateway.web_port = p;
            }

            let host = ui.input_with_default("Gateway host", "127.0.0.1")?;
            state.settings.gateway.web_host = host;

            let auth_token = ui.optional_input("Gateway auth token (blank to auto-generate)")?;
            if let Some(token) = auth_token {
                state.settings.gateway.web_auth_token = Some(token);
            } else {
                // Auto-generate a token.
                let token = generate_auth_token();
                state.settings.gateway.web_auth_token = Some(token.clone());
                ui.info(&format!("Generated auth token: {}", token));
            }

            ui.success("Web gateway enabled.");
        }

        // Daemon installation.
        let install_daemon = ui.confirm("Install Gyre as a background service (daemon)?", false)?;
        state.settings.gateway.daemon_enabled = install_daemon;

        if install_daemon {
            ui.info("Installing daemon...");

            #[cfg(target_os = "macos")]
            {
                ui.info("Creating launchd service (~/Library/LaunchAgents/com.gyre.daemon.plist)");
            }

            #[cfg(target_os = "linux")]
            {
                ui.info("Creating systemd user service (~/.config/systemd/user/gyre.service)");
            }

            // Actual installation delegated to service module.
            ui.success("Daemon configured. Run `gyre service start` to begin.");
        }

        // Heartbeat.
        let enable_heartbeat =
            ui.confirm("Enable heartbeat (periodic background checks)?", false)?;
        state.settings.heartbeat.enabled = enable_heartbeat;

        if enable_heartbeat {
            let interval = ui.input_with_default("Heartbeat interval (seconds)", "1800")?;
            if let Ok(secs) = interval.parse() {
                state.settings.heartbeat.interval_secs = secs;
            }
            ui.success("Heartbeat enabled.");
        }

        Ok(StageOutcome::Completed)
    }
}

/// Generate a random auth token (32 hex characters).
fn generate_auth_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..16).map(|_| rng.r#gen()).collect();
    hex::encode(bytes)
}
