//! Joining the embedder to the store.
//!
//! `ctxc-semantic` knows how to turn text into a vector and how to compare two
//! of them. `ctxc-store` knows how to keep vectors. Neither knows about the
//! other, which is deliberate — and this is the one place that holds both, so
//! indexing, searching and the MCP tools all embed the same way rather than
//! three slightly different ways.
//!
//! ```text
//! index  --> embed(content) --> EmbeddingStore
//! search --> embed(query)   --> compare against stored vectors --> ranking signal
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use ctxc_retrieval::SimilaritySource;
use ctxc_semantic::{Embedder, Embedding, Fidelity, SemanticOptions};
use ctxc_store::{EmbeddingStore, Provider};

use crate::error::Result;

/// Embeds a project's files, and answers how close they are to a query.
pub struct ProjectEmbedder<'a> {
    embedder: Arc<dyn Embedder>,
    store: &'a dyn EmbeddingStore,
    root: String,
}

impl<'a> ProjectEmbedder<'a> {
    pub fn new(embedder: Arc<dyn Embedder>, store: &'a dyn EmbeddingStore, root: &str) -> Self {
        ProjectEmbedder {
            embedder,
            store,
            root: root.to_owned(),
        }
    }

    /// Build one from configuration, or `None` when embeddings are off.
    ///
    /// Returning `None` rather than a no-op embedder keeps the caller honest:
    /// there is no vector to look up, and code that would have shown a
    /// similarity number has to notice.
    pub fn from_options(
        options: &SemanticOptions,
        store: &'a dyn EmbeddingStore,
        root: &str,
    ) -> Result<Option<Self>> {
        if !options.enabled {
            return Ok(None);
        }

        let embedder = ctxc_semantic::embedder(options)?;
        Ok(Some(ProjectEmbedder::new(embedder, store, root)))
    }

    /// What this provider's numbers are worth, for anything that shows one.
    pub fn fidelity(&self) -> Fidelity {
        self.embedder.fidelity()
    }

    pub fn provider(&self) -> Provider<'_> {
        Provider {
            name: self.embedder.name(),
            dimensions: self.embedder.dimensions(),
        }
    }

    /// Embed a file, unless an identical one is already stored.
    ///
    /// Returns whether it did any work, which is what lets an index pass report
    /// embedding separately from parsing.
    pub fn embed_file(&self, path: &str, content: &str, content_hash: &str) -> Result<bool> {
        let stored = self.store.embedding(&self.root, path, self.provider())?;
        if stored.is_some_and(|stored| stored.content_hash == content_hash) {
            return Ok(false);
        }

        let embedding = self.embedder.embed(content);
        self.store
            .put_embedding(&self.root, path, self.provider(), &embedding, content_hash)?;
        Ok(true)
    }

    /// Forget a file's vector.
    pub fn forget(&self, path: &str) -> Result<()> {
        self.store.delete_embedding(&self.root, path)?;
        Ok(())
    }

    /// How many files have a usable vector.
    pub fn count(&self) -> Result<u64> {
        Ok(self.store.embedding_count(&self.root, self.provider())?)
    }

    /// Embed a query, ready to rank a candidate set against.
    pub fn for_query(&self, query: &str) -> QuerySimilarity<'_> {
        QuerySimilarity {
            query: self.embedder.embed(query),
            project: self,
        }
    }

    /// The files most similar to `text`, best first.
    ///
    /// Brute force over the project's vectors. A repository has thousands of
    /// files, and comparing a query against all of them costs less than the
    /// index lookup that would have narrowed them down — and the vectors
    /// themselves are decoded once and reused until the project changes.
    pub fn nearest(&self, text: &str, limit: usize, floor: f32) -> Result<Similar> {
        let query = self.embedder.embed(text);
        // Decoded once per change to the project rather than once per query:
        // the daemon answers search after search over vectors that did not
        // move, and reading them again each time is the whole cost.
        let stored = crate::vectors::load(self.store, &self.root, self.provider())?;

        let candidates = stored
            .iter()
            .map(|(path, embedding)| (path.clone(), embedding.clone()));
        let matches = ctxc_semantic::nearest(&query, candidates, limit, floor);

        Ok(Similar {
            fidelity: self.embedder.fidelity(),
            provider: self.embedder.name().to_owned(),
            files: matches
                .into_iter()
                .map(|found| (found.item, found.similarity))
                .collect(),
        })
    }
}

/// What a similarity search found, and what its numbers are worth.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Similar {
    /// Whether these numbers track wording or meaning.
    pub fidelity: Fidelity,
    pub provider: String,
    /// Path and similarity, best first.
    pub files: Vec<(String, f32)>,
}

/// One query's view of a project's vectors.
///
/// Holds the query embedded once, rather than re-embedding it for every
/// candidate. Ranking asks for the whole candidate set at once, so this is
/// computed once per search.
pub struct QuerySimilarity<'a> {
    project: &'a ProjectEmbedder<'a>,
    query: Embedding,
}

impl SimilaritySource for QuerySimilarity<'_> {
    fn similarities(&self, paths: &[String]) -> HashMap<String, f64> {
        if self.query.is_empty() {
            return HashMap::new();
        }

        // A failure here costs the semantic signal and nothing else: the
        // keyword, symbol and graph signals still rank the results, and a
        // search that comes back slightly worse ordered beats one that errors.
        let stored = match self.project.store.embeddings_for(
            &self.project.root,
            paths,
            self.project.provider(),
        ) {
            Ok(stored) => stored,
            Err(err) => {
                tracing::debug!(error = %err, "could not read embeddings for ranking");
                return HashMap::new();
            }
        };

        stored
            .into_iter()
            .map(|stored| (stored.path, self.query.similarity(&stored.embedding) as f64))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_semantic::HashedEmbedder;
    use ctxc_store::{Database, SqliteEmbeddingStore};

    const ROOT: &str = "/work/app";

    fn embedder<'a>(store: &'a SqliteEmbeddingStore<'a>) -> ProjectEmbedder<'a> {
        ProjectEmbedder::new(Arc::new(HashedEmbedder::default()), store, ROOT)
    }

    #[test]
    fn embedding_is_skipped_when_the_content_has_not_changed() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);
        let project = embedder(&store);

        assert!(project.embed_file("a.rs", "fn one() {}", "hash-1").unwrap());
        assert!(
            !project.embed_file("a.rs", "fn one() {}", "hash-1").unwrap(),
            "re-embedding identical content is wasted work"
        );
        assert!(project.embed_file("a.rs", "fn two() {}", "hash-2").unwrap());
    }

    #[test]
    fn the_nearest_file_is_the_one_that_shares_wording() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);
        let project = embedder(&store);

        project
            .embed_file(
                "auth.rs",
                "fn authenticate(user) { check_token(user) }",
                "h1",
            )
            .unwrap();
        project
            .embed_file("shapes.rs", "struct Rectangle { width, height }", "h2")
            .unwrap();

        let found = project.nearest("authenticate user token", 5, 0.0).unwrap();
        assert_eq!(found.files[0].0, "auth.rs", "{found:?}");
        assert_eq!(
            found.fidelity,
            Fidelity::Lexical,
            "a caller showing these numbers has to be able to say what they are"
        );
    }

    #[test]
    fn a_forgotten_file_stops_being_a_candidate() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);
        let project = embedder(&store);

        project.embed_file("gone.rs", "some content", "h").unwrap();
        assert_eq!(project.count().unwrap(), 1);

        project.forget("gone.rs").unwrap();
        assert_eq!(project.count().unwrap(), 0);
        assert!(project.nearest("content", 5, 0.0).unwrap().files.is_empty());
    }

    #[test]
    fn ranking_similarity_covers_only_the_candidates_it_was_given() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);
        let project = embedder(&store);

        project
            .embed_file("auth.rs", "fn authenticate(user) token", "h1")
            .unwrap();
        project
            .embed_file("shapes.rs", "struct Rectangle width height", "h2")
            .unwrap();

        let similarity = project.for_query("authenticate token");
        let scores = similarity.similarities(&["auth.rs".to_string()]);

        assert_eq!(scores.len(), 1, "only what was asked for: {scores:?}");
        assert!(scores["auth.rs"] > 0.2, "{scores:?}");
    }

    #[test]
    fn an_empty_query_contributes_nothing_rather_than_zero_for_everything() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);
        let project = embedder(&store);
        project.embed_file("a.rs", "content", "h").unwrap();

        let similarity = project.for_query("");
        assert!(similarity.similarities(&["a.rs".to_string()]).is_empty());
    }

    #[test]
    fn a_file_with_no_vector_is_absent_rather_than_scored_zero() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);
        let project = embedder(&store);
        project.embed_file("a.rs", "content", "h").unwrap();

        let similarity = project.for_query("content");
        let scores =
            similarity.similarities(&["a.rs".to_string(), "never-embedded.rs".to_string()]);

        assert!(scores.contains_key("a.rs"));
        assert!(
            !scores.contains_key("never-embedded.rs"),
            "no evidence and no similarity are different things: {scores:?}"
        );
    }

    #[test]
    fn embeddings_stay_off_until_configuration_turns_them_on() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);

        let options = SemanticOptions::default();
        assert!(ProjectEmbedder::from_options(&options, &store, ROOT)
            .unwrap()
            .is_none());

        let enabled = SemanticOptions {
            enabled: true,
            ..SemanticOptions::default()
        };
        assert!(ProjectEmbedder::from_options(&enabled, &store, ROOT)
            .unwrap()
            .is_some());
    }
}
