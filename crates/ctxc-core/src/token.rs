//! Token budgets and token counting.
//!
//! What lives here is the [`Tokenizer`] trait every layer talks to, plus the
//! deterministic [`HeuristicTokenizer`] that makes CtxC usable with no model
//! and no network. That estimator is the default, and always will be: it is
//! what lets `cargo install ctxc` produce something that works offline.
//!
//! Exact tokenization is model specific, and a vocabulary is megabytes. So the
//! `cl100k` encoder — what GPT-4 and its relatives count with — is behind a
//! Cargo feature of the same name. A build without it does not offer the name,
//! and says so rather than quietly estimating instead: a budget that claims to
//! be exact and is not is worse than one that admits it is a guess.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// A ceiling on how many tokens a piece of optimized context may occupy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TokenBudget {
    total: u32,
}

impl TokenBudget {
    /// Create a budget of `total` tokens.
    pub const fn new(total: u32) -> Self {
        TokenBudget { total }
    }

    /// The full size of the budget.
    pub const fn total(self) -> u32 {
        self.total
    }

    /// Whether `tokens` fits within the budget.
    pub const fn fits(self, tokens: u32) -> bool {
        tokens <= self.total
    }

    /// Tokens left after spending `tokens`; saturates at zero rather than
    /// wrapping, because an overspent budget is reported, not panicked on.
    pub const fn remaining_after(self, tokens: u32) -> u32 {
        self.total.saturating_sub(tokens)
    }

    /// A smaller budget carved out of this one, clamped to what is available.
    pub const fn split(self, tokens: u32) -> TokenBudget {
        TokenBudget::new(if tokens < self.total {
            tokens
        } else {
            self.total
        })
    }
}

/// Counts the tokens a piece of text will occupy in a model's context window.
///
/// Implementations are expected to be cheap and side-effect free: the engine
/// counts the same text several times while attributing savings to stages.
pub trait Tokenizer: Send + Sync {
    /// Name reported in output, so a number can be traced to how it was made.
    fn name(&self) -> &'static str;

    /// Tokens `text` is expected to occupy.
    fn count(&self, text: &str) -> u32;

    /// Whether the count is approximate. Only an exact target tokenizer may
    /// return false, and CtxC labels estimates wherever they are shown.
    fn is_estimate(&self) -> bool {
        true
    }
}

/// Model-independent token estimator.
///
/// It approximates how byte-pair encoders behave without shipping any
/// vocabulary: short words cost one token, long words split roughly every four
/// characters, digits split about every three, punctuation runs merge in pairs,
/// and ideographic characters cost one each. Expect it to land within roughly
/// a quarter of a real tokenizer on prose and code — good enough to budget
/// with, never precise enough to bill with.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeuristicTokenizer;

impl HeuristicTokenizer {
    pub const fn new() -> Self {
        HeuristicTokenizer
    }

    /// Tokens for a run of `length` word characters.
    fn word_tokens(length: usize) -> u32 {
        match length {
            0 => 0,
            1..=5 => 1,
            6..=9 => 2,
            _ => length.div_ceil(4) as u32,
        }
    }
}

impl Tokenizer for HeuristicTokenizer {
    fn name(&self) -> &'static str {
        "heuristic"
    }

    fn count(&self, text: &str) -> u32 {
        let mut tokens: u32 = 0;
        let mut run = Run::None;
        let mut length = 0usize;

        for ch in text.chars() {
            let kind = Run::classify(ch);
            if kind == run {
                length += 1;
                continue;
            }
            tokens = tokens.saturating_add(run.tokens(length));
            run = kind;
            length = 1;
        }
        tokens.saturating_add(run.tokens(length))
    }
}

/// The tokenizer a configured name selects, or `None` when this build has no
/// such tokenizer.
///
/// Names are matched exactly. Guessing at a near miss would silently budget
/// against something other than what was asked for.
pub fn named(name: &str) -> Option<Arc<dyn Tokenizer>> {
    match name {
        "heuristic" => Some(Arc::new(HeuristicTokenizer::new())),
        #[cfg(feature = "cl100k")]
        "cl100k" => Some(Arc::new(Cl100kTokenizer::new())),
        _ => None,
    }
}

/// The tokenizer names this build understands, for an error message that can
/// say what the alternatives are.
pub fn available() -> Vec<&'static str> {
    let mut names = vec!["heuristic"];
    if cfg!(feature = "cl100k") {
        names.push("cl100k");
    }
    names
}

/// The exact byte-pair encoder GPT-4 and its relatives use.
///
/// Counts are the real thing, so [`Tokenizer::is_estimate`] finally returns
/// false and a budget can be filled to its edge instead of being left a
/// safety margin. The vocabulary is compiled into the binary — this adds
/// megabytes to the build and no network call at any point.
#[cfg(feature = "cl100k")]
pub struct Cl100kTokenizer {
    encoder: &'static tiktoken_rs::CoreBPE,
}

#[cfg(feature = "cl100k")]
impl Cl100kTokenizer {
    pub fn new() -> Self {
        // Built once: assembling the vocabulary is slow, and the engine counts
        // the same text several times while attributing savings to stages.
        static ENCODER: std::sync::OnceLock<tiktoken_rs::CoreBPE> = std::sync::OnceLock::new();
        Cl100kTokenizer {
            encoder: ENCODER.get_or_init(|| {
                tiktoken_rs::cl100k_base().expect("the cl100k vocabulary is compiled in")
            }),
        }
    }
}

#[cfg(feature = "cl100k")]
impl Default for Cl100kTokenizer {
    fn default() -> Self {
        Cl100kTokenizer::new()
    }
}

#[cfg(feature = "cl100k")]
impl Tokenizer for Cl100kTokenizer {
    fn name(&self) -> &'static str {
        "cl100k"
    }

    fn count(&self, text: &str) -> u32 {
        self.encoder
            .encode_ordinary(text)
            .len()
            .min(u32::MAX as usize) as u32
    }

    fn is_estimate(&self) -> bool {
        false
    }
}

/// Character classes the estimator prices differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Run {
    None,
    Word,
    Digit,
    /// Line breaks, which real tokenizers charge for.
    Newline,
    /// Spaces and tabs, which merge into the neighbouring token.
    Space,
    Punctuation,
    /// Chinese, Japanese and Korean characters, roughly one token each.
    Ideographic,
}

impl Run {
    fn classify(ch: char) -> Run {
        if ch == '\n' || ch == '\r' {
            Run::Newline
        } else if ch.is_whitespace() {
            Run::Space
        } else if ch.is_ascii_digit() {
            Run::Digit
        } else if is_ideographic(ch) {
            Run::Ideographic
        } else if ch.is_alphanumeric() || ch == '_' || ch == '\'' {
            Run::Word
        } else {
            Run::Punctuation
        }
    }

    fn tokens(self, length: usize) -> u32 {
        match self {
            Run::None | Run::Space => 0,
            Run::Word => HeuristicTokenizer::word_tokens(length),
            Run::Digit => length.div_ceil(3) as u32,
            // A run of blank lines collapses far more than it costs.
            Run::Newline => length.div_ceil(2).min(length) as u32,
            Run::Punctuation => length.div_ceil(2) as u32,
            Run::Ideographic => length as u32,
        }
    }
}

/// CJK ranges, which tokenize per character rather than per word.
fn is_ideographic(ch: char) -> bool {
    matches!(ch as u32,
        0x3040..=0x30FF   // hiragana, katakana
        | 0x3400..=0x4DBF // CJK extension A
        | 0x4E00..=0x9FFF // CJK unified ideographs
        | 0xAC00..=0xD7AF // hangul syllables
        | 0xF900..=0xFAFF // compatibility ideographs
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_capacity() {
        let budget = TokenBudget::new(32_000);
        assert!(budget.fits(32_000));
        assert!(!budget.fits(32_001));
        assert_eq!(budget.remaining_after(30_000), 2_000);
    }

    #[test]
    fn overspending_saturates() {
        assert_eq!(TokenBudget::new(100).remaining_after(500), 0);
    }

    #[test]
    fn split_is_clamped() {
        let budget = TokenBudget::new(1_000);
        assert_eq!(budget.split(400).total(), 400);
        assert_eq!(budget.split(4_000).total(), 1_000);
    }

    fn count(text: &str) -> u32 {
        HeuristicTokenizer::new().count(text)
    }

    #[test]
    fn empty_text_costs_nothing() {
        assert_eq!(count(""), 0);
        assert_eq!(count("   \t  "), 0, "spaces merge into their neighbours");
    }

    #[test]
    fn short_words_cost_one_token_each() {
        assert_eq!(count("hello"), 1);
        assert_eq!(count("hello world"), 2);
        assert_eq!(count("the quick brown fox"), 4);
    }

    #[test]
    fn long_words_are_split() {
        assert!(count("internationalization") >= 4);
        assert!(count("internationalization") < count("internationalization internationalization"));
    }

    #[test]
    fn punctuation_and_digits_are_charged() {
        assert!(count("hello, world!") > count("hello world"));
        assert!(count("1234567890") >= 3);
    }

    #[test]
    fn ideographic_text_is_charged_per_character() {
        assert_eq!(count("\u{4f60}\u{597d}\u{4e16}\u{754c}"), 4);
    }

    #[test]
    fn estimates_stay_in_the_right_ballpark_for_prose() {
        // Real byte-pair encoders land near 10 tokens for this sentence.
        let sentence = "The quick brown fox jumps over the lazy dog.";
        let tokens = count(sentence);
        assert!((8..=13).contains(&tokens), "estimated {tokens} tokens");
    }

    #[test]
    fn counting_is_monotonic_in_content() {
        let short = "fn main() {}";
        let long = "fn main() {\n    println!(\"hello\");\n}";
        assert!(count(long) > count(short));
    }

    #[test]
    fn the_estimator_says_it_is_estimating() {
        let tokenizer = HeuristicTokenizer::new();
        assert!(tokenizer.is_estimate());
        assert_eq!(tokenizer.name(), "heuristic");
    }

    #[test]
    fn the_estimator_is_always_available_and_is_the_only_name_by_default() {
        assert!(named("heuristic").is_some());
        assert!(named("no-such-tokenizer").is_none());
        assert!(available().contains(&"heuristic"));

        // Selecting a tokenizer this build does not have must fail rather than
        // hand back an estimate wearing the exact one's name.
        assert_eq!(named("cl100k").is_some(), cfg!(feature = "cl100k"));
        assert_eq!(available().contains(&"cl100k"), cfg!(feature = "cl100k"));
    }

    #[cfg(feature = "cl100k")]
    #[test]
    fn the_exact_tokenizer_is_exact() {
        let tokenizer = Cl100kTokenizer::new();
        assert!(!tokenizer.is_estimate(), "this one is not a guess");
        assert_eq!(tokenizer.name(), "cl100k");

        // The published cl100k count for this sentence.
        assert_eq!(
            tokenizer.count("The quick brown fox jumps over the lazy dog."),
            10
        );
        assert_eq!(tokenizer.count(""), 0);
    }
}
