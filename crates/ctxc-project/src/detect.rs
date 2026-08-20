//! Working out what a project is made of.
//!
//! Detection reads the files a project already has — manifests, lockfiles, a
//! `.git` directory — and never guesses beyond them. The results steer ignore
//! defaults, parser choice and ranking, and are always overridable, so a wrong
//! guess costs quality rather than correctness.

use std::path::Path;

use crate::model::Detection;

/// Inspect a project directory.
pub fn detect(root: &Path) -> Detection {
    let mut detection = Detection {
        git: root.join(".git").exists(),
        ..Detection::default()
    };

    if root.join("Cargo.toml").is_file() {
        detection.languages.push("rust".into());
        detection.package_manager.get_or_insert("cargo".into());
    }
    if root.join("go.mod").is_file() {
        detection.languages.push("go".into());
        detection.package_manager.get_or_insert("go".into());
    }
    if root.join("pyproject.toml").is_file()
        || root.join("requirements.txt").is_file()
        || root.join("setup.py").is_file()
    {
        detection.languages.push("python".into());
        detection
            .package_manager
            .get_or_insert(python_manager(root));
    }
    if root.join("package.json").is_file() {
        detect_node(root, &mut detection);
    }
    if root.join("tsconfig.json").is_file()
        && !detection.languages.iter().any(|l| l == "typescript")
    {
        detection.languages.push("typescript".into());
    }

    detection.languages.sort();
    detection.languages.dedup();
    detection.frameworks.sort();
    detection.frameworks.dedup();
    detection
}

/// Which Python workflow the project uses, judged by its lockfiles.
fn python_manager(root: &Path) -> String {
    for (file, manager) in [
        ("poetry.lock", "poetry"),
        ("uv.lock", "uv"),
        ("Pipfile.lock", "pipenv"),
    ] {
        if root.join(file).is_file() {
            return manager.into();
        }
    }
    "pip".into()
}

/// Read `package.json` for the language, the framework and the package manager.
fn detect_node(root: &Path, detection: &mut Detection) {
    detection.languages.push("javascript".into());
    detection
        .package_manager
        .get_or_insert_with(|| node_manager(root));

    let Ok(text) = std::fs::read_to_string(root.join("package.json")) else {
        return;
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&text) else {
        // A malformed manifest is a fact about the project, not a failure of
        // detection: what was learned from the file's existence still stands.
        tracing::debug!("package.json is not valid JSON; skipping its contents");
        return;
    };

    let dependencies = ["dependencies", "devDependencies"]
        .iter()
        .filter_map(|section| manifest.get(section))
        .filter_map(|section| section.as_object())
        .flat_map(|section| section.keys().map(String::as_str))
        .collect::<Vec<_>>();

    if dependencies.contains(&"typescript") {
        detection.languages.push("typescript".into());
    }

    const FRAMEWORKS: [(&str, &str); 10] = [
        ("next", "next.js"),
        ("nuxt", "nuxt"),
        ("react", "react"),
        ("vue", "vue"),
        ("svelte", "svelte"),
        ("@angular/core", "angular"),
        ("express", "express"),
        ("fastify", "fastify"),
        ("@nestjs/core", "nestjs"),
        ("astro", "astro"),
    ];
    for (package, framework) in FRAMEWORKS {
        if dependencies.contains(&package) {
            detection.frameworks.push(framework.into());
        }
    }

    // `packageManager: "pnpm@9"` is authoritative when it is present.
    if let Some(declared) = manifest.get("packageManager").and_then(|v| v.as_str()) {
        let name = declared.split('@').next().unwrap_or(declared);
        if !name.is_empty() {
            detection.package_manager = Some(name.to_string());
        }
    }
}

/// Which Node package manager the project uses, judged by its lockfile.
fn node_manager(root: &Path) -> String {
    for (file, manager) in [
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lockb", "bun"),
        ("package-lock.json", "npm"),
    ] {
        if root.join(file).is_file() {
            return manager.into();
        }
    }
    "npm".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str) -> Fixture {
            let path = std::env::temp_dir()
                .join("ctxc-detect-tests")
                .join(format!("{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Fixture(path)
        }

        fn file(self, name: &str, contents: &str) -> Self {
            std::fs::write(self.0.join(name), contents).unwrap();
            self
        }

        fn dir(self, name: &str) -> Self {
            std::fs::create_dir_all(self.0.join(name)).unwrap();
            self
        }

        fn detect(&self) -> Detection {
            detect(&self.0)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_rust_project_is_recognised() {
        let fixture = Fixture::new("rust")
            .file("Cargo.toml", "[package]\nname = \"demo\"\n")
            .dir(".git");
        let detection = fixture.detect();

        assert_eq!(detection.languages, vec!["rust"]);
        assert_eq!(detection.package_manager.as_deref(), Some("cargo"));
        assert!(detection.git);
    }

    #[test]
    fn a_next_js_project_is_recognised_from_its_manifest() {
        let fixture = Fixture::new("next")
            .file(
                "package.json",
                r#"{"dependencies":{"next":"14","react":"18"},"devDependencies":{"typescript":"5"}}"#,
            )
            .file("pnpm-lock.yaml", "")
            .file("tsconfig.json", "{}");
        let detection = fixture.detect();

        assert_eq!(detection.languages, vec!["javascript", "typescript"]);
        assert_eq!(detection.frameworks, vec!["next.js", "react"]);
        assert_eq!(detection.package_manager.as_deref(), Some("pnpm"));
    }

    #[test]
    fn a_declared_package_manager_wins_over_the_lockfile() {
        let fixture = Fixture::new("declared")
            .file("package.json", r#"{"packageManager":"yarn@4.1.0"}"#)
            .file("package-lock.json", "{}");

        assert_eq!(fixture.detect().package_manager.as_deref(), Some("yarn"));
    }

    #[test]
    fn python_workflows_are_told_apart() {
        let poetry = Fixture::new("poetry")
            .file("pyproject.toml", "")
            .file("poetry.lock", "");
        assert_eq!(poetry.detect().package_manager.as_deref(), Some("poetry"));

        let plain = Fixture::new("pip").file("requirements.txt", "");
        assert_eq!(plain.detect().package_manager.as_deref(), Some("pip"));
        assert_eq!(plain.detect().languages, vec!["python"]);
    }

    #[test]
    fn a_polyglot_project_reports_every_language() {
        let fixture = Fixture::new("polyglot")
            .file("Cargo.toml", "")
            .file("go.mod", "module demo")
            .file("package.json", "{}");
        let detection = fixture.detect();

        assert_eq!(detection.languages, vec!["go", "javascript", "rust"]);
    }

    #[test]
    fn a_broken_manifest_does_not_break_detection() {
        let fixture = Fixture::new("broken").file("package.json", "{ not json");
        let detection = fixture.detect();

        assert_eq!(detection.languages, vec!["javascript"]);
        assert!(detection.frameworks.is_empty());
    }

    #[test]
    fn an_empty_directory_detects_nothing() {
        let fixture = Fixture::new("empty");
        assert!(fixture.detect().is_empty());
    }

    #[test]
    fn detection_is_repeatable() {
        let fixture = Fixture::new("repeatable")
            .file("Cargo.toml", "")
            .dir(".git");
        assert_eq!(fixture.detect(), fixture.detect());
    }
}
