//! Egress policy — network-boundary audit and enforcement for native tools.
//!
//! Sub-agents and WASM tools are already sandboxed (Docker, capability
//! allowlists, credential injection, leak scanning). The main session's
//! native tools reach the network with host access, gated only by approval
//! prompts. This module closes that gap: every outbound request passes a
//! leak scan and a host rule match, and every decision lands in the
//! `egress_events` audit log.
//!
//! Rules are boundaries, not judgment: exact or `*.suffix` host matching
//! only. No pattern may judge intent — that is the judge's job (judge mode)
//! or the user's.
//!
//! Shell subprocesses (`curl` etc.) cannot be intercepted in-process; that
//! is an OS boundary. Run the main session in the Docker sandbox profile or
//! point `HTTP_PROXY`/`HTTPS_PROXY` at a real proxy for that layer.

use std::fmt;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::config::EgressConfig;
use crate::db::Database;
use crate::safety::LeakDetector;

/// What happens to egress that matches no configured rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgressMode {
    /// Allow unmatched egress, audit everything (default at beta).
    Observe,
    /// Deny unmatched egress, audit everything.
    Enforce,
    /// One LLM call decides unmatched egress; fail closed.
    Judge,
}

impl EgressMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Enforce => "enforce",
            Self::Judge => "judge",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "observe" => Ok(Self::Observe),
            "enforce" => Ok(Self::Enforce),
            "judge" => Ok(Self::Judge),
            other => Err(format!(
                "unknown egress mode '{}' (expected observe | enforce | judge)",
                other
            )),
        }
    }
}

impl fmt::Display for EgressMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The outcome of an egress check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgressDecision {
    Allowed,
    Denied,
}

impl EgressDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "allowed" => Ok(Self::Allowed),
            "denied" => Ok(Self::Denied),
            other => Err(format!("unknown egress decision '{}'", other)),
        }
    }
}

impl fmt::Display for EgressDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One audited egress decision. Persisted to `egress_events` on both
/// database backends; surfaced later via `gyre egress log` and the gateway.
#[derive(Debug, Clone)]
pub struct EgressEvent {
    pub id: Uuid,
    pub ts: DateTime<Utc>,
    /// Tool that initiated the request (`http`, a built tool name, or a WASM tool).
    pub tool: String,
    pub method: String,
    pub host: String,
    pub path: String,
    pub decision: EgressDecision,
    /// The layer/mode that produced the decision: `observe` | `enforce` |
    /// `judge` | `rule` | `leak-scan` | `wasm-allowlist`.
    pub mode: String,
    /// Matched rule or judge reason; empty when unmatched in observe mode.
    pub reason: String,
    /// `clean`, or `blocked: <pattern>` when the leak scan fired.
    pub leak_verdict: String,
}

/// Fields describing an outbound request to be checked.
///
/// `url` is the full URL (scanned for leaks); `host`/`path` are the parsed
/// components used for rule matching and the audit record.
pub struct EgressRequest<'a> {
    pub tool: &'a str,
    pub method: &'a str,
    pub url: &'a str,
    pub host: &'a str,
    pub path: &'a str,
    pub headers: &'a [(String, String)],
    pub body: Option<&'a [u8]>,
}

/// Error returned when an egress check blocks a request.
#[derive(Debug, Clone, thiserror::Error)]
pub enum EgressError {
    #[error("egress blocked by leak scan: {0}")]
    LeakBlocked(String),
    #[error("egress denied: host '{host}' {reason}")]
    Denied { host: String, reason: String },
}

/// Best-effort audit sink: every event is traced; when a database is
/// attached, events are also drained into `egress_events` by a background
/// task so the (possibly sync) caller never waits on a write.
pub struct EgressAuditor {
    tx: Option<mpsc::UnboundedSender<EgressEvent>>,
}

impl EgressAuditor {
    /// Auditor that only emits tracing events (no database available).
    pub fn log_only() -> Arc<Self> {
        Arc::new(Self { tx: None })
    }

    /// Spawn a background writer draining events into the database.
    pub fn spawn(db: Arc<dyn Database>) -> Arc<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<EgressEvent>();
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Err(e) = db.record_egress_event(&event).await {
                    tracing::warn!(error = %e, host = %event.host, "Failed to persist egress event");
                }
            }
        });
        Arc::new(Self { tx: Some(tx) })
    }

    /// Record an event: always traced, persisted when a database is attached.
    pub fn record(&self, event: EgressEvent) {
        tracing::info!(
            tool = %event.tool,
            method = %event.method,
            host = %event.host,
            path = %event.path,
            decision = %event.decision,
            mode = %event.mode,
            reason = %event.reason,
            leak = %event.leak_verdict,
            "egress"
        );
        if let Some(tx) = &self.tx {
            // Receiver only drops at shutdown; losing a tail event then is fine.
            let _ = tx.send(event);
        }
    }
}

/// How a host matched the configured rules.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RuleMatch {
    Allow(String),
    Deny(String),
    Unmatched,
}

/// Egress policy on the native tool HTTP boundary.
pub struct EgressPolicy {
    mode: EgressMode,
    allow: Vec<String>,
    deny: Vec<String>,
    leak_detector: LeakDetector,
    auditor: Arc<EgressAuditor>,
}

impl EgressPolicy {
    pub fn new(config: &EgressConfig, auditor: Arc<EgressAuditor>) -> Self {
        Self {
            mode: config.mode,
            allow: config.allow.clone(),
            deny: config.deny.clone(),
            leak_detector: LeakDetector::new(),
            auditor,
        }
    }

    pub fn mode(&self) -> EgressMode {
        self.mode
    }

    /// Check an outbound request: leak scan, then host rules, then the
    /// configured mode for unmatched hosts. Every path audits.
    ///
    /// Explicit rules apply in every mode (deny wins over allow); the mode
    /// only governs hosts that match no rule.
    pub fn check_request(&self, req: &EgressRequest<'_>) -> Result<(), EgressError> {
        // 1. Leak scan (blocks in all modes — this boundary predates egress policy)
        if let Err(e) = self
            .leak_detector
            .scan_http_request(req.url, req.headers, req.body)
        {
            let verdict = format!("blocked: {}", e);
            self.audit(req, EgressDecision::Denied, "leak-scan", &verdict, &verdict);
            return Err(EgressError::LeakBlocked(e.to_string()));
        }

        // 2. Explicit rules — boundaries the user drew, honored in every mode
        match self.match_rules(req.host) {
            RuleMatch::Deny(rule) => {
                let reason = format!("matched deny rule '{}'", rule);
                self.audit(req, EgressDecision::Denied, "rule", &reason, "clean");
                Err(EgressError::Denied {
                    host: req.host.to_string(),
                    reason,
                })
            }
            RuleMatch::Allow(rule) => {
                let reason = format!("matched allow rule '{}'", rule);
                self.audit(req, EgressDecision::Allowed, "rule", &reason, "clean");
                Ok(())
            }
            // 3. Unmatched → mode decides
            RuleMatch::Unmatched => self.check_unmatched(req),
        }
    }

    fn check_unmatched(&self, req: &EgressRequest<'_>) -> Result<(), EgressError> {
        match self.mode {
            EgressMode::Observe => {
                self.audit(req, EgressDecision::Allowed, "observe", "", "clean");
                Ok(())
            }
            EgressMode::Enforce => {
                let reason = "matched no allow rule (enforce mode)".to_string();
                self.audit(req, EgressDecision::Denied, "enforce", &reason, "clean");
                Err(EgressError::Denied {
                    host: req.host.to_string(),
                    reason,
                })
            }
            // Judge lands in its own phase; until then it fails closed
            // (enforce-deny) rather than silently allowing — the same
            // fallback the judge itself uses on timeout or parse failure.
            EgressMode::Judge => {
                let reason = "judge not yet implemented; failing closed (deny)".to_string();
                tracing::warn!(host = %req.host, "egress judge not yet implemented; denying");
                self.audit(req, EgressDecision::Denied, "judge", &reason, "clean");
                Err(EgressError::Denied {
                    host: req.host.to_string(),
                    reason,
                })
            }
        }
    }

    fn match_rules(&self, host: &str) -> RuleMatch {
        // Deny wins over allow
        for rule in &self.deny {
            if host_matches(rule, host) {
                return RuleMatch::Deny(rule.clone());
            }
        }
        for rule in &self.allow {
            if host_matches(rule, host) {
                return RuleMatch::Allow(rule.clone());
            }
        }
        RuleMatch::Unmatched
    }

    fn audit(
        &self,
        req: &EgressRequest<'_>,
        decision: EgressDecision,
        mode: &str,
        reason: &str,
        leak_verdict: &str,
    ) {
        self.auditor.record(EgressEvent {
            id: Uuid::new_v4(),
            ts: Utc::now(),
            tool: req.tool.to_string(),
            method: req.method.to_string(),
            host: req.host.to_string(),
            path: req.path.to_string(),
            decision,
            mode: mode.to_string(),
            reason: reason.to_string(),
            leak_verdict: leak_verdict.to_string(),
        });
    }
}

/// Exact or `*.suffix` host matching. `*.example.com` matches subdomains
/// only, not the apex — list both when both are intended. Case-insensitive;
/// trailing dots ignored. No pattern may judge intent.
fn host_matches(pattern: &str, host: &str) -> bool {
    let p = pattern.trim().trim_end_matches('.').to_ascii_lowercase();
    let h = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if p.is_empty() || h.is_empty() {
        return false;
    }
    if let Some(suffix) = p.strip_prefix("*.") {
        h.len() > suffix.len() + 1
            && h.ends_with(suffix)
            && h.as_bytes()[h.len() - suffix.len() - 1] == b'.'
    } else {
        h == p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(mode: EgressMode, allow: &[&str], deny: &[&str]) -> EgressConfig {
        EgressConfig {
            mode,
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: deny.iter().map(|s| s.to_string()).collect(),
            judge_max_latency_ms: 3000,
        }
    }

    fn request<'a>(host: &'a str, url: &'a str) -> EgressRequest<'a> {
        EgressRequest {
            tool: "http",
            method: "GET",
            url,
            host,
            path: "/",
            headers: &[],
            body: None,
        }
    }

    #[test]
    fn test_host_matches_exact() {
        assert!(host_matches("api.anthropic.com", "api.anthropic.com"));
        assert!(host_matches("API.Anthropic.COM", "api.anthropic.com"));
        assert!(host_matches("api.anthropic.com", "api.anthropic.com."));
        assert!(!host_matches("api.anthropic.com", "anthropic.com"));
        assert!(!host_matches(
            "api.anthropic.com",
            "evil-api.anthropic.com.attacker.io"
        ));
    }

    #[test]
    fn test_host_matches_suffix_subdomains_only() {
        assert!(host_matches(
            "*.githubusercontent.com",
            "raw.githubusercontent.com"
        ));
        assert!(host_matches(
            "*.githubusercontent.com",
            "a.b.githubusercontent.com"
        ));
        // Apex is NOT matched by a wildcard
        assert!(!host_matches(
            "*.githubusercontent.com",
            "githubusercontent.com"
        ));
        // Suffix must be on a label boundary
        assert!(!host_matches(
            "*.githubusercontent.com",
            "evilgithubusercontent.com"
        ));
    }

    #[test]
    fn test_host_matches_empty_never_matches() {
        assert!(!host_matches("", "example.com"));
        assert!(!host_matches("*.", "example.com"));
        assert!(!host_matches("example.com", ""));
    }

    #[test]
    fn test_observe_allows_unmatched() {
        let policy = EgressPolicy::new(
            &config(EgressMode::Observe, &["api.anthropic.com"], &[]),
            EgressAuditor::log_only(),
        );
        let req = request("unknown.example.com", "https://unknown.example.com/");
        assert!(policy.check_request(&req).is_ok());
    }

    #[test]
    fn test_explicit_deny_blocks_even_in_observe() {
        let policy = EgressPolicy::new(
            &config(EgressMode::Observe, &[], &["evil.example.com"]),
            EgressAuditor::log_only(),
        );
        let req = request("evil.example.com", "https://evil.example.com/");
        let err = policy.check_request(&req).unwrap_err();
        assert!(matches!(err, EgressError::Denied { .. }));
    }

    #[test]
    fn test_deny_wins_over_allow() {
        let policy = EgressPolicy::new(
            &config(
                EgressMode::Observe,
                &["*.example.com"],
                &["evil.example.com"],
            ),
            EgressAuditor::log_only(),
        );
        let denied = request("evil.example.com", "https://evil.example.com/");
        assert!(policy.check_request(&denied).is_err());
        let allowed = request("good.example.com", "https://good.example.com/");
        assert!(policy.check_request(&allowed).is_ok());
    }

    #[test]
    fn test_leak_scan_blocks_secret_in_url() {
        let policy = EgressPolicy::new(
            &config(EgressMode::Observe, &["api.example.com"], &[]),
            EgressAuditor::log_only(),
        );
        let url = "https://api.example.com/steal?key=AKIATESTONLY7EXAMPLE1";
        let req = request("api.example.com", url);
        let err = policy.check_request(&req).unwrap_err();
        assert!(matches!(err, EgressError::LeakBlocked(_)));
    }

    #[test]
    fn test_enforce_denies_unmatched() {
        let policy = EgressPolicy::new(
            &config(EgressMode::Enforce, &["api.anthropic.com"], &[]),
            EgressAuditor::log_only(),
        );
        let req = request("unknown.example.com", "https://unknown.example.com/");
        let err = policy.check_request(&req).unwrap_err();
        match err {
            EgressError::Denied { host, reason } => {
                assert_eq!(host, "unknown.example.com");
                assert!(reason.contains("enforce"));
            }
            other => panic!("expected Denied, got {:?}", other),
        }
    }

    #[test]
    fn test_enforce_allows_allow_listed() {
        let policy = EgressPolicy::new(
            &config(
                EgressMode::Enforce,
                &["api.anthropic.com", "*.githubusercontent.com"],
                &[],
            ),
            EgressAuditor::log_only(),
        );
        let exact = request("api.anthropic.com", "https://api.anthropic.com/v1/messages");
        assert!(policy.check_request(&exact).is_ok());
        let suffix = request(
            "raw.githubusercontent.com",
            "https://raw.githubusercontent.com/a/b",
        );
        assert!(policy.check_request(&suffix).is_ok());
    }

    #[test]
    fn test_enforce_empty_allow_denies_everything_unmatched() {
        let policy = EgressPolicy::new(
            &config(EgressMode::Enforce, &[], &[]),
            EgressAuditor::log_only(),
        );
        let req = request("example.com", "https://example.com/");
        assert!(policy.check_request(&req).is_err());
    }

    #[test]
    fn test_judge_unimplemented_fails_closed() {
        let policy = EgressPolicy::new(
            &config(EgressMode::Judge, &["api.anthropic.com"], &[]),
            EgressAuditor::log_only(),
        );
        // Allow-listed host still passes without consulting the judge
        let listed = request("api.anthropic.com", "https://api.anthropic.com/");
        assert!(policy.check_request(&listed).is_ok());
        // Unmatched host is denied until the judge lands
        let unmatched = request("unknown.example.com", "https://unknown.example.com/");
        assert!(policy.check_request(&unmatched).is_err());
    }

    #[test]
    fn test_mode_parse() {
        assert_eq!(EgressMode::parse("observe"), Ok(EgressMode::Observe));
        assert_eq!(EgressMode::parse("Enforce"), Ok(EgressMode::Enforce));
        assert_eq!(EgressMode::parse(" judge "), Ok(EgressMode::Judge));
        assert!(EgressMode::parse("block").is_err());
    }

    #[test]
    fn test_decision_roundtrip() {
        for d in [EgressDecision::Allowed, EgressDecision::Denied] {
            assert_eq!(EgressDecision::parse(d.as_str()), Ok(d));
        }
        assert!(EgressDecision::parse("maybe").is_err());
    }
}
