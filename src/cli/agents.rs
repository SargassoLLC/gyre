//! `gyre agents` subcommand — list all agent boxes in a directory.

use std::path::PathBuf;

use crate::cognitive::hermit_box::HermitBox;

/// List all agent boxes found under `base_dir`.
pub fn run_agents(base_dir: &PathBuf) -> Result<(), String> {
    let canonical = base_dir
        .canonicalize()
        .map_err(|e| format!("cannot canonicalize base_dir: {e}"))?;

    let entries =
        std::fs::read_dir(&canonical).map_err(|e| format!("cannot read base_dir: {e}"))?;

    let mut found = false;

    println!("{:<20} {:>8}   {}", "AGENT_ID", "MEMORIES", "SOUL");
    println!("{}", "-".repeat(70));

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip symlinks to prevent following links that expose unintended paths.
        let is_symlink = entry.file_type().map(|ft| ft.is_symlink()).unwrap_or(false);
        if is_symlink {
            continue;
        }

        if !name_str.ends_with("_box") || !entry.path().is_dir() {
            continue;
        }

        let agent_id = &name_str[..name_str.len() - 4]; // strip "_box"
        match HermitBox::open(&canonical, agent_id) {
            Ok(hermit_box) => {
                found = true;

                let memory_count = hermit_box
                    .memory_stream
                    .lock()
                    .ok()
                    .and_then(|ms| ms.recent(1000).ok())
                    .map(|v| v.len())
                    .unwrap_or(0);

                let soul = hermit_box.read_soul();
                let soul_preview: String =
                    soul.lines().next().unwrap_or("").chars().take(60).collect();

                println!("{:<20} {:>8}   {}", agent_id, memory_count, soul_preview);
            }
            Err(_) => {
                // Skip boxes that fail to open
            }
        }
    }

    if !found {
        println!("No agent boxes found in {}", canonical.display());
    }

    Ok(())
}
