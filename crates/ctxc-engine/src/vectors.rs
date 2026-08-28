//! A process-wide cache of decoded vectors.
//!
//! A similarity search compares a query against every vector a project has, so
//! every search used to read and decode the whole table again. In a one-shot
//! CLI invocation that is paid once; in the daemon, which answers search after
//! search over the same unchanged project, it is paid over and over for an
//! answer that has not changed.
//!
//! So the decoded vectors are kept, keyed by root and provider, and checked
//! against a summary the store can produce in one aggregate query. That check
//! is what makes the cache safe across processes: a CLI invocation re-indexing
//! while the daemon runs moves the summary, and the daemon reloads.
//!
//! Nothing here is required for correctness. A poisoned lock or a failed read
//! falls back to going to the database, which is what the code did before.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use ctxc_semantic::Embedding;
use ctxc_store::{EmbeddingState, EmbeddingStore, Provider};

use crate::error::Result;

/// Vectors for one root and provider, and the state they were read at.
struct Entry {
    state: EmbeddingState,
    vectors: Arc<Vec<(String, Embedding)>>,
}

/// How many roots are remembered at once.
///
/// A daemon looks after a handful of projects, not hundreds. The cap exists so
/// that a long-lived process cannot grow without bound, not to ration anything
/// anyone is using.
const MAX_ROOTS: usize = 16;

fn cache() -> &'static Mutex<HashMap<String, Entry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn key(root: &str, provider: Provider<'_>) -> String {
    format!("{root}\u{0}{}\u{0}{}", provider.name, provider.dimensions)
}

/// Every vector under a root, decoded, reading them only when they have moved.
pub(crate) fn load(
    store: &dyn EmbeddingStore,
    root: &str,
    provider: Provider<'_>,
) -> Result<Arc<Vec<(String, Embedding)>>> {
    let state = store.embedding_state(root, provider)?;
    let key = key(root, provider);

    if let Ok(cache) = cache().lock() {
        if let Some(entry) = cache.get(&key) {
            if entry.state == state {
                return Ok(Arc::clone(&entry.vectors));
            }
        }
    }

    let vectors: Arc<Vec<(String, Embedding)>> = Arc::new(
        store
            .embeddings(root, provider)?
            .into_iter()
            .map(|stored| (stored.path, stored.embedding))
            .collect(),
    );

    if let Ok(mut cache) = cache().lock() {
        // Evicting everything rather than the least recently used one: the
        // bound is reached by a process looking after more roots than CtxC
        // expects, and at that point precision about which to drop is worth
        // less than the code it takes.
        if cache.len() >= MAX_ROOTS && !cache.contains_key(&key) {
            cache.clear();
        }
        cache.insert(
            key,
            Entry {
                state,
                vectors: Arc::clone(&vectors),
            },
        );
    }
    Ok(vectors)
}

/// Forget everything cached. For tests, and for anything that must not read a
/// vector it put there itself.
pub fn clear() {
    if let Ok(mut cache) = cache().lock() {
        cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_semantic::{Embedder, HashedEmbedder};
    use ctxc_store::{Database, SqliteEmbeddingStore};

    const ROOT: &str = "/work/vectors";

    fn provider() -> Provider<'static> {
        Provider {
            name: "hashed",
            dimensions: HashedEmbedder::default().dimensions(),
        }
    }

    #[test]
    fn a_second_load_returns_the_same_decoded_vectors() {
        clear();
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);
        let embedder = HashedEmbedder::default();
        store
            .put_embedding(ROOT, "a.rs", provider(), &embedder.embed("hello"), "h1")
            .unwrap();

        let first = load(&store, ROOT, provider()).unwrap();
        let second = load(&store, ROOT, provider()).unwrap();

        assert!(
            Arc::ptr_eq(&first, &second),
            "an unchanged project was read twice"
        );
    }

    #[test]
    fn writing_a_vector_invalidates_what_was_cached() {
        clear();
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);
        let embedder = HashedEmbedder::default();
        store
            .put_embedding(ROOT, "a.rs", provider(), &embedder.embed("hello"), "h1")
            .unwrap();

        let first = load(&store, ROOT, provider()).unwrap();
        assert_eq!(first.len(), 1);

        store
            .put_embedding(ROOT, "b.rs", provider(), &embedder.embed("world"), "h2")
            .unwrap();

        let second = load(&store, ROOT, provider()).unwrap();
        assert_eq!(second.len(), 2, "a new vector was not picked up");
        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn deleting_a_vector_invalidates_what_was_cached() {
        clear();
        let database = Database::open_in_memory().unwrap();
        let store = SqliteEmbeddingStore::new(&database);
        let embedder = HashedEmbedder::default();
        for path in ["a.rs", "b.rs"] {
            store
                .put_embedding(ROOT, path, provider(), &embedder.embed(path), "h")
                .unwrap();
        }

        assert_eq!(load(&store, ROOT, provider()).unwrap().len(), 2);
        store.delete_embedding(ROOT, "a.rs").unwrap();
        assert_eq!(
            load(&store, ROOT, provider()).unwrap().len(),
            1,
            "a deleted vector kept turning up"
        );
    }
}
