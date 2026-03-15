//! CognitiveChannelBridge — bridges IncomingMessage → CognitiveAgent → OutgoingResponse.
//!
//! Provides the glue between the channel system's IncomingMessage type and the
//! cognitive agent's system prompt + LLM completion flow.

use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::channels::IncomingMessage;
use crate::cognitive::agent::CognitiveAgent;
use crate::cognitive::auto_memory::AutoMemoryConfig;
use crate::cognitive::turn::{format_cognitive_prefix, prepare_cognitive_context};
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider};

/// Maximum combined system prompt size (16 KB).  If the identity prefix +
/// per-turn cognitive prefix exceed this limit, the cognitive prefix is
/// truncated (identity is preserved) to prevent excessive LLM context
/// consumption or request failures from crafted long messages.
const MAX_SYSTEM_PROMPT_BYTES: usize = 16 * 1024;

/// Bridges channel messages to a CognitiveAgent's LLM completion loop.
///
/// Handles system prompt construction, per-turn cognitive context injection,
/// and post-turn memory storage.
pub struct CognitiveChannelBridge {
    pub agent: Arc<CognitiveAgent>,
    pub auto_memory_config: AutoMemoryConfig,
    /// Limits concurrent `process_message` calls to 1, preventing resource
    /// exhaustion if the bridge is driven by a fast producer (piped stdin,
    /// programmatic callers).
    in_flight: Semaphore,
}

impl CognitiveChannelBridge {
    /// Create a new bridge wrapping a shared CognitiveAgent.
    pub fn new(agent: Arc<CognitiveAgent>) -> Self {
        Self {
            agent,
            auto_memory_config: AutoMemoryConfig::default(),
            in_flight: Semaphore::new(1),
        }
    }

    /// Build the base system prompt from identity + cognitive context (no query).
    ///
    /// Combines the agent's identity block with cognitive context gathered using
    /// an empty query (recent memories by recency, all axioms).
    pub fn prepare_system_prompt(&self) -> String {
        self.agent.system_prompt_prefix()
    }

    /// Build per-turn cognitive context for a specific user message.
    ///
    /// Returns the formatted cognitive prefix with memories, KG activations,
    /// and axioms relevant to the given query.
    pub fn prepare_turn_context(&self, user_message: &str) -> String {
        let cog_ctx = prepare_cognitive_context(&self.agent.context, user_message);
        format_cognitive_prefix(&cog_ctx)
    }

    /// Post-turn processing: store memories from the assistant response.
    ///
    /// If the response is substantial (>100 chars), also stores the user message
    /// as context in the memory stream.
    pub fn post_turn(&self, user_message: &str, assistant_response: &str) {
        self.agent.post_turn(assistant_response);

        // If response seems important, also store the user message as context
        if assistant_response.len() > 100 {
            if let Ok(ms) = self.agent.context.memory_stream.lock() {
                let entry = crate::cognitive::memory_stream::MemoryEntry {
                    id: uuid::Uuid::new_v4(),
                    content: user_message.to_string(),
                    kind: crate::cognitive::memory_stream::MemoryKind::Observation,
                    importance: 3.0,
                    created_at: chrono::Utc::now(),
                    namespace: crate::cognitive::memory_stream::namespaces::PERSONAL.to_string(),
                };
                let _ = ms.add(&entry);
            }
        }
    }

    /// Process an incoming channel message through the cognitive agent.
    ///
    /// 1. Builds system prompt (identity + base cognitive context + turn context)
    /// 2. Sends to LLM via CompletionRequest
    /// 3. Runs post-turn memory storage
    /// 4. Returns the assistant's response text
    pub async fn process_message(
        &self,
        message: &IncomingMessage,
        llm: &dyn LlmProvider,
    ) -> Result<String, String> {
        // Acquire in-flight permit (max 1 concurrent LLM call)
        let _permit = self
            .in_flight
            .acquire()
            .await
            .map_err(|e| format!("in-flight semaphore closed: {e}"))?;

        // 1. Build system prompt: base prefix + turn-specific context
        let base_system = self.prepare_system_prompt();
        let turn_context = self.prepare_turn_context(&message.content);

        let system = if turn_context.is_empty() {
            base_system
        } else {
            let combined = format!("{}\n\n{}", base_system, turn_context);
            // Enforce combined size cap: if over limit, truncate the cognitive
            // prefix (keep identity intact, drop oldest memories first).
            if combined.len() > MAX_SYSTEM_PROMPT_BYTES {
                let budget = MAX_SYSTEM_PROMPT_BYTES.saturating_sub(base_system.len() + 6);
                if budget == 0 {
                    // Identity alone fills the budget — drop cognitive prefix entirely
                    base_system.clone()
                } else {
                    let mut end = budget.min(turn_context.len());
                    while end > 0 && !turn_context.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}\n\n{}...", base_system, &turn_context[..end])
                }
            } else {
                combined
            }
        };

        // 2. Build completion request
        let mut messages = Vec::new();
        if !system.is_empty() {
            messages.push(ChatMessage::system(&system));
        }
        messages.push(ChatMessage::user(&message.content));

        let request = CompletionRequest::new(messages)
            .with_max_tokens(4096)
            .with_temperature(0.7);

        // 3. Call LLM
        let response = llm
            .complete(request)
            .await
            .map_err(|e| format!("LLM completion failed: {e}"))?;

        // 4. Post-turn processing
        self.post_turn(&message.content, &response.content);

        Ok(response.content)
    }
}
