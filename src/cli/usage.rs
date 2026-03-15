//! `gyre usage` — display LLM cost and token usage reports.

use crate::llm::cost_tracker::{self, CostTracker, Period};

/// Options for the usage command.
#[derive(Debug, Clone)]
pub struct UsageOptions {
    /// Show only a specific time period.
    pub period: Option<Period>,
    /// Show pricing config path.
    pub show_config: bool,
}

/// Run `gyre usage`.
pub fn run_usage(opts: UsageOptions) -> anyhow::Result<()> {
    if opts.show_config {
        let path = cost_tracker::pricing_toml_path()?;
        println!("Pricing config: {}", path.display());
        if path.exists() {
            println!("  ✓ File exists");
        } else {
            println!("  ✗ File not found (will be created on first use)");
        }
        return Ok(());
    }

    let records = CostTracker::load_history();
    let report = cost_tracker::format_cost_report(&records);
    print!("{}", report);

    Ok(())
}
