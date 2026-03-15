use std::path::Path;

use crate::cognitive::auto_memory::{AutoMemoryConfig, auto_store_memories};
use crate::cognitive::context::CognitiveContext;
use crate::cognitive::hermit_box::HermitBox;
use crate::cognitive::identity::AgentIdentityFiles;
use crate::cognitive::learning::LearningLoop;
use crate::cognitive::turn::{format_cognitive_prefix, prepare_cognitive_context};

/// Unified cognitive agent that bundles a HermitBox, identity files,
/// cognitive context, and auto-memory configuration.
///
/// This is the primary entry point for opening a persistent agent
/// folder-world and interacting with it.
pub struct CognitiveAgent {
    pub hermit_box: HermitBox,
    pub identity: AgentIdentityFiles,
    pub context: CognitiveContext,
    pub auto_memory_config: AutoMemoryConfig,
    pub learning_loop: LearningLoop,
}

impl CognitiveAgent {
    /// Open (or create) a cognitive agent from a base directory and agent ID.
    ///
    /// - Opens the HermitBox (which validates agent_id for path safety)
    /// - Loads AgentIdentityFiles from the box
    /// - Creates CognitiveContext sharing the box's subsystem handles
    /// - Returns Self with default AutoMemoryConfig
    pub fn open(base_dir: &Path, agent_id: &str) -> Result<Self, String> {
        let hermit_box = HermitBox::open(base_dir, agent_id)?;
        let identity = AgentIdentityFiles::load(&hermit_box);
        let context = CognitiveContext::from_hermit_box(&hermit_box);
        Ok(Self {
            hermit_box,
            identity,
            context,
            auto_memory_config: AutoMemoryConfig::default(),
            learning_loop: LearningLoop::default(),
        })
    }

    /// Build a system prompt prefix from identity + cognitive context.
    ///
    /// Returns identity block followed by cognitive context (recent memories,
    /// knowledge graph activations, guiding axioms). When called with no
    /// prior user message, cognitive context will contain just axioms and
    /// recent memories by recency.
    pub fn system_prompt_prefix(&self) -> String {
        let mut parts: Vec<String> = Vec::new();

        let id_block = self.identity.system_prompt_block();
        if !id_block.is_empty() {
            parts.push(id_block);
        }

        let cog_prefix = prepare_cognitive_context(&self.context, "");
        let formatted = format_cognitive_prefix(&cog_prefix);
        if !formatted.is_empty() {
            parts.push(formatted);
        }

        parts.join("\n\n---\n\n")
    }

    /// Post-turn processing: extract and store memories from assistant response,
    /// then check if the learning loop should trigger reflection.
    ///
    /// Silently skips memory storage if the memory stream lock is poisoned,
    /// rather than panicking the process.
    pub fn post_turn(&self, assistant_response: &str) {
        let ms = match self.context.memory_stream.lock() {
            Ok(guard) => guard,
            Err(_) => {
                eprintln!(
                    "[CognitiveAgent] warning: memory_stream lock poisoned, skipping post_turn"
                );
                return;
            }
        };
        auto_store_memories(&ms, assistant_response, &self.auto_memory_config);

        // Check if learning loop should trigger reflection
        if self.learning_loop.record_turn() {
            let recent = ms.recent(20).unwrap_or_default();
            drop(ms); // release lock before writing telos files
            if let Err(e) = self.learning_loop.reflect(&self.hermit_box, &recent) {
                eprintln!("[CognitiveAgent] warning: learning reflection failed: {e}");
            }
        }
    }

    /// Get the agent's ID.
    pub fn agent_id(&self) -> &str {
        &self.hermit_box.agent_id
    }

    /// Return a one-line status summary for HTTP headers / UI display.
    ///
    /// Format: `agent:{id} memories:{count} queue:{pending}`
    pub fn status_summary(&self) -> String {
        let memory_count = self
            .context
            .memory_stream
            .lock()
            .ok()
            .map(|ms| ms.count())
            .unwrap_or(0);

        let queue_count =
            crate::cognitive::curiosity::ResearchQueue::open_for_hermit_box(&self.hermit_box)
                .ok()
                .and_then(|q| q.peek(1000).ok())
                .map(|tasks| tasks.len())
                .unwrap_or(0);

        format!(
            "agent:{} memories:{} queue:{}",
            self.hermit_box.agent_id, memory_count, queue_count
        )
    }
}
