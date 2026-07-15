//! Claude Code (Claude.ai subscription) OAuth credential handling.
//!
//! When a user picks their Claude.ai subscription at setup, Gyre borrows the
//! OAuth token that the Claude Code CLI stores. That token is short-lived
//! (~8-12h) but ships alongside a refresh token and an expiry timestamp.
//! Historically Gyre read only the access token and ignored the rest, so it
//! would use a dead token and surface a raw 401 with no way to recover.
//!
//! This module reads the *full* credential set, knows when a token is
//! expired, and can refresh it in place using the refresh token. Every
//! refresh path is best-effort: on any failure it reports `Expired` so the
//! caller can show a clear, actionable message instead of a cryptic 401.
//!
//! Credential store:
//! - macOS: Keychain generic password `Claude Code-credentials`
//! - Linux/other: `~/.claude/.credentials.json`
//!
//! Both hold `{"claudeAiOauth": {"accessToken", "refreshToken",
//! "expiresAt" (unix ms), ...}}`.

use std::time::Duration;

use serde_json::Value;

/// Claude Code's public OAuth client id. Overridable via
/// `CLAUDE_OAUTH_CLIENT_ID` in case the published value ever changes — a
/// wrong value only means refresh fails and we fall back to guided reauth.
const DEFAULT_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

/// Claude Code's OAuth token endpoint. Overridable via
/// `CLAUDE_OAUTH_TOKEN_URL`.
const DEFAULT_TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";

/// Refresh a token this many seconds before it actually expires, so a
/// long-running turn doesn't cross the boundary mid-request.
const EXPIRY_MARGIN_SECS: i64 = 300;

/// The Claude Code OAuth credential set.
#[derive(Debug, Clone)]
pub struct ClaudeCredentials {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Absolute expiry, if the store recorded one.
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl ClaudeCredentials {
    /// True when the access token is expired or within the refresh margin.
    /// A credential with no recorded expiry is treated as usable (the
    /// server remains the final authority).
    pub fn is_expired(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        match self.expires_at {
            Some(exp) => now + chrono::Duration::seconds(EXPIRY_MARGIN_SECS) >= exp,
            None => false,
        }
    }
}

/// Where the caller stands with respect to Claude.ai subscription auth.
#[derive(Debug, Clone)]
pub enum CredentialStatus {
    /// A usable (unexpired) access token.
    Valid {
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    },
    /// Credentials exist but the access token is expired (and we couldn't or
    /// didn't refresh). The user must re-authenticate Claude Code.
    Expired,
    /// No Claude Code credentials were found at all.
    Missing,
}

/// Parse the credential blob (keychain payload or file contents).
pub fn parse_credentials(json: &str) -> Option<ClaudeCredentials> {
    let root: Value = serde_json::from_str(json).ok()?;
    let oauth = root.get("claudeAiOauth")?;

    let access_token = oauth.get("accessToken")?.as_str()?.to_string();
    let refresh_token = oauth
        .get("refreshToken")
        .and_then(|v| v.as_str())
        .map(String::from);
    // Claude Code records expiresAt as unix milliseconds.
    let expires_at = oauth
        .get("expiresAt")
        .and_then(|v| v.as_i64())
        .and_then(chrono::DateTime::from_timestamp_millis);

    Some(ClaudeCredentials {
        access_token,
        refresh_token,
        expires_at,
    })
}

/// Read the current Claude Code credentials from the platform store.
/// Returns `None` when Claude Code is not installed / not signed in.
pub fn load_credentials() -> Option<ClaudeCredentials> {
    // macOS: Keychain generic password.
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                "Claude Code-credentials",
                "-w",
            ])
            .output()
            && output.status.success()
            && let Ok(json) = String::from_utf8(output.stdout)
        {
            return parse_credentials(json.trim());
        }
    }

    // Linux/other: ~/.claude/.credentials.json
    let creds_path = credentials_file_path()?;
    let json = std::fs::read_to_string(&creds_path).ok()?;
    parse_credentials(&json)
}

/// Path to the file-backed credential store (all platforms have it as a
/// fallback; it is the primary store on Linux).
fn credentials_file_path() -> Option<std::path::PathBuf> {
    Some(dirs::home_dir()?.join(".claude").join(".credentials.json"))
}

fn client_id() -> String {
    std::env::var("CLAUDE_OAUTH_CLIENT_ID").unwrap_or_else(|_| DEFAULT_CLIENT_ID.to_string())
}

fn token_url() -> String {
    std::env::var("CLAUDE_OAUTH_TOKEN_URL").unwrap_or_else(|_| DEFAULT_TOKEN_URL.to_string())
}

/// The JSON body for a refresh request. Extracted for testing.
fn refresh_request_body(refresh_token: &str) -> Value {
    serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": client_id(),
    })
}

/// Return a currently-valid access token, refreshing in place if the stored
/// one is expired. This is the primary entry point for callers that just
/// want "a token that works right now."
///
/// - `Valid` with a token when we have (or successfully refreshed to) a live token.
/// - `Expired` when credentials exist but are expired and refresh failed/unavailable.
/// - `Missing` when Claude Code isn't signed in.
///
/// The returned token (on `Valid`) is provided separately via `out_token`
/// to keep it out of the `Debug`-logged status enum.
pub async fn ensure_fresh_token() -> (CredentialStatus, Option<String>) {
    let Some(creds) = load_credentials() else {
        return (CredentialStatus::Missing, None);
    };

    if !creds.is_expired(chrono::Utc::now()) {
        return (
            CredentialStatus::Valid {
                expires_at: creds.expires_at,
            },
            Some(creds.access_token),
        );
    }

    // Expired: attempt a best-effort refresh.
    let Some(refresh_token) = creds.refresh_token.clone() else {
        tracing::debug!("Claude token expired and no refresh token present");
        return (CredentialStatus::Expired, None);
    };

    match refresh(&refresh_token).await {
        Some(refreshed) => {
            // Persist so subsequent runs (and Claude Code itself) see the new
            // token. Failure to write back is non-fatal — this run still works.
            if let Err(e) = write_back(&refreshed) {
                tracing::warn!(error = %e, "Refreshed Claude token but could not write it back");
            }
            (
                CredentialStatus::Valid {
                    expires_at: refreshed.expires_at,
                },
                Some(refreshed.access_token),
            )
        }
        None => (CredentialStatus::Expired, None),
    }
}

/// Non-refreshing status probe, for `gyre auth status`.
pub fn current_status() -> CredentialStatus {
    match load_credentials() {
        None => CredentialStatus::Missing,
        Some(creds) if creds.is_expired(chrono::Utc::now()) => CredentialStatus::Expired,
        Some(creds) => CredentialStatus::Valid {
            expires_at: creds.expires_at,
        },
    }
}

/// Perform the OAuth refresh. Returns the new credentials on success, `None`
/// on any failure (network, non-2xx, malformed body, missing fields).
async fn refresh(refresh_token: &str) -> Option<ClaudeCredentials> {
    let url = token_url();
    // Defense: only ever talk HTTPS to the token endpoint.
    if !url.starts_with("https://") {
        tracing::warn!(url = %url, "Claude OAuth token_url is not HTTPS, refusing refresh");
        return None;
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;

    let resp = match client
        .post(&url)
        .json(&refresh_request_body(refresh_token))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "Claude OAuth refresh request failed");
            return None;
        }
    };

    if !resp.status().is_success() {
        tracing::warn!(status = %resp.status(), "Claude OAuth refresh returned non-success");
        return None;
    }

    let body: Value = resp.json().await.ok()?;
    let access_token = body.get("access_token")?.as_str()?.to_string();
    // Rotated refresh token if present, else keep the current one.
    let new_refresh = body
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| Some(refresh_token.to_string()));
    let expires_at = body
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .map(|secs| chrono::Utc::now() + chrono::Duration::seconds(secs));

    tracing::info!("Refreshed Claude.ai subscription token");
    Some(ClaudeCredentials {
        access_token,
        refresh_token: new_refresh,
        expires_at,
    })
}

/// Write refreshed credentials back to the file store so the next run — and
/// Claude Code — see them. Preserves any other fields already in the blob.
/// Only writes the file store (avoids fragile Keychain mutation on macOS;
/// the file is read as a fallback there too).
fn write_back(creds: &ClaudeCredentials) -> std::io::Result<()> {
    let Some(path) = credentials_file_path() else {
        return Ok(());
    };
    // Merge into the existing blob to avoid dropping fields we don't model.
    let mut root: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({ "claudeAiOauth": {} }));

    let oauth = root
        .get_mut("claudeAiOauth")
        .and_then(|v| v.as_object_mut());
    let Some(oauth) = oauth else {
        return Ok(()); // unexpected shape; leave the store untouched
    };

    oauth.insert(
        "accessToken".into(),
        Value::from(creds.access_token.clone()),
    );
    if let Some(ref rt) = creds.refresh_token {
        oauth.insert("refreshToken".into(), Value::from(rt.clone()));
    }
    if let Some(exp) = creds.expires_at {
        oauth.insert("expiresAt".into(), Value::from(exp.timestamp_millis()));
    }

    // Atomic-ish: write a temp file then rename over the original.
    let tmp = path.with_extension("json.gyre-tmp");
    std::fs::write(&tmp, serde_json::to_string(&root)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts_ms(dt: chrono::DateTime<chrono::Utc>) -> i64 {
        dt.timestamp_millis()
    }

    #[test]
    fn parse_full_credentials() {
        let future = chrono::Utc::now() + chrono::Duration::hours(8);
        let json = format!(
            r#"{{"claudeAiOauth":{{"accessToken":"sk-ant-oat01-abc","refreshToken":"sk-ant-ort01-xyz","expiresAt":{},"scopes":["user:inference"]}}}}"#,
            ts_ms(future)
        );
        let c = parse_credentials(&json).expect("should parse");
        assert_eq!(c.access_token, "sk-ant-oat01-abc");
        assert_eq!(c.refresh_token.as_deref(), Some("sk-ant-ort01-xyz"));
        assert!(c.expires_at.is_some());
        assert!(!c.is_expired(chrono::Utc::now()));
    }

    #[test]
    fn parse_missing_oauth_object_is_none() {
        assert!(parse_credentials(r#"{"something":"else"}"#).is_none());
        assert!(parse_credentials("not json").is_none());
        // Present object but no accessToken → None (unusable).
        assert!(parse_credentials(r#"{"claudeAiOauth":{"refreshToken":"x"}}"#).is_none());
    }

    #[test]
    fn expired_when_past_expiry() {
        let past = chrono::Utc::now() - chrono::Duration::hours(1);
        let json = format!(
            r#"{{"claudeAiOauth":{{"accessToken":"sk-ant-oat01-old","refreshToken":"r","expiresAt":{}}}}}"#,
            ts_ms(past)
        );
        let c = parse_credentials(&json).unwrap();
        assert!(c.is_expired(chrono::Utc::now()));
    }

    #[test]
    fn expiry_margin_triggers_early_refresh() {
        // Expires in 2 minutes — inside the 5-minute margin → treated expired.
        let soon = chrono::Utc::now() + chrono::Duration::seconds(120);
        let c = ClaudeCredentials {
            access_token: "t".into(),
            refresh_token: Some("r".into()),
            expires_at: Some(soon),
        };
        assert!(c.is_expired(chrono::Utc::now()));
    }

    #[test]
    fn no_expiry_recorded_is_treated_valid() {
        let c = ClaudeCredentials {
            access_token: "t".into(),
            refresh_token: None,
            expires_at: None,
        };
        assert!(!c.is_expired(chrono::Utc::now()));
    }

    #[test]
    fn refresh_body_has_required_fields() {
        let body = refresh_request_body("sk-ant-ort01-xyz");
        assert_eq!(body["grant_type"], "refresh_token");
        assert_eq!(body["refresh_token"], "sk-ant-ort01-xyz");
        assert!(body["client_id"].as_str().is_some_and(|s| !s.is_empty()));
    }
}
