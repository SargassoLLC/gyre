//! SetupUi — abstraction over dialoguer for the setup wizard.
//!
//! Provides a consistent interface for interactive prompts with:
//! - Themed output (headers, steps, success/error messages)
//! - Non-TTY fallback (headless mode via JSON file)
//! - All prompt types: confirm, input, secret, select, multi-select, fuzzy-select

use std::io;
use std::path::PathBuf;

use console::{Style, Term};
use dialoguer::{Confirm, FuzzySelect, Input, MultiSelect, Password, Select};
use secrecy::SecretString;

/// Convert a `dialoguer::Error` into `io::Error`.
fn map_dialoguer_err(e: dialoguer::Error) -> io::Error {
    match e {
        dialoguer::Error::IO(io_err) => io_err,
    }
}

/// UI abstraction for the setup wizard.
///
/// In interactive mode, wraps `dialoguer` prompts with consistent theming.
/// In headless mode, reads answers from a JSON file (for CI/automation).
pub struct SetupUi {
    term: Term,
    headless: Option<HeadlessAnswers>,
    heading_style: Style,
    success_style: Style,
    error_style: Style,
    info_style: Style,
    dim_style: Style,
}

/// Pre-configured answers for headless (non-interactive) mode.
#[derive(Debug, serde::Deserialize)]
pub struct HeadlessAnswers {
    answers: std::collections::HashMap<String, serde_json::Value>,
}

impl SetupUi {
    /// Create a new interactive UI.
    pub fn new() -> Self {
        Self {
            term: Term::stderr(),
            headless: None,
            heading_style: Style::new().bold().cyan(),
            success_style: Style::new().green(),
            error_style: Style::new().red(),
            info_style: Style::new().dim(),
            dim_style: Style::new().dim(),
        }
    }

    /// Create a headless UI that reads from a JSON answers file.
    pub fn headless(path: &PathBuf) -> io::Result<Self> {
        let data = std::fs::read_to_string(path)?;
        let answers: HeadlessAnswers =
            serde_json::from_str(&data).map_err(|e| io::Error::other(e.to_string()))?;
        Ok(Self {
            term: Term::stderr(),
            headless: Some(answers),
            heading_style: Style::new().bold().cyan(),
            success_style: Style::new().green(),
            error_style: Style::new().red(),
            info_style: Style::new().dim(),
            dim_style: Style::new().dim(),
        })
    }

    /// Whether running in headless mode.
    pub fn is_headless(&self) -> bool {
        self.headless.is_some()
    }

    // ── Display helpers ──────────────────────────────────────────

    /// Print a major section header with decorative box.
    pub fn header(&self, text: &str) {
        let width = text.len() + 4;
        let border = "─".repeat(width);
        let _ = self.term.write_line("");
        let _ = self.term.write_line(
            &self
                .heading_style
                .apply_to(format!("╭{border}╮"))
                .to_string(),
        );
        let _ = self.term.write_line(
            &self
                .heading_style
                .apply_to(format!("│  {text}  │"))
                .to_string(),
        );
        let _ = self.term.write_line(
            &self
                .heading_style
                .apply_to(format!("╰{border}╯"))
                .to_string(),
        );
        let _ = self.term.write_line("");
    }

    /// Print a step indicator (e.g., "Stage 3/10: Database Setup").
    pub fn step(&self, current: usize, total: usize, name: &str) {
        let _ = self.term.write_line(&format!(
            "{}",
            self.heading_style
                .apply_to(format!("Stage {current}/{total}: {name}"))
        ));
        let _ = self
            .term
            .write_line(&self.dim_style.apply_to("━".repeat(40)).to_string());
        let _ = self.term.write_line("");
    }

    /// Print a success message with checkmark.
    pub fn success(&self, message: &str) {
        let _ = self
            .term
            .write_line(&format!("{} {}", self.success_style.apply_to("✓"), message));
    }

    /// Print an error message with X mark.
    pub fn error(&self, message: &str) {
        let _ = self
            .term
            .write_line(&format!("{} {}", self.error_style.apply_to("✗"), message));
    }

    /// Print an informational message.
    pub fn info(&self, message: &str) {
        let _ = self
            .term
            .write_line(&format!("  {}", self.info_style.apply_to(message)));
    }

    /// Print a blank line.
    pub fn blank(&self) {
        let _ = self.term.write_line("");
    }

    // ── Prompt helpers ───────────────────────────────────────────

    /// Yes/no confirmation prompt.
    pub fn confirm(&self, prompt: &str, default: bool) -> io::Result<bool> {
        if let Some(ref h) = self.headless {
            return Ok(h
                .answers
                .get(prompt)
                .and_then(|v| v.as_bool())
                .unwrap_or(default));
        }

        Confirm::new()
            .with_prompt(prompt)
            .default(default)
            .interact()
            .map_err(map_dialoguer_err)
    }

    /// Text input prompt.
    pub fn input(&self, prompt: &str) -> io::Result<String> {
        if let Some(ref h) = self.headless {
            return Ok(h
                .answers
                .get(prompt)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string());
        }

        Input::new()
            .with_prompt(prompt)
            .interact_text()
            .map_err(map_dialoguer_err)
    }

    /// Text input with a default value.
    pub fn input_with_default(&self, prompt: &str, default: &str) -> io::Result<String> {
        if let Some(ref h) = self.headless {
            return Ok(h
                .answers
                .get(prompt)
                .and_then(|v| v.as_str())
                .unwrap_or(default)
                .to_string());
        }

        Input::new()
            .with_prompt(prompt)
            .default(default.to_string())
            .interact_text()
            .map_err(map_dialoguer_err)
    }

    /// Optional text input (empty returns None).
    pub fn optional_input(&self, prompt: &str) -> io::Result<Option<String>> {
        let val = self.input_with_default(prompt, "")?;
        if val.is_empty() {
            Ok(None)
        } else {
            Ok(Some(val))
        }
    }

    /// Secret/password input (hidden characters).
    pub fn secret_input(&self, prompt: &str) -> io::Result<SecretString> {
        if let Some(ref h) = self.headless {
            return Ok(SecretString::from(
                h.answers
                    .get(prompt)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            ));
        }

        let val = Password::new()
            .with_prompt(prompt)
            .interact()
            .map_err(map_dialoguer_err)?;
        Ok(SecretString::from(val))
    }

    /// Single-select menu (arrow keys to navigate, enter to confirm).
    ///
    /// Returns the 0-based index of the selected item.
    pub fn select_one(&self, prompt: &str, options: &[&str]) -> io::Result<usize> {
        if let Some(ref h) = self.headless {
            return Ok(h.answers.get(prompt).and_then(|v| v.as_u64()).unwrap_or(0) as usize);
        }

        Select::new()
            .with_prompt(prompt)
            .items(options)
            .default(0)
            .interact()
            .map_err(map_dialoguer_err)
    }

    /// Multi-select menu (space to toggle, enter to confirm).
    ///
    /// `defaults` indicates which items are pre-selected.
    /// Returns indices of selected items.
    pub fn select_many(
        &self,
        prompt: &str,
        options: &[&str],
        defaults: &[bool],
    ) -> io::Result<Vec<usize>> {
        if let Some(ref h) = self.headless {
            if let Some(indices) = h.answers.get(prompt).and_then(|v| v.as_array()) {
                return Ok(indices
                    .iter()
                    .filter_map(|v| v.as_u64().map(|n| n as usize))
                    .collect());
            }
            // Return pre-selected items by default
            return Ok(defaults
                .iter()
                .enumerate()
                .filter_map(|(i, &selected)| if selected { Some(i) } else { None })
                .collect());
        }

        MultiSelect::new()
            .with_prompt(prompt)
            .items(options)
            .defaults(defaults)
            .interact()
            .map_err(map_dialoguer_err)
    }

    /// Fuzzy-select menu (type to filter, arrow keys to navigate).
    ///
    /// Returns the 0-based index of the selected item.
    pub fn fuzzy_select(&self, prompt: &str, options: &[String]) -> io::Result<usize> {
        if let Some(ref h) = self.headless {
            return Ok(h.answers.get(prompt).and_then(|v| v.as_u64()).unwrap_or(0) as usize);
        }

        FuzzySelect::new()
            .with_prompt(prompt)
            .items(options)
            .default(0)
            .interact()
            .map_err(map_dialoguer_err)
    }
}

impl Default for SetupUi {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_returns_defaults() {
        let json = r#"{"answers":{"Enable?": true, "Name": "test"}}"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("answers.json");
        std::fs::write(&path, json).unwrap();

        let ui = SetupUi::headless(&path).unwrap();
        assert!(ui.is_headless());
        assert!(ui.confirm("Enable?", false).unwrap());
        assert_eq!(ui.input("Name").unwrap(), "test");
        assert_eq!(ui.input("Missing").unwrap(), "");
    }
}
