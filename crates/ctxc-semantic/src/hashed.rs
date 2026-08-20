//! The embedder CtxC ships with.
//!
//! It is feature hashing: text is broken into words, word pairs, and character
//! n-grams; each feature is hashed to a bucket and a sign; the buckets are
//! weighted and normalized. No model, no download, no network, and the same
//! input always produces the same vector — which is what lets it be the default
//! in a tool whose first design principle is determinism.
//!
//! ```text
//! "fn authenticate(user)"
//!   -> words     [fn, authenticate, user]
//!   -> pairs     [fn authenticate, authenticate user]
//!   -> 4-grams   [auth, uthe, then, hent, ...]
//!   -> hash each to a bucket ±1, weight, normalize
//! ```
//!
//! **What it can and cannot do.** Two functions that share names, or one that
//! is a renamed copy of another, come out close together. Two functions that do
//! the same thing in different words do not — this measures wording, not
//! meaning, and [`Fidelity::Lexical`] is how that is reported everywhere it
//! matters. Char n-grams buy robustness to case, morphology and typos
//! (`authenticate` / `Authentication` / `authenticator`), which is most of what
//! is wanted when searching code; they do not buy synonyms.
//!
//! A trained local model plugs in as another [`Embedder`](crate::Embedder)
//! without anything else changing.

use crate::vector::{Embedding, Fidelity};
use crate::Embedder;

/// Dimensions the default embedder uses.
///
/// Big enough that hash collisions are rare for a repository's vocabulary,
/// small enough that a vector is 1 KB and brute-force comparison over tens of
/// thousands of files stays instant.
pub const DEFAULT_DIMENSIONS: usize = 256;

/// Length of the character n-grams that give morphological robustness.
const NGRAM: usize = 4;

/// Words longer than this are hashed whole and not broken into n-grams; past
/// this length a token is a minified blob or a base64 payload, and grinding it
/// into n-grams adds noise rather than signal.
const MAX_WORD: usize = 32;

/// Features taken from one text.
///
/// Bounded so that embedding a 10 MB log file costs the same as embedding a
/// large source file. Content past this point is almost never what makes a
/// file distinctive.
const MAX_FEATURES: usize = 20_000;

/// Feature-hashing embedder.
#[derive(Debug, Clone, Copy)]
pub struct HashedEmbedder {
    dimensions: usize,
}

impl Default for HashedEmbedder {
    fn default() -> Self {
        HashedEmbedder::new(DEFAULT_DIMENSIONS)
    }
}

impl HashedEmbedder {
    /// Build an embedder. A zero dimension count is treated as the default,
    /// since a zero-dimension vector can only ever say "no idea".
    pub fn new(dimensions: usize) -> Self {
        HashedEmbedder {
            dimensions: if dimensions == 0 {
                DEFAULT_DIMENSIONS
            } else {
                dimensions
            },
        }
    }
}

impl Embedder for HashedEmbedder {
    fn name(&self) -> &str {
        "hashed"
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Lexical
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn embed(&self, text: &str) -> Embedding {
        let mut buckets = vec![0f32; self.dimensions];
        let mut features = 0usize;

        let add = |feature: &str, weight: f32, buckets: &mut Vec<f32>| {
            let hash = fnv1a(feature.as_bytes());
            let bucket = (hash % self.dimensions as u64) as usize;
            // The sign comes from a bit the bucket did not use, so collisions
            // cancel out on average instead of always reinforcing.
            let sign = if hash & 0x8000_0000_0000_0000 == 0 {
                1.0
            } else {
                -1.0
            };
            buckets[bucket] += sign * weight;
        };

        let words = tokenize(text);

        for word in &words {
            if features >= MAX_FEATURES {
                break;
            }
            // A whole word is the strongest evidence; its fragments support it.
            add(word, 1.0, &mut buckets);
            features += 1;

            if word.len() <= MAX_WORD {
                for gram in ngrams(word, NGRAM) {
                    if features >= MAX_FEATURES {
                        break;
                    }
                    add(&gram, 0.3, &mut buckets);
                    features += 1;
                }
            }
        }

        // Adjacent pairs capture a little word order — `read file` and
        // `file read` are not the same thing in code.
        for pair in words.windows(2) {
            if features >= MAX_FEATURES {
                break;
            }
            add(&format!("{} {}", pair[0], pair[1]), 0.5, &mut buckets);
            features += 1;
        }

        // Sublinear scaling: a word repeated a hundred times is more important
        // than one used once, but not a hundred times more.
        for bucket in &mut buckets {
            *bucket = bucket.signum() * bucket.abs().sqrt();
        }

        Embedding::new(buckets)
    }
}

/// Split text into lowercase words, breaking identifiers apart.
///
/// `parseHTTPResponse` becomes `parse`, `http`, `response`, because a query
/// written as "parse http response" should find it.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();

    // A run of capitals is one word until a capital is followed by a
    // lowercase, which is where the next word starts: HTTPResponse -> HTTP,
    // Response.
    let chars: Vec<char> = text.chars().collect();
    for (position, character) in chars.iter().enumerate() {
        if !character.is_alphanumeric() {
            flush(&mut current, &mut words);
            continue;
        }

        let starts_word = character.is_uppercase()
            && (chars
                .get(position.wrapping_sub(1))
                .is_some_and(|previous| previous.is_lowercase() || previous.is_numeric())
                || (chars
                    .get(position + 1)
                    .is_some_and(|next| next.is_lowercase())
                    && current.chars().count() > 1));

        if starts_word {
            flush(&mut current, &mut words);
        }
        current.extend(character.to_lowercase());
    }
    flush(&mut current, &mut words);

    words
}

fn flush(current: &mut String, words: &mut Vec<String>) {
    if !current.is_empty() {
        words.push(std::mem::take(current));
    }
}

/// Overlapping character n-grams of a word.
fn ngrams(word: &str, size: usize) -> Vec<String> {
    let characters: Vec<char> = word.chars().collect();
    if characters.len() <= size {
        return Vec::new();
    }

    characters
        .windows(size)
        .map(|window| window.iter().collect())
        .collect()
}

/// FNV-1a, 64-bit.
///
/// Written out rather than taken from a crate: the whole point is that this
/// number is stable forever, because changing it invalidates every stored
/// vector at once. A hash defined here cannot be changed by a dependency
/// update.
fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = OFFSET;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn embedder() -> HashedEmbedder {
        HashedEmbedder::default()
    }

    #[test]
    fn identifiers_are_broken_into_the_words_they_are_made_of() {
        assert_eq!(tokenize("parseHttpResponse"), ["parse", "http", "response"]);
        assert_eq!(
            tokenize("parse_http_response"),
            ["parse", "http", "response"]
        );
        assert_eq!(tokenize("HTTPResponse"), ["http", "response"]);
        assert_eq!(
            tokenize("fn authenticate(user)"),
            ["fn", "authenticate", "user"]
        );
        assert_eq!(tokenize("value2Text"), ["value2", "text"]);
    }

    #[test]
    fn tokenizing_nothing_yields_nothing() {
        assert!(tokenize("").is_empty());
        assert!(tokenize("   \n\t  ").is_empty());
        assert!(tokenize("---===---").is_empty());
    }

    #[test]
    fn the_same_text_always_gives_the_same_vector() {
        let text = "fn authenticate(user: &User) -> Result<Token>";
        assert_eq!(embedder().embed(text), embedder().embed(text));
    }

    #[test]
    fn identical_text_is_maximally_similar() {
        let embedding = embedder().embed("the quick brown fox");
        assert!((embedding.similarity(&embedding) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn text_that_shares_wording_scores_above_text_that_does_not() {
        let embedder = embedder();
        let subject = embedder.embed("fn authenticate(user) { check_token(user.token) }");
        let related = embedder.embed("fn authenticate_admin(user) { check_token(user.admin) }");
        let unrelated = embedder.embed("struct Rectangle { width: f64, height: f64 }");

        let close = subject.similarity(&related);
        let far = subject.similarity(&unrelated);

        assert!(close > far, "related {close} should beat unrelated {far}");
        assert!(
            close > 0.3,
            "related text should be clearly similar: {close}"
        );
    }

    #[test]
    fn morphology_and_case_do_not_hide_a_match() {
        let embedder = embedder();
        let base = embedder.embed("authentication");

        for variant in ["Authentication", "authenticate", "authenticator"] {
            let score = base.similarity(&embedder.embed(variant));
            assert!(
                score > 0.3,
                "{variant} should stay close to authentication: {score}"
            );
        }
    }

    #[test]
    fn synonyms_are_not_found_and_that_is_reported_honestly() {
        let embedder = embedder();
        // "log in" and "authenticate" mean the same thing. A lexical embedder
        // cannot know that, and says so through its fidelity rather than
        // pretending otherwise.
        let score = embedder
            .embed("authenticate the user")
            .similarity(&embedder.embed("sign in the person"));

        assert!(
            score < 0.3,
            "a lexical embedder has no way to know: {score}"
        );
        assert_eq!(embedder.fidelity(), Fidelity::Lexical);
    }

    #[test]
    fn word_order_makes_some_difference() {
        let embedder = embedder();
        let forward = embedder.embed("read file write buffer");
        let backward = embedder.embed("buffer write file read");

        let score = forward.similarity(&backward);
        assert!(score < 0.999, "adjacent pairs should differ: {score}");
        assert!(score > 0.5, "but the same words still matter: {score}");
    }

    #[test]
    fn empty_input_produces_a_vector_that_matches_nothing() {
        let empty = embedder().embed("");

        assert!(empty.is_empty());
        assert_eq!(empty.similarity(&embedder().embed("anything at all")), 0.0);
    }

    #[test]
    fn the_dimension_count_is_respected() {
        for dimensions in [16, 64, 512] {
            let embedder = HashedEmbedder::new(dimensions);
            assert_eq!(embedder.embed("some text").dimensions(), dimensions);
        }
        assert_eq!(
            HashedEmbedder::new(0).dimensions(),
            DEFAULT_DIMENSIONS,
            "a zero-dimension vector could only ever say `no idea`"
        );
    }

    #[test]
    fn a_huge_input_is_bounded_rather_than_unbounded() {
        let enormous = "word ".repeat(200_000);
        let started = std::time::Instant::now();
        let embedding = embedder().embed(&enormous);

        assert_eq!(embedding.dimensions(), DEFAULT_DIMENSIONS);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "feature extraction must stay bounded"
        );
    }

    #[test]
    fn the_hash_is_pinned_because_changing_it_invalidates_every_stored_vector() {
        // If this fails, every embedding in every user's database silently
        // stopped matching. That is a migration, not a refactor.
        //
        // The first two are the published FNV-1a 64 test vectors, which pin the
        // implementation to the real algorithm. The third is captured from it,
        // and pins the algorithm to this build.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"authenticate"), 0x4b2d_077a_4b38_7c2c);
    }
}
