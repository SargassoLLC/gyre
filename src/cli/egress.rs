//! Egress audit log CLI command.
//!
//! Surfaces the `egress_events` table produced by `EgressPolicy`.
//! Every outbound network request from native tools is audited there,
//! regardless of mode; this command lets you inspect the log without
//! opening the web gateway.

use clap::Subcommand;

use crate::db::Database;
use crate::safety::{EgressDecision, EgressEvent};

const DEFAULT_LIMIT: i64 = 50;

#[derive(Subcommand, Debug, Clone)]
pub enum EgressCommand {
    /// Show the egress audit log (most recent events first).
    Log {
        /// Maximum number of events to show.
        #[arg(short, long, default_value_t = DEFAULT_LIMIT)]
        limit: i64,

        /// Output raw JSON instead of a formatted table.
        #[arg(long)]
        json: bool,
    },
}

/// Run an egress subcommand using a live database connection.
pub async fn run_egress_command(
    cmd: EgressCommand,
    db: std::sync::Arc<dyn Database>,
) -> anyhow::Result<()> {
    match cmd {
        EgressCommand::Log { limit, json } => log_command(db, limit, json).await,
    }
}

async fn log_command(
    db: std::sync::Arc<dyn Database>,
    limit: i64,
    json: bool,
) -> anyhow::Result<()> {
    let limit = limit.clamp(1, 1000);
    let events = db
        .list_egress_events(limit)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read egress events: {}", e))?;

    if json {
        print_json(&events)?;
    } else {
        print_table(&events);
    }

    Ok(())
}

// --- JSON output ---

fn print_json(events: &[EgressEvent]) -> anyhow::Result<()> {
    // Serialize each event as a JSON array.
    let values: Vec<serde_json::Value> = events.iter().map(event_to_json).collect();
    println!("{}", serde_json::to_string_pretty(&values)?);
    Ok(())
}

fn event_to_json(e: &EgressEvent) -> serde_json::Value {
    serde_json::json!({
        "id": e.id,
        "ts": e.ts.to_rfc3339(),
        "tool": e.tool,
        "method": e.method,
        "host": e.host,
        "path": e.path,
        "decision": e.decision.as_str(),
        "mode": e.mode,
        "reason": e.reason,
        "leak_verdict": e.leak_verdict,
    })
}

// --- Table output ---

/// Print events as a human-readable table.
///
/// Columns: timestamp, tool, method, host+path, decision, mode, reason.
/// Denied events are prefixed with a visual marker so they stand out.
fn print_table(events: &[EgressEvent]) {
    if events.is_empty() {
        println!("No egress events found.");
        println!();
        println!(
            "Egress auditing is on by default in observe mode. Events appear here after the\n\
             first outbound request from a native tool (http, built tools, or WASM allowlist\n\
             decisions). Start the agent and run a tool that makes a network call to populate\n\
             the log."
        );
        return;
    }

    // Column widths (fixed, values are truncated to fit)
    const W_TS: usize = 19; // "2026-07-14 10:23:45"
    const W_TOOL: usize = 14;
    const W_METHOD: usize = 6;
    const W_DEST: usize = 38; // host + path truncated
    const W_DECISION: usize = 7;
    const W_MODE: usize = 14;

    let header = format!(
        "{:<W_TS$}  {:<W_TOOL$}  {:<W_METHOD$}  {:<W_DEST$}  {:<W_DECISION$}  {:<W_MODE$}  {}",
        "Timestamp",
        "Tool",
        "Method",
        "Destination",
        "Decision",
        "Mode",
        "Reason",
        W_TS = W_TS,
        W_TOOL = W_TOOL,
        W_METHOD = W_METHOD,
        W_DEST = W_DEST,
        W_DECISION = W_DECISION,
        W_MODE = W_MODE,
    );
    let separator = "-".repeat(header.len().min(120));

    println!("{}", header);
    println!("{}", separator);

    for event in events {
        let ts = event.ts.format("%Y-%m-%d %H:%M:%S").to_string();
        let dest = {
            let full = format!("{}{}", event.host, event.path);
            truncate(&full, W_DEST)
        };
        let decision_str = match event.decision {
            EgressDecision::Denied => "[DENY]",
            EgressDecision::Allowed => "allow ",
        };
        let reason = if event.reason.is_empty() {
            event.leak_verdict.as_str()
        } else {
            event.reason.as_str()
        };

        println!(
            "{:<W_TS$}  {:<W_TOOL$}  {:<W_METHOD$}  {:<W_DEST$}  {:<W_DECISION$}  {:<W_MODE$}  {}",
            truncate(&ts, W_TS),
            truncate(&event.tool, W_TOOL),
            truncate(&event.method, W_METHOD),
            dest,
            decision_str,
            truncate(&event.mode, W_MODE),
            reason,
            W_TS = W_TS,
            W_TOOL = W_TOOL,
            W_METHOD = W_METHOD,
            W_DEST = W_DEST,
            W_DECISION = W_DECISION,
            W_MODE = W_MODE,
        );
    }

    println!();
    println!("({} event(s) shown)", events.len());
}

/// Truncate `s` to at most `max` bytes (ASCII-safe).
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else if max > 1 {
        format!("{}…", &s[..max.saturating_sub(1)])
    } else {
        s[..max].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_event(tool: &str, method: &str, host: &str, decision: EgressDecision) -> EgressEvent {
        EgressEvent {
            id: Uuid::new_v4(),
            ts: Utc::now(),
            tool: tool.to_string(),
            method: method.to_string(),
            host: host.to_string(),
            path: "/v1/test".to_string(),
            decision,
            mode: "observe".to_string(),
            reason: String::new(),
            leak_verdict: "clean".to_string(),
        }
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_exact() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_long() {
        let result = truncate("hello world", 8);
        assert!(result.len() <= 10, "truncated string should be short");
        assert!(result.starts_with("hello w"));
    }

    #[test]
    fn test_truncate_empty() {
        assert_eq!(truncate("", 5), "");
    }

    #[test]
    fn test_print_table_empty_no_panic() {
        // Calling print_table with an empty slice should not panic.
        print_table(&[]);
    }

    #[test]
    fn test_print_table_events_no_panic() {
        let events = vec![
            make_event("http", "GET", "api.anthropic.com", EgressDecision::Allowed),
            make_event("http", "POST", "evil.example.com", EgressDecision::Denied),
        ];
        print_table(&events);
    }

    #[test]
    fn test_event_to_json_fields() {
        let event = make_event("http", "GET", "api.example.com", EgressDecision::Allowed);
        let v = event_to_json(&event);
        assert_eq!(v["tool"], "http");
        assert_eq!(v["method"], "GET");
        assert_eq!(v["host"], "api.example.com");
        assert_eq!(v["decision"], "allowed");
        assert_eq!(v["mode"], "observe");
    }

    #[test]
    fn test_event_to_json_denied() {
        let event = make_event("http", "POST", "evil.example.com", EgressDecision::Denied);
        let v = event_to_json(&event);
        assert_eq!(v["decision"], "denied");
    }

    #[test]
    fn test_print_json_no_panic() {
        let events = vec![make_event(
            "http",
            "GET",
            "api.example.com",
            EgressDecision::Allowed,
        )];
        print_json(&events).expect("print_json should not fail");
    }

    #[test]
    fn test_print_table_long_dest_truncates() {
        let mut event = make_event(
            "my-tool",
            "DELETE",
            "very-long-subdomain.api.example.com",
            EgressDecision::Denied,
        );
        event.path = "/api/v1/some/very/long/path/that/exceeds/column/width".to_string();
        // Should not panic when destination exceeds column width
        print_table(&[event]);
    }
}
