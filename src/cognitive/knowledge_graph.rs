use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::cognitive::embedding::EmbeddingProvider;
use crate::cognitive::memory_stream::{blob_to_f32_vec, cosine_similarity, f32_vec_to_blob};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EntityLayer {
    Research,
    Concept,
    Axiom,
}

impl EntityLayer {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Research => "research",
            Self::Concept => "concept",
            Self::Axiom => "axiom",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "concept" => Self::Concept,
            "axiom" => Self::Axiom,
            _ => Self::Research,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KgEntity {
    pub id: Uuid,
    pub name: String,
    pub layer: EntityLayer,
    pub summary: String,
    pub importance: f32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KgEdge {
    pub id: Uuid,
    pub from_id: Uuid,
    pub to_id: Uuid,
    pub relationship: String,
    pub weight: f32,
    pub created_at: DateTime<Utc>,
}

pub struct KnowledgeGraph {
    db: Arc<Mutex<Connection>>,
}

impl KnowledgeGraph {
    pub fn new(db_path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS kg_entities (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                layer TEXT NOT NULL,
                summary TEXT NOT NULL DEFAULT '',
                importance REAL NOT NULL DEFAULT 5.0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_kg_name ON kg_entities(name);
            CREATE INDEX IF NOT EXISTS idx_kg_layer ON kg_entities(layer);
            CREATE TABLE IF NOT EXISTS kg_edges (
                id TEXT PRIMARY KEY,
                from_id TEXT NOT NULL,
                to_id TEXT NOT NULL,
                relationship TEXT NOT NULL,
                weight REAL NOT NULL DEFAULT 0.5,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_kg_from ON kg_edges(from_id);
            CREATE INDEX IF NOT EXISTS idx_kg_to ON kg_edges(to_id);
            ",
        )?;
        // Phase 5b migration: add embedding BLOB column to kg_entities.
        let has_embedding: bool = conn
            .prepare("SELECT embedding FROM kg_entities LIMIT 0")
            .is_ok();
        if !has_embedding {
            conn.execute_batch("ALTER TABLE kg_entities ADD COLUMN embedding BLOB;")?;
        }
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn upsert_entity(&self, e: &KgEntity) -> rusqlite::Result<()> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        db.execute(
            "INSERT OR REPLACE INTO kg_entities (id, name, layer, summary, importance, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                e.id.to_string(),
                e.name,
                e.layer.as_str(),
                e.summary,
                e.importance,
                e.created_at.to_rfc3339(),
                e.updated_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn add_edge(&self, edge: &KgEdge) -> rusqlite::Result<()> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        db.execute(
            "INSERT OR REPLACE INTO kg_edges VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                edge.id.to_string(),
                edge.from_id.to_string(),
                edge.to_id.to_string(),
                edge.relationship,
                edge.weight,
                edge.created_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn search_by_name(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<KgEntity>> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let pattern = format!("%{query}%");
        let mut stmt = db.prepare(
            "SELECT id, name, layer, summary, importance, created_at, updated_at \
             FROM kg_entities WHERE name LIKE ?1 LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![pattern, limit as i64], |row| {
                Ok(KgEntity {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    name: row.get(1)?,
                    layer: EntityLayer::from_str(&row.get::<_, String>(2)?),
                    summary: row.get(3)?,
                    importance: row.get(4)?,
                    created_at: row
                        .get::<_, String>(5)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: row
                        .get::<_, String>(6)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn get_axioms(&self) -> rusqlite::Result<Vec<KgEntity>> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let mut stmt = db.prepare(
            "SELECT id, name, layer, summary, importance, created_at, updated_at \
             FROM kg_entities WHERE layer = 'axiom'",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(KgEntity {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    name: row.get(1)?,
                    layer: EntityLayer::Axiom,
                    summary: row.get(3)?,
                    importance: row.get(4)?,
                    created_at: row
                        .get::<_, String>(5)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: row
                        .get::<_, String>(6)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Insert an entity by name if it doesn't exist, or return the existing one's ID.
    ///
    /// Uses INSERT OR IGNORE to avoid duplicates, then SELECT to get the ID.
    pub fn upsert_by_name(
        &self,
        name: &str,
        layer: EntityLayer,
        summary: &str,
        importance: f32,
    ) -> rusqlite::Result<Uuid> {
        self.upsert_by_name_with_embedding(name, layer, summary, importance, None)
    }

    /// Insert an entity by name with an optional embedding vector.
    ///
    /// If the entity already exists and an embedding is provided, updates the embedding.
    pub fn upsert_by_name_with_embedding(
        &self,
        name: &str,
        layer: EntityLayer,
        summary: &str,
        importance: f32,
        embedding: Option<&[f32]>,
    ) -> rusqlite::Result<Uuid> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let now = Utc::now().to_rfc3339();
        let new_id = Uuid::new_v4().to_string();
        let blob = embedding.map(f32_vec_to_blob);
        db.execute(
            "INSERT OR IGNORE INTO kg_entities (id, name, layer, summary, importance, created_at, updated_at, embedding) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![new_id, name, layer.as_str(), summary, importance, &now, &now, blob],
        )?;
        // If the entity already existed and we have an embedding, update it
        if let Some(ref b) = blob {
            db.execute(
                "UPDATE kg_entities SET embedding = ?1, updated_at = ?2 WHERE name = ?3 AND embedding IS NULL",
                params![b, &now, name],
            )?;
        }
        let id_str: String = db.query_row(
            "SELECT id FROM kg_entities WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )?;
        Ok(Uuid::parse_str(&id_str).unwrap_or_default())
    }

    /// Semantic search: find entities closest to the given query embedding.
    ///
    /// Pre-filters to entities that have embeddings, computes cosine similarity,
    /// and returns `(entity, similarity)` pairs sorted descending.
    pub fn search_semantic(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> rusqlite::Result<Vec<(KgEntity, f32)>> {
        if query_embedding.is_empty() {
            return Ok(Vec::new());
        }
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;

        let mut stmt = db.prepare(
            "SELECT id, name, layer, summary, importance, created_at, updated_at, embedding \
             FROM kg_entities WHERE embedding IS NOT NULL",
        )?;
        let mut results: Vec<(KgEntity, f32)> = stmt
            .query_map([], |row| {
                let blob: Vec<u8> = row.get(7)?;
                let entity = KgEntity {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    name: row.get(1)?,
                    layer: EntityLayer::from_str(&row.get::<_, String>(2)?),
                    summary: row.get(3)?,
                    importance: row.get(4)?,
                    created_at: row
                        .get::<_, String>(5)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: row
                        .get::<_, String>(6)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                };
                Ok((entity, blob))
            })?
            .filter_map(|r| r.ok())
            .map(|(entity, blob)| {
                let stored = blob_to_f32_vec(&blob);
                let score = cosine_similarity(query_embedding, &stored);
                (entity, score)
            })
            .collect();

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);
        Ok(results)
    }

    /// Backfill embeddings for entities that lack them.
    ///
    /// Generates embeddings from entity name + summary, stores as BLOBs.
    /// Returns the count of entities backfilled.
    pub fn backfill_embeddings(&self, provider: &dyn EmbeddingProvider) -> rusqlite::Result<usize> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;

        let mut stmt =
            db.prepare("SELECT id, name, summary FROM kg_entities WHERE embedding IS NULL")?;
        let rows: Vec<(String, String, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        if rows.is_empty() {
            return Ok(0);
        }

        let mut update_stmt = db.prepare("UPDATE kg_entities SET embedding = ?1 WHERE id = ?2")?;
        let mut count = 0usize;
        for (id, name, summary) in &rows {
            let text = if summary.is_empty() {
                name.clone()
            } else {
                format!("{name}: {summary}")
            };
            let embedding = provider.embed(&text);
            if !embedding.is_empty() {
                let blob = f32_vec_to_blob(&embedding);
                update_stmt.execute(params![blob, id])?;
                count += 1;
            }
        }
        if count > 0 {
            tracing::info!("Backfilled {count} KG entity embeddings");
        }
        Ok(count)
    }

    /// Find entities with high importance but few edges (isolated).
    pub fn entities_with_few_edges(
        &self,
        min_importance: f32,
        max_edges: u32,
    ) -> rusqlite::Result<Vec<KgEntity>> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let mut stmt = db.prepare(
            "SELECT e.id, e.name, e.layer, e.summary, e.importance, e.created_at, e.updated_at \
             FROM kg_entities e \
             WHERE e.importance > ?1 \
               AND (SELECT COUNT(*) FROM kg_edges WHERE from_id = e.id OR to_id = e.id) < ?2",
        )?;
        let rows = stmt
            .query_map(params![min_importance, max_edges], |row| {
                Ok(KgEntity {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    name: row.get(1)?,
                    layer: EntityLayer::from_str(&row.get::<_, String>(2)?),
                    summary: row.get(3)?,
                    importance: row.get(4)?,
                    created_at: row
                        .get::<_, String>(5)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: row
                        .get::<_, String>(6)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Find entities not updated since `older_than`.
    pub fn stale_entities(&self, older_than: DateTime<Utc>) -> rusqlite::Result<Vec<KgEntity>> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let mut stmt = db.prepare(
            "SELECT id, name, layer, summary, importance, created_at, updated_at \
             FROM kg_entities WHERE updated_at < ?1",
        )?;
        let rows = stmt
            .query_map(params![older_than.to_rfc3339()], |row| {
                Ok(KgEntity {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    name: row.get(1)?,
                    layer: EntityLayer::from_str(&row.get::<_, String>(2)?),
                    summary: row.get(3)?,
                    importance: row.get(4)?,
                    created_at: row
                        .get::<_, String>(5)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: row
                        .get::<_, String>(6)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn entity_count(&self) -> rusqlite::Result<usize> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let count: i64 = db.query_row("SELECT COUNT(*) FROM kg_entities", [], |r| r.get(0))?;
        Ok(count as usize)
    }

    /// Spreading activation from seed entities through edges.
    ///
    /// Seeds start with activation score 1.0. Each hop multiplies by
    /// `edge.weight * decay`. BFS up to `depth` hops. Returns the top 20
    /// entities sorted by activation score descending.
    pub fn activate(
        &self,
        seed_entity_names: &[&str],
        depth: u32,
        decay: f32,
    ) -> rusqlite::Result<Vec<(KgEntity, f32)>> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;

        // Resolve seed entity IDs and initialize activation scores
        let mut activation: std::collections::HashMap<String, f32> =
            std::collections::HashMap::new();
        let mut id_to_entity: std::collections::HashMap<String, KgEntity> =
            std::collections::HashMap::new();

        for name in seed_entity_names {
            let mut stmt = db.prepare(
                "SELECT id, name, layer, summary, importance, created_at, updated_at \
                 FROM kg_entities WHERE name = ?1",
            )?;
            let entities: Vec<KgEntity> = stmt
                .query_map(params![*name], |row| {
                    Ok(KgEntity {
                        id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                        name: row.get(1)?,
                        layer: EntityLayer::from_str(&row.get::<_, String>(2)?),
                        summary: row.get(3)?,
                        importance: row.get(4)?,
                        created_at: row
                            .get::<_, String>(5)?
                            .parse()
                            .unwrap_or_else(|_| Utc::now()),
                        updated_at: row
                            .get::<_, String>(6)?
                            .parse()
                            .unwrap_or_else(|_| Utc::now()),
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();

            for entity in entities {
                let id_str = entity.id.to_string();
                activation.insert(id_str.clone(), 1.0);
                id_to_entity.insert(id_str, entity);
            }
        }

        // BFS: expand frontier `depth` times
        let mut frontier: Vec<String> = activation.keys().cloned().collect();

        for _ in 0..depth {
            if frontier.is_empty() {
                break;
            }
            let mut next_frontier: Vec<String> = Vec::new();

            for node_id in &frontier {
                let current_score = activation.get(node_id).copied().unwrap_or(0.0);

                // Find outgoing edges
                let mut stmt = db.prepare(
                    "SELECT e.id, e.from_id, e.to_id, e.relationship, e.weight, e.created_at, \
                     n.id, n.name, n.layer, n.summary, n.importance, n.created_at, n.updated_at \
                     FROM kg_edges e \
                     JOIN kg_entities n ON n.id = e.to_id \
                     WHERE e.from_id = ?1",
                )?;
                let neighbors: Vec<(String, f32, KgEntity)> = stmt
                    .query_map(params![node_id], |row| {
                        let target_id: String = row.get(2)?;
                        let edge_weight: f32 = row.get(4)?;
                        let entity = KgEntity {
                            id: Uuid::parse_str(&row.get::<_, String>(6)?).unwrap_or_default(),
                            name: row.get(7)?,
                            layer: EntityLayer::from_str(&row.get::<_, String>(8)?),
                            summary: row.get(9)?,
                            importance: row.get(10)?,
                            created_at: row
                                .get::<_, String>(11)?
                                .parse()
                                .unwrap_or_else(|_| Utc::now()),
                            updated_at: row
                                .get::<_, String>(12)?
                                .parse()
                                .unwrap_or_else(|_| Utc::now()),
                        };
                        Ok((target_id, edge_weight, entity))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();

                // Also check incoming edges (bidirectional activation)
                let mut stmt_rev = db.prepare(
                    "SELECT e.id, e.from_id, e.to_id, e.relationship, e.weight, e.created_at, \
                     n.id, n.name, n.layer, n.summary, n.importance, n.created_at, n.updated_at \
                     FROM kg_edges e \
                     JOIN kg_entities n ON n.id = e.from_id \
                     WHERE e.to_id = ?1",
                )?;
                let rev_neighbors: Vec<(String, f32, KgEntity)> = stmt_rev
                    .query_map(params![node_id], |row| {
                        let source_id: String = row.get(1)?;
                        let edge_weight: f32 = row.get(4)?;
                        let entity = KgEntity {
                            id: Uuid::parse_str(&row.get::<_, String>(6)?).unwrap_or_default(),
                            name: row.get(7)?,
                            layer: EntityLayer::from_str(&row.get::<_, String>(8)?),
                            summary: row.get(9)?,
                            importance: row.get(10)?,
                            created_at: row
                                .get::<_, String>(11)?
                                .parse()
                                .unwrap_or_else(|_| Utc::now()),
                            updated_at: row
                                .get::<_, String>(12)?
                                .parse()
                                .unwrap_or_else(|_| Utc::now()),
                        };
                        Ok((source_id, edge_weight, entity))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();

                for (target_id, edge_weight, entity) in
                    neighbors.into_iter().chain(rev_neighbors.into_iter())
                {
                    let propagated = current_score * edge_weight * decay;
                    let existing = activation.get(&target_id).copied().unwrap_or(0.0);
                    if propagated > existing {
                        activation.insert(target_id.clone(), propagated);
                        id_to_entity.insert(target_id.clone(), entity);
                        next_frontier.push(target_id);
                    }
                }
            }

            frontier = next_frontier;
        }

        // Collect results, sort by activation score descending, top 20
        let mut results: Vec<(KgEntity, f32)> = activation
            .into_iter()
            .filter_map(|(id, score)| id_to_entity.remove(&id).map(|entity| (entity, score)))
            .collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(20);

        Ok(results)
    }

    /// Convenience: run spreading activation and format results as a context string.
    ///
    /// Uses depth=3, decay=0.7. Returns empty string if no activations found.
    pub fn activated_context_string(&self, seed_names: &[&str]) -> rusqlite::Result<String> {
        if seed_names.is_empty() {
            return Ok(String::new());
        }
        let activated = self.activate(seed_names, 3, 0.7)?;
        if activated.is_empty() {
            return Ok(String::new());
        }
        let lines: Vec<String> = activated
            .iter()
            .map(|(entity, score)| format!("- {} (score: {:.2})", entity.name, score))
            .collect();
        Ok(lines.join("\n"))
    }
}
