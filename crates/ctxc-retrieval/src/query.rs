//! Turning what someone typed into a query the index can run.
//!
//! FTS5 has its own syntax, and feeding raw user text to it is both a
//! correctness problem (a stray quote is a syntax error) and a safety one. So
//! the query is taken apart into plain terms and reassembled, quoted, by this
//! module. Nothing a user types is ever interpreted as an operator.

/// A search request, split into the terms it is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// The text exactly as it was given, for display.
    pub text: String,
    /// Individual search terms, lowercased.
    pub terms: Vec<String>,
}

impl Query {
    /// Parse user input into terms.
    ///
    /// Quoted sections stay together, so `"connection timeout"` is one term
    /// rather than two.
    pub fn parse(input: &str) -> Query {
        let mut terms = Vec::new();
        let mut current = String::new();
        let mut quoted = false;

        for ch in input.chars() {
            match ch {
                // A quote only groups words when it opens a term. One in the
                // middle of a word is part of that word, and gets escaped on
                // the way into the query rather than silently vanishing.
                '"' if quoted => {
                    quoted = false;
                    push_term(&mut terms, &mut current);
                }
                '"' if current.is_empty() => quoted = true,
                ch if ch.is_whitespace() && !quoted => push_term(&mut terms, &mut current),
                ch => current.push(ch),
            }
        }
        push_term(&mut terms, &mut current);

        Query {
            text: input.trim().to_string(),
            terms: without_stopwords(terms),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// The FTS5 expression for this query.
    ///
    /// Terms are ORed rather than ANDed: BM25 already ranks a document matching
    /// every term above one matching a single term, and OR keeps a query with
    /// one unusual word in it from returning nothing at all.
    pub fn to_fts5(&self) -> String {
        self.terms
            .iter()
            .map(|term| format!("\"{}\"", escape(term)))
            .collect::<Vec<_>>()
            .join(" OR ")
    }

    /// How strongly a symbol name matches this query, from 0.0 to 1.0.
    pub fn symbol_score(&self, name: &str) -> f64 {
        let lowered = name.to_lowercase();
        let mut best: f64 = 0.0;

        for term in &self.terms {
            let score = if lowered == *term {
                1.0
            } else if lowered.starts_with(term) {
                0.7
            } else if lowered.contains(term) {
                0.4
            } else {
                0.0
            };
            best = best.max(score);
        }
        best
    }

    /// The first line of `content` containing any term, 1-based.
    pub fn first_match_line(&self, content: &str) -> Option<u32> {
        content.lines().enumerate().find_map(|(index, line)| {
            let lowered = line.to_lowercase();
            self.terms
                .iter()
                .any(|term| lowered.contains(term))
                .then_some(index as u32 + 1)
        })
    }
}

/// Words too common to narrow anything down.
///
/// A question like "how are ignore rules applied?" is mostly grammar; leaving
/// it in dilutes the ranking with files that merely contain "are". They are
/// dropped only when something else survives, so a search for exactly "how to"
/// still searches for those words.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "do", "does", "for", "from",
    "how", "i", "if", "in", "is", "it", "its", "of", "on", "or", "the", "this", "that", "to",
    "was", "we", "what", "when", "where", "which", "why", "will", "with", "you",
];

/// Drop stopwords, unless that would leave nothing to search for.
fn without_stopwords(terms: Vec<String>) -> Vec<String> {
    let kept: Vec<String> = terms
        .iter()
        .filter(|term| !STOPWORDS.contains(&term.as_str()))
        .cloned()
        .collect();

    if kept.is_empty() {
        terms
    } else {
        kept
    }
}

fn push_term(terms: &mut Vec<String>, current: &mut String) {
    let term = current.trim().to_lowercase();
    if !term.is_empty() {
        terms.push(term);
    }
    current.clear();
}

/// Escape a term for use inside an FTS5 double-quoted string.
fn escape(term: &str) -> String {
    term.replace('"', "\"\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_become_terms() {
        let query = Query::parse("authentication timeout");
        assert_eq!(query.terms, vec!["authentication", "timeout"]);
        assert_eq!(query.to_fts5(), "\"authentication\" OR \"timeout\"");
    }

    #[test]
    fn quoted_sections_stay_together() {
        let query = Query::parse("\"connection refused\" retry");
        assert_eq!(query.terms, vec!["connection refused", "retry"]);
        assert_eq!(query.to_fts5(), "\"connection refused\" OR \"retry\"");
    }

    #[test]
    fn grammar_words_are_dropped_from_questions() {
        let query = Query::parse("how are ignore rules applied when walking");
        assert_eq!(query.terms, vec!["ignore", "rules", "applied", "walking"]);
    }

    #[test]
    fn a_query_made_only_of_common_words_still_searches_for_them() {
        let query = Query::parse("how to");
        assert_eq!(query.terms, vec!["how", "to"]);
    }

    #[test]
    fn terms_are_lowercased_and_trimmed() {
        let query = Query::parse("  Authentication   TIMEOUT ");
        assert_eq!(query.terms, vec!["authentication", "timeout"]);
        assert_eq!(query.text, "Authentication   TIMEOUT");
    }

    #[test]
    fn fts5_operators_typed_by_a_user_are_just_text() {
        // Without quoting, these would be parsed as FTS5 syntax or fail.
        for input in ["OR AND NOT", "auth*", "col:value", "(unbalanced"] {
            let query = Query::parse(input);
            let expression = query.to_fts5();
            assert!(expression.starts_with('"'), "{input} produced {expression}");
        }
    }

    #[test]
    fn embedded_quotes_are_escaped() {
        let query = Query::parse("say\"hello");
        assert_eq!(query.to_fts5(), "\"say\"\"hello\"");
    }

    #[test]
    fn an_empty_query_has_no_terms() {
        assert!(Query::parse("   ").is_empty());
        assert!(Query::parse("").is_empty());
    }

    #[test]
    fn symbol_scores_prefer_exact_names() {
        let query = Query::parse("authenticate");

        assert_eq!(query.symbol_score("authenticate"), 1.0);
        assert_eq!(query.symbol_score("Authenticate"), 1.0, "case insensitive");
        assert_eq!(query.symbol_score("authenticate_user"), 0.7);
        assert_eq!(query.symbol_score("reauthenticate"), 0.4);
        assert_eq!(query.symbol_score("render"), 0.0);
    }

    #[test]
    fn symbol_scores_take_the_best_term() {
        let query = Query::parse("render authenticate");
        assert_eq!(query.symbol_score("authenticate"), 1.0);
    }

    #[test]
    fn match_lines_are_one_based() {
        let query = Query::parse("timeout");
        let content = "fn main() {}\n\n// handle timeout here\n";

        assert_eq!(query.first_match_line(content), Some(3));
        assert_eq!(Query::parse("absent").first_match_line(content), None);
    }
}
