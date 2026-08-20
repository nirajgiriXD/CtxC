//! The log and terminal-output optimizer.
//!
//! Logs are the clearest case of context that costs a fortune and says very
//! little: the same message a thousand times, wrapped in colour codes and
//! progress redraws. Three stages handle that:
//!
//! * *filtering* strips ANSI escape sequences and progress-bar redraws, which
//!   carry no information once the output is text in a model's context;
//! * *deduplication* collapses repeats, matching lines by what they *say*
//!   rather than byte for byte, so the same message with a different timestamp
//!   still counts as a repeat — and the kept line records how many times it
//!   happened, because "400 times" is itself information;
//! * *selection* applies the budget.
//!
//! Normalizing numbers away when matching means two genuinely different
//! messages that differ only in a number collapse into one. That is a real
//! trade, taken deliberately: the count is preserved, and the original stays
//! retrievable through the context reference.

use std::sync::Arc;

use ctxc_context::fragment::SplitStrategy;
use ctxc_core::optimization::{OptimizedContext, Stage};
use ctxc_core::{ContentType, Context, TokenBudget, Tokenizer};

use crate::error::Result;
use crate::stages::{deduplicate, trim_and_drop_blanks, StageRunner};
use crate::ContextOptimizer;

/// Optimizer for log files and captured terminal output.
pub struct LogOptimizer {
    tokenizer: Arc<dyn Tokenizer>,
}

impl LogOptimizer {
    pub fn new(tokenizer: Arc<dyn Tokenizer>) -> Self {
        LogOptimizer { tokenizer }
    }
}

impl ContextOptimizer for LogOptimizer {
    fn name(&self) -> &'static str {
        "log"
    }

    fn supports(&self, context: &Context) -> bool {
        matches!(
            context.metadata.content_type,
            ContentType::Log | ContentType::Terminal
        )
    }

    fn optimize(&self, context: &Context, budget: Option<TokenBudget>) -> Result<OptimizedContext> {
        let mut runner = StageRunner::new(&*self.tokenizer, context, SplitStrategy::Lines);

        runner.apply(Stage::Filtering, |fragments| {
            for item in fragments.iter_mut() {
                item.content = strip_ansi(&item.content);
            }
            fragments.retain(|item| !is_noise(&item.content));
            trim_and_drop_blanks(fragments);
        });

        runner.apply(Stage::Deduplication, |fragments| {
            deduplicate(fragments, message_key, Some(annotate_repeats))
        });

        runner.select(budget);
        Ok(runner.finish(self.name(), context))
    }
}

/// Marker appended to a line that stood in for several identical ones.
fn annotate_repeats(content: &str, repeats: usize) -> String {
    format!("{content}  (repeated {repeats} times)")
}

/// Remove ANSI escape sequences.
///
/// Colour and cursor movement mean nothing to a model reading text, and they
/// are expensive: a coloured build log can spend a fifth of its tokens on
/// escape codes.
pub fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            // CSI sequences end at a byte in the 0x40..=0x7e range.
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&next) {
                        break;
                    }
                }
            }
            // OSC sequences run until BEL or a string terminator.
            Some(']') => {
                chars.next();
                while let Some(next) = chars.next() {
                    if next == '\u{7}' {
                        break;
                    }
                    if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // A lone escape or a two-character sequence.
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

/// Characters a progress bar is drawn from.
const BAR_CHARS: &str = "#=\u{2500}\u{2501}\u{2588}\u{2591}\u{2592}\u{2593}\u{25a0}\u{2022}";

/// Punctuation and units that surround a bar without being part of it.
const BAR_TRIM: &str = "->[]()<> \t.,:%|/\\";

/// Lines that carry nothing once the output is being read as text.
///
/// Only progress bars qualify: a run of bar characters, with nothing beside it
/// but brackets, digits and a percentage. A line with a word in it is never
/// noise, however decorative it looks.
fn is_noise(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return true;
    }

    let drawable = trimmed
        .chars()
        .all(|ch| BAR_CHARS.contains(ch) || BAR_TRIM.contains(ch) || ch.is_ascii_digit());
    if !drawable {
        return false;
    }

    // Require a real run of bar characters, so that a line of numbers or
    // punctuation is not mistaken for a progress bar.
    let mut run = 0usize;
    let mut longest = 0usize;
    for ch in trimmed.chars() {
        if BAR_CHARS.contains(ch) {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest >= 3
}

/// Identity used when matching repeated log lines.
///
/// Strips a leading timestamp, then replaces digit runs and long hex tokens
/// with a placeholder, so that lines differing only in when they happened, how
/// long they took, or which id they mention are recognised as the same message.
pub fn message_key(line: &str) -> String {
    let without_timestamp = strip_leading_timestamp(line.trim());
    let mut out = String::with_capacity(without_timestamp.len());

    for token in without_timestamp.split_inclusive(char::is_whitespace) {
        let trimmed = token.trim_end();
        let whitespace = &token[trimmed.len()..];

        if is_volatile_token(trimmed) {
            out.push('#');
        } else {
            out.push_str(&normalize_digits(trimmed));
        }
        if !whitespace.is_empty() {
            out.push(' ');
        }
    }
    out.trim_end().to_string()
}

/// Drop a leading ISO date and/or clock time, and any bracketed prefix.
fn strip_leading_timestamp(line: &str) -> &str {
    let mut rest = line;

    if let Some(stripped) = rest.strip_prefix('[') {
        if let Some(end) = stripped.find(']') {
            rest = stripped[end + 1..].trim_start();
        }
    }

    let mut fields = 0;
    while fields < 2 {
        let candidate = rest.split_whitespace().next().unwrap_or("");
        if candidate.is_empty() || !looks_like_time_field(candidate) {
            break;
        }
        rest = rest[candidate.len()..].trim_start();
        fields += 1;
    }
    rest
}

/// A date (`2026-08-19`) or a clock (`09:30:00.123Z`).
fn looks_like_time_field(field: &str) -> bool {
    let digits = field.chars().filter(char::is_ascii_digit).count();
    let separators = field
        .chars()
        .filter(|ch| matches!(ch, '-' | ':' | '.' | '/' | '+' | 'T' | 'Z'))
        .count();
    digits >= 4 && separators >= 2 && digits + separators == field.chars().count()
}

/// Tokens that change every run: hashes, ids, paths with digits in them.
fn is_volatile_token(token: &str) -> bool {
    let hex = token.len() >= 8
        && token
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() || ch == '-' || ch == '_');
    hex && token.chars().any(|ch| ch.is_ascii_digit())
}

/// Replace runs of digits with a placeholder.
fn normalize_digits(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    let mut in_digits = false;
    for ch in token.chars() {
        if ch.is_ascii_digit() {
            if !in_digits {
                out.push('#');
                in_digits = true;
            }
        } else {
            in_digits = false;
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_core::{ContextSource, HeuristicTokenizer};

    fn optimizer() -> LogOptimizer {
        LogOptimizer::new(Arc::new(HeuristicTokenizer::new()))
    }

    fn context(content: &str) -> Context {
        Context::new(ContextSource::Stdin, ContentType::Log, content)
    }

    #[test]
    fn it_handles_logs_and_terminal_output_only() {
        let optimizer = optimizer();
        assert!(optimizer.supports(&context("x")));
        assert!(optimizer.supports(&Context::new(
            ContextSource::Stdin,
            ContentType::Terminal,
            "x"
        )));
        assert!(!optimizer.supports(&Context::new(ContextSource::Stdin, ContentType::Code, "x")));
    }

    #[test]
    fn ansi_sequences_are_removed() {
        assert_eq!(strip_ansi("\u{1b}[32mok\u{1b}[0m"), "ok");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}text"), "text");
        assert_eq!(strip_ansi("no escapes here"), "no escapes here");
        assert_eq!(
            strip_ansi("\u{1b}[1;31merror\u{1b}[0m: broke"),
            "error: broke"
        );
    }

    #[test]
    fn repeated_messages_collapse_and_keep_their_count() {
        let log = "2026-08-19 09:30:01 WARN deprecated api\n\
                   2026-08-19 09:30:02 WARN deprecated api\n\
                   2026-08-19 09:30:03 WARN deprecated api\n\
                   2026-08-19 09:41:12 ERROR build failed\n";
        let optimized = optimizer().optimize(&context(log), None).unwrap();

        assert_eq!(
            optimized.content,
            "2026-08-19 09:30:01 WARN deprecated api  (repeated 3 times)\n\
             2026-08-19 09:41:12 ERROR build failed\n"
        );
        assert!(optimized.result.savings_by_stage.deduplication > 0);
    }

    #[test]
    fn different_messages_are_never_merged() {
        let log = "INFO started\nINFO connected\nERROR disconnected\n";
        let optimized = optimizer().optimize(&context(log), None).unwrap();

        assert_eq!(optimized.content, log);
        assert_eq!(optimized.result.savings_by_stage.deduplication, 0);
    }

    #[test]
    fn the_last_line_survives_however_much_repeats() {
        let mut log: String = (0..500)
            .map(|index| format!("2026-08-19 09:30:{index:02} INFO compiling module\n"))
            .collect();
        log.push_str("2026-08-19 09:41:12 ERROR missing symbol xyz\n");

        let optimized = optimizer().optimize(&context(&log), None).unwrap();
        assert!(optimized.content.contains("ERROR missing symbol xyz"));
        assert_eq!(optimized.content.lines().count(), 2);
        assert!(optimized.result.reduction_ratio > 0.9);
    }

    #[test]
    fn progress_redraws_are_dropped() {
        let log =
            "building\n[==============>          ] 45%\n[========================] 100%\ndone\n";
        let optimized = optimizer().optimize(&context(log), None).unwrap();

        assert_eq!(optimized.content, "building\ndone\n");
        assert!(optimized.result.savings_by_stage.filtering > 0);
    }

    #[test]
    fn timestamps_are_ignored_when_matching() {
        assert_eq!(
            message_key("2026-08-19 09:30:01 WARN slow request"),
            message_key("2026-08-20 11:02:59 WARN slow request")
        );
        assert_eq!(
            message_key("[2026-08-19T09:30:01Z] request finished"),
            message_key("[2026-08-19T09:31:44Z] request finished")
        );
    }

    #[test]
    fn numbers_are_normalized_but_words_are_not() {
        assert_eq!(
            message_key("processed 100 items"),
            message_key("processed 250 items")
        );
        assert_ne!(
            message_key("processed 100 items"),
            message_key("processed 100 files")
        );
    }

    #[test]
    fn identifiers_are_normalized() {
        assert_eq!(
            message_key("request a1b2c3d4e5f6 failed"),
            message_key("request 9f8e7d6c5b4a failed")
        );
    }

    /// Distinct messages that stay distinct under number normalization.
    fn distinct_messages(count: usize) -> String {
        (0..count)
            .map(|index| {
                let first = (b'a' + (index / 26) as u8) as char;
                let second = (b'a' + (index % 26) as u8) as char;
                format!("INFO subsystem {first}{second} reported something\n")
            })
            .collect()
    }

    #[test]
    fn lines_that_only_look_like_bars_are_kept() {
        let log = "12:00:00 INFO 45% complete\n---\ntotals: 1 2 3\n";
        let optimized = optimizer().optimize(&context(log), None).unwrap();

        assert!(optimized.content.contains("45% complete"));
        assert!(optimized.content.contains("totals: 1 2 3"));
    }

    #[test]
    fn a_budget_still_applies() {
        let log = distinct_messages(200);
        let optimized = optimizer()
            .optimize(&context(&log), Some(TokenBudget::new(50)))
            .unwrap();

        assert!(optimized.result.optimized_tokens <= 50);
        assert!(optimized.result.savings_by_stage.selection > 0);
    }

    #[test]
    fn stage_savings_sum_to_the_total() {
        let log = "\u{1b}[32mINFO\u{1b}[0m started   \n\nINFO started\n[====] 50%\nERROR failed\n";
        let optimized = optimizer()
            .optimize(&context(log), Some(TokenBudget::new(8)))
            .unwrap();

        assert_eq!(
            optimized.result.savings_by_stage.total(),
            optimized.result.tokens_saved()
        );
    }
}
