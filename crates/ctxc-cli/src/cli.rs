//! Command line surface.
//!
//! Ten commands are advertised, grouped by what a person is trying to do: set
//! a project up, shrink some input, find some context, look after a project,
//! start and stop the daemon, see how things stand, configure, open the
//! dashboard, upgrade.
//!
//! Every name CtxC used to have still parses, as a hidden command, so a script
//! written against an older build keeps working. Hidden means absent from
//! `--help`, not absent from the binary.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::output::OutputFormat;

#[derive(Debug, Parser)]
#[command(
    name = "ctxc",
    version,
    about = "CtxC — cross-platform context optimization for AI agents",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Configuration file to use instead of the platform default.
    #[arg(long, value_name = "PATH", global = true)]
    pub config: Option<PathBuf>,

    /// Output format.
    #[arg(long, value_name = "FORMAT", global = true, default_value = "human")]
    pub format: OutputFormat,

    /// Log informational messages to stderr.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Log debug messages to stderr.
    #[arg(long, global = true)]
    pub debug: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Set a project up: register it, index it, and tell your agents about it.
    ///
    /// One command instead of four. Every step is idempotent, so running it
    /// again on a project that is already set up is a way of checking on it.
    Init {
        /// The project directory. Defaults to the current one.
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,

        /// Register the project without reading its code.
        #[arg(long)]
        no_index: bool,

        /// Leave the agent instruction files alone.
        #[arg(long)]
        no_agents: bool,

        /// Start the daemon without asking.
        #[arg(long)]
        start: bool,

        /// Answer yes to every question, for unattended runs.
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Optimize input and write the result to stdout.
    ///
    /// Takes one file, several files, standard input, or — after `--` — a
    /// command whose output CtxC captures and optimizes:
    ///
    ///   ctxc optimize notes.md
    ///   ctxc optimize README.md src/lib.rs --budget 8000
    ///   git status | ctxc optimize --from "git status"
    ///   ctxc optimize -- cargo test
    Optimize {
        /// Files to optimize, in the order they should appear. Omit them, or
        /// pass `-`, to read standard input.
        #[arg(value_name = "INPUT")]
        inputs: Vec<PathBuf>,

        /// Describe what optimizing would save, without producing it.
        #[arg(long)]
        dry_run: bool,

        /// A command to run, written after `--`. It is executed directly,
        /// without a shell, and its output is what gets optimized.
        #[arg(value_name = "COMMAND", last = true, allow_hyphen_values = true)]
        command: Vec<String>,

        #[command(flatten)]
        source: SourceOptions,

        #[command(flatten)]
        options: OptimizeOptions,
    },

    /// Find the context most relevant to a question.
    ///
    /// Searches the index by default. `--similar` ranks by embedding
    /// similarity instead, and a `ctxc://context/<id>` query recovers the
    /// original content behind that reference.
    #[command(alias = "search")]
    Find {
        /// What to look for: a phrase, a symbol name, an error, or a
        /// `ctxc://context/<id>` reference.
        #[arg(value_name = "QUERY")]
        query: String,

        /// Project directory. Defaults to the current one.
        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,

        /// Rank by embedding similarity alone, rather than searching the index.
        #[arg(long)]
        similar: bool,

        #[command(flatten)]
        options: SearchOptions,
    },

    /// Manage the projects CtxC looks after, and what it knows about them.
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },

    /// Start the CtxC daemon.
    Start {
        /// Run it in the background instead of in this terminal.
        #[arg(long)]
        detach: bool,
    },

    /// Stop the daemon and every other CtxC process.
    Stop {
        /// Also stop CtxC processes belonging to other data directories.
        #[arg(long)]
        all: bool,
    },

    /// Check this installation for problems, and say how to fix each one.
    ///
    /// `status` reports what CtxC is. This reports whether anything about it
    /// is wrong, and exits non-zero when something is.
    Doctor,

    /// Show the state of this CtxC installation.
    Status {
        /// Report the running daemon in full: uptime, projects, what it watches.
        #[arg(long)]
        daemon: bool,

        /// Show what CtxC has saved, and where the saving came from, instead.
        #[arg(long, conflicts_with = "daemon")]
        metrics: bool,

        #[command(flatten)]
        options: MetricsOptions,
    },

    /// Inspect configuration, and set up the agents that use CtxC.
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },

    /// Open the local dashboard in a browser.
    Dashboard {
        /// Print the URL instead of launching a browser.
        #[arg(long)]
        no_open: bool,
    },

    /// Update CtxC: fetch the latest source, build it, replace this binary.
    Update {
        #[command(flatten)]
        options: UpdateOptions,
    },

    /// Print a shell completion script.
    ///
    /// Read once by a shell at startup, so it is installed rather than run;
    /// `ctxc init` prints the line that installs it for the shell in use.
    #[command(hide = true)]
    Completions {
        /// The shell to generate for.
        #[arg(value_name = "SHELL")]
        shell: clap_complete::Shell,
    },

    /// Serve CtxC over the Model Context Protocol, on stdin and stdout.
    ///
    /// Agents spawn this; people rarely run it directly, which is why it is
    /// not advertised in `--help`.
    #[command(hide = true)]
    Mcp,

    /// Superseded by `ctxc optimize --dry-run`.
    #[command(hide = true)]
    Analyze {
        #[arg(value_name = "INPUT")]
        input: Option<PathBuf>,

        #[command(flatten)]
        source: SourceOptions,
    },

    /// Superseded by `ctxc optimize` with several inputs.
    #[command(hide = true)]
    Compile {
        #[arg(value_name = "INPUT", required = true)]
        inputs: Vec<PathBuf>,

        #[command(flatten)]
        options: OptimizeOptions,
    },

    /// Superseded by `ctxc optimize -- <COMMAND>`.
    #[command(hide = true)]
    Capture {
        #[arg(
            value_name = "COMMAND",
            required = true,
            trailing_var_arg = true,
            allow_hyphen_values = true
        )]
        command: Vec<String>,

        #[command(flatten)]
        options: OptimizeOptions,
    },

    /// Superseded by `ctxc find --similar`.
    #[command(hide = true)]
    Similar {
        #[arg(value_name = "TEXT")]
        query: String,

        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,

        #[arg(long, default_value_t = 10, value_name = "COUNT")]
        limit: usize,
    },

    /// Superseded by `ctxc find <REFERENCE>`.
    #[command(hide = true)]
    Retrieve {
        #[arg(value_name = "REFERENCE")]
        reference: String,
    },

    /// Superseded by `ctxc project index`.
    #[command(hide = true)]
    Index {
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,

        #[arg(long)]
        force: bool,
    },

    /// Superseded by `ctxc project graph`.
    #[command(hide = true)]
    Graph {
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,

        #[arg(long, value_name = "RELATIVE_PATH")]
        file: Option<String>,

        #[arg(long, default_value_t = 10, value_name = "COUNT")]
        limit: usize,
    },

    /// Superseded by `ctxc status --metrics`.
    #[command(hide = true)]
    Metrics {
        #[command(flatten)]
        options: MetricsOptions,
    },

    /// Superseded by `ctxc config agents`.
    #[command(hide = true)]
    Integrations {
        #[command(subcommand)]
        action: Option<IntegrationAction>,
    },

    /// Superseded by `ctxc start`, `ctxc stop` and `ctxc status --daemon`.
    #[command(hide = true)]
    Daemon {
        #[command(subcommand)]
        action: Option<DaemonAction>,
    },

    /// Superseded by `ctxc --version`.
    #[command(hide = true)]
    Version,
}

/// What `ctxc config agents` can do.
#[derive(Debug, Subcommand)]
pub enum IntegrationAction {
    /// Show which agents are here and which have CtxC guidance.
    List {
        /// Project directory. Defaults to the current one.
        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,
    },

    /// Write CtxC guidance into an agent's instruction file.
    Install {
        /// The integration to install. Omit it to act on several.
        #[arg(value_name = "NAME")]
        name: Option<String>,

        /// Project directory. Defaults to the current one.
        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,

        /// Only agents that look like they are in use here.
        #[arg(long)]
        detected: bool,
    },

    /// Take CtxC guidance back out, leaving everything else in place.
    Uninstall {
        /// The integration to remove. Omit it to act on several.
        #[arg(value_name = "NAME")]
        name: Option<String>,

        /// Project directory. Defaults to the current one.
        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,

        /// Only agents that look like they are in use here.
        #[arg(long)]
        detected: bool,
    },
}

/// Options for `ctxc update`.
#[derive(Debug, clap::Args)]
pub struct UpdateOptions {
    /// Report what an update would do, without changing anything.
    #[arg(long)]
    pub check: bool,

    /// The CtxC source checkout to build from.
    ///
    /// Found on its own when the running binary sits inside a checkout. A path
    /// given here is remembered, so later updates work from anywhere.
    #[arg(long, value_name = "PATH")]
    pub source: Option<PathBuf>,

    /// The branch to update from.
    #[arg(long, default_value = "main", value_name = "BRANCH")]
    pub branch: String,

    /// Update even when the checkout is dirty, is on another branch, or has
    /// nothing new.
    #[arg(long)]
    pub force: bool,

    /// Do not rebuild the dashboard's web interface.
    #[arg(long)]
    pub no_dashboard: bool,
}

/// Shared options for `ctxc status --metrics`.
#[derive(Debug, clap::Args)]
pub struct MetricsOptions {
    /// Restrict to one project, by id, path or name.
    #[arg(long, value_name = "PROJECT")]
    pub project: Option<String>,

    /// How far back to look.
    #[arg(long, default_value_t = 30, value_name = "DAYS")]
    pub days: u32,

    /// Break the saving down by operation.
    #[arg(long)]
    pub breakdown: bool,

    /// Show one row per hour or per day.
    #[arg(long, value_name = "BUCKET")]
    pub by: Option<crate::commands::metrics::Bucket>,

    /// List this many recent operations.
    #[arg(long, default_value_t = 0, value_name = "COUNT")]
    pub activity: usize,
}

/// What `ctxc project` can do.
#[derive(Debug, Subcommand)]
pub enum ProjectAction {
    /// Register a project.
    Add {
        /// The project directory.
        #[arg(value_name = "PATH")]
        path: PathBuf,

        /// Index it straight away instead of leaving it to the daemon.
        #[arg(long)]
        index: bool,
    },

    /// List registered projects.
    List,

    /// Show one project, re-running detection.
    Status {
        /// Project id, path or name.
        #[arg(value_name = "PROJECT")]
        project: String,
    },

    /// Read a project's code: files, symbols and how they relate.
    Index {
        /// Project directory. Defaults to the current one.
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,

        /// Re-parse every file, even ones that look unchanged.
        #[arg(long)]
        force: bool,
    },

    /// Show how a project's files depend on each other.
    Graph {
        /// Project directory. Defaults to the current one.
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,

        /// Show one file's dependencies and dependents instead of a summary.
        #[arg(long, value_name = "RELATIVE_PATH")]
        file: Option<String>,

        /// How many files to list in the summary.
        #[arg(long, default_value_t = 10, value_name = "COUNT")]
        limit: usize,
    },

    /// Stop looking after a project, without forgetting it.
    Pause {
        #[arg(value_name = "PROJECT")]
        project: String,
    },

    /// Start looking after it again.
    Resume {
        #[arg(value_name = "PROJECT")]
        project: String,
    },

    /// Forget a project. Its files are never touched.
    Remove {
        #[arg(value_name = "PROJECT")]
        project: String,
    },

    /// Print a project's path, for a shell to act on.
    Open {
        #[arg(value_name = "PROJECT")]
        project: String,
    },
}

/// What the hidden `ctxc daemon` still answers to.
///
/// `start` and `stop` are top-level commands, and `ctxc status --daemon` is
/// the report; this exists so a script written against the old tree runs.
#[derive(Debug, Subcommand)]
pub enum DaemonAction {
    /// Superseded by `ctxc status --daemon`.
    Status,

    /// Superseded by `ctxc start`.
    Start {
        #[arg(long)]
        detach: bool,
    },

    /// Superseded by `ctxc stop`.
    Stop {
        #[arg(long)]
        all: bool,
    },
}

/// Where input came from, when CtxC cannot see that for itself.
#[derive(Debug, clap::Args)]
pub struct SourceOptions {
    /// The command that produced this input, e.g. `--from "git status"`.
    ///
    /// Piped input carries no provenance, so this is how a tool-specific
    /// optimizer gets selected for it:
    ///
    ///   git status | ctxc optimize --from "git status"
    #[arg(long, value_name = "COMMAND")]
    pub from: Option<String>,
}

/// Options for `ctxc find`.
#[derive(Debug, clap::Args)]
pub struct SearchOptions {
    /// Maximum number of results.
    #[arg(long, default_value_t = 20, value_name = "COUNT")]
    pub limit: usize,

    /// Emit the selected context as one optimized document.
    #[arg(long)]
    pub compile: bool,

    /// Token budget for `--compile`. Defaults to `budget.default`.
    #[arg(long, value_name = "TOKENS")]
    pub budget: Option<u32>,

    /// With `--compile`, do not keep the cited originals in the database.
    #[arg(long)]
    pub no_store: bool,
}

/// Options shared by the commands that produce optimized content.
#[derive(Debug, clap::Args)]
pub struct OptimizeOptions {
    /// Token budget for the result. Defaults to `budget.default`.
    #[arg(long, value_name = "TOKENS")]
    pub budget: Option<u32>,

    /// Do not keep the original in the context database.
    ///
    /// Originals are stored so that optimized output stays reversible through
    /// its `ctxc://context/<id>` reference.
    #[arg(long)]
    pub no_store: bool,

    /// Optimize again rather than reusing a remembered result.
    ///
    /// The same bytes under the same settings produce the same document, so
    /// the second run is normally free. This does the work anyway.
    #[arg(long)]
    pub no_cache: bool,
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Print the effective configuration after all layers are merged.
    Show,

    /// Show which configuration layers were consulted, and where they live.
    Path,

    /// Write a configuration file containing the built-in defaults.
    Init {
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
    },

    /// Tell coding agents about CtxC.
    Agents {
        #[command(subcommand)]
        action: Option<IntegrationAction>,
    },
}

/// Verbosity chosen on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Verbosity {
    Warn,
    Info,
    Debug,
}

impl Cli {
    /// Resolve the verbosity implied by the flags. `--debug` wins.
    pub fn verbosity(&self) -> Verbosity {
        if self.debug {
            Verbosity::Debug
        } else if self.verbose {
            Verbosity::Info
        } else {
            Verbosity::Warn
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_tree_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn defaults_are_human_output_at_warn_level() {
        let cli = Cli::try_parse_from(["ctxc", "status"]).unwrap();
        assert_eq!(cli.format, OutputFormat::Human);
        assert_eq!(cli.verbosity(), Verbosity::Warn);
    }

    #[test]
    fn debug_beats_verbose() {
        let cli = Cli::try_parse_from(["ctxc", "--verbose", "--debug", "status"]).unwrap();
        assert_eq!(cli.verbosity(), Verbosity::Debug);
    }

    #[test]
    fn global_flags_may_follow_the_subcommand() {
        let cli = Cli::try_parse_from(["ctxc", "config", "show", "--format", "json"]).unwrap();
        assert_eq!(cli.format, OutputFormat::Json);
    }

    #[test]
    fn config_defaults_to_no_action() {
        let cli = Cli::try_parse_from(["ctxc", "config"]).unwrap();
        assert!(matches!(cli.command, Command::Config { action: None }));
    }

    /// The commands `--help` advertises, and nothing else.
    #[test]
    fn only_the_grouped_commands_are_advertised() {
        let listed: Vec<String> = Cli::command()
            .get_subcommands()
            .filter(|command| !command.is_hide_set())
            .map(|command| command.get_name().to_string())
            .collect();

        assert_eq!(
            listed,
            [
                "init",
                "optimize",
                "find",
                "project",
                "start",
                "stop",
                "doctor",
                "status",
                "config",
                "dashboard",
                "update",
            ]
        );
    }

    #[test]
    fn the_dashboard_opens_a_browser_unless_told_not_to() {
        let cli = Cli::try_parse_from(["ctxc", "dashboard"]).unwrap();
        assert!(matches!(cli.command, Command::Dashboard { no_open: false }));

        let cli = Cli::try_parse_from(["ctxc", "dashboard", "--no-open"]).unwrap();
        assert!(matches!(cli.command, Command::Dashboard { no_open: true }));
    }

    #[test]
    fn the_daemon_and_registry_commands_parse() {
        assert!(Cli::try_parse_from(["ctxc", "start"]).is_ok());
        assert!(Cli::try_parse_from(["ctxc", "start", "--detach"]).is_ok());
        assert!(Cli::try_parse_from(["ctxc", "stop"]).is_ok());
        assert!(Cli::try_parse_from(["ctxc", "stop", "--all"]).is_ok());
        assert!(Cli::try_parse_from(["ctxc", "status", "--daemon"]).is_ok());
        assert!(Cli::try_parse_from(["ctxc", "project", "add", "."]).is_ok());
        assert!(Cli::try_parse_from(["ctxc", "project", "list"]).is_ok());
        assert!(Cli::try_parse_from(["ctxc", "project", "pause", "acme"]).is_ok());
        assert!(Cli::try_parse_from(["ctxc", "project", "index", "."]).is_ok());
        assert!(Cli::try_parse_from(["ctxc", "project", "graph", "--file", "a.rs"]).is_ok());
        assert!(Cli::try_parse_from(["ctxc", "config", "agents", "list"]).is_ok());

        assert!(
            Cli::try_parse_from(["ctxc", "project", "add"]).is_err(),
            "adding a project needs a path"
        );
    }

    /// Names from before the regrouping still parse, so scripts keep working.
    #[test]
    fn the_old_names_still_parse() {
        for old in [
            &["analyze", "notes.txt"][..],
            &["compile", "a.txt", "b.txt"][..],
            &["capture", "--", "git", "status"][..],
            &["similar", "text"][..],
            &["retrieve", "ctxc://context/abc"][..],
            &["index", "."][..],
            &["graph", "."][..],
            &["metrics", "--days", "7"][..],
            &["integrations", "list"][..],
            &["search", "auth"][..],
            &["daemon"][..],
            &["daemon", "status"][..],
            &["daemon", "start", "--detach"][..],
            &["daemon", "stop", "--all"][..],
            &["version"][..],
            &["mcp"][..],
        ] {
            let mut argv = vec!["ctxc"];
            argv.extend_from_slice(old);
            assert!(
                Cli::try_parse_from(&argv).is_ok(),
                "{old:?} stopped parsing"
            );
        }
    }

    #[test]
    fn update_follows_main_unless_told_otherwise() {
        let cli = Cli::try_parse_from(["ctxc", "update"]).unwrap();
        match cli.command {
            Command::Update { options } => {
                assert_eq!(options.branch, "main");
                assert!(!options.check);
                assert!(!options.force);
                assert!(options.source.is_none());
            }
            other => panic!("parsed as {other:?}"),
        }

        let cli = Cli::try_parse_from([
            "ctxc",
            "update",
            "--check",
            "--branch",
            "next",
            "--source",
            "/src/ctxc",
            "--no-dashboard",
        ])
        .unwrap();
        match cli.command {
            Command::Update { options } => {
                assert!(options.check);
                assert_eq!(options.branch, "next");
                assert_eq!(options.source, Some(PathBuf::from("/src/ctxc")));
                assert!(options.no_dashboard);
            }
            other => panic!("parsed as {other:?}"),
        }
    }

    #[test]
    fn find_takes_a_budget_and_a_compile_flag() {
        let cli = Cli::try_parse_from([
            "ctxc",
            "find",
            "auth timeout",
            "--compile",
            "--budget",
            "800",
        ])
        .unwrap();
        match cli.command {
            Command::Find {
                query,
                similar,
                options,
                ..
            } => {
                assert_eq!(query, "auth timeout");
                assert!(!similar);
                assert!(options.compile);
                assert_eq!(options.budget, Some(800));
                assert!(!options.no_store);
            }
            other => panic!("parsed as {other:?}"),
        }
    }

    #[test]
    fn search_is_an_alias_for_find() {
        let cli = Cli::try_parse_from(["ctxc", "search", "auth timeout"]).unwrap();
        assert!(matches!(cli.command, Command::Find { .. }));
    }

    #[test]
    fn find_takes_a_similar_flag_and_a_reference() {
        let cli = Cli::try_parse_from(["ctxc", "find", "connection refused", "--similar"]).unwrap();
        assert!(matches!(cli.command, Command::Find { similar: true, .. }));

        let cli = Cli::try_parse_from(["ctxc", "find", "ctxc://context/abc"]).unwrap();
        match cli.command {
            Command::Find { query, .. } => assert_eq!(query, "ctxc://context/abc"),
            other => panic!("parsed as {other:?}"),
        }
    }

    #[test]
    fn optimize_accepts_stdin_one_file_or_many() {
        let cli = Cli::try_parse_from(["ctxc", "optimize"]).unwrap();
        match cli.command {
            Command::Optimize {
                inputs,
                command,
                dry_run,
                ..
            } => {
                assert!(inputs.is_empty());
                assert!(command.is_empty());
                assert!(!dry_run);
            }
            other => panic!("parsed as {other:?}"),
        }

        let cli = Cli::try_parse_from(["ctxc", "optimize", "a.txt", "b.txt"]).unwrap();
        match cli.command {
            Command::Optimize { inputs, .. } => assert_eq!(inputs.len(), 2),
            other => panic!("parsed as {other:?}"),
        }

        assert!(Cli::try_parse_from(["ctxc", "optimize", "-"]).is_ok());
    }

    #[test]
    fn optimize_dry_run_replaces_analyze() {
        let cli = Cli::try_parse_from(["ctxc", "optimize", "--dry-run", "notes.txt"]).unwrap();
        match cli.command {
            Command::Optimize {
                dry_run, inputs, ..
            } => {
                assert!(dry_run);
                assert_eq!(inputs, vec![PathBuf::from("notes.txt")]);
            }
            other => panic!("parsed as {other:?}"),
        }
    }

    #[test]
    fn optimize_takes_a_command_after_a_double_dash() {
        let cli = Cli::try_parse_from([
            "ctxc", "optimize", "--budget", "500", "--", "git", "-c", "x=y", "status",
        ])
        .unwrap();
        match cli.command {
            Command::Optimize {
                inputs,
                command,
                options,
                ..
            } => {
                assert!(inputs.is_empty());
                assert_eq!(command, vec!["git", "-c", "x=y", "status"]);
                assert_eq!(options.budget, Some(500));
            }
            other => panic!("parsed as {other:?}"),
        }
    }

    #[test]
    fn optimize_options_parse() {
        let cli = Cli::try_parse_from([
            "ctxc",
            "optimize",
            "notes.txt",
            "--budget",
            "500",
            "--no-store",
        ])
        .unwrap();
        match cli.command {
            Command::Optimize { options, .. } => {
                assert_eq!(options.budget, Some(500));
                assert!(options.no_store);
            }
            other => panic!("parsed as {other:?}"),
        }
    }

    #[test]
    fn status_carries_the_metrics_flag() {
        let cli = Cli::try_parse_from(["ctxc", "status"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Status {
                metrics: false,
                daemon: false,
                ..
            }
        ));

        let cli = Cli::try_parse_from(["ctxc", "status", "--daemon"]).unwrap();
        assert!(matches!(cli.command, Command::Status { daemon: true, .. }));

        assert!(
            Cli::try_parse_from(["ctxc", "status", "--daemon", "--metrics"]).is_err(),
            "two different reports cannot both be the answer"
        );

        let cli =
            Cli::try_parse_from(["ctxc", "status", "--metrics", "--days", "7", "--breakdown"])
                .unwrap();
        match cli.command {
            Command::Status {
                metrics, options, ..
            } => {
                assert!(metrics);
                assert_eq!(options.days, 7);
                assert!(options.breakdown);
            }
            other => panic!("parsed as {other:?}"),
        }
    }
}
