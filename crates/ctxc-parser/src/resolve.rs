//! Turning what an import says into the file it means.
//!
//! Resolution is best effort and deliberately shallow: no `tsconfig` paths, no
//! Python package resolution, no Cargo workspace lookup. It resolves the cases
//! that are unambiguous from the text and the file layout, and leaves the rest
//! recorded as written — "imports something called express" is still worth
//! knowing, and a wrong edge is worse than a missing one.

use crate::language::Language;

/// Answers whether a repository-relative path exists.
///
/// Passing this in rather than touching the filesystem keeps resolution a pure
/// function, which is what makes it testable across platforms.
pub trait FileLookup {
    fn exists(&self, relative_path: &str) -> bool;
}

impl<F: Fn(&str) -> bool> FileLookup for F {
    fn exists(&self, relative_path: &str) -> bool {
        self(relative_path)
    }
}

/// Resolve `target` as imported by `from`, a repository-relative path.
pub fn resolve_import(
    language: Language,
    from: &str,
    target: &str,
    files: &dyn FileLookup,
) -> Option<String> {
    match language {
        Language::JavaScript | Language::TypeScript | Language::Tsx => {
            resolve_relative(language, from, target, files)
        }
        Language::Python => resolve_python(from, target, files),
        Language::Rust => resolve_rust(from, target, files),
        // Go imports name modules, not files; resolving them needs the module
        // graph, which CtxC does not read.
        Language::Go => None,
    }
}

/// JavaScript and TypeScript: only relative specifiers name a file.
fn resolve_relative(
    language: Language,
    from: &str,
    target: &str,
    files: &dyn FileLookup,
) -> Option<String> {
    if !target.starts_with('.') {
        return None;
    }

    let base = join(parent_of(from), target);
    let mut candidates = Vec::new();

    // An explicit extension is used as written; TypeScript sources are also
    // imported as ".js", which resolves back to the ".ts" that produced it.
    if let Some(stem) = base.strip_suffix(".js") {
        candidates.push(format!("{stem}.ts"));
        candidates.push(format!("{stem}.tsx"));
    }
    candidates.push(base.clone());

    for extension in language.extensions() {
        candidates.push(format!("{base}.{extension}"));
    }
    for extension in language.extensions() {
        candidates.push(format!("{base}/index.{extension}"));
    }

    candidates.into_iter().find(|path| files.exists(path))
}

/// Python: relative imports name a module beside the importer, absolute ones a
/// module from the repository root.
fn resolve_python(from: &str, target: &str, files: &dyn FileLookup) -> Option<String> {
    let (base, module) = if let Some(rest) = target.strip_prefix('.') {
        // Each leading dot climbs one directory.
        let mut directory = parent_of(from).to_string();
        let mut remainder = rest;
        while let Some(next) = remainder.strip_prefix('.') {
            directory = parent_of(&directory).to_string();
            remainder = next;
        }
        (directory, remainder.replace('.', "/"))
    } else {
        (String::new(), target.replace('.', "/"))
    };

    if module.is_empty() {
        return None;
    }

    let stem = join(&base, &module);
    [
        format!("{stem}.py"),
        format!("{stem}/__init__.py"),
        format!("{stem}.pyi"),
    ]
    .into_iter()
    .find(|path| files.exists(path))
}

/// Rust: `crate::` and `super::` paths point inside the same crate, and a
/// module is either `name.rs` or `name/mod.rs`.
fn resolve_rust(from: &str, target: &str, files: &dyn FileLookup) -> Option<String> {
    let mut segments: Vec<&str> = target.split("::").collect();
    let root = *segments.first()?;

    let base = match root {
        "crate" => {
            segments.remove(0);
            source_root_of(from)
        }
        "self" => {
            segments.remove(0);
            module_directory(from)
        }
        "super" => {
            segments.remove(0);
            parent_of(&module_directory(from)).to_string()
        }
        // Another crate: in a workspace it may still be in this repository, so
        // look for a crate directory with that name before giving up. Nothing
        // is invented — an edge appears only if the file is really there.
        name => {
            let base = workspace_crate_root(name, files)?;
            segments.remove(0);
            base
        }
    };

    // Trailing segments name items rather than modules, so try progressively
    // shorter module paths: `crate::auth::session::verify` may live in
    // `auth/session.rs` or in `auth.rs`.
    while !segments.is_empty() {
        let module = segments.join("/");
        let stem = join(&base, &module);
        for candidate in [format!("{stem}.rs"), format!("{stem}/mod.rs")] {
            if files.exists(&candidate) {
                return Some(candidate);
            }
        }
        segments.pop();
    }

    // Everything left names an item defined at the crate root, so the crate
    // root itself is the file that was imported.
    [format!("{base}/lib.rs"), format!("{base}/main.rs")]
        .into_iter()
        .find(|candidate| files.exists(candidate))
}

/// The `src` directory of a sibling crate in the same repository.
///
/// Cargo crate names use dashes where Rust paths use underscores, and workspace
/// members usually sit under `crates/`, so both spellings are tried. Resolution
/// still depends on the file existing, which keeps a guess from becoming a
/// wrong edge.
fn workspace_crate_root(name: &str, files: &dyn FileLookup) -> Option<String> {
    let dashed = name.replace('_', "-");
    let candidates = [
        format!("crates/{dashed}/src"),
        format!("crates/{name}/src"),
        format!("{dashed}/src"),
        format!("{name}/src"),
    ];

    candidates
        .into_iter()
        .find(|base| files.exists(&format!("{base}/lib.rs")))
}

/// The directory a Rust file's own submodules live in.
///
/// `src/auth/mod.rs` *is* the `auth` module, so its children are its siblings;
/// `src/auth/session.rs` is a module whose children live in a directory named
/// after it. Getting this wrong makes `super::` climb one level too far.
fn module_directory(from: &str) -> String {
    let stem = from.rsplit_once('.').map(|(head, _)| head).unwrap_or(from);
    let file_name = stem.rsplit('/').next().unwrap_or(stem);

    if matches!(file_name, "mod" | "lib" | "main") {
        parent_of(from).to_string()
    } else {
        stem.to_string()
    }
}

/// The `src` directory (or equivalent) a Rust file lives under.
fn source_root_of(from: &str) -> String {
    match from.split_once("src/") {
        Some((prefix, _)) => format!("{prefix}src"),
        None => parent_of(from).to_string(),
    }
}

/// Directory containing a repository-relative path.
fn parent_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(index) => &path[..index],
        None => "",
    }
}

/// Join two repository-relative path fragments, resolving `.` and `..`.
fn join(base: &str, relative: &str) -> String {
    let mut segments: Vec<&str> = base.split('/').filter(|part| !part.is_empty()).collect();

    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pretend repository listing.
    fn repository(paths: &'static [&'static str]) -> impl FileLookup {
        move |candidate: &str| paths.contains(&candidate)
    }

    #[test]
    fn typescript_relative_imports_resolve() {
        let files = repository(&["src/database.ts", "src/auth/index.ts", "src/util.tsx"]);

        assert_eq!(
            resolve_import(Language::TypeScript, "src/api.ts", "./database", &files),
            Some("src/database.ts".into())
        );
        assert_eq!(
            resolve_import(Language::TypeScript, "src/api.ts", "./auth", &files),
            Some("src/auth/index.ts".into()),
            "a directory resolves through its index file"
        );
        assert_eq!(
            resolve_import(Language::TypeScript, "src/auth/index.ts", "../util", &files),
            Some("src/util.tsx".into())
        );
    }

    #[test]
    fn typescript_js_specifiers_resolve_to_their_sources() {
        let files = repository(&["src/database.ts"]);
        assert_eq!(
            resolve_import(Language::TypeScript, "src/api.ts", "./database.js", &files),
            Some("src/database.ts".into()),
            "ESM imports name the emitted file, not the source"
        );
    }

    #[test]
    fn package_imports_stay_unresolved() {
        let files = repository(&["src/database.ts", "node_modules/express/index.js"]);
        assert_eq!(
            resolve_import(Language::TypeScript, "src/api.ts", "express", &files),
            None
        );
    }

    #[test]
    fn python_relative_and_absolute_imports_resolve() {
        let files = repository(&["app/database.py", "app/models/__init__.py", "lib/util.py"]);

        assert_eq!(
            resolve_import(Language::Python, "app/api.py", ".database", &files),
            Some("app/database.py".into())
        );
        assert_eq!(
            resolve_import(Language::Python, "app/api.py", ".models", &files),
            Some("app/models/__init__.py".into())
        );
        assert_eq!(
            resolve_import(Language::Python, "app/api.py", "lib.util", &files),
            Some("lib/util.py".into())
        );
        assert_eq!(
            resolve_import(Language::Python, "app/nested/api.py", "..database", &files),
            Some("app/database.py".into()),
            "each leading dot climbs a directory"
        );
    }

    #[test]
    fn python_standard_library_imports_stay_unresolved() {
        let files = repository(&["app/database.py"]);
        assert_eq!(
            resolve_import(Language::Python, "app/api.py", "os", &files),
            None
        );
    }

    #[test]
    fn rust_crate_paths_resolve_to_modules() {
        let files = repository(&[
            "src/database.rs",
            "src/auth/mod.rs",
            "src/auth/session.rs",
            "src/lib.rs",
        ]);

        assert_eq!(
            resolve_import(
                Language::Rust,
                "src/api.rs",
                "crate::database::Connection",
                &files
            ),
            Some("src/database.rs".into())
        );
        assert_eq!(
            resolve_import(Language::Rust, "src/api.rs", "crate::auth", &files),
            Some("src/auth/mod.rs".into())
        );
        assert_eq!(
            resolve_import(
                Language::Rust,
                "src/api.rs",
                "crate::auth::session::verify",
                &files
            ),
            Some("src/auth/session.rs".into()),
            "trailing item names are dropped until a module matches"
        );
    }

    #[test]
    fn rust_relative_paths_resolve_against_the_module_not_the_directory() {
        let files = repository(&[
            "src/auth/session.rs",
            "src/auth/mod.rs",
            "src/auth/tokens.rs",
            "src/database.rs",
        ]);

        assert_eq!(
            resolve_import(Language::Rust, "src/auth/mod.rs", "self::session", &files),
            Some("src/auth/session.rs".into()),
            "mod.rs is the module itself, so its children are its siblings"
        );
        assert_eq!(
            resolve_import(
                Language::Rust,
                "src/auth/session.rs",
                "super::tokens",
                &files
            ),
            Some("src/auth/tokens.rs".into()),
            "super from auth::session is auth, not the crate root"
        );
        assert_eq!(
            resolve_import(Language::Rust, "src/auth/mod.rs", "super::database", &files),
            Some("src/database.rs".into())
        );
    }

    #[test]
    fn external_crates_stay_unresolved() {
        let files = repository(&["src/database.rs"]);
        assert_eq!(
            resolve_import(
                Language::Rust,
                "src/api.rs",
                "std::collections::HashMap",
                &files
            ),
            None
        );
    }

    #[test]
    fn sibling_workspace_crates_resolve() {
        let files = repository(&[
            "crates/app/src/main.rs",
            "crates/my-core/src/lib.rs",
            "crates/my-core/src/config.rs",
        ]);

        assert_eq!(
            resolve_import(
                Language::Rust,
                "crates/app/src/main.rs",
                "my_core::Config",
                &files
            ),
            Some("crates/my-core/src/lib.rs".into()),
            "a crate name resolves to that crate's root, dashes and all"
        );
        assert_eq!(
            resolve_import(
                Language::Rust,
                "crates/app/src/main.rs",
                "my_core::config::Config",
                &files
            ),
            Some("crates/my-core/src/config.rs".into()),
            "and deeper paths resolve to the module inside it"
        );
    }

    #[test]
    fn a_crate_outside_the_repository_stays_unresolved() {
        let files = repository(&["crates/app/src/main.rs"]);
        assert_eq!(
            resolve_import(
                Language::Rust,
                "crates/app/src/main.rs",
                "serde::Serialize",
                &files
            ),
            None
        );
    }

    #[test]
    fn go_imports_are_never_resolved_to_files() {
        let files = repository(&["main.go"]);
        assert_eq!(
            resolve_import(Language::Go, "main.go", "github.com/example/db", &files),
            None
        );
    }

    #[test]
    fn joining_handles_traversal() {
        assert_eq!(join("src/auth", "../database"), "src/database");
        assert_eq!(join("src", "./util"), "src/util");
        assert_eq!(join("", "top"), "top");
        assert_eq!(join("src/a/b", "../../c"), "src/c");
    }
}
