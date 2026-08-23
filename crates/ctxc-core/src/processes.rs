//! A record of the long-lived CtxC processes for one data directory, and the
//! platform lookups that say whether one is still there.
//!
//! The daemon publishes itself in a lockfile because a client needs its port
//! and its token. The other processes CtxC runs — an MCP server an agent
//! spawned, a daemon someone left in a terminal — publish nothing, so nothing
//! can find them again afterwards. That is what this is: one small file per
//! process, named after its pid, written when the process starts and removed
//! when it ends.
//!
//! Scoped to the data directory rather than to the machine, deliberately. Two
//! installations pointed at two `CTXC_HOME`s are two independent systems, and
//! stopping one must not reach into the other.
//!
//! This module only ever *looks*. Ending a process is the CLI's job, because
//! `ctxc stop` is the only thing that should do it — the daemon reads these
//! records so a dashboard can say what is still running, and stops there.
//!
//! A record is a hint about a process, never proof of one. Operating systems
//! reuse pids, so every pid read back from here is checked against the
//! executable it claims to be running before anyone acts on it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

/// Directory inside the data directory holding one file per process.
const DIRECTORY: &str = "processes";

/// What a running CtxC process publishes about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub pid: u32,
    /// The command being run, as a user would type it: `mcp`, `start`.
    pub command: String,
    /// File name of the executable, checked before anything is acted on.
    pub executable: String,
    pub started_at: i64,
}

impl Entry {
    /// Whether this record still describes a process that is really there.
    pub fn is_alive(&self) -> bool {
        is_running(self.pid, &self.executable)
    }
}

/// Where the records live for a given data directory.
pub fn directory(data_dir: &Path) -> PathBuf {
    data_dir.join(DIRECTORY)
}

/// The file name of the running executable, or a sensible guess.
pub fn executable_name() -> String {
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| if cfg!(windows) { "ctxc.exe" } else { "ctxc" }.to_string())
}

/// Every process recorded for this data directory.
///
/// A file that cannot be read or parsed is skipped rather than reported: it
/// describes a process that may not exist any more, which is not worth failing
/// a caller over.
pub fn entries(data_dir: &Path) -> Vec<Entry> {
    let Ok(listing) = std::fs::read_dir(directory(data_dir)) else {
        return Vec::new();
    };

    let mut found: Vec<Entry> = listing
        .flatten()
        .filter_map(|item| std::fs::read_to_string(item.path()).ok())
        .filter_map(|text| serde_json::from_str::<Entry>(&text).ok())
        .collect();

    // Oldest first, so a caller reads in the order things were started.
    found.sort_by_key(|entry| (entry.started_at, entry.pid));
    found
}

/// Drop a record, whether or not it was there.
pub fn forget(data_dir: &Path, pid: u32) {
    let _ = std::fs::remove_file(path(data_dir, pid));
}

/// Record this process for as long as the returned guard is alive.
///
/// The error is returned rather than logged so the caller can decide how loud
/// to be. Failing to record is never fatal: the process still runs, it is only
/// harder to stop later.
pub fn register(data_dir: &Path, command: &str) -> std::io::Result<Registration> {
    let entry = Entry {
        pid: std::process::id(),
        command: command.to_string(),
        executable: executable_name(),
        started_at: crate::Timestamp::now().as_millis(),
    };

    let file = path(data_dir, entry.pid);
    write(&file, &entry)?;
    Ok(Registration { file })
}

/// Removes this process's record when the command ends.
///
/// A process that is killed outright leaves its record behind. That is what
/// [`Entry::is_alive`] is for.
pub struct Registration {
    file: PathBuf,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
    }
}

fn path(data_dir: &Path, pid: u32) -> PathBuf {
    directory(data_dir).join(format!("{pid}.json"))
}

fn write(file: &Path, entry: &Entry) -> std::io::Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        file,
        serde_json::to_string_pretty(entry).expect("an entry is always serializable"),
    )
}

// ---------------------------------------------------------------- the machine

/// Run a program and return its standard output, or `None` if it failed.
///
/// Every one of these is run directly, never through a shell, and the only
/// thing interpolated into one is a number.
fn output(program: &str, args: &[&str]) -> Option<String> {
    let produced = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    produced
        .status
        .success()
        .then(|| String::from_utf8_lossy(&produced.stdout).into_owned())
}

/// Whether `pid` is running the named executable.
#[cfg(windows)]
pub fn is_running(pid: u32, executable: &str) -> bool {
    let Some(listing) = output(
        "tasklist",
        &["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"],
    ) else {
        return false;
    };

    listing
        .lines()
        .filter_map(field)
        .any(|name| name.eq_ignore_ascii_case(executable))
}

/// Every process on this machine running the named executable.
#[cfg(windows)]
pub fn running(executable: &str) -> Vec<u32> {
    let filter = format!("IMAGENAME eq {executable}");
    let Some(listing) = output("tasklist", &["/FI", &filter, "/NH", "/FO", "CSV"]) else {
        return Vec::new();
    };

    listing
        .lines()
        .filter(|line| field(line).is_some_and(|name| name.eq_ignore_ascii_case(executable)))
        .filter_map(|line| field_at(line, 1))
        .filter_map(|pid| pid.parse().ok())
        .collect()
}

/// The first field of a `tasklist` CSV row, unquoted.
///
/// A run that matched nothing prints a line beginning `INFO:`, which has no
/// opening quote and so falls out here as absent rather than as a name.
#[cfg(windows)]
fn field(line: &str) -> Option<&str> {
    field_at(line, 0)
}

/// The nth quoted field of a `tasklist` CSV row.
#[cfg(windows)]
fn field_at(line: &str, index: usize) -> Option<&str> {
    line.trim()
        .strip_prefix('"')?
        .split("\",\"")
        .nth(index)
        .map(|field| field.trim_end_matches('"'))
}

/// Whether `pid` is running the named executable.
#[cfg(not(windows))]
pub fn is_running(pid: u32, executable: &str) -> bool {
    output("ps", &["-p", &pid.to_string(), "-o", "comm="])
        .and_then(|comm| comm.lines().next().map(str::trim).map(str::to_string))
        .is_some_and(|comm| file_name(&comm) == executable)
}

/// Every process on this machine running the named executable.
#[cfg(not(windows))]
pub fn running(executable: &str) -> Vec<u32> {
    let Some(listing) = output("ps", &["-A", "-o", "pid=,comm="]) else {
        return Vec::new();
    };

    listing
        .lines()
        .filter_map(|line| {
            let mut columns = line.trim().splitn(2, char::is_whitespace);
            let pid = columns.next()?.parse().ok()?;
            (file_name(columns.next()?.trim()) == executable).then_some(pid)
        })
        .collect()
}

/// The last path component, for a `comm` that came back as a full path.
#[cfg(not(windows))]
fn file_name(comm: &str) -> &str {
    comm.rsplit('/').next().unwrap_or(comm)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str) -> Fixture {
            let path = std::env::temp_dir()
                .join("ctxc-processes-tests")
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

    #[test]
    fn nothing_recorded_means_nothing_to_stop() {
        let fixture = Fixture::new("empty");
        assert!(entries(&fixture.0).is_empty());
    }

    #[test]
    fn a_registration_is_visible_until_it_is_dropped() {
        let fixture = Fixture::new("lifetime");

        let registration = register(&fixture.0, "mcp").unwrap();
        let found = entries(&fixture.0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pid, std::process::id());
        assert_eq!(found[0].command, "mcp");
        assert_eq!(found[0].executable, executable_name());

        drop(registration);
        assert!(entries(&fixture.0).is_empty());
    }

    #[test]
    fn an_unreadable_record_is_skipped_rather_than_fatal() {
        let fixture = Fixture::new("corrupt");
        std::fs::create_dir_all(directory(&fixture.0)).unwrap();
        std::fs::write(directory(&fixture.0).join("7.json"), "{ not json").unwrap();

        assert!(entries(&fixture.0).is_empty());
    }

    #[test]
    fn forgetting_is_idempotent() {
        let fixture = Fixture::new("forget");
        forget(&fixture.0, 999_999);

        let entry = Entry {
            pid: 12,
            command: "mcp".into(),
            executable: executable_name(),
            started_at: 1,
        };
        write(&path(&fixture.0, entry.pid), &entry).unwrap();

        forget(&fixture.0, 12);
        forget(&fixture.0, 12);
        assert!(entries(&fixture.0).is_empty());
    }

    #[test]
    fn a_record_naming_another_executable_is_not_alive() {
        let entry = Entry {
            pid: std::process::id(),
            command: "mcp".into(),
            // This process is real, but it is not running this.
            executable: "ctxc-not-a-real-binary".into(),
            started_at: 1,
        };
        assert!(!entry.is_alive(), "a reused pid must not count as ours");
    }

    #[cfg(windows)]
    #[test]
    fn tasklist_rows_are_parsed_and_its_no_match_notice_is_not() {
        let row = "\"ctxc.exe\",\"4321\",\"Console\",\"1\",\"12,345 K\"";
        assert_eq!(field(row).unwrap(), "ctxc.exe");
        assert_eq!(field_at(row, 1).unwrap(), "4321");
        assert_eq!(
            field("INFO: No tasks are running which match the specified criteria."),
            None
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn a_comm_is_compared_by_its_last_component() {
        assert_eq!(file_name("/usr/local/bin/ctxc"), "ctxc");
        assert_eq!(file_name("ctxc"), "ctxc");
    }

    #[test]
    fn enumeration_answers_rather_than_hangs() {
        // The test binary is not named `ctxc`, so what matters here is that
        // asking the operating system returns at all.
        let _ = running(&executable_name());
    }
}
