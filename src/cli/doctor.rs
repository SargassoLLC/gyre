//! `gyre doctor` — health diagnostics.
//!
//! Validates the Gyre configuration and probes external dependencies.
//! Each check reports pass / fail / skip with actionable guidance on failure.
//!
//! # Checks performed
//!
//! 1. Config file exists and has required keys
//! 2. Anthropic API key is present and valid (test ping)
//! 3. HermitBox structure is valid (if a box is configured)
//! 4. Telegram bot token works (if configured)
//! 5. Optional external binaries (docker, cloudflared, ngrok, tailscale)

use std::path::PathBuf;

/// Run all diagnostic checks and print results.
pub async fn run_doctor_command() -> anyhow::Result<()> {
    println!();
    println!("  \x1b[1m⟳ Gyre Doctor\x1b[0m");
    println!("  \x1b[2mChecking your Gyre installation…\x1b[0m");
    println!();

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;

    // ── Gyre config file ──────────────────────────────────────
    check(
        "Gyre config file",
        check_gyre_config(),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    // ── Anthropic API key ─────────────────────────────────────
    check(
        "Anthropic API key",
        check_anthropic_key(),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    // ── Anthropic LLM connection (live ping) ──────────────────
    check(
        "Anthropic LLM connection",
        check_anthropic_connection().await,
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    // ── HermitBox structure ───────────────────────────────────
    check(
        "HermitBox structure",
        check_hermit_box(),
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    // ── Telegram bot token ────────────────────────────────────
    check(
        "Telegram bot token",
        check_telegram_token().await,
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    // ── Database backend ──────────────────────────────────────
    check(
        "Database backend",
        check_database().await,
        &mut passed,
        &mut failed,
        &mut skipped,
    );

    // ── Optional external binaries ────────────────────────────
    println!();
    println!("  \x1b[2mOptional external tools:\x1b[0m");

    for (name, args) in &[
        ("docker", vec!["--version"]),
        ("cloudflared", vec!["--version"]),
        ("ngrok", vec!["version"]),
        ("tailscale", vec!["version"]),
    ] {
        check(
            name,
            check_binary(name, args),
            &mut passed,
            &mut failed,
            &mut skipped,
        );
    }

    // ── Summary ───────────────────────────────────────────────
    println!();
    let status = if failed == 0 {
        "\x1b[32m✅ All clear\x1b[0m"
    } else {
        "\x1b[33m⚠  Some checks failed\x1b[0m"
    };
    println!("  {status}  — {passed} passed, {failed} failed, {skipped} skipped");

    if failed > 0 {
        println!();
        println!("  \x1b[2mRun `gyre init` to fix configuration issues.\x1b[0m");
    }

    println!();
    Ok(())
}

// ── Check result type ─────────────────────────────────────────────────────────

enum CheckResult {
    Pass(String),
    Fail(String),
    Skip(String),
}

fn check(name: &str, result: CheckResult, passed: &mut u32, failed: &mut u32, skipped: &mut u32) {
    match result {
        CheckResult::Pass(detail) => {
            *passed += 1;
            println!("  \x1b[32m[pass]\x1b[0m {name}: \x1b[2m{detail}\x1b[0m");
        }
        CheckResult::Fail(detail) => {
            *failed += 1;
            println!("  \x1b[31m[FAIL]\x1b[0m {name}: {detail}");
        }
        CheckResult::Skip(reason) => {
            *skipped += 1;
            println!("  \x1b[2m[skip]\x1b[0m {name}: {reason}");
        }
    }
}

// ── Individual checks ─────────────────────────────────────────────────────────

/// Check that the Gyre config file exists and has required keys.
fn check_gyre_config() -> CheckResult {
    // Load env from gyre config path first
    let config_path = crate::settings::Settings::config_env_path();

    if config_path.exists() {
        // Try to load and validate required keys
        let _ = dotenvy::from_path(&config_path);

        let has_backend = std::env::var("LLM_BACKEND").is_ok();
        let has_key = std::env::var("ANTHROPIC_API_KEY").is_ok()
            || std::env::var("OPENAI_API_KEY").is_ok()
            || std::env::var("LLM_API_KEY").is_ok();

        if has_backend && has_key {
            CheckResult::Pass(format!("{}", config_path.display()))
        } else if !has_backend {
            CheckResult::Fail(format!(
                "config at {} is missing LLM_BACKEND. Run `gyre init`",
                config_path.display()
            ))
        } else {
            CheckResult::Fail(
                "config is missing an API key (ANTHROPIC_API_KEY / OPENAI_API_KEY). Run `gyre init`"
                    .to_string(),
            )
        }
    } else {
        // Check if configured via plain env vars (e.g., .env file in cwd)
        if std::env::var("LLM_BACKEND").is_ok() || std::env::var("ANTHROPIC_API_KEY").is_ok() {
            CheckResult::Pass("configured via environment variables".to_string())
        } else {
            CheckResult::Fail(format!(
                "config not found at {}. Run `gyre init` to create it.",
                config_path.display()
            ))
        }
    }
}

/// Check that an Anthropic API key is present.
fn check_anthropic_key() -> CheckResult {
    // Load gyre config so env vars are populated
    let config_path = crate::settings::Settings::config_env_path();
    if config_path.exists() {
        let _ = dotenvy::from_path(&config_path);
    }

    match std::env::var("ANTHROPIC_API_KEY") {
        Ok(key) if key.starts_with("sk-ant-") => {
            // Show a masked version
            let masked = format!("{}…{}", &key[..12], &key[key.len().saturating_sub(4)..]);
            CheckResult::Pass(masked)
        }
        Ok(key) if !key.is_empty() => CheckResult::Fail(format!(
            "ANTHROPIC_API_KEY looks malformed (expected sk-ant-… prefix, got: {}…)",
            &key[..key.len().min(10)]
        )),
        _ => {
            // Check if using a different LLM backend
            if let Ok(backend) = std::env::var("LLM_BACKEND") {
                if backend != "anthropic" {
                    return CheckResult::Skip(format!(
                        "using {backend} backend, ANTHROPIC_API_KEY not required"
                    ));
                }
            }
            CheckResult::Fail(
                "ANTHROPIC_API_KEY not set. Run `gyre init` or set the env var.".to_string(),
            )
        }
    }
}

/// Attempt a live ping to the Anthropic API.
async fn check_anthropic_connection() -> CheckResult {
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            return CheckResult::Skip("ANTHROPIC_API_KEY not set, skipping live ping".to_string());
        }
    };

    // Minimal Anthropic API ping: POST /v1/messages with max_tokens=1
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build();

    let client = match client {
        Ok(c) => c,
        Err(e) => return CheckResult::Fail(format!("failed to build HTTP client: {e}")),
    };

    let body = serde_json::json!({
        "model": "claude-haiku-4-20250514",
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "ping"}]
    });

    match client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                CheckResult::Pass("Anthropic API reachable ✓".to_string())
            } else if status.as_u16() == 401 {
                CheckResult::Fail(
                    "Anthropic API returned 401 Unauthorized — check your API key.".to_string(),
                )
            } else if status.as_u16() == 529 {
                CheckResult::Skip("Anthropic API overloaded (529), try again later.".to_string())
            } else {
                // 4xx with body often means valid key but model error — count as pass
                if status.as_u16() >= 400 && status.as_u16() < 500 {
                    CheckResult::Pass(format!("API reachable (HTTP {status})"))
                } else {
                    CheckResult::Fail(format!("unexpected HTTP {status} from Anthropic API"))
                }
            }
        }
        Err(e) if e.is_timeout() => CheckResult::Fail(
            "Anthropic API timed out (10s). Check network connectivity.".to_string(),
        ),
        Err(e) => CheckResult::Fail(format!("network error reaching Anthropic API: {e}")),
    }
}

/// Check that an agent's HermitBox structure exists and is valid.
fn check_hermit_box() -> CheckResult {
    // Load gyre config to get the configured agent + box dir
    let config_path = crate::settings::Settings::config_env_path();
    if config_path.exists() {
        let _ = dotenvy::from_path(&config_path);
    }

    let agent = match std::env::var("GYRE_AGENT") {
        Ok(a) if !a.is_empty() => a,
        _ => {
            return CheckResult::Skip(
                "GYRE_AGENT not set, skipping HermitBox check. Run `gyre init`.".to_string(),
            );
        }
    };

    let box_base = std::env::var("GYRE_BOX_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("agents")
        });

    let box_dir = box_base.join(format!("{agent}_box"));

    if !box_dir.exists() {
        return CheckResult::Fail(format!(
            "agent box not found at {}. Run `gyre init`.",
            box_dir.display()
        ));
    }

    // Validate expected structure
    let required = &[
        "memory/memories.db",
        "knowledge/kg.db",
        "axioms/axioms.db",
        "soul.md",
        "user.md",
        "memory.md",
        "TELOS/MISSION.md",
        "TELOS/GOALS.md",
        "TELOS/BELIEFS.md",
        "TELOS/BOUNDARIES.md",
        "TELOS/EXPERIENCES.md",
    ];

    let mut missing = Vec::new();
    for entry in required {
        if !box_dir.join(entry).exists() {
            missing.push(*entry);
        }
    }

    if missing.is_empty() {
        CheckResult::Pass(format!(
            "{agent}_box/ structure valid ({})",
            box_dir.display()
        ))
    } else {
        CheckResult::Fail(format!(
            "{agent}_box/ is incomplete. Missing: {}. Run `gyre init`.",
            missing.join(", ")
        ))
    }
}

/// Check if the Telegram bot token is valid (optional).
async fn check_telegram_token() -> CheckResult {
    let config_path = crate::settings::Settings::config_env_path();
    if config_path.exists() {
        let _ = dotenvy::from_path(&config_path);
    }

    let token = match std::env::var("TELEGRAM_BOT_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => {
            return CheckResult::Skip("TELEGRAM_BOT_TOKEN not configured (optional)".to_string());
        }
    };

    // Ping the Telegram getMe endpoint
    let url = format!("https://api.telegram.org/bot{}/getMe", token);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build();

    let client = match client {
        Ok(c) => c,
        Err(e) => return CheckResult::Fail(format!("failed to build HTTP client: {e}")),
    };

    match client.get(&url).send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                // Parse bot username from response
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    let username = json["result"]["username"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string();
                    CheckResult::Pass(format!("@{username} is online"))
                } else {
                    CheckResult::Pass("Telegram bot token valid".to_string())
                }
            } else if resp.status().as_u16() == 401 {
                CheckResult::Fail(
                    "Telegram bot token is invalid (401). Get a new one from @BotFather."
                        .to_string(),
                )
            } else {
                CheckResult::Fail(format!("Telegram API returned HTTP {}", resp.status()))
            }
        }
        Err(e) if e.is_timeout() => {
            CheckResult::Fail("Telegram API timed out (8s). Check network.".to_string())
        }
        Err(e) => CheckResult::Fail(format!("network error reaching Telegram: {e}")),
    }
}

/// Check database backend configuration.
async fn check_database() -> CheckResult {
    let config_path = crate::settings::Settings::config_env_path();
    if config_path.exists() {
        let _ = dotenvy::from_path(&config_path);
    }

    let backend = std::env::var("DATABASE_BACKEND")
        .ok()
        .unwrap_or_else(|| "libsql".into());

    match backend.as_str() {
        "libsql" | "turso" | "sqlite" => {
            let path = std::env::var("LIBSQL_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| crate::config::default_libsql_path());

            if path.exists() {
                CheckResult::Pass(format!("libSQL/SQLite at {}", path.display()))
            } else {
                CheckResult::Pass(format!(
                    "libSQL will be created at {} on first run",
                    path.display()
                ))
            }
        }
        "postgres" => {
            if std::env::var("DATABASE_URL").is_ok() {
                match try_pg_connect().await {
                    Ok(()) => CheckResult::Pass("PostgreSQL connected".to_string()),
                    Err(e) => CheckResult::Fail(format!("PostgreSQL connection failed: {e}")),
                }
            } else {
                CheckResult::Fail("DATABASE_URL not set for postgres backend".to_string())
            }
        }
        other => CheckResult::Skip(format!("unknown backend '{other}'")),
    }
}

#[cfg(feature = "postgres")]
async fn try_pg_connect() -> Result<(), String> {
    let url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL not set".to_string())?;

    let config = deadpool_postgres::Config {
        url: Some(url),
        ..Default::default()
    };
    let pool = config
        .create_pool(
            Some(deadpool_postgres::Runtime::Tokio1),
            tokio_postgres::NoTls,
        )
        .map_err(|e| format!("pool error: {e}"))?;

    let client = tokio::time::timeout(std::time::Duration::from_secs(5), pool.get())
        .await
        .map_err(|_| "connection timeout (5s)".to_string())?
        .map_err(|e| format!("{e}"))?;

    client
        .execute("SELECT 1", &[])
        .await
        .map_err(|e| format!("{e}"))?;

    Ok(())
}

#[cfg(not(feature = "postgres"))]
async fn try_pg_connect() -> Result<(), String> {
    Err("postgres feature not compiled in".into())
}

fn check_binary(name: &str, args: &[&str]) -> CheckResult {
    match std::process::Command::new(name)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout);
            let version = version.trim();
            let version = if version.is_empty() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                stderr.trim().lines().next().unwrap_or("").to_string()
            } else {
                version.lines().next().unwrap_or("").to_string()
            };

            if output.status.success() {
                CheckResult::Pass(version)
            } else {
                CheckResult::Fail(format!("exited with {}", output.status))
            }
        }
        Err(_) => CheckResult::Skip(format!("{name} not found in PATH")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_binary_finds_sh() {
        let _guard = crate::test_helpers::PROC_MUTEX.lock().unwrap();
        match check_binary("sh", &["-c", "echo ok"]) {
            CheckResult::Pass(_) => {}
            CheckResult::Fail(s) => panic!("expected Pass for sh, got Fail({s})"),
            CheckResult::Skip(s) => panic!("expected Pass for sh, got Skip({s})"),
        }
    }

    #[test]
    fn check_binary_skips_nonexistent() {
        match check_binary("__gyre_nonexistent_binary__", &["--version"]) {
            CheckResult::Skip(_) => {}
            CheckResult::Pass(s) => panic!("expected Skip, got Pass({s})"),
            CheckResult::Fail(s) => panic!("expected Skip, got Fail({s})"),
        }
    }

    #[test]
    fn check_gyre_config_does_not_panic() {
        // Just verify it doesn't panic; actual result depends on environment
        let _ = check_gyre_config();
    }

    #[test]
    fn check_anthropic_key_does_not_panic() {
        let _ = check_anthropic_key();
    }
}
