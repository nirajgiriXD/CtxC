//! Editing a file CtxC does not own.
//!
//! `CLAUDE.md`, `AGENTS.md` and their siblings belong to the user. CtxC writes
//! one clearly marked block into them and touches nothing else, which is what
//! makes installing safe to repeat and uninstalling safe to trust:
//!
//! ```text
//! <!-- ctxc:begin -->   <- everything between the markers is ours
//! ...
//! <!-- ctxc:end -->     <- everything outside is theirs, always
//! ```
//!
//! Every operation here is a pure string transformation, so the rules can be
//! tested exhaustively without a filesystem — which matters, because the cost
//! of getting this wrong is somebody's notes.

use std::fmt;

/// How a file spells a comment.
///
/// The markers have to be invisible to whatever reads the file, so they are
/// written in that file's own comment syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentStyle {
    /// `<!-- ... -->`, for Markdown.
    Html,
    /// `# ...`, for YAML and plain configuration.
    Hash,
}

impl CommentStyle {
    fn open(self, text: &str) -> String {
        match self {
            CommentStyle::Html => format!("<!-- {text} -->"),
            CommentStyle::Hash => format!("# {text}"),
        }
    }
}

/// The marker text, without its comment wrapper.
const BEGIN: &str = "ctxc:begin — managed by CtxC, do not edit inside this block";
const END: &str = "ctxc:end";

/// A managed region in someone else's file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagedBlock {
    style: CommentStyle,
}

/// What changed when a block was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The file had no block; one was appended.
    Added,
    /// A block was there and its contents differed.
    Updated,
    /// A block was there and already said exactly this.
    Unchanged,
}

impl Change {
    pub fn as_str(self) -> &'static str {
        match self {
            Change::Added => "added",
            Change::Updated => "updated",
            Change::Unchanged => "unchanged",
        }
    }

    pub fn wrote_anything(self) -> bool {
        !matches!(self, Change::Unchanged)
    }
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl ManagedBlock {
    pub fn new(style: CommentStyle) -> Self {
        ManagedBlock { style }
    }

    fn begin(&self) -> String {
        self.style.open(BEGIN)
    }

    fn end(&self) -> String {
        self.style.open(END)
    }

    /// Render `body` wrapped in markers.
    pub fn render(&self, body: &str) -> String {
        format!("{}\n{}\n{}", self.begin(), body.trim_end(), self.end())
    }

    /// Where the block sits in `document`, as a byte range covering the
    /// markers and everything between them.
    ///
    /// Markers are matched on their own line so that a block quoted inside a
    /// code fence — the kind of thing that ends up in documentation about
    /// CtxC — is not mistaken for the real one.
    fn find(&self, document: &str) -> Option<(usize, usize)> {
        let begin = self.begin();
        let end = self.end();

        let start = line_position(document, &begin)?;
        let after_start = start + begin.len();
        let close = line_position(&document[after_start..], &end)? + after_start;

        Some((start, close + end.len()))
    }

    /// Whether `document` already carries a block.
    pub fn is_present(&self, document: &str) -> bool {
        self.find(document).is_some()
    }

    /// The body currently inside the block, if there is one.
    ///
    /// Line endings are normalised, so a file checked out with CRLF reports the
    /// same body as one with LF. Without that, every install on Windows would
    /// read its own last write as different and rewrite the file forever.
    pub fn body(&self, document: &str) -> Option<String> {
        let (start, end) = self.find(document)?;
        let inner = &document[start + self.begin().len()..end - self.end().len()];
        Some(inner.replace("\r\n", "\n").trim_matches('\n').to_string())
    }

    /// Put `body` in the file, adding the block or replacing what is there.
    ///
    /// Content outside the markers is preserved byte for byte, including a
    /// file that ends without a newline and one that never had a block.
    pub fn write(&self, document: &str, body: &str) -> (String, Change) {
        let rendered = self.render(body);

        if let Some((start, end)) = self.find(document) {
            if document[start..end] == rendered {
                return (document.to_string(), Change::Unchanged);
            }

            let mut updated = String::with_capacity(document.len() + rendered.len());
            updated.push_str(&document[..start]);
            updated.push_str(&rendered);
            updated.push_str(&document[end..]);
            return (updated, Change::Updated);
        }

        // A new block goes at the end, after a blank line, so it never lands
        // in the middle of a sentence someone was writing.
        let mut appended = String::with_capacity(document.len() + rendered.len() + 2);
        if !document.is_empty() {
            appended.push_str(document.trim_end_matches('\n'));
            appended.push_str("\n\n");
        }
        appended.push_str(&rendered);
        appended.push('\n');

        (appended, Change::Added)
    }

    /// Take the block out, leaving everything else alone.
    ///
    /// Returns `None` when there was nothing to remove, so a caller can tell
    /// "already gone" from "just removed it".
    pub fn remove(&self, document: &str) -> Option<String> {
        let (start, end) = self.find(document)?;

        let before = &document[..start];
        let after = &document[end..];

        // The blank line that separated the block from what came before it
        // belongs to the block, not to the user's text.
        let mut result = String::with_capacity(document.len());
        result.push_str(before.trim_end_matches('\n'));

        let rest = after.trim_start_matches('\n');
        if !rest.is_empty() {
            if !result.is_empty() {
                result.push_str("\n\n");
            }
            result.push_str(rest);
        }

        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        Some(result)
    }
}

/// Byte offset of `needle` where it occupies a whole line of its own.
fn line_position(haystack: &str, needle: &str) -> Option<usize> {
    let mut offset = 0;

    for line in haystack.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']).trim() == needle {
            return Some(offset + (line.len() - line.trim_start().len()));
        }
        offset += line.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn markdown() -> ManagedBlock {
        ManagedBlock::new(CommentStyle::Html)
    }

    #[test]
    fn a_block_is_appended_to_an_existing_file() {
        let (result, change) = markdown().write("# My notes\n\nSome guidance.\n", "Ours.");

        assert_eq!(change, Change::Added);
        assert!(result.starts_with("# My notes\n\nSome guidance.\n\n<!-- ctxc:begin"));
        assert!(result.ends_with("<!-- ctxc:end -->\n"));
    }

    #[test]
    fn a_block_is_the_whole_file_when_there_was_nothing() {
        let (result, change) = markdown().write("", "Ours.");

        assert_eq!(change, Change::Added);
        assert!(result.starts_with("<!-- ctxc:begin"));
        assert!(
            !result.starts_with('\n'),
            "no leading blank line: {result:?}"
        );
    }

    #[test]
    fn writing_the_same_thing_twice_changes_nothing() {
        let block = markdown();
        let (once, _) = block.write("# Notes\n", "Ours.");
        let (twice, change) = block.write(&once, "Ours.");

        assert_eq!(change, Change::Unchanged);
        assert_eq!(once, twice);
    }

    #[test]
    fn updating_replaces_only_what_is_between_the_markers() {
        let block = markdown();
        let (first, _) = block.write("# Notes\n\nMine.\n", "Version one.");
        let (second, change) = block.write(&first, "Version two.");

        assert_eq!(change, Change::Updated);
        assert!(second.contains("# Notes"), "{second}");
        assert!(
            second.contains("Mine."),
            "the user's text survives: {second}"
        );
        assert!(second.contains("Version two."), "{second}");
        assert!(!second.contains("Version one."), "{second}");
    }

    #[test]
    fn text_after_the_block_survives_an_update() {
        let block = markdown();
        let seeded = format!(
            "Before.\n\n{}\n\nAfter, written by hand.\n",
            block.render("Old.")
        );

        let (result, change) = block.write(&seeded, "New.");
        assert_eq!(change, Change::Updated);
        assert!(result.contains("Before."), "{result}");
        assert!(result.contains("After, written by hand."), "{result}");
        assert!(result.contains("New."), "{result}");
    }

    #[test]
    fn removing_takes_the_block_and_nothing_else() {
        let block = markdown();
        let (seeded, _) = block.write("# Notes\n\nMine.\n", "Ours.");

        let cleaned = block.remove(&seeded).unwrap();
        assert_eq!(cleaned, "# Notes\n\nMine.\n");
        assert!(!block.is_present(&cleaned));
    }

    #[test]
    fn removing_keeps_what_came_after_the_block() {
        let block = markdown();
        let seeded = format!("Before.\n\n{}\n\nAfter.\n", block.render("Ours."));

        let cleaned = block.remove(&seeded).unwrap();
        assert_eq!(cleaned, "Before.\n\nAfter.\n");
    }

    #[test]
    fn removing_a_block_that_is_the_whole_file_leaves_nothing() {
        let block = markdown();
        let (seeded, _) = block.write("", "Ours.");

        assert_eq!(block.remove(&seeded).unwrap(), "");
    }

    #[test]
    fn removing_from_a_file_with_no_block_reports_that() {
        assert_eq!(markdown().remove("# Just my notes.\n"), None);
    }

    #[test]
    fn the_body_can_be_read_back() {
        let block = markdown();
        let (seeded, _) = block.write("# Notes\n", "Line one.\nLine two.");

        assert_eq!(block.body(&seeded).as_deref(), Some("Line one.\nLine two."));
        assert_eq!(block.body("# Nothing here.\n"), None);
    }

    #[test]
    fn a_marker_quoted_inside_a_code_fence_is_not_the_real_one() {
        // Documentation about CtxC will contain exactly this.
        let document = "# About CtxC\n\n```\n    <!-- ctxc:begin — managed by CtxC, do not edit inside this block -->\n```\n";

        // Indented, so it is not a line of its own by our rule: an
        // unterminated marker must never make us rewrite someone's example.
        let block = markdown();
        assert!(
            block.body(document).is_none(),
            "a block with no end marker is not a block"
        );
    }

    #[test]
    fn an_unterminated_marker_is_left_alone() {
        let block = markdown();
        let document = format!("Notes.\n\n{}\n\nAnd more.\n", block.begin());

        assert!(!block.is_present(&document));
        assert_eq!(block.remove(&document), None);

        // Writing appends a complete block rather than trying to repair it:
        // guessing where a half-written marker was meant to end is how files
        // get eaten.
        let (result, change) = block.write(&document, "Ours.");
        assert_eq!(change, Change::Added);
        assert!(result.contains("And more."), "{result}");
    }

    #[test]
    fn hash_comments_are_used_where_html_ones_would_show() {
        let block = ManagedBlock::new(CommentStyle::Hash);
        let (result, _) = block.write("key: value\n", "Ours.");

        assert!(result.contains("# ctxc:begin"), "{result}");
        assert!(!result.contains("<!--"), "{result}");
        assert_eq!(block.body(&result).as_deref(), Some("Ours."));
    }

    #[test]
    fn windows_line_endings_do_not_hide_the_markers() {
        let block = markdown();
        let document = format!(
            "Notes.\r\n\r\n{}\r\nOurs.\r\n{}\r\n",
            block.begin(),
            block.end()
        );

        assert!(block.is_present(&document), "{document:?}");
        assert_eq!(block.body(&document).as_deref(), Some("Ours."));
    }
}
