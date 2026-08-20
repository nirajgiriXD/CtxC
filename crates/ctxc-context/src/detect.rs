//! Content detection.
//!
//! Detection is deterministic and never calls a model: a file extension when
//! one is available, otherwise structural sniffing of the content itself. The
//! answer decides which optimizer the router picks, so a wrong guess costs
//! optimization quality, never correctness — every optimizer must be safe on
//! whatever it is handed.

use std::path::Path;

use ctxc_core::ContentType;

/// Bytes examined when sniffing. Enough to characterise a file, small enough
/// to stay cheap on large inputs.
const SNIFF_LIMIT: usize = 8 * 1024;

/// Detect the content type of raw bytes, optionally helped by a filename.
pub fn detect(bytes: &[u8], hint: Option<&Path>) -> ContentType {
    let window = &bytes[..bytes.len().min(SNIFF_LIMIT)];
    if window.contains(&0) {
        return ContentType::Binary;
    }

    if let Some(from_extension) = hint.and_then(extension_type) {
        return from_extension;
    }

    match std::str::from_utf8(window) {
        Ok(text) => sniff(text),
        // Valid UTF-8 can be cut mid-character by the sniff window; the prefix
        // that did decode is still enough to characterise the content.
        Err(err) if err.valid_up_to() > 0 => {
            sniff(&String::from_utf8_lossy(&window[..err.valid_up_to()]))
        }
        Err(_) => ContentType::Binary,
    }
}

/// Detect from text that is already known to be valid UTF-8.
pub fn detect_text(text: &str, hint: Option<&Path>) -> ContentType {
    detect(text.as_bytes(), hint)
}

/// Content type implied by a file extension.
pub fn extension_type(path: &Path) -> Option<ContentType> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let content_type = match extension.as_str() {
        "json" | "jsonl" | "ndjson" => ContentType::Json,
        "md" | "markdown" | "mdx" => ContentType::Markdown,
        "log" => ContentType::Log,
        "txt" | "text" | "rst" => ContentType::PlainText,
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "py" | "go" | "java" | "c" | "h"
        | "cc" | "cpp" | "cxx" | "hpp" | "cs" | "php" | "rb" | "swift" | "kt" | "kts" | "scala"
        | "sh" | "bash" | "zsh" | "fish" | "ps1" | "sql" | "html" | "htm" | "css" | "scss"
        | "sass" | "less" | "vue" | "svelte" | "toml" | "yaml" | "yml" | "xml" | "ini"
        | "gradle" | "lua" | "dart" | "ex" | "exs" | "hs" | "ml" | "r" => ContentType::Code,
        _ => return None,
    };
    Some(content_type)
}

/// Guess a content type from structure alone.
fn sniff(text: &str) -> ContentType {
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return ContentType::PlainText;
    }

    if looks_like_json(trimmed) {
        return ContentType::Json;
    }
    if text.contains('\u{1b}') {
        // ANSI escapes only appear in captured terminal output.
        return ContentType::Terminal;
    }

    let lines: Vec<&str> = text.lines().take(200).collect();
    if lines.is_empty() {
        return ContentType::PlainText;
    }

    if ratio(&lines, looks_like_log_line) >= 0.3 {
        return ContentType::Log;
    }
    if ratio(&lines, looks_like_markdown_line) >= 0.2 {
        return ContentType::Markdown;
    }
    if ratio(&lines, looks_like_code_line) >= 0.2 {
        return ContentType::Code;
    }

    ContentType::PlainText
}

/// Share of non-blank lines satisfying `predicate`.
fn ratio(lines: &[&str], predicate: fn(&str) -> bool) -> f64 {
    let considered: Vec<&&str> = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if considered.is_empty() {
        return 0.0;
    }
    let matching = considered
        .iter()
        .filter(|line| predicate(line.trim()))
        .count();
    matching as f64 / considered.len() as f64
}

/// A structural check, not a parse: the optimizer for JSON does the real
/// parsing, and it arrives with tool-output support.
fn looks_like_json(trimmed: &str) -> bool {
    let tail = trimmed.trim_end();
    let object = trimmed.starts_with('{') && tail.ends_with('}');
    let array = trimmed.starts_with('[') && tail.ends_with(']');

    if object || array {
        trimmed.contains(':') || (array && trimmed.contains(','))
    } else {
        false
    }
}

fn looks_like_log_line(line: &str) -> bool {
    const LEVELS: [&str; 12] = [
        "ERROR", "WARN", "WARNING", "INFO", "DEBUG", "TRACE", "FATAL", "error:", "warning:",
        "info:", "debug:", "panic:",
    ];
    if LEVELS.iter().any(|level| line.contains(level)) {
        return true;
    }
    // Leading timestamp, e.g. "2026-08-19 09:30:00" or "09:30:00".
    starts_with_date(line) || starts_with_clock(line)
}

fn starts_with_date(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes.len() >= 10
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && (bytes[4] == b'-' || bytes[4] == b'/')
        && bytes[5..7].iter().all(u8::is_ascii_digit)
}

fn starts_with_clock(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes.len() >= 8
        && bytes[..2].iter().all(u8::is_ascii_digit)
        && bytes[2] == b':'
        && bytes[3..5].iter().all(u8::is_ascii_digit)
        && bytes[5] == b':'
}

fn looks_like_markdown_line(line: &str) -> bool {
    line.starts_with("# ")
        || line.starts_with("## ")
        || line.starts_with("### ")
        || line.starts_with("- ")
        || line.starts_with("* ")
        || line.starts_with("> ")
        || line.starts_with("```")
        || line.starts_with("| ")
}

fn looks_like_code_line(line: &str) -> bool {
    const KEYWORDS: [&str; 14] = [
        "fn ",
        "def ",
        "function ",
        "class ",
        "import ",
        "from ",
        "const ",
        "let ",
        "var ",
        "public ",
        "private ",
        "#include",
        "package ",
        "return ",
    ];
    KEYWORDS.iter().any(|keyword| line.starts_with(keyword))
        || line.ends_with('{')
        || line.ends_with(';')
        || line == "}"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn detect_str(text: &str) -> ContentType {
        detect_text(text, None)
    }

    #[test]
    fn extensions_win_when_present() {
        let path = PathBuf::from("src/main.rs");
        assert_eq!(
            detect_text("anything at all", Some(&path)),
            ContentType::Code
        );

        let path = PathBuf::from("notes.md");
        assert_eq!(
            detect_text("plain prose", Some(&path)),
            ContentType::Markdown
        );

        let path = PathBuf::from("data.JSON");
        assert_eq!(
            detect_text("not really json", Some(&path)),
            ContentType::Json,
            "extensions are case insensitive"
        );
    }

    #[test]
    fn unknown_extensions_fall_through_to_sniffing() {
        let path = PathBuf::from("output.weird");
        assert_eq!(
            detect_text("{\"ok\": true}", Some(&path)),
            ContentType::Json
        );
    }

    #[test]
    fn binary_content_is_detected_before_anything_else() {
        let path = PathBuf::from("program.rs");
        assert_eq!(detect(b"fn main\0()", Some(&path)), ContentType::Binary);
    }

    #[test]
    fn json_is_sniffed() {
        assert_eq!(detect_str("{\"name\": \"ctxc\"}"), ContentType::Json);
        assert_eq!(detect_str("  [\n  {\"a\": 1}\n]\n"), ContentType::Json);
    }

    #[test]
    fn terminal_output_is_sniffed_from_escapes() {
        assert_eq!(
            detect_str("\u{1b}[32mok\u{1b}[0m all good"),
            ContentType::Terminal
        );
    }

    #[test]
    fn logs_are_sniffed_from_levels_and_timestamps() {
        let log = "2026-08-19 09:30:00 INFO starting\n2026-08-19 09:30:01 ERROR boom\n";
        assert_eq!(detect_str(log), ContentType::Log);

        let bare = "09:30:00 request finished\n09:30:02 request finished\n";
        assert_eq!(detect_str(bare), ContentType::Log);
    }

    #[test]
    fn markdown_is_sniffed_from_structure() {
        let markdown = "# Title\n\nSome prose.\n\n- one\n- two\n";
        assert_eq!(detect_str(markdown), ContentType::Markdown);
    }

    #[test]
    fn code_is_sniffed_from_structure() {
        let code = "import os\n\ndef main():\n    return 1\n";
        assert_eq!(detect_str(code), ContentType::Code);
    }

    #[test]
    fn prose_stays_plain_text() {
        let prose = "CtxC reduces the amount of context an agent has to read.\n\
                     It does that without throwing away the parts that matter.\n";
        assert_eq!(detect_str(prose), ContentType::PlainText);
    }

    #[test]
    fn empty_input_is_plain_text() {
        assert_eq!(detect_str(""), ContentType::PlainText);
        assert_eq!(detect_str("   \n\n"), ContentType::PlainText);
    }

    #[test]
    fn truncated_multibyte_input_still_sniffs() {
        // Place a three-byte character so the 8 KiB sniff window cuts through
        // it. The prefix that decoded must still be classified, not written off
        // as binary.
        let mut text = "# Heading\n".repeat(819); // 8190 bytes
        assert_eq!(text.len(), SNIFF_LIMIT - 2);
        text.push('\u{4e2d}');
        text.push_str("\n\n- trailing list item\n");

        assert_eq!(detect_str(&text), ContentType::Markdown);
    }
}
