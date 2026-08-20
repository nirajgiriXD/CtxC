//! Similarity, and the parts of CtxC that need it.
//!
//! ```text
//! text --> Embedder --> Embedding --> nearest / collapse_redundant / select_diverse
//! ```
//!
//! CtxC is deterministic first, and this crate does not change that. The
//! embedder that ships is feature hashing: no model, no download, no network,
//! and the same input always produces the same vector. A trained local model
//! can be added as another [`Embedder`] without anything else changing.
//!
//! **The honesty rule.** The default embedder measures *wording*, not
//! *meaning*, and every path that surfaces a number from it reports
//! [`Fidelity::Lexical`] alongside. Two functions that do the same thing in
//! different words will not be found by it. Presenting that as semantic search
//! would make a user trust an absence of results, which is the worst thing a
//! search tool can do.

pub mod error;
pub mod hashed;
pub mod similarity;
pub mod vector;

pub use error::{Result, SemanticError};
pub use hashed::{HashedEmbedder, DEFAULT_DIMENSIONS};
pub use similarity::{collapse_redundant, nearest, select_diverse, Match, Verdict};
pub use vector::{Embedding, Fidelity};

use std::sync::Arc;

/// Turns text into a vector.
///
/// Implementations must be deterministic: the same text and the same build
/// produce the same vector, forever. Stored embeddings are compared against
/// ones computed later, and a provider that drifts silently degrades every
/// result rather than failing loudly.
pub trait Embedder: Send + Sync {
    /// Stable identifier, stored alongside every vector this produces.
    ///
    /// Vectors from different providers are not comparable, and the name is
    /// how that is detected rather than discovered through bad results.
    fn name(&self) -> &str;

    /// What this provider's numbers are actually worth.
    fn fidelity(&self) -> Fidelity;

    fn dimensions(&self) -> usize;

    fn embed(&self, text: &str) -> Embedding;

    /// Embed several texts. Overridden by providers that batch.
    fn embed_all(&self, texts: &[&str]) -> Vec<Embedding> {
        texts.iter().map(|text| self.embed(text)).collect()
    }
}

/// How embeddings are configured.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticOptions {
    /// Whether anything embeds at all. Off by default: embedding costs index
    /// time and database size, and the deterministic path works without it.
    pub enabled: bool,
    pub provider: String,
    pub dimensions: usize,
    /// Similarity at which two pieces of text count as saying the same thing.
    pub redundancy_threshold: f32,
    /// How much relevance to trade for coverage when selecting. Zero is pure
    /// ranking.
    pub diversity: f32,
}

impl Default for SemanticOptions {
    fn default() -> Self {
        SemanticOptions {
            enabled: false,
            provider: "hashed".into(),
            dimensions: DEFAULT_DIMENSIONS,
            // Chosen so that log lines differing only in a timestamp collapse
            // and genuinely different text does not. Deliberately cautious:
            // wrongly dropping content is worse than keeping a repeat.
            redundancy_threshold: 0.92,
            diversity: 0.25,
        }
    }
}

impl SemanticOptions {
    pub fn from_config(config: &ctxc_core::Config) -> Self {
        SemanticOptions {
            enabled: config.semantic.enabled,
            provider: config.semantic.provider.clone(),
            dimensions: config.semantic.dimensions as usize,
            redundancy_threshold: config.semantic.redundancy_threshold as f32,
            diversity: config.semantic.diversity as f32,
        }
    }
}

/// Build the configured embedder.
///
/// An unknown provider name is an error rather than a silent fall back to the
/// default: someone who configured a model and got hashed vectors would have no
/// way to tell, and would conclude the model was bad.
pub fn embedder(options: &SemanticOptions) -> Result<Arc<dyn Embedder>> {
    match options.provider.as_str() {
        "hashed" => Ok(Arc::new(HashedEmbedder::new(options.dimensions))),
        other => Err(SemanticError::UnknownProvider {
            name: other.to_owned(),
            available: available_providers(),
        }),
    }
}

/// Provider names this build supports.
///
/// One today. A local model provider is the intended second, and the only thing
/// it has to satisfy is [`Embedder`].
pub fn available_providers() -> Vec<&'static str> {
    vec!["hashed"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_is_off_until_someone_turns_it_on() {
        let options = SemanticOptions::default();

        assert!(
            !options.enabled,
            "the deterministic path has to work without paying for embeddings"
        );
        assert_eq!(options.provider, "hashed");
    }

    #[test]
    fn the_default_provider_reports_itself_as_lexical() {
        let embedder = embedder(&SemanticOptions::default()).unwrap();

        assert_eq!(embedder.name(), "hashed");
        assert_eq!(embedder.fidelity(), Fidelity::Lexical);
        assert_eq!(embedder.dimensions(), DEFAULT_DIMENSIONS);
    }

    #[test]
    fn an_unknown_provider_is_refused_rather_than_quietly_replaced() {
        let options = SemanticOptions {
            provider: "some-model".into(),
            ..SemanticOptions::default()
        };

        let Err(error) = embedder(&options) else {
            panic!("a provider CtxC does not have must not resolve to a different one");
        };
        assert!(matches!(error, SemanticError::UnknownProvider { .. }));
        assert!(error.hint().unwrap().contains("hashed"));
    }

    #[test]
    fn the_configured_dimension_count_reaches_the_embedder() {
        let options = SemanticOptions {
            dimensions: 64,
            ..SemanticOptions::default()
        };

        assert_eq!(embedder(&options).unwrap().dimensions(), 64);
    }

    #[test]
    fn batching_gives_the_same_answer_as_embedding_one_at_a_time() {
        let embedder = HashedEmbedder::default();
        let texts = ["first text", "second text", ""];

        let batched = embedder.embed_all(&texts);
        for (index, text) in texts.iter().enumerate() {
            assert_eq!(batched[index], embedder.embed(text));
        }
    }

    #[test]
    fn options_come_from_configuration() {
        let mut config = ctxc_core::Config::default();
        config.semantic.enabled = true;
        config.semantic.dimensions = 128;

        let options = SemanticOptions::from_config(&config);
        assert!(options.enabled);
        assert_eq!(options.dimensions, 128);
    }
}
