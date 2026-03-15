//! Fan-out recall: one query, three search layers combined with Reciprocal Rank Fusion.
//!
//! Layers:
//! 1. Semantic search via embedding similarity (memory_stream.recall_relevant)
//! 2. Knowledge graph spreading activation (knowledge_graph.activate)
//! 3. Full-text/recency fallback (memory_stream.recall)
//!
//! Results are merged using RRF (k=60) and deduped by content prefix.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::cognitive::embedding::EmbeddingProvider;
use crate::cognitive::knowledge_graph::KnowledgeGraph;
use crate::cognitive::memory_stream::MemoryStream;

/// Source layer that produced a recall result.
#[derive(Debug, Clone, PartialEq)]
pub enum RecallSource {
    /// From memory_entries via embedding cosine similarity.
    MemoryStream,
    /// From kg_entities via spreading activation.
    KnowledgeGraph,
    /// From kg_entities via embedding cosine similarity.
    KgSemantic,
    /// From FTS / recency query on memory content.
    FullTextSearch,
}

impl RecallSource {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::MemoryStream => "memory",
            Self::KnowledgeGraph => "kg",
            Self::KgSemantic => "kg_vec",
            Self::FullTextSearch => "fts",
        }
    }
}

/// A single recall result from any layer.
#[derive(Debug, Clone)]
pub struct RecallResult {
    pub content: String,
    pub source: RecallSource,
    pub score: f32,
    pub metadata: serde_json::Value,
}

/// Dedup key: first 200 chars of content, lowercased.
fn dedup_key(content: &str) -> String {
    let end = content
        .char_indices()
        .nth(200)
        .map(|(i, _)| i)
        .unwrap_or(content.len());
    content[..end].to_lowercase()
}

/// Extract likely entity names from a query string.
///
/// Takes capitalized words and the full query as seeds for graph activation.
fn extract_seed_names(query: &str) -> Vec<String> {
    let mut seeds: Vec<String> = query
        .split_whitespace()
        .filter(|w| w.len() > 1 && w.chars().next().map(|c| c.is_uppercase()).unwrap_or(false))
        .map(|w| w.to_string())
        .collect();

    // Always include the full query as a seed
    if !query.is_empty() {
        seeds.push(query.to_string());
    }
    seeds.dedup();
    seeds
}

/// Fan-out recall: query three layers and merge with Reciprocal Rank Fusion.
///
/// 1. Embed the query → semantic search on memory stream
/// 2. Extract entity seeds → spreading activation on knowledge graph
/// 3. Recency/keyword fallback on memory stream
/// 4. RRF merge (k=60), dedup by content prefix, return top `limit`
pub fn fan_out_recall(
    query: &str,
    embedding_provider: &dyn EmbeddingProvider,
    memory_stream: &Arc<Mutex<MemoryStream>>,
    knowledge_graph: &Arc<Mutex<KnowledgeGraph>>,
    limit: usize,
) -> Vec<RecallResult> {
    let fetch_limit = limit * 2;

    // ── Layer 1: Semantic search ────────────────────────────────────────
    let semantic_results: Vec<RecallResult> = {
        let query_vec = embedding_provider.embed(query);
        if query_vec.is_empty() {
            Vec::new()
        } else {
            let ms = match memory_stream.lock() {
                Ok(ms) => ms,
                Err(_) => return Vec::new(),
            };
            ms.recall_relevant(&query_vec, fetch_limit)
                .unwrap_or_default()
                .into_iter()
                .map(|(entry, sim)| RecallResult {
                    content: entry.content.clone(),
                    source: RecallSource::MemoryStream,
                    score: sim,
                    metadata: serde_json::json!({
                        "kind": entry.kind.as_str(),
                        "created_at": entry.created_at.to_rfc3339(),
                        "importance": entry.importance,
                        "id": entry.id.to_string(),
                    }),
                })
                .collect()
        }
    };

    // ── Layer 2: Knowledge graph spreading activation ───────────────────
    let kg_results: Vec<RecallResult> = {
        let seeds = extract_seed_names(query);
        if seeds.is_empty() {
            Vec::new()
        } else {
            let kg = match knowledge_graph.lock() {
                Ok(kg) => kg,
                Err(_) => return semantic_results,
            };
            let seed_refs: Vec<&str> = seeds.iter().map(|s| s.as_str()).collect();
            kg.activate(&seed_refs, 3, 0.7)
                .unwrap_or_default()
                .into_iter()
                .map(|(entity, activation)| {
                    let content = if entity.summary.is_empty() {
                        entity.name.clone()
                    } else {
                        format!("{}: {}", entity.name, entity.summary)
                    };
                    RecallResult {
                        content,
                        source: RecallSource::KnowledgeGraph,
                        score: activation,
                        metadata: serde_json::json!({
                            "entity_name": entity.name,
                            "layer": entity.layer.as_str(),
                            "importance": entity.importance,
                            "id": entity.id.to_string(),
                        }),
                    }
                })
                .collect()
        }
    };

    // ── Layer 3: KG semantic search ─────────────────────────────────────
    let kg_semantic_results: Vec<RecallResult> = {
        let query_vec = embedding_provider.embed(query);
        if query_vec.is_empty() {
            Vec::new()
        } else {
            match knowledge_graph.lock() {
                Ok(kg) => kg
                    .search_semantic(&query_vec, fetch_limit)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(entity, sim)| {
                        let content = if entity.summary.is_empty() {
                            entity.name.clone()
                        } else {
                            format!("{}: {}", entity.name, entity.summary)
                        };
                        RecallResult {
                            content,
                            source: RecallSource::KgSemantic,
                            score: sim,
                            metadata: serde_json::json!({
                                "entity_name": entity.name,
                                "layer": entity.layer.as_str(),
                                "importance": entity.importance,
                                "id": entity.id.to_string(),
                            }),
                        }
                    })
                    .collect(),
                Err(_) => Vec::new(),
            }
        }
    };

    // ── Layer 4: FTS / recency fallback ─────────────────────────────────
    let fts_results: Vec<RecallResult> = {
        let ms = match memory_stream.lock() {
            Ok(ms) => ms,
            Err(_) => return semantic_results,
        };
        ms.recall(query, fetch_limit)
            .unwrap_or_default()
            .into_iter()
            .map(|entry| RecallResult {
                content: entry.content.clone(),
                source: RecallSource::FullTextSearch,
                score: 0.0, // recency-ordered, no relevance score
                metadata: serde_json::json!({
                    "kind": entry.kind.as_str(),
                    "created_at": entry.created_at.to_rfc3339(),
                    "importance": entry.importance,
                    "id": entry.id.to_string(),
                }),
            })
            .collect()
    };

    // ── RRF merge ───────────────────────────────────────────────────────
    rrf_merge(
        &[
            &semantic_results,
            &kg_results,
            &kg_semantic_results,
            &fts_results,
        ],
        60,
        limit,
    )
}

/// Reciprocal Rank Fusion across multiple ranked lists.
///
/// `score(d) = sum(1 / (k + rank_i))` for each list where d appears.
/// Deduplicates by content prefix (first 200 chars). When duplicates are
/// found, the first occurrence's metadata is kept.
fn rrf_merge(lists: &[&[RecallResult]], k: u32, limit: usize) -> Vec<RecallResult> {
    // Map from dedup_key → (accumulated RRF score, best RecallResult)
    let mut scores: HashMap<String, (f32, RecallResult)> = HashMap::new();

    for list in lists {
        for (rank, result) in list.iter().enumerate() {
            let key = dedup_key(&result.content);
            let rrf_contribution = 1.0 / (k as f32 + rank as f32 + 1.0);

            scores
                .entry(key)
                .and_modify(|(acc_score, _existing)| {
                    *acc_score += rrf_contribution;
                })
                .or_insert_with(|| (rrf_contribution, result.clone()));
        }
    }

    let mut merged: Vec<RecallResult> = scores
        .into_values()
        .map(|(fused_score, mut result)| {
            result.score = fused_score;
            result
        })
        .collect();

    merged.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    merged.truncate(limit);
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a RecallResult for testing.
    fn make_result(content: &str, source: RecallSource, score: f32) -> RecallResult {
        RecallResult {
            content: content.to_string(),
            source,
            score,
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn test_rrf_scoring_three_lists() {
        let list_a = vec![
            make_result("alpha", RecallSource::MemoryStream, 0.9),
            make_result("beta", RecallSource::MemoryStream, 0.8),
            make_result("gamma", RecallSource::MemoryStream, 0.7),
        ];
        let list_b = vec![
            make_result("beta", RecallSource::KnowledgeGraph, 1.0),
            make_result("delta", RecallSource::KnowledgeGraph, 0.5),
        ];
        let list_c = vec![
            make_result("alpha", RecallSource::FullTextSearch, 0.0),
            make_result("gamma", RecallSource::FullTextSearch, 0.0),
            make_result("epsilon", RecallSource::FullTextSearch, 0.0),
        ];

        let merged = rrf_merge(&[&list_a, &list_b, &list_c], 60, 10);

        // "alpha" appears in list_a rank 0 and list_c rank 0:
        //   1/(60+1) + 1/(60+1) = 2/61
        // "beta" appears in list_a rank 1 and list_b rank 0:
        //   1/(60+2) + 1/(60+1) = 1/62 + 1/61
        // Both should be near the top
        let alpha = merged.iter().find(|r| r.content == "alpha");
        let beta = merged.iter().find(|r| r.content == "beta");
        assert!(alpha.is_some());
        assert!(beta.is_some());

        let alpha_score = alpha.map(|r| r.score).unwrap_or(0.0);
        let beta_score = beta.map(|r| r.score).unwrap_or(0.0);

        // alpha: 2 * 1/61 ≈ 0.03279
        // beta: 1/62 + 1/61 ≈ 0.03277
        // alpha should score >= beta
        assert!(
            alpha_score >= beta_score - 0.001,
            "alpha ({alpha_score}) should score >= beta ({beta_score})"
        );

        // All 5 unique items should be present
        assert_eq!(merged.len(), 5);
    }

    #[test]
    fn test_dedup_same_content_different_sources() {
        let list_a = vec![make_result(
            "We decided to use Rust for safety",
            RecallSource::MemoryStream,
            0.9,
        )];
        let list_b = vec![make_result(
            "We decided to use Rust for safety",
            RecallSource::FullTextSearch,
            0.0,
        )];

        let merged = rrf_merge(&[&list_a, &list_b], 60, 10);

        // Should be deduped to 1 result
        assert_eq!(merged.len(), 1);
        // Score should be sum of both RRF contributions
        let expected = 1.0 / 61.0 + 1.0 / 61.0;
        assert!(
            (merged[0].score - expected).abs() < 0.0001,
            "expected {expected}, got {}",
            merged[0].score
        );
    }

    #[test]
    fn test_empty_inputs() {
        let empty: Vec<RecallResult> = vec![];
        let merged = rrf_merge(&[&empty, &empty, &empty], 60, 10);
        assert!(merged.is_empty());
    }

    #[test]
    fn test_single_source_works() {
        let list = vec![
            make_result("first", RecallSource::MemoryStream, 0.9),
            make_result("second", RecallSource::MemoryStream, 0.8),
        ];
        let empty: Vec<RecallResult> = vec![];

        let merged = rrf_merge(&[&list, &empty, &empty], 60, 10);
        assert_eq!(merged.len(), 2);
        // First item should have higher RRF score
        assert!(merged[0].score > merged[1].score);
    }

    #[test]
    fn test_limit_respected() {
        let list: Vec<RecallResult> = (0..20)
            .map(|i| make_result(&format!("item-{i}"), RecallSource::MemoryStream, 0.0))
            .collect();

        let merged = rrf_merge(&[&list], 60, 5);
        assert_eq!(merged.len(), 5);
    }

    #[test]
    fn test_extract_seed_names() {
        let seeds = extract_seed_names("What did Greg say about Rust");
        assert!(seeds.contains(&"Greg".to_string()));
        assert!(seeds.contains(&"Rust".to_string()));
        assert!(seeds.contains(&"What".to_string()));
        // Full query is always included
        assert!(seeds.contains(&"What did Greg say about Rust".to_string()));
    }

    #[test]
    fn test_extract_seed_names_empty() {
        let seeds = extract_seed_names("");
        assert!(seeds.is_empty());
    }

    #[test]
    fn test_dedup_key_case_insensitive() {
        assert_eq!(dedup_key("Hello World"), dedup_key("hello world"));
    }
}
