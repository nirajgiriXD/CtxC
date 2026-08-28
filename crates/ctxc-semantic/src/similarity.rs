//! Finding what is close, and dropping what is redundant.
//!
//! Searching is brute force. A repository has thousands of files, not millions
//! of them, and comparing a query against ten thousand 256-float vectors is a
//! few milliseconds — cheaper than the index lookup that produced the
//! candidates. An approximate index would add a structure to build, invalidate
//! and get wrong, in exchange for time nobody is waiting on.
//!
//! Deduplication is the one that grows: every fragment against every fragment
//! kept so far, which is quadratic exactly when nothing is a duplicate — the
//! normal case for source code. So each comparison stops as soon as the pair
//! provably cannot reach the threshold, which for two unrelated fragments is
//! after the first eighth of the vector.
//!
//! The bound that decides this only ever over-estimates, deliberately. A
//! random-hyperplane sketch would reject pairs faster and miss real duplicates
//! a few percent of the time, which would make what CtxC decides depend on
//! which sketch it happened to draw. This changes how fast the answer is
//! reached and never what it is.

use crate::vector::Embedding;

/// How many pieces a vector is cut into when comparing it.
///
/// Each piece is a chance to stop early. Eight over 256 dimensions means a
/// pair that cannot possibly reach the threshold is usually rejected after the
/// first 32 multiplies instead of all 256.
const PIECES: usize = 8;

/// Below this many fragments, the plain scan is already fast and exactly
/// right, and building the bounds would cost more than they save.
const PRUNE_FROM: usize = 64;

/// Per-suffix magnitudes of a vector, for stopping a comparison early.
///
/// `suffix[i]` is the magnitude of everything from piece `i` onwards. Part way
/// through a dot product, Cauchy-Schwarz says the pieces not yet looked at can
/// contribute at most the product of the two vectors' remaining magnitudes. So
/// "what we have so far, plus that product" is an upper bound on the final
/// answer, and once it drops below the threshold the rest of the comparison
/// cannot change the verdict.
///
/// The bound only ever over-estimates, which is what makes this an
/// optimization rather than a different answer: no pair that would have been
/// called a duplicate is ever rejected by it.
#[derive(Debug, Clone)]
struct Bounds {
    /// One longer than `PIECES`, so the last entry is zero: nothing remains
    /// after the final piece, and the bound there is the exact answer.
    suffix: [f32; PIECES + 1],
    /// Dimensions per piece, so both sides walk the same boundaries.
    piece: usize,
}

impl Bounds {
    fn of(embedding: &Embedding) -> Bounds {
        let values = embedding.values();
        let mut suffix = [0.0f32; PIECES + 1];
        if values.is_empty() {
            return Bounds { suffix, piece: 1 };
        }

        // Ceiling division: the last piece takes the remainder, so no dimension
        // falls outside the bound. One that did would make the bound an
        // under-estimate, and the prune wrong.
        let piece = values.len().div_ceil(PIECES);
        for index in (0..PIECES).rev() {
            let from = (index * piece).min(values.len());
            let to = ((index + 1) * piece).min(values.len());
            let squared: f32 = values[from..to].iter().map(|value| value * value).sum();
            suffix[index] = (squared + suffix[index + 1] * suffix[index + 1]).sqrt();
        }
        Bounds { suffix, piece }
    }
}

/// Whether two vectors are at least `threshold` similar, stopping as soon as
/// the answer cannot be yes.
///
/// Equivalent to `left.similarity(right) >= threshold`, and it agrees with it
/// exactly; the bound decides when to stop, never what to conclude.
fn at_least(
    left: &Embedding,
    right: &Embedding,
    bounds: (&Bounds, &Bounds),
    threshold: f32,
) -> bool {
    let (a, b) = (left.values(), right.values());
    if a.len() != b.len() {
        // Vectors of different lengths score zero, exactly as `similarity`
        // says: two providers' vectors are not comparable.
        return 0.0 >= threshold;
    }

    let piece = bounds.0.piece;
    let mut total = 0.0f32;

    for index in 0..PIECES {
        let from = (index * piece).min(a.len());
        let to = ((index + 1) * piece).min(a.len());
        total += a[from..to]
            .iter()
            .zip(&b[from..to])
            .map(|(left, right)| left * right)
            .sum::<f32>();

        // The most the pieces not yet looked at could add.
        let remaining = bounds.0.suffix[index + 1] * bounds.1.suffix[index + 1];
        if total + remaining < threshold {
            return false;
        }
    }

    total.clamp(-1.0, 1.0) >= threshold
}

/// One scored candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Match<T> {
    pub item: T,
    /// Cosine similarity to the query, in `-1..=1`.
    pub similarity: f32,
}

/// The `limit` candidates closest to `query`, best first.
///
/// Candidates below `floor` are dropped. Without a floor, a query matches
/// everything a little, and "a little" presented as a result reads as evidence.
pub fn nearest<T: Clone>(
    query: &Embedding,
    candidates: impl IntoIterator<Item = (T, Embedding)>,
    limit: usize,
    floor: f32,
) -> Vec<Match<T>> {
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }

    let mut scored: Vec<Match<T>> = candidates
        .into_iter()
        .map(|(item, embedding)| Match {
            similarity: query.similarity(&embedding),
            item,
        })
        .filter(|found| found.similarity >= floor)
        .collect();

    // Descending by similarity. `total_cmp` rather than `partial_cmp`, so a
    // stray NaN sorts somewhere defined instead of panicking.
    scored.sort_by(|left, right| right.similarity.total_cmp(&left.similarity));
    scored.truncate(limit);
    scored
}

/// What [`collapse_redundant`] decided about one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing already kept says the same thing.
    Keep,
    /// This repeats an item already kept, at the given index.
    Redundant { duplicate_of: usize },
}

/// Drop items that repeat something already kept.
///
/// This is deduplication that survives rewording: two log lines differing only
/// in a timestamp, two error blocks describing the same failure, the same
/// boilerplate in four files. Exact-match deduplication misses all of them.
///
/// Order is the priority order — the caller has already decided what matters
/// most, and the first thing to say something wins. Comparison is against
/// everything *kept*, not everything seen, so a chain of slightly-different
/// items does not drift away from where it started.
pub fn collapse_redundant(embeddings: &[Embedding], threshold: f32) -> Vec<Verdict> {
    // The bounds only earn their keep once there are enough fragments for the
    // quadratic term to matter. Below that the plain scan is the whole job.
    let bounds: Option<Vec<Bounds>> =
        (embeddings.len() >= PRUNE_FROM).then(|| embeddings.iter().map(Bounds::of).collect());

    let mut verdicts = Vec::with_capacity(embeddings.len());
    let mut kept: Vec<usize> = Vec::new();

    for (index, embedding) in embeddings.iter().enumerate() {
        // An item with no signal cannot be shown to repeat anything, and
        // dropping it on that basis would be a guess.
        if embedding.is_empty() {
            verdicts.push(Verdict::Keep);
            kept.push(index);
            continue;
        }

        let duplicate = kept.iter().find(|&&other| match &bounds {
            Some(bounds) => at_least(
                &embeddings[other],
                embedding,
                (&bounds[other], &bounds[index]),
                threshold,
            ),
            None => embeddings[other].similarity(embedding) >= threshold,
        });

        match duplicate {
            Some(&other) => verdicts.push(Verdict::Redundant {
                duplicate_of: other,
            }),
            None => {
                verdicts.push(Verdict::Keep);
                kept.push(index);
            }
        }
    }

    verdicts
}

/// Choose items that are relevant *and* different from each other.
///
/// Maximal marginal relevance. Ranking alone fills a budget with the ten most
/// relevant files, which in a real repository are often near-copies of one
/// another — the caller spends its whole budget learning one thing. This trades
/// a little relevance for coverage.
///
/// `diversity` is how much that trade is worth: 0.0 is pure ranking, 1.0 picks
/// the most different item each time regardless of relevance.
pub fn select_diverse(
    relevance: &[f32],
    embeddings: &[Embedding],
    limit: usize,
    diversity: f32,
) -> Vec<usize> {
    let count = relevance.len().min(embeddings.len());
    if count == 0 || limit == 0 {
        return Vec::new();
    }

    let diversity = diversity.clamp(0.0, 1.0);
    let mut chosen: Vec<usize> = Vec::new();
    let mut remaining: Vec<usize> = (0..count).collect();

    while chosen.len() < limit.min(count) {
        let best = remaining
            .iter()
            .enumerate()
            .max_by(|(_, &left), (_, &right)| {
                marginal(left, &chosen, relevance, embeddings, diversity)
                    .total_cmp(&marginal(right, &chosen, relevance, embeddings, diversity))
            })
            .map(|(position, _)| position);

        match best {
            Some(position) => chosen.push(remaining.remove(position)),
            None => break,
        }
    }

    chosen
}

/// One candidate's value given what has already been chosen.
fn marginal(
    candidate: usize,
    chosen: &[usize],
    relevance: &[f32],
    embeddings: &[Embedding],
    diversity: f32,
) -> f32 {
    let score = relevance[candidate];
    if chosen.is_empty() {
        return score;
    }

    let closest = chosen
        .iter()
        .map(|&other| embeddings[candidate].similarity(&embeddings[other]))
        .fold(f32::MIN, f32::max);

    (1.0 - diversity) * score - diversity * closest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hashed::HashedEmbedder;
    use crate::Embedder;

    fn embed(texts: &[&str]) -> Vec<Embedding> {
        let embedder = HashedEmbedder::default();
        texts.iter().map(|text| embedder.embed(text)).collect()
    }

    #[test]
    fn the_closest_candidate_comes_first() {
        let embedder = HashedEmbedder::default();
        let query = embedder.embed("authenticate user token");

        let candidates = vec![
            ("rectangle", embedder.embed("struct Rectangle width height")),
            ("auth", embedder.embed("fn authenticate(user) checks token")),
            ("logger", embedder.embed("fn log(message) writes to file")),
        ];

        let found = nearest(&query, candidates, 3, 0.0);
        assert_eq!(found[0].item, "auth", "{found:?}");
    }

    #[test]
    fn a_floor_keeps_weak_matches_from_looking_like_evidence() {
        let embedder = HashedEmbedder::default();
        let query = embedder.embed("authenticate user");
        let candidates = vec![("unrelated", embedder.embed("rectangle geometry area"))];

        assert!(nearest(&query, candidates.clone(), 5, 0.0).len() <= 1);
        assert!(
            nearest(&query, candidates, 5, 0.5).is_empty(),
            "a weak match presented as a result reads as evidence"
        );
    }

    #[test]
    fn a_query_with_no_signal_returns_nothing() {
        let embedder = HashedEmbedder::default();
        let candidates = vec![("anything", embedder.embed("some text"))];

        assert!(nearest(&embedder.embed(""), candidates, 5, 0.0).is_empty());
    }

    #[test]
    fn the_limit_is_respected() {
        let embedder = HashedEmbedder::default();
        let query = embedder.embed("word");
        let candidates: Vec<(usize, Embedding)> = (0..10)
            .map(|index| (index, embedder.embed("word")))
            .collect();

        assert_eq!(nearest(&query, candidates.clone(), 3, 0.0).len(), 3);
        assert!(nearest(&query, candidates, 0, 0.0).is_empty());
    }

    #[test]
    fn rewording_does_not_hide_a_repeat() {
        // Exact-match deduplication keeps all four of these.
        let embeddings = embed(&[
            "ERROR 12:00:01 connection to database failed after 3 retries",
            "ERROR 12:00:04 connection to database failed after 3 retries",
            "ERROR 12:00:09 connection to database failed after 3 retries",
            "INFO  12:00:11 listening on port 8080",
        ]);

        let verdicts = collapse_redundant(&embeddings, 0.9);
        assert_eq!(verdicts[0], Verdict::Keep);
        assert!(matches!(
            verdicts[1],
            Verdict::Redundant { duplicate_of: 0 }
        ));
        assert!(matches!(
            verdicts[2],
            Verdict::Redundant { duplicate_of: 0 }
        ));
        assert_eq!(verdicts[3], Verdict::Keep, "a different line survives");
    }

    #[test]
    fn a_high_threshold_only_collapses_near_copies() {
        let embeddings = embed(&["the cat sat on the mat", "a dog stood on the floor"]);

        let verdicts = collapse_redundant(&embeddings, 0.95);
        assert!(
            verdicts.iter().all(|verdict| *verdict == Verdict::Keep),
            "different content must survive: {verdicts:?}"
        );
    }

    #[test]
    fn everything_is_compared_against_what_was_kept_not_what_was_seen() {
        // Without that, a chain of small changes drifts: b is close to a, c is
        // close to b, and c ends up dropped despite being unlike a.
        let embeddings = embed(&["alpha beta gamma", "alpha beta gamma", "alpha beta gamma"]);
        let verdicts = collapse_redundant(&embeddings, 0.9);

        for verdict in &verdicts[1..] {
            assert!(matches!(verdict, Verdict::Redundant { duplicate_of: 0 }));
        }
    }

    #[test]
    fn an_item_with_no_signal_is_never_called_a_duplicate() {
        let embeddings = embed(&["", "", "real content here"]);
        let verdicts = collapse_redundant(&embeddings, 0.5);

        assert!(
            verdicts.iter().all(|verdict| *verdict == Verdict::Keep),
            "dropping something on no evidence is a guess: {verdicts:?}"
        );
    }

    #[test]
    fn collapsing_nothing_returns_nothing() {
        assert!(collapse_redundant(&[], 0.9).is_empty());
    }

    /// The early exit decides when to stop, never what to conclude. Every pair
    /// it rules on must match what measuring the whole vector would have said.
    #[test]
    fn stopping_early_never_changes_the_answer() {
        let embedder = HashedEmbedder::default();
        let embeddings: Vec<Embedding> = (0..150)
            .map(|index| {
                embedder.embed(&format!(
                    "worker {} handled request {index} in {} ms",
                    index % 5,
                    index % 17
                ))
            })
            .collect();
        let bounds: Vec<Bounds> = embeddings.iter().map(Bounds::of).collect();

        for threshold in [0.0, 0.3, 0.7, 0.92, 0.99] {
            for left in 0..embeddings.len() {
                for right in 0..embeddings.len() {
                    let exact = embeddings[left].similarity(&embeddings[right]) >= threshold;
                    let bounded = at_least(
                        &embeddings[left],
                        &embeddings[right],
                        (&bounds[left], &bounds[right]),
                        threshold,
                    );
                    assert_eq!(
                        bounded, exact,
                        "pair ({left}, {right}) at threshold {threshold}"
                    );
                }
            }
        }
    }

    /// Pruning is an optimization, so the verdicts either side of the size at
    /// which it switches on must be the same verdicts.
    #[test]
    fn pruning_reaches_the_same_verdicts_as_the_exact_scan() {
        let embedder = HashedEmbedder::default();
        let mut texts: Vec<String> = Vec::new();
        for index in 0..PRUNE_FROM * 2 {
            // A third of these repeat an earlier line with a different
            // timestamp, so there is something real for both paths to find.
            if index % 3 == 0 {
                texts.push(format!(
                    "ERROR 12:00:{index:02} connection to database failed"
                ));
            } else {
                texts.push(format!(
                    "INFO  12:00:{index:02} handled request {index} for user {index}"
                ));
            }
        }
        let embeddings: Vec<Embedding> = texts.iter().map(|text| embedder.embed(text)).collect();

        for threshold in [0.5, 0.8, 0.9, 0.95] {
            let pruned = collapse_redundant(&embeddings, threshold);

            // The same input, one item short of the size that turns pruning on.
            let mut exact = Vec::with_capacity(embeddings.len());
            let mut kept: Vec<usize> = Vec::new();
            for (index, embedding) in embeddings.iter().enumerate() {
                if embedding.is_empty() {
                    exact.push(Verdict::Keep);
                    kept.push(index);
                    continue;
                }
                match kept
                    .iter()
                    .find(|&&other| embeddings[other].similarity(embedding) >= threshold)
                {
                    Some(&other) => exact.push(Verdict::Redundant {
                        duplicate_of: other,
                    }),
                    None => {
                        exact.push(Verdict::Keep);
                        kept.push(index);
                    }
                }
            }

            assert_eq!(
                pruned, exact,
                "pruning changed what was decided at threshold {threshold}"
            );
        }
    }

    #[test]
    fn pure_ranking_keeps_the_original_order() {
        let embeddings = embed(&["one", "two", "three"]);
        let chosen = select_diverse(&[0.9, 0.8, 0.7], &embeddings, 3, 0.0);

        assert_eq!(chosen, [0, 1, 2]);
    }

    #[test]
    fn diversity_breaks_up_a_run_of_near_copies() {
        // The three highest-ranked items say the same thing; the fourth is the
        // only one that adds anything.
        let embeddings = embed(&[
            "database connection pool timeout setting",
            "database connection pool timeout config",
            "database connection pool timeout option",
            "rendering the user interface layout",
        ]);

        let ranked = [1.0, 0.95, 0.9, 0.5];
        let chosen = select_diverse(&ranked, &embeddings, 2, 0.5);

        assert_eq!(chosen[0], 0, "the best result still comes first");
        assert_eq!(
            chosen[1], 3,
            "a budget spent on three ways of saying one thing is wasted: {chosen:?}"
        );
    }

    #[test]
    fn selection_never_returns_more_than_there_is() {
        let embeddings = embed(&["one", "two"]);

        assert_eq!(select_diverse(&[1.0, 0.5], &embeddings, 10, 0.3).len(), 2);
        assert!(select_diverse(&[], &[], 5, 0.3).is_empty());
        assert!(select_diverse(&[1.0], &embeddings, 0, 0.3).is_empty());
    }

    #[test]
    fn every_item_is_chosen_at_most_once() {
        let embeddings = embed(&["alpha", "beta", "gamma", "delta"]);
        let chosen = select_diverse(&[1.0, 0.9, 0.8, 0.7], &embeddings, 4, 0.7);

        let unique: std::collections::HashSet<usize> = chosen.iter().copied().collect();
        assert_eq!(unique.len(), chosen.len(), "{chosen:?}");
    }
}
