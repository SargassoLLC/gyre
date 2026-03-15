//! Curiosity Engine — autonomous research queue, knowledge gap detection, and
//! background exploration loop.  Gyre curiosity exploration engine.

use chrono::{DateTime, Utc};
use futures::FutureExt;
use rusqlite::{Connection, params};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::cognitive::agent::CognitiveAgent;
use crate::cognitive::hermit_box::HermitBox;
use crate::cognitive::knowledge_graph::{KgEntity, KnowledgeGraph};
use crate::cognitive::orchestrator::TribeOrchestrator;
use crate::llm::LlmProvider;

// ─── ResearchTask ────────────────────────────────────────────────────────────

/// Maximum topic string length (bytes). Longer topics are truncated on push.
const MAX_TOPIC_LEN: usize = 500;

/// Maximum retry attempts before a task is permanently marked `Exhausted`.
const MAX_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Done,
    Failed,
    /// Permanently failed — exceeded MAX_ATTEMPTS retries.
    Exhausted,
}

impl TaskStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Exhausted => "exhausted",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "in_progress" => Self::InProgress,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "exhausted" => Self::Exhausted,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaskPriority {
    Low = 1,
    Medium = 5,
    High = 8,
    Critical = 10,
}

#[derive(Debug, Clone)]
pub struct ResearchTask {
    pub id: Uuid,
    pub topic: String,
    pub priority: f32,
    pub source: String,
    pub status: TaskStatus,
    pub attempts: u32,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub failure_reason: Option<String>,
}

// ─── ResearchQueue ───────────────────────────────────────────────────────────

pub struct ResearchQueue {
    db: Arc<Mutex<Connection>>,
}

impl ResearchQueue {
    pub fn new(db_path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS research_tasks (
                id TEXT PRIMARY KEY,
                topic TEXT NOT NULL,
                priority REAL NOT NULL DEFAULT 5.0,
                source TEXT NOT NULL DEFAULT 'manual',
                status TEXT NOT NULL DEFAULT 'pending',
                attempts INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                completed_at TEXT,
                failure_reason TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_rt_status ON research_tasks(status);
            CREATE INDEX IF NOT EXISTS idx_rt_priority ON research_tasks(priority);",
        )?;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Insert a new research task. Returns the task ID.
    /// Deduplicates by topic: skips if a Pending or InProgress task with the
    /// same topic already exists.  Priority is clamped to 1.0–10.0.
    /// Topic is truncated to MAX_TOPIC_LEN bytes.
    pub fn push(&self, topic: &str, priority: f32, source: &str) -> rusqlite::Result<Uuid> {
        // Truncate topic to MAX_TOPIC_LEN bytes on a char boundary.
        let topic = truncate_to_byte_boundary(topic, MAX_TOPIC_LEN);

        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;

        // Dedup: skip if same topic already pending or in_progress
        let existing: i64 = db.query_row(
            "SELECT COUNT(*) FROM research_tasks WHERE topic = ?1 AND status IN ('pending', 'in_progress')",
            params![topic],
            |row| row.get(0),
        )?;
        if existing > 0 {
            // Return the existing task's ID
            let id_str: String = db.query_row(
                "SELECT id FROM research_tasks WHERE topic = ?1 AND status IN ('pending', 'in_progress') LIMIT 1",
                params![topic],
                |row| row.get(0),
            )?;
            return Ok(Uuid::parse_str(&id_str).unwrap_or_default());
        }

        let id = Uuid::new_v4();
        let clamped_priority = priority.clamp(1.0, 10.0);
        let now = Utc::now().to_rfc3339();

        db.execute(
            "INSERT INTO research_tasks (id, topic, priority, source, status, attempts, created_at) \
             VALUES (?1, ?2, ?3, ?4, 'pending', 0, ?5)",
            params![id.to_string(), topic, clamped_priority, source, now],
        )?;
        Ok(id)
    }

    /// Pop the highest-priority Pending task, mark it InProgress, and return it.
    pub fn pop_next(&self) -> rusqlite::Result<Option<ResearchTask>> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;

        let task = {
            let mut stmt = db.prepare(
                "SELECT id, topic, priority, source, status, attempts, created_at, completed_at, failure_reason \
                 FROM research_tasks WHERE status = 'pending' ORDER BY priority DESC LIMIT 1",
            )?;
            let mut rows = stmt.query_map([], |row| {
                Ok(ResearchTask {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    topic: row.get(1)?,
                    priority: row.get(2)?,
                    source: row.get(3)?,
                    status: TaskStatus::from_str(&row.get::<_, String>(4)?),
                    attempts: row.get(5)?,
                    created_at: row
                        .get::<_, String>(6)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    completed_at: row
                        .get::<_, Option<String>>(7)?
                        .and_then(|s| s.parse().ok()),
                    failure_reason: row.get(8)?,
                })
            })?;
            rows.next().and_then(|r| r.ok())
        };

        if let Some(ref t) = task {
            db.execute(
                "UPDATE research_tasks SET status = 'in_progress' WHERE id = ?1",
                params![t.id.to_string()],
            )?;
        }

        Ok(task.map(|mut t| {
            t.status = TaskStatus::InProgress;
            t
        }))
    }

    /// Peek at the top N pending tasks ordered by priority descending.
    pub fn peek(&self, limit: usize) -> rusqlite::Result<Vec<ResearchTask>> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let mut stmt = db.prepare(
            "SELECT id, topic, priority, source, status, attempts, created_at, completed_at, failure_reason \
             FROM research_tasks WHERE status = 'pending' ORDER BY priority DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(ResearchTask {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                    topic: row.get(1)?,
                    priority: row.get(2)?,
                    source: row.get(3)?,
                    status: TaskStatus::from_str(&row.get::<_, String>(4)?),
                    attempts: row.get(5)?,
                    created_at: row
                        .get::<_, String>(6)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    completed_at: row
                        .get::<_, Option<String>>(7)?
                        .and_then(|s| s.parse().ok()),
                    failure_reason: row.get(8)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Mark a task as done with a completion timestamp.
    pub fn mark_done(&self, id: Uuid, completed_at: DateTime<Utc>) -> rusqlite::Result<()> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        db.execute(
            "UPDATE research_tasks SET status = 'done', completed_at = ?2 WHERE id = ?1",
            params![id.to_string(), completed_at.to_rfc3339()],
        )?;
        Ok(())
    }

    /// Mark a task as failed with a reason, and increment attempts.
    /// If attempts reaches MAX_ATTEMPTS, the task is permanently marked `exhausted`
    /// and will never be retried.
    pub fn mark_failed(&self, id: Uuid, reason: &str) -> rusqlite::Result<()> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;

        // Read current attempts to decide final status.
        let current_attempts: u32 = db.query_row(
            "SELECT attempts FROM research_tasks WHERE id = ?1",
            params![id.to_string()],
            |row| row.get(0),
        )?;

        let new_attempts = current_attempts + 1;
        let new_status = if new_attempts >= MAX_ATTEMPTS {
            "exhausted"
        } else {
            "failed"
        };

        db.execute(
            "UPDATE research_tasks SET status = ?3, failure_reason = ?2, attempts = ?4 WHERE id = ?1",
            params![id.to_string(), reason, new_status, new_attempts],
        )?;
        Ok(())
    }

    /// Count tasks completed since a given timestamp (for daily limit).
    pub fn daily_completed_count(&self, since: DateTime<Utc>) -> rusqlite::Result<u32> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let count: i64 = db.query_row(
            "SELECT COUNT(*) FROM research_tasks WHERE status = 'done' AND completed_at > ?1",
            params![since.to_rfc3339()],
            |row| row.get(0),
        )?;
        Ok(count as u32)
    }

    /// Open a ResearchQueue for an agent's HermitBox directory.
    ///
    /// Creates the `curiosity/` subdirectory and `queue.db` if needed.
    pub fn open_for_hermit_box(hermit_box: &HermitBox) -> rusqlite::Result<Self> {
        let curiosity_dir = hermit_box.box_dir.join("curiosity");
        std::fs::create_dir_all(&curiosity_dir).map_err(|e| {
            rusqlite::Error::InvalidParameterName(format!("failed to create curiosity dir: {e}"))
        })?;
        let queue_db_path = curiosity_dir.join("queue.db");
        Self::new(&queue_db_path)
    }

    /// Count pending tasks.
    pub fn pending_count(&self) -> rusqlite::Result<u32> {
        let db = self
            .db
            .lock()
            .map_err(|e| rusqlite::Error::InvalidParameterName(format!("lock poisoned: {e}")))?;
        let count: i64 = db.query_row(
            "SELECT COUNT(*) FROM research_tasks WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )?;
        Ok(count as u32)
    }
}

// ─── KnowledgeGapDetector ────────────────────────────────────────────────────

pub struct GapReport {
    pub isolated_entities: Vec<KgEntity>,
    pub stale_entities: Vec<KgEntity>,
    pub total_gaps: usize,
}

pub struct KnowledgeGapDetector;

impl KnowledgeGapDetector {
    /// Scan the knowledge graph for gaps: isolated high-importance entities and
    /// stale entities not updated in over 7 days.
    pub fn scan(kg: &KnowledgeGraph) -> rusqlite::Result<GapReport> {
        let isolated = kg.entities_with_few_edges(6.0, 2)?;
        let stale_cutoff = Utc::now() - chrono::Duration::days(7);
        let stale = kg.stale_entities(stale_cutoff)?;
        let total_gaps = isolated.len() + stale.len();
        Ok(GapReport {
            isolated_entities: isolated,
            stale_entities: stale,
            total_gaps,
        })
    }

    /// Convert a GapReport into research task tuples (topic, priority, source).
    pub fn to_research_tasks(report: &GapReport) -> Vec<(String, f32, String)> {
        let mut tasks = Vec::new();
        for entity in &report.isolated_entities {
            tasks.push((
                format!("Research and expand knowledge about: {}", entity.name),
                7.0,
                "gap_detector_isolated".to_string(),
            ));
        }
        for entity in &report.stale_entities {
            tasks.push((
                format!("Research and expand knowledge about: {}", entity.name),
                5.0,
                "gap_detector_stale".to_string(),
            ));
        }
        tasks
    }
}

// ─── CuriosityEngine ─────────────────────────────────────────────────────────

pub struct CuriosityConfig {
    pub max_daily_tasks: u32,
    pub max_concurrent: u32,
    pub cycle_interval_secs: u64,
}

impl Default for CuriosityConfig {
    fn default() -> Self {
        Self {
            max_daily_tasks: 20,
            max_concurrent: 2,
            cycle_interval_secs: 3600,
        }
    }
}

pub struct CycleReport {
    pub gaps_detected: usize,
    pub tasks_enqueued: usize,
    pub task_processed: Option<ResearchTask>,
    pub entities_added: usize,
    pub memories_stored: usize,
    pub skipped_daily_limit: bool,
}

pub struct CuriosityEngine {
    pub queue: ResearchQueue,
    pub config: CuriosityConfig,
}

impl CuriosityEngine {
    pub fn new(queue_db_path: &Path, config: CuriosityConfig) -> rusqlite::Result<Self> {
        let queue = ResearchQueue::new(queue_db_path)?;
        Ok(Self { queue, config })
    }

    /// Run a single curiosity cycle:
    /// 1. Check daily limit
    /// 2. Scan for knowledge gaps
    /// 3. Enqueue gap-derived tasks
    /// 4. Pop next task and execute via TribeOrchestrator
    pub async fn run_cycle(
        &self,
        chief: &CognitiveAgent,
        llm: &dyn LlmProvider,
    ) -> Result<CycleReport, String> {
        // 1. Check daily limit
        let today_start = Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
            .ok_or_else(|| "Failed to compute start of day".to_string())?;

        let daily_done = self
            .queue
            .daily_completed_count(today_start)
            .map_err(|e| format!("daily_completed_count failed: {e}"))?;

        if daily_done >= self.config.max_daily_tasks {
            return Ok(CycleReport {
                gaps_detected: 0,
                tasks_enqueued: 0,
                task_processed: None,
                entities_added: 0,
                memories_stored: 0,
                skipped_daily_limit: true,
            });
        }

        // 2. Gap scan (lock scoped to avoid holding across .await)
        let (gaps_detected, gap_tasks) = {
            let kg = chief
                .context
                .knowledge_graph
                .lock()
                .map_err(|e| format!("kg lock poisoned: {e}"))?;
            let gap_report =
                KnowledgeGapDetector::scan(&kg).map_err(|e| format!("gap scan failed: {e}"))?;
            let detected = gap_report.total_gaps;
            let tasks = KnowledgeGapDetector::to_research_tasks(&gap_report);
            (detected, tasks)
        };

        // 3. Enqueue gaps (dedup via push)
        let mut tasks_enqueued = 0;
        for (topic, priority, source) in &gap_tasks {
            match self.queue.push(topic, *priority, source) {
                Ok(_) => tasks_enqueued += 1,
                Err(e) => {
                    tracing::warn!("Failed to enqueue gap task: {e}");
                }
            }
        }

        // 4. Pop next task
        let task = self
            .queue
            .pop_next()
            .map_err(|e| format!("pop_next failed: {e}"))?;

        let task = match task {
            Some(t) => t,
            None => {
                return Ok(CycleReport {
                    gaps_detected,
                    tasks_enqueued,
                    task_processed: None,
                    entities_added: 0,
                    memories_stored: 0,
                    skipped_daily_limit: false,
                });
            }
        };

        // 5. Execute via TribeOrchestrator
        let kg_count_before = {
            let kg = chief
                .context
                .knowledge_graph
                .lock()
                .map_err(|e| format!("kg lock: {e}"))?;
            kg.entity_count().unwrap_or(0)
        };

        match TribeOrchestrator::execute(chief, &task.topic, llm).await {
            Ok(_job) => {
                // 6. Success
                self.queue
                    .mark_done(task.id, Utc::now())
                    .map_err(|e| format!("mark_done failed: {e}"))?;

                let kg_count_after = {
                    let kg = chief
                        .context
                        .knowledge_graph
                        .lock()
                        .map_err(|e| format!("kg lock: {e}"))?;
                    kg.entity_count().unwrap_or(0)
                };

                let entities_added = kg_count_after.saturating_sub(kg_count_before);

                let mut completed_task = task;
                completed_task.status = TaskStatus::Done;
                completed_task.completed_at = Some(Utc::now());

                Ok(CycleReport {
                    gaps_detected,
                    tasks_enqueued,
                    task_processed: Some(completed_task),
                    entities_added,
                    memories_stored: 1,
                    skipped_daily_limit: false,
                })
            }
            Err(error) => {
                // 7. Failure
                self.queue
                    .mark_failed(task.id, &error)
                    .map_err(|e| format!("mark_failed failed: {e}"))?;

                let mut failed_task = task;
                failed_task.status = TaskStatus::Failed;
                failed_task.failure_reason = Some(error);

                Ok(CycleReport {
                    gaps_detected,
                    tasks_enqueued,
                    task_processed: Some(failed_task),
                    entities_added: 0,
                    memories_stored: 0,
                    skipped_daily_limit: false,
                })
            }
        }
    }

    /// Open a CuriosityEngine for an agent using its HermitBox directory.
    /// Creates the `curiosity/` subdirectory if needed.
    pub fn open_for_agent(hermit_box: &HermitBox) -> rusqlite::Result<Self> {
        let curiosity_dir = hermit_box.box_dir.join("curiosity");
        std::fs::create_dir_all(&curiosity_dir).map_err(|e| {
            rusqlite::Error::InvalidParameterName(format!("failed to create curiosity dir: {e}"))
        })?;
        let queue_db_path = curiosity_dir.join("queue.db");
        Self::new(&queue_db_path, CuriosityConfig::default())
    }
}

// ─── Background curiosity ticker ─────────────────────────────────────────────

pub async fn start_curiosity_loop(
    engine: Arc<CuriosityEngine>,
    chief: Arc<CognitiveAgent>,
    llm: Arc<dyn LlmProvider>,
) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(
                engine.config.cycle_interval_secs,
            ))
            .await;

            // Wrap run_cycle in catch_unwind so a panic in one cycle
            // does not kill the background ticker permanently.
            let result = AssertUnwindSafe(engine.run_cycle(&chief, &*llm))
                .catch_unwind()
                .await;

            match result {
                Ok(Ok(report)) => {
                    if report.skipped_daily_limit {
                        tracing::info!("[Curiosity] Skipped: daily limit reached");
                    } else if let Some(ref task) = report.task_processed {
                        tracing::info!(
                            "[Curiosity] Cycle complete: gaps={}, enqueued={}, processed=\"{}\", entities_added={}, memories={}",
                            report.gaps_detected,
                            report.tasks_enqueued,
                            task.topic,
                            report.entities_added,
                            report.memories_stored,
                        );
                    } else {
                        tracing::info!(
                            "[Curiosity] Cycle complete: gaps={}, enqueued={}, no pending task to process",
                            report.gaps_detected,
                            report.tasks_enqueued,
                        );
                    }
                }
                Ok(Err(e)) => {
                    tracing::error!("[Curiosity] Cycle failed: {e}");
                }
                Err(panic_err) => {
                    let msg = panic_err
                        .downcast_ref::<String>()
                        .map(|s| s.as_str())
                        .or_else(|| panic_err.downcast_ref::<&str>().copied())
                        .unwrap_or("unknown panic");
                    tracing::error!("[Curiosity] Cycle panicked (recovered): {msg}");
                }
            }
        }
    });
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Truncate a string to at most `max_bytes` bytes on a UTF-8 char boundary.
fn truncate_to_byte_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Sanitize a topic string for terminal display: replace control chars and
/// newlines with spaces so the table layout is not broken.
pub fn sanitize_display_topic(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

// ─── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_status_roundtrip() {
        for status in [
            TaskStatus::Pending,
            TaskStatus::InProgress,
            TaskStatus::Done,
            TaskStatus::Failed,
            TaskStatus::Exhausted,
        ] {
            assert_eq!(TaskStatus::from_str(status.as_str()), status);
        }
    }

    #[test]
    fn test_priority_values() {
        assert_eq!(TaskPriority::Low as i32, 1);
        assert_eq!(TaskPriority::Medium as i32, 5);
        assert_eq!(TaskPriority::High as i32, 8);
        assert_eq!(TaskPriority::Critical as i32, 10);
    }

    #[test]
    fn test_queue_push_pop() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let queue = ResearchQueue::new(&tmp.path().join("q.db")).expect("new queue");

        let id1 = queue.push("topic-low", 2.0, "manual").expect("push");
        let id2 = queue.push("topic-high", 9.0, "manual").expect("push");
        let _id3 = queue.push("topic-med", 5.0, "manual").expect("push");

        assert_eq!(queue.pending_count().expect("count"), 3);

        // Pop returns highest priority first
        let next = queue.pop_next().expect("pop").expect("should have task");
        assert_eq!(next.id, id2);
        assert_eq!(next.status, TaskStatus::InProgress);

        assert_eq!(queue.pending_count().expect("count"), 2);

        // Clean up: also test mark_done
        queue.mark_done(id2, Utc::now()).expect("mark_done");

        // Next pop should return medium
        let next2 = queue.pop_next().expect("pop").expect("should have task");
        assert!(next2.priority > 1.5 && next2.priority < 6.0);

        // Mark it failed
        queue
            .mark_failed(next2.id, "test failure")
            .expect("mark_failed");

        // Last one
        let next3 = queue.pop_next().expect("pop").expect("should have task");
        assert_eq!(next3.id, id1);

        // Queue should be empty now
        assert!(queue.pop_next().expect("pop").is_none());
    }

    #[test]
    fn test_queue_dedup() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let queue = ResearchQueue::new(&tmp.path().join("q.db")).expect("new queue");

        let id1 = queue.push("same-topic", 5.0, "manual").expect("push");
        let id2 = queue.push("same-topic", 8.0, "manual").expect("push");

        // Second push returns existing ID, no new task created
        assert_eq!(id1, id2);
        assert_eq!(queue.pending_count().expect("count"), 1);
    }

    #[test]
    fn test_priority_clamping() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let queue = ResearchQueue::new(&tmp.path().join("q.db")).expect("new queue");

        queue.push("over-max", 999.0, "test").expect("push");
        queue.push("under-min", -5.0, "test").expect("push");

        let tasks = queue.peek(10).expect("peek");
        assert_eq!(tasks.len(), 2);
        // Highest priority (clamped to 10.0) comes first
        assert!((tasks[0].priority - 10.0).abs() < f32::EPSILON);
        assert!((tasks[1].priority - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_daily_completed_count() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let queue = ResearchQueue::new(&tmp.path().join("q.db")).expect("new queue");

        // Push and complete 3 tasks
        for i in 0..3 {
            let id = queue.push(&format!("task-{i}"), 5.0, "test").expect("push");
            let _ = queue.pop_next();
            queue.mark_done(id, Utc::now()).expect("mark_done");
        }

        let today_start = Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
            .expect("today start");

        let count = queue.daily_completed_count(today_start).expect("count");
        assert_eq!(count, 3);
    }

    #[test]
    fn test_peek() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let queue = ResearchQueue::new(&tmp.path().join("q.db")).expect("new queue");

        queue.push("a", 3.0, "test").expect("push");
        queue.push("b", 7.0, "test").expect("push");
        queue.push("c", 5.0, "test").expect("push");

        let peeked = queue.peek(2).expect("peek");
        assert_eq!(peeked.len(), 2);
        assert!(peeked[0].priority >= peeked[1].priority);
    }

    #[test]
    fn test_gap_detector_to_research_tasks() {
        let report = GapReport {
            isolated_entities: vec![KgEntity {
                id: Uuid::new_v4(),
                name: "TestEntity".to_string(),
                layer: crate::cognitive::knowledge_graph::EntityLayer::Research,
                summary: String::new(),
                importance: 8.0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
            stale_entities: vec![KgEntity {
                id: Uuid::new_v4(),
                name: "OldEntity".to_string(),
                layer: crate::cognitive::knowledge_graph::EntityLayer::Concept,
                summary: String::new(),
                importance: 5.0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
            total_gaps: 2,
        };

        let tasks = KnowledgeGapDetector::to_research_tasks(&report);
        assert_eq!(tasks.len(), 2);
        assert!(tasks[0].0.contains("TestEntity"));
        assert!((tasks[0].1 - 7.0).abs() < f32::EPSILON);
        assert_eq!(tasks[0].2, "gap_detector_isolated");
        assert!(tasks[1].0.contains("OldEntity"));
        assert!((tasks[1].1 - 5.0).abs() < f32::EPSILON);
        assert_eq!(tasks[1].2, "gap_detector_stale");
    }

    // ── Security tests ──────────────────────────────────────────────────────

    #[test]
    fn test_max_attempts_exhausts_task() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let queue = ResearchQueue::new(&tmp.path().join("q.db")).expect("new queue");

        let id = queue.push("will-exhaust", 5.0, "test").expect("push");
        let _ = queue.pop_next(); // move to in_progress

        // Fail it MAX_ATTEMPTS times
        for i in 1..=MAX_ATTEMPTS {
            queue
                .mark_failed(id, &format!("fail #{i}"))
                .expect("mark_failed");
        }

        // Read back the task — should be exhausted, not merely failed
        let db = queue.db.lock().expect("lock");
        let status: String = db
            .query_row(
                "SELECT status FROM research_tasks WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(status, "exhausted");

        let attempts: u32 = db
            .query_row(
                "SELECT attempts FROM research_tasks WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(attempts, MAX_ATTEMPTS);
    }

    #[test]
    fn test_topic_length_cap() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let queue = ResearchQueue::new(&tmp.path().join("q.db")).expect("new queue");

        let long_topic = "a".repeat(1000);
        queue.push(&long_topic, 5.0, "test").expect("push");

        let tasks = queue.peek(1).expect("peek");
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].topic.len() <= MAX_TOPIC_LEN);
    }

    #[test]
    fn test_topic_truncation_unicode_boundary() {
        // Ensure truncation doesn't split a multi-byte char
        let topic = "a".repeat(498) + "\u{1F600}\u{1F600}"; // 498 + 4 + 4 = 506 bytes
        let truncated = truncate_to_byte_boundary(&topic, MAX_TOPIC_LEN);
        assert!(truncated.len() <= MAX_TOPIC_LEN);
        // Should be valid UTF-8 (this won't compile/run if not)
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    #[test]
    fn test_sanitize_display_topic() {
        assert_eq!(sanitize_display_topic("normal topic"), "normal topic");
        assert_eq!(
            sanitize_display_topic("line1\nline2\ttab\r\0null"),
            "line1 line2 tab  null"
        );
        assert_eq!(sanitize_display_topic(""), "");
    }
}
