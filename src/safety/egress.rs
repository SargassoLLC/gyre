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
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider};
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
    /// Model consulted for unmatched hosts in judge mode. Judgment lives in
    /// the model, boundaries in the rules; without a model, judge mode
    /// fails closed.
    judge_llm: Option<Arc<dyn LlmProvider>>,
    judge_max_latency_ms: u64,
}

impl EgressPolicy {
    pub fn new(config: &EgressConfig, auditor: Arc<EgressAuditor>) -> Self {
        Self {
            mode: config.mode,
            allow: config.allow.clone(),
            deny: config.deny.clone(),
            leak_detector: LeakDetector::new(),
            auditor,
            judge_llm: None,
            judge_max_latency_ms: config.judge_max_latency_ms,
        }
    }

    /// Attach the model consulted in judge mode.
    pub fn with_judge(mut self, llm: Arc<dyn LlmProvider>) -> Self {
        self.judge_llm = Some(llm);
        self
    }

    pub fn mode(&self) -> EgressMode {
        self.mode
    }

    /// Check an outbound request: leak scan, then host rules, then the
    /// configured mode for unmatched hosts. Every path audits.
    ///
    /// Explicit rules apply in every mode (deny wins over allow); the mode
    /// only governs hosts that match no rule.
    pub async fn check_request(&self, req: &EgressRequest<'_>) -> Result<(), EgressError> {
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
            RuleMatch::Unmatched => self.check_unmatched(req).await,
        }
    }

    async fn check_unmatched(&self, req: &EgressRequest<'_>) -> Result<(), EgressError> {
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
            EgressMode::Judge => self.judge_unmatched(req).await,
        }
    }

    /// One LLM call decides an unmatched host. Every failure path — no
    /// model attached, timeout, provider error, unparseable verdict —
    /// fails closed to deny, same as `routine_test`'s readiness rule.
    async fn judge_unmatched(&self, req: &EgressRequest<'_>) -> Result<(), EgressError> {
        let deny = |reason: String| -> Result<(), EgressError> {
            self.audit(req, EgressDecision::Denied, "judge", &reason, "clean");
            Err(EgressError::Denied {
                host: req.host.to_string(),
                reason,
            })
        };

        let Some(llm) = &self.judge_llm else {
            tracing::warn!(host = %req.host, "egress judge mode with no judge model; denying");
            return deny("judge mode with no judge model configured (fail closed)".to_string());
        };

        let prompt = judge_prompt(req);
        let request =
            CompletionRequest::new(vec![ChatMessage::user(&prompt)]).with_temperature(0.2);

        let timeout = std::time::Duration::from_millis(self.judge_max_latency_ms);
        let response = match tokio::time::timeout(timeout, llm.complete(request)).await {
            Err(_) => {
                return deny(format!(
                    "judge timed out after {}ms (fail closed)",
                    self.judge_max_latency_ms
                ));
            }
            Ok(Err(e)) => return deny(format!("judge LLM call failed: {e} (fail closed)")),
            Ok(Ok(resp)) => resp,
        };

        match parse_judge_verdict(&response.content) {
            Some(verdict) if verdict.allow => {
                let reason = format!("judge allowed: {}", verdict.reason);
                self.audit(req, EgressDecision::Allowed, "judge", &reason, "clean");
                Ok(())
            }
            Some(verdict) => deny(format!("judge denied: {}", verdict.reason)),
            None => deny("judge returned unparseable verdict (fail closed)".to_string()),
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

/// Build the judge prompt from a redacted request summary: destination,
/// method, tool, header names (never values), and body size. Query strings
/// and bodies are not forwarded — the judge rules on destination and shape,
/// not content.
fn judge_prompt(req: &EgressRequest<'_>) -> String {
    let header_names: Vec<&str> = req.headers.iter().map(|(name, _)| name.as_str()).collect();
    format!(
        "You are the egress judge for a personal AI agent. A tool is about \
         to make an outbound HTTP request to a host that matches no \
         configured allow or deny rule. Decide whether to permit it.\n\n\
         Be skeptical: unfamiliar hosts contacted by autonomous tools are \
         the main exfiltration channel. Consider whether the destination is \
         a well-known legitimate service and whether it makes sense for \
         this tool to contact it.\n\n\
         Request summary (values redacted):\n\
         - tool: {}\n\
         - method: {}\n\
         - host: {}\n\
         - path: {}\n\
         - header names: [{}]\n\
         - body size: {} bytes\n\n\
         Respond with a single JSON object and nothing else:\n\
         {{\"allow\": <true|false>, \"reason\": \"<one short sentence>\"}}\n\
         If uncertain, deny.",
        req.tool,
        req.method,
        req.host,
        req.path,
        header_names.join(", "),
        req.body.map(<[u8]>::len).unwrap_or(0),
    )
}

/// Parsed judge verdict.
struct JudgeVerdict {
    allow: bool,
    reason: String,
}

/// Parse the judge's JSON verdict. Structural parsing only — `None`
/// (unparseable) must be treated as deny by the caller.
fn parse_judge_verdict(content: &str) -> Option<JudgeVerdict> {
    #[derive(serde::Deserialize)]
    struct Wire {
        allow: bool,
        #[serde(default)]
        reason: String,
    }

    let candidate = crate::agent::attention::strip_code_fence(content.trim());
    serde_json::from_str::<Wire>(candidate)
        .ok()
        .map(|w| JudgeVerdict {
            allow: w.allow,
            reason: if w.reason.trim().is_empty() {
                "(no reason given)".to_string()
            } else {
                w.reason.trim().to_string()
            },
        })
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

    #[tokio::test]
    async fn test_observe_allows_unmatched() {
        let policy = EgressPolicy::new(
            &config(EgressMode::Observe, &["api.anthropic.com"], &[]),
            EgressAuditor::log_only(),
        );
        let req = request("unknown.example.com", "https://unknown.example.com/");
        assert!(policy.check_request(&req).await.is_ok());
    }

    #[tokio::test]
    async fn test_explicit_deny_blocks_even_in_observe() {
        let policy = EgressPolicy::new(
            &config(EgressMode::Observe, &[], &["evil.example.com"]),
            EgressAuditor::log_only(),
        );
        let req = request("evil.example.com", "https://evil.example.com/");
        let err = policy.check_request(&req).await.unwrap_err();
        assert!(matches!(err, EgressError::Denied { .. }));
    }

    #[tokio::test]
    async fn test_deny_wins_over_allow() {
        let policy = EgressPolicy::new(
            &config(
                EgressMode::Observe,
                &["*.example.com"],
                &["evil.example.com"],
            ),
            EgressAuditor::log_only(),
        );
        let denied = request("evil.example.com", "https://evil.example.com/");
        assert!(policy.check_request(&denied).await.is_err());
        let allowed = request("good.example.com", "https://good.example.com/");
        assert!(policy.check_request(&allowed).await.is_ok());
    }

    #[tokio::test]
    async fn test_leak_scan_blocks_secret_in_url() {
        let policy = EgressPolicy::new(
            &config(EgressMode::Observe, &["api.example.com"], &[]),
            EgressAuditor::log_only(),
        );
        let url = "https://api.example.com/steal?key=AKIATESTONLY7EXAMPLE1";
        let req = request("api.example.com", url);
        let err = policy.check_request(&req).await.unwrap_err();
        assert!(matches!(err, EgressError::LeakBlocked(_)));
    }

    #[tokio::test]
    async fn test_enforce_denies_unmatched() {
        let policy = EgressPolicy::new(
            &config(EgressMode::Enforce, &["api.anthropic.com"], &[]),
            EgressAuditor::log_only(),
        );
        let req = request("unknown.example.com", "https://unknown.example.com/");
        let err = policy.check_request(&req).await.unwrap_err();
        match err {
            EgressError::Denied { host, reason } => {
                assert_eq!(host, "unknown.example.com");
                assert!(reason.contains("enforce"));
            }
            other => panic!("expected Denied, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_enforce_allows_allow_listed() {
        let policy = EgressPolicy::new(
            &config(
                EgressMode::Enforce,
                &["api.anthropic.com", "*.githubusercontent.com"],
                &[],
            ),
            EgressAuditor::log_only(),
        );
        let exact = request("api.anthropic.com", "https://api.anthropic.com/v1/messages");
        assert!(policy.check_request(&exact).await.is_ok());
        let suffix = request(
            "raw.githubusercontent.com",
            "https://raw.githubusercontent.com/a/b",
        );
        assert!(policy.check_request(&suffix).await.is_ok());
    }

    #[tokio::test]
    async fn test_enforce_empty_allow_denies_everything_unmatched() {
        let policy = EgressPolicy::new(
            &config(EgressMode::Enforce, &[], &[]),
            EgressAuditor::log_only(),
        );
        let req = request("example.com", "https://example.com/");
        assert!(policy.check_request(&req).await.is_err());
    }

    #[tokio::test]
    async fn test_judge_without_model_fails_closed() {
        let policy = EgressPolicy::new(
            &config(EgressMode::Judge, &["api.anthropic.com"], &[]),
            EgressAuditor::log_only(),
        );
        // Allow-listed host still passes without consulting the judge
        let listed = request("api.anthropic.com", "https://api.anthropic.com/");
        assert!(policy.check_request(&listed).await.is_ok());
        // Unmatched host with no judge model attached is denied
        let unmatched = request("unknown.example.com", "https://unknown.example.com/");
        let err = policy.check_request(&unmatched).await.unwrap_err();
        assert!(err.to_string().contains("no judge model"));
    }

    fn judge_policy(mock: crate::llm::MockLlmProvider) -> EgressPolicy {
        EgressPolicy::new(
            &config(EgressMode::Judge, &[], &[]),
            EgressAuditor::log_only(),
        )
        .with_judge(Arc::new(mock))
    }

    #[tokio::test]
    async fn test_judge_allow_verdict_allows() {
        let policy = judge_policy(crate::llm::MockLlmProvider::success(
            r#"{"allow": true, "reason": "well-known public API"}"#,
        ));
        let req = request("api.example.com", "https://api.example.com/v1");
        assert!(policy.check_request(&req).await.is_ok());
    }

    #[tokio::test]
    async fn test_judge_deny_verdict_denies() {
        let policy = judge_policy(crate::llm::MockLlmProvider::success(
            r#"{"allow": false, "reason": "unfamiliar host"}"#,
        ));
        let req = request("evil.example.com", "https://evil.example.com/");
        let err = policy.check_request(&req).await.unwrap_err();
        assert!(err.to_string().contains("unfamiliar host"));
    }

    #[tokio::test]
    async fn test_judge_code_fenced_verdict_parses() {
        let policy = judge_policy(crate::llm::MockLlmProvider::success(
            "```json\n{\"allow\": true, \"reason\": \"ok\"}\n```",
        ));
        let req = request("api.example.com", "https://api.example.com/");
        assert!(policy.check_request(&req).await.is_ok());
    }

    #[tokio::test]
    async fn test_judge_unparseable_verdict_fails_closed() {
        let policy = judge_policy(crate::llm::MockLlmProvider::success(
            "Sure! I think this request is probably fine.",
        ));
        let req = request("api.example.com", "https://api.example.com/");
        let err = policy.check_request(&req).await.unwrap_err();
        assert!(err.to_string().contains("unparseable"));
    }

    #[tokio::test]
    async fn test_judge_llm_error_fails_closed() {
        let policy = judge_policy(crate::llm::MockLlmProvider::failing());
        let req = request("api.example.com", "https://api.example.com/");
        let err = policy.check_request(&req).await.unwrap_err();
        assert!(err.to_string().contains("judge LLM call failed"));
    }

    #[tokio::test]
    async fn test_judge_timeout_fails_closed() {
        // Judge answers "allow" but only after 5s; latency budget is 50ms.
        let mut policy = judge_policy(crate::llm::MockLlmProvider::slow(
            r#"{"allow": true, "reason": "too late"}"#,
            5_000,
        ));
        policy.judge_max_latency_ms = 50;
        let req = request("api.example.com", "https://api.example.com/");
        let err = policy.check_request(&req).await.unwrap_err();
        assert!(err.to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn test_judge_explicit_rules_bypass_judge() {
        // Explicit deny is honored without consulting the (allow-happy) judge
        let policy = EgressPolicy::new(
            &config(EgressMode::Judge, &[], &["evil.example.com"]),
            EgressAuditor::log_only(),
        )
        .with_judge(Arc::new(crate::llm::MockLlmProvider::success(
            r#"{"allow": true, "reason": "should never be asked"}"#,
        )));
        let req = request("evil.example.com", "https://evil.example.com/");
        let err = policy.check_request(&req).await.unwrap_err();
        assert!(err.to_string().contains("deny rule"));
    }

    #[test]
    fn test_parse_judge_verdict_shapes() {
        assert!(parse_judge_verdict(r#"{"allow": true, "reason": "x"}"#).is_some());
        assert!(parse_judge_verdict(r#"{"allow": false}"#).is_some());
        assert!(parse_judge_verdict("not json").is_none());
        assert!(parse_judge_verdict(r#"{"reason": "missing allow"}"#).is_none());
    }

    #[test]
    fn test_judge_prompt_redacts_header_values() {
        let headers = vec![(
            "Authorization".to_string(),
            "Bearer supersecret".to_string(),
        )];
        let req = EgressRequest {
            tool: "http",
            method: "POST",
            url: "https://api.example.com/v1",
            host: "api.example.com",
            path: "/v1",
            headers: &headers,
            body: Some(b"payload"),
        };
        let prompt = judge_prompt(&req);
        assert!(prompt.contains("Authorization"));
        assert!(!prompt.contains("supersecret"));
        assert!(prompt.contains("7 bytes"));
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
