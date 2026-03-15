pub mod a2a;
pub mod agent;
pub mod auto_memory;
pub mod axiom_culture;
pub mod channel_bridge;
pub mod context;
pub mod curiosity;
pub mod distillation;
pub mod embedding;
pub mod entity_extraction;
pub mod executor;
pub mod hermit_box;
pub mod identity;
pub mod knowledge_graph;
pub mod learning;
pub mod memory_stream;
pub mod orchestrator;
pub mod recall;
pub mod roles;
pub mod tools;
pub mod turn;
pub mod uocs;

pub use a2a::{A2AMessage, A2ARouter};
pub use agent::CognitiveAgent;
pub use auto_memory::{
    AutoMemoryConfig, auto_store_memories, auto_store_memories_guarded, extract_memories_from_turn,
};
pub use axiom_culture::{Axiom, AxiomCulture};
pub use channel_bridge::CognitiveChannelBridge;
pub use context::CognitiveContext;
pub use curiosity::{
    CuriosityConfig, CuriosityEngine, CycleReport, GapReport, KnowledgeGapDetector, ResearchQueue,
    ResearchTask, TaskPriority, TaskStatus, sanitize_display_topic, start_curiosity_loop,
};
pub use distillation::{TribeContext, distill_for_worker};
#[cfg(feature = "fastembed")]
pub use embedding::FastEmbedProvider;
pub use embedding::{EmbeddingProvider, NullEmbedder, TfIdfEmbedder};
pub use entity_extraction::{ExtractedTriplet, extract_and_store, extract_triplets};
pub use executor::{ExecutorError, WorkerExecutor};
pub use hermit_box::HermitBox;
pub use identity::AgentIdentityFiles;
pub use knowledge_graph::{EntityLayer, KgEdge, KgEntity, KnowledgeGraph};
pub use learning::LearningLoop;
pub use memory_stream::{
    MemoryEntry, MemoryKind, MemoryStream, blob_to_f32_vec, cosine_similarity, f32_vec_to_blob,
    namespaces,
};
pub use orchestrator::{TribeOrchestrator, WorkerJob, WorkerJobStatus, store_worker_result};
pub use recall::{RecallResult, RecallSource, fan_out_recall};
pub use roles::{AgentIdentity, AgentRole};
pub use tools::{
    CognitiveKgSearchTool, CognitiveRecallTool, CognitiveRememberTool, register_cognitive_tools,
};
pub use turn::{CognitiveTurnContext, format_cognitive_prefix, prepare_cognitive_context};
pub use uocs::UocsWriter;
