//! Gyre licensing system — core types.
//!
//! # Key format
//! `GYRE-XXXX-XXXX-XXXX-XXXX-XXXX` (base32, 100-bit entropy, opaque)
//!
//! # Tier hierarchy
//! Free → Standard ($99/mo) → Pro ($199/mo) → Enterprise ($399/mo)
//!
//! # Offline grace
//! 72h offline → degrade to Free tier (not hard lockout)

pub mod gates;
pub mod validator;

pub use validator::validate_license;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─── Tier ────────────────────────────────────────────────────────────────────

/// License tier.  Tier is determined server-side; the key itself is opaque.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    #[default]
    Free,
    Standard,
    Pro,
    Enterprise,
}

impl Tier {
    pub fn display_name(&self) -> &str {
        match self {
            Tier::Free => "Free",
            Tier::Standard => "Standard ($99/mo)",
            Tier::Pro => "Pro ($199/mo)",
            Tier::Enterprise => "Enterprise ($399/mo)",
        }
    }

    pub fn seat_limit(&self) -> Option<u32> {
        match self {
            Tier::Free => Some(1),
            Tier::Standard => Some(3),
            Tier::Pro => Some(10),
            Tier::Enterprise => None, // unlimited
        }
    }
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

// ─── FeatureGates ────────────────────────────────────────────────────────────

/// Feature flags derived from the active license tier.
///
/// These are enforced at the Rust runtime level — not in config files.
/// Config-based gating is trivially bypassed; the binary is the enforcer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureGates {
    /// Maximum number of concurrent agents (None = unlimited).
    pub max_agents: Option<u32>,
    /// Tribe (multi-agent orchestration) is available.
    pub tribe_enabled: bool,
    /// Background curiosity/research engine is available.
    pub curiosity_engine: bool,
    /// How many days of memory to retain (None = unlimited).
    pub memory_days: Option<u32>,
    /// Multi-tenant deployments supported.
    pub multi_tenant: bool,
    /// Priority support SLA.
    pub priority_support: bool,
    /// White-label branding.
    pub white_label: bool,
}

impl FeatureGates {
    /// Build feature gates for a given tier, matching the security spec §1.4.
    pub fn for_tier(tier: &Tier) -> Self {
        match tier {
            Tier::Free => Self {
                max_agents: Some(1),
                tribe_enabled: false,
                curiosity_engine: false,
                memory_days: Some(7),
                multi_tenant: false,
                priority_support: false,
                white_label: false,
            },
            Tier::Standard => Self {
                max_agents: Some(3),
                tribe_enabled: true,
                curiosity_engine: true,
                memory_days: None,
                multi_tenant: false,
                priority_support: false,
                white_label: false,
            },
            Tier::Pro => Self {
                max_agents: None,
                tribe_enabled: true,
                curiosity_engine: true,
                memory_days: None,
                multi_tenant: true,
                priority_support: true,
                white_label: false,
            },
            Tier::Enterprise => Self {
                max_agents: None,
                tribe_enabled: true,
                curiosity_engine: true,
                memory_days: None,
                multi_tenant: true,
                priority_support: true,
                white_label: true,
            },
        }
    }

    /// Free-tier gates (used when grace period expires or no license present).
    pub fn free() -> Self {
        Self::for_tier(&Tier::Free)
    }
}

// ─── License ─────────────────────────────────────────────────────────────────

/// A validated license with full metadata from the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct License {
    /// The license key (redacted in display).
    pub key: String,
    /// License tier.
    pub tier: Tier,
    /// Feature gates derived from this license.
    pub gates: FeatureGates,
    /// Customer email (for display only).
    pub customer_email: Option<String>,
    /// When this license was created (ISO 8601).
    pub created_at: Option<String>,
    /// Expiry date if set (None = perpetual subscription).
    pub expires_at: Option<String>,
    /// Seat limit for this key.
    pub seat_limit: Option<u32>,
    /// How many seats are currently registered.
    pub seats_registered: u32,
    /// Whether this machine is registered in the seat list.
    pub this_machine_registered: bool,
    /// When the local cache was last validated (Unix timestamp).
    pub validated_at: u64,
}

impl License {
    /// Redacted key for display: `GYRE-A3F8-****-****-****-****`.
    pub fn display_key(&self) -> String {
        let parts: Vec<&str> = self.key.splitn(3, '-').collect();
        if parts.len() >= 2 {
            format!("{}-{}****", parts[0], parts[1])
        } else {
            "GYRE-****".to_string()
        }
    }
}

// ─── LicenseStatus ───────────────────────────────────────────────────────────

/// Result of license validation.  Drives UX and feature enforcement.
#[derive(Debug, Clone)]
pub enum LicenseStatus {
    /// Server confirmed valid, or local cache is fresh (< 24h old).
    Valid(License),
    /// Offline and within 72h grace window.  Full tier features still active.
    GracePeriod(License),
    /// Offline > 72h.  Degraded to Free tier.
    GraceExpired,
    /// Server explicitly rejected the key (revoked, not found, seat limit, etc.).
    Invalid(String),
    /// No license key has ever been activated on this machine.
    NoLicense,
}

impl LicenseStatus {
    /// Returns the active license if the status is Valid or GracePeriod.
    pub fn license(&self) -> Option<&License> {
        match self {
            LicenseStatus::Valid(l) | LicenseStatus::GracePeriod(l) => Some(l),
            _ => None,
        }
    }

    /// Returns the feature gates for the current status.
    ///
    /// Grace expired and no-license both fall back to Free tier.
    pub fn feature_gates(&self) -> FeatureGates {
        match self {
            LicenseStatus::Valid(l) | LicenseStatus::GracePeriod(l) => l.gates.clone(),
            LicenseStatus::GraceExpired | LicenseStatus::Invalid(_) | LicenseStatus::NoLicense => {
                FeatureGates::free()
            }
        }
    }

    /// Returns the effective tier for the current status.
    pub fn effective_tier(&self) -> Tier {
        match self {
            LicenseStatus::Valid(l) | LicenseStatus::GracePeriod(l) => l.tier.clone(),
            _ => Tier::Free,
        }
    }

    /// True if the user has access to a paid tier (either online or in grace).
    pub fn is_paid(&self) -> bool {
        matches!(
            self,
            LicenseStatus::Valid(_) | LicenseStatus::GracePeriod(_)
        ) && self.effective_tier() != Tier::Free
    }
}

// ─── LicenseError ────────────────────────────────────────────────────────────

/// Errors returned by the licensing system.
#[derive(Debug, Error)]
pub enum LicenseError {
    #[error("Feature '{feature}' requires {required_tier} or higher. Upgrade at {upgrade_url}")]
    FeatureNotAvailable {
        feature: &'static str,
        required_tier: &'static str,
        upgrade_url: &'static str,
    },

    #[error(
        "Agent limit reached: maximum {max} agents allowed on {tier}. Upgrade at {upgrade_url}"
    )]
    AgentLimitReached {
        max: u32,
        tier: String,
        upgrade_url: &'static str,
    },

    #[error("Memory retention limit: {tier} keeps {days} days. Upgrade at {upgrade_url}")]
    MemoryLimitReached {
        tier: String,
        days: u32,
        upgrade_url: &'static str,
    },

    #[error("License validation failed: {0}")]
    ValidationFailed(String),

    #[error("Cache error: {0}")]
    CacheError(String),

    #[error("Network error during license validation: {0}")]
    NetworkError(String),

    #[error("Invalid key format: expected GYRE-XXXX-XXXX-XXXX-XXXX-XXXX")]
    InvalidKeyFormat,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

// ─── Key validation helpers ───────────────────────────────────────────────────

/// Validate that a key matches the `GYRE-XXXX-XXXX-XXXX-XXXX-XXXX` format.
///
/// Character set: alphanumeric, excluding ambiguous chars (0, O, I, 1) for readability.
/// The spec describes this as "base32-like" — opaque, high-entropy tokens.
pub fn is_valid_key_format(key: &str) -> bool {
    let key = key.trim().to_uppercase();
    let parts: Vec<&str> = key.split('-').collect();
    if parts.len() != 6 {
        return false;
    }
    if parts[0] != "GYRE" {
        return false;
    }
    // Each of the 5 segments must be exactly 4 alphanumeric characters.
    // We allow A-Z and 2-9 (excludes 0/O and 1/I as ambiguous).
    parts[1..].iter().all(|p| {
        p.len() == 4
            && p.chars().all(|c| {
                matches!(c,
                    'A'..='H' | 'J'..='N' | 'P'..='Z' |  // letters except I and O
                    '2'..='9'                               // digits except 0 and 1
                )
            })
    })
}

/// Normalize a key: strip whitespace, uppercase.
pub fn normalize_key(key: &str) -> String {
    key.trim().to_uppercase()
}

// ─── Startup banner ───────────────────────────────────────────────────────────

/// Print the license status banner on `gyre serve` / `gyre run` startup.
pub fn print_startup_banner(status: &LicenseStatus) {
    const UPGRADE_URL: &str = "https://gyre.ai/pricing";
    match status {
        LicenseStatus::Valid(lic) => {
            println!(
                "[license] ✅  {} tier{}",
                lic.tier.display_name(),
                lic.customer_email
                    .as_deref()
                    .map(|e| format!(" — {}", e))
                    .unwrap_or_default()
            );
            if lic.tier == Tier::Free {
                println!(
                    "[license]     Upgrade at {} to unlock tribe + curiosity engine.",
                    UPGRADE_URL
                );
            }
        }
        LicenseStatus::GracePeriod(lic) => {
            let hours = validator::grace_hours_remaining(lic.validated_at);
            if hours < 48 {
                println!(
                    "[license] ⚠️  {} — offline grace period active ({} h remaining).",
                    lic.tier.display_name(),
                    hours
                );
            } else {
                println!("╔══════════════════════════════════════════════════╗");
                println!("║  ⚠️  LICENSE CHECK REQUIRED                       ║");
                println!(
                    "║  Offline for too long. {}h until Free tier.        ║",
                    hours
                );
                println!("║  Connect to the internet to re-validate.          ║");
                println!("╚══════════════════════════════════════════════════╝");
            }
        }
        LicenseStatus::GraceExpired => {
            println!("[license] ⚠️  Grace period expired. Running on Free tier.");
            println!("[license]     Reconnect and run `gyre license activate <KEY>` to restore.");
        }
        LicenseStatus::Invalid(reason) => {
            println!("[license] ❌  License invalid: {}.", reason);
            println!(
                "[license]     Running on Free tier. Upgrade at {}.",
                UPGRADE_URL
            );
        }
        LicenseStatus::NoLicense => {
            println!("[license] ℹ️   No license configured. Running on Free tier.");
            println!(
                "[license]     Upgrade at {} to unlock tribe + curiosity engine.",
                UPGRADE_URL
            );
            println!("[license]     Activate with: gyre license activate <KEY>");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_format_valid() {
        // Valid: A-Z (excluding I, O) and 2-9 (excluding 0, 1)
        assert!(is_valid_key_format("GYRE-A3F8-B2C9-D4E7-F3A2-B3C4"));
        assert!(is_valid_key_format("gyre-a3f8-b2c9-d4e7-f3a2-b3c4")); // lowercase OK
        assert!(is_valid_key_format("GYRE-TEST-TEST-TEST-TEST-TEST")); // all letters
    }

    #[test]
    fn test_key_format_invalid() {
        assert!(!is_valid_key_format("GYRE-A3F8-B2C9-D4E7-F3A2")); // too short
        assert!(!is_valid_key_format("NOTGYRE-A3F8-B2C9-D4E7-F3A2-B3C4")); // wrong prefix
        assert!(!is_valid_key_format("")); // empty
        assert!(!is_valid_key_format("GYRE-O000-AAAA-BBBB-CCCC-DDDD")); // contains O, 0
    }

    #[test]
    fn test_feature_gates_free() {
        let gates = FeatureGates::for_tier(&Tier::Free);
        assert_eq!(gates.max_agents, Some(1));
        assert!(!gates.tribe_enabled);
        assert!(!gates.curiosity_engine);
        assert_eq!(gates.memory_days, Some(7));
    }

    #[test]
    fn test_feature_gates_enterprise() {
        let gates = FeatureGates::for_tier(&Tier::Enterprise);
        assert!(gates.max_agents.is_none());
        assert!(gates.tribe_enabled);
        assert!(gates.white_label);
    }

    #[test]
    fn test_tier_ordering() {
        assert!(Tier::Free < Tier::Standard);
        assert!(Tier::Standard < Tier::Pro);
        assert!(Tier::Pro < Tier::Enterprise);
    }
}
