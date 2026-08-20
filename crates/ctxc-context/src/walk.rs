//! Walking a project directory.
//!
//! Ignore rules are applied *before* descending, not after collecting: a
//! repository with a `node_modules` in it must never cost the time it would
//! take to enumerate one. Paths come back relative to the root and always with
//! forward slashes, so an index built on Windows describes the same files as
//! one built on Linux.

use std::collections::VecDeque;
use std::path::{Component, Path, PathBuf};

use crate::error::{ContextError, Result};

/// Directories and files CtxC never indexes.
///
/// Build output, dependency trees and virtual environments: large, generated,
/// and never what an agent needs to reason about the project.
pub const DEFAULT_IGNORES: &[&str] = &[
    ".git/",
    ".hg/",
    ".svn/",
    "node_modules/",
    "target/",
    "dist/",
    "build/",
    "out/",
    ".next/",
    ".nuxt/",
    ".svelte-kit/",
    ".venv/",
    "venv/",
    "__pycache__/",
    ".mypy_cache/",
    ".pytest_cache/",
    ".ruff_cache/",
    ".tox/",
    ".gradle/",
    "vendor/",
    "coverage/",
    ".cache/",
    ".idea/",
    ".vscode/",
    "*.min.js",
    "*.min.css",
];

/// A file found by the walker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkEntry {
    /// Path relative to the root, with forward slashes.
    pub path: String,
    /// Full path on disk.
    pub absolute: PathBuf,
    pub size: u64,
    pub mtime_ms: i64,
}

/// How to walk.
#[derive(Debug, Clone)]
pub struct WalkOptions {
    /// Apply CtxC's built-in ignore list.
    pub use_defaults: bool,
    /// Read `.gitignore` at the root.
    pub use_gitignore: bool,
    /// Extra patterns, in gitignore syntax.
    pub extra: Vec<String>,
    /// Largest file to consider, in bytes.
    pub max_file_size: u64,
}

impl Default for WalkOptions {
    fn default() -> Self {
        WalkOptions {
            use_defaults: true,
            use_gitignore: true,
            extra: Vec::new(),
            max_file_size: 2 * 1024 * 1024,
        }
    }
}

/// What a walk found, and what it skipped.
#[derive(Debug, Clone, Default)]
pub struct Walk {
    pub entries: Vec<WalkEntry>,
    /// Files skipped because a rule matched them.
    pub ignored: u64,
    /// Files skipped for being larger than the limit.
    pub too_large: u64,
}

/// Walk `root`, returning the files that survive the ignore rules.
pub fn walk(root: &Path, options: &WalkOptions) -> Result<Walk> {
    let metadata = std::fs::metadata(root).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ContextError::NotFound {
                path: root.to_path_buf(),
            }
        } else {
            ContextError::Io {
                action: "read",
                path: root.to_path_buf(),
                source,
            }
        }
    })?;
    if !metadata.is_dir() {
        return Err(ContextError::NotADirectory {
            path: root.to_path_buf(),
        });
    }

    let rules = IgnoreRules::for_root(root, options)?;
    let mut walk = Walk::default();
    let mut queue = VecDeque::new();
    queue.push_back((root.to_path_buf(), String::new()));

    while let Some((directory, prefix)) = queue.pop_front() {
        let listing = match std::fs::read_dir(&directory) {
            Ok(listing) => listing,
            // A directory that disappeared or cannot be read is skipped rather
            // than fatal: indexing must survive a repository being worked on.
            Err(err) => {
                tracing::debug!(path = %directory.display(), error = %err, "skipping directory");
                continue;
            }
        };

        for entry in listing.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };

            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            // Symlinks are not followed: they turn a walk into a graph search
            // and can loop.
            if file_type.is_symlink() {
                continue;
            }

            if file_type.is_dir() {
                if rules.is_ignored(&relative, true) {
                    walk.ignored += 1;
                    continue;
                }
                queue.push_back((entry.path(), relative));
                continue;
            }

            if rules.is_ignored(&relative, false) {
                walk.ignored += 1;
                continue;
            }

            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.len() > options.max_file_size {
                walk.too_large += 1;
                continue;
            }

            walk.entries.push(WalkEntry {
                path: relative,
                absolute: entry.path(),
                size: metadata.len(),
                mtime_ms: mtime_ms(&metadata),
            });
        }
    }

    walk.entries
        .sort_by(|left, right| left.path.cmp(&right.path));
    Ok(walk)
}

/// Modification time in epoch milliseconds, or 0 when the platform will not say.
fn mtime_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|delta| delta.as_millis() as i64)
        .unwrap_or(0)
}

/// Convert a path to the repository-relative form the index stores.
pub fn relative_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

/// An ordered set of ignore patterns.
#[derive(Debug, Default)]
pub struct IgnoreRules {
    patterns: Vec<Pattern>,
}

impl IgnoreRules {
    /// Build the rules for a root: defaults, then `.gitignore`, then extras.
    ///
    /// Later rules win, which is how a project overrides a default and how a
    /// negation re-includes something an earlier rule excluded.
    pub fn for_root(root: &Path, options: &WalkOptions) -> Result<IgnoreRules> {
        let mut rules = IgnoreRules::default();

        if options.use_defaults {
            rules.extend(DEFAULT_IGNORES.iter().copied());
        }
        if options.use_gitignore {
            let gitignore = root.join(".gitignore");
            match std::fs::read_to_string(&gitignore) {
                Ok(contents) => rules.extend(contents.lines()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(ContextError::Io {
                        action: "read .gitignore",
                        path: gitignore,
                        source,
                    })
                }
            }
        }
        rules.extend(options.extra.iter().map(String::as_str));

        Ok(rules)
    }

    /// Add patterns in gitignore syntax, skipping blanks and comments.
    pub fn extend<'a>(&mut self, patterns: impl IntoIterator<Item = &'a str>) {
        for line in patterns {
            if let Some(pattern) = Pattern::parse(line) {
                self.patterns.push(pattern);
            }
        }
    }

    /// Whether a repository-relative path is ignored.
    pub fn is_ignored(&self, path: &str, is_dir: bool) -> bool {
        let mut ignored = false;
        for pattern in &self.patterns {
            if pattern.matches(path, is_dir) {
                ignored = !pattern.negated;
            }
        }
        ignored
    }
}

/// One ignore pattern.
#[derive(Debug)]
struct Pattern {
    text: String,
    /// `!pattern` re-includes what an earlier pattern excluded.
    negated: bool,
    /// `pattern/` matches directories only.
    dir_only: bool,
    /// A pattern containing a slash is matched against the whole relative path;
    /// one without is matched against any single path component.
    anchored: bool,
}

impl Pattern {
    fn parse(line: &str) -> Option<Pattern> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }

        let (negated, rest) = match trimmed.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, trimmed),
        };

        let (dir_only, rest) = match rest.strip_suffix('/') {
            Some(rest) => (true, rest),
            None => (false, rest),
        };

        let anchored = rest.contains('/');
        let rest = rest.strip_prefix('/').unwrap_or(rest);
        if rest.is_empty() {
            return None;
        }

        Some(Pattern {
            text: rest.to_string(),
            negated,
            dir_only,
            anchored,
        })
    }

    fn matches(&self, path: &str, is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            // A directory rule still hides everything under it, which the
            // walker gets for free by not descending; a file path that starts
            // with the directory name is matched here for callers that test
            // paths directly.
            return path
                .strip_prefix(&self.text)
                .is_some_and(|rest| rest.starts_with('/'));
        }

        if self.anchored {
            return glob_match(&self.text, path);
        }

        // Unanchored patterns match any component, and everything beneath a
        // matching directory component.
        path.split('/').any(|part| glob_match(&self.text, part))
    }
}

/// Glob matching with `*`, `?` and `**`.
///
/// `*` stops at a path separator, `**` crosses them, which is the behaviour
/// gitignore describes and what callers expect from `src/**/*.rs`.
fn glob_match(pattern: &str, text: &str) -> bool {
    match_from(pattern.as_bytes(), text.as_bytes())
}

fn match_from(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }

    match pattern[0] {
        b'*' => {
            if pattern.starts_with(b"**") {
                let rest = strip_double_star(pattern);
                // `**` matches any number of characters, separators included.
                (0..=text.len()).any(|split| match_from(rest, &text[split..]))
            } else {
                let rest = &pattern[1..];
                // `*` stops at the next separator.
                let limit = text
                    .iter()
                    .position(|byte| *byte == b'/')
                    .unwrap_or(text.len());
                (0..=limit).any(|split| match_from(rest, &text[split..]))
            }
        }
        b'?' => !text.is_empty() && text[0] != b'/' && match_from(&pattern[1..], &text[1..]),
        literal => !text.is_empty() && text[0] == literal && match_from(&pattern[1..], &text[1..]),
    }
}

/// Skip a `**` and the separator that usually follows it.
fn strip_double_star(pattern: &[u8]) -> &[u8] {
    let mut rest = &pattern[2..];
    while rest.first() == Some(&b'*') {
        rest = &rest[1..];
    }
    if rest.first() == Some(&b'/') {
        // `**/x` must also match a bare `x`, so the separator is optional.
        return &rest[1..];
    }
    rest
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempTree(PathBuf);

    impl TempTree {
        fn new(name: &str) -> TempTree {
            let path = std::env::temp_dir()
                .join("ctxc-walk-tests")
                .join(format!("{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempTree(path)
        }

        fn file(&self, relative: &str, contents: &str) {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn rules(patterns: &[&str]) -> IgnoreRules {
        let mut rules = IgnoreRules::default();
        rules.extend(patterns.iter().copied());
        rules
    }

    #[test]
    fn globs_respect_path_separators() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(
            !glob_match("*.rs", "src/main.rs"),
            "* does not cross a slash"
        );
        assert!(glob_match("src/*.rs", "src/main.rs"));
        assert!(glob_match("src/**/*.rs", "src/a/b/main.rs"));
        assert!(glob_match("**/target", "a/b/target"));
        assert!(glob_match("**/target", "target"), "** may match nothing");
        assert!(glob_match("?.rs", "a.rs"));
        assert!(!glob_match("?.rs", "ab.rs"));
    }

    #[test]
    fn plain_names_match_at_any_depth() {
        let rules = rules(&["node_modules/"]);
        assert!(rules.is_ignored("node_modules", true));
        assert!(rules.is_ignored("packages/web/node_modules", true));
        assert!(rules.is_ignored("node_modules/react/index.js", false));
        assert!(!rules.is_ignored("src/main.rs", false));
    }

    #[test]
    fn directory_rules_do_not_match_files_of_the_same_name() {
        let rules = rules(&["build/"]);
        assert!(rules.is_ignored("build", true));
        assert!(
            !rules.is_ignored("build", false),
            "a file named build stays"
        );
    }

    #[test]
    fn anchored_patterns_only_match_from_the_root() {
        let rules = rules(&["/dist"]);
        assert!(rules.is_ignored("dist", true));
        assert!(
            !rules.is_ignored("packages/web/dist", true),
            "a leading slash anchors the pattern"
        );
    }

    #[test]
    fn negations_re_include() {
        let rules = rules(&["*.log", "!keep.log"]);
        assert!(rules.is_ignored("debug.log", false));
        assert!(!rules.is_ignored("keep.log", false));
    }

    #[test]
    fn later_rules_win() {
        let rules = rules(&["!keep.log", "*.log"]);
        assert!(rules.is_ignored("keep.log", false), "order decides");
    }

    #[test]
    fn comments_and_blanks_are_skipped() {
        let rules = rules(&["# a comment", "", "   ", "*.tmp"]);
        assert!(rules.is_ignored("scratch.tmp", false));
        assert!(!rules.is_ignored("a comment", false));
    }

    #[test]
    fn the_walker_returns_relative_forward_slash_paths() {
        let tree = TempTree::new("paths");
        tree.file("src/main.rs", "fn main() {}");
        tree.file("src/nested/deep.rs", "");
        tree.file("README.md", "# hi");

        let walk = walk(&tree.0, &WalkOptions::default()).unwrap();
        let paths: Vec<&str> = walk
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();

        assert_eq!(
            paths,
            vec!["README.md", "src/main.rs", "src/nested/deep.rs"]
        );
    }

    #[test]
    fn the_walker_skips_ignored_directories_entirely() {
        let tree = TempTree::new("ignored");
        tree.file("src/main.rs", "");
        tree.file("node_modules/react/index.js", "");
        tree.file("target/debug/build.rs", "");

        let walk = walk(&tree.0, &WalkOptions::default()).unwrap();
        let paths: Vec<&str> = walk
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();

        assert_eq!(paths, vec!["src/main.rs"]);
        assert!(walk.ignored >= 2);
    }

    #[test]
    fn the_walker_reads_gitignore() {
        let tree = TempTree::new("gitignore");
        tree.file(".gitignore", "secrets.txt\ngenerated/\n");
        tree.file("secrets.txt", "");
        tree.file("generated/api.ts", "");
        tree.file("src/app.ts", "");

        let walk = walk(&tree.0, &WalkOptions::default()).unwrap();
        let paths: Vec<&str> = walk
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();

        assert_eq!(paths, vec![".gitignore", "src/app.ts"]);
    }

    #[test]
    fn gitignore_can_be_disabled() {
        let tree = TempTree::new("no-gitignore");
        tree.file(".gitignore", "keep-me.txt\n");
        tree.file("keep-me.txt", "");

        let options = WalkOptions {
            use_gitignore: false,
            ..WalkOptions::default()
        };
        let walk = walk(&tree.0, &options).unwrap();
        assert!(walk.entries.iter().any(|entry| entry.path == "keep-me.txt"));
    }

    #[test]
    fn oversized_files_are_counted_not_returned() {
        let tree = TempTree::new("large");
        tree.file("big.txt", &"x".repeat(2_000));
        tree.file("small.txt", "x");

        let options = WalkOptions {
            max_file_size: 1_000,
            ..WalkOptions::default()
        };
        let walk = walk(&tree.0, &options).unwrap();

        assert_eq!(walk.entries.len(), 1);
        assert_eq!(walk.too_large, 1);
    }

    #[test]
    fn entries_carry_metadata_for_change_detection() {
        let tree = TempTree::new("metadata");
        tree.file("a.txt", "hello");

        let walk = walk(&tree.0, &WalkOptions::default()).unwrap();
        let entry = &walk.entries[0];

        assert_eq!(entry.size, 5);
        assert!(entry.mtime_ms > 0);
        assert!(entry.absolute.ends_with("a.txt"));
    }

    #[test]
    fn walking_a_file_or_a_missing_path_is_an_error() {
        let tree = TempTree::new("errors");
        tree.file("a.txt", "");

        let error = walk(&tree.0.join("a.txt"), &WalkOptions::default()).unwrap_err();
        assert!(matches!(error, ContextError::NotADirectory { .. }));

        let error = walk(&tree.0.join("missing"), &WalkOptions::default()).unwrap_err();
        assert!(matches!(error, ContextError::NotFound { .. }));
    }

    #[test]
    fn relative_paths_use_forward_slashes() {
        let root = PathBuf::from("/repo");
        assert_eq!(
            relative_path(&root, &root.join("src").join("main.rs")),
            Some("src/main.rs".to_string())
        );
        assert_eq!(relative_path(&root, &PathBuf::from("/elsewhere")), None);
    }
}
