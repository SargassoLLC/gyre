//! CLI commands for axiom sharing: `gyre axiom share` and `gyre axiom pull`.

use clap::Subcommand;
use std::path::{Path, PathBuf};

use crate::cognitive::axiom_culture::AxiomCulture;
use crate::template::axiom_sharing::{AnonymizedAxiom, export_shareable, import_community};

#[derive(Subcommand, Debug, Clone)]
pub enum AxiomCommand {
    /// Share universal axioms with the community registry (opt-in).
    Share {
        /// Path to the agent's axiom database (axioms.db).
        #[arg(long, default_value = "axioms/axioms.db")]
        db: PathBuf,

        /// Skip the interactive confirmation prompt.
        #[arg(long)]
        yes: bool,
    },

    /// Pull community axioms from the registry and merge into local DB.
    Pull {
        /// Path to the agent's axiom database (axioms.db).
        #[arg(long, default_value = "axioms/axioms.db")]
        db: PathBuf,
    },
}

pub fn run_axiom_command(cmd: AxiomCommand) -> anyhow::Result<()> {
    match cmd {
        AxiomCommand::Share { db, yes } => run_axiom_share(&db, yes),
        AxiomCommand::Pull { db } => run_axiom_pull(&db),
    }
}

fn run_axiom_share(db_path: &Path, skip_confirm: bool) -> anyhow::Result<()> {
    if !db_path.exists() {
        anyhow::bail!("Axiom database not found at {}", db_path.display());
    }

    let culture = AxiomCulture::new(db_path)
        .map_err(|e| anyhow::anyhow!("Failed to open axiom DB: {}", e))?;

    let exported = export_shareable(&culture)
        .map_err(|e| anyhow::anyhow!("Failed to export axioms: {}", e))?;

    if exported.is_empty() {
        println!("No universal axioms found to share.");
        return Ok(());
    }

    println!(
        "Found {} universal axiom(s) eligible for sharing:",
        exported.len()
    );
    for (i, axiom) in exported.iter().enumerate() {
        println!(
            "  {}. [{}] {}",
            i + 1,
            axiom.domain,
            truncate(&axiom.statement, 80)
        );
    }
    println!();

    if !skip_confirm {
        println!("Share these axioms with the community? (y/N)");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }

    // In a full implementation this would upload to registry.gyre.ai.
    // For now, write to a local shareable JSON file as a staging step.
    let share_path = db_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("shared_axioms.json");

    let json = serde_json::to_string_pretty(&exported)?;
    std::fs::write(&share_path, &json)?;

    println!(
        "Exported {} axiom(s) to {}",
        exported.len(),
        share_path.display()
    );
    println!("(Registry upload will be available when registry.gyre.ai is live)");

    Ok(())
}

fn run_axiom_pull(db_path: &Path) -> anyhow::Result<()> {
    let culture = AxiomCulture::new(db_path)
        .map_err(|e| anyhow::anyhow!("Failed to open axiom DB: {}", e))?;

    // In a full implementation this would download from registry.gyre.ai.
    // For now, read from a local community axioms file if present.
    let community_path = db_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("community_axioms.json");

    if !community_path.exists() {
        println!(
            "No community axioms file found at {}",
            community_path.display()
        );
        println!("(Community axiom pool will be available when registry.gyre.ai is live)");
        return Ok(());
    }

    let json = std::fs::read_to_string(&community_path)?;
    let community_axioms: Vec<AnonymizedAxiom> = serde_json::from_str(&json)?;

    let imported = import_community(&community_axioms, &culture)
        .map_err(|e| anyhow::anyhow!("Failed to import axioms: {}", e))?;

    println!(
        "Imported {} new axiom(s) from community pool ({} total in pool, {} deduplicated)",
        imported,
        community_axioms.len(),
        community_axioms.len() - imported
    );

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
