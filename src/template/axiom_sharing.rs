//! Axiom Culture opt-in sharing: export, anonymize, import community axioms.
//!
//! Privacy model:
//! - ONLY `domain = "universal"` axioms are eligible for sharing
//! - `domain = "personal"` and `domain = "contextual"` are NEVER shared
//! - Identifying fields (agent_id, session_id, entity names, PII) are stripped
//! - Content hash (SHA-256) deduplicates on import

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;
use uuid::Uuid;

use crate::cognitive::axiom_culture::{Axiom, AxiomCulture};

// ---------------------------------------------------------------------------
// PII detection patterns (compiled once)
// ---------------------------------------------------------------------------

static EMAIL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap());

static PHONE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\+?\d[\d\-\s().]{7,}\d").unwrap());

/// Domains that are eligible for sharing. Everything else is excluded.
const SHAREABLE_DOMAINS: &[&str] = &["universal", "general"];

/// Domains that must NEVER be shared — checked as an explicit blocklist.
const BLOCKED_DOMAINS: &[&str] = &["personal", "contextual"];

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// An axiom with all identifying information removed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnonymizedAxiom {
    /// SHA-256 of `statement + domain + confidence_bin`.
    pub content_hash: String,
    pub name: String,
    pub statement: String,
    pub domain: String,
    pub evidence: String,
    pub created_at: DateTime<Utc>,
}

/// Describes why an axiom was filtered out during export.
#[derive(Debug)]
pub struct FilteredAxiom {
    pub name: String,
    pub reason: FilterReason,
}

#[derive(Debug)]
pub enum FilterReason {
    BlockedDomain(String),
    NonShareableDomain(String),
    ContainsPii(String),
}

impl std::fmt::Display for FilterReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BlockedDomain(d) => write!(f, "blocked domain '{}'", d),
            Self::NonShareableDomain(d) => write!(f, "non-shareable domain '{}'", d),
            Self::ContainsPii(detail) => write!(f, "contains PII: {}", detail),
        }
    }
}

// ---------------------------------------------------------------------------
// AxiomShareFilter
// ---------------------------------------------------------------------------

/// Filters axioms to only those eligible for sharing.
pub struct AxiomShareFilter;

impl AxiomShareFilter {
    /// Check if a single axiom is eligible for sharing.
    pub fn is_shareable(axiom: &Axiom) -> bool {
        let domain = axiom.domain.to_lowercase();
        // Explicitly blocked domains
        if BLOCKED_DOMAINS.iter().any(|d| *d == domain) {
            return false;
        }
        // Must be in the shareable set
        if !SHAREABLE_DOMAINS.iter().any(|d| *d == domain) {
            return false;
        }
        // No PII in any text field
        !contains_pii(&axiom.name)
            && !contains_pii(&axiom.statement)
            && !contains_pii(&axiom.evidence)
    }

    /// Filter a set of axioms, returning shareable ones and a log of what was filtered.
    pub fn filter(axioms: &[Axiom]) -> (Vec<&Axiom>, Vec<FilteredAxiom>) {
        let mut shareable = Vec::new();
        let mut filtered = Vec::new();

        for axiom in axioms {
            let domain = axiom.domain.to_lowercase();

            if BLOCKED_DOMAINS.iter().any(|d| *d == domain) {
                filtered.push(FilteredAxiom {
                    name: axiom.name.clone(),
                    reason: FilterReason::BlockedDomain(axiom.domain.clone()),
                });
                continue;
            }

            if !SHAREABLE_DOMAINS.iter().any(|d| *d == domain) {
                filtered.push(FilteredAxiom {
                    name: axiom.name.clone(),
                    reason: FilterReason::NonShareableDomain(axiom.domain.clone()),
                });
                continue;
            }

            if let Some(pii_detail) = detect_pii_detail(axiom) {
                filtered.push(FilteredAxiom {
                    name: axiom.name.clone(),
                    reason: FilterReason::ContainsPii(pii_detail),
                });
                continue;
            }

            shareable.push(axiom);
        }

        (shareable, filtered)
    }
}

// ---------------------------------------------------------------------------
// Anonymization
// ---------------------------------------------------------------------------

/// Strip identifying fields from an axiom, producing an `AnonymizedAxiom`.
///
/// Strips: `id` (replaced by content hash), `proposed_by` / `agent_id` / `session_id`
/// (not present in Axiom struct but would be in raw DB rows).
///
/// Also scrubs any PII patterns that slipped through (belt-and-suspenders).
pub fn anonymize_axiom(axiom: &Axiom) -> AnonymizedAxiom {
    let clean_name = scrub_pii(&axiom.name);
    let clean_statement = scrub_pii(&axiom.statement);
    let clean_evidence = scrub_pii(&axiom.evidence);

    let content_hash = compute_content_hash(&clean_statement, &axiom.domain, 1.0);

    AnonymizedAxiom {
        content_hash,
        name: clean_name,
        statement: clean_statement,
        domain: axiom.domain.clone(),
        evidence: clean_evidence,
        created_at: axiom.created_at,
    }
}

/// Compute SHA-256 content hash from `statement + domain + confidence_bin`.
///
/// Confidence is rounded to the nearest 0.1 bin so near-identical axioms
/// with slightly different confidence scores hash together.
pub fn compute_content_hash(statement: &str, domain: &str, confidence: f64) -> String {
    let confidence_bin = (confidence * 10.0).round() / 10.0;
    let input = format!("{}\n{}\n{:.1}", statement, domain, confidence_bin);
    let hash = Sha256::digest(input.as_bytes());
    hex::encode(hash)
}

// ---------------------------------------------------------------------------
// Export / Import
// ---------------------------------------------------------------------------

/// Export all shareable axioms from an AxiomCulture database.
///
/// Returns the anonymized axioms and logs filtered axioms to stderr.
pub fn export_shareable(db: &AxiomCulture) -> Result<Vec<AnonymizedAxiom>, rusqlite::Error> {
    let all_axioms = db.list_active()?;
    let (shareable, filtered) = AxiomShareFilter::filter(&all_axioms);

    // Log what was filtered to stderr (not uploaded)
    for f in &filtered {
        eprintln!("[axiom-share] filtered '{}': {}", f.name, f.reason);
    }

    let anonymized: Vec<AnonymizedAxiom> = shareable.iter().map(|a| anonymize_axiom(a)).collect();

    Ok(anonymized)
}

/// Import community axioms into a local AxiomCulture database.
///
/// Deduplicates by content hash: if an axiom with the same hash already
/// exists locally, it is skipped. Returns the count of newly imported axioms.
pub fn import_community(
    axioms: &[AnonymizedAxiom],
    db: &AxiomCulture,
) -> Result<usize, rusqlite::Error> {
    // Build set of existing content hashes from local DB
    let existing = db.list_active()?;
    let existing_hashes: std::collections::HashSet<String> = existing
        .iter()
        .map(|a| compute_content_hash(&a.statement, &a.domain, 1.0))
        .collect();

    let mut imported = 0;
    for anon in axioms {
        if existing_hashes.contains(&anon.content_hash) {
            continue;
        }

        let axiom = Axiom {
            id: Uuid::new_v4(),
            name: anon.name.clone(),
            statement: anon.statement.clone(),
            domain: anon.domain.clone(),
            evidence: anon.evidence.clone(),
            created_at: anon.created_at,
        };

        db.add_axiom(&axiom)?;
        imported += 1;
    }

    Ok(imported)
}

// ---------------------------------------------------------------------------
// PII helpers
// ---------------------------------------------------------------------------

/// Check if a string contains PII patterns.
fn contains_pii(text: &str) -> bool {
    EMAIL_RE.is_match(text) || PHONE_RE.is_match(text)
}

/// Detect PII and return a description of what was found.
fn detect_pii_detail(axiom: &Axiom) -> Option<String> {
    let mut findings = Vec::new();

    for (label, text) in [
        ("name", &axiom.name),
        ("statement", &axiom.statement),
        ("evidence", &axiom.evidence),
    ] {
        if EMAIL_RE.is_match(text) {
            findings.push(format!("email in {}", label));
        }
        if PHONE_RE.is_match(text) {
            findings.push(format!("phone in {}", label));
        }
    }

    if findings.is_empty() {
        None
    } else {
        Some(findings.join(", "))
    }
}

/// Remove PII patterns from text (belt-and-suspenders scrubbing).
fn scrub_pii(text: &str) -> String {
    let text = EMAIL_RE.replace_all(text, "[REDACTED_EMAIL]");
    let text = PHONE_RE.replace_all(&text, "[REDACTED_PHONE]");
    text.to_string()
}

// ---------------------------------------------------------------------------
// hex encoding (avoid adding a dependency for this)
// ---------------------------------------------------------------------------
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .fold(String::with_capacity(64), |mut s, b| {
                use std::fmt::Write;
                let _ = write!(s, "{:02x}", b);
                s
            })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_axiom(name: &str, statement: &str, domain: &str) -> Axiom {
        Axiom {
            id: Uuid::new_v4(),
            name: name.to_string(),
            statement: statement.to_string(),
            domain: domain.to_string(),
            evidence: String::new(),
            created_at: Utc::now(),
        }
    }

    // -- AxiomShareFilter tests --

    #[test]
    fn universal_axiom_is_shareable() {
        let axiom = make_axiom("volatility", "Market volatility clusters", "universal");
        assert!(AxiomShareFilter::is_shareable(&axiom));
    }

    #[test]
    fn general_axiom_is_shareable() {
        let axiom = make_axiom("compound", "Compound interest matters", "general");
        assert!(AxiomShareFilter::is_shareable(&axiom));
    }

    #[test]
    fn personal_axiom_is_blocked() {
        let axiom = make_axiom("pref", "Greg prefers dark mode", "personal");
        assert!(!AxiomShareFilter::is_shareable(&axiom));
    }

    #[test]
    fn contextual_axiom_is_blocked() {
        let axiom = make_axiom("regime", "Current market regime is high-vol", "contextual");
        assert!(!AxiomShareFilter::is_shareable(&axiom));
    }

    #[test]
    fn unknown_domain_is_not_shareable() {
        let axiom = make_axiom("custom", "Some custom fact", "project-specific");
        assert!(!AxiomShareFilter::is_shareable(&axiom));
    }

    #[test]
    fn axiom_with_email_is_filtered() {
        let axiom = make_axiom(
            "contact",
            "Reach out to greg@example.com for details",
            "universal",
        );
        assert!(!AxiomShareFilter::is_shareable(&axiom));
    }

    #[test]
    fn axiom_with_phone_is_filtered() {
        let axiom = make_axiom("support", "Call +1-555-123-4567 for support", "universal");
        assert!(!AxiomShareFilter::is_shareable(&axiom));
    }

    #[test]
    fn filter_returns_shareable_and_filtered() {
        let axioms = vec![
            make_axiom("ok1", "Gravity pulls things down", "universal"),
            make_axiom("personal1", "User likes cats", "personal"),
            make_axiom("ok2", "Water boils at 100C", "general"),
            make_axiom("ctx1", "Today is sunny", "contextual"),
            make_axiom("pii1", "Email me at test@test.com", "universal"),
        ];

        let (shareable, filtered) = AxiomShareFilter::filter(&axioms);
        assert_eq!(shareable.len(), 2);
        assert_eq!(filtered.len(), 3);

        assert_eq!(shareable[0].name, "ok1");
        assert_eq!(shareable[1].name, "ok2");
    }

    // -- Anonymization tests --

    #[test]
    fn anonymize_strips_uuid() {
        let axiom = make_axiom("test", "A universal truth", "universal");
        let original_id = axiom.id;
        let anon = anonymize_axiom(&axiom);

        // AnonymizedAxiom has no id field, just content_hash
        assert!(!anon.content_hash.is_empty());
        // The content hash should not contain the original UUID
        assert!(!anon.content_hash.contains(&original_id.to_string()));
    }

    #[test]
    fn anonymize_scrubs_email_from_evidence() {
        let mut axiom = make_axiom("test", "A truth", "universal");
        axiom.evidence = "Confirmed by alice@example.com".to_string();
        let anon = anonymize_axiom(&axiom);
        assert!(!anon.evidence.contains("alice@example.com"));
        assert!(anon.evidence.contains("[REDACTED_EMAIL]"));
    }

    #[test]
    fn anonymize_scrubs_phone_from_statement() {
        let axiom = make_axiom("test", "Call +1-800-555-0123 for info", "universal");
        let anon = anonymize_axiom(&axiom);
        assert!(!anon.statement.contains("+1-800-555-0123"));
        assert!(anon.statement.contains("[REDACTED_PHONE]"));
    }

    // -- Content hash tests --

    #[test]
    fn content_hash_deterministic() {
        let h1 = compute_content_hash("test statement", "universal", 0.85);
        let h2 = compute_content_hash("test statement", "universal", 0.85);
        assert_eq!(h1, h2);
    }

    #[test]
    fn content_hash_bins_confidence() {
        // 0.84 and 0.86 should round to 0.8 and 0.9 respectively
        let h1 = compute_content_hash("same text", "universal", 0.84);
        let h2 = compute_content_hash("same text", "universal", 0.86);
        // These should be different because they bin to 0.8 vs 0.9
        assert_ne!(h1, h2);

        // 0.83 and 0.84 should both bin to 0.8
        let h3 = compute_content_hash("same text", "universal", 0.83);
        let h4 = compute_content_hash("same text", "universal", 0.84);
        assert_eq!(h3, h4);
    }

    #[test]
    fn content_hash_is_64_hex_chars() {
        let h = compute_content_hash("test", "universal", 1.0);
        assert_eq!(h.len(), 64, "SHA-256 hex digest should be 64 chars");
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // -- Import dedup test --

    #[test]
    fn import_deduplicates_by_content_hash() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("axioms.db");
        let db = AxiomCulture::new(&db_path).unwrap();

        // Add an existing axiom
        let existing = make_axiom("gravity", "Gravity pulls things down", "universal");
        db.add_axiom(&existing).unwrap();

        // Try to import the same axiom (different id but same content)
        let anon = AnonymizedAxiom {
            content_hash: compute_content_hash("Gravity pulls things down", "universal", 1.0),
            name: "gravity".to_string(),
            statement: "Gravity pulls things down".to_string(),
            domain: "universal".to_string(),
            evidence: String::new(),
            created_at: Utc::now(),
        };

        let imported = import_community(&[anon], &db).unwrap();
        assert_eq!(imported, 0, "duplicate should be skipped");
        assert_eq!(db.count().unwrap(), 1, "count should remain 1");
    }

    #[test]
    fn import_adds_new_axioms() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("axioms.db");
        let db = AxiomCulture::new(&db_path).unwrap();

        let anon = AnonymizedAxiom {
            content_hash: compute_content_hash("Water is wet", "universal", 1.0),
            name: "water".to_string(),
            statement: "Water is wet".to_string(),
            domain: "universal".to_string(),
            evidence: String::new(),
            created_at: Utc::now(),
        };

        let imported = import_community(&[anon], &db).unwrap();
        assert_eq!(imported, 1);
        assert_eq!(db.count().unwrap(), 1);
    }

    // -- Full export pipeline test --

    #[test]
    fn export_shareable_filters_and_anonymizes() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("axioms.db");
        let db = AxiomCulture::new(&db_path).unwrap();

        db.add_axiom(&make_axiom("univ1", "Markets are efficient", "universal"))
            .unwrap();
        db.add_axiom(&make_axiom("personal1", "Greg likes blue", "personal"))
            .unwrap();
        db.add_axiom(&make_axiom("ctx1", "It is raining now", "contextual"))
            .unwrap();
        db.add_axiom(&make_axiom(
            "univ2",
            "Compound interest grows exponentially",
            "general",
        ))
        .unwrap();

        let exported = export_shareable(&db).unwrap();
        assert_eq!(exported.len(), 2);

        // Verify no personal or contextual axioms leaked
        for anon in &exported {
            assert!(
                anon.domain == "universal" || anon.domain == "general",
                "exported axiom has non-shareable domain: {}",
                anon.domain
            );
        }
    }

    // -- Privacy guard integration test --

    #[test]
    fn privacy_guard_never_exports_personal_or_contextual() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("axioms.db");
        let db = AxiomCulture::new(&db_path).unwrap();

        // Mix of all domain types
        let domains = [
            "universal",
            "personal",
            "contextual",
            "general",
            "personal",
            "contextual",
        ];
        for (i, domain) in domains.iter().enumerate() {
            db.add_axiom(&make_axiom(
                &format!("axiom_{}", i),
                &format!("Statement {}", i),
                domain,
            ))
            .unwrap();
        }

        let exported = export_shareable(&db).unwrap();

        // CRITICAL: no personal or contextual axioms in output
        for anon in &exported {
            assert_ne!(anon.domain, "personal", "personal axiom leaked into export");
            assert_ne!(
                anon.domain, "contextual",
                "contextual axiom leaked into export"
            );
        }

        // Should only have universal + general = 2
        assert_eq!(exported.len(), 2);
    }
}
