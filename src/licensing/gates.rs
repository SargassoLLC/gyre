//! Feature gate enforcement.
//!
//! Gates are enforced at the Rust runtime level inside the binary.
//! Config-file gating is trivially bypassed; here the binary IS the enforcer.
//!
//! # Usage
//!
//! ```rust,ignore
//! use gyre::licensing::gates::{check_agent_limit, require_feature};
//!
//! require_feature!(gates, tribe_enabled, "tribe", "Standard")?;
//! check_agent_limit(&gates, active_agents, &tier)?;
//! ```

use super::{FeatureGates, LicenseError, Tier};

/// Upgrade URL shown in all gate errors.
pub const UPGRADE_URL: &'static str = "https://gyre.ai/pricing";

// ─── Macro: require_feature! ──────────────────────────────────────────────────

/// Check that a boolean feature flag is enabled, returning `LicenseError` if not.
///
/// # Example
/// ```rust,ignore
/// require_feature!(gates, tribe_enabled, "tribe", "Standard")?;
/// ```
///
/// Expands to: if `!gates.<flag>` return `Err(LicenseError::FeatureNotAvailable { ... })`
#[macro_export]
macro_rules! require_feature {
    ($gates:expr, $flag:ident, $feature_name:expr, $required_tier:expr) => {{
        if !$gates.$flag {
            return Err($crate::licensing::LicenseError::FeatureNotAvailable {
                feature: $feature_name,
                required_tier: $required_tier,
                upgrade_url: $crate::licensing::gates::UPGRADE_URL,
            });
        }
    }};
}

// ─── Gate check functions ─────────────────────────────────────────────────────

/// Check if the current agent count is within the licensed limit.
///
/// # Arguments
/// * `gates` — current feature gates
/// * `current_count` — number of currently active agents
/// * `tier` — current tier (for error message)
///
/// # Returns
/// `Ok(())` if within limits, `Err(LicenseError::AgentLimitReached)` if not.
pub fn check_agent_limit(
    gates: &FeatureGates,
    current_count: usize,
    tier: &Tier,
) -> Result<(), LicenseError> {
    if let Some(max) = gates.max_agents {
        if current_count >= max as usize {
            return Err(LicenseError::AgentLimitReached {
                max,
                tier: tier.to_string(),
                upgrade_url: UPGRADE_URL,
            });
        }
    }
    Ok(())
}

/// Check if tribe (multi-agent orchestration) is enabled.
pub fn check_tribe_enabled(gates: &FeatureGates) -> Result<(), LicenseError> {
    if !gates.tribe_enabled {
        return Err(LicenseError::FeatureNotAvailable {
            feature: "tribe",
            required_tier: "Standard",
            upgrade_url: UPGRADE_URL,
        });
    }
    Ok(())
}

/// Check if the curiosity engine is enabled.
pub fn check_curiosity_enabled(gates: &FeatureGates) -> Result<(), LicenseError> {
    if !gates.curiosity_engine {
        return Err(LicenseError::FeatureNotAvailable {
            feature: "curiosity_engine",
            required_tier: "Standard",
            upgrade_url: UPGRADE_URL,
        });
    }
    Ok(())
}

/// Check if multi-tenant deployments are supported.
pub fn check_multi_tenant(gates: &FeatureGates) -> Result<(), LicenseError> {
    if !gates.multi_tenant {
        return Err(LicenseError::FeatureNotAvailable {
            feature: "multi_tenant",
            required_tier: "Pro",
            upgrade_url: UPGRADE_URL,
        });
    }
    Ok(())
}

/// Check if white-label branding is enabled.
pub fn check_white_label(gates: &FeatureGates) -> Result<(), LicenseError> {
    if !gates.white_label {
        return Err(LicenseError::FeatureNotAvailable {
            feature: "white_label",
            required_tier: "Enterprise",
            upgrade_url: UPGRADE_URL,
        });
    }
    Ok(())
}

/// Check memory retention days — warn if we'd be pruning beyond the limit.
///
/// Returns `None` if no limit (unlimited), `Some(days)` if limited.
pub fn memory_retention_days(gates: &FeatureGates) -> Option<u32> {
    gates.memory_days
}

// ─── Summary display ──────────────────────────────────────────────────────────

/// Build a human-readable feature summary for `gyre license status`.
pub fn format_feature_summary(gates: &FeatureGates, tier: &Tier) -> String {
    let max_agents = gates
        .max_agents
        .map(|n| n.to_string())
        .unwrap_or_else(|| "unlimited".to_string());

    let memory = gates
        .memory_days
        .map(|d| format!("{} days", d))
        .unwrap_or_else(|| "unlimited".to_string());

    let mut lines = vec![
        format!("  Tier:              {}", tier.display_name()),
        format!("  Max agents:        {}", max_agents),
        format!("  Memory retention:  {}", memory),
        format!("  Tribe:             {}", yn(gates.tribe_enabled)),
        format!("  Curiosity engine:  {}", yn(gates.curiosity_engine)),
        format!("  Multi-tenant:      {}", yn(gates.multi_tenant)),
        format!("  Priority support:  {}", yn(gates.priority_support)),
        format!("  White label:       {}", yn(gates.white_label)),
    ];

    if tier == &Tier::Free {
        lines.push(String::new());
        lines.push(format!("  Upgrade at {}", UPGRADE_URL));
    }

    lines.join("\n")
}

fn yn(b: bool) -> &'static str {
    if b { "✅" } else { "❌" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::licensing::FeatureGates;

    #[test]
    fn test_agent_limit_free() {
        let gates = FeatureGates::for_tier(&Tier::Free);
        assert!(check_agent_limit(&gates, 0, &Tier::Free).is_ok());
        assert!(check_agent_limit(&gates, 1, &Tier::Free).is_err()); // max=1, count=1 is at limit
    }

    #[test]
    fn test_agent_limit_pro_unlimited() {
        let gates = FeatureGates::for_tier(&Tier::Pro);
        // Pro has None (unlimited) max_agents
        assert!(check_agent_limit(&gates, 999, &Tier::Pro).is_ok());
    }

    #[test]
    fn test_tribe_free_blocked() {
        let gates = FeatureGates::for_tier(&Tier::Free);
        assert!(check_tribe_enabled(&gates).is_err());
    }

    #[test]
    fn test_tribe_standard_allowed() {
        let gates = FeatureGates::for_tier(&Tier::Standard);
        assert!(check_tribe_enabled(&gates).is_ok());
    }

    #[test]
    fn test_white_label_pro_blocked() {
        let gates = FeatureGates::for_tier(&Tier::Pro);
        assert!(check_white_label(&gates).is_err());
    }

    #[test]
    fn test_white_label_enterprise_allowed() {
        let gates = FeatureGates::for_tier(&Tier::Enterprise);
        assert!(check_white_label(&gates).is_ok());
    }
}
