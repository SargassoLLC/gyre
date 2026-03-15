//! Unified Output Capture System (UOCS) — dual-write markdown files for memory entries.
//!
//! Every memory stored in the SQLite memory stream is also written as a Markdown
//! file under `{box_dir}/memory/{kind}/`, enabling human-readable browsing and
//! file-based search alongside the database.

use std::path::{Path, PathBuf};

use crate::cognitive::memory_stream::{MemoryEntry, MemoryKind};

/// All 8 memory kinds as subdirectory names.
const KIND_DIRS: &[&str] = &[
    "decisions",
    "lessons",
    "persons",
    "projects",
    "commitments",
    "preferences",
    "handoffs",
    "observations",
];

/// Maximum file content size for a single memory markdown file (10 KB).
const MAX_FILE_CONTENT_BYTES: usize = 10 * 1024;

/// Maximum entries per section in the generated INDEX.md.
const MAX_INDEX_ENTRIES_PER_SECTION: usize = 50;

/// Map a MemoryKind to its subdirectory name.
fn kind_dir_name(kind: &MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Decision => "decisions",
        MemoryKind::Lesson => "lessons",
        MemoryKind::Person => "persons",
        MemoryKind::Project => "projects",
        MemoryKind::Commitment => "commitments",
        MemoryKind::Preference => "preferences",
        MemoryKind::Handoff => "handoffs",
        MemoryKind::Observation => "observations",
    }
}

pub struct UocsWriter {
    memory_base: PathBuf,
}

impl UocsWriter {
    /// Create a new UocsWriter, ensuring all kind subdirectories exist.
    pub fn new(box_dir: &Path) -> Self {
        let memory_base = box_dir.join("memory");
        for dir_name in KIND_DIRS {
            let dir = memory_base.join(dir_name);
            let _ = std::fs::create_dir_all(&dir);
        }
        Self { memory_base }
    }

    /// Write a memory entry as a markdown file.
    ///
    /// File path: `memory/{kind}/{date}-{id_short}.md`
    /// Uses atomic .tmp rename pattern.
    pub fn write_memory(&self, entry: &MemoryEntry) -> Result<(), String> {
        let dir_name = kind_dir_name(&entry.kind);
        let date = entry.created_at.format("%Y-%m-%d").to_string();
        let id_short = &entry.id.to_string()[..8];
        let filename = format!("{date}-{id_short}.md");
        let target = self.memory_base.join(dir_name).join(&filename);
        let tmp = target.with_extension("md.tmp");

        // Build frontmatter + content
        let mut content = format!(
            "---\ntype: {kind}\nimportance: {importance}\ndate: {date}\n---\n\n{body}\n",
            kind = entry.kind.as_str(),
            importance = entry.importance,
            date = date,
            body = entry.content,
        );

        // Truncate if needed
        if content.len() > MAX_FILE_CONTENT_BYTES {
            let mut end = MAX_FILE_CONTENT_BYTES;
            while !content.is_char_boundary(end) && end > 0 {
                end -= 1;
            }
            content.truncate(end);
        }

        std::fs::write(&tmp, &content)
            .map_err(|e| format!("failed to write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &target).map_err(|e| {
            format!(
                "failed to rename {} -> {}: {e}",
                tmp.display(),
                target.display()
            )
        })?;
        Ok(())
    }

    /// Regenerate the `memory/INDEX.md` file from all kind subdirectories.
    ///
    /// Scans all `*.md` files, reads the first non-frontmatter line as summary,
    /// and writes a structured index sorted by date descending.
    pub fn regenerate_index(&self) -> Result<(), String> {
        let target = self.memory_base.join("INDEX.md");
        let tmp = target.with_extension("md.tmp");

        let mut sections: Vec<(&str, Vec<(String, String)>)> = Vec::new();

        for dir_name in KIND_DIRS {
            let dir = self.memory_base.join(dir_name);
            if !dir.is_dir() {
                continue;
            }

            let mut entries: Vec<(String, String)> = Vec::new();

            // Read all .md files in the directory
            let mut files: Vec<_> = match std::fs::read_dir(&dir) {
                Ok(rd) => rd
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        e.path().extension().is_some_and(|ext| ext == "md")
                            && !e.file_name().to_string_lossy().ends_with(".tmp")
                    })
                    .collect(),
                Err(_) => continue,
            };
            files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

            for file_entry in files.iter().take(MAX_INDEX_ENTRIES_PER_SECTION) {
                let path = file_entry.path();
                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                // Extract date from filename (YYYY-MM-DD-xxxxxxxx.md)
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                let date = if stem.len() >= 10 {
                    stem[..10].to_string()
                } else {
                    "unknown".to_string()
                };

                // Find first non-frontmatter line as summary
                let summary = extract_summary(&content);
                entries.push((date, summary));
            }

            if !entries.is_empty() {
                sections.push((dir_name, entries));
            }
        }

        // Build index content
        let mut index = String::from("# Memory Index\n\n");
        for (kind_name, entries) in &sections {
            // Capitalize section name
            let section_title = capitalize(kind_name);
            index.push_str(&format!("## {section_title}\n"));
            for (date, summary) in entries {
                index.push_str(&format!("- {date}: {summary}\n"));
            }
            index.push('\n');
        }

        std::fs::write(&tmp, &index).map_err(|e| format!("failed to write INDEX.md.tmp: {e}"))?;
        std::fs::rename(&tmp, &target)
            .map_err(|e| format!("failed to rename INDEX.md.tmp -> INDEX.md: {e}"))?;
        Ok(())
    }

    /// Search memory markdown files for content matching a query.
    ///
    /// Performs case-insensitive substring matching. Skips INDEX.md.
    /// Returns up to `limit` matching file contents.
    /// Symlinks are skipped to prevent following links outside the memory tree.
    pub fn search_content(&self, query: &str, limit: usize) -> Vec<String> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for entry in walkdir::WalkDir::new(&self.memory_base)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            // Skip symlinks to prevent following links outside the memory tree.
            if entry.path_is_symlink() {
                continue;
            }
            if results.len() >= limit {
                break;
            }
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "md") {
                continue;
            }
            // Skip INDEX.md and .tmp files
            let filename = path.file_name().unwrap_or_default().to_string_lossy();
            if filename == "INDEX.md" || filename.ends_with(".tmp") {
                continue;
            }

            if let Ok(content) = std::fs::read_to_string(path) {
                if content.to_lowercase().contains(&query_lower) {
                    results.push(content);
                }
            }
        }

        results
    }

    /// Return the base memory directory path.
    pub fn memory_base(&self) -> &Path {
        &self.memory_base
    }
}

/// Extract the first non-frontmatter, non-empty line as a summary.
fn extract_summary(content: &str) -> String {
    let mut in_frontmatter = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "---" {
            in_frontmatter = !in_frontmatter;
            continue;
        }
        if in_frontmatter {
            continue;
        }
        if !trimmed.is_empty() {
            // Truncate long summaries (char-boundary safe)
            let max_summary = 120;
            if trimmed.len() > max_summary {
                let mut end = max_summary;
                while !trimmed.is_char_boundary(end) && end > 0 {
                    end -= 1;
                }
                return format!("{}...", &trimmed[..end]);
            }
            return trimmed.to_string();
        }
    }
    "(empty)".to_string()
}

/// Capitalize the first letter of a string.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}
