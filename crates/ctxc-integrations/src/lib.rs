//! Teaching coding agents that CtxC exists.
//!
//! Every agent has its own way of being told about a project's tools, and none
//! of them is an API. They read a file. So an integration here is small on
//! purpose: it finds the file that agent reads, writes one clearly marked block
//! into it, and can take that block out again.
//!
//! ```text
//! ctxc config agents install claude-code
//!        |
//!        v
//!   CLAUDE.md  <- one managed block, everything else untouched
//!        |
//!        v
//!   the agent reads it, and knows to run `ctxc find`
//! ```
//!
//! Two rules shape this crate. Integrations must never be coupled to the
//! optimization engine — nothing here depends on `ctxc-engine`, and removing
//! this crate removes nothing from CtxC but the adapters. And an integration
//! must never damage a file it did not create: everything outside CtxC's
//! markers is the user's, and is preserved byte for byte.

pub mod block;
pub mod error;
pub mod guidance;
pub mod json;

pub use block::{Change, CommentStyle, ManagedBlock};
pub use error::{IntegrationError, Result};
pub use json::ManagedEntry;

use std::path::{Path, PathBuf};

use serde::Serialize;

/// The interface every adapter satisfies.
///
/// Deliberately narrow. An integration answers four questions — is this agent
/// here, put CtxC in it, take CtxC out, and what is the state now — and knows
/// nothing about optimization, indexing, or the daemon.
pub trait AgentIntegration {
    /// The name used on the command line.
    fn name(&self) -> &str;

    /// Whether this agent looks like it is in use for this project.
    ///
    /// A hint, never a gate: `install` works on an undetected agent, because
    /// somebody setting one up before its first run is entirely reasonable.
    fn detect(&self) -> bool;

    /// Write CtxC's guidance. Safe to repeat.
    fn install(&self) -> Result<Change>;

    /// Remove CtxC's guidance, and nothing else.
    fn uninstall(&self) -> Result<Removal>;

    /// What is installed right now.
    fn status(&self) -> Result<IntegrationStatus>;
}

/// Whether a file CtxC writes is shared with the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ownership {
    /// The agent reads a file the user also writes in — `CLAUDE.md`,
    /// `AGENTS.md`. CtxC gets a marked block and touches nothing else.
    Shared,
    /// The agent reads a directory of rule files, so CtxC gets one of its own.
    /// Uninstalling deletes it.
    Owned,
}

/// What `uninstall` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Removal {
    /// The block, or the file, is gone.
    Removed,
    /// There was nothing there to remove.
    NothingToDo,
}

impl Removal {
    pub fn as_str(self) -> &'static str {
        match self {
            Removal::Removed => "removed",
            Removal::NothingToDo => "nothing to do",
        }
    }
}

/// What one integration looks like right now.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IntegrationStatus {
    pub name: &'static str,
    /// The agent this is for, as a person would name it.
    pub agent: &'static str,
    /// Whether the agent appears to be in use here.
    pub detected: bool,
    /// Whether CtxC's guidance is in place.
    pub installed: bool,
    /// Installed, but written by a different version of CtxC.
    pub outdated: bool,
    /// The file this integration writes.
    pub path: PathBuf,
    pub ownership: Ownership,
    /// How this integration reaches the agent.
    pub mechanism: Mechanism,
}

impl IntegrationStatus {
    /// A one-word summary for a person scanning a list.
    pub fn summary(&self) -> &'static str {
        match (self.installed, self.outdated, self.detected) {
            (true, true, _) => "outdated",
            (true, false, _) => "installed",
            (false, _, true) => "available",
            (false, _, false) => "not detected",
        }
    }
}

/// How an agent is told about CtxC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mechanism {
    /// A block of prose in the file the agent reads for project instructions.
    Instructions,
    /// An entry in the agent's MCP server list, which is what actually gives it
    /// CtxC's tools rather than only telling it they exist.
    McpServer,
}

/// One agent CtxC knows how to talk to.
#[derive(Debug, Clone, Copy)]
struct Agent {
    name: &'static str,
    title: &'static str,
    /// The file this agent reads, relative to the project root.
    instructions: &'static str,
    /// Paths whose presence means this agent is probably in use.
    markers: &'static [&'static str],
    comment: CommentStyle,
    ownership: Ownership,
    /// Text written above CtxC's block in a file CtxC creates itself.
    preamble: Option<&'static str>,
    mechanism: Mechanism,
    /// The JSON key CtxC owns, for MCP registrations.
    entry: Option<ManagedEntry>,
}

/// The agents CtxC ships adapters for.
///
/// Each is independently installable, and each writes exactly one file. Where
/// several agents read the same file — `AGENTS.md` is a cross-agent convention
/// that Codex, OpenCode, Aider and others follow — there is one integration for
/// the file rather than one per agent, because two adapters fighting over one
/// block would make uninstalling either of them a lie.
const AGENTS: &[Agent] = &[
    Agent {
        name: "claude-code",
        title: "Claude Code",
        instructions: "CLAUDE.md",
        markers: &[".claude", "CLAUDE.md"],
        comment: CommentStyle::Html,
        ownership: Ownership::Shared,
        preamble: None,
        mechanism: Mechanism::Instructions,
        entry: None,
    },
    Agent {
        name: "agents-md",
        title: "Codex, OpenCode, Aider, and other AGENTS.md readers",
        instructions: "AGENTS.md",
        markers: &["AGENTS.md", ".codex", ".opencode", ".aider.conf.yml"],
        comment: CommentStyle::Html,
        ownership: Ownership::Shared,
        preamble: None,
        mechanism: Mechanism::Instructions,
        entry: None,
    },
    Agent {
        name: "copilot",
        title: "GitHub Copilot",
        instructions: ".github/copilot-instructions.md",
        markers: &[".github/copilot-instructions.md", ".github"],
        comment: CommentStyle::Html,
        ownership: Ownership::Shared,
        preamble: None,
        mechanism: Mechanism::Instructions,
        entry: None,
    },
    Agent {
        name: "gemini",
        title: "Gemini CLI",
        instructions: "GEMINI.md",
        markers: &["GEMINI.md", ".gemini"],
        comment: CommentStyle::Html,
        ownership: Ownership::Shared,
        preamble: None,
        mechanism: Mechanism::Instructions,
        entry: None,
    },
    Agent {
        name: "cursor",
        title: "Cursor",
        instructions: ".cursor/rules/ctxc.mdc",
        markers: &[".cursor"],
        comment: CommentStyle::Html,
        ownership: Ownership::Owned,
        // Cursor reads the frontmatter to decide when a rule applies.
        // `alwaysApply` puts this in front of the model for every request,
        // which is what a "here is how to find context" rule is for.
        preamble: Some(
            "---\ndescription: How to find context in this project with CtxC\nalwaysApply: true\n---\n",
        ),
        mechanism: Mechanism::Instructions,
        entry: None,
    },
    // MCP registrations. These are what actually hand an agent CtxC's tools;
    // the instruction files above only tell it the CLI exists. Both are worth
    // having: not every agent speaks MCP, and an agent that does still
    // benefits from being told when to reach for it.
    Agent {
        name: "claude-code-mcp",
        title: "Claude Code (MCP server)",
        instructions: ".mcp.json",
        markers: &[".claude", ".mcp.json", "CLAUDE.md"],
        comment: CommentStyle::Html,
        ownership: Ownership::Shared,
        preamble: None,
        mechanism: Mechanism::McpServer,
        entry: Some(ManagedEntry::new("mcpServers", "ctxc")),
    },
    Agent {
        name: "cursor-mcp",
        title: "Cursor (MCP server)",
        instructions: ".cursor/mcp.json",
        markers: &[".cursor"],
        comment: CommentStyle::Html,
        ownership: Ownership::Shared,
        preamble: None,
        mechanism: Mechanism::McpServer,
        entry: Some(ManagedEntry::new("mcpServers", "ctxc")),
    },
    Agent {
        name: "cline",
        title: "Cline",
        instructions: ".clinerules/ctxc.md",
        markers: &[".clinerules"],
        comment: CommentStyle::Html,
        ownership: Ownership::Owned,
        preamble: None,
        mechanism: Mechanism::Instructions,
        entry: None,
    },
];

/// Every integration name CtxC knows.
pub fn names() -> Vec<&'static str> {
    AGENTS.iter().map(|agent| agent.name).collect()
}

/// Build every integration for a project.
pub fn all(root: &Path, project: &str) -> Vec<Box<dyn AgentIntegration>> {
    AGENTS
        .iter()
        .map(|agent| {
            Box::new(FileIntegration::new(*agent, root, project)) as Box<dyn AgentIntegration>
        })
        .collect()
}

/// Build one integration by name.
pub fn get(name: &str, root: &Path, project: &str) -> Result<Box<dyn AgentIntegration>> {
    AGENTS
        .iter()
        .find(|agent| agent.name == name)
        .map(|agent| {
            Box::new(FileIntegration::new(*agent, root, project)) as Box<dyn AgentIntegration>
        })
        .ok_or_else(|| IntegrationError::Unknown {
            name: name.to_owned(),
        })
}

/// An integration that works by writing an instruction file.
///
/// Every adapter CtxC ships is one of these. That is not a limitation of the
/// design — the trait allows anything — but a reflection of how agents actually
/// take configuration today.
pub struct FileIntegration {
    agent: Agent,
    path: PathBuf,
    project: String,
    root: PathBuf,
}

impl FileIntegration {
    fn new(agent: Agent, root: &Path, project: &str) -> Self {
        FileIntegration {
            path: root.join(agent.instructions),
            agent,
            project: project.to_owned(),
            root: root.to_path_buf(),
        }
    }

    fn block(&self) -> ManagedBlock {
        ManagedBlock::new(self.agent.comment)
    }

    /// The guidance this integration should have written.
    fn wanted(&self) -> String {
        guidance::body(&self.project)
    }

    /// The file this integration writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the instruction file, treating "not there" as empty.
    ///
    /// A file that is not UTF-8 is refused rather than replaced: it is not
    /// something an agent reads as instructions, and overwriting it would
    /// destroy whatever it actually is.
    fn read(&self) -> Result<String> {
        match std::fs::read(&self.path) {
            Ok(bytes) => String::from_utf8(bytes).map_err(|_| IntegrationError::NotText {
                path: self.path.clone(),
            }),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(source) => Err(IntegrationError::Io {
                action: "read",
                path: self.path.clone(),
                source,
            }),
        }
    }

    /// The MCP server entry CtxC registers.
    ///
    /// `ctxc` rather than an absolute path: an agent inherits the user's PATH,
    /// and a hard-coded path breaks the moment CtxC is upgraded or moved.
    fn server_entry(&self) -> serde_json::Value {
        serde_json::json!({
            "command": "ctxc",
            "args": ["mcp"],
        })
    }

    /// Register CtxC as an MCP server.
    fn install_server(&self) -> Result<Change> {
        let entry = self.agent.entry.expect("an MCP agent has an entry");
        let existing = self.read()?;

        let document = entry
            .parse(&existing)
            .ok_or_else(|| IntegrationError::NotJson {
                path: self.path.clone(),
            })?;

        let (updated, change) = entry.write(&document, self.server_entry());
        if change.wrote_anything() {
            self.write(&json::render(&updated))?;
            tracing::debug!(
                integration = self.agent.name,
                path = %self.path.display(),
                %change,
                "MCP server registered"
            );
        }
        Ok(change)
    }

    /// Take CtxC out of an agent's server list.
    fn uninstall_server(&self) -> Result<Removal> {
        let entry = self.agent.entry.expect("an MCP agent has an entry");
        let existing = self.read()?;
        if existing.trim().is_empty() {
            return Ok(Removal::NothingToDo);
        }

        let document = entry
            .parse(&existing)
            .ok_or_else(|| IntegrationError::NotJson {
                path: self.path.clone(),
            })?;

        let Some(cleaned) = entry.remove(&document) else {
            return Ok(Removal::NothingToDo);
        };

        // A file that held nothing but CtxC's entry goes entirely, rather than
        // being left as an empty `{}` nobody put there.
        if entry.is_now_empty(&cleaned) {
            std::fs::remove_file(&self.path).map_err(|source| IntegrationError::Io {
                action: "remove",
                path: self.path.clone(),
                source,
            })?;
            return Ok(Removal::Removed);
        }

        self.write(&json::render(&cleaned))?;
        Ok(Removal::Removed)
    }

    fn write(&self, contents: &str) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| IntegrationError::Io {
                action: "create",
                path: parent.to_path_buf(),
                source,
            })?;
        }

        std::fs::write(&self.path, contents).map_err(|source| IntegrationError::Io {
            action: "write",
            path: self.path.clone(),
            source,
        })
    }
}

impl AgentIntegration for FileIntegration {
    fn name(&self) -> &str {
        self.agent.name
    }

    fn detect(&self) -> bool {
        self.agent
            .markers
            .iter()
            .any(|marker| self.root.join(marker).exists())
    }

    fn install(&self) -> Result<Change> {
        if self.agent.mechanism == Mechanism::McpServer {
            return self.install_server();
        }

        let existing = self.read()?;
        let (updated, change) = self.block().write(&existing, &self.wanted());

        // A file CtxC creates gets its preamble once, above the block, and is
        // never given a second one on reinstall.
        let contents = match (self.agent.preamble, existing.is_empty()) {
            (Some(preamble), true) => format!("{preamble}\n{updated}"),
            _ => updated,
        };

        if change.wrote_anything() || existing != contents {
            self.write(&contents)?;
            tracing::debug!(
                integration = self.agent.name,
                path = %self.path.display(),
                %change,
                "integration installed"
            );
        }
        Ok(change)
    }

    fn uninstall(&self) -> Result<Removal> {
        if self.agent.mechanism == Mechanism::McpServer {
            return self.uninstall_server();
        }

        let existing = self.read()?;
        let Some(cleaned) = self.block().remove(&existing) else {
            return Ok(Removal::NothingToDo);
        };

        // A file CtxC created for itself goes entirely, rather than being left
        // behind as a stub with nothing but frontmatter in it.
        if self.agent.ownership == Ownership::Owned && is_only_preamble(&cleaned) {
            std::fs::remove_file(&self.path).map_err(|source| IntegrationError::Io {
                action: "remove",
                path: self.path.clone(),
                source,
            })?;
            return Ok(Removal::Removed);
        }

        self.write(&cleaned)?;
        Ok(Removal::Removed)
    }

    fn status(&self) -> Result<IntegrationStatus> {
        let existing = self.read()?;

        if let Some(entry) = self.agent.entry {
            let document = entry
                .parse(&existing)
                .ok_or_else(|| IntegrationError::NotJson {
                    path: self.path.clone(),
                })?;
            let stored = entry.read(&document);

            return Ok(IntegrationStatus {
                name: self.agent.name,
                agent: self.agent.title,
                detected: self.detect(),
                installed: stored.is_some(),
                outdated: stored.is_some_and(|value| value != self.server_entry()),
                path: self.path.clone(),
                ownership: self.agent.ownership,
                mechanism: self.agent.mechanism,
            });
        }

        let installed = self.block().body(&existing);

        Ok(IntegrationStatus {
            name: self.agent.name,
            agent: self.agent.title,
            detected: self.detect(),
            installed: installed.is_some(),
            // Guidance changes between releases; an installed block that no
            // longer matches is worth flagging rather than silently leaving an
            // agent with last year's advice.
            outdated: installed.is_some_and(|body| body != self.wanted()),
            path: self.path.clone(),
            ownership: self.agent.ownership,
            mechanism: self.agent.mechanism,
        })
    }
}

/// Whether what is left of a CtxC-owned file is only its frontmatter.
fn is_only_preamble(contents: &str) -> bool {
    let trimmed = contents.trim();
    trimmed.is_empty() || (trimmed.starts_with("---") && trimmed.ends_with("---"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway project directory.
    struct Project {
        root: PathBuf,
    }

    impl Project {
        fn new(name: &str) -> Project {
            let root = std::env::temp_dir()
                .join("ctxc-integration-tests")
                .join(format!("{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Project { root }
        }

        fn integration(&self, name: &str) -> Box<dyn AgentIntegration> {
            get(name, &self.root, "demo").unwrap()
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }

        fn read(&self, relative: &str) -> String {
            std::fs::read_to_string(self.root.join(relative)).unwrap()
        }

        fn exists(&self, relative: &str) -> bool {
            self.root.join(relative).exists()
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn every_agent_has_a_unique_name_and_a_file() {
        let mut seen = std::collections::HashSet::new();
        for agent in AGENTS {
            assert!(seen.insert(agent.name), "duplicate name: {}", agent.name);
            assert!(!agent.instructions.is_empty(), "{}", agent.name);
            assert!(!agent.markers.is_empty(), "{}", agent.name);
        }
    }

    #[test]
    fn only_one_integration_claims_each_file() {
        // Two adapters writing one block would make uninstalling either of
        // them silently break the other.
        let mut seen = std::collections::HashSet::new();
        for agent in AGENTS {
            assert!(
                seen.insert(agent.instructions),
                "{} is claimed twice",
                agent.instructions
            );
        }
    }

    #[test]
    fn an_unknown_name_is_refused_with_a_way_to_find_the_right_one() {
        let Err(error) = get("emacs", Path::new("."), "demo") else {
            panic!("an unknown agent must not resolve to some other one");
        };
        assert!(matches!(error, IntegrationError::Unknown { .. }));
        assert!(error.hint().unwrap().contains("config agents list"));
    }

    #[test]
    fn installing_creates_the_file_and_status_agrees() {
        let project = Project::new("install");
        let integration = project.integration("claude-code");

        assert_eq!(integration.status().unwrap().summary(), "not detected");
        assert_eq!(integration.install().unwrap(), Change::Added);

        let status = integration.status().unwrap();
        assert!(status.installed);
        assert!(!status.outdated);
        assert_eq!(status.summary(), "installed");
        assert!(project.read("CLAUDE.md").contains("ctxc find"));
    }

    #[test]
    fn installing_twice_changes_nothing_the_second_time() {
        let project = Project::new("idempotent");
        let integration = project.integration("claude-code");

        integration.install().unwrap();
        let before = project.read("CLAUDE.md");

        assert_eq!(integration.install().unwrap(), Change::Unchanged);
        assert_eq!(project.read("CLAUDE.md"), before);
    }

    #[test]
    fn the_users_own_instructions_survive_install_and_uninstall() {
        let project = Project::new("preserve");
        let original = "# House rules\n\nAlways run the tests.\n";
        project.write("CLAUDE.md", original);

        let integration = project.integration("claude-code");
        integration.install().unwrap();

        let after_install = project.read("CLAUDE.md");
        assert!(after_install.contains("Always run the tests."));
        assert!(after_install.contains("ctxc find"));

        assert_eq!(integration.uninstall().unwrap(), Removal::Removed);
        assert_eq!(
            project.read("CLAUDE.md"),
            original,
            "uninstalling must leave the file exactly as it was found"
        );
    }

    #[test]
    fn uninstalling_when_nothing_is_installed_says_so() {
        let project = Project::new("uninstall-empty");
        let integration = project.integration("claude-code");

        assert_eq!(integration.uninstall().unwrap(), Removal::NothingToDo);
        assert!(!project.exists("CLAUDE.md"), "and creates nothing");
    }

    #[test]
    fn a_file_ctxc_owns_is_created_with_its_preamble_and_deleted_on_uninstall() {
        let project = Project::new("owned");
        let integration = project.integration("cursor");

        integration.install().unwrap();
        let contents = project.read(".cursor/rules/ctxc.mdc");
        assert!(contents.starts_with("---\n"), "{contents}");
        assert!(contents.contains("alwaysApply: true"), "{contents}");
        assert!(contents.contains("ctxc find"), "{contents}");

        integration.uninstall().unwrap();
        assert!(
            !project.exists(".cursor/rules/ctxc.mdc"),
            "a file CtxC created should not be left behind as an empty stub"
        );
    }

    #[test]
    fn reinstalling_an_owned_file_does_not_stack_up_preambles() {
        let project = Project::new("owned-twice");
        let integration = project.integration("cursor");

        integration.install().unwrap();
        integration.install().unwrap();

        let contents = project.read(".cursor/rules/ctxc.mdc");
        assert_eq!(contents.matches("alwaysApply").count(), 1, "{contents}");
    }

    #[test]
    fn detection_follows_the_marker_an_agent_leaves() {
        let project = Project::new("detect");
        assert!(!project.integration("claude-code").detect());

        project.write(".claude/settings.json", "{}");
        assert!(project.integration("claude-code").detect());

        // One agent's marker does not make another agent present.
        assert!(!project.integration("cursor").detect());
    }

    #[test]
    fn guidance_from_an_older_version_is_reported_as_outdated() {
        let project = Project::new("outdated");
        let integration = project.integration("claude-code");
        integration.install().unwrap();

        // Stand in for a release whose guidance said something else.
        let block = ManagedBlock::new(CommentStyle::Html);
        let (stale, _) = block.write(&project.read("CLAUDE.md"), "Old advice.");
        project.write("CLAUDE.md", &stale);

        let status = integration.status().unwrap();
        assert!(status.installed);
        assert!(status.outdated);
        assert_eq!(status.summary(), "outdated");

        // And installing brings it back up to date.
        assert_eq!(integration.install().unwrap(), Change::Updated);
        assert!(!integration.status().unwrap().outdated);
    }

    #[test]
    fn a_binary_file_is_refused_rather_than_overwritten() {
        let project = Project::new("binary");
        std::fs::write(project.root.join("CLAUDE.md"), [0xff, 0xfe, 0x00]).unwrap();

        let integration = project.integration("claude-code");
        let error = integration.install().unwrap_err();

        assert!(matches!(error, IntegrationError::NotText { .. }));
        assert_eq!(
            std::fs::read(project.root.join("CLAUDE.md")).unwrap(),
            vec![0xff, 0xfe, 0x00],
            "the file must be left exactly as it was"
        );
    }

    #[test]
    fn each_integration_installs_and_uninstalls_independently() {
        let project = Project::new("independent");

        for name in names() {
            let integration = project.integration(name);
            integration.install().unwrap();
            assert!(integration.status().unwrap().installed, "{name}");
        }

        // Removing one leaves the rest alone.
        project.integration("claude-code").uninstall().unwrap();
        assert!(
            !project
                .integration("claude-code")
                .status()
                .unwrap()
                .installed
        );
        for name in names().into_iter().filter(|name| *name != "claude-code") {
            assert!(
                project.integration(name).status().unwrap().installed,
                "{name} should be untouched"
            );
        }
    }

    #[test]
    fn registering_an_mcp_server_writes_a_command_the_agent_can_run() {
        let project = Project::new("mcp-install");
        let integration = project.integration("claude-code-mcp");

        assert_eq!(integration.install().unwrap(), Change::Added);

        let document: serde_json::Value = serde_json::from_str(&project.read(".mcp.json")).unwrap();
        assert_eq!(document["mcpServers"]["ctxc"]["command"], "ctxc");
        assert_eq!(document["mcpServers"]["ctxc"]["args"][0], "mcp");
    }

    #[test]
    fn another_tools_mcp_entry_is_left_alone() {
        let project = Project::new("mcp-coexist");
        project.write(
            ".mcp.json",
            r#"{"mcpServers":{"other":{"command":"other-tool"}}}"#,
        );

        let integration = project.integration("claude-code-mcp");
        integration.install().unwrap();

        let document: serde_json::Value = serde_json::from_str(&project.read(".mcp.json")).unwrap();
        assert_eq!(document["mcpServers"]["other"]["command"], "other-tool");

        integration.uninstall().unwrap();
        let document: serde_json::Value = serde_json::from_str(&project.read(".mcp.json")).unwrap();
        assert_eq!(
            document["mcpServers"]["other"]["command"], "other-tool",
            "uninstalling CtxC must not unregister somebody else's server"
        );
        assert!(document["mcpServers"].get("ctxc").is_none());
    }

    #[test]
    fn a_config_file_ctxc_created_is_deleted_when_it_is_uninstalled() {
        let project = Project::new("mcp-delete");
        let integration = project.integration("claude-code-mcp");

        integration.install().unwrap();
        assert!(project.exists(".mcp.json"));

        integration.uninstall().unwrap();
        assert!(
            !project.exists(".mcp.json"),
            "an empty `{{}}` nobody wrote should not be left behind"
        );
    }

    #[test]
    fn registering_twice_changes_nothing_the_second_time() {
        let project = Project::new("mcp-idempotent");
        let integration = project.integration("cursor-mcp");

        integration.install().unwrap();
        let before = project.read(".cursor/mcp.json");

        assert_eq!(integration.install().unwrap(), Change::Unchanged);
        assert_eq!(project.read(".cursor/mcp.json"), before);
    }

    #[test]
    fn an_entry_written_by_an_older_version_is_reported_as_outdated() {
        let project = Project::new("mcp-outdated");
        project.write(
            ".mcp.json",
            r#"{"mcpServers":{"ctxc":{"command":"/old/path/ctxc","args":["mcp"]}}}"#,
        );

        let integration = project.integration("claude-code-mcp");
        let status = integration.status().unwrap();
        assert!(status.installed);
        assert!(status.outdated, "a stale command should be flagged");

        assert_eq!(integration.install().unwrap(), Change::Updated);
        assert!(!integration.status().unwrap().outdated);
    }

    #[test]
    fn a_config_file_with_comments_is_refused_rather_than_rewritten() {
        let project = Project::new("mcp-jsonc");
        let original = "{\n  // the tools I use\n  \"mcpServers\": {}\n}\n";
        project.write(".mcp.json", original);

        let integration = project.integration("claude-code-mcp");
        let error = integration.install().unwrap_err();

        assert!(matches!(error, IntegrationError::NotJson { .. }));
        assert_eq!(
            project.read(".mcp.json"),
            original,
            "rewriting JSONC as JSON would silently delete the comment"
        );
    }

    #[test]
    fn the_two_mechanisms_are_reported_apart() {
        let project = Project::new("mechanisms");

        assert_eq!(
            project
                .integration("claude-code")
                .status()
                .unwrap()
                .mechanism,
            Mechanism::Instructions
        );
        assert_eq!(
            project
                .integration("claude-code-mcp")
                .status()
                .unwrap()
                .mechanism,
            Mechanism::McpServer
        );
    }

    #[test]
    fn instructions_and_an_mcp_server_can_both_be_installed_for_one_agent() {
        let project = Project::new("both");

        project.integration("claude-code").install().unwrap();
        project.integration("claude-code-mcp").install().unwrap();

        assert!(project.read("CLAUDE.md").contains("ctxc find"));
        assert!(project.read(".mcp.json").contains("\"ctxc\""));

        // And removing one leaves the other in place.
        project.integration("claude-code-mcp").uninstall().unwrap();
        assert!(project.read("CLAUDE.md").contains("ctxc find"));
    }

    #[test]
    fn all_returns_one_integration_per_agent() {
        let project = Project::new("all");
        let integrations = all(&project.root, "demo");

        assert_eq!(integrations.len(), AGENTS.len());
        let listed: Vec<&str> = integrations.iter().map(|i| i.name()).collect();
        assert_eq!(listed, names());
    }
}
