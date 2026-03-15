use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Axiom {
    pub id: Uuid,
    pub name: String,
    pub statement: String,
    pub domain: String,
    pub evidence: String,
    pub created_at: DateTime<Utc>,
}

pub struct AxiomCulture {
    db: Arc<Mutex<Connection>>,
}

impl AxiomCulture {
    pub fn new(db_path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS axioms (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                statement TEXT NOT NULL,
                domain TEXT NOT NULL DEFAULT 'general',
                evidence TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_axiom_domain ON axioms(domain);",
        )?;
        // Phase 5 migration: add active column (default true for backward compat).
        let has_active: bool = conn.prepare("SELECT active FROM axioms LIMIT 0").is_ok();
        if !has_active {
            conn.execute_batch("ALTER TABLE axioms ADD COLUMN active INTEGER NOT NULL DEFAULT 1;")?;
        }
        // Phase 5 migration: add proposed_by column (nullable).
        let has_proposed_by: bool = conn
            .prepare("SELECT proposed_by FROM axioms LIMIT 0")
            .is_ok();
        if !has_proposed_by {
            conn.execute_batch("ALTER TABLE axioms ADD COLUMN proposed_by TEXT;")?;
        }
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Add an axiom (active by default). Ignores duplicates by name.
    pub fn add_axiom(&self, axiom: &Axiom) -> rusqlite::Result<()> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        db.execute(
            "INSERT OR IGNORE INTO axioms (id, name, statement, domain, evidence, created_at, active) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
            params![
                axiom.id.to_string(),
                axiom.name,
                axiom.statement,
                axiom.domain,
                axiom.evidence,
                axiom.created_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    /// Propose an axiom (inserted as inactive, with proposer recorded).
    /// Workers use this instead of add_axiom.
    pub fn propose(&self, text: &str, proposer_id: &str) -> rusqlite::Result<Uuid> {
        let id = Uuid::new_v4();
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        db.execute(
            "INSERT OR IGNORE INTO axioms (id, name, statement, domain, evidence, created_at, active, proposed_by) \
             VALUES (?1, ?2, ?3, 'general', '', ?4, 0, ?5)",
            params![
                id.to_string(),
                text,
                text,
                Utc::now().to_rfc3339(),
                proposer_id
            ],
        )?;
        Ok(id)
    }

    /// Approve a proposed axiom, making it active.
    pub fn approve(&self, id: Uuid) -> rusqlite::Result<()> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        db.execute(
            "UPDATE axioms SET active = 1 WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    /// List only active axioms.
    pub fn list_active(&self) -> rusqlite::Result<Vec<Axiom>> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let mut stmt = db.prepare(
            "SELECT id, name, statement, domain, evidence, created_at \
             FROM axioms WHERE active = 1",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Axiom {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    name: row.get(1)?,
                    statement: row.get(2)?,
                    domain: row.get(3)?,
                    evidence: row.get(4)?,
                    created_at: row
                        .get::<_, String>(5)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn get_axioms(&self, domain: Option<&str>) -> rusqlite::Result<Vec<Axiom>> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let (sql, domain_val) = match domain {
            Some(d) => (
                "SELECT id, name, statement, domain, evidence, created_at \
                 FROM axioms WHERE active = 1 AND (domain = ?1 OR domain = 'general')",
                d.to_string(),
            ),
            None => (
                "SELECT id, name, statement, domain, evidence, created_at \
                 FROM axioms WHERE active = 1 AND (1=1 OR domain = ?1)",
                String::new(),
            ),
        };
        let mut stmt = db.prepare(sql)?;
        let rows = stmt
            .query_map(params![domain_val], |row| {
                Ok(Axiom {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    name: row.get(1)?,
                    statement: row.get(2)?,
                    domain: row.get(3)?,
                    evidence: row.get(4)?,
                    created_at: row
                        .get::<_, String>(5)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn as_prompt_context(&self, domain: Option<&str>) -> rusqlite::Result<String> {
        let axioms = self.get_axioms(domain)?;
        if axioms.is_empty() {
            return Ok(String::new());
        }
        let lines: Vec<String> = axioms
            .iter()
            .map(|a| format!("- {}: {}", a.name, a.statement))
            .collect();
        Ok(format!(
            "## Universal Principles (Axiom Culture)\n{}",
            lines.join("\n")
        ))
    }

    pub fn count(&self) -> rusqlite::Result<usize> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let n: i64 = db.query_row("SELECT COUNT(*) FROM axioms", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    /// Sync active axioms from another database file.
    /// Inserts any axioms not already present (matched by statement text).
    /// Returns the count of newly synced axioms.
    ///
    /// **Security:** Validates that the path points to a regular file with a `.db`
    /// extension and contains no path traversal (`..`) components.
    pub fn sync_from(&self, other_db_path: &Path) -> rusqlite::Result<usize> {
        // Path validation: reject path traversal and non-.db files
        for component in other_db_path.components() {
            if let std::path::Component::ParentDir = component {
                return Err(rusqlite::Error::InvalidParameterName(
                    "sync_from: path traversal ('..') not allowed".to_string(),
                ));
            }
        }
        match other_db_path.extension().and_then(|e| e.to_str()) {
            Some("db") => {}
            _ => {
                return Err(rusqlite::Error::InvalidParameterName(
                    "sync_from: source must have .db extension".to_string(),
                ));
            }
        }
        if other_db_path.exists() && !other_db_path.is_file() {
            return Err(rusqlite::Error::InvalidParameterName(
                "sync_from: source path is not a regular file".to_string(),
            ));
        }
        let other_conn = Connection::open(other_db_path)?;
        let mut stmt = other_conn.prepare(
            "SELECT id, name, statement, domain, evidence, created_at \
             FROM axioms WHERE active = 1",
        )?;
        let remote_axioms: Vec<Axiom> = stmt
            .query_map([], |row| {
                Ok(Axiom {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    name: row.get(1)?,
                    statement: row.get(2)?,
                    domain: row.get(3)?,
                    evidence: row.get(4)?,
                    created_at: row
                        .get::<_, String>(5)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let mut synced = 0usize;
        for axiom in &remote_axioms {
            // Check if already present by statement text match
            let exists: bool = db
                .query_row(
                    "SELECT COUNT(*) FROM axioms WHERE statement = ?1",
                    params![axiom.statement],
                    |r| r.get::<_, i64>(0),
                )
                .map(|c| c > 0)
                .unwrap_or(false);
            if !exists {
                db.execute(
                    "INSERT INTO axioms (id, name, statement, domain, evidence, created_at, active) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
                    params![
                        Uuid::new_v4().to_string(),
                        axiom.name,
                        axiom.statement,
                        axiom.domain,
                        axiom.evidence,
                        axiom.created_at.to_rfc3339()
                    ],
                )?;
                synced += 1;
            }
        }
        Ok(synced)
    }
}
