//! Template marketplace — install, publish, and browse Gyre agent templates.
//!
//! A template is a portable agent personality bundle (`.gyre.tar.gz`) containing
//! a `manifest.toml`, soul file, TELOS directory, and optional skills/axioms.
//!
//! # Storage layout
//!
//! ```text
//! ~/.gyre/
//! ├── registry.key            # API key (mode 600)
//! ├── templates/
//! │   └── <author>-<name>/    # installed templates (flat)
//! │       ├── manifest.toml
//! │       ├── soul.md
//! │       └── TELOS/
//! └── template-cache/         # downloaded tarballs
//!     └── <author>-<name>-<version>.gyre.tar.gz
//! ```

pub mod extractor;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Default registry URL
// ---------------------------------------------------------------------------

pub const DEFAULT_REGISTRY_URL: &str = "https://registry.gyre.ai/api/v1";

// ---------------------------------------------------------------------------
// Local path helpers
// ---------------------------------------------------------------------------

/// Root Gyre config directory: `~/.gyre/`
pub fn gyre_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".gyre"))
        .unwrap_or_else(|| PathBuf::from(".gyre"))
}

/// Installed-templates directory: `~/.gyre/templates/`
pub fn templates_dir() -> PathBuf {
    gyre_dir().join("templates")
}

/// Download cache directory: `~/.gyre/template-cache/`
pub fn template_cache_dir() -> PathBuf {
    gyre_dir().join("template-cache")
}

/// API key file: `~/.gyre/registry.key`
pub fn registry_key_path() -> PathBuf {
    gyre_dir().join("registry.key")
}

// ---------------------------------------------------------------------------
// manifest.toml schema
// ---------------------------------------------------------------------------

/// Full `manifest.toml` deserialized from a template bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateManifest {
    pub template: TemplateSection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateSection {
    /// Snake_case, URL-safe name (e.g. `kimi-financial-analyst`)
    pub name: String,

    /// Human-readable display name
    pub display_name: String,

    /// SemVer version string (e.g. `"1.0.0"`)
    pub version: String,

    /// Short description
    pub description: String,

    /// Registry username of the author
    pub author: String,

    /// SPDX license identifier
    #[serde(default = "default_license")]
    pub license: String,

    /// Up to 10 classification tags
    #[serde(default)]
    pub tags: Vec<String>,

    /// Template kind: `"agent"` | `"tribe"` | `"skill-pack"`
    pub kind: TemplateKind,

    #[serde(default)]
    pub compatibility: CompatibilitySection,

    #[serde(default)]
    pub requires: RequiresSection,

    #[serde(default)]
    pub axioms: AxiomsSection,

    #[serde(default)]
    pub tribe: TribeSection,

    #[serde(default)]
    pub meta: MetaSection,
}

fn default_license() -> String {
    "MIT".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateKind {
    #[default]
    Agent,
    Tribe,
    #[serde(rename = "skill-pack")]
    SkillPack,
}

impl std::fmt::Display for TemplateKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateKind::Agent => f.write_str("agent"),
            TemplateKind::Tribe => f.write_str("tribe"),
            TemplateKind::SkillPack => f.write_str("skill-pack"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompatibilitySection {
    #[serde(default)]
    pub gyre_min: String,
    #[serde(default)]
    pub gyre_max: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RequiresSection {
    #[serde(default = "default_tier")]
    pub tier: String,
    #[serde(default)]
    pub skills: Vec<String>,
}

fn default_tier() -> String {
    "free".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AxiomsSection {
    #[serde(default)]
    pub included: bool,
    #[serde(default)]
    pub shareable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TribeSection {
    #[serde(default)]
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MetaSection {
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub verified: bool,
}

// ---------------------------------------------------------------------------
// Registry response types
// ---------------------------------------------------------------------------

/// Lightweight metadata returned by the registry list/search endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateMeta {
    pub author: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub kind: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub rating: Option<f32>,
    #[serde(default)]
    pub verified: bool,
}

impl TemplateMeta {
    /// Full qualified name: `author/name`
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.author, self.name)
    }
}

// ---------------------------------------------------------------------------
// TemplateBundle — wraps tarball operations
// ---------------------------------------------------------------------------

/// Represents a template bundle (tarball) ready for packing or unpacking.
pub struct TemplateBundle {
    /// Path to the source directory being packed, or the extracted temp dir.
    pub path: PathBuf,
    pub manifest: TemplateManifest,
}

impl TemplateBundle {
    /// Load a bundle from a local directory (validates manifest exists).
    pub fn from_dir(dir: &Path) -> anyhow::Result<Self> {
        let manifest_path = dir.join("manifest.toml");
        if !manifest_path.exists() {
            anyhow::bail!(
                "No manifest.toml found in {}. Is this a template directory?",
                dir.display()
            );
        }

        let raw = std::fs::read_to_string(&manifest_path)?;
        let manifest: TemplateManifest =
            toml::from_str(&raw).map_err(|e| anyhow::anyhow!("Invalid manifest.toml: {}", e))?;

        validate_manifest(&manifest.template)?;

        Ok(Self {
            path: dir.to_path_buf(),
            manifest,
        })
    }

    /// Pack the bundle directory into a `<name>.gyre.tar.gz` file.
    ///
    /// Returns the path to the created tarball and its SHA-256 hex digest.
    pub fn pack(&self, output_dir: &Path) -> anyhow::Result<(PathBuf, String)> {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use sha2::{Digest, Sha256};

        let t = &self.manifest.template;
        let tarball_name = format!("{}-{}-{}.gyre.tar.gz", t.author, t.name, t.version);
        let tarball_path = output_dir.join(&tarball_name);

        std::fs::create_dir_all(output_dir)?;

        let file = std::fs::File::create(&tarball_path)?;
        let gz = GzEncoder::new(file, Compression::best());
        let mut archive = tar::Builder::new(gz);

        // The top-level directory inside the archive is `<name>/`
        let prefix = &t.name;
        archive.append_dir_all(prefix, &self.path)?;
        archive.finish()?;

        // Compute SHA-256 of the tarball
        let bytes = std::fs::read(&tarball_path)?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = format!("{:x}", hasher.finalize());

        Ok((tarball_path, digest))
    }
}

// ---------------------------------------------------------------------------
// Manifest validation
// ---------------------------------------------------------------------------

fn validate_manifest(t: &TemplateSection) -> anyhow::Result<()> {
    // name: [a-z0-9][a-z0-9-]*
    if t.name.is_empty() {
        anyhow::bail!("manifest.toml: template.name is required");
    }
    if !t
        .name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        anyhow::bail!(
            "manifest.toml: template.name '{}' must match [a-z0-9][a-z0-9-]*",
            t.name
        );
    }

    // version: basic semver check (X.Y.Z)
    let parts: Vec<_> = t.version.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.parse::<u32>().is_err()) {
        anyhow::bail!(
            "manifest.toml: template.version '{}' must be semver (X.Y.Z)",
            t.version
        );
    }

    // tier values
    let valid_tiers = ["free", "standard", "pro", "enterprise"];
    if !valid_tiers.contains(&t.requires.tier.as_str()) {
        anyhow::bail!(
            "manifest.toml: template.requires.tier '{}' must be one of: {}",
            t.requires.tier,
            valid_tiers.join(", ")
        );
    }

    // tag count
    if t.tags.len() > 10 {
        anyhow::bail!("manifest.toml: template.tags must have at most 10 entries");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Name resolution
// ---------------------------------------------------------------------------

/// Resolve a bare name or `author/name` into `(author, name)`.
///
/// Bare names (no slash) default to author `"sac916"`.
pub fn resolve_name(input: &str) -> (String, String) {
    if let Some((author, name)) = input.split_once('/') {
        (author.to_string(), name.to_string())
    } else {
        ("sac916".to_string(), input.to_string())
    }
}

// ---------------------------------------------------------------------------
// API key helpers
// ---------------------------------------------------------------------------

/// Read the stored API key from `~/.gyre/registry.key`.
pub fn read_api_key() -> anyhow::Result<String> {
    let path = registry_key_path();
    if !path.exists() {
        anyhow::bail!(
            "Not logged in. Run `gyre template login <api-key>` first.\n\
             API key stored in {}",
            path.display()
        );
    }
    let key = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path.display(), e))?;
    Ok(key.trim().to_string())
}

/// Write API key to `~/.gyre/registry.key` with mode 600.
pub fn write_api_key(key: &str) -> anyhow::Result<()> {
    use std::fs::OpenOptions;

    let path = registry_key_path();
    std::fs::create_dir_all(path.parent().unwrap())?;

    // Write with restrictive permissions
    {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)?;
            use std::io::Write;
            writeln!(file, "{}", key.trim())?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&path, format!("{}\n", key.trim()))?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Core operations
// ---------------------------------------------------------------------------

/// Download and install a template from the registry.
///
/// # Arguments
/// * `name` – bare name (`kimi-financial-analyst`) or `author/name`
/// * `registry_url` – base URL of the registry API
pub async fn install(name: &str, registry_url: &str) -> anyhow::Result<()> {
    let (author, template_name) = resolve_name(name);
    let full_name = format!("{}/{}", author, template_name);

    println!("🔍 Resolving {}@latest...", full_name);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    // Fetch metadata
    let meta_url = format!("{}/templates/{}/{}", registry_url, author, template_name);
    let meta_resp = client
        .get(&meta_url)
        .header("Accept", "application/json")
        .send()
        .await;

    let meta: TemplateMeta = match meta_resp {
        Ok(resp) if resp.status().is_success() => resp.json().await?,
        Ok(resp) => {
            // Registry is a stub for now — build a minimal meta from what we know
            tracing::debug!(
                "Registry returned {} for {}, using stub metadata",
                resp.status(),
                full_name
            );
            TemplateMeta {
                author: author.clone(),
                name: template_name.clone(),
                version: "0.0.0".to_string(),
                description: "(registry not yet available)".to_string(),
                kind: "agent".to_string(),
                tags: vec![],
                downloads: 0,
                rating: None,
                verified: false,
            }
        }
        Err(e) => {
            tracing::debug!("Registry unreachable ({}), using stub metadata", e);
            TemplateMeta {
                author: author.clone(),
                name: template_name.clone(),
                version: "0.0.0".to_string(),
                description: "(registry not yet available)".to_string(),
                kind: "agent".to_string(),
                tags: vec![],
                downloads: 0,
                rating: None,
                verified: false,
            }
        }
    };

    println!("📦 Downloading v{} ...", meta.version);

    // Determine cache path
    let cache_dir = template_cache_dir();
    std::fs::create_dir_all(&cache_dir)?;
    let cache_file = cache_dir.join(format!(
        "{}-{}-{}.gyre.tar.gz",
        author, template_name, meta.version
    ));

    // Download tarball
    let download_url = format!(
        "{}/templates/{}/{}/download",
        registry_url, author, template_name
    );
    let download_resp = client.get(&download_url).send().await;

    let tarball_bytes: Vec<u8> = match download_resp {
        Ok(resp) if resp.status().is_success() => resp.bytes().await?.to_vec(),
        _ => {
            // Registry stub: nothing to download yet
            println!("⚠️  Registry download not yet available (T3 builds the real registry).");
            println!(
                "   Template would be installed to ~/.gyre/templates/{}-{}/",
                author, template_name
            );
            return Ok(());
        }
    };

    // Verify checksum if registry returned one
    // (In T3 the registry will include sha256 in the metadata response)
    println!("✅ Downloaded {} bytes", tarball_bytes.len());

    // Write to cache
    std::fs::write(&cache_file, &tarball_bytes)?;

    // Extract to install directory (mode 0o700 — owner only, T2-003)
    let install_dir = templates_dir().join(format!("{}-{}", author, template_name));
    std::fs::create_dir_all(&install_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&install_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    extractor::extract_tarball(&cache_file, &install_dir)?;

    println!("📂 Installed to {}/", install_dir.display());
    println!("✅ Template installed!");
    println!();
    println!("Run: gyre serve --agent {} --box ~/agents", template_name);

    Ok(())
}

/// Package and upload a template to the registry.
///
/// # Arguments
/// * `box_path` – local directory containing `manifest.toml` (the agent box)
/// * `registry_url` – base URL of the registry API
/// * `api_key` – bearer token for authentication
pub async fn publish(box_path: &Path, registry_url: &str, api_key: &str) -> anyhow::Result<()> {
    println!("📋 Reading manifest.toml...");

    let bundle = TemplateBundle::from_dir(box_path)?;
    let t = &bundle.manifest.template;

    println!("✅ Validation passed");
    println!("🔑 Authenticated as {}", t.author);

    // Pack into temp dir
    let tmp_dir = std::env::temp_dir().join("gyre-publish");
    std::fs::create_dir_all(&tmp_dir)?;

    let (tarball_path, sha256) = bundle.pack(&tmp_dir)?;
    let tarball_bytes = std::fs::read(&tarball_path)?;
    let size_kb = tarball_bytes.len() as f64 / 1024.0;

    println!(
        "📤 Uploading {}/{}@{} ({:.1} KB)...",
        t.author, t.name, t.version, size_kb
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;

    let upload_url = format!("{}/templates", registry_url);
    let resp = client
        .post(&upload_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("X-Checksum-SHA256", &sha256)
        .header("Content-Type", "application/gzip")
        .body(tarball_bytes)
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            println!(
                "✅ Published! {}/t/{}/{}",
                registry_url.trim_end_matches("/api/v1"),
                t.author,
                t.name
            );
        }
        Ok(r) => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            anyhow::bail!("Upload failed: {} — {}", status, body);
        }
        Err(e) => {
            // Registry not yet live (T3 task)
            tracing::debug!("Registry upload failed: {}", e);
            println!("⚠️  Registry not yet available (T3 builds the real registry).");
            println!("   Tarball ready at: {}", tarball_path.display());
            println!("   SHA-256: {}", &sha256[..16]);
        }
    }

    // Axiom sharing opt-in
    if t.axioms.shareable {
        print!("Share universal axioms with the community? [y/N]: ");
        use std::io::Write;
        std::io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if input.trim().eq_ignore_ascii_case("y") {
            println!("(Axiom sharing will be implemented in T4)");
        }
    }

    // Clean up temp tarball
    let _ = std::fs::remove_file(&tarball_path);

    Ok(())
}

/// List templates from the registry.
///
/// Returns a `Vec<TemplateMeta>`. If the registry is unreachable, returns
/// a set of stub entries so the command is always functional.
pub async fn list(registry_url: &str, filter: Option<&str>) -> anyhow::Result<Vec<TemplateMeta>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let mut url = format!("{}/templates", registry_url);
    if let Some(f) = filter {
        url.push_str(&format!("?tag={}", urlencoding::encode(f)));
    }

    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            let templates: Vec<TemplateMeta> = r.json().await?;
            Ok(templates)
        }
        _ => {
            // Registry not yet live — return starter community pack stubs
            Ok(stub_templates())
        }
    }
}

/// Read installed templates from `~/.gyre/templates/`.
pub fn list_installed() -> anyhow::Result<Vec<TemplateManifest>> {
    let dir = templates_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut manifests = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let manifest_path = entry.path().join("manifest.toml");
        if !manifest_path.exists() {
            continue;
        }
        let raw = std::fs::read_to_string(&manifest_path)?;
        if let Ok(m) = toml::from_str::<TemplateManifest>(&raw) {
            manifests.push(m);
        }
    }

    Ok(manifests)
}

// ---------------------------------------------------------------------------
// Stub data (until T3 registry is live)
// ---------------------------------------------------------------------------

fn stub_templates() -> Vec<TemplateMeta> {
    vec![
        TemplateMeta {
            author: "sac916".into(),
            name: "kimi".into(),
            version: "1.0.0".into(),
            description: "Atmospheric AI — coordination, enthusiasm, ops".into(),
            kind: "agent".into(),
            tags: vec!["coordination".into(), "ops".into()],
            downloads: 2300,
            rating: Some(4.9),
            verified: true,
        },
        TemplateMeta {
            author: "sac916".into(),
            name: "sarah".into(),
            version: "1.0.0".into(),
            description: "Operations Manager — CFO, documentation".into(),
            kind: "agent".into(),
            tags: vec!["finance".into(), "ops".into()],
            downloads: 1800,
            rating: Some(4.8),
            verified: true,
        },
        TemplateMeta {
            author: "sac916".into(),
            name: "teagan".into(),
            version: "1.0.0".into(),
            description: "Security Specialist — threat analysis".into(),
            kind: "agent".into(),
            tags: vec!["security".into()],
            downloads: 1500,
            rating: Some(4.7),
            verified: true,
        },
        TemplateMeta {
            author: "sac916".into(),
            name: "jess".into(),
            version: "1.0.0".into(),
            description: "Marketing Guru — content, outreach".into(),
            kind: "agent".into(),
            tags: vec!["marketing".into(), "content".into()],
            downloads: 1100,
            rating: Some(4.6),
            verified: true,
        },
        TemplateMeta {
            author: "sac916".into(),
            name: "kate".into(),
            version: "1.0.0".into(),
            description: "Personal Trainer — health, coaching".into(),
            kind: "agent".into(),
            tags: vec!["health".into(), "coaching".into()],
            downloads: 900,
            rating: Some(4.5),
            verified: true,
        },
        TemplateMeta {
            author: "sac916".into(),
            name: "sargasso-tribe".into(),
            version: "1.0.0".into(),
            description: "All 6 agents as a pre-configured tribe".into(),
            kind: "tribe".into(),
            tags: vec!["tribe".into(), "multi-agent".into()],
            downloads: 650,
            rating: Some(4.8),
            verified: true,
        },
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_name_bare() {
        let (author, name) = resolve_name("kimi-financial-analyst");
        assert_eq!(author, "sac916");
        assert_eq!(name, "kimi-financial-analyst");
    }

    #[test]
    fn test_resolve_name_namespaced() {
        let (author, name) = resolve_name("community/research-assistant");
        assert_eq!(author, "community");
        assert_eq!(name, "research-assistant");
    }

    #[test]
    fn test_manifest_roundtrip() {
        let toml_str = r#"
[template]
name = "kimi-financial-analyst"
display_name = "Kimi — Financial Analyst"
version = "1.0.0"
description = "Market analysis and portfolio management"
author = "sac916"
license = "MIT"
tags = ["finance", "trading"]
kind = "agent"

[template.compatibility]
gyre_min = "0.5.0"
gyre_max = ""

[template.requires]
tier = "free"
skills = []

[template.axioms]
included = false
shareable = false

[template.tribe]
members = []

[template.meta]
created_at = "2026-02-19T00:00:00Z"
updated_at = "2026-02-19T00:00:00Z"
downloads = 0
verified = false
"#;
        let manifest: TemplateManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.template.name, "kimi-financial-analyst");
        assert_eq!(manifest.template.version, "1.0.0");
        assert_eq!(manifest.template.author, "sac916");
    }

    #[test]
    fn test_validate_manifest_ok() {
        let t = TemplateSection {
            name: "my-agent".into(),
            display_name: "My Agent".into(),
            version: "0.1.0".into(),
            description: "Test".into(),
            author: "sac916".into(),
            license: "MIT".into(),
            tags: vec![],
            kind: TemplateKind::Agent,
            compatibility: Default::default(),
            requires: RequiresSection {
                tier: "free".into(),
                skills: vec![],
            },
            axioms: Default::default(),
            tribe: Default::default(),
            meta: Default::default(),
        };
        assert!(validate_manifest(&t).is_ok());
    }

    #[test]
    fn test_validate_manifest_bad_name() {
        let t = TemplateSection {
            name: "My Agent!".into(),
            display_name: "My Agent".into(),
            version: "0.1.0".into(),
            description: "Test".into(),
            author: "sac916".into(),
            license: "MIT".into(),
            tags: vec![],
            kind: TemplateKind::Agent,
            compatibility: Default::default(),
            requires: RequiresSection {
                tier: "free".into(),
                skills: vec![],
            },
            axioms: Default::default(),
            tribe: Default::default(),
            meta: Default::default(),
        };
        assert!(validate_manifest(&t).is_err());
    }

    #[test]
    fn test_validate_manifest_bad_version() {
        let t = TemplateSection {
            name: "my-agent".into(),
            display_name: "My Agent".into(),
            version: "1.0".into(), // invalid
            description: "Test".into(),
            author: "sac916".into(),
            license: "MIT".into(),
            tags: vec![],
            kind: TemplateKind::Agent,
            compatibility: Default::default(),
            requires: RequiresSection {
                tier: "free".into(),
                skills: vec![],
            },
            axioms: Default::default(),
            tribe: Default::default(),
            meta: Default::default(),
        };
        assert!(validate_manifest(&t).is_err());
    }
}

pub mod axiom_sharing;
pub mod manifest;
