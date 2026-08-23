//! The command surface, as data.
//!
//! The dashboard has a page that explains what CtxC can do, and it must not be
//! a second, hand-written copy of `ctxc --help` that drifts the first time a
//! flag is added. So the command tree is described once — by the CLI, from the
//! same `clap` definition that parses arguments — and handed to the API at
//! startup.
//!
//! The types live here because they are wire types, and the API is where the
//! wire is. Nothing in this module knows what any command *does*.
//!
//! The catalog is process-wide for the same reason [`crate::logs`] is: the
//! daemon runs inside the `ctxc` binary, so the command tree it should describe
//! is the one that binary has, and there is exactly one of those.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Everything `ctxc` can be asked to do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// Options accepted by every command.
    pub global_options: Vec<OptionInfo>,
    pub commands: Vec<CommandInfo>,
}

impl Catalog {
    /// Find one command by its full path, e.g. `project add`.
    pub fn find(&self, path: &str) -> Option<&CommandInfo> {
        self.commands.iter().find_map(|command| command.find(path))
    }

    /// Every command and subcommand, flattened.
    pub fn flatten(&self) -> Vec<&CommandInfo> {
        let mut found = Vec::new();
        for command in &self.commands {
            command.walk(&mut found);
        }
        found
    }
}

/// One command or subcommand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandInfo {
    /// The full path a user would type, without `ctxc`: `project add`.
    pub path: String,
    /// The last segment of the path: `add`.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The longer explanation, when the command has one worth showing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// A one-line usage string, as `--help` prints it.
    pub usage: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<ArgumentInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<OptionInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subcommands: Vec<CommandInfo>,
    /// Where the dashboard does the same thing, when it can.
    ///
    /// Absent means the command is terminal-only — piping, scripting, or
    /// anything whose result is a stream rather than a screen. Saying so is the
    /// point: a commands page that implied everything had a button would be
    /// worse than one that admits what needs a shell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dashboard: Option<DashboardEquivalent>,
}

impl CommandInfo {
    fn find(&self, path: &str) -> Option<&CommandInfo> {
        if self.path == path {
            return Some(self);
        }
        self.subcommands
            .iter()
            .find_map(|command| command.find(path))
    }

    fn walk<'a>(&'a self, found: &mut Vec<&'a CommandInfo>) {
        found.push(self);
        for command in &self.subcommands {
            command.walk(found);
        }
    }
}

/// A positional argument.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgumentInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    pub required: bool,
    /// True for an argument that may be given more than once.
    pub repeated: bool,
}

/// A flag or an option with a value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionInfo {
    /// The long form, without dashes: `budget`.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short: Option<char>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// The placeholder shown for its value, when it takes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    pub required: bool,
}

/// Where in the dashboard a command's work is done.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardEquivalent {
    /// The dashboard route, e.g. `/projects`.
    pub route: String,
    /// What the control is called there, e.g. "Add project".
    pub label: String,
}

impl DashboardEquivalent {
    pub fn new(route: &str, label: &str) -> DashboardEquivalent {
        DashboardEquivalent {
            route: route.to_owned(),
            label: label.to_owned(),
        }
    }
}

/// Publish the command tree this binary has.
///
/// Called once, by whatever is about to run the daemon. A second call is
/// ignored rather than fatal: the catalog is a description, and the first one
/// is as true as the second.
pub fn install(catalog: Catalog) {
    let _ = slot().set(catalog);
}

/// The command tree, if one was installed.
///
/// `None` in a process that never announced one — an embedding of the API that
/// is not the `ctxc` binary. The route says so rather than inventing a tree.
pub fn catalog() -> Option<&'static Catalog> {
    slot().get()
}

fn slot() -> &'static OnceLock<Catalog> {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    &CATALOG
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(path: &str, subcommands: Vec<CommandInfo>) -> CommandInfo {
        CommandInfo {
            name: path.rsplit(' ').next().unwrap_or(path).to_owned(),
            path: path.to_owned(),
            summary: None,
            description: None,
            usage: format!("ctxc {path}"),
            arguments: Vec::new(),
            options: Vec::new(),
            examples: Vec::new(),
            subcommands,
            dashboard: None,
        }
    }

    fn catalog() -> Catalog {
        Catalog {
            name: "ctxc".into(),
            version: "0.1.0".into(),
            about: None,
            global_options: Vec::new(),
            commands: vec![
                command("status", Vec::new()),
                command(
                    "project",
                    vec![
                        command("project add", Vec::new()),
                        command("project list", Vec::new()),
                    ],
                ),
            ],
        }
    }

    #[test]
    fn a_subcommand_is_found_by_its_full_path() {
        let catalog = catalog();
        assert_eq!(catalog.find("project add").unwrap().name, "add");
        assert_eq!(catalog.find("status").unwrap().path, "status");
        assert!(catalog.find("add").is_none(), "paths are not suffixes");
    }

    #[test]
    fn flattening_reaches_every_subcommand() {
        let catalog = catalog();
        let paths: Vec<&str> = catalog
            .flatten()
            .iter()
            .map(|command| command.path.as_str())
            .collect();

        assert_eq!(paths, ["status", "project", "project add", "project list"]);
    }
}
