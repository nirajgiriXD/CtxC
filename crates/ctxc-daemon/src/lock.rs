//! The daemon lockfile.
//!
//! One file in the data directory records the running daemon: its process id,
//! the port it listens on, and the token a client needs. It answers three
//! questions — is one already running, where do I reach it, and how do I prove
//! I am allowed to.
//!
//! Liveness is decided by asking the recorded port for `/v1/health`, not by
//! looking up the process id. A pid can be reused by an unrelated program after
//! a crash; a CtxC daemon answering on the recorded port cannot be anything
//! else. That makes an unclean shutdown recoverable without any platform
//! specific process API.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use ctxc_api::client::Client;
use ctxc_api::AccessToken;
use ctxc_core::Timestamp;

use crate::error::{DaemonError, Result};

/// Name of the lockfile inside the data directory.
pub const LOCK_FILE: &str = "daemon.lock";

/// What a running daemon publishes about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lock {
    pub pid: u32,
    pub port: u16,
    pub bind: String,
    /// Token required by every route except `/v1/health`.
    pub token: String,
    pub started_at: i64,
    pub version: String,
}

impl Lock {
    /// Where the lockfile lives for a given data directory.
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join(LOCK_FILE)
    }

    /// Describe the daemon this process is about to become.
    pub fn describe(port: u16, bind: &str, token: &AccessToken) -> Lock {
        Lock {
            pid: std::process::id(),
            port,
            bind: bind.to_string(),
            token: token.as_str().to_string(),
            started_at: Timestamp::now().as_millis(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    /// Read the lockfile, if there is one.
    ///
    /// A lockfile that cannot be parsed is treated as absent: it is a hint
    /// about a process that may not exist any more, not a source of truth worth
    /// failing over.
    pub fn read(data_dir: &Path) -> Result<Option<Lock>> {
        let path = Lock::path(data_dir);
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(serde_json::from_str(&text).ok()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(DaemonError::Io {
                action: "read the daemon lockfile",
                path,
                source,
            }),
        }
    }

    /// Write the lockfile, replacing any stale one.
    pub fn write(&self, data_dir: &Path) -> Result<()> {
        ctxc_core::Paths::ensure_dir(data_dir).map_err(DaemonError::Core)?;
        let path = Lock::path(data_dir);

        let text = serde_json::to_string_pretty(self).expect("a lock is always serializable");
        std::fs::write(&path, text).map_err(|source| DaemonError::Io {
            action: "write the daemon lockfile",
            path: path.clone(),
            source,
        })?;

        restrict(&path);
        Ok(())
    }

    /// Remove the lockfile. Missing is success: the point is that it is gone.
    pub fn remove(data_dir: &Path) -> Result<()> {
        let path = Lock::path(data_dir);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(DaemonError::Io {
                action: "remove the daemon lockfile",
                path,
                source,
            }),
        }
    }

    /// The token, if it is one this build accepts.
    pub fn token(&self) -> Option<AccessToken> {
        AccessToken::parse(&self.token)
    }

    /// A client for the daemon this lock describes.
    pub fn client(&self) -> Client {
        Client::new(self.port, self.token())
    }

    /// Whether the daemon this lock describes is actually answering.
    pub fn is_alive(&self) -> bool {
        self.client().is_alive()
    }
}

/// Make the lockfile readable only by its owner.
///
/// It holds an access token, so on Unix the permissions are tightened. On
/// Windows the file inherits the data directory's ACL, which is already
/// user-scoped for a per-user application directory.
#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    if let Err(err) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        tracing::warn!(error = %err, "could not restrict permissions on the lockfile");
    }
}

#[cfg(not(unix))]
fn restrict(_path: &Path) {}

/// What a would-be daemon found when it checked for another one.
#[derive(Debug)]
pub enum Occupancy {
    /// No daemon is running for this data directory.
    Free,
    /// A daemon is already running.
    Running(Box<Lock>),
    /// A lockfile was left behind by a daemon that is gone.
    Stale(Box<Lock>),
}

/// Decide whether this data directory already has a daemon.
pub fn occupancy(data_dir: &Path) -> Result<Occupancy> {
    let Some(lock) = Lock::read(data_dir)? else {
        return Ok(Occupancy::Free);
    };

    if lock.is_alive() {
        Ok(Occupancy::Running(Box::new(lock)))
    } else {
        Ok(Occupancy::Stale(Box::new(lock)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str) -> Fixture {
            let path = std::env::temp_dir()
                .join("ctxc-lock-tests")
                .join(format!("{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Fixture(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn lock() -> Lock {
        Lock::describe(7717, "127.0.0.1", &AccessToken::generate())
    }

    #[test]
    fn a_lock_round_trips_through_the_file() {
        let fixture = Fixture::new("round-trip");
        let written = lock();
        written.write(&fixture.0).unwrap();

        let read = Lock::read(&fixture.0).unwrap().unwrap();
        assert_eq!(read, written);
        assert_eq!(read.token().unwrap().as_str(), written.token);
    }

    #[test]
    fn no_lockfile_means_no_daemon() {
        let fixture = Fixture::new("absent");
        assert!(Lock::read(&fixture.0).unwrap().is_none());
        assert!(matches!(occupancy(&fixture.0).unwrap(), Occupancy::Free));
    }

    #[test]
    fn a_lock_pointing_at_nothing_is_stale_rather_than_fatal() {
        let fixture = Fixture::new("stale");
        // Port 1 on loopback has no daemon behind it.
        let mut stale = lock();
        stale.port = 1;
        stale.write(&fixture.0).unwrap();

        assert!(!stale.is_alive());
        match occupancy(&fixture.0).unwrap() {
            Occupancy::Stale(found) => assert_eq!(found.port, 1),
            other => panic!("expected a stale lock, got {other:?}"),
        }
    }

    #[test]
    fn an_unreadable_lockfile_is_treated_as_absent() {
        let fixture = Fixture::new("corrupt");
        std::fs::write(Lock::path(&fixture.0), "{ this is not json").unwrap();

        assert!(Lock::read(&fixture.0).unwrap().is_none());
    }

    #[test]
    fn removing_is_idempotent() {
        let fixture = Fixture::new("remove");
        lock().write(&fixture.0).unwrap();

        Lock::remove(&fixture.0).unwrap();
        Lock::remove(&fixture.0).unwrap();
        assert!(Lock::read(&fixture.0).unwrap().is_none());
    }

    #[test]
    fn writing_creates_the_data_directory() {
        let fixture = Fixture::new("nested");
        let nested = fixture.0.join("deeper").join("still");

        lock().write(&nested).unwrap();
        assert!(Lock::path(&nested).exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_lockfile_is_not_world_readable() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = Fixture::new("permissions");
        lock().write(&fixture.0).unwrap();

        let mode = std::fs::metadata(Lock::path(&fixture.0))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "the token must not be readable by others");
    }
}
