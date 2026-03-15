//! Multi-agent configuration schema.
//!
//! Extends the base Settings with multi-agent support, per-agent
//! channel bindings, and gateway configuration.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Multi-agent configuration.
///
/// When `agents` is non-empty, Gyre runs multiple cognitive agents,
/// each with their own hermit box, optional LLM override, and
/// channel bindings. When empty, falls back to single-agent mode.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MultiAgentSettings {
    /// Default agent ID (used when a channel message has no explicit routing).
    #[serde(default)]
    pub default_agent: Option<String>,

    /// Base directory containing agent hermit boxes.
    #[serde(default)]
    pub agents_dir: Option<PathBuf>,

    /// Agent definitions.
    #[serde(default)]
    pub agents: Vec<AgentDefinition>,
}

/// Definition of a single agent in a multi-agent configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    /// Unique agent identifier (alphanumeric + hyphens + underscores).
    pub id: String,

    /// Human-readable display name.
    #[serde(default)]
    pub name: Option<String>,

    /// LLM backend override (falls back to global `llm_backend`).
    #[serde(default)]
    pub llm_backend: Option<String>,

    /// Model override (falls back to global `selected_model`).
    #[serde(default)]
    pub model: Option<String>,

    /// Channel bindings for this agent.
    #[serde(default)]
    pub channels: Vec<ChannelBinding>,

    /// Whether this is the primary agent (receives unbound channel messages).
    #[serde(default)]
    pub primary: bool,
}

/// Binding of a channel instance to an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelBinding {
    /// Channel type: "telegram", "web", "http", "tui", etc.
    pub channel_type: String,

    /// Instance disambiguator (for multiple channels of the same type).
    #[serde(default)]
    pub name: Option<String>,

    /// Channel-specific configuration overrides.
    #[serde(default)]
    pub config: HashMap<String, toml::Value>,
}

/// Web gateway configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewaySettings {
    /// Whether the daemon/service is enabled.
    #[serde(default)]
    pub daemon_enabled: bool,

    /// Whether the web gateway is enabled.
    #[serde(default)]
    pub web_enabled: bool,

    /// Web gateway port.
    #[serde(default = "default_gateway_port")]
    pub web_port: u16,

    /// Web gateway host.
    #[serde(default = "default_gateway_host")]
    pub web_host: String,

    /// Authentication token for the web gateway.
    #[serde(default)]
    pub web_auth_token: Option<String>,
}

fn default_gateway_port() -> u16 {
    3000
}

fn default_gateway_host() -> String {
    "127.0.0.1".to_string()
}

impl Default for GatewaySettings {
    fn default() -> Self {
        Self {
            daemon_enabled: false,
            web_enabled: false,
            web_port: default_gateway_port(),
            web_host: default_gateway_host(),
            web_auth_token: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_agent_settings_default_is_empty() {
        let settings = MultiAgentSettings::default();
        assert!(settings.agents.is_empty());
        assert!(settings.default_agent.is_none());
    }

    #[test]
    fn gateway_settings_default_values() {
        let gw = GatewaySettings::default();
        assert!(!gw.daemon_enabled);
        assert!(!gw.web_enabled);
        assert_eq!(gw.web_port, 3000);
        assert_eq!(gw.web_host, "127.0.0.1");
    }

    #[test]
    fn agent_definition_serde_round_trip() {
        let agent = AgentDefinition {
            id: "kimi".to_string(),
            name: Some("Kimi".to_string()),
            llm_backend: Some("anthropic".to_string()),
            model: Some("claude-sonnet-4-20250514".to_string()),
            channels: vec![ChannelBinding {
                channel_type: "telegram".to_string(),
                name: None,
                config: HashMap::new(),
            }],
            primary: true,
        };

        let json = serde_json::to_string(&agent).unwrap();
        let restored: AgentDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, "kimi");
        assert!(restored.primary);
        assert_eq!(restored.channels.len(), 1);
    }
}
