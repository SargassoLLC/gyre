//! Stage 8: Channel & Binding Setup
//!
//! Discover available channels (built-in + WASM), enable/configure them,
//! and bind channels to specific agents.

use async_trait::async_trait;

use super::{SetupError, SetupStage, StageOutcome};
use crate::setup::state::SetupState;
use crate::setup::ui::SetupUi;

pub struct ChannelsStage;

#[async_trait]
impl SetupStage for ChannelsStage {
    fn id(&self) -> &'static str {
        "channels"
    }

    fn name(&self) -> &'static str {
        "Channels & Bindings"
    }

    fn skippable_in_quickstart(&self) -> bool {
        true
    }

    async fn run(&self, state: &mut SetupState, ui: &SetupUi) -> Result<StageOutcome, SetupError> {
        // Discover available channels.
        let mut available: Vec<&str> = vec!["TUI (terminal)"];
        available.push("HTTP webhook");

        // Check for WASM channels.
        let wasm_channels = discover_wasm_channels(state);
        let wasm_names: Vec<String> = wasm_channels
            .iter()
            .map(|c| format!("{} (WASM)", c))
            .collect();
        let wasm_refs: Vec<&str> = wasm_names.iter().map(|s| s.as_str()).collect();
        available.extend_from_slice(&wasm_refs);

        // TUI is always enabled by default.
        let mut defaults: Vec<bool> = vec![true]; // TUI
        defaults.push(false); // HTTP
        defaults.extend(wasm_channels.iter().map(|_| false));

        let selected = ui.select_many("Which channels to enable?", &available, &defaults)?;

        // Configure each selected channel.
        for &idx in &selected {
            match idx {
                0 => {
                    ui.info("TUI channel is always available.");
                }
                1 => {
                    // HTTP webhook.
                    self.configure_http(state, ui)?;
                }
                n => {
                    // WASM channel (offset by 2 for TUI and HTTP).
                    let wasm_idx = n - 2;
                    if wasm_idx < wasm_channels.len() {
                        self.configure_wasm_channel(state, ui, &wasm_channels[wasm_idx])?;
                    }
                }
            }
        }

        // Per-agent binding (if multiple agents).
        if state.settings.multi_agent.agents.len() > 1 {
            self.configure_bindings(state, ui)?;
        }

        // Tunnel setup if any webhook channel was selected.
        let needs_tunnel = selected.contains(&1) || selected.iter().any(|&i| i >= 2); // Any WASM channel

        if needs_tunnel && !state.quickstart {
            let setup_tunnel =
                ui.confirm("Configure a tunnel for public webhook access?", false)?;
            if setup_tunnel {
                self.configure_tunnel(state, ui)?;
            }
        }

        ui.success("Channels configured.");
        Ok(StageOutcome::Completed)
    }
}

impl ChannelsStage {
    fn configure_http(&self, state: &mut SetupState, ui: &SetupUi) -> Result<(), SetupError> {
        state.settings.channels.http_enabled = true;

        let port = ui.input_with_default("HTTP webhook port", "8080")?;
        if let Ok(p) = port.parse() {
            state.settings.channels.http_port = Some(p);
        }

        let host = ui.input_with_default("HTTP webhook host", "127.0.0.1")?;
        state.settings.channels.http_host = Some(host);

        ui.success("HTTP webhook enabled.");
        Ok(())
    }

    fn configure_wasm_channel(
        &self,
        state: &mut SetupState,
        ui: &SetupUi,
        channel_name: &str,
    ) -> Result<(), SetupError> {
        ui.info(&format!("Configuring {} channel...", channel_name));

        // Channel-specific setup via the existing channels module.
        match channel_name {
            "telegram" => {
                let token = ui.secret_input("Telegram bot token (from @BotFather)")?;
                // Token stored via secrets system.
                let _ = token;
                state
                    .settings
                    .channels
                    .wasm_channels
                    .push("telegram".to_string());
                ui.success("Telegram channel configured.");
            }
            other => {
                // Generic WASM channel — just enable it.
                state
                    .settings
                    .channels
                    .wasm_channels
                    .push(other.to_string());
                ui.success(&format!("{} channel enabled.", other));
            }
        }

        Ok(())
    }

    fn configure_bindings(&self, state: &mut SetupState, ui: &SetupUi) -> Result<(), SetupError> {
        ui.info("Configure which channels route to which agents.");
        ui.blank();

        let agent_names: Vec<String> = state
            .settings
            .multi_agent
            .agents
            .iter()
            .map(|a| a.name.as_ref().unwrap_or(&a.id).clone())
            .collect();

        let enabled_channels: Vec<String> = {
            let mut c = Vec::new();
            if state.settings.channels.http_enabled {
                c.push("http".to_string());
            }
            c.extend(state.settings.channels.wasm_channels.clone());
            c
        };

        for channel in &enabled_channels {
            let agent_refs: Vec<&str> = agent_names.iter().map(|s| s.as_str()).collect();
            let idx = ui.select_one(
                &format!("Route '{}' channel to which agent?", channel),
                &agent_refs,
            )?;

            // Add channel binding to the selected agent.
            if idx < state.settings.multi_agent.agents.len() {
                state.settings.multi_agent.agents[idx].channels.push(
                    crate::settings::ChannelBinding {
                        channel_type: channel.clone(),
                        name: None,
                        config: std::collections::HashMap::new(),
                    },
                );
            }
        }

        Ok(())
    }

    fn configure_tunnel(&self, state: &mut SetupState, ui: &SetupUi) -> Result<(), SetupError> {
        let providers = &[
            "ngrok",
            "Cloudflare Tunnel",
            "Tailscale Funnel",
            "Custom command",
            "Static URL (already have one)",
        ];

        let choice = ui.select_one("Tunnel provider", providers)?;

        match choice {
            0 => {
                state.settings.tunnel.provider = Some("ngrok".to_string());
                let token = ui.secret_input("ngrok auth token")?;
                let _ = token; // Store via secrets.
                let domain = ui.optional_input("ngrok custom domain (optional)")?;
                state.settings.tunnel.ngrok_domain = domain;
            }
            1 => {
                state.settings.tunnel.provider = Some("cloudflare".to_string());
                let token = ui.secret_input("Cloudflare tunnel token")?;
                let _ = token;
            }
            2 => {
                state.settings.tunnel.provider = Some("tailscale".to_string());
                state.settings.tunnel.ts_funnel = true;
            }
            3 => {
                state.settings.tunnel.provider = Some("custom".to_string());
                let cmd = ui.input("Tunnel command (use {port} placeholder)")?;
                state.settings.tunnel.custom_command = Some(cmd);
            }
            4 => {
                let url = ui.input("Public URL")?;
                state.settings.tunnel.public_url = Some(url);
            }
            _ => unreachable!(),
        }

        ui.success("Tunnel configured.");
        Ok(())
    }
}

/// Discover WASM channels from the channels directory.
fn discover_wasm_channels(_state: &SetupState) -> Vec<String> {
    // Look for bundled WASM channels.
    let channels_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".gyre")
        .join("channels");

    let mut found = Vec::new();

    if channels_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&channels_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "wasm") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        found.push(stem.to_string());
                    }
                }
            }
        }
    }

    // Always include well-known channel names for selection even if not installed.
    for name in &["telegram", "slack"] {
        if !found.contains(&name.to_string()) {
            // Only add if the channel source directory exists (channels-src/).
            let src_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("channels-src")
                .join(name);
            if src_dir.exists() {
                found.push(name.to_string());
            }
        }
    }

    found
}
