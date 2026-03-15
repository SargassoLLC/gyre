//! Agent-to-Agent (A2A) protocol — simple delegation between Chiefs via ResearchQueue.
//!
//! Chiefs can send tasks to other Chiefs by pushing into the target agent's
//! research queue with a `[FROM:{agent}]` prefix and `source='a2a'`.
//!
//! Security: Both sender and receiver agent IDs are validated to exist as agent
//! boxes and to contain only safe characters (alphanumeric, hyphens, underscores).

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::cognitive::curiosity::{ResearchQueue, ResearchTask};
use crate::cognitive::hermit_box::HermitBox;

/// Validate that an agent identifier contains only safe characters.
/// Alphanumeric, hyphens, and underscores only — prevents format injection
/// in the `[FROM:{agent}]` prefix and path traversal in box lookups.
fn validate_agent_id(agent_id: &str) -> Result<(), String> {
    if agent_id.is_empty() {
        return Err("agent_id must not be empty".to_string());
    }
    if agent_id.len() > 64 {
        return Err(format!(
            "agent_id too long ({} chars, max 64)",
            agent_id.len()
        ));
    }
    if !agent_id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "agent_id '{}' contains invalid characters (alphanumeric, hyphens, underscores only)",
            agent_id
        ));
    }
    Ok(())
}

/// A message from one agent to another, requesting work.
pub struct A2AMessage {
    pub from_agent: String,
    pub to_agent: String,
    pub task: String,
    pub priority: f32,
    pub created_at: DateTime<Utc>,
}

/// Routes A2A messages between agent boxes in a shared base directory.
pub struct A2ARouter {
    pub base_dir: PathBuf,
}

impl A2ARouter {
    /// Create a new router that looks for agent boxes under `base_dir`.
    pub fn new(base_dir: &Path) -> Self {
        Self {
            base_dir: base_dir.to_path_buf(),
        }
    }

    /// Validate that a sender agent's box directory exists.
    ///
    /// Prevents spoofing: `from_agent` must correspond to an existing agent box
    /// so agents cannot impersonate each other by setting arbitrary sender IDs.
    pub fn validate_sender(&self, from_agent: &str) -> Result<(), String> {
        validate_agent_id(from_agent)?;
        let canonical_base = self
            .base_dir
            .canonicalize()
            .map_err(|e| format!("cannot canonicalize base_dir: {e}"))?;
        let sender_box = canonical_base.join(format!("{}_box", from_agent));
        if !sender_box.is_dir() {
            return Err(format!(
                "Sender agent '{}' not found (no box at {})",
                from_agent,
                sender_box.display()
            ));
        }
        Ok(())
    }

    /// Send a task to another agent by pushing into their ResearchQueue.
    ///
    /// Validates both sender and target agent IDs for safe characters and
    /// existing box directories (won't create a new agent or allow spoofing),
    /// then opens the queue and pushes the task with an
    /// `[FROM:{from_agent}]` prefix and `source='a2a'`.
    pub fn send(&self, msg: &A2AMessage) -> Result<(), String> {
        // Validate agent IDs: safe characters only (prevents format injection)
        validate_agent_id(&msg.from_agent)?;
        validate_agent_id(&msg.to_agent)?;

        // Validate sender exists (anti-spoofing)
        self.validate_sender(&msg.from_agent)?;

        // Check that the target agent box already exists (don't auto-create)
        let canonical_base = self
            .base_dir
            .canonicalize()
            .map_err(|e| format!("cannot canonicalize base_dir: {e}"))?;
        let expected_box = canonical_base.join(format!("{}_box", msg.to_agent));
        if !expected_box.is_dir() {
            return Err(format!(
                "Agent '{}' not found (no box at {})",
                msg.to_agent,
                expected_box.display()
            ));
        }

        let target_box = HermitBox::open(&self.base_dir, &msg.to_agent)?;

        // Open the target agent's research queue
        let queue = ResearchQueue::open_for_hermit_box(&target_box)
            .map_err(|e| format!("Failed to open queue for '{}': {e}", msg.to_agent))?;

        // Format the task with sender attribution
        let formatted_task = format!("[FROM:{}] {}", msg.from_agent, msg.task);

        queue
            .push(&formatted_task, msg.priority, "a2a")
            .map_err(|e| format!("Failed to push A2A task: {e}"))?;

        Ok(())
    }

    /// Peek at pending A2A messages for a given agent.
    ///
    /// Returns all pending tasks in the agent's queue that have `source='a2a'`.
    pub fn pending_messages(&self, agent_id: &str) -> Result<Vec<ResearchTask>, String> {
        let target_box = HermitBox::open(&self.base_dir, agent_id)?;

        let queue = ResearchQueue::open_for_hermit_box(&target_box)
            .map_err(|e| format!("Failed to open queue for '{}': {e}", agent_id))?;

        let all_pending = queue
            .peek(100)
            .map_err(|e| format!("Failed to peek queue: {e}"))?;

        Ok(all_pending
            .into_iter()
            .filter(|t| t.source == "a2a")
            .collect())
    }
}

// ─── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_agent_id_valid() {
        assert!(validate_agent_id("kimi").is_ok());
        assert!(validate_agent_id("teagan").is_ok());
        assert!(validate_agent_id("agent-1").is_ok());
        assert!(validate_agent_id("my_agent").is_ok());
        assert!(validate_agent_id("Agent123").is_ok());
    }

    #[test]
    fn test_validate_agent_id_rejects_empty() {
        assert!(validate_agent_id("").is_err());
    }

    #[test]
    fn test_validate_agent_id_rejects_special_chars() {
        // Bracket injection attempt
        assert!(validate_agent_id("kimi]fake").is_err());
        assert!(validate_agent_id("[evil").is_err());
        // Path traversal
        assert!(validate_agent_id("../etc").is_err());
        assert!(validate_agent_id("a/b").is_err());
        // Spaces and other chars
        assert!(validate_agent_id("has space").is_err());
        assert!(validate_agent_id("semi;colon").is_err());
    }

    #[test]
    fn test_validate_agent_id_rejects_too_long() {
        let long_id = "a".repeat(65);
        assert!(validate_agent_id(&long_id).is_err());
        let ok_id = "a".repeat(64);
        assert!(validate_agent_id(&ok_id).is_ok());
    }

    #[test]
    fn test_send_rejects_spoofed_sender() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path();

        // Create target box but NOT sender box
        std::fs::create_dir_all(base.join("target_box")).expect("mkdir target");

        let router = A2ARouter::new(base);
        let msg = A2AMessage {
            from_agent: "nonexistent-sender".to_string(),
            to_agent: "target".to_string(),
            task: "test task".to_string(),
            priority: 5.0,
            created_at: Utc::now(),
        };

        let result = router.send(&msg);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Sender agent"));
    }

    #[test]
    fn test_send_rejects_invalid_from_agent_chars() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let router = A2ARouter::new(tmp.path());

        let msg = A2AMessage {
            from_agent: "kimi]spoofed".to_string(),
            to_agent: "teagan".to_string(),
            task: "test".to_string(),
            priority: 5.0,
            created_at: Utc::now(),
        };

        let result = router.send(&msg);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid characters"));
    }

    #[test]
    fn test_send_rejects_invalid_to_agent_chars() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let router = A2ARouter::new(tmp.path());

        let msg = A2AMessage {
            from_agent: "kimi".to_string(),
            to_agent: "teagan/../evil".to_string(),
            task: "test".to_string(),
            priority: 5.0,
            created_at: Utc::now(),
        };

        let result = router.send(&msg);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid characters"));
    }
}
