//! Context fragments.
//!
//! Optimizers work on fragments rather than whole documents: it is what makes
//! "drop this, keep that" expressible, and what lets savings be attributed and
//! reversed. A fragment always knows where in the original it came from, so a
//! later phase can hand back exactly what was removed.

use std::ops::Range;

use ctxc_core::ContentType;

/// How a document was cut up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitStrategy {
    /// Paragraph-like blocks separated by blank lines. The default for prose,
    /// code and documentation, where a blank line is a real boundary.
    Blocks,
    /// One fragment per line. Used for line-oriented material such as logs and
    /// terminal output, where each line stands alone.
    Lines,
}

impl SplitStrategy {
    /// The separator that rejoins fragments of this strategy.
    pub fn separator(self) -> &'static str {
        match self {
            SplitStrategy::Blocks => "\n\n",
            SplitStrategy::Lines => "\n",
        }
    }
}

/// The strategy that suits a content type.
pub fn strategy_for(content_type: ContentType) -> SplitStrategy {
    match content_type {
        ContentType::Log | ContentType::Terminal => SplitStrategy::Lines,
        ContentType::PlainText
        | ContentType::Markdown
        | ContentType::Json
        | ContentType::Code
        | ContentType::Binary
        | ContentType::Unknown => SplitStrategy::Blocks,
    }
}

/// A slice of a context, with its position in the original content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    /// Position in the original sequence of fragments.
    pub index: usize,
    pub content: String,
    /// Byte range this fragment occupies in the original content.
    pub span: Range<usize>,
}

impl Fragment {
    /// Content with trailing whitespace removed from every line, and with no
    /// leading or trailing blank lines.
    pub fn trimmed(&self) -> String {
        let mut lines: Vec<&str> = self
            .content
            .lines()
            .map(|line| line.trim_end())
            .collect::<Vec<_>>();

        while lines.first().is_some_and(|line| line.is_empty()) {
            lines.remove(0);
        }
        while lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
        lines.join("\n")
    }

    /// Whether the fragment carries no content at all.
    pub fn is_blank(&self) -> bool {
        self.content.trim().is_empty()
    }
}

/// Split `text` into fragments.
///
/// Splitting is lossless in the sense that every fragment's span points at the
/// bytes it came from; the separators between fragments are the only thing not
/// captured, and rejoining restores an equivalent document.
pub fn split(text: &str, strategy: SplitStrategy) -> Vec<Fragment> {
    match strategy {
        SplitStrategy::Blocks => split_blocks(text),
        SplitStrategy::Lines => split_lines(text),
    }
}

/// Rejoin fragments into a document.
pub fn join(fragments: &[Fragment], strategy: SplitStrategy) -> String {
    let mut out = String::new();
    for (position, fragment) in fragments.iter().enumerate() {
        if position > 0 {
            out.push_str(strategy.separator());
        }
        out.push_str(&fragment.content);
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Split on runs of blank lines, keeping byte spans accurate.
fn split_blocks(text: &str) -> Vec<Fragment> {
    let mut fragments = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0usize;
    let mut offset = 0usize;

    for line in text.split_inclusive('\n') {
        let line_start = offset;
        offset += line.len();

        if line.trim().is_empty() {
            if let Some(begin) = start.take() {
                push_fragment(&mut fragments, text, begin..end);
            }
        } else {
            start.get_or_insert(line_start);
            end = offset;
        }
    }
    if let Some(begin) = start {
        push_fragment(&mut fragments, text, begin..end);
    }

    fragments
}

/// One fragment per non-empty line.
fn split_lines(text: &str) -> Vec<Fragment> {
    let mut fragments = Vec::new();
    let mut offset = 0usize;

    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let trimmed_len = line.trim_end_matches(['\n', '\r']).len();
        if line.trim().is_empty() {
            continue;
        }
        push_fragment(&mut fragments, text, start..start + trimmed_len);
    }

    fragments
}

fn push_fragment(fragments: &mut Vec<Fragment>, text: &str, span: Range<usize>) {
    let content = text[span.clone()]
        .trim_end_matches(['\n', '\r'])
        .to_string();
    fragments.push(Fragment {
        index: fragments.len(),
        content,
        span,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_split_on_blank_lines() {
        let text = "first block\nstill first\n\n\nsecond block\n";
        let fragments = split(text, SplitStrategy::Blocks);

        assert_eq!(fragments.len(), 2);
        assert_eq!(fragments[0].content, "first block\nstill first");
        assert_eq!(fragments[1].content, "second block");
        assert_eq!(fragments[1].index, 1);
    }

    #[test]
    fn spans_point_back_at_the_original() {
        let text = "alpha\n\nbeta\n";
        let fragments = split(text, SplitStrategy::Blocks);

        for fragment in &fragments {
            assert_eq!(
                text[fragment.span.clone()].trim_end(),
                fragment.content,
                "span must reproduce the fragment"
            );
        }
    }

    #[test]
    fn lines_split_one_per_line_and_skip_blanks() {
        let text = "one\n\ntwo\nthree\n";
        let fragments = split(text, SplitStrategy::Lines);

        assert_eq!(fragments.len(), 3);
        assert_eq!(fragments[2].content, "three");
    }

    #[test]
    fn carriage_returns_are_not_kept_in_fragments() {
        let fragments = split("one\r\ntwo\r\n", SplitStrategy::Lines);
        assert_eq!(fragments[0].content, "one");
        assert_eq!(fragments[1].content, "two");
    }

    #[test]
    fn empty_input_produces_no_fragments() {
        assert!(split("", SplitStrategy::Blocks).is_empty());
        assert!(split("\n\n  \n", SplitStrategy::Blocks).is_empty());
        assert_eq!(join(&[], SplitStrategy::Blocks), "");
    }

    #[test]
    fn splitting_then_joining_round_trips_normalized_text() {
        let text = "alpha\n\nbeta\ngamma\n";
        let fragments = split(text, SplitStrategy::Blocks);
        assert_eq!(join(&fragments, SplitStrategy::Blocks), text);
    }

    #[test]
    fn trimming_removes_trailing_whitespace_per_line() {
        let fragments = split("code   \n    indented   \n", SplitStrategy::Blocks);
        assert_eq!(fragments[0].trimmed(), "code\n    indented");
        assert!(!fragments[0].is_blank());
    }

    #[test]
    fn strategies_follow_content_type() {
        assert_eq!(strategy_for(ContentType::Log), SplitStrategy::Lines);
        assert_eq!(strategy_for(ContentType::Terminal), SplitStrategy::Lines);
        assert_eq!(strategy_for(ContentType::Code), SplitStrategy::Blocks);
        assert_eq!(strategy_for(ContentType::PlainText), SplitStrategy::Blocks);
    }
}
