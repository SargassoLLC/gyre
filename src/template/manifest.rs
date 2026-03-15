use serde::Deserialize;
use std::path::Path;

/// Parsed representation of a template's `manifest.toml`.
#[derive(Debug, Deserialize)]
pub struct TemplateManifest {
    pub template: TemplateSection,
}

#[derive(Debug, Deserialize)]
pub struct TemplateSection {
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub compatibility: Option<CompatibilitySection>,
    #[serde(default)]
    pub requires: Option<RequiresSection>,
    #[serde(default)]
    pub axioms: Option<AxiomsSection>,
    #[serde(default)]
    pub tribe: Option<TribeSection>,
    #[serde(default)]
    pub meta: Option<MetaSection>,
}

fn default_kind() -> String {
    "agent".to_string()
}

#[derive(Debug, Deserialize)]
pub struct CompatibilitySection {
    #[serde(default)]
    pub gyre_min: String,
    #[serde(default)]
    pub gyre_max: String,
}

#[derive(Debug, Deserialize)]
pub struct RequiresSection {
    #[serde(default = "default_tier")]
    pub tier: String,
    #[serde(default)]
    pub skills: Vec<String>,
}

fn default_tier() -> String {
    "free".to_string()
}

#[derive(Debug, Default, Deserialize)]
pub struct AxiomsSection {
    #[serde(default)]
    pub included: bool,
    #[serde(default)]
    pub shareable: bool,
}

#[derive(Debug, Deserialize)]
pub struct TribeSection {
    #[serde(default)]
    pub members: Vec<String>,
}

#[derive(Debug, Deserialize)]
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

impl TemplateManifest {
    /// Parse a manifest from TOML text.
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// Load and parse a manifest from a file path.
    pub fn from_file(path: &Path) -> Result<Self, ManifestError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ManifestError::Io(path.display().to_string(), e))?;
        Self::parse(&text).map_err(ManifestError::Parse)
    }

    /// Whether this template includes axioms that are marked shareable.
    pub fn axioms_shareable(&self) -> bool {
        self.template
            .axioms
            .as_ref()
            .is_some_and(|a| a.included && a.shareable)
    }
}

#[derive(Debug)]
pub enum ManifestError {
    Io(String, std::io::Error),
    Parse(toml::de::Error),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(path, e) => write!(f, "failed to read {}: {}", path, e),
            Self::Parse(e) => write!(f, "manifest parse error: {}", e),
        }
    }
}

impl std::error::Error for ManifestError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_manifest() {
        let toml = r#"
[template]
name = "test-agent"
"#;
        let m = TemplateManifest::parse(toml).unwrap();
        assert_eq!(m.template.name, "test-agent");
        assert_eq!(m.template.kind, "agent");
        assert!(!m.axioms_shareable());
    }

    #[test]
    fn parse_full_manifest_with_axioms() {
        let toml = r#"
[template]
name = "kimi-financial-analyst"
display_name = "Kimi — Financial Analyst"
version = "1.0.0"
description = "Financial analysis assistant"
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
included = true
shareable = true

[template.meta]
created_at = "2026-02-19T00:00:00Z"
updated_at = "2026-02-19T00:00:00Z"
downloads = 0
verified = false
"#;
        let m = TemplateManifest::parse(toml).unwrap();
        assert_eq!(m.template.name, "kimi-financial-analyst");
        assert!(m.axioms_shareable());
        let axioms = m.template.axioms.unwrap();
        assert!(axioms.included);
        assert!(axioms.shareable);
    }

    #[test]
    fn axioms_shareable_requires_both_flags() {
        let toml = r#"
[template]
name = "test"

[template.axioms]
included = false
shareable = true
"#;
        let m = TemplateManifest::parse(toml).unwrap();
        assert!(
            !m.axioms_shareable(),
            "shareable without included should be false"
        );
    }
}
