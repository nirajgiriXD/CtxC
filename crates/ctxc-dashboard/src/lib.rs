//! The dashboard, as bytes.
//!
//! A user must never have to install a web application separately, so the built
//! dashboard ships inside the `ctxc` binary. This crate is only the delivery
//! mechanism: it holds the compiled assets and answers "what should this URL
//! return?". It knows nothing about HTTP, and nothing about CtxC.
//!
//! Rust must build without Node, so the asset table is generated at compile
//! time from `ui/dist` if it is there and left empty if it is not. A build with
//! no dashboard says so ([`is_bundled`]) rather than serving a blank page, and
//! everything else about CtxC works exactly the same — which is the point of
//! keeping the dashboard behind the HTTP API.

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

/// One embedded file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    /// The path it answers, relative to the dashboard root, with no leading
    /// slash: `index.html`, `assets/index-a1b2c3.js`.
    pub path: &'static str,
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

impl Asset {
    /// Whether this file's name contains a build hash.
    ///
    /// Vite fingerprints everything it can, and a fingerprinted file can be
    /// cached forever because a new build produces a new name. `index.html` is
    /// never fingerprinted — it is what points at the rest — so it must not be.
    pub fn is_immutable(&self) -> bool {
        !self.path.ends_with(".html")
            && self
                .path
                .rsplit('/')
                .next()
                .map(has_build_hash)
                .unwrap_or(false)
    }

    /// The `Cache-Control` value this file should be served with.
    pub fn cache_control(&self) -> &'static str {
        if self.is_immutable() {
            "public, max-age=31536000, immutable"
        } else {
            // The entry point decides which fingerprinted files load, so a
            // stale copy of it would pin a browser to an old build.
            "no-cache"
        }
    }
}

/// Whether a file name looks like `name-a1b2c3d4.js`.
fn has_build_hash(name: &str) -> bool {
    let Some((stem, _)) = name.rsplit_once('.') else {
        return false;
    };
    let Some((_, suffix)) = stem.rsplit_once('-') else {
        return false;
    };

    suffix.len() >= 8
        && suffix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() && !c.is_uppercase())
}

/// Whether this build carries a dashboard at all.
///
/// False in a checkout where `npm run build` has not been run. Callers should
/// say so plainly rather than serving nothing.
pub fn is_bundled() -> bool {
    !ASSETS.is_empty()
}

/// Every embedded file.
pub fn assets() -> &'static [Asset] {
    ASSETS
}

/// The asset a request path should be answered with.
///
/// Leading slashes are ignored and an empty path means the entry point, so
/// `/`, `""` and `index.html` all resolve to the same file.
///
/// Returns `None` for anything not bundled; it does *not* fall back to
/// `index.html`. Deciding that an unknown path is a client-side route is the
/// server's call, and [`entry_point`] is there for it — quietly answering a
/// missing script with HTML would turn a 404 into a syntax error.
pub fn asset(path: &str) -> Option<&'static Asset> {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    ASSETS.iter().find(|asset| asset.path == path)
}

/// The dashboard's entry point, when one is bundled.
///
/// This is what a single-page application's unknown routes should fall back to.
pub fn entry_point() -> Option<&'static Asset> {
    asset("index.html")
}

/// Total size of everything embedded, for `ctxc status`.
pub fn total_bytes() -> usize {
    ASSETS.iter().map(|asset| asset.bytes.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for a real build, so the rules can be tested in a checkout
    /// that has not run `npm run build`.
    fn sample(path: &'static str) -> Asset {
        Asset {
            path,
            content_type: "text/plain",
            bytes: b"",
        }
    }

    #[test]
    fn a_missing_dashboard_is_reported_rather_than_faked() {
        // Both are legitimate: this test asserts the two agree, whichever
        // checkout it runs in.
        assert_eq!(is_bundled(), !assets().is_empty());
        if is_bundled() {
            assert!(
                entry_point().is_some(),
                "a bundled dashboard without an entry point could never load"
            );
        }
    }

    #[test]
    fn the_root_resolves_to_the_entry_point() {
        if !is_bundled() {
            return;
        }
        assert_eq!(asset("/"), entry_point());
        assert_eq!(asset(""), entry_point());
        assert_eq!(asset("index.html"), entry_point());
    }

    #[test]
    fn an_unknown_path_is_not_quietly_answered_with_html() {
        assert!(
            asset("assets/does-not-exist.js").is_none(),
            "a missing script must 404, not return the page that asked for it"
        );
    }

    #[test]
    fn fingerprinted_files_are_cacheable_and_the_entry_point_is_not() {
        let hashed = sample("assets/index-a1b2c3d4.js");
        assert!(hashed.is_immutable());
        assert!(hashed.cache_control().contains("immutable"));

        let entry = sample("index.html");
        assert!(!entry.is_immutable());
        assert_eq!(entry.cache_control(), "no-cache");

        // A hashed name is what makes forever-caching safe; without one, no.
        assert!(!sample("assets/logo.svg").is_immutable());
        assert!(!sample("assets/app-short.js").is_immutable());
    }

    #[test]
    fn embedded_files_have_a_content_type_and_a_route() {
        for asset in assets() {
            assert!(!asset.path.starts_with('/'), "{}", asset.path);
            assert!(!asset.content_type.is_empty(), "{}", asset.path);
        }
    }

    #[test]
    fn build_hashes_are_recognised_by_shape() {
        assert!(has_build_hash("index-a1b2c3d4.js"));
        assert!(has_build_hash("index-4f3a9b8c1d.css"));
        assert!(!has_build_hash("index.js"), "no hash at all");
        assert!(!has_build_hash("index-abc.js"), "too short to be a hash");
        assert!(!has_build_hash("no-extension"));
    }
}
