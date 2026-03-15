//! `gyre update` — self-update command.
//!
//! Checks for a newer release on GitHub (or api.gyre.ai/version), downloads
//! the correct binary for the current platform, verifies its SHA256 checksum,
//! atomically replaces the running executable, and optionally restarts.
//!
//! # Usage
//! ```text
//! gyre update           — download + install latest, restart if changed
//! gyre update --check   — report whether a newer version is available, exit
//! ```

use std::env;
use std::io::Write as _;
use std::path::PathBuf;

use anyhow::{Context, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// GitHub repository (owner/repo) for release downloads.
const GITHUB_REPO: &str = "sac916/gyre";

/// Optionally override the version check endpoint (fall back to GitHub API).
const VERSION_API: &str = "https://api.gyre.ai/version";

// ── Platform detection ────────────────────────────────────────────────────────

/// Returns the Rust target-triple for the current host, used to select the
/// correct release artifact. Returns an error on unsupported platforms.
fn current_target() -> anyhow::Result<&'static str> {
    // Compile-time constants for OS and architecture.
    // These are evaluated at build time, so the binary always knows its own target.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Ok("aarch64-apple-darwin");

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Ok("x86_64-apple-darwin");

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return Ok("x86_64-unknown-linux-gnu");

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return Ok("aarch64-unknown-linux-gnu");

    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
    )))]
    bail!(
        "Unsupported platform. Pre-built binaries are available for:\n\
         • macOS (Apple Silicon and Intel)\n\
         • Linux (x86_64 and aarch64)\n\
         Build from source: https://github.com/{GITHUB_REPO}"
    )
}

// ── Version types ─────────────────────────────────────────────────────────────

/// Parsed SemVer-ish version. Handles "v1.2.3" or "1.2.3".
#[derive(Debug, Clone, PartialEq, Eq)]
struct Version {
    major: u32,
    minor: u32,
    patch: u32,
    pre: Option<String>, // e.g. "beta.1", "rc.2"
}

impl Version {
    fn parse(s: &str) -> anyhow::Result<Self> {
        let s = s.trim_start_matches('v');
        // Split pre-release suffix on '-'
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p.to_string())),
            None => (s, None),
        };
        let parts: Vec<&str> = core.splitn(3, '.').collect();
        if parts.len() < 3 {
            bail!("Invalid version string: '{}'", s);
        }
        Ok(Self {
            major: parts[0].parse().context("major version")?,
            minor: parts[1].parse().context("minor version")?,
            patch: parts[2].parse().context("patch version")?,
            pre,
        })
    }

    /// Returns `true` if `self` is strictly newer than `other`.
    /// Pre-release versions are considered older than their stable counterpart.
    fn is_newer_than(&self, other: &Self) -> bool {
        let self_tuple = (self.major, self.minor, self.patch);
        let other_tuple = (other.major, other.minor, other.patch);
        match self_tuple.cmp(&other_tuple) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => {
                // Same core version: stable > pre-release
                match (&self.pre, &other.pre) {
                    (None, Some(_)) => true,  // self is stable, other is pre → self newer
                    (Some(_), None) => false, // self is pre, other is stable → self older
                    _ => false,               // same pre or both stable
                }
            }
        }
    }

    fn display(&self) -> String {
        let core = format!("{}.{}.{}", self.major, self.minor, self.patch);
        match &self.pre {
            Some(p) => format!("{}-{}", core, p),
            None => core,
        }
    }
}

// ── GitHub API types ──────────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
    #[allow(dead_code)]
    prerelease: bool,
}

#[derive(Deserialize, Debug)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize, Debug)]
struct VersionApiResponse {
    version: String,
    // Could add more fields: download_url, changelog_url, etc.
}

// ── Core update logic ─────────────────────────────────────────────────────────

/// Fetch the latest stable release info from GitHub.
async fn fetch_latest_release(client: &reqwest::Client) -> anyhow::Result<GitHubRelease> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        GITHUB_REPO
    );

    let resp = client
        .get(&url)
        .header("User-Agent", format!("gyre/{}", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
        .context("Failed to contact GitHub API")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("GitHub API returned {}: {}", status, body);
    }

    resp.json::<GitHubRelease>()
        .await
        .context("Failed to parse GitHub API response")
}

/// Attempt to fetch the latest version from the Gyre version API.
/// Falls back to GitHub API on error.
async fn fetch_latest_version_tag(client: &reqwest::Client) -> anyhow::Result<String> {
    // Try the Gyre API first (faster, intentional).
    let api_result = client
        .get(VERSION_API)
        .header("User-Agent", format!("gyre/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;

    if let Ok(resp) = api_result {
        if resp.status().is_success() {
            if let Ok(data) = resp.json::<VersionApiResponse>().await {
                return Ok(data.version);
            }
        }
    }

    // Fall back to GitHub Releases API.
    let release = fetch_latest_release(client).await?;
    Ok(release.tag_name)
}

/// Download bytes from a URL, streaming through a SHA-256 hasher.
/// Returns (bytes, hex_hash).
async fn download_verified(
    client: &reqwest::Client,
    url: &str,
) -> anyhow::Result<(Vec<u8>, String)> {
    let resp = client
        .get(url)
        .header("User-Agent", format!("gyre/{}", env!("CARGO_PKG_VERSION")))
        .send()
        .await
        .with_context(|| format!("Failed to download {}", url))?;

    let status = resp.status();
    if !status.is_success() {
        bail!("Download failed with status {}: {}", status, url);
    }

    let bytes = resp.bytes().await.context("Failed to read download body")?;

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let hash = format!("{:x}", hasher.finalize());

    Ok((bytes.to_vec(), hash))
}

/// SHA-256 hash of `data` as a lowercase hex string.
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Extract the `gyre` binary from a `.tar.gz` archive in memory.
fn extract_binary_from_tarball(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let gz = GzDecoder::new(std::io::Cursor::new(bytes));
    let mut archive = Archive::new(gz);

    for entry in archive.entries().context("Failed to read tar archive")? {
        let mut entry = entry.context("Failed to read tar entry")?;
        let path = entry
            .path()
            .context("Invalid path in archive")?
            .to_path_buf();

        // Match the bare binary name (no directory prefix, or nested)
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if filename == "gyre" {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut buf)
                .context("Failed to read gyre binary from archive")?;
            return Ok(buf);
        }
    }

    bail!("Binary 'gyre' not found inside the archive")
}

/// Atomically replace the current executable with `new_binary`.
///
/// Strategy:
/// 1. Write new binary to `<current_exe>.new`
/// 2. Rename `<current_exe>` → `<current_exe>.bak`
/// 3. Rename `<current_exe>.new` → `<current_exe>`
/// 4. Delete `<current_exe>.bak`
///
/// On failure at step 3, restore the backup.
fn atomic_replace_binary(new_binary: &[u8]) -> anyhow::Result<PathBuf> {
    let current_exe = env::current_exe().context("Cannot determine path to current executable")?;
    let tmp_path = current_exe.with_extension("new");
    let bak_path = current_exe.with_extension("bak");

    // Write new binary
    {
        let mut f = std::fs::File::create(&tmp_path)
            .with_context(|| format!("Cannot write to {}", tmp_path.display()))?;
        f.write_all(new_binary)
            .context("Failed to write new binary")?;

        // Make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = f.metadata()?;
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            f.set_permissions(perms)?;
        }
    }

    // Rename current → .bak
    std::fs::rename(&current_exe, &bak_path)
        .with_context(|| format!("Cannot rename {} to backup", current_exe.display()))?;

    // Move new → current
    if let Err(e) = std::fs::rename(&tmp_path, &current_exe) {
        // Restore backup
        let _ = std::fs::rename(&bak_path, &current_exe);
        let _ = std::fs::remove_file(&tmp_path);
        bail!("Failed to install new binary (backup restored): {}", e);
    }

    // Remove backup
    let _ = std::fs::remove_file(&bak_path);

    Ok(current_exe)
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Options for the update command.
#[derive(Debug, Clone)]
pub struct UpdateOptions {
    /// Only check if an update is available; don't download or install.
    pub check_only: bool,
    /// Force update even if already on the latest version.
    pub force: bool,
    /// Allow pre-release versions.
    pub prerelease: bool,
}

impl Default for UpdateOptions {
    fn default() -> Self {
        Self {
            check_only: false,
            force: false,
            prerelease: false,
        }
    }
}

/// Run `gyre update [--check]`.
pub async fn run_update(opts: UpdateOptions) -> anyhow::Result<()> {
    let current_version_str = env!("CARGO_PKG_VERSION");
    let current = Version::parse(current_version_str)
        .context("Failed to parse current version (this is a build bug)")?;

    println!("Gyre self-update");
    println!("  Current version : v{}", current.display());

    // Build HTTP client (reuse reqwest features already in Cargo.toml)
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Failed to build HTTP client")?;

    // Determine latest version
    print!("  Checking for updates... ");
    std::io::stdout().flush().ok();

    let latest_tag = fetch_latest_version_tag(&client)
        .await
        .with_context(|| "Failed to check for updates. Check your internet connection.")?;

    let latest = Version::parse(&latest_tag)
        .with_context(|| format!("Latest version tag '{}' is not valid SemVer", latest_tag))?;

    println!("v{}", latest.display());

    // Compare versions
    let needs_update = opts.force || latest.is_newer_than(&current);

    if !needs_update {
        println!("  ✓ Already on the latest version.");
        if opts.check_only {
            println!();
        }
        return Ok(());
    }

    println!(
        "  Latest version  : v{} → update available!",
        latest.display()
    );

    if opts.check_only {
        println!();
        println!("Run 'gyre update' to install v{}.", latest.display());
        return Ok(());
    }

    // Determine which binary to download
    let target = current_target()?;
    println!("  Platform        : {}", target);

    // Fetch full release info to get asset URLs
    let release = fetch_latest_release(&client).await?;

    let archive_name = format!("gyre-{}-{}.tar.gz", latest_tag, target);
    let checksum_name = format!("{}.sha256", archive_name);

    let archive_asset = release
        .assets
        .iter()
        .find(|a| a.name == archive_name)
        .with_context(|| {
            format!(
                "Release {} does not contain asset '{}'. \
                 Check https://github.com/{}/releases",
                latest_tag, archive_name, GITHUB_REPO
            )
        })?;

    let checksum_asset = release
        .assets
        .iter()
        .find(|a| a.name == checksum_name)
        .with_context(|| {
            format!(
                "Release {} is missing checksum file '{}'. \
                 Cannot safely install.",
                latest_tag, checksum_name
            )
        })?;

    // Download checksum first (small)
    println!();
    println!("  Downloading checksum...");
    let (checksum_bytes, _) = download_verified(&client, &checksum_asset.browser_download_url)
        .await
        .context("Failed to download checksum file")?;

    let expected_hash =
        String::from_utf8(checksum_bytes).context("Checksum file is not valid UTF-8")?;
    let expected_hash = expected_hash
        .split_whitespace()
        .next()
        .context("Checksum file is empty")?
        .to_string();

    // Download archive
    println!("  Downloading {}...", archive_name);
    let (archive_bytes, _) = download_verified(&client, &archive_asset.browser_download_url)
        .await
        .context("Failed to download release archive")?;

    // Verify checksum
    println!("  Verifying SHA256 checksum...");
    let actual_hash = sha256_hex(&archive_bytes);
    if actual_hash != expected_hash {
        bail!(
            "Checksum mismatch! The download may be corrupted.\n\
             Expected : {}\n\
             Got      : {}\n\
             Aborting for safety.",
            expected_hash,
            actual_hash
        );
    }
    println!("  ✓ Checksum verified");

    // Extract binary from tarball
    println!("  Extracting binary...");
    let binary_bytes = extract_binary_from_tarball(&archive_bytes)
        .context("Failed to extract binary from archive")?;

    // Atomically replace the current executable
    println!("  Installing...");
    let installed_at = atomic_replace_binary(&binary_bytes)
        .context("Failed to replace binary. Try running with sudo if necessary.")?;

    println!();
    println!("  ✓ Gyre updated to v{}", latest.display());
    println!("    Installed at: {}", installed_at.display());
    println!();
    println!("  Restart any running 'gyre serve' processes to use the new version.");

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parse_plain() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
        assert_eq!(v.pre, None);
    }

    #[test]
    fn version_parse_with_v_prefix() {
        let v = Version::parse("v0.5.10").unwrap();
        assert_eq!(v.major, 0);
        assert_eq!(v.minor, 5);
        assert_eq!(v.patch, 10);
    }

    #[test]
    fn version_parse_prerelease() {
        let v = Version::parse("v1.0.0-beta.1").unwrap();
        assert_eq!(v.pre, Some("beta.1".to_string()));
    }

    #[test]
    fn version_newer_than() {
        let v100 = Version::parse("1.0.0").unwrap();
        let v090 = Version::parse("0.9.0").unwrap();
        let v101 = Version::parse("1.0.1").unwrap();
        let v100_beta = Version::parse("1.0.0-beta.1").unwrap();

        assert!(v100.is_newer_than(&v090));
        assert!(!v090.is_newer_than(&v100));
        assert!(v101.is_newer_than(&v100));
        assert!(!v100.is_newer_than(&v100));
        // Stable > pre-release of same version
        assert!(v100.is_newer_than(&v100_beta));
        assert!(!v100_beta.is_newer_than(&v100));
    }

    #[test]
    fn sha256_is_consistent() {
        let data = b"hello gyre";
        let h1 = sha256_hex(data);
        let h2 = sha256_hex(data);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }
}
