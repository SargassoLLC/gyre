//! `gyre license` subcommands.
//!
//! ```text
//! gyre license activate GYRE-XXXX-XXXX-XXXX-XXXX-XXXX   # save key, run validation
//! gyre license status                                      # show tier, features, seats
//! gyre license deactivate                                  # remove key + cache
//! ```

use clap::Subcommand;

use crate::licensing::gates::format_feature_summary;
use crate::licensing::validator::{
    activate_license, deactivate_license, get_machine_id, grace_hours_remaining, load_and_validate,
};
use crate::licensing::{LicenseStatus, Tier, is_valid_key_format};

// ─── CLI types ────────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug, Clone)]
pub enum LicenseCommand {
    /// Activate a license key: validate with server and save locally.
    Activate {
        /// License key in GYRE-XXXX-XXXX-XXXX-XXXX-XXXX format.
        key: String,
    },
    /// Show current license tier, features, and seat usage.
    Status,
    /// Remove the stored license key and cache (returns to Free tier).
    Deactivate,
}

// ─── Command runner ───────────────────────────────────────────────────────────

pub async fn run_license_command(cmd: LicenseCommand) -> anyhow::Result<()> {
    match cmd {
        LicenseCommand::Activate { key } => run_activate(&key).await,
        LicenseCommand::Status => run_status().await,
        LicenseCommand::Deactivate => run_deactivate(),
    }
}

// ─── Activate ────────────────────────────────────────────────────────────────

async fn run_activate(key: &str) -> anyhow::Result<()> {
    let key_upper = key.trim().to_uppercase();

    if !is_valid_key_format(&key_upper) {
        eprintln!("❌ Invalid key format. Expected: GYRE-XXXX-XXXX-XXXX-XXXX-XXXX");
        eprintln!("   Got: {}", key);
        std::process::exit(1);
    }

    println!("🔑 Activating license {}...", redact_key(&key_upper));

    match activate_license(&key_upper).await {
        Ok(LicenseStatus::Valid(license)) => {
            println!("✅ License activated successfully!");
            println!();
            println!("  Tier:    {}", license.tier.display_name());
            if let Some(email) = &license.customer_email {
                println!("  Account: {}", email);
            }
            if let Some(limit) = license.seat_limit {
                println!(
                    "  Seats:   {}/{} registered",
                    license.seats_registered, limit
                );
            } else {
                println!("  Seats:   unlimited");
            }
            if !license.this_machine_registered {
                println!();
                println!("  ℹ️  This machine has been registered for this license.");
            }
            println!();
            println!("  Features:");
            println!("{}", format_feature_summary(&license.gates, &license.tier));
        }
        Ok(status) => {
            // Unexpected non-Valid status after activate (shouldn't happen)
            eprintln!(
                "⚠️  Unexpected status after activation: {:?}",
                status_name(&status)
            );
            std::process::exit(1);
        }
        Err(crate::licensing::LicenseError::InvalidKeyFormat) => {
            eprintln!("❌ Invalid key format. Expected: GYRE-XXXX-XXXX-XXXX-XXXX-XXXX");
            std::process::exit(1);
        }
        Err(crate::licensing::LicenseError::ValidationFailed(reason)) => {
            eprintln!("❌ License validation failed: {}", reason);
            eprintln!("   Purchase or renew at https://gyre.ai/pricing");
            std::process::exit(1);
        }
        Err(crate::licensing::LicenseError::NetworkError(e)) => {
            eprintln!("⚠️  Could not reach license server: {}", e);
            eprintln!("   Check your internet connection and try again.");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("❌ Activation error: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}

// ─── Status ───────────────────────────────────────────────────────────────────

async fn run_status() -> anyhow::Result<()> {
    let machine_id = get_machine_id();
    println!("🔍 Checking license status...");
    println!("  Machine ID: {}", machine_id);
    println!();

    let status = load_and_validate().await;

    match &status {
        LicenseStatus::Valid(license) => {
            println!("✅ License: VALID");
            println!("   Key:     {}", license.display_key());
            if let Some(email) = &license.customer_email {
                println!("   Account: {}", email);
            }
            if let Some(limit) = license.seat_limit {
                println!(
                    "   Seats:   {}/{} registered",
                    license.seats_registered, limit
                );
            } else {
                println!("   Seats:   unlimited");
            }
            println!();
            println!("  Features:");
            println!("{}", format_feature_summary(&license.gates, &license.tier));
        }

        LicenseStatus::GracePeriod(license) => {
            let hours = grace_hours_remaining(license.validated_at);
            println!("⚠️  License: OFFLINE (grace period)");
            println!("   Key:     {}", license.display_key());
            println!("   Tier:    {}", license.tier.display_name());
            println!();
            if hours >= 24 {
                println!("   ⚠️  {}h remaining before downgrade to Free tier.", hours);
            } else {
                println!("   🚨 Only {}h remaining! Connect to renew license.", hours);
            }
            println!("   Features are active at your licensed tier during grace period.");
            println!();
            println!("  Features:");
            println!("{}", format_feature_summary(&license.gates, &license.tier));
        }

        LicenseStatus::GraceExpired => {
            println!("🔴 License: GRACE PERIOD EXPIRED");
            println!();
            println!(
                "   Could not reach license server and offline grace period (72h) has expired."
            );
            println!("   Running on Free tier until license is renewed.");
            println!();
            let free_gates = crate::licensing::FeatureGates::free();
            println!("  Current features (Free tier):");
            println!("{}", format_feature_summary(&free_gates, &Tier::Free));
        }

        LicenseStatus::Invalid(reason) => {
            println!("❌ License: INVALID");
            println!("   Reason:  {}", reason);
            println!();
            println!("   Running on Free tier.");
            println!("   Purchase at https://gyre.ai/pricing");
            println!();
            let free_gates = crate::licensing::FeatureGates::free();
            println!("  Current features (Free tier):");
            println!("{}", format_feature_summary(&free_gates, &Tier::Free));
        }

        LicenseStatus::NoLicense => {
            println!("ℹ️  No license activated.");
            println!();
            println!("   Running on Free tier.");
            println!("   Activate a license key with: gyre license activate <KEY>");
            println!("   Purchase at https://gyre.ai/pricing");
            println!();
            let free_gates = crate::licensing::FeatureGates::free();
            println!("  Free tier features:");
            println!("{}", format_feature_summary(&free_gates, &Tier::Free));
        }
    }

    Ok(())
}

// ─── Deactivate ───────────────────────────────────────────────────────────────

fn run_deactivate() -> anyhow::Result<()> {
    match deactivate_license() {
        Ok(()) => {
            println!("✅ License deactivated. Key and cache removed.");
            println!("   Gyre is now running on the Free tier.");
            println!("   Reactivate at any time with: gyre license activate <KEY>");
        }
        Err(e) => {
            eprintln!("❌ Failed to deactivate license: {}", e);
            std::process::exit(1);
        }
    }
    Ok(())
}

// ─── Startup integration ──────────────────────────────────────────────────────

/// Print license status on startup (called by `gyre serve` and `gyre run`).
///
/// - Free tier: show upgrade notice
/// - GracePeriod: warn if > 24h offline
/// - GraceExpired: warn that features degraded
/// - Valid: brief tier info (quiet on Standard+)
pub async fn print_startup_license_info(status: &LicenseStatus) {
    match status {
        LicenseStatus::Valid(license) => {
            if license.tier == Tier::Free {
                eprintln!(
                    "[license] Free tier active. Upgrade at https://gyre.ai/pricing for more agents, tribe, and curiosity engine."
                );
            } else {
                eprintln!("[license] {} license active.", license.tier.display_name());
            }
        }

        LicenseStatus::GracePeriod(license) => {
            let hours = grace_hours_remaining(license.validated_at);
            if hours < 24 {
                eprintln!(
                    "[license] ⚠️  OFFLINE GRACE PERIOD: {}h remaining. Connect to renew.",
                    hours
                );
            } else if hours < 48 {
                eprintln!(
                    "[license] Offline grace period active: {}h remaining.",
                    hours
                );
            }
            // < 24h: silent (avoid noise on short disconnects)
        }

        LicenseStatus::GraceExpired => {
            eprintln!(
                "[license] ⚠️  Offline grace period expired. Degraded to Free tier. \
                 Connect to restore {} features.",
                "paid"
            );
        }

        LicenseStatus::Invalid(reason) => {
            eprintln!(
                "[license] License invalid ({}). Running on Free tier.",
                reason
            );
        }

        LicenseStatus::NoLicense => {
            eprintln!(
                "[license] No license. Free tier active. \
                 Activate with: gyre license activate <KEY>"
            );
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn redact_key(key: &str) -> String {
    let parts: Vec<&str> = key.splitn(3, '-').collect();
    if parts.len() >= 2 {
        format!("{}-{}-****-****-****-****", parts[0], parts[1])
    } else {
        "GYRE-****".to_string()
    }
}

fn status_name(status: &LicenseStatus) -> &'static str {
    match status {
        LicenseStatus::Valid(_) => "Valid",
        LicenseStatus::GracePeriod(_) => "GracePeriod",
        LicenseStatus::GraceExpired => "GraceExpired",
        LicenseStatus::Invalid(_) => "Invalid",
        LicenseStatus::NoLicense => "NoLicense",
    }
}
