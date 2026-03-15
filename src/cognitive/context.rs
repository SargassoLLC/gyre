use std::sync::{Arc, Mutex};

use crate::cognitive::embedding::EmbeddingProvider;
use crate::cognitive::{AxiomCulture, HermitBox, KnowledgeGraph, MemoryStream};

/// Shared cognitive state for the agent.
///
/// Holds thread-safe handles to all cognitive subsystems.
/// Passed into the agent loop as an optional dependency.
/// Clone shares the same underlying data (all fields are `Arc`).
#[derive(Clone)]
pub struct CognitiveContext {
    pub memory_stream: Arc<Mutex<MemoryStream>>,
    pub knowledge_graph: Arc<Mutex<KnowledgeGraph>>,
    pub axiom_culture: Arc<Mutex<AxiomCulture>>,
    pub hermit_box: Arc<HermitBox>,
    pub embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl CognitiveContext {
    /// Build from individual subsystems (useful for tests with `:memory:` DBs).
    pub fn new(
        memory_stream: MemoryStream,
        knowledge_graph: KnowledgeGraph,
        axiom_culture: AxiomCulture,
        hermit_box: HermitBox,
    ) -> Self {
        Self {
            memory_stream: Arc::new(Mutex::new(memory_stream)),
            knowledge_graph: Arc::new(Mutex::new(knowledge_graph)),
            axiom_culture: Arc::new(Mutex::new(axiom_culture)),
            hermit_box: Arc::new(hermit_box),
            embedding_provider: None,
        }
    }

    /// Build from a HermitBox, sharing its already-open subsystem handles.
    pub fn from_hermit_box(hermit_box: &HermitBox) -> Self {
        Self {
            memory_stream: Arc::clone(&hermit_box.memory_stream),
            knowledge_graph: Arc::clone(&hermit_box.knowledge_graph),
            axiom_culture: Arc::clone(&hermit_box.axiom_culture),
            hermit_box: Arc::new(HermitBox {
                agent_id: hermit_box.agent_id.clone(),
                box_dir: hermit_box.box_dir.clone(),
                memory_stream: Arc::clone(&hermit_box.memory_stream),
                knowledge_graph: Arc::clone(&hermit_box.knowledge_graph),
                axiom_culture: Arc::clone(&hermit_box.axiom_culture),
            }),
            embedding_provider: None,
        }
    }

    /// Build from an Arc-wrapped HermitBox, sharing all handles.
    pub fn from_hermit_box_arc(hermit_box: Arc<HermitBox>) -> Self {
        Self {
            memory_stream: Arc::clone(&hermit_box.memory_stream),
            knowledge_graph: Arc::clone(&hermit_box.knowledge_graph),
            axiom_culture: Arc::clone(&hermit_box.axiom_culture),
            hermit_box,
            embedding_provider: None,
        }
    }

    /// Set the embedding provider for semantic search in fan-out recall.
    pub fn with_embedding_provider(mut self, provider: Arc<dyn EmbeddingProvider>) -> Self {
        self.embedding_provider = Some(provider);
        self
    }
}
