//! `gyre auth` — inspect and refresh LLM authentication.
//!
//! Gives Claude.ai subscription users a place to re-authenticate. The
//! subscription token (borrowed from Claude Code) is short-lived; this
//! command reports its state and refreshes it in place using the refresh
//! token, so an expired session no longer means a cryptic 401 with nowhere
//! to turn.

use clap::Subcommand;

use crate::llm::claude_oauth::{self, CredentialStatus};

#[derive(Subcommand, Debug, Clone)]
pub enum AuthCommand {
    /// Show the current LLM auth state (provider + token validity).
    Status,
    /// Refresh the Claude.ai subscription token now (same as `login`).
    Refresh,
    /// Re-authenticate: refresh the subscription token, or print how to sign in.
    Login,
}

pub async fn run_auth_command(cmd: AuthCommand) -> anyhow::Result<()> {
    match cmd {
        AuthCommand::Status => status(),
        AuthCommand::Refresh | AuthCommand::Login => login().await,
    }
    Ok(())
}

fn fmt_expiry(expires_at: Option<chrono::DateTime<chrono::Utc>>) -> String {
    match expires_at {
        Some(exp) => {
            let rel = exp.signed_duration_since(chrono::Utc::now());
            if rel.num_seconds() <= 0 {
                format!("expired ({})", exp.format("%Y-%m-%d %H:%M UTC"))
            } else if rel.num_hours() >= 1 {
                format!(
                    "valid for ~{}h ({})",
                    rel.num_hours(),
                    exp.format("%Y-%m-%d %H:%M UTC")
                )
            } else {
                format!(
                    "valid for ~{}m ({})",
                    rel.num_minutes().max(0),
                    exp.format("%Y-%m-%d %H:%M UTC")
                )
            }
        }
        None => "no recorded expiry".to_string(),
    }
}

fn status() {
    // An explicit API key always wins at runtime.
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        println!("Anthropic: using ANTHROPIC_API_KEY from the environment (no expiry).");
        return;
    }

    match claude_oauth::current_status() {
        CredentialStatus::Valid { expires_at } => {
            println!(
                "Claude.ai subscription (via Claude Code): {}",
                fmt_expiry(expires_at)
            );
        }
        CredentialStatus::Expired => {
            println!("Claude.ai subscription (via Claude Code): EXPIRED.");
            println!("Run `gyre auth login` to refresh it.");
        }
        CredentialStatus::Missing => {
            println!("No LLM credentials found.");
            println!(
                "Sign in to Claude Code (`claude` CLI) for your Claude.ai subscription, \
                 or set ANTHROPIC_API_KEY to a console.anthropic.com API key, then re-run setup."
            );
        }
    }
}

async fn login() {
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        println!(
            "ANTHROPIC_API_KEY is set in your environment — Gyre uses that directly, \
             nothing to refresh."
        );
        return;
    }

    match claude_oauth::current_status() {
        CredentialStatus::Missing => {
            println!("No Claude.ai subscription credentials found to refresh.");
            println!(
                "Sign in with the Claude Code CLI first:\n  claude        # then use /login\n\
                 …or set ANTHROPIC_API_KEY to a console.anthropic.com API key."
            );
        }
        CredentialStatus::Valid { expires_at } => {
            // ensure_fresh_token wouldn't refresh a still-valid token, so
            // don't claim we did — just report it.
            println!("Claude.ai subscription session is still valid — {}", fmt_expiry(expires_at));
        }
        CredentialStatus::Expired => {
            print!("Refreshing expired Claude.ai subscription token… ");
            let (status, _token) = claude_oauth::ensure_fresh_token().await;
            match status {
                CredentialStatus::Valid { expires_at } => {
                    println!("done. {}", fmt_expiry(expires_at));
                }
                _ => {
                    println!("could not refresh automatically.");
                    println!(
                        "Re-sign-in with the Claude Code CLI, then try again:\n\
                         \x20 claude        # then use /login\n\
                         Or set ANTHROPIC_API_KEY to a console.anthropic.com API key."
                    );
                }
            }
        }
    }
}
