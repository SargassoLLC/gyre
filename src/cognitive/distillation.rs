use crate::cognitive::context::CognitiveContext;
use crate::cognitive::memory_stream::{MemoryEntry, namespaces};

/// Distilled context passed from a Chief to a Worker for task delegation.
pub struct TribeContext {
    /// Top-5 tribe memories relevant to the task.
    pub memories: Vec<MemoryEntry>,
    /// Spreading activation output from the knowledge graph.
    pub kg_context: String,
    /// Active axiom statement texts.
    pub axioms: Vec<String>,
    /// Description of what the worker is being asked to do.
    pub task_hint: String,
}

impl TribeContext {
    /// Format this context into a system prompt block for the worker.
    pub fn system_prompt_block(&self) -> String {
        let mut sections = Vec::new();

        sections.push(format!("### Task\n{}", self.task_hint));

        if !self.memories.is_empty() {
            let mut s = String::from("### Relevant Memories\n");
            for m in &self.memories {
                s.push_str(&format!("- [{}] {}\n", m.kind.as_str(), m.content));
            }
            sections.push(s);
        }

        if !self.kg_context.is_empty() {
            let mut s = String::from("### Knowledge\n");
            s.push_str(&self.kg_context);
            s.push('\n');
            sections.push(s);
        }

        if !self.axioms.is_empty() {
            let mut s = String::from("### Guiding Axioms\n");
            for a in &self.axioms {
                s.push_str(&format!("- {}\n", a));
            }
            sections.push(s);
        }

        format!("## Delegated Context\n\n{}", sections.join("\n"))
    }
}

/// Distill a CognitiveContext into a TribeContext for worker delegation.
///
/// - Recalls up to 5 tribe-namespace memories relevant to the task (NOT personal)
/// - Activates knowledge graph from task keywords
/// - Gathers all active axiom texts
///
/// **Security:** Only tribe-namespace memories are included. Personal memories
/// are never leaked to Workers.
pub fn distill_for_worker(ctx: &CognitiveContext, task: &str) -> TribeContext {
    // Recall ONLY tribe-namespace memories — never personal
    let memories = ctx
        .memory_stream
        .lock()
        .ok()
        .and_then(|ms| ms.recall_in_namespace(namespaces::TRIBE, 5).ok())
        .unwrap_or_default();

    // Extract keywords (words > 4 chars) for KG activation
    let keywords: Vec<&str> = task.split_whitespace().filter(|w| w.len() > 4).collect();

    let kg_context = ctx
        .knowledge_graph
        .lock()
        .ok()
        .and_then(|kg| kg.activated_context_string(&keywords).ok())
        .unwrap_or_default();

    // Active axioms as text strings
    let axioms = ctx
        .axiom_culture
        .lock()
        .ok()
        .map(|ac| {
            ac.list_active()
                .unwrap_or_default()
                .into_iter()
                .map(|a| format!("{}: {}", a.name, a.statement))
                .collect()
        })
        .unwrap_or_default();

    TribeContext {
        memories,
        kg_context,
        axioms,
        task_hint: task.to_string(),
    }
}
