//! Platform abstraction.
//!
//! All platform differences that CtxC cares about are resolved here, once.
//! Conditional compilation appears in exactly one place — [`Os::current`] —
//! and every rule below is a pure function of (operating system, environment),
//! so the Windows rules can be tested on Linux and vice versa.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::APP_NAME;

/// Environment variable that relocates every CtxC directory at once. Useful for
/// portable installs, sandboxes, and tests.
pub const HOME_ENV: &str = "CTXC_HOME";

/// The families of platform behaviour CtxC distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    MacOs,
    /// Linux, the BSDs, and anything else following the XDG conventions.
    Unix,
}

impl Os {
    /// The operating system this binary was built for.
    pub const fn current() -> Os {
        #[cfg(target_os = "windows")]
        {
            Os::Windows
        }
        #[cfg(target_os = "macos")]
        {
            Os::MacOs
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            Os::Unix
        }
    }

    /// Name used in diagnostics and JSON output.
    pub fn as_str(self) -> &'static str {
        match self {
            Os::Windows => "windows",
            Os::MacOs => "macos",
            Os::Unix => "unix",
        }
    }
}

/// Read-only view of the process environment.
///
/// Injecting this is what makes path resolution testable without mutating
/// global state, which is not safe to do from concurrent tests.
pub trait Environment {
    fn var(&self, key: &str) -> Option<String>;
}

/// The real process environment.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemEnvironment;

impl Environment for SystemEnvironment {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|value| !value.is_empty())
    }
}

/// An in-memory environment, for tests and for simulating other platforms.
#[derive(Debug, Clone, Default)]
pub struct MapEnvironment(BTreeMap<String, String>);

impl MapEnvironment {
    pub fn new<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        MapEnvironment(
            vars.into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        )
    }
}

impl Environment for MapEnvironment {
    fn var(&self, key: &str) -> Option<String> {
        self.0.get(key).filter(|value| !value.is_empty()).cloned()
    }
}

/// The directories CtxC uses on a given machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    config_dir: PathBuf,
    data_dir: PathBuf,
    cache_dir: PathBuf,
}

impl Paths {
    /// Resolve directories for `os` using `env`.
    ///
    /// `CTXC_HOME` wins over everything; otherwise the platform convention
    /// applies: Known Folders on Windows, `~/Library` on macOS, XDG elsewhere.
    pub fn resolve(os: Os, env: &dyn Environment) -> Result<Paths> {
        if let Some(home) = env.var(HOME_ENV).map(PathBuf::from) {
            return Ok(Paths {
                config_dir: home.clone(),
                cache_dir: home.join("cache"),
                data_dir: home,
            });
        }

        match os {
            Os::Windows => {
                let roaming = required(env, "APPDATA", "config", "APPDATA, CTXC_HOME")?;
                let local = env
                    .var("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| roaming.clone());
                Ok(Paths {
                    config_dir: roaming.join(APP_NAME),
                    data_dir: local.join(APP_NAME),
                    cache_dir: local.join(APP_NAME).join("cache"),
                })
            }
            Os::MacOs => {
                let home = required(env, "HOME", "home", "HOME, CTXC_HOME")?;
                let support = home
                    .join("Library")
                    .join("Application Support")
                    .join(APP_NAME);
                Ok(Paths {
                    config_dir: support.clone(),
                    data_dir: support,
                    cache_dir: home.join("Library").join("Caches").join(APP_NAME),
                })
            }
            Os::Unix => {
                let home = required(env, "HOME", "home", "HOME, CTXC_HOME")?;
                Ok(Paths {
                    config_dir: xdg(env, "XDG_CONFIG_HOME", &home, ".config"),
                    data_dir: xdg(env, "XDG_DATA_HOME", &home, ".local/share"),
                    cache_dir: xdg(env, "XDG_CACHE_HOME", &home, ".cache"),
                })
            }
        }
    }

    /// Resolve directories for the current platform and process environment.
    pub fn discover() -> Result<Paths> {
        Paths::resolve(Os::current(), &SystemEnvironment)
    }

    /// Build paths explicitly, bypassing the environment entirely.
    pub fn new(config_dir: PathBuf, data_dir: PathBuf, cache_dir: PathBuf) -> Paths {
        Paths {
            config_dir,
            data_dir,
            cache_dir,
        }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// The global configuration file.
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// The SQLite database backing storage, metrics and the registry.
    pub fn database_file(&self) -> PathBuf {
        self.data_dir.join("ctxc.db")
    }

    /// Create a directory and its parents, reporting the path on failure.
    pub fn ensure_dir(path: &Path) -> Result<()> {
        std::fs::create_dir_all(path).map_err(|source| Error::Io {
            action: "create directory",
            path: path.to_path_buf(),
            source,
        })
    }
}

/// A resolved platform: which OS this is, and where its directories live.
pub trait Platform {
    fn os(&self) -> Os;
    fn paths(&self) -> &Paths;
}

/// The platform the process is actually running on.
#[derive(Debug, Clone)]
pub struct HostPlatform {
    os: Os,
    paths: Paths,
}

impl HostPlatform {
    /// Detect the host platform from the process environment.
    pub fn detect() -> Result<Self> {
        Ok(HostPlatform {
            os: Os::current(),
            paths: Paths::discover()?,
        })
    }

    /// Build a platform from explicit parts (tests, `--data-dir` style flags).
    pub fn with_paths(os: Os, paths: Paths) -> Self {
        HostPlatform { os, paths }
    }
}

impl Platform for HostPlatform {
    fn os(&self) -> Os {
        self.os
    }

    fn paths(&self) -> &Paths {
        &self.paths
    }
}

fn required(
    env: &dyn Environment,
    key: &str,
    kind: &'static str,
    variables: &'static str,
) -> Result<PathBuf> {
    env.var(key)
        .map(PathBuf::from)
        .ok_or(Error::MissingDirectory { kind, variables })
}

/// Apply an XDG base directory variable, ignoring relative values as the spec
/// requires, then append the application name.
///
/// Absoluteness is judged by POSIX rules rather than the host's, because these
/// are POSIX paths even when the resolution is being exercised from Windows.
fn xdg(env: &dyn Environment, key: &str, home: &Path, fallback: &str) -> PathBuf {
    let base = env
        .var(key)
        .filter(|value| value.starts_with('/'))
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(fallback));
    base.join(APP_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unix_env() -> MapEnvironment {
        MapEnvironment::new([("HOME", "/home/dev")])
    }

    #[test]
    fn windows_uses_known_folders() {
        let env = MapEnvironment::new([
            ("APPDATA", r"C:\Users\dev\AppData\Roaming"),
            ("LOCALAPPDATA", r"C:\Users\dev\AppData\Local"),
        ]);
        let paths = Paths::resolve(Os::Windows, &env).unwrap();

        assert_eq!(
            paths.config_dir(),
            Path::new(r"C:\Users\dev\AppData\Roaming").join("ctxc")
        );
        assert_eq!(
            paths.data_dir(),
            Path::new(r"C:\Users\dev\AppData\Local").join("ctxc")
        );
        assert_eq!(
            paths.cache_dir(),
            Path::new(r"C:\Users\dev\AppData\Local")
                .join("ctxc")
                .join("cache")
        );
    }

    #[test]
    fn windows_falls_back_to_roaming_without_localappdata() {
        let env = MapEnvironment::new([("APPDATA", r"C:\Users\dev\AppData\Roaming")]);
        let paths = Paths::resolve(Os::Windows, &env).unwrap();
        assert_eq!(paths.data_dir(), paths.config_dir());
    }

    #[test]
    fn macos_uses_library_directories() {
        let env = MapEnvironment::new([("HOME", "/Users/dev")]);
        let paths = Paths::resolve(Os::MacOs, &env).unwrap();

        assert_eq!(
            paths.config_dir(),
            Path::new("/Users/dev/Library/Application Support/ctxc")
        );
        assert_eq!(paths.data_dir(), paths.config_dir());
        assert_eq!(
            paths.cache_dir(),
            Path::new("/Users/dev/Library/Caches/ctxc")
        );
    }

    #[test]
    fn unix_uses_xdg_defaults() {
        let paths = Paths::resolve(Os::Unix, &unix_env()).unwrap();

        assert_eq!(paths.config_dir(), Path::new("/home/dev/.config/ctxc"));
        assert_eq!(paths.data_dir(), Path::new("/home/dev/.local/share/ctxc"));
        assert_eq!(paths.cache_dir(), Path::new("/home/dev/.cache/ctxc"));
    }

    #[test]
    fn unix_honours_absolute_xdg_overrides_only() {
        let env = MapEnvironment::new([
            ("HOME", "/home/dev"),
            ("XDG_CONFIG_HOME", "/etc/xdg-user"),
            ("XDG_DATA_HOME", "relative/path"),
        ]);
        let paths = Paths::resolve(Os::Unix, &env).unwrap();

        assert_eq!(paths.config_dir(), Path::new("/etc/xdg-user/ctxc"));
        assert_eq!(
            paths.data_dir(),
            Path::new("/home/dev/.local/share/ctxc"),
            "relative XDG values must be ignored"
        );
    }

    #[test]
    fn ctxc_home_overrides_every_platform() {
        let env = MapEnvironment::new([
            (HOME_ENV, "/srv/ctxc"),
            ("HOME", "/home/dev"),
            ("APPDATA", r"C:\Users\dev\AppData\Roaming"),
        ]);
        for os in [Os::Windows, Os::MacOs, Os::Unix] {
            let paths = Paths::resolve(os, &env).unwrap();
            assert_eq!(paths.config_dir(), Path::new("/srv/ctxc"));
            assert_eq!(paths.data_dir(), Path::new("/srv/ctxc"));
            assert_eq!(paths.cache_dir(), Path::new("/srv/ctxc/cache"));
            assert_eq!(paths.config_file(), Path::new("/srv/ctxc/config.toml"));
            assert_eq!(paths.database_file(), Path::new("/srv/ctxc/ctxc.db"));
        }
    }

    #[test]
    fn missing_variables_produce_an_actionable_error() {
        let empty = MapEnvironment::default();
        let error = Paths::resolve(Os::Unix, &empty).unwrap_err();
        assert!(error.hint().unwrap().contains("CTXC_HOME"));

        assert!(Paths::resolve(Os::Windows, &empty).is_err());
    }

    #[test]
    fn empty_variables_are_treated_as_unset() {
        let env = MapEnvironment::new([("HOME", "/home/dev"), ("XDG_CONFIG_HOME", "")]);
        let paths = Paths::resolve(Os::Unix, &env).unwrap();
        assert_eq!(paths.config_dir(), Path::new("/home/dev/.config/ctxc"));
    }
}
