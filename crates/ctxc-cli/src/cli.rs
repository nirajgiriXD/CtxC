//! Command line surface.
//!
//! The command tree follows the structure in ARCHITECTURE.md section 5; only
//! the commands implemented in this phase are declared, so `ctxc --help` never
//! advertises something that does not work.

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
    /// Describe input and what optimizing it would save.
    Analyze {
        /// File to analyze. Omit it, or pass `-`, to read standard input.
        #[arg(value_name = "INPUT")]
        input: Option<PathBuf>,

        #[command(flatten)]
        source: SourceOptions,
    },

    /// Optimize input and write the result to stdout.
    Optimize {
        /// File to optimize. Omit it, or pass `-`, to read standard input.
        #[arg(value_name = "INPUT")]
        input: Option<PathBuf>,

        #[command(flatten)]
        source: SourceOptions,

        #[command(flatten)]
        options: OptimizeOptions,
    },

    /// Optimize several inputs into one AI-ready document.
    Compile {
        /// Files to compile, in the order they should appear. `-` reads
        /// standard input.
        #[arg(value_name = "INPUT", required = true)]
        inputs: Vec<PathBuf>,

        #[command(flatten)]
        options: OptimizeOptions,
    },

    /// Index a project's code: files, symbols and how they relate.
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

    /// Find the files closest to a piece of text, by embedding similarity.
    Similar {
        /// The text to compare against. A phrase, a symbol name, an error.
        #[arg(value_name = "TEXT")]
        query: String,

        /// Project directory. Defaults to the current one.
        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,

        /// How many files to list.
        #[arg(long, default_value_t = 10, value_name = "COUNT")]
        limit: usize,
    },

    /// Find the context most relevant to a question.
    Search {
        /// What to look for. Quote a phrase to keep it together.
        #[arg(value_name = "QUERY")]
        query: String,

        /// Project directory. Defaults to the current one.
        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,

        #[command(flatten)]
        options: SearchOptions,
    },

    /// Recover the original content behind a `ctxc://context/<id>` reference.
    Retrieve {
        /// The reference, or just the id.
        #[arg(value_name = "REFERENCE")]
        reference: String,
    },

    /// Run a command and optimize what it prints.
    Capture {
        /// The command to run, after `--`. It is executed directly, without a
        /// shell.
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

    /// Manage the projects CtxC looks after.
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

    /// Lower-level daemon control and diagnostics.
    Daemon {
        #[command(subcommand)]
        action: Option<DaemonAction>,
    },

    /// Show what CtxC has saved, and where the saving came from.
    Metrics {
        #[command(flatten)]
        options: MetricsOptions,
    },

    /// Serve CtxC over the Model Context Protocol, on stdin and stdout.
    ///
    /// Agents spawn this; people rarely run it directly.
    Mcp,

    /// Tell coding agents about CtxC.
    Integrations {
        #[command(subcommand)]
        action: Option<IntegrationAction>,
    },

    /// Open the local dashboard in a browser.
    Dashboard {
        /// Print the URL instead of launching a browser.
        #[arg(long)]
        no_open: bool,
    },

    /// Print version and build information.
    Version,

    /// Update CtxC: fetch the latest source, build it, replace this binary.
    Update {
        #[command(flatten)]
        options: UpdateOptions,
    },

    /// Show the state of this CtxC installation.
    Status,

    /// Inspect and create configuration.
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
}

/// What `ctxc integrations` can do.
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

/// Shared options for `ctxc metrics`.
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

/// What `ctxc daemon` can do.
#[derive(Debug, Subcommand)]
pub enum DaemonAction {
    /// Report whether a daemon is running.
    Status,

    /// Start one in the background.
    Start,

    /// Stop the running one.
    Stop,
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

/// Options for `ctxc search`.
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
        assert!(Cli::try_parse_from(["ctxc", "daemon"]).is_ok());
        assert!(Cli::try_parse_from(["ctxc", "daemon", "status"]).is_ok());
        assert!(Cli::try_parse_from(["ctxc", "project", "add", "."]).is_ok());
        assert!(Cli::try_parse_from(["ctxc", "project", "list"]).is_ok());
        assert!(Cli::try_parse_from(["ctxc", "project", "pause", "acme"]).is_ok());

        assert!(
            Cli::try_parse_from(["ctxc", "project", "add"]).is_err(),
            "adding a project needs a path"
        );
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
    fn search_takes_a_budget_and_a_compile_flag() {
        let cli = Cli::try_parse_from([
            "ctxc",
            "search",
            "auth timeout",
            "--compile",
            "--budget",
            "800",
        ])
        .unwrap();
        match cli.command {
            Command::Search { query, options, .. } => {
                assert_eq!(query, "auth timeout");
                assert!(options.compile);
                assert_eq!(options.budget, Some(800));
                assert!(!options.no_store);
            }
            other => panic!("parsed as {other:?}"),
        }
    }

    #[test]
    fn retrieve_takes_a_reference() {
        let cli = Cli::try_parse_from(["ctxc", "retrieve", "ctxc://context/abc"]).unwrap();
        assert!(matches!(cli.command, Command::Retrieve { .. }));
        assert!(Cli::try_parse_from(["ctxc", "retrieve"]).is_err());
    }

    #[test]
    fn optimize_input_is_optional_so_stdin_can_be_used() {
        let cli = Cli::try_parse_from(["ctxc", "optimize"]).unwrap();
        assert!(matches!(cli.command, Command::Optimize { input: None, .. }));
        assert!(Cli::try_parse_from(["ctxc", "optimize", "notes.txt"]).is_ok());
        assert!(Cli::try_parse_from(["ctxc", "optimize", "-"]).is_ok());
    }

    #[test]
    fn capture_takes_a_command_and_its_flags() {
        let cli = Cli::try_parse_from([
            "ctxc", "capture", "--budget", "500", "--", "git", "-c", "x=y", "status",
        ])
        .unwrap();
        match cli.command {
            Command::Capture { command, options } => {
                assert_eq!(command, vec!["git", "-c", "x=y", "status"]);
                assert_eq!(options.budget, Some(500));
            }
            other => panic!("parsed as {other:?}"),
        }
        assert!(Cli::try_parse_from(["ctxc", "capture"]).is_err());
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
    fn compile_takes_several_inputs() {
        let cli = Cli::try_parse_from(["ctxc", "compile", "a.txt", "b.txt"]).unwrap();
        match cli.command {
            Command::Compile { inputs, .. } => assert_eq!(inputs.len(), 2),
            other => panic!("parsed as {other:?}"),
        }
        assert!(Cli::try_parse_from(["ctxc", "compile"]).is_err());
    }
}
