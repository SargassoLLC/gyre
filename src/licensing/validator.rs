//! License validation logic.
//!
//! Validation flow:
//! 1. Attempt remote validation against `api.gyre.ai/v1/license/validate` (5s timeout)
//! 2. On network error, check HMAC-signed local cache (`~/.gyre/license.cache`)
//! 3. If offline but within 72h grace → GracePeriod
//! 4. If offline > 72h → GraceExpired (degrade to Free, not hard lockout)
//!
//! Privacy: machine_id is a one-way hash of hardware UUID; the raw UUID is never sent.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{FeatureGates, License, LicenseError, LicenseStatus, Tier, normalize_key};

/// Compile-time secret mixed into HMAC to bind the cache file to this binary.
/// (Not security-critical alone — combined with machine_id for cache binding.)
const HMAC_DOMAIN_SALT: &[u8] = b"gyre-license-cache-v1";

/// Remote validation endpoint.
const VALIDATE_URL: &str = "https://api.gyre.ai/v1/license/validate";

/// HTTP timeout for remote validation (don't block startup).
const VALIDATE_TIMEOUT_SECS: u64 = 5;

/// Offline grace period: 72 hours in seconds.
const GRACE_PERIOD_SECS: u64 = 72 * 3600;

/// Warn after 24h offline.
const WARN_AFTER_SECS: u64 = 24 * 3600;

/// Gyre version string (embedded at compile time).
const GYRE_VERSION: &str = env!("CARGO_PKG_VERSION");

// ─── Remote API types ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ValidateRequest {
    key: String,
    machine_id: String,
    gyre_version: String,
    platform: String,
}

#[derive(Debug, Deserialize)]
struct ValidateResponse {
    valid: bool,
    #[serde(default)]
    tier: Option<String>,
    #[serde(default)]
    features: Option<ServerFeatures>,
    #[serde(default)]
    seats: Option<ServerSeats>,
    #[serde(default)]
    license: Option<ServerLicenseInfo>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    cache_ttl_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ServerFeatures {
    #[serde(default)]
    max_agents: Option<u32>,
    #[serde(default)]
    tribe_enabled: bool,
    #[serde(default)]
    curiosity_engine: bool,
    #[serde(default)]
    memory_days: Option<u32>,
    #[serde(default)]
    multi_tenant: bool,
    #[serde(default)]
    priority_support: bool,
    #[serde(default)]
    white_label: bool,
}

#[derive(Debug, Deserialize)]
struct ServerSeats {
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    registered: Vec<String>,
    #[serde(default)]
    this_machine_registered: bool,
}

#[derive(Debug, Deserialize)]
struct ServerLicenseInfo {
    #[serde(default)]
    customer_email: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
}

// ─── Cache types ──────────────────────────────────────────────────────────────

/// HMAC-signed local cache.  Prevents the cache file from being trivially
/// tampered with or copied to another machine (machine_id is part of HMAC key).
#[derive(Debug, Serialize, Deserialize)]
struct LicenseCache {
    /// SHA-256 of the normalized key (not the raw key).
    key_hash: String,
    /// Unix timestamp of last successful server validation.
    validated_at: u64,
    /// Unix timestamp after which grace period expires.
    expires_grace_at: u64,
    /// Full server response (preserved as-is).
    response: serde_json::Value,
    /// HMAC-SHA256 over `key_hash || validated_at || response_json`.
    hmac: String,
}

impl LicenseCache {
    fn compute_hmac(
        machine_id: &str,
        key_hash: &str,
        validated_at: u64,
        response_json: &str,
    ) -> String {
        let hmac_key = derive_hmac_key(machine_id);
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&hmac_key).expect("HMAC accepts any key length");
        mac.update(HMAC_DOMAIN_SALT);
        mac.update(key_hash.as_bytes());
        mac.update(&validated_at.to_le_bytes());
        mac.update(response_json.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    fn is_hmac_valid(&self, machine_id: &str) -> bool {
        let response_json = self.response.to_string();
        let expected = Self::compute_hmac(
            machine_id,
            &self.key_hash,
            self.validated_at,
            &response_json,
        );
        // Constant-time comparison via subtle would be ideal; string comparison is
        // acceptable here since the attacker cannot predict the HMAC key.
        expected == self.hmac
    }

    fn new(key: &str, machine_id: &str, response: serde_json::Value) -> Self {
        let now = unix_now();
        let key_hash = sha256_hex(key.as_bytes());
        let response_json = response.to_string();
        let hmac = Self::compute_hmac(machine_id, &key_hash, now, &response_json);
        Self {
            key_hash,
            validated_at: now,
            expires_grace_at: now + GRACE_PERIOD_SECS,
            response,
            hmac,
        }
    }
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Validate a license key.
///
/// 1. Tries the remote server (5s timeout).
/// 2. Falls back to HMAC-signed local cache.
/// 3. Applies 72h grace period logic.
pub async fn validate_license(key: &str) -> LicenseStatus {
    let key = normalize_key(key);
    let machine_id = get_machine_id();

    // 1. Attempt remote validation
    match validate_remote(&key, &machine_id).await {
        Ok(status) => status,
        Err(LicenseError::NetworkError(_)) => {
            // Offline — check local cache
            check_cache(&key, &machine_id)
        }
        Err(LicenseError::ValidationFailed(reason)) => {
            // Server explicitly rejected — clear cache
            let _ = clear_cache();
            LicenseStatus::Invalid(reason)
        }
        Err(e) => {
            tracing::warn!("[license] Unexpected validation error: {}", e);
            check_cache(&key, &machine_id)
        }
    }
}

/// Activate a license key: validate remotely and save to disk.
pub async fn activate_license(key: &str) -> Result<LicenseStatus, LicenseError> {
    let key = normalize_key(key);
    if !super::is_valid_key_format(&key) {
        return Err(LicenseError::InvalidKeyFormat);
    }

    let machine_id = get_machine_id();
    let status = validate_remote(&key, &machine_id).await?;

    // Save key to disk
    save_key_file(&key)?;

    Ok(status)
}

/// Load the saved license key (if any) and validate it.
pub async fn load_and_validate() -> LicenseStatus {
    match load_key_file() {
        Some(key) => validate_license(&key).await,
        None => LicenseStatus::NoLicense,
    }
}

/// Remove the stored key and cache.
pub fn deactivate_license() -> Result<(), LicenseError> {
    let _ = clear_cache();
    let key_path = gyre_dir().join("license.key");
    if key_path.exists() {
        fs::remove_file(&key_path)?;
    }
    Ok(())
}

// ─── Remote validation ────────────────────────────────────────────────────────

async fn validate_remote(key: &str, machine_id: &str) -> Result<LicenseStatus, LicenseError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(VALIDATE_TIMEOUT_SECS))
        .build()
        .map_err(|e| LicenseError::NetworkError(e.to_string()))?;

    let req = ValidateRequest {
        key: key.to_string(),
        machine_id: machine_id.to_string(),
        gyre_version: GYRE_VERSION.to_string(),
        platform: get_platform(),
    };

    let resp = client
        .post(VALIDATE_URL)
        .json(&req)
        .send()
        .await
        .map_err(|e| LicenseError::NetworkError(e.to_string()))?;

    let status_code = resp.status();
    let body: ValidateResponse = resp
        .json()
        .await
        .map_err(|e| LicenseError::ValidationFailed(format!("bad server response: {e}")))?;

    if !body.valid {
        let reason = body
            .message
            .or(body.reason)
            .unwrap_or_else(|| "License is not valid".to_string());
        return Err(LicenseError::ValidationFailed(reason));
    }

    if !status_code.is_success() {
        return Err(LicenseError::ValidationFailed(format!(
            "Server returned HTTP {}",
            status_code
        )));
    }

    // Build License from response
    let tier = parse_tier(body.tier.as_deref().unwrap_or("free"));
    let gates = body.features.as_ref().map_or_else(
        || FeatureGates::for_tier(&tier),
        |f| FeatureGates {
            max_agents: f.max_agents,
            tribe_enabled: f.tribe_enabled,
            curiosity_engine: f.curiosity_engine,
            memory_days: f.memory_days,
            multi_tenant: f.multi_tenant,
            priority_support: f.priority_support,
            white_label: f.white_label,
        },
    );

    let seat_limit = body.seats.as_ref().and_then(|s| s.limit);
    let seats_registered = body.seats.as_ref().map_or(0, |s| s.registered.len() as u32);
    let this_machine_registered = body
        .seats
        .as_ref()
        .map_or(false, |s| s.this_machine_registered);
    let customer_email = body.license.as_ref().and_then(|l| l.customer_email.clone());
    let created_at = body.license.as_ref().and_then(|l| l.created_at.clone());
    let expires_at = body.license.as_ref().and_then(|l| l.expires_at.clone());

    let license = License {
        key: key.to_string(),
        tier,
        gates,
        customer_email,
        created_at,
        expires_at,
        seat_limit,
        seats_registered,
        this_machine_registered,
        validated_at: unix_now(),
    };

    // Persist cache — serialize the resolved License struct (body is already consumed)
    let license_value = serde_json::to_value(&license).unwrap_or(serde_json::Value::Null);
    if let Err(e) = save_cache(key, machine_id, license_value) {
        tracing::warn!("[license] Failed to save cache: {}", e);
    }

    Ok(LicenseStatus::Valid(license))
}

// ─── Cache operations ─────────────────────────────────────────────────────────

fn check_cache(key: &str, machine_id: &str) -> LicenseStatus {
    match load_cache(machine_id) {
        Some(cache) => {
            // Verify the cache was for the same key
            let expected_key_hash = sha256_hex(key.as_bytes());
            if cache.key_hash != expected_key_hash {
                return LicenseStatus::NoLicense;
            }

            let now = unix_now();
            let age_secs = now.saturating_sub(cache.validated_at);

            if age_secs > GRACE_PERIOD_SECS {
                tracing::warn!(
                    "[license] Offline grace period expired ({}h > 72h). Degrading to Free tier.",
                    age_secs / 3600
                );
                LicenseStatus::GraceExpired
            } else {
                // Reconstruct license from cached data
                match serde_json::from_value::<License>(cache.response.clone()) {
                    Ok(mut license) => {
                        // Update validated_at to reflect cache load time
                        license.validated_at = cache.validated_at;

                        let remaining_secs = GRACE_PERIOD_SECS.saturating_sub(age_secs);
                        let remaining_hours = remaining_secs / 3600;

                        if age_secs >= WARN_AFTER_SECS {
                            tracing::warn!(
                                "[license] Offline grace period active: {}h remaining. \
                                 Connect to renew license.",
                                remaining_hours
                            );
                        }

                        LicenseStatus::GracePeriod(license)
                    }
                    Err(e) => {
                        tracing::warn!("[license] Cache parse failed: {}", e);
                        LicenseStatus::NoLicense
                    }
                }
            }
        }
        None => LicenseStatus::NoLicense,
    }
}

fn load_cache(machine_id: &str) -> Option<LicenseCache> {
    let path = cache_path();
    if !path.exists() {
        return None;
    }

    let data = fs::read_to_string(&path).ok()?;
    let cache: LicenseCache = serde_json::from_str(&data).ok()?;

    if !cache.is_hmac_valid(machine_id) {
        tracing::warn!("[license] Cache HMAC validation failed — cache may be tampered with.");
        return None;
    }

    Some(cache)
}

fn save_cache(
    key: &str,
    machine_id: &str,
    response: serde_json::Value,
) -> Result<(), LicenseError> {
    let cache = LicenseCache::new(key, machine_id, response);
    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| LicenseError::CacheError(e.to_string()))?;

    let path = cache_path();
    ensure_gyre_dir()?;
    write_mode_600(&path, json.as_bytes())?;
    Ok(())
}

fn clear_cache() -> Result<(), LicenseError> {
    let path = cache_path();
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

// ─── Key file operations ──────────────────────────────────────────────────────

fn save_key_file(key: &str) -> Result<(), LicenseError> {
    ensure_gyre_dir()?;
    let path = gyre_dir().join("license.key");
    write_mode_600(&path, key.as_bytes())?;
    Ok(())
}

fn load_key_file() -> Option<String> {
    let path = gyre_dir().join("license.key");
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

// ─── Machine ID (privacy-preserving) ─────────────────────────────────────────

/// Returns a privacy-preserving machine ID: SHA-256 of hardware UUID + domain salt.
///
/// The raw hardware UUID is never transmitted; only its hash is used.
/// Domain-separated with "gyre-machine-id-v1" to prevent cross-protocol reuse.
pub fn get_machine_id() -> String {
    let raw = get_hardware_uuid();
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    hasher.update(b"gyre-machine-id-v1");
    let hash = hasher.finalize();
    hex::encode(&hash[..8]) // 16 hex chars, sufficient for seat tracking
}

/// Get the raw hardware UUID from the OS (best-effort).
fn get_hardware_uuid() -> String {
    #[cfg(target_os = "macos")]
    {
        // macOS: ioreg or system_profiler
        if let Ok(out) = std::process::Command::new("ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout);
            for line in s.lines() {
                if line.contains("IOPlatformUUID") {
                    if let Some(uuid) = extract_quoted(line) {
                        return uuid;
                    }
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        // Linux: /etc/machine-id
        if let Ok(id) = fs::read_to_string("/etc/machine-id") {
            let trimmed = id.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
        if let Ok(id) = fs::read_to_string("/var/lib/dbus/machine-id") {
            let trimmed = id.trim().to_string();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
    }

    // Fallback: hash of hostname + username
    let hostname = hostname_fallback();
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string());
    format!("{}-{}", hostname, username)
}

fn hostname_fallback() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown-host".to_string())
}

#[cfg(target_os = "macos")]
fn extract_quoted(s: &str) -> Option<String> {
    let start = s.find('"')?;
    let rest = &s[start + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn derive_hmac_key(machine_id: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(machine_id.as_bytes());
    hasher.update(HMAC_DOMAIN_SALT);
    // Mix in compile-time salt (binary-bound)
    hasher.update(b"gyre-binary-v1");
    hasher.finalize().to_vec()
}

fn parse_tier(s: &str) -> Tier {
    match s.to_lowercase().as_str() {
        "standard" => Tier::Standard,
        "pro" => Tier::Pro,
        "enterprise" => Tier::Enterprise,
        _ => Tier::Free,
    }
}

fn get_platform() -> String {
    format!(
        "{}-{}-{}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        std::env::consts::FAMILY
    )
}

fn gyre_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".gyre")
}

fn cache_path() -> PathBuf {
    gyre_dir().join("license.cache")
}

fn ensure_gyre_dir() -> Result<(), LicenseError> {
    let dir = gyre_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
        set_dir_permissions_700(&dir)?;
    }
    Ok(())
}

/// Write file content with mode 600 (owner read/write only).
fn write_mode_600(path: &Path, data: &[u8]) -> Result<(), LicenseError> {
    // Write to temp file first, then rename (atomic)
    let tmp_path = path.with_extension("tmp");

    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)?;
        f.write_all(data)?;
        f.flush()?;
    }

    // Set permissions before renaming
    set_file_permissions_600(&tmp_path)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(unix)]
fn set_file_permissions_600(path: &Path) -> Result<(), LicenseError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_file_permissions_600(_path: &Path) -> Result<(), LicenseError> {
    // Windows: permissions handled by the OS (user-scoped paths)
    Ok(())
}

#[cfg(unix)]
fn set_dir_permissions_700(path: &Path) -> Result<(), LicenseError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_dir_permissions_700(_path: &Path) -> Result<(), LicenseError> {
    Ok(())
}

/// Hours remaining in grace period for display.
pub fn grace_hours_remaining(validated_at: u64) -> u64 {
    let now = unix_now();
    let age = now.saturating_sub(validated_at);
    GRACE_PERIOD_SECS.saturating_sub(age) / 3600
}

// Suppress unused import warning when not on macOS
#[allow(unused_imports)]
use serde_json;
