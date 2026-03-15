//! Cognitive memory tools for the agent.
//!
//! Three tools that integrate the cognitive layer with the tool system:
//! - `cognitive_remember`: Store a memory for future recall
//! - `cognitive_recall`: Search and retrieve past memories
//! - `cognitive_kg_search`: Search the knowledge graph by entity name

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::cognitive::context::CognitiveContext;
use crate::cognitive::embedding::NullEmbedder;
use crate::cognitive::memory_stream::{MemoryEntry, MemoryKind};
use crate::cognitive::recall::{RecallSource, fan_out_recall};
use crate::context::JobContext;
use crate::tools::{Tool, ToolError, ToolOutput, ToolRegistry, require_str};

// ─── CognitiveRememberTool ──────────────────────────────────────────────────

/// Store an important memory for future recall across sessions.
pub struct CognitiveRememberTool {
    ctx: Arc<CognitiveContext>,
}

impl CognitiveRememberTool {
    pub fn new(ctx: Arc<CognitiveContext>) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl Tool for CognitiveRememberTool {
    fn name(&self) -> &str {
        "cognitive_remember"
    }

    fn description(&self) -> &str {
        "Store an important memory for future recall across sessions"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The memory content to store"
                },
                "kind": {
                    "type": "string",
                    "description": "Memory kind: decision, lesson, person, project, commitment, preference, handoff, or observation (default: observation)"
                },
                "importance": {
                    "type": "number",
                    "description": "Importance score 0-10 (default: 5.0)"
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let content = require_str(&params, "content")?;
        let kind_str = params
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("observation");
        let importance = params
            .get("importance")
            .and_then(|v| v.as_f64())
            .unwrap_or(5.0) as f32;

        let kind = MemoryKind::from_str(kind_str);

        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            content: content.to_string(),
            kind,
            importance,
            created_at: Utc::now(),
            namespace: crate::cognitive::memory_stream::namespaces::PERSONAL.to_string(),
        };

        let id = entry.id;

        let ms = self
            .ctx
            .memory_stream
            .lock()
            .map_err(|e| ToolError::ExecutionFailed(format!("lock error: {e}")))?;

        // Auto-embed if an embedding provider is available (prevents ghost entries)
        if let Some(ref provider) = self.ctx.embedding_provider {
            ms.add_with_auto_embed(&entry, provider.as_ref())
                .map_err(|e| ToolError::ExecutionFailed(format!("memory store error: {e}")))?;
        } else {
            ms.add(&entry)
                .map_err(|e| ToolError::ExecutionFailed(format!("memory store error: {e}")))?;
        }

        Ok(ToolOutput::text(
            format!("Memory stored: {id}"),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ─── CognitiveRecallTool ────────────────────────────────────────────────────

/// Search and retrieve past memories by relevance.
pub struct CognitiveRecallTool {
    ctx: Arc<CognitiveContext>,
}

impl CognitiveRecallTool {
    pub fn new(ctx: Arc<CognitiveContext>) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl Tool for CognitiveRecallTool {
    fn name(&self) -> &str {
        "cognitive_recall"
    }

    fn description(&self) -> &str {
        "Search and retrieve past memories by relevance"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query for memory recall"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of memories to return (default: 5)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let query = require_str(&params, "query")?;
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

        let null_embedder = NullEmbedder;
        let embedder: &dyn crate::cognitive::embedding::EmbeddingProvider =
            match self.ctx.embedding_provider.as_ref() {
                Some(ep) => ep.as_ref(),
                None => &null_embedder,
            };

        let results = fan_out_recall(
            query,
            embedder,
            &self.ctx.memory_stream,
            &self.ctx.knowledge_graph,
            limit,
        );

        if results.is_empty() {
            return Ok(ToolOutput::text("No memories found.", start.elapsed()));
        }

        let lines: Vec<String> = results
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let timestamp = r
                    .metadata
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let date_part = if timestamp.len() >= 16 {
                    &timestamp[..16]
                } else {
                    timestamp
                };
                let kind_or_entity = match r.source {
                    RecallSource::MemoryStream => {
                        let kind = r
                            .metadata
                            .get("kind")
                            .and_then(|v| v.as_str())
                            .unwrap_or("observation");
                        format!("memory/{kind}")
                    }
                    RecallSource::KnowledgeGraph | RecallSource::KgSemantic => {
                        let name = r
                            .metadata
                            .get("entity_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("entity");
                        format!("entity: {name}")
                    }
                    RecallSource::FullTextSearch => {
                        let kind = r
                            .metadata
                            .get("kind")
                            .and_then(|v| v.as_str())
                            .unwrap_or("observation");
                        format!("fts/{kind}")
                    }
                };
                let tag = r.source.tag();
                format!(
                    "{}. [{}] ({}) ({tag}) {}",
                    i + 1,
                    date_part,
                    kind_or_entity,
                    r.content
                )
            })
            .collect();

        Ok(ToolOutput::text(lines.join("\n"), start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ─── CognitiveKgSearchTool ─────────────────────────────────────────────────

/// Search the knowledge graph for entities by name.
pub struct CognitiveKgSearchTool {
    ctx: Arc<CognitiveContext>,
}

impl CognitiveKgSearchTool {
    pub fn new(ctx: Arc<CognitiveContext>) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl Tool for CognitiveKgSearchTool {
    fn name(&self) -> &str {
        "cognitive_kg_search"
    }

    fn description(&self) -> &str {
        "Search the knowledge graph for entities by name"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Entity name to search for (case-insensitive)"
                },
                "limit": {
                    "type": "number",
                    "description": "Maximum number of entities to return (default: 10)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let query = require_str(&params, "query")?;
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

        let kg = self
            .ctx
            .knowledge_graph
            .lock()
            .map_err(|e| ToolError::ExecutionFailed(format!("lock error: {e}")))?;
        let entities = kg
            .search_by_name(query, limit)
            .map_err(|e| ToolError::ExecutionFailed(format!("kg search error: {e}")))?;

        if entities.is_empty() {
            return Ok(ToolOutput::text("No entities found.", start.elapsed()));
        }

        let lines: Vec<String> = entities
            .iter()
            .map(|e| format!("{} ({}): {}", e.name, e.layer.as_str(), e.summary))
            .collect();

        Ok(ToolOutput::text(lines.join("\n"), start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ─── Registration ───────────────────────────────────────────────────────────

/// Register all cognitive tools with the given registry.
pub fn register_cognitive_tools(registry: &mut ToolRegistry, ctx: Arc<CognitiveContext>) {
    registry.register_sync(Arc::new(CognitiveRememberTool::new(Arc::clone(&ctx))));
    registry.register_sync(Arc::new(CognitiveRecallTool::new(Arc::clone(&ctx))));
    registry.register_sync(Arc::new(CognitiveKgSearchTool::new(ctx)));
    tracing::info!("Registered 3 cognitive tools");
}
