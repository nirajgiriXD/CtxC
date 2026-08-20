//! Hybrid retrieval.
//!
//! ```text
//!            query
//!              |
//!     +--------+--------+
//!     |                 |
//! full text         symbol names
//!     |                 |
//!     +--------+--------+
//!              |
//!       graph expansion
//!              |
//!           ranking
//!              |
//!      relevant context
//! ```
//!
//! Two lookups find files the query names directly; the graph then pulls in
//! what those files depend on, because the file that answers a question is
//! often not the file that mentions it. Everything is scored together, so an
//! expanded file can outrank a weak direct match — with a decay per hop so it
//! usually does not.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use ctxc_core::{TokenBudget, Tokenizer};
use ctxc_graph::DependencyGraph;
use ctxc_store::IndexStore;

use crate::error::Result;
use crate::query::Query;
use crate::rank::{self, RankingWeights, Signals};

/// How retrieval should behave.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetrievalOptions {
    pub weights: RankingWeights,
    /// Results returned.
    pub limit: usize,
    /// Candidates pulled from each lookup before ranking.
    pub candidates: u32,
    /// Graph hops to expand. Zero disables expansion.
    pub expansion_depth: usize,
    /// Days after which a file counts as half as recent.
    pub recency_half_life_days: f64,
    /// Lines of context shown around a match.
    pub snippet_lines: usize,
}

impl Default for RetrievalOptions {
    fn default() -> Self {
        RetrievalOptions {
            weights: RankingWeights::default(),
            limit: 20,
            candidates: 50,
            expansion_depth: 1,
            recency_half_life_days: 30.0,
            snippet_lines: 3,
        }
    }
}

impl RetrievalOptions {
    /// Read the options out of a loaded configuration.
    ///
    /// The weights live in configuration because what makes a file relevant is
    /// not the same in every repository; the defaults are a starting point, not
    /// a law.
    pub fn from_config(config: &ctxc_core::Config) -> Self {
        RetrievalOptions {
            weights: RankingWeights {
                keyword: config.ranking.keyword,
                semantic: config.ranking.semantic,
                symbol: config.ranking.symbol,
                graph: config.ranking.graph,
                recency: config.ranking.recency,
                hop_decay: config.ranking.hop_decay,
            },
            expansion_depth: config.ranking.expansion_depth as usize,
            recency_half_life_days: config.ranking.recency_half_life_days,
            ..RetrievalOptions::default()
        }
    }
}

/// Why a file is in the results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reason {
    /// The query matched this file's content.
    Content,
    /// The query matched a symbol defined here.
    Symbol,
    /// Both.
    ContentAndSymbol,
    /// Reached through the dependency graph from another result.
    Related { to: String, hops: u32 },
}

/// One file worth reading, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievedFile {
    pub path: String,
    pub language: Option<String>,
    pub score: f64,
    pub signals: Signals,
    pub reason: Reason,
    /// Symbols in this file whose names matched.
    pub matched_symbols: Vec<String>,
    /// Line the match was found on, when there is one.
    pub line: Option<u32>,
    /// A few lines around the match.
    pub snippet: Option<String>,
}

/// What a search found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Retrieval {
    pub query: String,
    pub root: String,
    /// Files examined before ranking cut the list down.
    pub considered: usize,
    pub files: Vec<RetrievedFile>,
}

impl Retrieval {
    /// Choose files to fill a token budget, in rank order.
    ///
    /// Files are taken most-relevant first until the budget is spoken for. A
    /// file larger than what is left is still taken rather than skipped: the
    /// optimizer trims it to fit, and the reference keeps the whole thing
    /// reachable. Skipping it instead would mean answering a question with the
    /// second-best file simply because the best one was long, which is the
    /// opposite of what ranking is for.
    pub fn select_within_budget(
        &self,
        budget: TokenBudget,
        tokenizer: &dyn Tokenizer,
        content_of: impl Fn(&str) -> Option<String>,
    ) -> Vec<String> {
        let mut selected = Vec::new();
        let mut spent = 0u32;

        for file in &self.files {
            if spent >= budget.total() {
                break;
            }
            let Some(content) = content_of(&file.path) else {
                continue;
            };
            let cost = tokenizer.count(&content).min(budget.total() - spent);
            spent = spent.saturating_add(cost);
            selected.push(file.path.clone());
        }
        selected
    }
}

/// Runs hybrid searches against an index.
pub struct Retriever<'a> {
    store: &'a dyn IndexStore,
    options: RetrievalOptions,
    similarity: Option<&'a dyn SimilaritySource>,
}

/// Where embedding similarity comes from, when it is available.
///
/// A trait rather than a store handle, so that retrieval keeps knowing nothing
/// about databases or embedders. Ranking asks "how close is each of these
/// candidates to the query?" and does not care who answers.
pub trait SimilaritySource {
    /// Similarity of each path to the query, in `0..=1`.
    ///
    /// A path that is absent from the result has no vector, which is not the
    /// same as a similarity of zero — but for ranking, no evidence and no
    /// similarity contribute the same amount, which is nothing.
    fn similarities(&self, paths: &[String]) -> std::collections::HashMap<String, f64>;
}

/// A candidate being assembled during a search.
struct Candidate {
    path: String,
    language: Option<String>,
    mtime_ms: i64,
    keyword_relevance: f64,
    symbol_score: f64,
    matched_symbols: Vec<String>,
    symbol_line: Option<u32>,
    hops: u32,
    related_to: Option<String>,
    content: Option<String>,
}

impl<'a> Retriever<'a> {
    /// Rank with embedding similarity as an extra signal.
    ///
    /// Has no effect unless `ranking.semantic` is above zero — turning
    /// embeddings on should not silently change what search returns.
    pub fn with_similarity(mut self, source: &'a dyn SimilaritySource) -> Self {
        self.similarity = Some(source);
        self
    }

    pub fn new(store: &'a dyn IndexStore, options: RetrievalOptions) -> Self {
        Retriever {
            store,
            options,
            similarity: None,
        }
    }

    /// Find the files most relevant to `query` under `root`.
    pub fn retrieve(&self, root: &str, query: &Query, now_ms: i64) -> Result<Retrieval> {
        let mut candidates: HashMap<String, Candidate> = HashMap::new();

        if !query.is_empty() {
            self.collect_text_matches(root, query, &mut candidates)?;
            self.collect_symbol_matches(root, query, &mut candidates)?;
        }

        let graph = DependencyGraph::from_edges(self.store.edges(root)?);
        self.expand_through_graph(root, &graph, &mut candidates)?;

        let considered = candidates.len();
        let files = self.rank(candidates, &graph, query, now_ms);

        Ok(Retrieval {
            query: query.text.clone(),
            root: root.to_string(),
            considered,
            files,
        })
    }

    /// Files whose text matches.
    fn collect_text_matches(
        &self,
        root: &str,
        query: &Query,
        candidates: &mut HashMap<String, Candidate>,
    ) -> Result<()> {
        for hit in self
            .store
            .search_text(root, &query.to_fts5(), self.options.candidates)?
        {
            candidates.insert(
                hit.path.clone(),
                Candidate {
                    path: hit.path,
                    language: hit.language,
                    mtime_ms: hit.mtime_ms,
                    keyword_relevance: hit.relevance,
                    symbol_score: 0.0,
                    matched_symbols: Vec::new(),
                    symbol_line: None,
                    hops: 0,
                    related_to: None,
                    content: Some(hit.content),
                },
            );
        }
        Ok(())
    }

    /// Files defining a symbol whose name matches.
    fn collect_symbol_matches(
        &self,
        root: &str,
        query: &Query,
        candidates: &mut HashMap<String, Candidate>,
    ) -> Result<()> {
        for term in &query.terms {
            for hit in self
                .store
                .search_symbols(root, term, self.options.candidates)?
            {
                let score = query.symbol_score(&hit.name);
                if score <= 0.0 {
                    continue;
                }

                let candidate = match candidates.get_mut(&hit.path) {
                    Some(existing) => existing,
                    None => {
                        let file = self.store.file(root, &hit.path)?;
                        candidates.entry(hit.path.clone()).or_insert(Candidate {
                            path: hit.path.clone(),
                            language: hit.language.clone(),
                            mtime_ms: file.map(|file| file.fingerprint.mtime_ms).unwrap_or(0),
                            keyword_relevance: 0.0,
                            symbol_score: 0.0,
                            matched_symbols: Vec::new(),
                            symbol_line: None,
                            hops: 0,
                            related_to: None,
                            content: None,
                        })
                    }
                };

                if score > candidate.symbol_score {
                    candidate.symbol_score = score;
                    candidate.symbol_line = Some(hit.start_line);
                }
                if !candidate.matched_symbols.contains(&hit.name) {
                    candidate.matched_symbols.push(hit.name);
                }
            }
        }
        Ok(())
    }

    /// Pull in what the direct matches depend on, and what depends on them.
    fn expand_through_graph(
        &self,
        root: &str,
        graph: &DependencyGraph,
        candidates: &mut HashMap<String, Candidate>,
    ) -> Result<()> {
        if self.options.expansion_depth == 0 {
            return Ok(());
        }

        let mut frontier: Vec<String> = candidates.keys().cloned().collect();

        for hop in 1..=self.options.expansion_depth {
            let mut next = Vec::new();

            for source in &frontier {
                let neighbours = graph
                    .dependencies_of(source)
                    .into_iter()
                    .chain(graph.dependents_of(source))
                    .map(String::from)
                    .collect::<Vec<_>>();

                for neighbour in neighbours {
                    if candidates.contains_key(&neighbour) {
                        continue;
                    }
                    let Some(file) = self.store.file(root, &neighbour)? else {
                        continue;
                    };

                    candidates.insert(
                        neighbour.clone(),
                        Candidate {
                            path: neighbour.clone(),
                            language: file.language,
                            mtime_ms: file.fingerprint.mtime_ms,
                            keyword_relevance: 0.0,
                            symbol_score: 0.0,
                            matched_symbols: Vec::new(),
                            symbol_line: None,
                            hops: hop as u32,
                            related_to: Some(source.clone()),
                            content: None,
                        },
                    );
                    next.push(neighbour);
                }
            }

            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        Ok(())
    }

    /// Normalize the signals, score everything, and keep the best.
    fn rank(
        &self,
        candidates: HashMap<String, Candidate>,
        graph: &DependencyGraph,
        query: &Query,
        now_ms: i64,
    ) -> Vec<RetrievedFile> {
        let best_relevance = candidates
            .values()
            .map(|candidate| candidate.keyword_relevance)
            .fold(0.0_f64, f64::max);
        let most_dependents = candidates
            .keys()
            .map(|path| graph.dependents_of(path).len())
            .max()
            .unwrap_or(0);

        // Looked up once for the whole candidate set rather than per file: it
        // is one query, and the source may have to read every vector to answer.
        let similarities = match self.similarity {
            Some(source) if self.options.weights.semantic > 0.0 => {
                let paths: Vec<String> = candidates.keys().cloned().collect();
                source.similarities(&paths)
            }
            _ => std::collections::HashMap::new(),
        };

        let mut files: Vec<RetrievedFile> = candidates
            .into_values()
            .map(|candidate| {
                let signals = Signals {
                    keyword: rank::normalize_keyword(candidate.keyword_relevance, best_relevance),
                    semantic: similarities
                        .get(&candidate.path)
                        .copied()
                        .unwrap_or(0.0)
                        .clamp(0.0, 1.0),
                    symbol: candidate.symbol_score,
                    graph: rank::normalize_graph(
                        graph.dependents_of(&candidate.path).len(),
                        most_dependents,
                    ),
                    recency: rank::normalize_recency(
                        candidate.mtime_ms,
                        now_ms,
                        self.options.recency_half_life_days,
                    ),
                    hops: candidate.hops,
                };

                let line = candidate.symbol_line.or_else(|| {
                    candidate
                        .content
                        .as_deref()
                        .and_then(|content| query.first_match_line(content))
                });
                let snippet = candidate
                    .content
                    .as_deref()
                    .zip(line)
                    .map(|(content, line)| self.snippet(content, line));

                RetrievedFile {
                    score: rank::score(&signals, &self.options.weights),
                    reason: reason_for(&candidate),
                    path: candidate.path,
                    language: candidate.language,
                    signals,
                    matched_symbols: candidate.matched_symbols,
                    line,
                    snippet,
                }
            })
            .collect();

        // Score descending, then path, so equal scores come back in a stable
        // order rather than whatever the hash map felt like.
        files.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.path.cmp(&right.path))
        });
        files.retain(|file| file.score > 0.0);
        files.truncate(self.options.limit);
        files
    }

    /// A few lines of `content` centred on `line`.
    fn snippet(&self, content: &str, line: u32) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let centre = (line as usize).saturating_sub(1);
        let start = centre.saturating_sub(self.options.snippet_lines / 2);
        let end = (centre + self.options.snippet_lines.max(1) - self.options.snippet_lines / 2)
            .min(lines.len());

        lines[start.min(lines.len())..end]
            .iter()
            .map(|line| line.trim_end())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn reason_for(candidate: &Candidate) -> Reason {
    if let Some(to) = &candidate.related_to {
        return Reason::Related {
            to: to.clone(),
            hops: candidate.hops,
        };
    }
    match (
        candidate.keyword_relevance > 0.0,
        candidate.symbol_score > 0.0,
    ) {
        (true, true) => Reason::ContentAndSymbol,
        (false, true) => Reason::Symbol,
        _ => Reason::Content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_core::{HeuristicTokenizer, Timestamp};
    use ctxc_graph::model::{
        FileFingerprint, FileIntelligence, Relationship, RelationshipKind, Symbol, SymbolKind,
    };
    use ctxc_store::{Database, SqliteIndexStore};

    const ROOT: &str = "/repo";
    const NOW: i64 = 1_700_000_000_000;

    /// A small project: auth imports database, session is unrelated.
    fn indexed_project(database: &Database) {
        let store = SqliteIndexStore::new(database);

        let files = [
            (
                "src/auth.ts",
                "import { Database } from './database';\nexport function authenticate(token) {\n  // session timeout handling\n  return db.query(token);\n}\n",
                vec![("authenticate", SymbolKind::Function, 2u32)],
                vec![("./database", Some("src/database.ts"))],
            ),
            (
                "src/database.ts",
                "export class Database {\n  query(sql) {}\n}\n",
                vec![("Database", SymbolKind::Class, 1)],
                vec![],
            ),
            (
                "src/render.ts",
                "export function render(node) {\n  return node;\n}\n",
                vec![("render", SymbolKind::Function, 1)],
                vec![],
            ),
        ];

        for (path, content, symbols, imports) in files {
            let id = store
                .upsert_file(
                    ROOT,
                    path,
                    Some("typescript"),
                    &FileFingerprint {
                        size: content.len() as u64,
                        mtime_ms: NOW,
                        content_hash: format!("hash-{path}"),
                    },
                    Timestamp::from_millis(NOW),
                )
                .unwrap();

            store
                .replace_intelligence(
                    id,
                    &FileIntelligence {
                        symbols: symbols
                            .into_iter()
                            .map(|(name, kind, line)| Symbol {
                                name: name.into(),
                                kind,
                                start_line: line,
                                end_line: line + 2,
                            })
                            .collect(),
                        relationships: imports
                            .into_iter()
                            .map(|(target, resolved)| Relationship {
                                kind: RelationshipKind::Imports,
                                from_symbol: None,
                                target: target.into(),
                                target_path: resolved.map(String::from),
                                line: 1,
                            })
                            .collect(),
                    },
                )
                .unwrap();
            store.index_content(id, path, content).unwrap();
        }
    }

    fn retrieve(options: RetrievalOptions, query: &str) -> Retrieval {
        let database = Database::open_in_memory().unwrap();
        indexed_project(&database);
        let store = SqliteIndexStore::new(&database);

        Retriever::new(&store, options)
            .retrieve(ROOT, &Query::parse(query), NOW)
            .unwrap()
    }

    fn paths(retrieval: &Retrieval) -> Vec<&str> {
        retrieval
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect()
    }

    #[test]
    fn a_symbol_match_leads_the_results() {
        let retrieval = retrieve(RetrievalOptions::default(), "authenticate");

        assert_eq!(retrieval.files[0].path, "src/auth.ts");
        assert_eq!(
            retrieval.files[0].matched_symbols,
            vec!["authenticate".to_string()]
        );
        assert!(matches!(
            retrieval.files[0].reason,
            Reason::ContentAndSymbol | Reason::Symbol
        ));
    }

    #[test]
    fn content_matches_are_found_without_a_symbol() {
        let retrieval = retrieve(RetrievalOptions::default(), "timeout");

        assert_eq!(retrieval.files[0].path, "src/auth.ts");
        assert_eq!(retrieval.files[0].reason, Reason::Content);
        assert_eq!(retrieval.files[0].line, Some(3));
        assert!(retrieval.files[0]
            .snippet
            .as_deref()
            .unwrap()
            .contains("timeout"));
    }

    #[test]
    fn the_graph_pulls_in_what_a_match_depends_on() {
        let retrieval = retrieve(RetrievalOptions::default(), "authenticate");

        let database = retrieval
            .files
            .iter()
            .find(|file| file.path == "src/database.ts")
            .expect("the file auth.ts imports must be retrieved");

        assert_eq!(
            database.reason,
            Reason::Related {
                to: "src/auth.ts".into(),
                hops: 1
            }
        );
        assert!(
            database.score < retrieval.files[0].score,
            "and rank below it"
        );
    }

    #[test]
    fn expansion_can_be_switched_off() {
        let options = RetrievalOptions {
            expansion_depth: 0,
            ..RetrievalOptions::default()
        };
        let retrieval = retrieve(options, "authenticate");

        assert!(!paths(&retrieval).contains(&"src/database.ts"));
    }

    #[test]
    fn unrelated_files_stay_out() {
        let retrieval = retrieve(RetrievalOptions::default(), "authenticate");
        assert!(
            !paths(&retrieval).contains(&"src/render.ts"),
            "render.ts neither matches nor is connected to one that does"
        );
    }

    #[test]
    fn an_empty_query_returns_nothing() {
        let retrieval = retrieve(RetrievalOptions::default(), "   ");
        assert!(retrieval.files.is_empty());
    }

    #[test]
    fn a_query_matching_nothing_returns_nothing() {
        let retrieval = retrieve(RetrievalOptions::default(), "kubernetes");
        assert!(retrieval.files.is_empty());
    }

    #[test]
    fn results_are_capped_by_the_limit() {
        let options = RetrievalOptions {
            limit: 1,
            ..RetrievalOptions::default()
        };
        let retrieval = retrieve(options, "authenticate");

        assert_eq!(retrieval.files.len(), 1);
        assert!(retrieval.considered > 1, "more were weighed than returned");
    }

    #[test]
    fn ranking_is_deterministic() {
        let first = retrieve(RetrievalOptions::default(), "authenticate");
        let second = retrieve(RetrievalOptions::default(), "authenticate");
        assert_eq!(paths(&first), paths(&second));
    }

    #[test]
    fn weights_change_the_order() {
        let symbol_first = retrieve(RetrievalOptions::default(), "database query");
        let graph_heavy = retrieve(
            RetrievalOptions {
                weights: RankingWeights {
                    symbol: 0.0,
                    keyword: 0.0,
                    graph: 5.0,
                    ..RankingWeights::default()
                },
                ..RetrievalOptions::default()
            },
            "database query",
        );

        assert!(!symbol_first.files.is_empty());
        assert_eq!(
            graph_heavy.files[0].path, "src/database.ts",
            "with only the graph weighted, the most depended on file leads"
        );
    }

    #[test]
    fn selection_stops_at_the_budget() {
        let retrieval = retrieve(RetrievalOptions::default(), "authenticate");
        let tokenizer = HeuristicTokenizer::new();

        let all = retrieval.select_within_budget(TokenBudget::new(10_000), &tokenizer, |path| {
            Some(format!("content of {path}"))
        });
        assert_eq!(all.len(), retrieval.files.len());

        let tight = retrieval.select_within_budget(TokenBudget::new(1), &tokenizer, |path| {
            Some(format!("a fairly long piece of content for {path}"))
        });
        assert_eq!(
            tight.len(),
            1,
            "a tiny budget still takes the best file and lets the optimizer trim it"
        );
        assert_eq!(tight[0], retrieval.files[0].path);
    }

    #[test]
    fn selection_skips_files_it_cannot_read() {
        let retrieval = retrieve(RetrievalOptions::default(), "authenticate");
        let tokenizer = HeuristicTokenizer::new();

        let selected =
            retrieval.select_within_budget(TokenBudget::new(10_000), &tokenizer, |_| None);
        assert!(selected.is_empty());
    }
}
