//! Describing the command surface for the dashboard.
//!
//! The dashboard has a page listing what CtxC can do. Writing that list by hand
//! would guarantee it drifts: a flag added here would go unmentioned there
//! until somebody noticed. So the list is read out of the same `clap`
//! definition that parses the command line, and handed to the API at startup.
//!
//! Two things are not derivable and are written down here instead: the examples
//! worth showing, and where in the dashboard a command's work can be done. Both
//! are editorial. Only commands that genuinely have an equivalent get one —
//! claiming a button exists for `ctxc optimize`, whose product is a stream on
//! stdout, would be worse than admitting it needs a terminal.

use clap::{Arg, ArgAction, Command, CommandFactory};

use ctxc_api::commands::{ArgumentInfo, Catalog, CommandInfo, DashboardEquivalent, OptionInfo};

use crate::cli::Cli;

/// Build the catalog for this binary.
pub fn build() -> Catalog {
    // `build()` resolves defaults, propagates global arguments and settles help
    // text — reading a command before that would describe a half-made tree.
    let mut root = Cli::command();
    root.build();

    let global = root
        .get_arguments()
        .filter(|argument| argument.is_global_set())
        .map(describe_option)
        .collect::<Vec<_>>();
    let global_ids = root
        .get_arguments()
        .filter(|argument| argument.is_global_set())
        .map(|argument| argument.get_id().to_string())
        .collect::<Vec<_>>();

    Catalog {
        name: root.get_name().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        about: root.get_about().map(ToString::to_string),
        commands: root
            .get_subcommands()
            .filter(|command| !command.is_hide_set())
            .map(|command| describe(command, "", &global_ids))
            .collect(),
        global_options: global,
    }
}

/// Describe one command and everything under it.
fn describe(command: &Command, parent: &str, global_ids: &[String]) -> CommandInfo {
    let name = command.get_name().to_string();
    let path = if parent.is_empty() {
        name.clone()
    } else {
        format!("{parent} {name}")
    };

    let visible = |argument: &&Arg| {
        !argument.is_hide_set()
            // `--help` and `--version` are true of everything, and repeating
            // them on every card is noise rather than documentation.
            && !matches!(argument.get_id().as_str(), "help" | "version")
            && !global_ids.iter().any(|id| id == argument.get_id().as_str())
    };

    let arguments: Vec<ArgumentInfo> = command
        .get_arguments()
        .filter(visible)
        .filter(|argument| argument.is_positional())
        .map(describe_argument)
        .collect();

    let options: Vec<OptionInfo> = command
        .get_arguments()
        .filter(visible)
        .filter(|argument| !argument.is_positional())
        .map(describe_option)
        .collect();

    CommandInfo {
        usage: usage(&path, &arguments, !options.is_empty(), command),
        summary: command.get_about().map(ToString::to_string),
        description: command
            .get_long_about()
            .map(ToString::to_string)
            // A long description that only repeats the summary is not worth the
            // space it takes on a card.
            .filter(|long| {
                Some(long.as_str())
                    != command
                        .get_about()
                        .map(|about| about.to_string())
                        .as_deref()
            }),
        examples: examples(&path),
        dashboard: equivalent(&path),
        subcommands: command
            .get_subcommands()
            .filter(|child| !child.is_hide_set())
            .map(|child| describe(child, &path, global_ids))
            .collect(),
        arguments,
        options,
        name,
        path,
    }
}

fn describe_argument(argument: &Arg) -> ArgumentInfo {
    ArgumentInfo {
        name: value_name(argument),
        help: argument.get_help().map(ToString::to_string),
        required: argument.is_required_set(),
        repeated: matches!(argument.get_action(), ArgAction::Append | ArgAction::Count)
            || argument
                .get_num_args()
                .is_some_and(|range| range.max_values() > 1),
    }
}

fn describe_option(argument: &Arg) -> OptionInfo {
    let takes_value = !matches!(
        argument.get_action(),
        ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count
    );

    OptionInfo {
        name: argument
            .get_long()
            .map(ToString::to_string)
            .unwrap_or_else(|| argument.get_id().to_string()),
        short: argument.get_short(),
        help: argument.get_help().map(ToString::to_string),
        value_name: takes_value.then(|| value_name(argument)),
        default: argument
            .get_default_values()
            .first()
            .map(|value| value.to_string_lossy().into_owned()),
        values: argument
            .get_possible_values()
            .iter()
            .map(|value| value.get_name().to_string())
            .collect(),
        required: argument.is_required_set(),
    }
}

/// The placeholder an argument's value is shown as.
fn value_name(argument: &Arg) -> String {
    argument
        .get_value_names()
        .and_then(|names| names.first())
        .map(ToString::to_string)
        .unwrap_or_else(|| argument.get_id().to_string().to_uppercase())
}

/// A one-line usage string, in the shape `--help` prints.
fn usage(path: &str, arguments: &[ArgumentInfo], has_options: bool, command: &Command) -> String {
    let mut usage = format!("ctxc {path}");
    if has_options {
        usage.push_str(" [OPTIONS]");
    }
    if command.get_subcommands().next().is_some() {
        usage.push_str(" <COMMAND>");
    }
    for argument in arguments {
        let repeat = if argument.repeated { "..." } else { "" };
        if argument.required {
            usage.push_str(&format!(" <{}>{repeat}", argument.name));
        } else {
            usage.push_str(&format!(" [{}]{repeat}", argument.name));
        }
    }
    usage
}

/// Examples worth showing, for the commands where one explains more than the
/// flags do.
fn examples(path: &str) -> Vec<String> {
    let lines: &[&str] = match path {
        "optimize" => &[
            "ctxc optimize notes.md --budget 2000",
            "ctxc optimize README.md src/lib.rs --budget 8000",
            "ctxc optimize --dry-run notes.md",
            "git status | ctxc optimize --from \"git status\"",
            "ctxc optimize --budget 1500 -- cargo test",
        ],
        "find" => &[
            "ctxc find \"auth timeout\"",
            "ctxc find \"retry policy\" --compile --budget 4000",
            "ctxc find \"connection refused\" --similar --limit 5",
            "ctxc find ctxc://context/9f2c1d",
        ],
        "project add" => &["ctxc project add . --index"],
        "project index" => &["ctxc project index .", "ctxc project index . --force"],
        "project graph" => &[
            "ctxc project graph .",
            "ctxc project graph . --file src/main.rs",
        ],
        "status" => &[
            "ctxc status",
            "ctxc status --daemon",
            "ctxc status --metrics --days 7 --breakdown",
        ],
        "start" => &["ctxc start", "ctxc start --detach"],
        "stop" => &["ctxc stop", "ctxc stop --all"],
        "config" => &["ctxc config show", "ctxc config init"],
        "config agents" => &[
            "ctxc config agents list",
            "ctxc config agents install --detected",
        ],
        "update" => &["ctxc update --check"],
        _ => &[],
    };
    lines.iter().map(|line| line.to_string()).collect()
}

/// Where the dashboard does the same work, when it does.
fn equivalent(path: &str) -> Option<DashboardEquivalent> {
    let (route, label) = match path {
        "status" => ("/system", "System status"),
        "stop" => ("/system", "Stop daemon & dashboard"),
        "config" | "config show" | "config path" | "config init" => ("/settings", "Settings"),
        "config agents" => ("/settings", "Agent integrations"),
        "project" | "project list" => ("/projects", "Projects"),
        "project add" => ("/projects", "Add project"),
        "project status" => ("/projects", "Project details"),
        "project pause" => ("/projects", "Pause monitoring"),
        "project resume" => ("/projects", "Resume monitoring"),
        "project remove" => ("/projects", "Remove project"),
        "project index" => ("/projects", "Re-index"),
        "project graph" => ("/context", "Dependency graph"),
        "find" => ("/context", "Search"),
        "dashboard" => ("/", "You are looking at it"),
        // Everything else is a terminal shape: content on stdout, a process to
        // wrap, a protocol on stdin, or a rebuild of this binary.
        _ => return None,
    };
    Some(DashboardEquivalent::new(route, label))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_covers_the_command_tree() {
        let catalog = build();

        assert_eq!(catalog.name, "ctxc");
        assert!(catalog.find("project add").is_some());
        assert!(catalog.find("project index").is_some());
        assert!(catalog.find("config init").is_some());
        assert!(catalog.find("config agents install").is_some());
        assert!(
            catalog.find("status").is_some(),
            "a top-level command must be reachable"
        );
    }

    /// The dashboard lists what a person should learn, not every name the
    /// binary still answers to.
    #[test]
    fn commands_kept_only_for_compatibility_stay_out_of_the_catalog() {
        let catalog = build();
        let top: Vec<&str> = catalog
            .commands
            .iter()
            .map(|command| command.name.as_str())
            .collect();

        assert_eq!(
            top,
            [
                "optimize",
                "find",
                "project",
                "start",
                "stop",
                "status",
                "config",
                "dashboard",
                "update",
            ]
        );

        for gone in [
            "analyze", "compile", "capture", "index", "graph", "metrics", "daemon", "mcp",
        ] {
            assert!(catalog.find(gone).is_none(), "{gone} is still catalogued");
        }
    }

    #[test]
    fn global_options_are_listed_once_rather_than_on_every_command() {
        let catalog = build();
        let names: Vec<&str> = catalog
            .global_options
            .iter()
            .map(|option| option.name.as_str())
            .collect();
        assert!(names.contains(&"format"), "{names:?}");

        let status = catalog.find("status").unwrap();
        assert!(
            status.options.iter().all(|option| option.name != "format"),
            "a global option must not be repeated on each command"
        );
    }

    #[test]
    fn arguments_and_options_carry_what_a_reader_needs() {
        let catalog = build();

        let add = catalog.find("project add").unwrap();
        let path = &add.arguments[0];
        assert!(path.required);
        assert!(path.help.is_some());

        let index = add
            .options
            .iter()
            .find(|option| option.name == "index")
            .expect("`--index` is an option of `project add`");
        assert!(
            index.value_name.is_none(),
            "a flag takes no value and must not advertise a placeholder"
        );

        let find = catalog.find("find").unwrap();
        let limit = find
            .options
            .iter()
            .find(|option| option.name == "limit")
            .expect("`--limit` is an option of `find`");
        assert_eq!(limit.default.as_deref(), Some("20"));
        assert_eq!(limit.value_name.as_deref(), Some("COUNT"));
    }

    #[test]
    fn usage_reads_the_way_help_prints_it() {
        let catalog = build();
        assert_eq!(
            catalog.find("project add").unwrap().usage,
            "ctxc project add [OPTIONS] <PATH>"
        );
        assert_eq!(
            catalog.find("project index").unwrap().usage,
            "ctxc project index [OPTIONS] [PATH]"
        );
        assert_eq!(
            catalog.find("project").unwrap().usage,
            "ctxc project <COMMAND>"
        );
    }

    #[test]
    fn only_commands_with_a_real_equivalent_claim_one() {
        let catalog = build();

        assert_eq!(
            catalog
                .find("project add")
                .unwrap()
                .dashboard
                .as_ref()
                .unwrap()
                .route,
            "/projects"
        );
        assert!(
            catalog.find("optimize").unwrap().dashboard.is_none(),
            "a command whose product is a stream has no dashboard equivalent"
        );
        assert!(catalog.find("update").unwrap().dashboard.is_none());
    }

    #[test]
    fn every_equivalent_points_at_a_route_the_dashboard_has() {
        const ROUTES: &[&str] = &[
            "/",
            "/projects",
            "/activity",
            "/performance",
            "/context",
            "/commands",
            "/settings",
            "/system",
        ];

        for command in build().flatten() {
            if let Some(equivalent) = &command.dashboard {
                assert!(
                    ROUTES.contains(&equivalent.route.as_str()),
                    "{} points at {}, which is not a dashboard route",
                    command.path,
                    equivalent.route
                );
            }
        }
    }
}
