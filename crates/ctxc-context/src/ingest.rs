//! Context ingestion.
//!
//! Turning raw material into a [`Context`]: read it, normalize it, detect what
//! it is. Normalization is limited to things that carry no information — a byte
//! order mark, platform line endings — because anything that changes meaning
//! belongs to an optimizer, where it is measured and attributed.

use std::io::Read;
use std::path::{Path, PathBuf};

use ctxc_core::{ContentType, Context, ContextSource};

use crate::detect;
use crate::error::{ContextError, Result};

/// Largest input accepted in one piece. Bigger material is a job for the
/// indexer, not for a single in-memory context.
pub const MAX_INPUT_BYTES: u64 = 16 * 1024 * 1024;

/// Read a file into a context.
pub fn from_path(path: &Path) -> Result<Context> {
    let metadata = std::fs::metadata(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ContextError::NotFound {
                path: path.to_path_buf(),
            }
        } else {
            ContextError::Io {
                action: "read",
                path: path.to_path_buf(),
                source,
            }
        }
    })?;

    if metadata.is_dir() {
        return Err(ContextError::IsDirectory {
            path: path.to_path_buf(),
        });
    }
    if metadata.len() > MAX_INPUT_BYTES {
        return Err(ContextError::TooLarge {
            path: path.to_path_buf(),
            size: metadata.len(),
            limit: MAX_INPUT_BYTES,
        });
    }

    let bytes = std::fs::read(path).map_err(|source| ContextError::Io {
        action: "read",
        path: path.to_path_buf(),
        source,
    })?;

    from_bytes(
        ContextSource::File {
            path: path.to_path_buf(),
        },
        &bytes,
        Some(path),
    )
}

/// Read everything from `reader` into a context.
///
/// Used for stdin and for captured command output, where the size is not known
/// in advance: the limit is enforced while reading rather than after, so a
/// runaway producer cannot exhaust memory.
pub fn from_reader(source: ContextSource, reader: &mut dyn Read) -> Result<Context> {
    let mut bytes = Vec::new();
    let mut limited = reader.take(MAX_INPUT_BYTES + 1);
    limited
        .read_to_end(&mut bytes)
        .map_err(|source| ContextError::Io {
            action: "read input",
            path: PathBuf::from("<stdin>"),
            source,
        })?;

    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(ContextError::TooLarge {
            path: PathBuf::from("<stdin>"),
            size: bytes.len() as u64,
            limit: MAX_INPUT_BYTES,
        });
    }

    from_bytes(source, &bytes, None)
}

/// Read standard input into a context.
pub fn from_stdin() -> Result<Context> {
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    from_reader(ContextSource::Stdin, &mut handle)
}

/// Build a context from raw bytes.
///
/// Binary material is rejected rather than mangled: a context is text, and
/// silently lossy decoding would be worse than a clear error.
pub fn from_bytes(source: ContextSource, bytes: &[u8], hint: Option<&Path>) -> Result<Context> {
    let content_type = detect::detect(bytes, hint);
    if content_type == ContentType::Binary {
        return Err(ContextError::Binary {
            path: hint.map(Path::to_path_buf),
        });
    }

    let text = std::str::from_utf8(bytes).map_err(|_| ContextError::Binary {
        path: hint.map(Path::to_path_buf),
    })?;

    Ok(Context::new(source, content_type, normalize(text)))
}

/// Build a context from text that is already in memory.
pub fn from_text(source: ContextSource, text: &str, hint: Option<&Path>) -> Context {
    let normalized = normalize(text);
    let content_type = detect::detect_text(&normalized, hint);
    Context::new(source, content_type, normalized)
}

/// Strip a byte order mark and normalize line endings to `\n`.
///
/// Nothing else: this must not change what the content says, only how it is
/// encoded, so that the same file ingested on Windows and Linux produces the
/// same context id.
pub fn normalize(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    if !text.contains('\r') {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                // Both CRLF and a lone CR become a single newline.
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            other => out.push(other),
        }
    }
    out
}

/// Describe an input path for error messages and reports.
pub fn label(source: &ContextSource) -> String {
    match source {
        ContextSource::File { path } => path.display().to_string(),
        ContextSource::Command { command } => command.clone(),
        ContextSource::Api { endpoint } => endpoint.clone(),
        ContextSource::Stdin => "<stdin>".into(),
        ContextSource::Memory => "<memory>".into(),
        ContextSource::Unknown => "<unknown>".into(),
    }
}

/// Path of a file-backed source, if it has one.
pub fn source_path(source: &ContextSource) -> Option<PathBuf> {
    match source {
        ContextSource::File { path } => Some(path.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let path = std::env::temp_dir()
                .join("ctxc-ingest-tests")
                .join(format!("{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn file(&self, name: &str, contents: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, contents).unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reads_a_file_and_detects_its_type() {
        let dir = TempDir::new("read");
        let path = dir.file("main.rs", b"fn main() {}\n");

        let context = from_path(&path).unwrap();
        assert_eq!(context.metadata.content_type, ContentType::Code);
        assert_eq!(context.content, "fn main() {}\n");
        assert!(matches!(
            context.metadata.source,
            ContextSource::File { .. }
        ));
    }

    #[test]
    fn line_endings_are_normalized() {
        assert_eq!(normalize("a\r\nb\r\n"), "a\nb\n");
        assert_eq!(normalize("a\rb"), "a\nb");
        assert_eq!(normalize("\u{feff}already clean\n"), "already clean\n");
    }

    #[test]
    fn identical_files_have_the_same_id_across_platforms() {
        let windows = from_text(ContextSource::Stdin, "line one\r\nline two\r\n", None);
        let unix = from_text(ContextSource::Stdin, "line one\nline two\n", None);
        assert_eq!(windows.id, unix.id);
    }

    #[test]
    fn directories_are_refused_with_a_pointer_to_indexing() {
        let dir = TempDir::new("directory");
        let error = from_path(&dir.0).unwrap_err();

        assert!(matches!(error, ContextError::IsDirectory { .. }));
        assert!(error.hint().unwrap().contains("file"));
    }

    #[test]
    fn missing_files_report_the_path() {
        let error = from_path(Path::new("definitely-not-here.txt")).unwrap_err();
        assert!(matches!(error, ContextError::NotFound { .. }));
    }

    #[test]
    fn binary_files_are_refused() {
        let dir = TempDir::new("binary");
        let path = dir.file("blob.bin", &[0x00, 0x01, 0x02, 0x03]);

        let error = from_path(&path).unwrap_err();
        assert!(matches!(error, ContextError::Binary { .. }));
    }

    #[test]
    fn invalid_utf8_is_refused_rather_than_mangled() {
        let error = from_bytes(ContextSource::Stdin, &[0xff, 0xfe, 0x41], None).unwrap_err();
        assert!(matches!(error, ContextError::Binary { .. }));
    }

    #[test]
    fn text_ingestion_detects_content_without_a_path() {
        let context = from_text(ContextSource::Stdin, "{\"a\": 1}", None);
        assert_eq!(context.metadata.content_type, ContentType::Json);
    }

    #[test]
    fn readers_are_ingested_like_files() {
        let mut input = std::io::Cursor::new(b"{\"piped\": true}".to_vec());
        let context = from_reader(ContextSource::Stdin, &mut input).unwrap();

        assert_eq!(context.metadata.content_type, ContentType::Json);
        assert_eq!(context.metadata.source, ContextSource::Stdin);
    }

    #[test]
    fn oversized_streams_are_refused_rather_than_buffered() {
        let oversized = vec![b'x'; (MAX_INPUT_BYTES + 10) as usize];
        let mut input = std::io::Cursor::new(oversized);
        let error = from_reader(ContextSource::Stdin, &mut input).unwrap_err();

        assert!(matches!(error, ContextError::TooLarge { .. }));
    }

    #[test]
    fn captured_output_keeps_its_command() {
        let mut input = std::io::Cursor::new(b"On branch main\n".to_vec());
        let context = from_reader(
            ContextSource::Command {
                command: "git status".into(),
            },
            &mut input,
        )
        .unwrap();

        assert_eq!(
            context.metadata.source,
            ContextSource::Command {
                command: "git status".into()
            }
        );
    }

    #[test]
    fn labels_describe_every_source() {
        assert_eq!(label(&ContextSource::Stdin), "<stdin>");
        assert_eq!(
            label(&ContextSource::Command {
                command: "git status".into()
            }),
            "git status"
        );
    }
}
