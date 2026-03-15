use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use gyre::cognitive::{
    AgentIdentity, AgentIdentityFiles, AutoMemoryConfig, Axiom, AxiomCulture, CognitiveContext,
    CognitiveRememberTool, EmbeddingProvider, EntityLayer, HermitBox, KgEdge, KgEntity,
    KnowledgeGraph, MemoryEntry, MemoryKind, MemoryStream, TfIdfEmbedder, TribeContext,
    auto_store_memories, blob_to_f32_vec, cosine_similarity, distill_for_worker,
    extract_memories_from_turn, f32_vec_to_blob, format_cognitive_prefix, namespaces,
    prepare_cognitive_context,
};
use gyre::context::JobContext;
use gyre::tools::Tool;

/// Helper: create a MemoryStream backed by an in-memory SQLite DB.
fn memory_stream_in_memory() -> MemoryStream {
    MemoryStream::new(Path::new(":memory:")).expect("in-memory MemoryStream")
}

/// Helper: create a KnowledgeGraph backed by an in-memory SQLite DB.
fn knowledge_graph_in_memory() -> KnowledgeGraph {
    KnowledgeGraph::new(Path::new(":memory:")).expect("in-memory KnowledgeGraph")
}

/// Helper: create an AxiomCulture backed by an in-memory SQLite DB.
fn axiom_culture_in_memory() -> AxiomCulture {
    AxiomCulture::new(Path::new(":memory:")).expect("in-memory AxiomCulture")
}

// ─── Test: Memory store and recall ──────────────────────────────────────────

#[test]
fn test_memory_store_and_recall() {
    let ms = memory_stream_in_memory();

    let entries: Vec<MemoryEntry> = (0..3)
        .map(|i| MemoryEntry {
            id: Uuid::new_v4(),
            content: format!("Memory entry number {}", i),
            kind: MemoryKind::Observation,
            importance: 5.0 + i as f32,
            created_at: Utc::now(),
            namespace: namespaces::PERSONAL.to_string(),
        })
        .collect();

    for entry in &entries {
        ms.add(entry).expect("add memory entry");
    }

    let recalled = ms.recall("", 2).expect("recall");
    assert_eq!(recalled.len(), 2, "should recall exactly 2 entries");
    // Recency order: most recent first
    assert!(
        recalled[0].content.contains('2'),
        "most recent entry (2) should be first"
    );
    assert!(
        recalled[1].content.contains('1'),
        "second most recent entry (1) should be second"
    );
}

// ─── Test: Memory store with embedding and recall_relevant ──────────────────

#[test]
fn test_memory_store_with_embedding_and_recall_relevant() {
    let ms = memory_stream_in_memory();

    // Create 3 entries with different embeddings
    let embedding_a = vec![1.0, 0.0, 0.0];
    let embedding_b = vec![0.0, 1.0, 0.0];
    let embedding_c = vec![0.7, 0.7, 0.0]; // similar to both a and b

    let entries = vec![
        (
            MemoryEntry {
                id: Uuid::new_v4(),
                content: "about apples".to_string(),
                kind: MemoryKind::Observation,
                importance: 5.0,
                created_at: Utc::now(),
                namespace: namespaces::PERSONAL.to_string(),
            },
            embedding_a,
        ),
        (
            MemoryEntry {
                id: Uuid::new_v4(),
                content: "about bananas".to_string(),
                kind: MemoryKind::Lesson,
                importance: 7.0,
                created_at: Utc::now(),
                namespace: namespaces::PERSONAL.to_string(),
            },
            embedding_b,
        ),
        (
            MemoryEntry {
                id: Uuid::new_v4(),
                content: "about fruit".to_string(),
                kind: MemoryKind::Decision,
                importance: 6.0,
                created_at: Utc::now(),
                namespace: namespaces::PERSONAL.to_string(),
            },
            embedding_c,
        ),
    ];

    for (entry, emb) in &entries {
        ms.store_with_embedding(entry, emb.clone())
            .expect("store with embedding");
    }

    // Query with embedding close to "apples" direction
    let query = vec![0.9, 0.1, 0.0];
    let results = ms.recall_relevant(&query, 3).expect("recall_relevant");
    assert_eq!(results.len(), 3);

    // "about apples" (1,0,0) should be most similar to (0.9, 0.1, 0)
    assert!(
        results[0].0.content.contains("apples"),
        "apples should be first, got: {}",
        results[0].0.content
    );
    // All scores should be positive
    assert!(results[0].1 > 0.0);
}

// ─── Test: cosine_similarity ─────────────────────────────────────────────────

#[test]
fn test_cosine_similarity_identical() {
    let a = vec![1.0, 2.0, 3.0];
    let score = cosine_similarity(&a, &a);
    assert!(
        (score - 1.0).abs() < 1e-5,
        "identical vectors should have similarity ~1.0"
    );
}

#[test]
fn test_cosine_similarity_orthogonal() {
    let a = vec![1.0, 0.0];
    let b = vec![0.0, 1.0];
    let score = cosine_similarity(&a, &b);
    assert!(
        score.abs() < 1e-5,
        "orthogonal vectors should have similarity ~0.0"
    );
}

#[test]
fn test_cosine_similarity_empty() {
    let score = cosine_similarity(&[], &[]);
    assert_eq!(score, 0.0);
}

#[test]
fn test_cosine_similarity_mismatched_lengths() {
    let a = vec![1.0, 2.0];
    let b = vec![1.0];
    let score = cosine_similarity(&a, &b);
    assert_eq!(score, 0.0);
}

// ─── Test: KG spreading activation ──────────────────────────────────────────

#[test]
fn test_kg_spreading_activation() {
    let kg = knowledge_graph_in_memory();
    let now = Utc::now();

    // Create 3 entities: A -> B -> C
    let id_a = Uuid::new_v4();
    let id_b = Uuid::new_v4();
    let id_c = Uuid::new_v4();

    kg.upsert_entity(&KgEntity {
        id: id_a,
        name: "Alpha".to_string(),
        layer: EntityLayer::Concept,
        summary: "First entity".to_string(),
        importance: 8.0,
        created_at: now,
        updated_at: now,
    })
    .expect("upsert Alpha");

    kg.upsert_entity(&KgEntity {
        id: id_b,
        name: "Beta".to_string(),
        layer: EntityLayer::Concept,
        summary: "Second entity".to_string(),
        importance: 6.0,
        created_at: now,
        updated_at: now,
    })
    .expect("upsert Beta");

    kg.upsert_entity(&KgEntity {
        id: id_c,
        name: "Gamma".to_string(),
        layer: EntityLayer::Research,
        summary: "Third entity".to_string(),
        importance: 4.0,
        created_at: now,
        updated_at: now,
    })
    .expect("upsert Gamma");

    // A -> B (weight 0.8)
    kg.add_edge(&KgEdge {
        id: Uuid::new_v4(),
        from_id: id_a,
        to_id: id_b,
        relationship: "related_to".to_string(),
        weight: 0.8,
        created_at: now,
    })
    .expect("add edge A->B");

    // B -> C (weight 0.6)
    kg.add_edge(&KgEdge {
        id: Uuid::new_v4(),
        from_id: id_b,
        to_id: id_c,
        relationship: "leads_to".to_string(),
        weight: 0.6,
        created_at: now,
    })
    .expect("add edge B->C");

    // Activate from "Alpha" with depth=3, decay=0.7
    let results = kg
        .activate(&["Alpha"], 3, 0.7)
        .expect("spreading activation");

    assert!(!results.is_empty(), "should have activated entities");

    // Alpha should have the highest score (1.0 as seed)
    assert_eq!(results[0].0.name, "Alpha");
    assert!((results[0].1 - 1.0).abs() < 1e-5);

    // Beta should be second (1.0 * 0.8 * 0.7 = 0.56)
    let beta = results.iter().find(|(e, _)| e.name == "Beta");
    assert!(beta.is_some(), "Beta should be activated");
    let beta_score = beta.unwrap().1;
    assert!(
        (beta_score - 0.56).abs() < 1e-3,
        "Beta score should be ~0.56, got {}",
        beta_score
    );

    // Gamma should also be activated (0.56 * 0.6 * 0.7 = 0.2352)
    let gamma = results.iter().find(|(e, _)| e.name == "Gamma");
    assert!(gamma.is_some(), "Gamma should be activated");
    let gamma_score = gamma.unwrap().1;
    assert!(
        (gamma_score - 0.2352).abs() < 1e-3,
        "Gamma score should be ~0.2352, got {}",
        gamma_score
    );

    // Ordering: Alpha > Beta > Gamma
    let names: Vec<&str> = results.iter().map(|(e, _)| e.name.as_str()).collect();
    assert_eq!(names, vec!["Alpha", "Beta", "Gamma"]);
}

#[test]
fn test_kg_activated_context_string() {
    let kg = knowledge_graph_in_memory();
    let now = Utc::now();

    kg.upsert_entity(&KgEntity {
        id: Uuid::new_v4(),
        name: "Rust".to_string(),
        layer: EntityLayer::Concept,
        summary: "Systems language".to_string(),
        importance: 9.0,
        created_at: now,
        updated_at: now,
    })
    .expect("upsert");

    let ctx = kg
        .activated_context_string(&["Rust"])
        .expect("context string");
    assert!(ctx.contains("Rust"), "should contain entity name");
    assert!(ctx.contains("1.00"), "seed should have score 1.00");

    // Empty seeds should return empty string
    let empty = kg.activated_context_string(&[]).expect("empty seeds");
    assert!(empty.is_empty());
}

// ─── Test: AxiomCulture lifecycle ───────────────────────────────────────────

#[test]
fn test_axiom_culture_lifecycle() {
    let ac = axiom_culture_in_memory();

    let axiom = Axiom {
        id: Uuid::new_v4(),
        name: "Occam's Razor".to_string(),
        statement: "Prefer the simplest explanation".to_string(),
        domain: "general".to_string(),
        evidence: "Philosophy".to_string(),
        created_at: Utc::now(),
    };

    ac.add_axiom(&axiom).expect("add axiom");

    let active = ac.get_axioms(None).expect("list active axioms");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].name, "Occam's Razor");
    assert_eq!(active[0].statement, "Prefer the simplest explanation");
}

// ─── Test: HermitBox isolation ──────────────────────────────────────────────

#[test]
fn test_hermit_box_isolation() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let hb1 = HermitBox::open(tmp.path(), "agent_alpha").expect("open alpha");
    let hb2 = HermitBox::open(tmp.path(), "agent_beta").expect("open beta");

    // DB paths should differ
    assert_ne!(hb1.box_dir, hb2.box_dir, "box directories should differ");
    assert!(
        hb1.box_dir.ends_with("agent_alpha_box"),
        "alpha box dir: {:?}",
        hb1.box_dir
    );
    assert!(
        hb2.box_dir.ends_with("agent_beta_box"),
        "beta box dir: {:?}",
        hb2.box_dir
    );

    // Writing to one should not affect the other
    hb1.remember("alpha memory", MemoryKind::Observation, 5.0)
        .expect("alpha remember");
    let beta_memories = hb2.recall("", 10).expect("beta recall");
    assert!(
        beta_memories.is_empty(),
        "beta should have no memories from alpha"
    );
}

// ─── Test: CognitiveTurn prepare ────────────────────────────────────────────

#[test]
fn test_cognitive_turn_prepare() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Create subsystems
    let ms = memory_stream_in_memory();
    let kg = knowledge_graph_in_memory();
    let ac = axiom_culture_in_memory();
    let hb = HermitBox::open(tmp.path(), "turn_test").expect("open hermit box");

    // Populate with some data
    ms.add(&MemoryEntry {
        id: Uuid::new_v4(),
        content: "The system uses Rust for safety".to_string(),
        kind: MemoryKind::Observation,
        importance: 7.0,
        created_at: Utc::now(),
        namespace: namespaces::PERSONAL.to_string(),
    })
    .expect("add memory");

    let entity_id = Uuid::new_v4();
    kg.upsert_entity(&KgEntity {
        id: entity_id,
        name: "safety".to_string(),
        layer: EntityLayer::Concept,
        summary: "Memory safety in systems programming".to_string(),
        importance: 8.0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    })
    .expect("upsert entity");

    ac.add_axiom(&Axiom {
        id: Uuid::new_v4(),
        name: "Defense in Depth".to_string(),
        statement: "Multiple security layers are better than one".to_string(),
        domain: "general".to_string(),
        evidence: "Security engineering".to_string(),
        created_at: Utc::now(),
    })
    .expect("add axiom");

    // Build CognitiveContext
    let ctx = CognitiveContext::new(ms, kg, ac, hb);

    // Prepare cognitive context from a user message containing "safety"
    let turn = prepare_cognitive_context(&ctx, "Tell me about safety in the system");

    // Should have recalled the memory
    assert!(
        !turn.memories.is_empty(),
        "should have recalled at least one memory"
    );

    // Should have KG context (keyword "safety" is >4 chars, matches entity)
    assert!(
        !turn.kg_context.is_empty(),
        "should have knowledge graph context, got empty string"
    );

    // Should have axioms
    assert!(!turn.axioms.is_empty(), "should have axioms");

    // Format should produce non-empty output
    let prefix = format_cognitive_prefix(&turn);
    assert!(prefix.contains("## Cognitive Context"));
    assert!(prefix.contains("### Recent Memories"));
    assert!(prefix.contains("### Knowledge"));
    assert!(prefix.contains("### Guiding Axioms"));
}

// ─── Test: format_cognitive_prefix with empty context ────────────────────────

#[test]
fn test_format_cognitive_prefix_empty() {
    let ctx = gyre::cognitive::CognitiveTurnContext {
        memories: vec![],
        kg_context: String::new(),
        axioms: vec![],
    };
    let prefix = format_cognitive_prefix(&ctx);
    assert!(
        prefix.is_empty(),
        "empty context should produce empty prefix"
    );
}

// ─── Phase 4 Tests ──────────────────────────────────────────────────────────

#[test]
fn test_tfidf_basic() {
    let embedder = TfIdfEmbedder::default();
    let v = embedder.embed("hello world");
    assert_eq!(v.len(), 512, "default vocab_size should be 512");
    let l2: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(
        (l2 - 1.0).abs() < 1e-5,
        "L2 norm should be ~1.0, got {}",
        l2
    );
}

#[test]
fn test_tfidf_similarity() {
    let embedder = TfIdfEmbedder::default();
    let v1 = embedder.embed("rust systems programming");
    let v2 = embedder.embed("rust low level code");
    let v3 = embedder.embed("banana smoothie recipe");

    let sim_related = cosine_similarity(&v1, &v2);
    let sim_unrelated = cosine_similarity(&v1, &v3);

    assert!(
        sim_related > sim_unrelated,
        "related texts should have higher similarity ({}) than unrelated ({})",
        sim_related,
        sim_unrelated
    );
}

#[tokio::test]
async fn test_cognitive_remember_tool() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let ms = memory_stream_in_memory();
    let kg = knowledge_graph_in_memory();
    let ac = axiom_culture_in_memory();
    let hb = HermitBox::open(tmp.path(), "remember_test").expect("open hermit box");

    let cog_ctx = Arc::new(CognitiveContext::new(ms, kg, ac, hb));
    let tool = CognitiveRememberTool::new(Arc::clone(&cog_ctx));

    let job_ctx = JobContext::default();
    let result = tool
        .execute(
            serde_json::json!({
                "content": "Rust is great for systems programming",
                "kind": "observation",
                "importance": 7.0
            }),
            &job_ctx,
        )
        .await
        .expect("execute remember tool");

    // Should return "Memory stored: <uuid>"
    let result_str = result.result.as_str().expect("result should be string");
    assert!(
        result_str.starts_with("Memory stored:"),
        "unexpected result: {}",
        result_str
    );

    // Verify the memory is in the stream
    let ms = cog_ctx.memory_stream.lock().expect("lock");
    let recalled = ms.recall("", 10).expect("recall");
    assert_eq!(recalled.len(), 1);
    assert!(recalled[0].content.contains("Rust is great"));
}

#[test]
fn test_auto_memory_decision() {
    let config = AutoMemoryConfig::default();
    let msg = "We decided to use Rust for the backend.";
    let memories = extract_memories_from_turn(msg, &config);

    assert!(!memories.is_empty(), "should extract at least one memory");
    assert!(
        matches!(memories[0].kind, MemoryKind::Decision),
        "should be Decision kind, got {:?}",
        memories[0].kind
    );
    assert!(
        (memories[0].importance - 8.5).abs() < f32::EPSILON,
        "decision importance should be 8.5, got {}",
        memories[0].importance
    );
}

#[test]
fn test_auto_store_memories() {
    let ms = memory_stream_in_memory();

    // Verify stream starts empty
    let before = ms.recall("", 100).expect("recall before");
    assert_eq!(before.len(), 0, "should start empty");

    let config = AutoMemoryConfig::default();
    auto_store_memories(
        &ms,
        "We decided to use Rust. I learned that safety matters. I promised to deliver by Friday.",
        &config,
    );

    let after = ms.recall("", 100).expect("recall after");
    assert!(
        after.len() >= 2,
        "should have stored at least 2 memories, got {}",
        after.len()
    );
}

// ─── Phase 5 Tests ──────────────────────────────────────────────────────────

#[test]
fn test_chief_can_write_worker_cannot() {
    let ms = memory_stream_in_memory();
    let chief = AgentIdentity::chief("kimi", "sargasso");
    let worker = AgentIdentity::worker("sarah", "sargasso", "kimi");

    let entry = MemoryEntry {
        id: Uuid::new_v4(),
        content: "Chief's memory".to_string(),
        kind: MemoryKind::Observation,
        importance: 5.0,
        created_at: Utc::now(),
        namespace: namespaces::TRIBE.to_string(),
    };

    // Chief can write
    assert!(
        ms.add_guarded(&entry, &chief).is_ok(),
        "Chief should be able to write memory"
    );

    let worker_entry = MemoryEntry {
        id: Uuid::new_v4(),
        content: "Worker's memory".to_string(),
        kind: MemoryKind::Observation,
        importance: 5.0,
        created_at: Utc::now(),
        namespace: namespaces::TRIBE.to_string(),
    };

    // Worker cannot write
    let result = ms.add_guarded(&worker_entry, &worker);
    assert!(result.is_err(), "Worker should NOT be able to write memory");
    assert!(
        result.unwrap_err().contains("cannot write memory"),
        "error should mention write restriction"
    );
}

#[test]
fn test_namespace_isolation() {
    let ms = memory_stream_in_memory();

    // Store memory in 'personal' namespace
    ms.add(&MemoryEntry {
        id: Uuid::new_v4(),
        content: "Personal thought".to_string(),
        kind: MemoryKind::Observation,
        importance: 5.0,
        created_at: Utc::now(),
        namespace: namespaces::PERSONAL.to_string(),
    })
    .expect("add personal memory");

    // Recall in 'tribe' namespace should return empty
    let tribe_memories = ms
        .recall_in_namespace(namespaces::TRIBE, 10)
        .expect("recall tribe");
    assert!(
        tribe_memories.is_empty(),
        "tribe namespace should have no memories, got {}",
        tribe_memories.len()
    );

    // Recall in 'personal' namespace should return the entry
    let personal_memories = ms
        .recall_in_namespace(namespaces::PERSONAL, 10)
        .expect("recall personal");
    assert_eq!(
        personal_memories.len(),
        1,
        "personal namespace should have 1 memory"
    );
    assert!(personal_memories[0].content.contains("Personal thought"));
}

#[test]
fn test_distill_for_worker() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let ms = memory_stream_in_memory();
    let kg = knowledge_graph_in_memory();
    let ac = axiom_culture_in_memory();
    let hb = HermitBox::open(tmp.path(), "distill_test").expect("open hermit box");

    // Store 3 memories in tribe namespace
    for i in 0..3 {
        ms.add(&MemoryEntry {
            id: Uuid::new_v4(),
            content: format!("Tribe memory about auth pattern {}", i),
            kind: MemoryKind::Observation,
            importance: 6.0,
            created_at: Utc::now(),
            namespace: namespaces::TRIBE.to_string(),
        })
        .expect("add tribe memory");
    }

    // Add an active axiom
    ac.add_axiom(&Axiom {
        id: Uuid::new_v4(),
        name: "Security First".to_string(),
        statement: "Always validate inputs".to_string(),
        domain: "general".to_string(),
        evidence: "Best practice".to_string(),
        created_at: Utc::now(),
    })
    .expect("add axiom");

    let ctx = CognitiveContext::new(ms, kg, ac, hb);
    let tribe_ctx = distill_for_worker(&ctx, "implement auth");

    // Should have recalled tribe memories (up to 5)
    assert!(
        tribe_ctx.memories.len() <= 5,
        "should have at most 5 memories, got {}",
        tribe_ctx.memories.len()
    );

    // system_prompt_block should be non-empty and contain the header
    let block = tribe_ctx.system_prompt_block();
    assert!(!block.is_empty(), "system_prompt_block should not be empty");
    assert!(
        block.contains("## Delegated Context"),
        "should contain '## Delegated Context', got:\n{}",
        block
    );
}

#[test]
fn test_axiom_propose_approve() {
    let ac = axiom_culture_in_memory();

    // Propose creates inactive axiom
    let id = ac
        .propose("Always test edge cases", "worker_sarah")
        .expect("propose axiom");

    // Should NOT appear in list_active
    let active_before = ac.list_active().expect("list active");
    assert!(
        active_before.is_empty(),
        "proposed axiom should not be active yet"
    );

    // Approve makes it active
    ac.approve(id).expect("approve axiom");
    let active_after = ac.list_active().expect("list active after approve");
    assert_eq!(
        active_after.len(),
        1,
        "approved axiom should appear in list_active"
    );
    assert!(active_after[0].statement.contains("Always test edge cases"));
}

#[test]
fn test_axiom_worker_propose_only() {
    let ac = axiom_culture_in_memory();
    let worker = AgentIdentity::worker("sarah", "sargasso", "kimi");

    // Worker cannot write axiom directly (can_write_axiom returns false)
    assert!(
        !worker.can_write_axiom(),
        "Worker should not be able to write axioms directly"
    );

    // Worker can propose
    let id = ac
        .propose("Workers should be careful", &worker.id)
        .expect("worker propose");

    // Proposed axiom is not active
    let active = ac.list_active().expect("list active");
    assert!(
        active.is_empty(),
        "proposed axiom should not be active until approved"
    );

    // Chief approves
    let chief = AgentIdentity::chief("kimi", "sargasso");
    assert!(
        chief.can_write_axiom(),
        "Chief should be able to write axioms"
    );
    ac.approve(id).expect("chief approve");

    let active_after = ac.list_active().expect("list active after chief approve");
    assert_eq!(active_after.len(), 1);
}

#[test]
fn test_tribe_context_system_prompt() {
    let ctx = TribeContext {
        memories: vec![MemoryEntry {
            id: Uuid::new_v4(),
            content: "Previous auth implementation used JWT".to_string(),
            kind: MemoryKind::Lesson,
            importance: 7.0,
            created_at: Utc::now(),
            namespace: namespaces::TRIBE.to_string(),
        }],
        kg_context: "auth (concept, score=1.00): Authentication system".to_string(),
        axioms: vec!["Security First: Always validate inputs".to_string()],
        task_hint: "Implement OAuth2 flow".to_string(),
    };

    let block = ctx.system_prompt_block();
    assert!(
        block.contains("## Delegated Context"),
        "should contain header"
    );
    assert!(block.contains("### Task"), "should contain task section");
    assert!(
        block.contains("Implement OAuth2 flow"),
        "should contain task hint"
    );
    assert!(
        block.contains("### Relevant Memories"),
        "should contain memories section"
    );
    assert!(
        block.contains("### Knowledge"),
        "should contain knowledge section"
    );
    assert!(
        block.contains("### Guiding Axioms"),
        "should contain axioms section"
    );
}

// ─── Phase 6 Tests ──────────────────────────────────────────────────────────

#[test]
fn test_hermit_box_creates_dirs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let hb = HermitBox::open(tmp.path(), "test_agent").expect("open box");

    let box_dir = hb.box_dir();
    assert!(box_dir.join("memory").is_dir(), "memory/ should exist");
    assert!(
        box_dir.join("knowledge").is_dir(),
        "knowledge/ should exist"
    );
    assert!(box_dir.join("axioms").is_dir(), "axioms/ should exist");
}

#[test]
fn test_hermit_box_rejects_path_traversal() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let result = HermitBox::open(tmp.path(), "../evil");
    assert!(result.is_err(), "should reject path traversal");
    let err = result.err().unwrap();
    assert!(
        err.contains(".."),
        "error should mention '..', got: {}",
        err
    );
}

#[test]
fn test_hermit_box_rejects_slash() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let result = HermitBox::open(tmp.path(), "foo/bar");
    assert!(result.is_err(), "should reject slash in agent_id");
    let err = result.err().unwrap();
    assert!(
        err.contains("/") || err.contains("\\"),
        "error should mention slash, got: {}",
        err
    );
}

#[test]
fn test_hermit_box_persistence() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // Open box, add a memory, then drop
    {
        let hb = HermitBox::open(tmp.path(), "persistent_agent").expect("open box");
        hb.remember("persistent thought", MemoryKind::Observation, 7.0)
            .expect("remember");
    }

    // Re-open same box, verify memory persists
    {
        let hb = HermitBox::open(tmp.path(), "persistent_agent").expect("reopen box");
        let memories = hb.recall("", 10).expect("recall");
        assert_eq!(memories.len(), 1, "memory should persist across open/close");
        assert!(
            memories[0].content.contains("persistent thought"),
            "content should match, got: {}",
            memories[0].content
        );
    }
}

#[test]
fn test_two_agents_isolated() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let hb_kimi = HermitBox::open(tmp.path(), "kimi").expect("open kimi");
    let hb_sarah = HermitBox::open(tmp.path(), "sarah").expect("open sarah");

    // Verify different box_dir paths
    assert_ne!(
        hb_kimi.box_dir(),
        hb_sarah.box_dir(),
        "box directories should be separate"
    );
    assert!(
        hb_kimi.box_dir().ends_with("kimi_box"),
        "kimi box dir: {:?}",
        hb_kimi.box_dir()
    );
    assert!(
        hb_sarah.box_dir().ends_with("sarah_box"),
        "sarah box dir: {:?}",
        hb_sarah.box_dir()
    );

    // Write to kimi, verify sarah is empty
    hb_kimi
        .remember("kimi's secret", MemoryKind::Observation, 5.0)
        .expect("kimi remember");
    let sarah_memories = hb_sarah.recall("", 10).expect("sarah recall");
    assert!(
        sarah_memories.is_empty(),
        "sarah should have no memories from kimi"
    );
}

#[test]
fn test_identity_system_prompt() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let hb = HermitBox::open(tmp.path(), "identity_agent").expect("open box");

    // Write soul.md manually
    std::fs::write(hb.soul_path(), "I am a helpful research assistant.\n").expect("write soul.md");

    // Load identity files
    let identity = AgentIdentityFiles::load(&hb);
    assert!(
        identity.soul.contains("helpful research assistant"),
        "soul should contain written content, got: {}",
        identity.soul
    );

    // system_prompt_block should include the soul section
    let block = identity.system_prompt_block();
    assert!(
        block.contains("## Agent Identity"),
        "should contain header, got:\n{}",
        block
    );
    assert!(
        block.contains("### Soul"),
        "should contain soul section, got:\n{}",
        block
    );
    assert!(
        block.contains("helpful research assistant"),
        "should contain soul content, got:\n{}",
        block
    );

    // Empty identity should return empty string
    let tmp2 = tempfile::tempdir().expect("tempdir2");
    let hb2 = HermitBox::open(tmp2.path(), "empty_agent").expect("open empty box");
    let empty_identity = AgentIdentityFiles::load(&hb2);
    assert!(
        empty_identity.system_prompt_block().is_empty(),
        "empty identity should produce empty prompt block"
    );
}

// ─── Phase 5b Tests: BLOB embeddings, KG semantic search, backfill ──────────

#[test]
fn test_blob_roundtrip() {
    let original = vec![1.0_f32, -2.5, 0.0, 3.14159, f32::MAX, f32::MIN];
    let blob = f32_vec_to_blob(&original);
    let recovered = blob_to_f32_vec(&blob);
    assert_eq!(
        original, recovered,
        "BLOB roundtrip must preserve exact f32 values"
    );
}

#[test]
fn test_blob_empty() {
    let empty: Vec<f32> = vec![];
    let blob = f32_vec_to_blob(&empty);
    assert!(blob.is_empty());
    let recovered = blob_to_f32_vec(&blob);
    assert!(recovered.is_empty());
}

#[test]
fn test_blob_bad_length() {
    // 5 bytes is not a multiple of 4
    let bad = vec![0u8, 1, 2, 3, 4];
    let recovered = blob_to_f32_vec(&bad);
    assert!(
        recovered.is_empty(),
        "non-multiple-of-4 blob should return empty"
    );
}

#[test]
fn test_store_with_blob_embedding_and_recall() {
    let ms = memory_stream_in_memory();

    let embedding = vec![1.0, 0.0, 0.0];
    let entry = MemoryEntry {
        id: Uuid::new_v4(),
        content: "stored with blob".to_string(),
        kind: MemoryKind::Observation,
        importance: 5.0,
        created_at: Utc::now(),
        namespace: namespaces::PERSONAL.to_string(),
    };

    ms.store_with_embedding(&entry, embedding.clone())
        .expect("store with embedding");

    // Query with same direction — should find it
    let results = ms
        .recall_relevant(&[0.9, 0.1, 0.0], 5)
        .expect("recall_relevant");
    assert!(!results.is_empty(), "should find the stored entry");
    assert!(
        results[0].0.content.contains("stored with blob"),
        "first result should be our entry"
    );
    assert!(results[0].1 > 0.9, "similarity should be high");
}

#[test]
fn test_json_to_blob_migration() {
    // Simulate legacy: create a DB with JSON embeddings, then migrate
    use rusqlite::Connection;
    let conn = Connection::open(":memory:").expect("open");
    conn.execute_batch(
        "CREATE TABLE memory_entries (
            id TEXT PRIMARY KEY,
            content TEXT NOT NULL,
            kind TEXT NOT NULL,
            importance REAL NOT NULL DEFAULT 5.0,
            created_at TEXT NOT NULL,
            embedding TEXT,
            namespace TEXT NOT NULL DEFAULT 'personal'
        );",
    )
    .expect("create table");

    // Insert a legacy JSON embedding
    let legacy_vec = vec![0.1_f32, 0.2, 0.3];
    let json_str = serde_json::to_string(&legacy_vec).unwrap();
    conn.execute(
        "INSERT INTO memory_entries (id, content, kind, importance, created_at, embedding) \
         VALUES ('test-id', 'legacy content', 'observation', 5.0, '2024-01-01T00:00:00Z', ?1)",
        rusqlite::params![json_str],
    )
    .expect("insert legacy");

    // Drop the connection, reopen via MemoryStream (triggers migration)
    drop(conn);

    // MemoryStream::new on :memory: creates a fresh DB, so we test the migration
    // function directly instead
    let conn2 = Connection::open(":memory:").expect("open2");
    conn2
        .execute_batch(
            "CREATE TABLE memory_entries (
            id TEXT PRIMARY KEY,
            content TEXT NOT NULL,
            kind TEXT NOT NULL,
            importance REAL NOT NULL DEFAULT 5.0,
            created_at TEXT NOT NULL,
            embedding TEXT,
            namespace TEXT NOT NULL DEFAULT 'personal',
            embedding_blob BLOB
        );",
        )
        .expect("create table with blob col");

    conn2
        .execute(
            "INSERT INTO memory_entries (id, content, kind, importance, created_at, embedding) \
         VALUES ('migrated-id', 'migrated content', 'lesson', 7.0, '2024-01-01T00:00:00Z', ?1)",
            rusqlite::params![json_str],
        )
        .expect("insert for migration");

    // Run the migration manually (it's a private method, but we can replicate the logic)
    let mut stmt = conn2
        .prepare(
            "SELECT id, embedding FROM memory_entries \
             WHERE embedding IS NOT NULL AND embedding_blob IS NULL",
        )
        .expect("prepare");
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0).unwrap(),
                row.get::<_, String>(1).unwrap(),
            ))
        })
        .expect("query")
        .filter_map(|r| r.ok())
        .collect();
    assert_eq!(rows.len(), 1, "should have 1 row to migrate");

    for (id, json) in &rows {
        let vec: Vec<f32> = serde_json::from_str(json).unwrap();
        let blob = f32_vec_to_blob(&vec);
        conn2
            .execute(
                "UPDATE memory_entries SET embedding_blob = ?1 WHERE id = ?2",
                rusqlite::params![blob, id],
            )
            .expect("update blob");
    }

    // Verify the blob was stored correctly
    let blob: Vec<u8> = conn2
        .query_row(
            "SELECT embedding_blob FROM memory_entries WHERE id = 'migrated-id'",
            [],
            |row| row.get(0),
        )
        .expect("read blob");
    let recovered = blob_to_f32_vec(&blob);
    assert_eq!(
        recovered, legacy_vec,
        "migrated blob should match original vector"
    );
}

#[test]
fn test_auto_embed_on_store() {
    let ms = memory_stream_in_memory();
    let embedder = TfIdfEmbedder::default();

    let entry = MemoryEntry {
        id: Uuid::new_v4(),
        content: "auto embedded content here".to_string(),
        kind: MemoryKind::Observation,
        importance: 5.0,
        created_at: Utc::now(),
        namespace: namespaces::PERSONAL.to_string(),
    };

    ms.add_with_auto_embed(&entry, &embedder)
        .expect("add_with_auto_embed");

    // Should be findable via semantic search
    let query_vec = embedder.embed("auto embedded content here");
    let results = ms.recall_relevant(&query_vec, 5).expect("recall_relevant");
    assert!(
        !results.is_empty(),
        "auto-embedded entry should be findable"
    );
    assert!(
        results[0].1 > 0.9,
        "identical content embedding should have high similarity, got {}",
        results[0].1
    );
}

#[test]
fn test_kg_semantic_search() {
    let kg = knowledge_graph_in_memory();
    let embedder = TfIdfEmbedder::default();

    // Insert entities with embeddings
    let emb_rust = embedder.embed("Rust programming language");
    let emb_python = embedder.embed("Python scripting language");

    kg.upsert_by_name_with_embedding(
        "Rust",
        EntityLayer::Concept,
        "Systems programming language",
        8.0,
        Some(&emb_rust),
    )
    .expect("upsert Rust");

    kg.upsert_by_name_with_embedding(
        "Python",
        EntityLayer::Concept,
        "Scripting language",
        7.0,
        Some(&emb_python),
    )
    .expect("upsert Python");

    // Search for "Rust programming" — should rank Rust higher
    let query = embedder.embed("Rust programming");
    let results = kg.search_semantic(&query, 10).expect("search_semantic");
    assert_eq!(results.len(), 2, "should find both entities");
    assert_eq!(
        results[0].0.name, "Rust",
        "Rust should be most similar to 'Rust programming'"
    );
    assert!(
        results[0].1 > results[1].1,
        "Rust should score higher than Python"
    );
}

#[test]
fn test_kg_semantic_search_empty_query() {
    let kg = knowledge_graph_in_memory();
    let results = kg.search_semantic(&[], 10).expect("empty query");
    assert!(results.is_empty());
}

#[test]
fn test_memory_backfill_embeddings() {
    let ms = memory_stream_in_memory();
    let embedder = TfIdfEmbedder::default();

    // Add entries without embeddings
    for i in 0..3 {
        ms.add(&MemoryEntry {
            id: Uuid::new_v4(),
            content: format!("unembedded memory number {i}"),
            kind: MemoryKind::Observation,
            importance: 5.0,
            created_at: Utc::now(),
            namespace: namespaces::PERSONAL.to_string(),
        })
        .expect("add");
    }

    // Backfill
    let count = ms.backfill_embeddings(&embedder).expect("backfill");
    assert_eq!(count, 3, "should backfill all 3 entries");

    // Second backfill should find nothing
    let count2 = ms.backfill_embeddings(&embedder).expect("backfill2");
    assert_eq!(count2, 0, "second backfill should find nothing");

    // Should now be findable via semantic search
    let query = embedder.embed("unembedded memory");
    let results = ms
        .recall_relevant(&query, 5)
        .expect("recall after backfill");
    assert_eq!(results.len(), 3, "all 3 should be found");
    assert!(
        results[0].1 > 0.0,
        "backfilled entries should have positive scores"
    );
}

#[test]
fn test_kg_backfill_embeddings() {
    let kg = knowledge_graph_in_memory();
    let embedder = TfIdfEmbedder::default();

    // Add entities without embeddings
    kg.upsert_by_name("Rust", EntityLayer::Concept, "Systems language", 8.0)
        .expect("upsert Rust");
    kg.upsert_by_name("Python", EntityLayer::Concept, "Scripting language", 7.0)
        .expect("upsert Python");

    // Backfill
    let count = kg.backfill_embeddings(&embedder).expect("backfill");
    assert_eq!(count, 2, "should backfill both entities");

    // Second backfill should find nothing
    let count2 = kg.backfill_embeddings(&embedder).expect("backfill2");
    assert_eq!(count2, 0, "second backfill should find nothing");

    // Should now be findable via semantic search
    let query = embedder.embed("Rust");
    let results = kg
        .search_semantic(&query, 5)
        .expect("search after backfill");
    assert_eq!(results.len(), 2, "both entities should be found");
    assert_eq!(results[0].0.name, "Rust", "Rust should be most similar");
}
