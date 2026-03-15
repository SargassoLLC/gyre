//! Learning loop — periodic reflection that distills memories into telos files.
//!
//! After every N turns, the learning loop scans recent memories and appends
//! lessons to EXPERIENCES.md, high-importance decisions to BELIEFS.md as
//! candidate beliefs, and commitments to GOALS.md.

use std::sync::atomic::{AtomicU32, Ordering};

use chrono::Utc;
use uuid::Uuid;

use crate::cognitive::hermit_box::HermitBox;
use crate::cognitive::memory_stream::{MemoryEntry, MemoryKind};

/// Sanitize content before appending to telos markdown files.
///
/// Strips lines that consist solely of `---` (YAML frontmatter markers) to
/// prevent a crafted memory from injecting frontmatter blocks that break the
/// telos file structure. Also strips null bytes.
fn sanitize_telos_content(content: &str) -> String {
    content
        .lines()
        .filter(|line| line.trim() != "---")
        .collect::<Vec<_>>()
        .join("\n")
        .replace('\0', "")
}

pub struct LearningLoop {
    pub turns_before_reflection: u32,
    turn_count: AtomicU32,
}

impl LearningLoop {
    pub fn new(turns_before_reflection: u32) -> Self {
        Self {
            turns_before_reflection,
            turn_count: AtomicU32::new(0),
        }
    }

    /// Current turn count.
    pub fn turn_count(&self) -> u32 {
        self.turn_count.load(Ordering::Relaxed)
    }

    /// Record a turn. Returns `true` if a reflection should run (every N turns).
    pub fn record_turn(&self) -> bool {
        let count = self.turn_count.fetch_add(1, Ordering::Relaxed) + 1;
        count % self.turns_before_reflection == 0
    }

    /// Reflect on recent memories and distill into telos files.
    ///
    /// - Lessons → EXPERIENCES.md
    /// - High-importance Decisions (>8.0) → BELIEFS.md as candidate beliefs
    /// - Commitments → GOALS.md
    pub fn reflect(
        &self,
        hermit_box: &HermitBox,
        recent_memories: &[MemoryEntry],
    ) -> Result<(), String> {
        // Lessons → EXPERIENCES.md
        for entry in recent_memories
            .iter()
            .filter(|e| e.kind == MemoryKind::Lesson)
        {
            let date = entry.created_at.format("%Y-%m-%d");
            let safe_content = sanitize_telos_content(&entry.content);
            let block = format!("### {date}\n{safe_content}\n");
            hermit_box.append_telos_file("EXPERIENCES.md", &block)?;
        }

        // High-importance Decisions → BELIEFS.md as candidate beliefs
        for entry in recent_memories
            .iter()
            .filter(|e| e.kind == MemoryKind::Decision && e.importance > 8.0)
        {
            let date = entry.created_at.format("%Y-%m-%d");
            let safe_content = sanitize_telos_content(&entry.content);
            let block = format!(
                "### Candidate Belief ({date})\n{safe_content}\n*(from Decision memory \u{2014} review and accept if appropriate)*\n",
            );
            hermit_box.append_telos_file("BELIEFS.md", &block)?;
        }

        // Commitments → GOALS.md
        for entry in recent_memories
            .iter()
            .filter(|e| e.kind == MemoryKind::Commitment)
        {
            let date = entry.created_at.format("%Y-%m-%d");
            let safe_content = sanitize_telos_content(&entry.content);
            let block = format!("### Active Commitment ({date})\n{safe_content}\n");
            hermit_box.append_telos_file("GOALS.md", &block)?;
        }

        Ok(())
    }

    /// End-of-session reflection: same as reflect() but also creates a Handoff memory.
    ///
    /// Returns the handoff entry so the caller can store it in the memory stream.
    pub fn end_of_session_reflect(
        &self,
        hermit_box: &HermitBox,
        all_memories: &[MemoryEntry],
    ) -> Result<MemoryEntry, String> {
        // Run standard reflection
        self.reflect(hermit_box, all_memories)?;

        // Build handoff summary
        let first_preview = all_memories
            .first()
            .map(|m| {
                if m.content.len() > 80 {
                    // Find a valid char boundary at or before 80 to avoid panic
                    let mut end = 80;
                    while !m.content.is_char_boundary(end) && end > 0 {
                        end -= 1;
                    }
                    format!("{}...", &m.content[..end])
                } else {
                    m.content.clone()
                }
            })
            .unwrap_or_else(|| "(no memories)".to_string());

        let handoff_content = format!(
            "Session ended. {} memories stored this session. Recent focus: {}",
            all_memories.len(),
            first_preview
        );

        Ok(MemoryEntry {
            id: Uuid::new_v4(),
            content: handoff_content,
            kind: MemoryKind::Handoff,
            importance: 9.5,
            created_at: Utc::now(),
            namespace: crate::cognitive::memory_stream::namespaces::PERSONAL.to_string(),
        })
    }
}

impl Default for LearningLoop {
    fn default() -> Self {
        Self::new(10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_frontmatter_markers() {
        let input = "line one\n---\nline two\n---\nline three";
        let result = sanitize_telos_content(input);
        assert_eq!(result, "line one\nline two\nline three");
    }

    #[test]
    fn sanitize_strips_null_bytes() {
        let input = "hello\0world";
        let result = sanitize_telos_content(input);
        assert_eq!(result, "helloworld");
    }

    #[test]
    fn sanitize_preserves_normal_content() {
        let input = "A normal lesson learned today.";
        let result = sanitize_telos_content(input);
        assert_eq!(result, input);
    }

    #[test]
    fn sanitize_strips_padded_frontmatter_markers() {
        let input = "before\n  ---  \nafter";
        let result = sanitize_telos_content(input);
        assert_eq!(result, "before\nafter");
    }

    #[test]
    fn record_turn_triggers_at_threshold() {
        let ll = LearningLoop::new(3);
        assert!(!ll.record_turn()); // 1
        assert!(!ll.record_turn()); // 2
        assert!(ll.record_turn()); // 3 — reflection
        assert!(!ll.record_turn()); // 4
    }
}
