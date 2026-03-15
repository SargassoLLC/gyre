//! Embedding providers for the cognitive layer.
//!
//! Provides a trait for text-to-vector embedding and multiple implementations:
//! - `NullEmbedder`: returns an empty vector (fallback/no-op)
//! - `TfIdfEmbedder`: simple bag-of-words with hashed buckets, normalized to unit vector
//! - `FastEmbedProvider` (feature `fastembed`): real semantic embeddings via ONNX (all-MiniLM-L6-v2)

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

/// Trait for converting text into a dense vector representation.
pub trait EmbeddingProvider: Send + Sync {
    /// Embed a text string into a vector of f32 values.
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// No-op embedder that always returns an empty vector.
pub struct NullEmbedder;

impl EmbeddingProvider for NullEmbedder {
    fn embed(&self, _text: &str) -> Vec<f32> {
        vec![]
    }
}

/// Simple TF-IDF-style bag-of-words embedder using hashed token buckets.
///
/// Tokenizes on whitespace/punctuation, lowercases, hashes each token to a
/// bucket index, counts frequencies, and normalizes to a unit vector (L2 norm).
pub struct TfIdfEmbedder {
    pub vocab_size: usize,
}

impl Default for TfIdfEmbedder {
    fn default() -> Self {
        Self { vocab_size: 512 }
    }
}

impl TfIdfEmbedder {
    /// Tokenize text: split on non-alphanumeric chars, lowercase, filter empty.
    fn tokenize<'a>(&self, text: &'a str) -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_lowercase())
            .collect()
    }

    /// Hash a token to a bucket index.
    fn hash_to_bucket(&self, token: &str) -> usize {
        let mut hasher = DefaultHasher::new();
        token.hash(&mut hasher);
        (hasher.finish() as usize) % self.vocab_size
    }
}

impl EmbeddingProvider for TfIdfEmbedder {
    fn embed(&self, text: &str) -> Vec<f32> {
        let tokens = self.tokenize(text);
        if tokens.is_empty() {
            return vec![0.0; self.vocab_size];
        }

        // Count frequencies per bucket
        let mut counts: HashMap<usize, f32> = HashMap::new();
        for token in &tokens {
            let bucket = self.hash_to_bucket(token);
            *counts.entry(bucket).or_insert(0.0) += 1.0;
        }

        // Build vector
        let mut vec = vec![0.0_f32; self.vocab_size];
        for (bucket, count) in counts {
            vec[bucket] = count;
        }

        // Normalize to unit vector (L2 norm)
        let l2: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if l2 > 0.0 {
            for v in &mut vec {
                *v /= l2;
            }
        }

        vec
    }
}

/// Real semantic embedding provider using fastembed (ONNX-based, all-MiniLM-L6-v2).
///
/// Downloads the model (~23MB) on first use and caches it locally.
/// Produces 384-dimensional embeddings with no API key required.
#[cfg(feature = "fastembed")]
pub struct FastEmbedProvider {
    model: std::sync::Mutex<fastembed::TextEmbedding>,
}

#[cfg(feature = "fastembed")]
impl FastEmbedProvider {
    /// Create a new provider using the default all-MiniLM-L6-v2 model.
    pub fn new() -> Result<Self, fastembed::Error> {
        let model = fastembed::TextEmbedding::try_new(
            fastembed::InitOptions::new(fastembed::EmbeddingModel::AllMiniLML6V2)
                .with_show_download_progress(true),
        )?;
        Ok(Self {
            model: std::sync::Mutex::new(model),
        })
    }

    /// Embedding dimension (384 for all-MiniLM-L6-v2).
    pub fn dimension() -> usize {
        384
    }
}

#[cfg(feature = "fastembed")]
impl EmbeddingProvider for FastEmbedProvider {
    fn embed(&self, text: &str) -> Vec<f32> {
        let mut model = self.model.lock().expect("fastembed mutex poisoned");
        model
            .embed(vec![text.to_string()], None)
            .ok()
            .and_then(|mut vecs| {
                if vecs.is_empty() {
                    None
                } else {
                    Some(vecs.swap_remove(0))
                }
            })
            .unwrap_or_else(|| vec![0.0; Self::dimension()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null_embedder() {
        let e = NullEmbedder;
        assert!(e.embed("anything").is_empty());
    }

    #[test]
    fn test_tfidf_produces_correct_length() {
        let e = TfIdfEmbedder::default();
        let v = e.embed("hello world");
        assert_eq!(v.len(), 512);
    }

    #[test]
    fn test_tfidf_unit_norm() {
        let e = TfIdfEmbedder::default();
        let v = e.embed("hello world test tokens");
        let l2: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (l2 - 1.0).abs() < 1e-5,
            "L2 norm should be ~1.0, got {}",
            l2
        );
    }

    #[test]
    fn test_tfidf_empty_text() {
        let e = TfIdfEmbedder::default();
        let v = e.embed("");
        assert_eq!(v.len(), 512);
        // All zeros for empty text
        assert!(v.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn test_tfidf_deterministic() {
        let e = TfIdfEmbedder::default();
        let v1 = e.embed("rust programming");
        let v2 = e.embed("rust programming");
        assert_eq!(v1, v2);
    }

    #[cfg(feature = "fastembed")]
    mod fastembed_tests {
        use super::*;

        fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
            let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
            let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
            let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm_a == 0.0 || norm_b == 0.0 {
                return 0.0;
            }
            dot / (norm_a * norm_b)
        }

        #[test]
        fn test_fastembed_basic() {
            let provider = FastEmbedProvider::new().expect("failed to init fastembed");
            let v = provider.embed("hello world");
            assert_eq!(v.len(), 384);
        }

        #[test]
        fn test_fastembed_deterministic() {
            let provider = FastEmbedProvider::new().expect("failed to init fastembed");
            let v1 = provider.embed("rust programming language");
            let v2 = provider.embed("rust programming language");
            assert_eq!(v1, v2);
        }

        #[test]
        fn test_fastembed_semantic_similarity() {
            let provider = FastEmbedProvider::new().expect("failed to init fastembed");
            let related_a = provider.embed("the cat sat on the mat");
            let related_b = provider.embed("a kitten rested on the rug");
            let unrelated = provider.embed("quantum computing algorithms");

            let sim_related = cosine_similarity(&related_a, &related_b);
            let sim_unrelated = cosine_similarity(&related_a, &unrelated);

            assert!(
                sim_related > sim_unrelated,
                "Related texts should be more similar ({} vs {})",
                sim_related,
                sim_unrelated
            );
        }
    }
}
