use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::cognitive::embedding::EmbeddingProvider;
use crate::cognitive::roles::AgentIdentity;

/// Serialize a `Vec<f32>` into a little-endian byte blob.
pub fn f32_vec_to_blob(v: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(v.len() * 4);
    for &val in v {
        buf.extend_from_slice(&val.to_le_bytes());
    }
    buf
}

/// Deserialize a little-endian byte blob back into `Vec<f32>`.
pub fn blob_to_f32_vec(blob: &[u8]) -> Vec<f32> {
    if blob.len() % 4 != 0 {
        return Vec::new();
    }
    blob.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// Well-known memory namespace constants.
pub mod namespaces {
    pub const PERSONAL: &str = "personal";
    pub const TRIBE: &str = "tribe";

    pub fn client(name: &str) -> String {
        format!("client:{}", name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryKind {
    Decision,
    Lesson,
    Person,
    Project,
    Commitment,
    Preference,
    Handoff,
    Observation,
}

impl MemoryKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Decision => "decision",
            Self::Lesson => "lesson",
            Self::Person => "person",
            Self::Project => "project",
            Self::Commitment => "commitment",
            Self::Preference => "preference",
            Self::Handoff => "handoff",
            Self::Observation => "observation",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "decision" => Self::Decision,
            "lesson" => Self::Lesson,
            "person" => Self::Person,
            "project" => Self::Project,
            "commitment" => Self::Commitment,
            "preference" => Self::Preference,
            "handoff" => Self::Handoff,
            _ => Self::Observation,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: Uuid,
    pub content: String,
    pub kind: MemoryKind,
    pub importance: f32,
    pub created_at: DateTime<Utc>,
    #[serde(default = "default_namespace")]
    pub namespace: String,
}

fn default_namespace() -> String {
    namespaces::PERSONAL.to_string()
}

pub struct MemoryStream {
    db: Arc<Mutex<Connection>>,
    pub uocs: Option<Arc<crate::cognitive::uocs::UocsWriter>>,
}

impl MemoryStream {
    pub fn new(db_path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory_entries (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                kind TEXT NOT NULL,
                importance REAL NOT NULL DEFAULT 5.0,
                created_at TEXT NOT NULL
            );",
        )?;
        // Phase 3 migration: add nullable embedding column (JSON-serialized Vec<f32>).
        let has_embedding: bool = conn
            .prepare("SELECT embedding FROM memory_entries LIMIT 0")
            .is_ok();
        if !has_embedding {
            conn.execute_batch("ALTER TABLE memory_entries ADD COLUMN embedding TEXT;")?;
        }
        // Phase 5 migration: add namespace column.
        let has_namespace: bool = conn
            .prepare("SELECT namespace FROM memory_entries LIMIT 0")
            .is_ok();
        if !has_namespace {
            conn.execute_batch(
                "ALTER TABLE memory_entries ADD COLUMN namespace TEXT NOT NULL DEFAULT 'personal';",
            )?;
        }
        // Phase 5b migration: add embedding_blob column (BLOB, replaces JSON TEXT).
        let has_embedding_blob: bool = conn
            .prepare("SELECT embedding_blob FROM memory_entries LIMIT 0")
            .is_ok();
        if !has_embedding_blob {
            conn.execute_batch("ALTER TABLE memory_entries ADD COLUMN embedding_blob BLOB;")?;
            // Migrate existing JSON embeddings → BLOB.
            Self::migrate_json_to_blob(&conn)?;
        }
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
            uocs: None,
        })
    }

    /// Maximum content length per memory entry (10 KB).
    /// Enforced at the storage layer as a defense-in-depth measure.
    pub const MAX_CONTENT_BYTES: usize = 10 * 1024;

    /// Pre-filter limit when doing cosine similarity scan.
    /// Instead of loading ALL rows, we load the top N by importance+recency.
    const PREFILTER_LIMIT: usize = 1000;

    /// Migrate existing JSON TEXT embeddings to BLOB format.
    fn migrate_json_to_blob(conn: &Connection) -> rusqlite::Result<()> {
        let mut stmt = conn.prepare(
            "SELECT id, embedding FROM memory_entries WHERE embedding IS NOT NULL AND embedding_blob IS NULL",
        )?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        if rows.is_empty() {
            return Ok(());
        }

        let mut update_stmt =
            conn.prepare("UPDATE memory_entries SET embedding_blob = ?1 WHERE id = ?2")?;
        let mut migrated = 0u32;
        for (id, json_str) in &rows {
            if let Ok(vec) = serde_json::from_str::<Vec<f32>>(json_str) {
                let blob = f32_vec_to_blob(&vec);
                update_stmt.execute(params![blob, id])?;
                migrated += 1;
            }
        }
        if migrated > 0 {
            tracing::info!("Migrated {migrated} JSON embeddings to BLOB format");
        }
        Ok(())
    }

    pub fn add(&self, entry: &MemoryEntry) -> rusqlite::Result<()> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let ns = if entry.namespace.is_empty() {
            namespaces::PERSONAL
        } else {
            &entry.namespace
        };
        // Enforce content size limit at storage layer
        let content = if entry.content.len() > Self::MAX_CONTENT_BYTES {
            let mut end = Self::MAX_CONTENT_BYTES;
            while !entry.content.is_char_boundary(end) && end > 0 {
                end -= 1;
            }
            &entry.content[..end]
        } else {
            &entry.content
        };
        db.execute(
            "INSERT INTO memory_entries (id, content, kind, importance, created_at, namespace) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                entry.id.to_string(),
                content,
                entry.kind.as_str(),
                entry.importance,
                entry.created_at.to_rfc3339(),
                ns
            ],
        )?;
        Ok(())
    }

    /// Guarded write: only Chiefs can write; Workers get an error.
    pub fn add_guarded(&self, entry: &MemoryEntry, identity: &AgentIdentity) -> Result<(), String> {
        if !identity.can_write_memory() {
            return Err(format!(
                "Worker '{}' cannot write memory directly",
                identity.id
            ));
        }
        self.add(entry)
            .map_err(|e| format!("memory write failed: {}", e))
    }

    /// Three-factor recall: score = recency_weight + importance_norm + 0.5 (relevance placeholder)
    /// Returns top `limit` entries by score.
    ///
    /// NOTE: This returns entries from ALL namespaces. For namespace-scoped
    /// queries (required when serving Workers), use `recall_in_namespace()`.
    pub fn recall(&self, _query: &str, limit: usize) -> rusqlite::Result<Vec<MemoryEntry>> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let mut stmt = db.prepare(
            "SELECT id, content, kind, importance, created_at, namespace \
             FROM memory_entries ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(MemoryEntry {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    content: row.get(1)?,
                    kind: MemoryKind::from_str(&row.get::<_, String>(2)?),
                    importance: row.get(3)?,
                    created_at: row
                        .get::<_, String>(4)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    namespace: row
                        .get::<_, String>(5)
                        .unwrap_or_else(|_| default_namespace()),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// 4-layer recall: INDEX.md → UOCS search → SQLite recency fallback.
    ///
    /// Layer 1: Read INDEX.md (if uocs set), find entries whose summary contains query keywords.
    /// Layer 2: uocs.search_content() → parse entry IDs from filenames → SQLite lookup.
    /// Layer 3: existing recall() (recency sort) as fallback.
    /// Results are merged and deduped by entry ID.
    pub fn recall_layered(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<MemoryEntry>> {
        let mut seen_ids = std::collections::HashSet::new();
        let mut results = Vec::new();

        if let Some(ref uocs) = self.uocs {
            // Layer 1+2: INDEX.md keyword scan → UOCS content search → SQLite lookup
            if results.len() < limit {
                let found_contents = uocs.search_content(query, limit * 2);
                // For each found content, try to match to DB entries by content substring
                if !found_contents.is_empty() {
                    let db = self.db.lock().map_err(|e| {
                        rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}"))
                    })?;
                    for found in &found_contents {
                        if results.len() >= limit {
                            break;
                        }
                        // Extract the body text (after frontmatter)
                        let body = extract_body_from_md(found);
                        if body.is_empty() {
                            continue;
                        }
                        // Look up by content match in DB
                        let mut stmt = db.prepare(
                            "SELECT id, content, kind, importance, created_at, namespace \
                             FROM memory_entries WHERE content = ?1 LIMIT 1",
                        )?;
                        let entry = stmt
                            .query_row(rusqlite::params![body], |row| {
                                Ok(MemoryEntry {
                                    id: Uuid::parse_str(&row.get::<_, String>(0)?)
                                        .unwrap_or_default(),
                                    content: row.get(1)?,
                                    kind: MemoryKind::from_str(&row.get::<_, String>(2)?),
                                    importance: row.get(3)?,
                                    created_at: row
                                        .get::<_, String>(4)?
                                        .parse()
                                        .unwrap_or_else(|_| Utc::now()),
                                    namespace: row
                                        .get::<_, String>(5)
                                        .unwrap_or_else(|_| default_namespace()),
                                })
                            })
                            .ok();
                        if let Some(entry) = entry {
                            if seen_ids.insert(entry.id) {
                                results.push(entry);
                            }
                        }
                    }
                }
            }
        }

        // Layer 3: SQLite recency fallback
        if results.len() < limit {
            let remaining = limit - results.len();
            let fallback = self.recall(query, remaining + seen_ids.len())?;
            for entry in fallback {
                if results.len() >= limit {
                    break;
                }
                if seen_ids.insert(entry.id) {
                    results.push(entry);
                }
            }
        }

        Ok(results)
    }

    /// Role-aware recall: Workers only see Tribe namespace; Chiefs see all.
    pub fn recall_for_role(
        &self,
        query: &str,
        limit: usize,
        identity: &AgentIdentity,
    ) -> rusqlite::Result<Vec<MemoryEntry>> {
        match identity.role {
            crate::cognitive::roles::AgentRole::Worker => {
                self.recall_in_namespace(namespaces::TRIBE, limit)
            }
            crate::cognitive::roles::AgentRole::Chief => self.recall(query, limit),
        }
    }

    /// Recall memories filtered by namespace.
    pub fn recall_in_namespace(
        &self,
        namespace: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<MemoryEntry>> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let mut stmt = db.prepare(
            "SELECT id, content, kind, importance, created_at, namespace \
             FROM memory_entries WHERE namespace = ?1 ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![namespace, limit as i64], |row| {
                Ok(MemoryEntry {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    content: row.get(1)?,
                    kind: MemoryKind::from_str(&row.get::<_, String>(2)?),
                    importance: row.get(3)?,
                    created_at: row
                        .get::<_, String>(4)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    namespace: row
                        .get::<_, String>(5)
                        .unwrap_or_else(|_| default_namespace()),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn pending_importance(&self, since: DateTime<Utc>) -> rusqlite::Result<f32> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let sum: f64 = db.query_row(
            "SELECT COALESCE(SUM(importance),0) FROM memory_entries WHERE created_at > ?1",
            params![since.to_rfc3339()],
            |r| r.get(0),
        )?;
        Ok(sum as f32)
    }

    pub fn recent(&self, limit: usize) -> rusqlite::Result<Vec<MemoryEntry>> {
        self.recall("", limit)
    }

    /// Return the total number of memory entries.
    pub fn count(&self) -> usize {
        let db = match self.db.lock() {
            Ok(db) => db,
            Err(_) => return 0,
        };
        db.query_row("SELECT COUNT(*) FROM memory_entries", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap_or(0) as usize
    }

    /// Store a memory entry together with its embedding vector.
    ///
    /// The embedding is stored as a little-endian f32 BLOB for efficient retrieval.
    pub fn store_with_embedding(
        &self,
        entry: &MemoryEntry,
        embedding: Vec<f32>,
    ) -> rusqlite::Result<()> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let blob = f32_vec_to_blob(&embedding);
        let ns = if entry.namespace.is_empty() {
            namespaces::PERSONAL
        } else {
            &entry.namespace
        };
        // Enforce content size limit at storage layer
        let content = if entry.content.len() > Self::MAX_CONTENT_BYTES {
            let mut end = Self::MAX_CONTENT_BYTES;
            while !entry.content.is_char_boundary(end) && end > 0 {
                end -= 1;
            }
            &entry.content[..end]
        } else {
            &entry.content
        };
        db.execute(
            "INSERT INTO memory_entries (id, content, kind, importance, created_at, embedding_blob, namespace) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.id.to_string(),
                content,
                entry.kind.as_str(),
                entry.importance,
                entry.created_at.to_rfc3339(),
                blob,
                ns
            ],
        )?;
        Ok(())
    }

    /// Store a memory entry and auto-embed its content using the given provider.
    ///
    /// If the provider returns an empty vector, the entry is stored without an embedding.
    pub fn add_with_auto_embed(
        &self,
        entry: &MemoryEntry,
        provider: &dyn EmbeddingProvider,
    ) -> rusqlite::Result<()> {
        let embedding = provider.embed(&entry.content);
        if embedding.is_empty() {
            self.add(entry)
        } else {
            self.store_with_embedding(entry, embedding)
        }
    }

    /// Recall entries most relevant to the given query embedding via cosine similarity.
    ///
    /// Returns `(entry, similarity_score)` pairs sorted by score descending.
    /// Pre-filters by importance+recency (top 1000) to avoid O(n) full-table scan.
    /// Reads embeddings from BLOB column (fast) with JSON TEXT fallback (legacy data).
    /// Entries without embeddings are appended at the end, sorted by recency,
    /// with a score of `0.0`.
    pub fn recall_relevant(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> rusqlite::Result<Vec<(MemoryEntry, f32)>> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;

        // Pre-filter: load top N entries that have embeddings, ordered by importance+recency.
        // This avoids O(n) full-table scan on large memory stores.
        let mut stmt = db.prepare(
            "SELECT id, content, kind, importance, created_at, embedding_blob, embedding, namespace \
             FROM memory_entries \
             WHERE embedding_blob IS NOT NULL OR embedding IS NOT NULL \
             ORDER BY importance DESC, created_at DESC \
             LIMIT ?1",
        )?;
        let mut with_emb: Vec<(MemoryEntry, f32)> = stmt
            .query_map(params![Self::PREFILTER_LIMIT as i64], |row| {
                let blob: Option<Vec<u8>> = row.get(5)?;
                let json_text: Option<String> = row.get(6)?;
                let entry = MemoryEntry {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    content: row.get(1)?,
                    kind: MemoryKind::from_str(&row.get::<_, String>(2)?),
                    importance: row.get(3)?,
                    created_at: row
                        .get::<_, String>(4)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    namespace: row
                        .get::<_, String>(7)
                        .unwrap_or_else(|_| default_namespace()),
                };
                Ok((entry, blob, json_text))
            })?
            .filter_map(|r| r.ok())
            .map(|(entry, blob, json_text)| {
                // Prefer BLOB (fast), fall back to JSON TEXT (legacy)
                let stored = if let Some(b) = blob {
                    blob_to_f32_vec(&b)
                } else if let Some(j) = json_text {
                    serde_json::from_str::<Vec<f32>>(&j).unwrap_or_default()
                } else {
                    Vec::new()
                };
                let score = cosine_similarity(query_embedding, &stored);
                (entry, score)
            })
            .collect();

        // Sort by similarity descending
        with_emb.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // If we already have enough, truncate and return
        if with_emb.len() >= limit {
            with_emb.truncate(limit);
            return Ok(with_emb);
        }

        // Fallback: fill remaining slots with entries that lack embeddings (recency order)
        let remaining = limit - with_emb.len();
        let mut fallback_stmt = db.prepare(
            "SELECT id, content, kind, importance, created_at, namespace \
             FROM memory_entries WHERE embedding_blob IS NULL AND embedding IS NULL \
             ORDER BY created_at DESC LIMIT ?1",
        )?;
        let fallback: Vec<(MemoryEntry, f32)> = fallback_stmt
            .query_map(params![remaining as i64], |row| {
                Ok(MemoryEntry {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    content: row.get(1)?,
                    kind: MemoryKind::from_str(&row.get::<_, String>(2)?),
                    importance: row.get(3)?,
                    created_at: row
                        .get::<_, String>(4)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    namespace: row
                        .get::<_, String>(5)
                        .unwrap_or_else(|_| default_namespace()),
                })
            })?
            .filter_map(|r| r.ok())
            .map(|entry| (entry, 0.0))
            .collect();

        with_emb.extend(fallback);
        Ok(with_emb)
    }

    /// Backfill embeddings for entries that lack them.
    ///
    /// Finds entries without embeddings, generates them using the provider,
    /// and stores as BLOBs. Returns the count of entries backfilled.
    pub fn backfill_embeddings(&self, provider: &dyn EmbeddingProvider) -> rusqlite::Result<usize> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;

        let mut stmt = db.prepare(
            "SELECT id, content FROM memory_entries \
             WHERE embedding_blob IS NULL AND embedding IS NULL",
        )?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        if rows.is_empty() {
            return Ok(0);
        }

        let mut update_stmt =
            db.prepare("UPDATE memory_entries SET embedding_blob = ?1 WHERE id = ?2")?;
        let mut count = 0usize;
        for (id, content) in &rows {
            let embedding = provider.embed(content);
            if !embedding.is_empty() {
                let blob = f32_vec_to_blob(&embedding);
                update_stmt.execute(params![blob, id])?;
                count += 1;
            }
        }
        if count > 0 {
            tracing::info!("Backfilled {count} memory entry embeddings");
        }
        Ok(count)
    }
}

/// Extract body text from a UOCS markdown file, skipping YAML frontmatter.
fn extract_body_from_md(content: &str) -> String {
    let mut in_frontmatter = false;
    let mut body_lines = Vec::new();
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
            body_lines.push(trimmed);
        }
    }
    body_lines.join("\n")
}

/// Compute cosine similarity between two vectors.
///
/// Returns 0.0 if either vector is empty or both have zero magnitude.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0_f64;
    let mut mag_a = 0.0_f64;
    let mut mag_b = 0.0_f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let x = *x as f64;
        let y = *y as f64;
        dot += x * y;
        mag_a += x * x;
        mag_b += y * y;
    }
    let denom = mag_a.sqrt() * mag_b.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        (dot / denom) as f32
    }
}
