//! Session ingestion hook — extracts entity-relationship triplets from
//! conversation messages at session end and stores them in the knowledge graph.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use crate::cognitive::entity_extraction::extract_triplets;
use crate::cognitive::knowledge_graph::{EntityLayer, KgEdge, KnowledgeGraph};
use crate::db::Database;
use crate::error::LlmError;
use crate::hooks::hook::{
    Hook, HookContext, HookError, HookEvent, HookFailureMode, HookOutcome, HookPoint,
};
use crate::llm::LlmProvider;

/// Maximum characters per chunk sent to the LLM for entity extraction.
const CHUNK_SIZE: usize = 2000;

/// Minimum confidence threshold for storing extracted triplets.
const MIN_CONFIDENCE: f32 = 0.5;

/// Hook that ingests conversation content at session end, extracts entity-relationship
/// triplets via LLM, and stores them in the knowledge graph.
pub struct SessionIngestionHook {
    db: Arc<dyn Database>,
    llm: Arc<dyn LlmProvider>,
    kg: Arc<Mutex<KnowledgeGraph>>,
    model: String,
}

impl SessionIngestionHook {
    /// Create a new session ingestion hook.
    pub fn new(
        db: Arc<dyn Database>,
        llm: Arc<dyn LlmProvider>,
        kg: Arc<Mutex<KnowledgeGraph>>,
        model: String,
    ) -> Self {
        Self { db, llm, kg, model }
    }
}

#[async_trait]
impl Hook for SessionIngestionHook {
    fn name(&self) -> &str {
        "session-ingestion"
    }

    fn hook_points(&self) -> &[HookPoint] {
        &[HookPoint::OnSessionEnd]
    }

    fn failure_mode(&self) -> HookFailureMode {
        HookFailureMode::FailOpen
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(120)
    }

    async fn execute(
        &self,
        event: &HookEvent,
        _ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        let thread_ids = match event {
            HookEvent::SessionEnd { thread_ids, .. } => thread_ids,
            _ => return Ok(HookOutcome::ok()),
        };

        if thread_ids.is_empty() {
            return Ok(HookOutcome::ok());
        }

        let mut total_triplets = 0usize;

        for thread_id in thread_ids {
            // Fetch persisted messages from the database.
            let messages = match self.db.list_conversation_messages(*thread_id).await {
                Ok(msgs) => msgs,
                Err(e) => {
                    tracing::warn!(
                        thread_id = %thread_id,
                        "session-ingestion: failed to load messages: {e}"
                    );
                    continue;
                }
            };

            if messages.is_empty() {
                continue;
            }

            // Concatenate messages into text, then chunk on message boundaries.
            let chunks = chunk_messages(&messages);

            for chunk in &chunks {
                match extract_and_store_split(&*self.llm, &self.kg, chunk, &self.model).await {
                    Ok(count) => total_triplets += count,
                    Err(e) => {
                        tracing::warn!(
                            thread_id = %thread_id,
                            "session-ingestion: extraction failed: {e}"
                        );
                        // Continue with remaining chunks — fail-open per chunk.
                    }
                }
            }
        }

        if total_triplets > 0 {
            tracing::info!(
                "session-ingestion: extracted {total_triplets} triplet(s) from {} thread(s)",
                thread_ids.len()
            );
        }

        Ok(HookOutcome::ok())
    }
}

/// Concatenate conversation messages into chunks of approximately `CHUNK_SIZE`
/// characters, splitting only on message boundaries (never mid-message).
fn chunk_messages(messages: &[crate::history::ConversationMessage]) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for msg in messages {
        let line = format!("{}: {}\n", msg.role, msg.content);

        // If adding this message would exceed the limit and we already have content,
        // flush the current chunk first.
        if !current.is_empty() && current.len() + line.len() > CHUNK_SIZE {
            chunks.push(std::mem::take(&mut current));
        }

        current.push_str(&line);
    }

    // Don't forget the last chunk.
    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

/// Extract triplets via LLM (async), then lock the KG only for the synchronous
/// storage step. This avoids holding a `std::sync::Mutex` across await points.
async fn extract_and_store_split(
    provider: &dyn LlmProvider,
    kg: &Mutex<KnowledgeGraph>,
    text: &str,
    model: &str,
) -> Result<usize, LlmError> {
    // Phase 1: async LLM call — no lock held.
    let triplets = extract_triplets(provider, text, model).await?;

    if triplets.is_empty() {
        return Ok(0);
    }

    // Phase 2: synchronous KG writes under the lock.
    let kg_guard = kg.lock().map_err(|e| LlmError::RequestFailed {
        provider: "session-ingestion".to_string(),
        reason: format!("KG lock poisoned: {e}"),
    })?;

    let mut stored = 0usize;
    for triplet in &triplets {
        if triplet.confidence < MIN_CONFIDENCE {
            continue;
        }

        let subject_id = kg_guard
            .upsert_by_name(&triplet.subject, EntityLayer::Research, "", 5.0)
            .map_err(|e| LlmError::RequestFailed {
                provider: "session-ingestion".to_string(),
                reason: format!("KG upsert subject failed: {e}"),
            })?;

        let object_id = kg_guard
            .upsert_by_name(&triplet.object, EntityLayer::Research, "", 5.0)
            .map_err(|e| LlmError::RequestFailed {
                provider: "session-ingestion".to_string(),
                reason: format!("KG upsert object failed: {e}"),
            })?;

        let edge = KgEdge {
            id: uuid::Uuid::new_v4(),
            from_id: subject_id,
            to_id: object_id,
            relationship: triplet.relationship.clone(),
            weight: triplet.confidence,
            created_at: chrono::Utc::now(),
        };

        kg_guard
            .add_edge(&edge)
            .map_err(|e| LlmError::RequestFailed {
                provider: "session-ingestion".to_string(),
                reason: format!("KG add_edge failed: {e}"),
            })?;

        stored += 1;
    }

    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a fake ConversationMessage for testing.
    fn msg(role: &str, content: &str) -> crate::history::ConversationMessage {
        crate::history::ConversationMessage {
            id: uuid::Uuid::new_v4(),
            role: role.to_string(),
            content: content.to_string(),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_chunk_messages_single_small_message() {
        let messages = vec![msg("user", "hello world")];
        let chunks = chunk_messages(&messages);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("user: hello world"));
    }

    #[test]
    fn test_chunk_messages_empty() {
        let chunks = chunk_messages(&[]);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_messages_splits_on_boundary() {
        // Create messages that collectively exceed CHUNK_SIZE.
        let long_content = "x".repeat(1500);
        let messages = vec![msg("user", &long_content), msg("assistant", &long_content)];
        let chunks = chunk_messages(&messages);
        // Each message is ~1506 chars ("user: " + 1500 + "\n"), so they should split.
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("user: "));
        assert!(chunks[1].contains("assistant: "));
    }

    #[test]
    fn test_chunk_messages_keeps_small_together() {
        let messages = vec![
            msg("user", "hi"),
            msg("assistant", "hello"),
            msg("user", "how are you"),
        ];
        let chunks = chunk_messages(&messages);
        // All messages are tiny, should fit in one chunk.
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_chunk_messages_large_single_message_not_split() {
        // A single message larger than CHUNK_SIZE should still be a single chunk
        // (we never split mid-message).
        let huge = "y".repeat(5000);
        let messages = vec![msg("user", &huge)];
        let chunks = chunk_messages(&messages);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].len() > CHUNK_SIZE);
    }

    #[test]
    fn test_hook_metadata() {
        // Verify the hook returns correct metadata without needing real deps.
        // We can't construct SessionIngestionHook without real deps, so just
        // verify the constants.
        assert_eq!(CHUNK_SIZE, 2000);
        assert!((MIN_CONFIDENCE - 0.5).abs() < f32::EPSILON);
    }
}
