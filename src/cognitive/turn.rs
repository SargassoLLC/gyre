use crate::cognitive::context::CognitiveContext;
use crate::cognitive::{Axiom, MemoryEntry};

/// Maximum size of the formatted cognitive prefix (8 KB).
/// Prevents an oversized system prompt from consuming excessive LLM context tokens
/// or causing request failures. If the formatted prefix exceeds this limit it is
/// truncated on a char boundary with an ellipsis marker.
const MAX_COGNITIVE_PREFIX_BYTES: usize = 8 * 1024;

/// Output of cognitive pre-processing for a single turn.
pub struct CognitiveTurnContext {
    pub memories: Vec<MemoryEntry>,
    pub kg_context: String,
    pub axioms: Vec<Axiom>,
}

/// Gather cognitive context relevant to the user's message.
///
/// - Recalls recent memories (recency-based; caller can use embedding variant separately)
/// - Activates knowledge graph nodes from keywords in the message
/// - Loads active axioms
pub fn prepare_cognitive_context(
    ctx: &CognitiveContext,
    user_message: &str,
) -> CognitiveTurnContext {
    // Memory recall (recency, limit 5)
    let memories = match ctx.memory_stream.lock() {
        Ok(ms) => ms.recall(user_message, 5).unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    // Extract keywords: words longer than 4 chars
    let keywords: Vec<&str> = user_message
        .split_whitespace()
        .filter(|w| w.len() > 4)
        .collect();

    let kg_context = match ctx.knowledge_graph.lock() {
        Ok(kg) => kg.activated_context_string(&keywords).unwrap_or_default(),
        Err(_) => String::new(),
    };

    // Active axioms (all domains)
    let axioms = match ctx.axiom_culture.lock() {
        Ok(ac) => ac.get_axioms(None).unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    CognitiveTurnContext {
        memories,
        kg_context,
        axioms,
    }
}

/// Format cognitive context into a string block suitable for prepending to the system prompt.
pub fn format_cognitive_prefix(ctx: &CognitiveTurnContext) -> String {
    let mut sections: Vec<String> = Vec::new();

    // Recent Memories
    if !ctx.memories.is_empty() {
        let mut s = String::from("### Recent Memories\n");
        for m in &ctx.memories {
            s.push_str(&format!("- [{}] {}\n", m.kind.as_str(), m.content));
        }
        sections.push(s);
    }

    // Knowledge Graph context
    if !ctx.kg_context.is_empty() {
        let mut s = String::from("### Knowledge\n");
        s.push_str(&ctx.kg_context);
        s.push('\n');
        sections.push(s);
    }

    // Guiding Axioms
    if !ctx.axioms.is_empty() {
        let mut s = String::from("### Guiding Axioms\n");
        for a in &ctx.axioms {
            s.push_str(&format!("- {}: {}\n", a.name, a.statement));
        }
        sections.push(s);
    }

    if sections.is_empty() {
        return String::new();
    }

    let full = format!("## Cognitive Context\n\n{}", sections.join("\n"));

    // Enforce size cap to prevent oversized system prompts
    if full.len() > MAX_COGNITIVE_PREFIX_BYTES {
        let mut end = MAX_COGNITIVE_PREFIX_BYTES.saturating_sub(4); // room for "...\n"
        while !full.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        let mut truncated = full[..end].to_string();
        truncated.push_str("...\n");
        return truncated;
    }

    full
}
