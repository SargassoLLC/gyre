use crate::cognitive::{AxiomCulture, KnowledgeGraph, MemoryEntry, MemoryKind, MemoryStream};
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Telos identity files created for every agent box.
const TELOS_FILES: &[&str] = &[
    "MISSION.md",
    "GOALS.md",
    "BELIEFS.md",
    "EXPERIENCES.md",
    "BOUNDARIES.md",
    "SKILLS.md",
];

/// Maximum size for identity files (soul.md, user.md, memory.md) read into memory.
/// Prevents memory exhaustion from oversized files injected into system prompts.
const MAX_IDENTITY_FILE_BYTES: u64 = 50 * 1024; // 50 KB

pub struct HermitBox {
    pub agent_id: String,
    pub box_dir: PathBuf,
    pub memory_stream: Arc<Mutex<MemoryStream>>,
    pub knowledge_graph: Arc<Mutex<KnowledgeGraph>>,
    pub axiom_culture: Arc<Mutex<AxiomCulture>>,
}

/// Validate that an agent_id is safe for filesystem use.
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
    if agent_id.contains("..") {
        return Err("agent_id must not contain '..'".to_string());
    }
    if agent_id.contains('/') || agent_id.contains('\\') {
        return Err("agent_id must not contain '/' or '\\'".to_string());
    }
    // Alphanumeric + hyphens + underscores only
    if !agent_id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "agent_id must contain only alphanumeric characters, hyphens, or underscores"
                .to_string(),
        );
    }
    Ok(())
}

/// Validate that a telos filename is safe for filesystem use.
///
/// Must end in `.md` and the stem must contain only alphanumeric chars,
/// hyphens, or underscores. Rejects path traversal (`../`), slashes, null
/// bytes, and any other special characters.
fn validate_telos_filename(name: &str) -> Result<(), String> {
    if !name.ends_with(".md") {
        return Err(format!("telos filename must end with .md, got: {name}"));
    }
    let stem = &name[..name.len() - 3];
    if stem.is_empty()
        || !stem
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "telos filename stem must be alphanumeric/hyphens/underscores, got: {stem}"
        ));
    }
    Ok(())
}

/// Read a file up to `max_bytes`, returning an empty string if the file is
/// missing. Silently truncates oversized files to prevent memory exhaustion
/// when identity files are injected into LLM system prompts.
fn read_capped(path: PathBuf, max_bytes: u64) -> String {
    use std::io::Read;
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    let mut limited = file.take(max_bytes);
    let mut buf = String::new();
    if limited.read_to_string(&mut buf).is_err() {
        return String::new();
    }
    buf
}

impl HermitBox {
    pub fn open(base_dir: &Path, agent_id: &str) -> Result<Self, String> {
        validate_agent_id(agent_id)?;

        // Canonicalize base_dir to resolve symlinks and prevent symlink-based
        // escapes (e.g., base_dir itself is a symlink pointing outside the
        // intended storage tree).
        let canonical_base = base_dir.canonicalize().map_err(|e| {
            format!(
                "failed to canonicalize base_dir {}: {e}",
                base_dir.display()
            )
        })?;
        let box_dir = canonical_base.join(format!("{agent_id}_box"));
        let memory_dir = box_dir.join("memory");
        let knowledge_dir = box_dir.join("knowledge");
        let axioms_dir = box_dir.join("axioms");

        let telos_dir = box_dir.join("telos");

        std::fs::create_dir_all(&memory_dir)
            .map_err(|e| format!("failed to create memory dir: {e}"))?;
        std::fs::create_dir_all(&knowledge_dir)
            .map_err(|e| format!("failed to create knowledge dir: {e}"))?;
        std::fs::create_dir_all(&axioms_dir)
            .map_err(|e| format!("failed to create axioms dir: {e}"))?;
        std::fs::create_dir_all(&telos_dir)
            .map_err(|e| format!("failed to create telos dir: {e}"))?;

        // Create telos files if they don't exist
        for filename in TELOS_FILES {
            let path = telos_dir.join(filename);
            if !path.exists() {
                std::fs::write(&path, "")
                    .map_err(|e| format!("failed to create telos/{filename}: {e}"))?;
            }
        }

        let memory_stream = MemoryStream::new(&memory_dir.join("memories.db"))
            .map_err(|e| format!("failed to open MemoryStream: {e}"))?;
        let knowledge_graph = KnowledgeGraph::new(&knowledge_dir.join("kg.db"))
            .map_err(|e| format!("failed to open KnowledgeGraph: {e}"))?;
        let axiom_culture = AxiomCulture::new(&axioms_dir.join("axioms.db"))
            .map_err(|e| format!("failed to open AxiomCulture: {e}"))?;

        Ok(Self {
            agent_id: agent_id.to_string(),
            box_dir,
            memory_stream: Arc::new(Mutex::new(memory_stream)),
            knowledge_graph: Arc::new(Mutex::new(knowledge_graph)),
            axiom_culture: Arc::new(Mutex::new(axiom_culture)),
        })
    }

    pub fn box_dir(&self) -> &Path {
        &self.box_dir
    }

    pub fn soul_path(&self) -> PathBuf {
        self.box_dir.join("soul.md")
    }

    pub fn user_path(&self) -> PathBuf {
        self.box_dir.join("user.md")
    }

    pub fn memory_path(&self) -> PathBuf {
        self.box_dir.join("memory.md")
    }

    pub fn read_soul(&self) -> String {
        read_capped(self.soul_path(), MAX_IDENTITY_FILE_BYTES)
    }

    pub fn read_user(&self) -> String {
        read_capped(self.user_path(), MAX_IDENTITY_FILE_BYTES)
    }

    pub fn read_memory_summary(&self) -> String {
        read_capped(self.memory_path(), MAX_IDENTITY_FILE_BYTES)
    }

    pub fn write_memory_summary(&self, content: &str) -> Result<(), String> {
        let target = self.memory_path();
        let tmp = target.with_extension("md.tmp");
        std::fs::write(&tmp, content).map_err(|e| format!("failed to write memory.md.tmp: {e}"))?;
        std::fs::rename(&tmp, &target)
            .map_err(|e| format!("failed to rename memory.md.tmp -> memory.md: {e}"))?;
        Ok(())
    }

    /// Store a memory in this agent's memory stream.
    pub fn remember(
        &self,
        content: &str,
        kind: MemoryKind,
        importance: f32,
    ) -> rusqlite::Result<()> {
        let ms = self.memory_stream.lock().map_err(|_| {
            rusqlite::Error::InvalidParameterName("memory_stream lock poisoned".into())
        })?;
        ms.add(&MemoryEntry {
            id: Uuid::new_v4(),
            content: content.to_string(),
            kind,
            importance,
            created_at: Utc::now(),
            namespace: crate::cognitive::memory_stream::namespaces::PERSONAL.to_string(),
        })
    }

    /// Recall memories from this agent's memory stream.
    pub fn recall(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<MemoryEntry>> {
        let ms = self.memory_stream.lock().map_err(|_| {
            rusqlite::Error::InvalidParameterName("memory_stream lock poisoned".into())
        })?;
        ms.recall(query, limit)
    }

    /// Path to the telos/ subdirectory.
    pub fn telos_dir(&self) -> PathBuf {
        self.box_dir.join("telos")
    }

    /// Read a telos file by name (e.g. "MISSION.md"). Returns empty string if missing.
    /// Capped at 50 KB to prevent memory exhaustion.
    ///
    /// Validates that `name` contains only alphanumeric chars, hyphens,
    /// underscores, and ends with `.md` — same rules as `append_telos_file`.
    pub fn read_telos_file(&self, name: &str) -> String {
        if validate_telos_filename(name).is_err() {
            return String::new();
        }
        read_capped(self.telos_dir().join(name), MAX_IDENTITY_FILE_BYTES)
    }

    /// Append content to a telos file atomically using .tmp rename pattern.
    ///
    /// Validates that `name` contains only alphanumeric chars, hyphens,
    /// underscores, and ends with `.md`.
    pub fn append_telos_file(&self, name: &str, content: &str) -> Result<(), String> {
        validate_telos_filename(name)?;

        let target = self.telos_dir().join(name);
        let tmp = target.with_extension("md.tmp");

        // Read existing content
        let existing = if target.exists() {
            std::fs::read_to_string(&target)
                .map_err(|e| format!("failed to read telos/{name}: {e}"))?
        } else {
            String::new()
        };

        // Append with newline
        let new_content = if existing.is_empty() {
            format!("{content}\n")
        } else if existing.ends_with('\n') {
            format!("{existing}{content}\n")
        } else {
            format!("{existing}\n{content}\n")
        };

        std::fs::write(&tmp, &new_content)
            .map_err(|e| format!("failed to write telos/{name}.tmp: {e}"))?;
        std::fs::rename(&tmp, &target)
            .map_err(|e| format!("failed to rename telos/{name}.tmp -> {name}: {e}"))?;
        Ok(())
    }

    /// Get axiom prompt context.
    pub fn axiom_context(&self, domain: Option<&str>) -> rusqlite::Result<String> {
        let ac = self.axiom_culture.lock().map_err(|_| {
            rusqlite::Error::InvalidParameterName("axiom_culture lock poisoned".into())
        })?;
        ac.as_prompt_context(domain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telos_filename_rejects_path_traversal() {
        assert!(validate_telos_filename("../evil.md").is_err());
    }

    #[test]
    fn telos_filename_rejects_slashes() {
        assert!(validate_telos_filename("x/y.md").is_err());
    }

    #[test]
    fn telos_filename_rejects_null_bytes() {
        assert!(validate_telos_filename("x\0y.md").is_err());
    }

    #[test]
    fn telos_filename_rejects_backslash() {
        assert!(validate_telos_filename("x\\y.md").is_err());
    }

    #[test]
    fn telos_filename_rejects_dots_only() {
        assert!(validate_telos_filename("...md").is_err());
        assert!(validate_telos_filename(".md").is_err());
    }

    #[test]
    fn telos_filename_rejects_no_extension() {
        assert!(validate_telos_filename("MISSION").is_err());
    }

    #[test]
    fn telos_filename_accepts_valid_names() {
        assert!(validate_telos_filename("MISSION.md").is_ok());
        assert!(validate_telos_filename("my-goals.md").is_ok());
        assert!(validate_telos_filename("test_123.md").is_ok());
    }

    #[test]
    fn agent_id_rejects_traversal() {
        assert!(validate_agent_id("../etc").is_err());
        assert!(validate_agent_id("a/b").is_err());
        assert!(validate_agent_id("a\\b").is_err());
    }

    #[test]
    fn agent_id_accepts_valid() {
        assert!(validate_agent_id("agent-01").is_ok());
        assert!(validate_agent_id("test_agent").is_ok());
    }
}
