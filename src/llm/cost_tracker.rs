//! Per-model cost tracking with `pricing.toml` support.
//!
//! Records token usage after each LLM completion and provides cost reports
//! grouped by model, time period, and session.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Datelike, Local, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::llm::costs;

// ── Pricing TOML schema ─────────────────────────────────────────────────────

/// Root structure of `pricing.toml`.
#[derive(Debug, Deserialize, Serialize)]
pub struct PricingConfig {
    #[serde(default)]
    pub providers: HashMap<String, ProviderPricing>,
}

/// Pricing for a single provider (e.g., "anthropic", "openai").
#[derive(Debug, Deserialize, Serialize)]
pub struct ProviderPricing {
    #[serde(default)]
    pub models: Vec<ModelPricing>,
}

/// Cost per million tokens for a model prefix.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ModelPricing {
    /// Model name prefix to match (e.g., "claude-sonnet-4").
    pub prefix: String,
    /// Input cost per million tokens (USD).
    pub input_per_m: f64,
    /// Output cost per million tokens (USD).
    pub output_per_m: f64,
}

impl PricingConfig {
    /// Look up per-token costs for a model by checking prefix matches.
    /// Returns `(input_cost_per_token, output_cost_per_token)`.
    pub fn lookup(&self, model_id: &str) -> Option<(Decimal, Decimal)> {
        let id = model_id
            .rsplit_once('/')
            .map(|(_, name)| name)
            .unwrap_or(model_id);

        for provider in self.providers.values() {
            for model in &provider.models {
                if id.starts_with(&model.prefix) {
                    let input =
                        Decimal::try_from(model.input_per_m / 1_000_000.0).unwrap_or(Decimal::ZERO);
                    let output = Decimal::try_from(model.output_per_m / 1_000_000.0)
                        .unwrap_or(Decimal::ZERO);
                    return Some((input, output));
                }
            }
        }
        None
    }
}

/// Default pricing table written to `~/.gyre/pricing.toml` on first use.
pub fn default_pricing_toml() -> &'static str {
    r#"# Gyre model pricing configuration
# Costs are in USD per million tokens.
# Model matching uses prefix: "claude-sonnet-4" matches "claude-sonnet-4-20250514" etc.

[providers.anthropic]

[[providers.anthropic.models]]
prefix = "claude-sonnet-4"
input_per_m = 3.0
output_per_m = 15.0

[[providers.anthropic.models]]
prefix = "claude-opus-4"
input_per_m = 15.0
output_per_m = 75.0

[[providers.anthropic.models]]
prefix = "claude-3-5-sonnet"
input_per_m = 3.0
output_per_m = 15.0

[[providers.anthropic.models]]
prefix = "claude-3-5-haiku"
input_per_m = 0.8
output_per_m = 4.0

[[providers.anthropic.models]]
prefix = "claude-3-haiku"
input_per_m = 0.25
output_per_m = 1.25

[[providers.anthropic.models]]
prefix = "claude-3-opus"
input_per_m = 15.0
output_per_m = 75.0

[providers.openai]

[[providers.openai.models]]
prefix = "gpt-4o-mini"
input_per_m = 0.15
output_per_m = 0.6

[[providers.openai.models]]
prefix = "gpt-4o"
input_per_m = 2.5
output_per_m = 10.0

[[providers.openai.models]]
prefix = "gpt-4-turbo"
input_per_m = 10.0
output_per_m = 30.0

[[providers.openai.models]]
prefix = "o1-mini"
input_per_m = 3.0
output_per_m = 12.0

[[providers.openai.models]]
prefix = "o1"
input_per_m = 15.0
output_per_m = 60.0

[[providers.openai.models]]
prefix = "o3-mini"
input_per_m = 1.1
output_per_m = 4.4

[providers.google]

[[providers.google.models]]
prefix = "gemini-2.5-flash"
input_per_m = 0.15
output_per_m = 0.6

[[providers.google.models]]
prefix = "gemini-2.5-pro"
input_per_m = 1.25
output_per_m = 10.0

[[providers.google.models]]
prefix = "gemini-2.0-flash"
input_per_m = 0.1
output_per_m = 0.4
"#
}

/// Load pricing config from `~/.gyre/pricing.toml`, creating it with defaults if missing.
pub fn load_pricing_config() -> anyhow::Result<PricingConfig> {
    let path = pricing_toml_path()?;

    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, default_pricing_toml())?;
    }

    let content = std::fs::read_to_string(&path)?;
    let config: PricingConfig = toml::from_str(&content)?;
    Ok(config)
}

/// Path to the pricing config file.
pub fn pricing_toml_path() -> anyhow::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    Ok(home.join(".gyre").join("pricing.toml"))
}

// ── Usage record ─────────────────────────────────────────────────────────────

/// A single recorded LLM usage event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost: Decimal,
    pub timestamp: DateTime<Utc>,
    /// Optional session identifier for per-session grouping.
    pub session_id: Option<String>,
}

// ── CostTracker ──────────────────────────────────────────────────────────────

/// Accumulates token usage and costs across LLM calls.
///
/// Thread-safe via interior `Mutex`. Records are also appended to a JSONL file
/// at `~/.gyre/usage.jsonl` for persistence across sessions.
pub struct CostTracker {
    records: Mutex<Vec<UsageRecord>>,
    pricing: Option<PricingConfig>,
    log_path: Option<PathBuf>,
}

impl CostTracker {
    /// Create a new tracker, loading pricing from `~/.gyre/pricing.toml`.
    pub fn new() -> Self {
        let pricing = load_pricing_config().ok();
        let log_path = dirs::home_dir().map(|h| h.join(".gyre").join("usage.jsonl"));

        Self {
            records: Mutex::new(Vec::new()),
            pricing,
            log_path,
        }
    }

    /// Look up per-token cost for a model. Checks pricing.toml first, then
    /// the built-in `costs::model_cost` table, then falls back to defaults.
    pub fn cost_per_token(&self, model_id: &str) -> (Decimal, Decimal) {
        // 1. pricing.toml
        if let Some(ref pricing) = self.pricing {
            if let Some(costs) = pricing.lookup(model_id) {
                return costs;
            }
        }
        // 2. Built-in table
        costs::model_cost(model_id).unwrap_or_else(costs::default_cost)
    }

    /// Record a completion's token usage.
    pub fn record(
        &self,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
        session_id: Option<&str>,
    ) {
        let (input_cost, output_cost) = self.cost_per_token(model);
        let cost =
            input_cost * Decimal::from(input_tokens) + output_cost * Decimal::from(output_tokens);

        let record = UsageRecord {
            model: model.to_string(),
            input_tokens,
            output_tokens,
            cost,
            timestamp: Utc::now(),
            session_id: session_id.map(|s| s.to_string()),
        };

        // Append to JSONL log file (best-effort, don't fail on IO errors)
        if let Some(ref path) = self.log_path {
            if let Ok(line) = serde_json::to_string(&record) {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(f, "{}", line);
                }
            }
        }

        if let Ok(mut records) = self.records.lock() {
            records.push(record);
        }
    }

    /// Load all historical records from the JSONL log file.
    pub fn load_history() -> Vec<UsageRecord> {
        let path = match dirs::home_dir() {
            Some(h) => h.join(".gyre").join("usage.jsonl"),
            None => return Vec::new(),
        };

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        content
            .lines()
            .filter_map(|line| serde_json::from_str::<UsageRecord>(line).ok())
            .collect()
    }

    /// Get in-memory records for the current session.
    pub fn session_records(&self) -> Vec<UsageRecord> {
        self.records.lock().map(|r| r.clone()).unwrap_or_default()
    }
}

// ── Cost report formatting ───────────────────────────────────────────────────

/// Time period for filtering usage.
#[derive(Debug, Clone, Copy)]
pub enum Period {
    Today,
    ThisWeek,
    ThisMonth,
    AllTime,
}

impl std::fmt::Display for Period {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Period::Today => write!(f, "Today"),
            Period::ThisWeek => write!(f, "This Week"),
            Period::ThisMonth => write!(f, "This Month"),
            Period::AllTime => write!(f, "All Time"),
        }
    }
}

/// Filter records by time period.
fn filter_by_period(records: &[UsageRecord], period: Period) -> Vec<&UsageRecord> {
    let now = Local::now();
    let today = now.date_naive();

    records
        .iter()
        .filter(|r| {
            let record_date = r.timestamp.with_timezone(&Local).date_naive();
            match period {
                Period::Today => record_date == today,
                Period::ThisWeek => {
                    let week_start = today
                        - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
                    record_date >= week_start
                }
                Period::ThisMonth => {
                    record_date.year() == today.year() && record_date.month() == today.month()
                }
                Period::AllTime => true,
            }
        })
        .collect()
}

/// Model-level cost summary.
#[derive(Debug)]
struct ModelSummary {
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    total_cost: Decimal,
    call_count: u32,
}

/// Aggregate records by model.
fn aggregate_by_model(records: &[&UsageRecord]) -> Vec<ModelSummary> {
    let mut map: HashMap<String, ModelSummary> = HashMap::new();

    for r in records {
        let entry = map.entry(r.model.clone()).or_insert(ModelSummary {
            model: r.model.clone(),
            input_tokens: 0,
            output_tokens: 0,
            total_cost: Decimal::ZERO,
            call_count: 0,
        });
        entry.input_tokens += r.input_tokens as u64;
        entry.output_tokens += r.output_tokens as u64;
        entry.total_cost += r.cost;
        entry.call_count += 1;
    }

    let mut summaries: Vec<ModelSummary> = map.into_values().collect();
    summaries.sort_by(|a, b| b.total_cost.cmp(&a.total_cost));
    summaries
}

/// Format a full cost report for CLI display.
pub fn format_cost_report(records: &[UsageRecord]) -> String {
    let mut out = String::new();

    out.push_str("Gyre Usage Report\n");
    out.push_str(&"─".repeat(60));
    out.push('\n');

    if records.is_empty() {
        out.push_str("\n  No usage data recorded yet.\n");
        out.push_str("  Usage is tracked automatically during agent sessions.\n");
        return out;
    }

    for period in [
        Period::Today,
        Period::ThisWeek,
        Period::ThisMonth,
        Period::AllTime,
    ] {
        let filtered = filter_by_period(records, period);
        if filtered.is_empty() && !matches!(period, Period::AllTime) {
            continue;
        }

        out.push_str(&format!("\n  {} ({} calls)\n", period, filtered.len()));
        out.push_str(&format!("  {}\n", "─".repeat(50)));

        let summaries = aggregate_by_model(&filtered);
        let total_cost: Decimal = summaries.iter().map(|s| s.total_cost).sum();
        let total_input: u64 = summaries.iter().map(|s| s.input_tokens).sum();
        let total_output: u64 = summaries.iter().map(|s| s.output_tokens).sum();

        for s in &summaries {
            out.push_str(&format!(
                "    {:<40} {:>6} calls  ${:.4}\n",
                s.model, s.call_count, s.total_cost
            ));
            out.push_str(&format!(
                "      input: {:>12} tokens   output: {:>12} tokens\n",
                format_tokens(s.input_tokens),
                format_tokens(s.output_tokens),
            ));
        }

        out.push_str(&format!("  {}\n", "─".repeat(50)));
        out.push_str(&format!(
            "    Total: {} input + {} output = ${:.4}\n",
            format_tokens(total_input),
            format_tokens(total_output),
            total_cost,
        ));
    }

    // Session stats
    let mut sessions: HashMap<String, (Decimal, u32)> = HashMap::new();
    for r in records {
        let session = r.session_id.as_deref().unwrap_or("unknown");
        let entry = sessions
            .entry(session.to_string())
            .or_insert((Decimal::ZERO, 0));
        entry.0 += r.cost;
        entry.1 += 1;
    }

    if sessions.len() > 1 {
        let mut session_list: Vec<_> = sessions.into_iter().collect();
        session_list.sort_by(|a, b| b.1.0.cmp(&a.1.0));

        out.push_str(&format!("\n  Sessions ({} total)\n", session_list.len()));
        out.push_str(&format!("  {}\n", "─".repeat(50)));

        if let Some((id, (cost, calls))) = session_list.first() {
            out.push_str(&format!(
                "    Most expensive : {} (${:.4}, {} calls)\n",
                truncate_id(id),
                cost,
                calls
            ));
        }
        if let Some((id, (cost, calls))) = session_list.last() {
            out.push_str(&format!(
                "    Cheapest       : {} (${:.4}, {} calls)\n",
                truncate_id(id),
                cost,
                calls
            ));
        }
    }

    out.push('\n');
    out
}

/// Format token count with thousands separator.
fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Truncate a session ID for display.
fn truncate_id(id: &str) -> String {
    if id.len() > 16 {
        format!("{}…", &id[..15])
    } else {
        id.to_string()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_pricing_config_lookup() {
        let toml_str = r#"
[providers.anthropic]

[[providers.anthropic.models]]
prefix = "claude-sonnet-4"
input_per_m = 3.0
output_per_m = 15.0
"#;
        let config: PricingConfig = toml::from_str(toml_str).unwrap();
        let (input, output) = config.lookup("claude-sonnet-4-20250514").unwrap();
        assert!(input > Decimal::ZERO);
        assert!(output > input);
    }

    #[test]
    fn test_pricing_config_no_match() {
        let config = PricingConfig {
            providers: HashMap::new(),
        };
        assert!(config.lookup("some-unknown-model").is_none());
    }

    #[test]
    fn test_default_pricing_toml_parses() {
        let config: PricingConfig = toml::from_str(default_pricing_toml()).unwrap();
        assert!(!config.providers.is_empty());
        // Should have anthropic, openai, google
        assert!(config.providers.contains_key("anthropic"));
        assert!(config.providers.contains_key("openai"));
        assert!(config.providers.contains_key("google"));
    }

    #[test]
    fn test_cost_tracker_record() {
        let tracker = CostTracker {
            records: Mutex::new(Vec::new()),
            pricing: None,
            log_path: None, // Don't write to disk in tests
        };

        tracker.record("gpt-4o", 1000, 500, Some("test-session"));
        let records = tracker.session_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].model, "gpt-4o");
        assert_eq!(records[0].input_tokens, 1000);
        assert_eq!(records[0].output_tokens, 500);
        assert!(records[0].cost > Decimal::ZERO);
    }

    #[test]
    fn test_format_cost_report_empty() {
        let report = format_cost_report(&[]);
        assert!(report.contains("No usage data"));
    }

    #[test]
    fn test_format_cost_report_with_data() {
        let records = vec![UsageRecord {
            model: "gpt-4o".to_string(),
            input_tokens: 1000,
            output_tokens: 500,
            cost: dec!(0.0075),
            timestamp: Utc::now(),
            session_id: Some("sess-1".to_string()),
        }];
        let report = format_cost_report(&records);
        assert!(report.contains("gpt-4o"));
        assert!(report.contains("Today"));
    }

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_tokens(500), "500");
        assert_eq!(format_tokens(1500), "1.5K");
        assert_eq!(format_tokens(1_500_000), "1.5M");
    }

    #[test]
    fn test_truncate_id() {
        assert_eq!(truncate_id("short"), "short");
        let result = truncate_id("a-very-long-session-identifier-here");
        assert!(result.len() <= 20);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn test_period_display() {
        assert_eq!(format!("{}", Period::Today), "Today");
        assert_eq!(format!("{}", Period::AllTime), "All Time");
    }
}
