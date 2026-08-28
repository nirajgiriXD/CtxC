//! End-to-end tests that run the real `ctxc` binary.
//!
//! Every test gets its own `CTXC_HOME`, so nothing touches the developer's real
//! configuration or database, and the tests can run in parallel.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A throwaway CtxC home directory, removed when the test ends.
struct Sandbox {
    home: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Sandbox {
        let home = std::env::temp_dir()
            .join("ctxc-cli-tests")
            .join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("create sandbox");
        Sandbox { home }
    }

    /// A sandbox whose daemon binds a port the operating system picks.
    ///
    /// Tests run in parallel, so a fixed port would make them fight over one
    /// socket. Port 0 means "any free port", and the lockfile records whichever
    /// one was chosen.
    fn with_daemon(name: &str) -> Sandbox {
        let sandbox = Sandbox::new(name);
        std::fs::write(sandbox.path("config.toml"), "[daemon]\nport = 0\n").unwrap();
        sandbox
    }

    /// Run `ctxc` with the sandbox as its home and a clean environment.
    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("run ctxc")
    }

    /// Run `ctxc` with `input` written to its standard input.
    fn run_with_stdin(&self, args: &[&str], input: &str) -> Output {
        use std::io::Write;

        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn ctxc");
        child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(input.as_bytes())
            .expect("write stdin");
        child.wait_with_output().expect("run ctxc")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ctxc"));
        command.args(args).env("CTXC_HOME", &self.home);
        // Configuration variables leaking in from the developer's shell would
        // make these assertions depend on the machine.
        for key in ["CTXC_LOG", "RUST_LOG", "CTXC_DAEMON_PORT"] {
            command.env_remove(key);
        }
        command
    }

    /// Start a daemon this test owns.
    ///
    /// Deliberately the foreground `start`, run as a child the test can wait
    /// on: the process is reaped when the test ends, so a failure cannot leave
    /// a daemon behind holding the sandbox open. `--detach` is exercised
    /// separately, where its own behaviour is the subject.
    fn spawn_daemon(&self) -> std::process::Child {
        let mut child = self
            .command(&["start"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn the daemon");

        for _ in 0..100 {
            let status = self.run(&["daemon", "status", "--format", "json"]);
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout(&status)) {
                if json["running"] == true {
                    return child;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        // Never leave the process behind, even when the test is about to fail.
        let _ = child.kill();
        let _ = child.wait();
        panic!("the daemon did not start");
    }

    fn path(&self, name: &str) -> PathBuf {
        self.home.join(name)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // A daemon left running would outlive the test, keep the sandbox
        // locked, and hold the harness open. Stopping it is not optional.
        if self.path("daemon.lock").exists() {
            let _ = self.command(&["stop"]).output();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is utf-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is utf-8")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed: {}",
        stderr(output)
    );
}

#[test]
fn version_reports_build_information() {
    let sandbox = Sandbox::new("version");
    let output = sandbox.run(&["version", "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("valid json");
    assert_eq!(json["name"], "ctxc");
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    assert!(json["schema_version"].as_u64().unwrap() >= 1);
}

#[test]
fn status_creates_and_reports_the_database() {
    let sandbox = Sandbox::new("status");
    let output = sandbox.run(&["status"]);
    assert_success(&output);

    let text = stdout(&output);
    assert!(text.contains("Database:"), "{text}");
    assert!(text.contains("Contexts:   0"), "{text}");
    assert!(sandbox.path("ctxc.db").exists(), "database was not created");
}

#[test]
fn status_json_is_machine_readable() {
    let sandbox = Sandbox::new("status-json");
    let output = sandbox.run(&["status", "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("valid json");
    assert_eq!(json["database"]["contexts"], 0);
    assert_eq!(json["config_file_exists"], false);
    assert_eq!(json["daemon"]["running"], false);
    assert_eq!(json["indexed_roots"], 0);
}

#[test]
fn logs_never_contaminate_machine_output() {
    let sandbox = Sandbox::new("logs");
    let output = sandbox.run(&["--debug", "status", "--format", "json"]);
    assert_success(&output);

    serde_json::from_str::<serde_json::Value>(&stdout(&output))
        .expect("stdout stays valid json while debug logging is on");
}

#[test]
fn quiet_prints_nothing() {
    let sandbox = Sandbox::new("quiet");
    let output = sandbox.run(&["status", "--format", "quiet"]);
    assert_success(&output);
    assert_eq!(stdout(&output), "");
}

#[test]
fn config_show_prints_effective_configuration() {
    let sandbox = Sandbox::new("config-show");
    let output = sandbox.run(&["config", "show"]);
    assert_success(&output);

    let parsed: toml::Value = toml::from_str(&stdout(&output)).expect("valid toml");
    assert_eq!(parsed["daemon"]["port"].as_integer(), Some(7717));
    assert_eq!(parsed["telemetry"]["enabled"].as_bool(), Some(false));
}

#[test]
fn configuration_layers_are_applied_in_order() {
    let sandbox = Sandbox::new("config-layers");
    std::fs::write(sandbox.path("config.toml"), "[daemon]\nport = 7000\n").unwrap();

    let file_only = sandbox.run(&["config", "show"]);
    assert_success(&file_only);
    let parsed: toml::Value = toml::from_str(&stdout(&file_only)).unwrap();
    assert_eq!(parsed["daemon"]["port"].as_integer(), Some(7000));

    let mut command = Command::new(env!("CARGO_BIN_EXE_ctxc"));
    let with_env = command
        .args(["config", "show"])
        .env("CTXC_HOME", &sandbox.home)
        .env("CTXC_DAEMON_PORT", "7100")
        .output()
        .unwrap();
    assert_success(&with_env);
    let parsed: toml::Value = toml::from_str(&stdout(&with_env)).unwrap();
    assert_eq!(
        parsed["daemon"]["port"].as_integer(),
        Some(7100),
        "environment must override the file"
    );
}

#[test]
fn config_path_lists_the_layer_stack() {
    let sandbox = Sandbox::new("config-path");
    let output = sandbox.run(&["config", "path", "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let layers = json["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 4);
    assert_eq!(layers[0]["kind"], "defaults");
    assert_eq!(layers[1]["kind"], "file");
    assert_eq!(layers[1]["applied"], false, "no file written yet");
}

#[test]
fn config_init_writes_defaults_and_refuses_to_clobber() {
    let sandbox = Sandbox::new("config-init");
    let config_file = sandbox.path("config.toml");

    assert_success(&sandbox.run(&["config", "init"]));
    let written = std::fs::read_to_string(&config_file).unwrap();
    assert!(written.starts_with("# CtxC configuration."), "{written}");
    toml::from_str::<toml::Value>(&written).expect("written file is valid toml");

    let second = sandbox.run(&["config", "init"]);
    assert!(!second.status.success(), "overwrote an existing file");
    let message = stderr(&second);
    assert!(message.contains("already exists"), "{message}");
    assert!(
        message.contains("Try:\n  ctxc config init --force"),
        "{message}"
    );

    assert_success(&sandbox.run(&["config", "init", "--force"]));
}

#[test]
fn a_broken_configuration_file_fails_with_an_actionable_error() {
    let sandbox = Sandbox::new("config-broken");
    std::fs::write(sandbox.path("config.toml"), "[daemon]\nprot = 7000\n").unwrap();

    let output = sandbox.run(&["status"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "", "failures must not write to stdout");

    let message = stderr(&output);
    assert!(
        message.contains("failed to load configuration"),
        "{message}"
    );
    assert!(message.contains("Reason:"), "{message}");
    assert!(message.contains("Try:"), "{message}");
}

#[test]
fn an_invalid_value_names_the_key() {
    let sandbox = Sandbox::new("config-invalid");
    std::fs::write(
        sandbox.path("config.toml"),
        "[optimization]\ntarget_reduction = 2.0\n",
    )
    .unwrap();

    let output = sandbox.run(&["status"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("optimization.target_reduction"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_explicit_config_file_replaces_the_default() {
    let sandbox = Sandbox::new("config-explicit");
    let elsewhere = sandbox.path("elsewhere.toml");
    std::fs::write(&elsewhere, "[dashboard]\nport = 9999\n").unwrap();

    let output = sandbox.run(&["config", "show", "--config", path_str(&elsewhere)]);
    assert_success(&output);
    let parsed: toml::Value = toml::from_str(&stdout(&output)).unwrap();
    assert_eq!(parsed["dashboard"]["port"].as_integer(), Some(9999));
}

/// `--help` lists the grouped commands and nothing else. Every older name
/// still parses (see `old_names_still_work`), but a person reading help should
/// not have to choose between twenty-one of them.
#[test]
fn help_lists_the_grouped_commands_only() {
    let sandbox = Sandbox::new("help");
    let output = sandbox.run(&["--help"]);
    assert_success(&output);

    // Command names only: a description may legitimately mention a word that
    // is also the name of a command.
    let text = stdout(&output);
    let listed: Vec<&str> = text
        .lines()
        .skip_while(|line| !line.starts_with("Commands:"))
        .skip(1)
        .take_while(|line| line.starts_with("  ") || line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
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
            "status",
            "config",
            "dashboard",
            "update",
        ],
        "{text}"
    );
}

/// Prose with a duplicated paragraph and ragged whitespace: something every
/// optimizer stage has an opinion about.
const SAMPLE: &str = "The service refused the connection.   \n\n\n\
                      The service refused the connection.\n\n\
                      A different sentence that carries new information.\n";

#[test]
fn analyze_describes_the_input() {
    let sandbox = Sandbox::new("analyze");
    let input = sandbox.path("notes.txt");
    std::fs::write(&input, SAMPLE).unwrap();

    let output = sandbox.run(&["analyze", path_str(&input)]);
    assert_success(&output);

    let text = stdout(&output);
    assert!(text.contains("Type:        plain_text"), "{text}");
    assert!(text.contains("Fragments:   3  (1 duplicated)"), "{text}");
    assert!(text.contains("(estimated)"), "{text}");
}

#[test]
fn analyze_json_carries_the_projection() {
    let sandbox = Sandbox::new("analyze-json");
    let input = sandbox.path("notes.txt");
    std::fs::write(&input, SAMPLE).unwrap();

    let output = sandbox.run(&["analyze", path_str(&input), "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["content_type"], "plain_text");
    assert_eq!(json["duplicate_fragments"], 1);
    assert!(json["projected_tokens"].as_u64().unwrap() < json["tokens"].as_u64().unwrap());
    assert!(
        json["projected_savings_by_stage"]["deduplication"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[test]
fn optimize_writes_content_to_stdout_and_the_summary_to_stderr() {
    let sandbox = Sandbox::new("optimize");
    let input = sandbox.path("notes.txt");
    std::fs::write(&input, SAMPLE).unwrap();

    let output = sandbox.run(&["optimize", path_str(&input)]);
    assert_success(&output);

    let content = stdout(&output);
    assert_eq!(
        content,
        "The service refused the connection.\n\nA different sentence that carries new information.\n"
    );

    let summary = stderr(&output);
    assert!(summary.contains("Original tokens:"), "{summary}");
    assert!(summary.contains("Reduction:"), "{summary}");
    assert!(summary.contains("deduplication"), "{summary}");
    assert!(
        summary.contains("Reference:        ctxc://context/"),
        "{summary}"
    );
}

#[test]
fn optimize_json_puts_everything_on_stdout() {
    let sandbox = Sandbox::new("optimize-json");
    let input = sandbox.path("notes.txt");
    std::fs::write(&input, SAMPLE).unwrap();

    let output = sandbox.run(&["optimize", path_str(&input), "--format", "json"]);
    assert_success(&output);
    assert_eq!(stderr(&output), "", "json output must not print to stderr");

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert!(json["content"]
        .as_str()
        .unwrap()
        .contains("A different sentence"));
    assert!(json["reference"]
        .as_str()
        .unwrap()
        .starts_with("ctxc://context/"));
    assert_eq!(json["result"]["optimizer"], "text");
    assert_eq!(json["stored"], true);

    let result = &json["result"];
    let saved =
        result["original_tokens"].as_u64().unwrap() - result["optimized_tokens"].as_u64().unwrap();
    let stages = &result["savings_by_stage"];
    let attributed: u64 = ["filtering", "deduplication", "compression", "selection"]
        .iter()
        .map(|stage| stages[stage].as_u64().unwrap())
        .sum();
    assert_eq!(saved, attributed, "every saved token must be attributed");
}

#[test]
fn optimize_honours_an_explicit_budget() {
    let sandbox = Sandbox::new("optimize-budget");
    let input = sandbox.path("long.txt");
    let long: String = (0..60)
        .map(|index| format!("paragraph {index} carrying a handful of words\n\n"))
        .collect();
    std::fs::write(&input, long).unwrap();

    let output = sandbox.run(&[
        "optimize",
        path_str(&input),
        "--budget",
        "40",
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert!(json["result"]["optimized_tokens"].as_u64().unwrap() <= 40);
    assert!(
        json["result"]["savings_by_stage"]["selection"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[test]
fn optimize_keeps_the_original_retrievable() {
    let sandbox = Sandbox::new("optimize-store");
    let input = sandbox.path("notes.txt");
    std::fs::write(&input, SAMPLE).unwrap();

    assert_success(&sandbox.run(&["optimize", path_str(&input)]));

    let status = sandbox.run(&["status", "--format", "json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&status)).unwrap();
    assert_eq!(
        json["database"]["contexts"], 1,
        "the original must be stored"
    );
}

#[test]
fn no_store_skips_the_database() {
    let sandbox = Sandbox::new("optimize-no-store");
    let input = sandbox.path("notes.txt");
    std::fs::write(&input, SAMPLE).unwrap();

    let output = sandbox.run(&[
        "optimize",
        path_str(&input),
        "--no-store",
        "--format",
        "json",
    ]);
    assert_success(&output);
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["stored"], false);

    let status = sandbox.run(&["status", "--format", "json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&status)).unwrap();
    assert_eq!(json["database"]["contexts"], 0);
}

#[test]
fn optimize_is_pipe_safe_in_quiet_mode() {
    let sandbox = Sandbox::new("optimize-quiet");
    let input = sandbox.path("notes.txt");
    std::fs::write(&input, SAMPLE).unwrap();

    let output = sandbox.run(&["optimize", path_str(&input), "--format", "quiet"]);
    assert_success(&output);
    assert!(stdout(&output).contains("A different sentence"));
    assert_eq!(stderr(&output), "");
}

#[test]
fn compile_joins_several_inputs() {
    let sandbox = Sandbox::new("compile");
    let first = sandbox.path("first.txt");
    let second = sandbox.path("second.md");
    std::fs::write(&first, "alpha content\n").unwrap();
    std::fs::write(&second, "# beta\n\nbeta content\n").unwrap();

    let output = sandbox.run(&["compile", path_str(&first), path_str(&second)]);
    assert_success(&output);

    let content = stdout(&output);
    assert!(content.contains("=== "), "{content}");
    assert!(content.contains("alpha content"), "{content}");
    assert!(content.contains("beta content"), "{content}");
    assert!(content.contains("ctxc://context/"), "{content}");

    let summary = stderr(&output);
    assert!(summary.contains("Sections:"), "{summary}");
}

#[test]
fn compile_json_lists_every_section() {
    let sandbox = Sandbox::new("compile-json");
    let first = sandbox.path("first.txt");
    let second = sandbox.path("second.txt");
    std::fs::write(&first, "alpha\n").unwrap();
    std::fs::write(&second, "beta\n").unwrap();

    let output = sandbox.run(&[
        "compile",
        path_str(&first),
        path_str(&second),
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["sections"].as_array().unwrap().len(), 2);
    assert_eq!(json["budget"], 32000);
}

#[test]
fn code_is_optimized_conservatively() {
    let sandbox = Sandbox::new("optimize-code");
    let input = sandbox.path("main.rs");
    std::fs::write(
        &input,
        "fn a() {\n    log();\n}\n\nfn b() {\n    log();\n}\n\nfn a() {\n    log();\n}\n",
    )
    .unwrap();

    let output = sandbox.run(&["optimize", path_str(&input), "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["result"]["optimizer"], "text-conservative");
    assert_eq!(
        json["result"]["savings_by_stage"]["deduplication"], 0,
        "duplicate code blocks must survive"
    );
    assert_eq!(
        json["content"].as_str().unwrap().matches("fn a()").count(),
        2
    );
}

#[test]
fn a_missing_input_fails_with_an_actionable_error() {
    let sandbox = Sandbox::new("optimize-missing");
    let output = sandbox.run(&["optimize", "no-such-file.txt"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    let message = stderr(&output);
    assert!(message.contains("no such file"), "{message}");
    assert!(message.contains("Try:"), "{message}");
}

#[test]
fn a_directory_input_points_at_indexing() {
    let sandbox = Sandbox::new("optimize-directory");
    let directory = sandbox.path("project");
    std::fs::create_dir_all(&directory).unwrap();

    let output = sandbox.run(&["analyze", path_str(&directory)]);
    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("is a directory"), "{message}");
    assert!(message.contains("pass a single file"), "{message}");
}

#[test]
fn binary_input_is_refused() {
    let sandbox = Sandbox::new("optimize-binary");
    let input = sandbox.path("blob.bin");
    std::fs::write(&input, [0x00u8, 0x01, 0x02, 0x03]).unwrap();

    let output = sandbox.run(&["optimize", path_str(&input)]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("is not text"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn stdin_is_optimized_when_no_input_is_given() {
    let sandbox = Sandbox::new("stdin");
    let output = sandbox.run_with_stdin(&["optimize"], SAMPLE);
    assert_success(&output);

    assert_eq!(
        stdout(&output),
        "The service refused the connection.\n\nA different sentence that carries new information.\n"
    );
}

#[test]
fn the_dash_marker_also_means_stdin() {
    let sandbox = Sandbox::new("stdin-dash");
    let output = sandbox.run_with_stdin(&["analyze", "-", "--format", "json"], SAMPLE);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["source"], "<stdin>");
    assert_eq!(json["duplicate_fragments"], 1);
}

#[test]
fn piped_logs_are_collapsed_with_their_counts() {
    let sandbox = Sandbox::new("stdin-log");
    let mut log: String = (0..200)
        .map(|index| format!("2026-08-19 09:30:{index:02} WARN deprecated api used\n"))
        .collect();
    log.push_str("2026-08-19 09:41:12 ERROR build failed: missing symbol\n");

    let output = sandbox.run_with_stdin(&["optimize", "--format", "json"], &log);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["result"]["optimizer"], "log");

    let content = json["content"].as_str().unwrap();
    assert!(content.contains("(repeated 200 times)"), "{content}");
    assert!(
        content.contains("ERROR build failed: missing symbol"),
        "the one line that matters must survive: {content}"
    );
    assert!(json["result"]["reduction_ratio"].as_f64().unwrap() > 0.9);
}

#[test]
fn piped_json_is_minified_without_losing_data() {
    let sandbox = Sandbox::new("stdin-json");
    let pretty = "{\n  \"name\": \"ctxc\",\n  \"tags\": [\n    \"one\",\n    \"two\"\n  ]\n}\n";

    let output = sandbox.run_with_stdin(&["optimize", "--format", "json"], pretty);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["result"]["optimizer"], "json");

    let optimized: serde_json::Value = serde_json::from_str(json["content"].as_str().unwrap())
        .expect("optimized json is still json");
    assert_eq!(
        optimized,
        serde_json::from_str::<serde_json::Value>(pretty).unwrap()
    );
    assert!(
        json["result"]["savings_by_stage"]["compression"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[test]
fn ansi_escapes_are_stripped_from_terminal_output() {
    let sandbox = Sandbox::new("stdin-ansi");
    let colored = "\u{1b}[32mPASS\u{1b}[0m first check\n\u{1b}[31mFAIL\u{1b}[0m second check\n";

    let output = sandbox.run_with_stdin(&["optimize"], colored);
    assert_success(&output);
    assert_eq!(stdout(&output), "PASS first check\nFAIL second check\n");
}

#[test]
fn piped_input_can_be_attributed_to_the_command_that_produced_it() {
    let sandbox = Sandbox::new("stdin-from");
    let status = "On branch main\n\
                  Changes not staged for commit:\n\
                  \x20 (use \"git add <file>...\" to update what will be committed)\n\
                  \tmodified:   src/main.rs\n";

    let unlabelled = sandbox.run_with_stdin(&["optimize", "--format", "json"], status);
    assert_success(&unlabelled);
    let json: serde_json::Value = serde_json::from_str(&stdout(&unlabelled)).unwrap();
    assert_ne!(json["result"]["optimizer"], "git");

    let labelled = sandbox.run_with_stdin(
        &["optimize", "--from", "git status", "--format", "json"],
        status,
    );
    assert_success(&labelled);
    let json: serde_json::Value = serde_json::from_str(&stdout(&labelled)).unwrap();

    assert_eq!(json["result"]["optimizer"], "git");
    let content = json["content"].as_str().unwrap();
    assert!(content.contains("modified:   src/main.rs"));
    assert!(!content.contains("(use \"git add"), "{content}");
}

#[test]
fn capture_runs_a_command_and_optimizes_its_output() {
    let sandbox = Sandbox::new("capture");
    let ctxc = env!("CARGO_BIN_EXE_ctxc");

    let output = sandbox.run(&["capture", "--format", "json", "--", ctxc, "version"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["exit_code"], 0);
    assert!(json["source"].as_str().unwrap().contains("version"));
    assert!(json["content"].as_str().unwrap().contains("ctxc "));
    assert_eq!(json["result"]["optimizer"], "tool");
}

#[test]
fn capture_reports_a_failing_command_without_failing_itself() {
    let sandbox = Sandbox::new("capture-failing");
    let ctxc = env!("CARGO_BIN_EXE_ctxc");

    let output = sandbox.run(&[
        "capture",
        "--format",
        "json",
        "--",
        ctxc,
        "optimize",
        "no-such-file.txt",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["exit_code"], 1);
    assert!(json["content"].as_str().unwrap().contains("no such file"));
}

#[test]
fn capture_reports_a_missing_command_with_a_hint() {
    let sandbox = Sandbox::new("capture-missing");
    let output = sandbox.run(&["capture", "--", "ctxc-no-such-program-exists"]);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("command not found"), "{message}");
    assert!(message.contains("Try:"), "{message}");
}

#[test]
fn captured_output_keeps_the_command_as_its_source() {
    let sandbox = Sandbox::new("capture-source");
    let ctxc = env!("CARGO_BIN_EXE_ctxc");

    assert_success(&sandbox.run(&["capture", "--", ctxc, "version"]));

    let status = sandbox.run(&["status", "--format", "json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&status)).unwrap();
    assert_eq!(
        json["database"]["contexts"], 1,
        "captured output is stored like any other context"
    );
}

/// A small TypeScript project with one real dependency edge.
fn sample_project(sandbox: &Sandbox) -> PathBuf {
    let project = sandbox.path("project");
    let write = |relative: &str, contents: &str| {
        let path = project.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    };

    write(
        "src/database.ts",
        "export class Database {\n  query(sql: string) {}\n}\n",
    );
    write(
        "src/auth.ts",
        "import { Database } from './database';\nimport express from 'express';\n\n\
         export function authenticate(token: string) {\n  return new Database().query(token);\n}\n",
    );
    write("node_modules/react/index.js", "module.exports = {};\n");
    write("README.md", "# sample\n");
    project
}

#[test]
fn status_counts_indexed_projects() {
    let sandbox = Sandbox::new("status-indexed");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["index", path_str(&project)]));

    let output = sandbox.run(&["status", "--format", "json"]);
    assert_success(&output);
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["indexed_roots"], 1);
}

#[test]
fn index_reports_what_it_did() {
    let sandbox = Sandbox::new("index");
    let project = sample_project(&sandbox);

    let output = sandbox.run(&["index", path_str(&project), "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["scanned"], 3, "node_modules must not be scanned");
    assert_eq!(json["indexed"], 3);
    assert_eq!(json["unparsed"], 1);
    assert!(json["symbols"].as_u64().unwrap() >= 3);
    assert!(json["ignored"].as_u64().unwrap() >= 1);
}

#[test]
fn re_indexing_skips_unchanged_files() {
    let sandbox = Sandbox::new("index-incremental");
    let project = sample_project(&sandbox);

    assert_success(&sandbox.run(&["index", path_str(&project)]));
    let second = sandbox.run(&["index", path_str(&project), "--format", "json"]);
    assert_success(&second);

    let json: serde_json::Value = serde_json::from_str(&stdout(&second)).unwrap();
    assert_eq!(json["indexed"], 0);
    assert_eq!(json["unchanged"], 3);
}

#[test]
fn search_finds_indexed_symbols() {
    let sandbox = Sandbox::new("search");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["index", path_str(&project)]));

    let output = sandbox.run(&[
        "search",
        "authenticate",
        "--path",
        path_str(&project),
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let files = json["files"].as_array().unwrap();
    assert_eq!(files[0]["path"], "src/auth.ts");
    assert_eq!(files[0]["language"], "typescript");
    assert_eq!(files[0]["matched_symbols"][0], "authenticate");
    assert!(files[0]["line"].as_u64().unwrap() >= 1);
    assert!(files[0]["score"].as_f64().unwrap() > 0.0);
}

#[test]
fn search_says_so_when_nothing_matches() {
    let sandbox = Sandbox::new("search-empty");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["index", path_str(&project)]));

    let output = sandbox.run(&["search", "nonexistent", "--path", path_str(&project)]);
    assert_success(&output);
    assert!(stdout(&output).contains("matches"), "{}", stdout(&output));
}

#[test]
fn graph_summarises_the_project() {
    let sandbox = Sandbox::new("graph");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["index", path_str(&project)]));

    let output = sandbox.run(&["graph", path_str(&project), "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["edges"], 1);
    let ranked = json["most_depended_on"].as_array().unwrap();
    assert_eq!(ranked[0]["path"], "src/database.ts");
    assert_eq!(ranked[0]["dependents"], 1);
}

#[test]
fn graph_shows_one_files_neighbourhood() {
    let sandbox = Sandbox::new("graph-file");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["index", path_str(&project)]));

    let output = sandbox.run(&[
        "graph",
        path_str(&project),
        "--file",
        "src/auth.ts",
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let file = &json["file"];
    assert_eq!(file["dependencies"][0], "src/database.ts");
    assert_eq!(
        file["unresolved_imports"][0], "express",
        "packages stay listed as written"
    );
}

#[test]
fn graph_refuses_an_unindexed_project() {
    let sandbox = Sandbox::new("graph-unindexed");
    let project = sample_project(&sandbox);

    let output = sandbox.run(&["graph", path_str(&project)]);
    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("has not been indexed"), "{message}");
    assert!(message.contains("ctxc project index"), "{message}");
}

#[test]
fn deleting_a_file_removes_it_from_the_index() {
    let sandbox = Sandbox::new("index-delete");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["index", path_str(&project)]));

    std::fs::remove_file(project.join("src").join("database.ts")).unwrap();
    let output = sandbox.run(&["index", path_str(&project), "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["removed"], 1);

    let search = sandbox.run(&[
        "search",
        "Database",
        "--path",
        path_str(&project),
        "--format",
        "json",
    ]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&search)).unwrap();
    let paths: Vec<&str> = json["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert!(
        !paths.contains(&"src/database.ts"),
        "the deleted file must be gone from the index; got {paths:?}"
    );
}

#[test]
fn indexing_a_missing_directory_fails_clearly() {
    let sandbox = Sandbox::new("index-missing");
    let output = sandbox.run(&["index", "no-such-directory-here"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    assert!(stderr(&output).contains("Try:"), "{}", stderr(&output));
}

#[test]
fn optimization_can_be_disabled_in_configuration() {
    let sandbox = Sandbox::new("optimize-disabled");
    std::fs::write(
        sandbox.path("config.toml"),
        "[optimization]\nenabled = false\n",
    )
    .unwrap();
    let input = sandbox.path("notes.txt");
    std::fs::write(&input, SAMPLE).unwrap();

    let output = sandbox.run(&["optimize", path_str(&input), "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["result"]["optimizer"], "passthrough");
    assert_eq!(
        json["result"]["original_tokens"],
        json["result"]["optimized_tokens"]
    );
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("test paths are utf-8")
}

#[test]
fn search_finds_files_by_content_and_follows_the_graph() {
    let sandbox = Sandbox::new("search-hybrid");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["index", path_str(&project)]));

    let output = sandbox.run(&[
        "search",
        "authenticate",
        "--path",
        path_str(&project),
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let files = json["files"].as_array().unwrap();
    assert_eq!(files[0]["path"], "src/auth.ts");

    let related = files
        .iter()
        .find(|file| file["path"] == "src/database.ts")
        .expect("the file auth.ts imports must be pulled in by graph expansion");
    assert_eq!(related["reason"]["kind"], "related");
    assert_eq!(related["reason"]["to"], "src/auth.ts");
    assert!(
        related["score"].as_f64().unwrap() < files[0]["score"].as_f64().unwrap(),
        "an expanded file ranks below the match it came from"
    );
}

#[test]
fn search_can_compile_the_context_it_selected() {
    let sandbox = Sandbox::new("search-compile");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["index", path_str(&project)]));

    let output = sandbox.run(&[
        "search",
        "authenticate",
        "--path",
        path_str(&project),
        "--compile",
        "--budget",
        "2000",
    ]);
    assert_success(&output);

    let content = stdout(&output);
    assert!(content.contains("=== "), "sections are labelled: {content}");
    assert!(content.contains("authenticate"), "{content}");
    assert!(content.contains("ctxc://context/"), "with references");

    let summary = stderr(&output);
    assert!(summary.contains("Files selected:"), "{summary}");
    assert!(summary.contains("of 2,000 budget"), "{summary}");
}

#[test]
fn compiled_context_respects_its_budget() {
    let sandbox = Sandbox::new("search-compile-budget");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["index", path_str(&project)]));

    let output = sandbox.run(&[
        "search",
        "authenticate",
        "--path",
        path_str(&project),
        "--compile",
        "--budget",
        "120",
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert!(json["optimized_tokens"].as_u64().unwrap() <= 120);
}

#[test]
fn search_refuses_an_unindexed_project() {
    let sandbox = Sandbox::new("search-unindexed");
    let project = sample_project(&sandbox);

    let output = sandbox.run(&["search", "anything", "--path", path_str(&project)]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("ctxc project index"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_optimized_context_can_be_retrieved_in_full() {
    let sandbox = Sandbox::new("retrieve");
    let input = sandbox.path("notes.txt");
    std::fs::write(&input, SAMPLE).unwrap();

    let optimized = sandbox.run(&["optimize", path_str(&input), "--format", "json"]);
    assert_success(&optimized);
    let json: serde_json::Value = serde_json::from_str(&stdout(&optimized)).unwrap();
    let reference = json["reference"].as_str().unwrap().to_string();
    assert!(
        json["content"].as_str().unwrap().len() < SAMPLE.len(),
        "the optimized form is smaller"
    );

    let recovered = sandbox.run(&["retrieve", &reference]);
    assert_success(&recovered);
    assert_eq!(
        stdout(&recovered),
        SAMPLE,
        "retrieval returns the original byte for byte"
    );
    assert!(
        stderr(&recovered).contains("Reference:"),
        "{}",
        stderr(&recovered)
    );
}

#[test]
fn retrieve_accepts_a_bare_id_and_reports_metadata() {
    let sandbox = Sandbox::new("retrieve-id");
    let input = sandbox.path("notes.txt");
    std::fs::write(&input, SAMPLE).unwrap();

    let optimized = sandbox.run(&["optimize", path_str(&input), "--format", "json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&optimized)).unwrap();
    let id = json["reference"]
        .as_str()
        .unwrap()
        .trim_start_matches("ctxc://context/")
        .to_string();

    let recovered = sandbox.run(&["retrieve", &id, "--format", "json"]);
    assert_success(&recovered);

    let json: serde_json::Value = serde_json::from_str(&stdout(&recovered)).unwrap();
    assert_eq!(json["content"], SAMPLE);
    assert_eq!(json["content_type"], "plain_text");
    assert!(json["source"].as_str().unwrap().contains("notes.txt"));
}

#[test]
fn retrieving_an_unknown_reference_fails_with_a_hint() {
    let sandbox = Sandbox::new("retrieve-missing");
    let output = sandbox.run(&["retrieve", &format!("ctxc://context/{}", "0".repeat(32))]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    let message = stderr(&output);
    assert!(message.contains("no context is stored"), "{message}");
    assert!(message.contains("Try:"), "{message}");
}

#[test]
fn a_malformed_reference_is_rejected() {
    let sandbox = Sandbox::new("retrieve-malformed");
    let output = sandbox.run(&["retrieve", "not-a-reference"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("32 lowercase hex"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn ranking_weights_come_from_configuration() {
    let sandbox = Sandbox::new("search-weights");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["index", path_str(&project)]));

    // With expansion switched off, the imported file stops being retrieved.
    std::fs::write(
        sandbox.path("config.toml"),
        "[ranking]\nexpansion_depth = 0\n",
    )
    .unwrap();

    let output = sandbox.run(&[
        "search",
        "authenticate",
        "--path",
        path_str(&project),
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let paths: Vec<&str> = json["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();

    assert!(paths.contains(&"src/auth.ts"));
    assert!(!paths.contains(&"src/database.ts"));
}

#[test]
fn compiled_context_references_can_be_retrieved() {
    let sandbox = Sandbox::new("search-compile-retrieve");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["index", path_str(&project)]));

    let compiled = sandbox.run(&[
        "search",
        "authenticate",
        "--path",
        path_str(&project),
        "--compile",
        "--format",
        "json",
    ]);
    assert_success(&compiled);

    let json: serde_json::Value = serde_json::from_str(&stdout(&compiled)).unwrap();
    assert_eq!(json["stored"], true);

    let content = json["content"].as_str().unwrap();
    let reference = content
        .split_once("ctxc://context/")
        .map(|(_, rest)| format!("ctxc://context/{}", &rest[..32]))
        .expect("the document cites a reference");

    let recovered = sandbox.run(&["retrieve", &reference, "--format", "json"]);
    assert_success(&recovered);

    let json: serde_json::Value = serde_json::from_str(&stdout(&recovered)).unwrap();
    assert!(
        json["content"].as_str().unwrap().contains("authenticate"),
        "a reference printed in compiled output must resolve"
    );
}

// ---------------------------------------------------------------------------
// Project registry
// ---------------------------------------------------------------------------

#[test]
fn a_project_can_be_registered_and_listed() {
    let sandbox = Sandbox::new("project-add");
    let project = sample_project(&sandbox);
    std::fs::write(
        project.join("package.json"),
        "{\"dependencies\":{\"react\":\"18\"}}",
    )
    .unwrap();

    let output = sandbox.run(&["project", "add", path_str(&project), "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["name"], "project");
    assert_eq!(json["status"], "active");
    assert_eq!(json["frameworks"][0], "react");
    assert_eq!(json["indexed_files"], 0, "adding does not index by default");

    let listed = sandbox.run(&["project", "list"]);
    assert_success(&listed);
    assert!(stdout(&listed).contains("project"), "{}", stdout(&listed));
}

#[test]
fn adding_a_project_twice_is_harmless() {
    let sandbox = Sandbox::new("project-twice");
    let project = sample_project(&sandbox);

    let first = sandbox.run(&["project", "add", path_str(&project), "--format", "json"]);
    let second = sandbox.run(&["project", "add", path_str(&project), "--format", "json"]);
    assert_success(&second);

    let first: serde_json::Value = serde_json::from_str(&stdout(&first)).unwrap();
    let second: serde_json::Value = serde_json::from_str(&stdout(&second)).unwrap();
    assert_eq!(first["id"], second["id"], "the identity is kept");
}

#[test]
fn adding_with_index_makes_the_project_searchable_at_once() {
    let sandbox = Sandbox::new("project-index");
    let project = sample_project(&sandbox);

    let output = sandbox.run(&[
        "project",
        "add",
        path_str(&project),
        "--index",
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert!(json["indexed_files"].as_u64().unwrap() >= 3);
    assert!(json["last_indexed_at"].is_string());

    let search = sandbox.run(&[
        "search",
        "authenticate",
        "--path",
        path_str(&project),
        "--format",
        "json",
    ]);
    assert_success(&search);
    let json: serde_json::Value = serde_json::from_str(&stdout(&search)).unwrap();
    assert!(!json["files"].as_array().unwrap().is_empty());
}

#[test]
fn projects_can_be_paused_and_resumed() {
    let sandbox = Sandbox::new("project-pause");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["project", "add", path_str(&project)]));

    let paused = sandbox.run(&["project", "pause", "project", "--format", "json"]);
    assert_success(&paused);
    let json: serde_json::Value = serde_json::from_str(&stdout(&paused)).unwrap();
    assert_eq!(json["status"], "paused");

    let resumed = sandbox.run(&["project", "resume", "project", "--format", "json"]);
    assert_success(&resumed);
    let json: serde_json::Value = serde_json::from_str(&stdout(&resumed)).unwrap();
    assert_eq!(json["status"], "active");
}

#[test]
fn removing_a_project_keeps_its_files() {
    let sandbox = Sandbox::new("project-remove");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["project", "add", path_str(&project)]));

    assert_success(&sandbox.run(&["project", "remove", "project"]));
    let listed = sandbox.run(&["project", "list"]);
    assert!(
        stdout(&listed).contains("No projects registered"),
        "{}",
        stdout(&listed)
    );
    assert!(
        project.join("src").join("auth.ts").exists(),
        "removing from the registry must never delete files"
    );
}

#[test]
fn an_unknown_project_is_reported_with_a_hint() {
    let sandbox = Sandbox::new("project-unknown");
    let output = sandbox.run(&["project", "pause", "not-a-project"]);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("no project matches"), "{message}");
    assert!(message.contains("ctxc project list"), "{message}");
}

#[test]
fn project_open_prints_the_path_for_a_shell_to_use() {
    let sandbox = Sandbox::new("project-open");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["project", "add", path_str(&project)]));

    let output = sandbox.run(&["project", "open", "project"]);
    assert_success(&output);
    assert!(
        stdout(&output).trim().ends_with("project"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_project_declaring_its_own_identity_keeps_it() {
    let sandbox = Sandbox::new("project-identity");
    let project = sample_project(&sandbox);
    std::fs::write(
        project.join(".ctxc.toml"),
        "[project]\nid = \"stable-id\"\nname = \"Renamed\"\n",
    )
    .unwrap();

    let output = sandbox.run(&["project", "add", path_str(&project), "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["id"], "stable-id");
    assert_eq!(json["name"], "Renamed");
}

// ---------------------------------------------------------------------------
// Daemon lifecycle
// ---------------------------------------------------------------------------

#[test]
fn daemon_status_reports_nothing_running() {
    let sandbox = Sandbox::new("daemon-stopped");
    let output = sandbox.run(&["daemon", "status", "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["running"], false);
    assert_eq!(json["stale"], false);
}

#[test]
fn stopping_nothing_fails_with_a_hint() {
    let sandbox = Sandbox::new("daemon-stop-nothing");
    let output = sandbox.run(&["stop"]);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("no daemon is running"), "{message}");
    assert!(message.contains("ctxc start"), "{message}");
}

#[test]
fn a_lockfile_left_behind_is_reported_and_can_be_cleared() {
    let sandbox = Sandbox::new("daemon-stale");
    // Port 1 on loopback has nothing behind it, so this lock is stale.
    std::fs::write(
        sandbox.path("daemon.lock"),
        "{\"pid\":999999,\"port\":1,\"bind\":\"127.0.0.1\",\
         \"token\":\"x\",\"started_at\":1,\"version\":\"0.1.0\"}",
    )
    .unwrap();

    let status = sandbox.run(&["daemon", "status", "--format", "json"]);
    assert_success(&status);
    let json: serde_json::Value = serde_json::from_str(&stdout(&status)).unwrap();
    assert_eq!(json["running"], false);
    assert_eq!(json["stale"], true, "a crash must be visible, not hidden");

    assert_success(&sandbox.run(&["stop"]));
    assert!(!sandbox.path("daemon.lock").exists());
}

#[test]
fn a_running_daemon_serves_the_api_and_stops_cleanly() {
    let sandbox = Sandbox::with_daemon("daemon-lifecycle");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["project", "add", path_str(&project), "--index"]));

    let mut daemon = sandbox.spawn_daemon();

    // The lockfile is what a client uses to find and authenticate to it.
    let lock: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(sandbox.path("daemon.lock")).unwrap())
            .unwrap();
    assert!(lock["port"].as_u64().unwrap() > 0, "a port was chosen");
    assert!(!lock["token"].as_str().unwrap().is_empty(), "and a token");

    let status = sandbox.run(&["daemon", "status", "--format", "json"]);
    assert_success(&status);
    let json: serde_json::Value = serde_json::from_str(&stdout(&status)).unwrap();
    assert_eq!(json["running"], true);
    assert_eq!(json["projects"], 1);
    assert!(json["indexed_files"].as_u64().unwrap() >= 3);

    assert_success(&sandbox.run(&["stop"]));
    let exit = daemon.wait().expect("the daemon exits when asked");
    assert!(
        exit.success(),
        "a daemon that was asked to stop exits cleanly"
    );

    assert!(
        !sandbox.path("daemon.lock").exists(),
        "a clean stop removes its own lockfile"
    );
    let after = sandbox.run(&["daemon", "status", "--format", "json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&after)).unwrap();
    assert_eq!(json["running"], false);
}

#[test]
fn stopping_ends_the_other_ctxc_processes_too() {
    let sandbox = Sandbox::new("stop-everything");

    // An MCP server is the case this exists for: an agent spawns one, the agent
    // goes away, and nothing is left that knows how to find it. Its stdin stays
    // open so it blocks on the protocol rather than exiting on its own.
    let mut server = sandbox
        .command(&["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the MCP server");

    let recorded = wait_for_records(&sandbox, 1);
    assert_eq!(recorded.len(), 1, "the MCP server records itself");
    assert_eq!(recorded[0]["command"], "mcp");

    // No daemon is running, so stopping the MCP server is the whole result.
    let output = sandbox.run(&["stop"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Stopped 1"), "{}", stdout(&output));

    let exit = server.wait().expect("the MCP server is gone");
    assert!(!exit.success(), "it was terminated rather than asked");
    assert!(
        records(&sandbox).is_empty(),
        "a stopped process leaves no record behind"
    );
}

#[test]
fn stopping_clears_records_of_processes_that_are_already_gone() {
    let sandbox = Sandbox::new("stop-stale-records");
    std::fs::create_dir_all(sandbox.path("processes")).unwrap();
    // A pid that is not running CtxC must never be signalled, whatever the
    // record says — pids get reused, and this one belongs to somebody else.
    std::fs::write(
        sandbox.path("processes").join("999999.json"),
        "{\"pid\":999999,\"command\":\"mcp\",\
         \"executable\":\"ctxc-not-a-real-binary\",\"started_at\":1}",
    )
    .unwrap();

    let output = sandbox.run(&["stop"]);
    assert_success(&output);
    assert!(stdout(&output).contains("Cleared 1"), "{}", stdout(&output));
    assert!(records(&sandbox).is_empty());
}

#[test]
fn a_running_daemon_is_stopped_cleanly_even_when_it_is_not_alone() {
    let sandbox = Sandbox::with_daemon("stop-daemon-and-server");
    let mut daemon = sandbox.spawn_daemon();

    let mut server = sandbox
        .command(&["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the MCP server");
    wait_for_records(&sandbox, 2);

    let output = sandbox.run(&["stop", "--format", "json"]);
    assert_success(&output);
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert!(json["daemon"].as_u64().unwrap() > 0, "the daemon is named");
    assert_eq!(json["terminated"].as_array().unwrap().len(), 1);
    assert_eq!(json["terminated"][0]["command"], "mcp");

    // The daemon was asked over its API, not killed, so it still exits cleanly.
    let exit = daemon.wait().expect("the daemon exits when asked");
    assert!(exit.success(), "the daemon is asked to stop, never killed");
    let _ = server.wait();

    assert!(!sandbox.path("daemon.lock").exists());
    assert!(records(&sandbox).is_empty());
}

#[test]
fn the_daemon_reports_what_stopping_it_would_leave_running() {
    let sandbox = Sandbox::with_daemon("api-processes");
    let mut daemon = sandbox.spawn_daemon();
    let client = daemon_client(&sandbox);

    // With only the daemon, stopping it really does stop all of CtxC.
    let alone: serde_json::Value = client.get("/v1/processes").expect("list processes");
    assert_eq!(alone["others"], 0);
    assert_eq!(alone["stop_command"], "ctxc stop");
    assert_eq!(alone["processes"][0]["is_daemon"], true);

    let mut server = sandbox
        .command(&["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the MCP server");
    wait_for_records(&sandbox, 2);

    // The dashboard needs this to tell someone what its Stop button misses.
    let crowded: serde_json::Value = client.get("/v1/processes").expect("list processes");
    assert_eq!(crowded["others"], 1);
    let others: Vec<&serde_json::Value> = crowded["processes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|process| process["is_daemon"] == false)
        .collect();
    assert_eq!(others.len(), 1);
    assert_eq!(others[0]["command"], "mcp");

    // Shutting down says the same thing, so a caller that never reads the list
    // still learns it did not stop everything.
    let stopping: serde_json::Value = client
        .post("/v1/shutdown", &serde_json::json!({}))
        .expect("ask the daemon to stop");
    assert_eq!(stopping["stopping"], true);
    assert_eq!(stopping["still_running"], 1);
    assert_eq!(stopping["stop_command"], "ctxc stop");

    let exit = daemon.wait().expect("the daemon exits when asked");
    assert!(exit.success());

    // And it really is still running: the API only ever looked.
    assert_eq!(
        records(&sandbox).len(),
        1,
        "the MCP server outlived the daemon"
    );
    assert_success(&sandbox.run(&["stop"]));
    let _ = server.wait();
}

/// The processes CtxC has recorded for a sandbox.
fn records(sandbox: &Sandbox) -> Vec<serde_json::Value> {
    let Ok(listing) = std::fs::read_dir(sandbox.path("processes")) else {
        return Vec::new();
    };
    listing
        .flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| serde_json::from_str(&text).ok())
        .collect()
}

/// Wait for `count` processes to have recorded themselves.
fn wait_for_records(sandbox: &Sandbox, count: usize) -> Vec<serde_json::Value> {
    for _ in 0..100 {
        let found = records(sandbox);
        if found.len() >= count {
            return found;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("{count} process record(s) never appeared");
}

#[test]
fn starting_detached_does_not_hold_its_caller_open() {
    let sandbox = Sandbox::with_daemon("daemon-detach-pipes");

    // `run` captures stdout and stderr through pipes. A detached daemon that
    // inherits those handles keeps them open for its whole life, and this call
    // never returns — on Windows, where CreateProcess inherits every
    // inheritable handle rather than only the three it is given.
    let output = sandbox.run(&["start", "--detach", "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["started"], true);
    assert!(json["port"].as_u64().unwrap() > 0);

    assert_success(&sandbox.run(&["stop"]));
}

#[test]
fn a_second_daemon_refuses_to_start() {
    let sandbox = Sandbox::with_daemon("daemon-double");
    let mut daemon = sandbox.spawn_daemon();

    let second = sandbox.run(&["start", "--detach"]);
    assert_eq!(second.status.code(), Some(1));
    assert!(
        stderr(&second).contains("already running"),
        "{}",
        stderr(&second)
    );

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

#[test]
fn the_daemon_indexes_registered_projects_on_its_own() {
    let sandbox = Sandbox::with_daemon("daemon-indexing");
    std::fs::write(
        sandbox.path("config.toml"),
        "[daemon]\nport = 0\n[ranking]\nexpansion_depth = 0\n",
    )
    .unwrap();

    // Registered without --index: nothing has read this project yet.
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["project", "add", path_str(&project)]));

    let listed = sandbox.run(&["project", "list", "--format", "json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&listed)).unwrap();
    assert_eq!(json["projects"][0]["indexed_files"], 0);

    let mut daemon = sandbox.spawn_daemon();

    // The daemon indexes on startup, so the project becomes searchable without
    // anyone asking for it.
    let mut indexed = 0;
    for _ in 0..100 {
        let listed = sandbox.run(&["project", "list", "--format", "json"]);
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout(&listed)) {
            indexed = json["projects"][0]["indexed_files"].as_u64().unwrap_or(0);
            if indexed > 0 {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");

    assert!(
        indexed >= 3,
        "the daemon should have indexed the project by itself, saw {indexed} files"
    );
}

#[test]
fn status_reports_a_running_daemon() {
    let sandbox = Sandbox::with_daemon("status-daemon");
    let mut daemon = sandbox.spawn_daemon();

    let output = sandbox.run(&["status", "--format", "json"]);
    assert_success(&output);
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["daemon"]["running"], true);
    assert!(json["daemon"]["port"].as_u64().unwrap() > 0);

    let human = sandbox.run(&["status"]);
    assert!(
        stdout(&human).contains("Daemon:     running"),
        "{}",
        stdout(&human)
    );

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

// ---------------------------------------------------------------------------
// Continuous mode
// ---------------------------------------------------------------------------

/// Wait for a condition to hold, polling briefly.
///
/// Watching is inherently asynchronous: the assertion is that CtxC gets there,
/// not that it gets there within one particular tick. The budget is generous
/// because these tests run in parallel with every other suite in the
/// workspace, and a loaded machine must not read as a broken watcher.
fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..300 {
        if check() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("timed out waiting for {what}");
}

/// A sandbox whose daemon watches with a short debounce.
fn watching_sandbox(name: &str) -> Sandbox {
    let sandbox = Sandbox::new(name);
    std::fs::write(
        sandbox.path("config.toml"),
        "[daemon]\nport = 0\n[watch]\ndebounce_ms = 100\n",
    )
    .unwrap();
    sandbox
}

/// Whether a symbol is currently findable in a project.
fn finds_symbol(sandbox: &Sandbox, project: &Path, symbol: &str) -> bool {
    let output = sandbox.run(&[
        "search",
        symbol,
        "--path",
        path_str(project),
        "--format",
        "jsonl",
    ]);
    serde_json::from_str::<serde_json::Value>(&stdout(&output))
        .ok()
        .and_then(|json| {
            let files = json["files"].as_array()?.clone();
            Some(files.iter().any(|file| {
                file["matched_symbols"]
                    .as_array()
                    .is_some_and(|symbols| symbols.iter().any(|name| name == symbol))
            }))
        })
        .unwrap_or(false)
}

#[test]
fn the_daemon_reports_which_projects_it_is_watching() {
    let sandbox = watching_sandbox("watch-status");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["project", "add", path_str(&project), "--index"]));

    let mut daemon = sandbox.spawn_daemon();
    eventually("the project to be watched", || {
        let output = sandbox.run(&["daemon", "status", "--format", "jsonl"]);
        serde_json::from_str::<serde_json::Value>(&stdout(&output))
            .map(|json| json["watching"] == 1)
            .unwrap_or(false)
    });

    let human = sandbox.run(&["status"]);
    assert!(
        stdout(&human).contains("Watching:   1 project(s)"),
        "{}",
        stdout(&human)
    );

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

#[test]
fn a_saved_file_becomes_searchable_without_anyone_asking() {
    let sandbox = watching_sandbox("watch-modify");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["project", "add", path_str(&project), "--index"]));

    let mut daemon = sandbox.spawn_daemon();
    eventually("watching to start", || {
        let output = sandbox.run(&["daemon", "status", "--format", "jsonl"]);
        serde_json::from_str::<serde_json::Value>(&stdout(&output))
            .map(|json| json["watching"] == 1)
            .unwrap_or(false)
    });

    assert!(!finds_symbol(&sandbox, &project, "brandNewFunction"));
    std::fs::write(
        project.join("src").join("fresh.ts"),
        "export function brandNewFunction() {}\n",
    )
    .unwrap();

    eventually("the new symbol to be indexed", || {
        finds_symbol(&sandbox, &project, "brandNewFunction")
    });

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

#[test]
fn a_deleted_file_leaves_the_index_on_its_own() {
    let sandbox = watching_sandbox("watch-delete");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["project", "add", path_str(&project), "--index"]));

    let mut daemon = sandbox.spawn_daemon();
    eventually("the symbol to be indexed", || {
        finds_symbol(&sandbox, &project, "authenticate")
    });

    std::fs::remove_file(project.join("src").join("auth.ts")).unwrap();
    eventually("the deleted file to leave the index", || {
        !finds_symbol(&sandbox, &project, "authenticate")
    });

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

#[test]
fn writes_to_ignored_directories_are_never_indexed() {
    let sandbox = watching_sandbox("watch-ignored");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["project", "add", path_str(&project), "--index"]));

    let listed = sandbox.run(&["project", "list", "--format", "jsonl"]);
    let before: serde_json::Value = serde_json::from_str(&stdout(&listed)).unwrap();
    let before = before["projects"][0]["indexed_files"].as_u64().unwrap();

    let mut daemon = sandbox.spawn_daemon();
    eventually("watching to start", || {
        let output = sandbox.run(&["daemon", "status", "--format", "jsonl"]);
        serde_json::from_str::<serde_json::Value>(&stdout(&output))
            .map(|json| json["watching"] == 1)
            .unwrap_or(false)
    });

    // A dependency install or a build: hundreds of files CtxC must not care
    // about, written while it is watching.
    let noisy = project.join("node_modules").join("package");
    std::fs::create_dir_all(&noisy).unwrap();
    for index in 0..100 {
        std::fs::write(
            noisy.join(format!("file-{index}.ts")),
            format!("export function noise{index}() {{}}\n"),
        )
        .unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(1_500));

    let listed = sandbox.run(&["project", "list", "--format", "jsonl"]);
    let after: serde_json::Value = serde_json::from_str(&stdout(&listed)).unwrap();
    assert_eq!(
        after["projects"][0]["indexed_files"].as_u64().unwrap(),
        before,
        "ignored trees must cost nothing, however busy they are"
    );
    assert!(!finds_symbol(&sandbox, &project, "noise42"));

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

#[test]
fn watching_can_be_turned_off_and_the_daemon_still_indexes() {
    let sandbox = Sandbox::new("watch-disabled");
    std::fs::write(
        sandbox.path("config.toml"),
        "[daemon]\nport = 0\n[watch]\nenabled = false\npoll_interval_ms = 500\n",
    )
    .unwrap();

    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["project", "add", path_str(&project)]));

    let mut daemon = sandbox.spawn_daemon();
    eventually("the degradation to be reported", || {
        let output = sandbox.run(&["daemon", "status", "--format", "jsonl"]);
        serde_json::from_str::<serde_json::Value>(&stdout(&output))
            .map(|json| json["degraded"] == 1 && json["watching"] == 0)
            .unwrap_or(false)
    });

    // Polling still keeps the index current, only less promptly.
    eventually("polling to index the project", || {
        finds_symbol(&sandbox, &project, "authenticate")
    });

    let status = sandbox.run(&["daemon", "status", "--format", "jsonl"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&status)).unwrap();
    assert!(
        json["watch"][0]["degraded_reason"]
            .as_str()
            .unwrap()
            .contains("disabled"),
        "the reason must be visible, not guessed at"
    );

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

#[test]
fn a_project_added_while_the_daemon_runs_is_picked_up() {
    let sandbox = watching_sandbox("watch-late-project");
    let mut daemon = sandbox.spawn_daemon();

    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["project", "add", path_str(&project)]));

    eventually("the new project to be indexed", || {
        finds_symbol(&sandbox, &project, "authenticate")
    });

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

#[test]
fn metrics_start_empty_and_say_so() {
    let sandbox = Sandbox::new("metrics-empty");

    let output = sandbox.run(&["metrics"]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("Nothing recorded yet"),
        "an empty installation must not print a wall of zeroes: {}",
        stdout(&output)
    );
}

#[test]
fn optimizing_is_counted_and_attributed_to_its_stages() {
    let sandbox = Sandbox::new("metrics-optimize");

    let input = "same line\n\n\nsame line\n\n\n\nsame line\n";
    assert_success(&sandbox.run_with_stdin(&["optimize"], input));

    let output = sandbox.run(&["metrics", "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let summary = &json["summary"];
    assert_eq!(summary["operations"], 1);
    assert!(
        summary["tokens_saved"].as_i64().unwrap() > 0,
        "optimizing repeated lines saves tokens: {summary}"
    );

    let stages = &summary["savings_by_stage"];
    let attributed: i64 = ["filtering", "deduplication", "compression", "selection"]
        .iter()
        .map(|stage| stages[stage].as_i64().unwrap())
        .sum();
    assert_eq!(
        attributed,
        summary["tokens_saved"].as_i64().unwrap(),
        "the stage breakdown must add up to the total, not approximate it"
    );
}

#[test]
fn token_counts_are_labelled_as_estimates() {
    let sandbox = Sandbox::new("metrics-estimates");
    assert_success(&sandbox.run_with_stdin(&["optimize"], "one\n\ntwo\n\nthree\n"));

    let output = sandbox.run(&["metrics"]);
    assert_success(&output);
    assert!(
        stdout(&output).contains("estimates"),
        "a reader must never take an estimated count for an exact one: {}",
        stdout(&output)
    );
}

#[test]
fn no_configured_rate_means_no_invented_cost() {
    let sandbox = Sandbox::new("metrics-no-cost");
    assert_success(&sandbox.run_with_stdin(&["optimize"], "one\n\ntwo\n"));

    let human = sandbox.run(&["metrics"]);
    assert!(
        stdout(&human).contains("not estimated"),
        "an unconfigured rate must not become a dollar figure: {}",
        stdout(&human)
    );

    let json = sandbox.run(&["metrics", "--format", "json"]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout(&json)).unwrap();
    assert!(parsed["summary"]["estimated_cost_saved"].is_null());
}

#[test]
fn a_configured_rate_prices_the_saving_as_an_estimate() {
    let sandbox = Sandbox::new("metrics-cost");
    std::fs::write(
        sandbox.path("config.toml"),
        "[metrics]\ncost_model = \"some-model\"\ncost_per_million_input_tokens = 3.0\n",
    )
    .unwrap();

    let long = "duplicated paragraph\n\n".repeat(200);
    assert_success(&sandbox.run_with_stdin(&["optimize"], &long));

    let output = sandbox.run(&["metrics", "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let cost = &json["summary"]["estimated_cost_saved"];
    assert_eq!(cost["model"], "some-model");
    assert_eq!(
        cost["estimated"], true,
        "a cost figure must carry its own uncertainty"
    );
}

#[test]
fn metrics_break_down_by_operation() {
    let sandbox = Sandbox::new("metrics-breakdown");
    assert_success(&sandbox.run_with_stdin(&["optimize"], "one\n\ntwo\n"));
    assert_success(&sandbox.run_with_stdin(&["analyze"], "three\n\nfour\n"));

    let output = sandbox.run(&["metrics", "--breakdown", "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let operations: Vec<&str> = json["breakdown"]["by_operation"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["operation"].as_str().unwrap())
        .collect();

    assert!(operations.contains(&"optimize"), "{operations:?}");
    assert!(operations.contains(&"analyze"), "{operations:?}");
}

#[test]
fn recent_activity_lists_what_just_happened() {
    let sandbox = Sandbox::new("metrics-activity");
    assert_success(&sandbox.run_with_stdin(&["optimize"], "one\n\ntwo\n"));

    let output = sandbox.run(&["metrics", "--activity", "5", "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let activity = json["activity"].as_array().unwrap();
    assert_eq!(activity.len(), 1);
    assert_eq!(activity[0]["operation"], "optimize");
}

#[test]
fn a_timeseries_has_one_point_per_bucket() {
    let sandbox = Sandbox::new("metrics-timeseries");
    assert_success(&sandbox.run_with_stdin(&["optimize"], "one\n\ntwo\n"));

    let output = sandbox.run(&["metrics", "--by", "day", "--days", "3", "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let points = json["timeseries"]["points"].as_array().unwrap();
    assert_eq!(
        points.len(),
        4,
        "three days back, plus today, quiet buckets included: {points:?}"
    );
}

#[test]
fn metrics_can_be_scoped_to_one_project() {
    let sandbox = Sandbox::new("metrics-project");
    let project = sample_project(&sandbox);

    assert_success(&sandbox.run(&["project", "add", path_str(&project), "--index"]));
    assert_success(&sandbox.run(&["index", path_str(&project)]));
    assert_success(&sandbox.run_with_stdin(&["optimize"], "unrelated\n\ninput\n"));

    let output = sandbox.run(&[
        "metrics",
        "--project",
        path_str(&project),
        "--breakdown",
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let operations: Vec<&str> = json["breakdown"]["by_operation"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["operation"].as_str().unwrap())
        .collect();

    assert!(operations.contains(&"index"), "{operations:?}");
    assert!(
        !operations.contains(&"optimize"),
        "a piped optimization belongs to no project: {operations:?}"
    );
}

#[test]
fn switching_metrics_off_records_nothing() {
    let sandbox = Sandbox::new("metrics-disabled");
    std::fs::write(sandbox.path("config.toml"), "[metrics]\nenabled = false\n").unwrap();

    assert_success(&sandbox.run_with_stdin(&["optimize"], "one\n\ntwo\n"));

    let output = sandbox.run(&["metrics", "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["summary"]["operations"], 0);
}

#[test]
fn an_unknown_project_is_refused_rather_than_reported_as_empty() {
    let sandbox = Sandbox::new("metrics-unknown-project");
    let output = sandbox.run(&["metrics", "--project", "no-such-project"]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no-such-project"),
        "{}",
        stderr(&output)
    );
}

/// A client for the daemon this sandbox started, authorised from its lockfile.
fn daemon_client(sandbox: &Sandbox) -> ctxc_api::client::Client {
    let lock = ctxc_daemon::lock::Lock::read(&sandbox.home)
        .expect("read the lockfile")
        .expect("the daemon wrote a lockfile");
    lock.client()
}

#[test]
fn the_daemon_reports_metrics_over_the_api() {
    let sandbox = Sandbox::with_daemon("api-metrics");
    let mut daemon = sandbox.spawn_daemon();

    let client = daemon_client(&sandbox);
    let request = serde_json::json!({
        "content": "same line\n\n\nsame line\n\n\nsame line\n",
    });
    let optimized: serde_json::Value = client
        .post("/v1/context/optimize", &request)
        .expect("optimize over the API");
    assert!(optimized["result"]["original_tokens"].as_i64().unwrap() > 0);

    // The endpoint flushes and rolls up before reading, so what just happened
    // is visible without waiting for the daemon's maintenance tick.
    let summary: serde_json::Value = client
        .get("/v1/metrics/summary")
        .expect("read the metrics summary");
    assert_eq!(summary["operations"], 1);
    assert!(summary["tokens_saved"].as_i64().unwrap() > 0);

    let activity: serde_json::Value = client.get("/v1/activity").expect("read activity");
    let events = activity.as_array().unwrap();
    assert_eq!(events[0]["operation"], "optimize");

    let breakdown: serde_json::Value = client.get("/v1/metrics/breakdown").expect("read breakdown");
    assert_eq!(breakdown["by_operation"][0]["operation"], "optimize");

    let series: serde_json::Value = client
        .get("/v1/metrics/timeseries?granularity=day&days=1")
        .expect("read a timeseries");
    assert_eq!(series["granularity"], "day");
    assert_eq!(
        series["points"].as_array().unwrap().len(),
        2,
        "yesterday and today, quiet buckets included"
    );

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

#[test]
fn an_unreadable_granularity_is_refused_rather_than_guessed_at() {
    let sandbox = Sandbox::with_daemon("api-metrics-granularity");
    let mut daemon = sandbox.spawn_daemon();

    let error = daemon_client(&sandbox)
        .get::<serde_json::Value>("/v1/metrics/timeseries?granularity=fortnight")
        .expect_err("a granularity CtxC does not have must not be silently replaced");
    assert!(format!("{error}").contains("fortnight"), "{error}");

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

#[test]
fn metrics_endpoints_require_the_token() {
    let sandbox = Sandbox::with_daemon("api-metrics-auth");
    let mut daemon = sandbox.spawn_daemon();

    let port = ctxc_daemon::lock::Lock::read(&sandbox.home)
        .unwrap()
        .unwrap()
        .port;
    let anonymous = ctxc_api::client::Client::new(port, None);

    let error = anonymous
        .get::<serde_json::Value>("/v1/metrics/summary")
        .expect_err("metrics are local, but still behind the token");
    assert!(matches!(
        error,
        ctxc_api::client::ClientError::Api { status: 401, .. }
    ));

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

#[test]
fn watching_a_project_is_recorded_as_an_operation() {
    let sandbox = watching_sandbox("metrics-watch");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["project", "add", path_str(&project)]));

    let mut daemon = sandbox.spawn_daemon();
    let client = daemon_client(&sandbox);

    eventually("the supervisor to record that it is watching", || {
        let activity: serde_json::Value = match client.get("/v1/activity") {
            Ok(activity) => activity,
            Err(_) => return false,
        };
        activity
            .as_array()
            .map(|events| events.iter().any(|event| event["operation"] == "watch"))
            .unwrap_or(false)
    });

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

/// A raw GET against the daemon, returning `(status, content_type, body)`.
///
/// The dashboard is served as plain HTTP rather than JSON, so the API client —
/// which decodes every response as JSON — cannot be used to check it.
fn fetch(port: u16, path: &str) -> (u16, String, String) {
    use std::io::{Read, Write};

    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .expect("send the request");

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read the response");
    let text = String::from_utf8_lossy(&response).into_owned();

    let (head, body) = text.split_once("\r\n\r\n").expect("a complete response");
    let status: u16 = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .expect("a status line");
    let content_type = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-type")
                .then(|| value.trim().to_owned())
        })
        .unwrap_or_default();

    (status, content_type, body.to_owned())
}

#[test]
fn the_dashboard_command_reports_a_url_without_opening_a_browser() {
    let sandbox = Sandbox::with_daemon("dashboard-url");
    let mut daemon = sandbox.spawn_daemon();

    let output = sandbox.run(&["dashboard", "--no-open", "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["opened"], false);
    assert_eq!(
        json["started_daemon"], false,
        "a daemon was already running"
    );

    let url = json["url"].as_str().unwrap();
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    assert!(
        url.contains("token="),
        "a browser cannot send a header, so the token has to be in the URL: {url}"
    );

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

#[test]
fn asking_for_the_dashboard_starts_a_daemon_if_there_is_none() {
    let sandbox = Sandbox::with_daemon("dashboard-starts-daemon");

    let output = sandbox.run(&["dashboard", "--no-open", "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(
        json["started_daemon"], true,
        "someone asking to look at the dashboard should not have to start a daemon first"
    );

    assert_success(&sandbox.run(&["daemon", "status"]));
    assert_success(&sandbox.run(&["stop"]));
}

#[test]
fn a_disabled_dashboard_is_refused_rather_than_served() {
    let sandbox = Sandbox::new("dashboard-disabled");
    std::fs::write(
        sandbox.path("config.toml"),
        "[daemon]\nport = 0\n[dashboard]\nenabled = false\n",
    )
    .unwrap();

    let output = sandbox.run(&["dashboard", "--no-open"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("disabled"), "{}", stderr(&output));
}

#[test]
fn the_daemon_serves_the_dashboard_from_its_root() {
    let sandbox = Sandbox::with_daemon("dashboard-assets");
    let mut daemon = sandbox.spawn_daemon();
    let port = ctxc_daemon::lock::Lock::read(&sandbox.home)
        .unwrap()
        .unwrap()
        .port;

    let (status, content_type, body) = fetch(port, "/");

    if ctxc_dashboard::is_bundled() {
        assert_eq!(status, 200);
        assert!(content_type.starts_with("text/html"), "{content_type}");
        assert!(body.contains("<div id=\"root\">"), "{body}");

        // A path the browser routes rather than a file it needs.
        let (status, content_type, _) = fetch(port, "/projects");
        assert_eq!(status, 200, "a deep link is a route, not a missing file");
        assert!(content_type.starts_with("text/html"));

        // A file it does need, and does not have.
        let (status, ..) = fetch(port, "/assets/does-not-exist.js");
        assert_eq!(
            status, 404,
            "answering a missing script with HTML turns a 404 into a syntax error"
        );
    } else {
        assert_eq!(status, 404);
        assert!(
            body.contains("does not include the dashboard"),
            "a build without a dashboard must say so: {body}"
        );
    }

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

#[test]
fn the_dashboard_is_served_without_a_token_but_the_api_is_not() {
    let sandbox = Sandbox::with_daemon("dashboard-auth");
    let mut daemon = sandbox.spawn_daemon();
    let port = ctxc_daemon::lock::Lock::read(&sandbox.home)
        .unwrap()
        .unwrap()
        .port;

    // The page itself is static and carries nothing private; everything it
    // then asks for does need the token.
    let (_, _, _) = fetch(port, "/");
    let (status, ..) = fetch(port, "/v1/status");
    assert_eq!(status, 401, "the API stays behind the token");

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

#[test]
fn the_event_stream_refuses_a_handshake_with_no_token() {
    use std::io::{Read, Write};

    let sandbox = Sandbox::with_daemon("dashboard-events-auth");
    let mut daemon = sandbox.spawn_daemon();
    let lock = ctxc_daemon::lock::Lock::read(&sandbox.home)
        .unwrap()
        .unwrap();

    let handshake = |query: &str| -> u16 {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", lock.port)).expect("connect");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        write!(
            stream,
            "GET /v1/events{query} HTTP/1.1\r\n\
             Host: 127.0.0.1:{}\r\n\
             Connection: Upgrade\r\n\
             Upgrade: websocket\r\n\
             Sec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
            lock.port
        )
        .expect("send the handshake");

        let mut head = [0u8; 64];
        let read = stream.read(&mut head).expect("read the response");
        String::from_utf8_lossy(&head[..read])
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .expect("a status line")
    };

    assert_eq!(handshake(""), 401);

    let token = lock.token().expect("the lockfile carries a token");
    assert_eq!(
        handshake(&format!("?token={}", token.as_str())),
        101,
        "a client with the token gets an upgraded socket"
    );

    assert_success(&sandbox.run(&["stop"]));
    daemon.wait().expect("the daemon exits");
}

// ---------------------------------------------------------------------------
// Agent integrations
// ---------------------------------------------------------------------------

/// A project directory with nothing in it but a source file.
fn plain_project(sandbox: &Sandbox, name: &str) -> PathBuf {
    let project = sandbox.path(name);
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src").join("main.rs"), "fn main() {}\n").unwrap();
    project
}

#[test]
fn integrations_are_listed_with_what_was_detected() {
    let sandbox = Sandbox::new("integrations-list");
    let project = plain_project(&sandbox, "app");
    std::fs::create_dir_all(project.join(".claude")).unwrap();

    let output = sandbox.run(&[
        "integrations",
        "list",
        "--path",
        path_str(&project),
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let rows = json["integrations"].as_array().unwrap();

    let claude = rows
        .iter()
        .find(|row| row["name"] == "claude-code")
        .expect("claude-code is one of the integrations");
    assert_eq!(
        claude["detected"], true,
        "the .claude directory is the marker"
    );
    assert_eq!(claude["installed"], false);

    let cursor = rows.iter().find(|row| row["name"] == "cursor").unwrap();
    assert_eq!(
        cursor["detected"], false,
        "one agent being present does not imply another"
    );
}

#[test]
fn installing_writes_guidance_and_leaves_the_rest_of_the_file_alone() {
    let sandbox = Sandbox::new("integrations-install");
    let project = plain_project(&sandbox, "app");
    let instructions = project.join("CLAUDE.md");
    std::fs::write(&instructions, "# House rules\n\nAlways run the tests.\n").unwrap();

    assert_success(&sandbox.run(&[
        "integrations",
        "install",
        "claude-code",
        "--path",
        path_str(&project),
    ]));

    let contents = std::fs::read_to_string(&instructions).unwrap();
    assert!(contents.contains("Always run the tests."), "{contents}");
    assert!(contents.contains("ctxc find"), "{contents}");
    assert!(contents.contains("ctxc:begin"), "{contents}");
}

#[test]
fn installing_twice_reports_that_nothing_changed() {
    let sandbox = Sandbox::new("integrations-idempotent");
    let project = plain_project(&sandbox, "app");
    let args = [
        "integrations",
        "install",
        "claude-code",
        "--path",
        path_str(&project),
        "--format",
        "json",
    ];

    assert_success(&sandbox.run(&args));
    let before = std::fs::read_to_string(project.join("CLAUDE.md")).unwrap();

    let output = sandbox.run(&args);
    assert_success(&output);
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["changes"][0]["outcome"], "unchanged");

    assert_eq!(
        std::fs::read_to_string(project.join("CLAUDE.md")).unwrap(),
        before
    );
}

#[test]
fn uninstalling_restores_the_file_exactly() {
    let sandbox = Sandbox::new("integrations-uninstall");
    let project = plain_project(&sandbox, "app");
    let instructions = project.join("CLAUDE.md");
    let original = "# House rules\n\nAlways run the tests.\n";
    std::fs::write(&instructions, original).unwrap();

    let path = path_str(&project);
    assert_success(&sandbox.run(&["integrations", "install", "claude-code", "--path", path]));
    assert_success(&sandbox.run(&["integrations", "uninstall", "claude-code", "--path", path]));

    assert_eq!(
        std::fs::read_to_string(&instructions).unwrap(),
        original,
        "everything CtxC did not write must come back untouched"
    );
}

#[test]
fn only_detected_agents_are_touched_when_asked() {
    let sandbox = Sandbox::new("integrations-detected");
    let project = plain_project(&sandbox, "app");
    std::fs::create_dir_all(project.join(".claude")).unwrap();

    let output = sandbox.run(&[
        "integrations",
        "install",
        "--detected",
        "--path",
        path_str(&project),
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let changed: Vec<&str> = json["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|change| change["name"].as_str().unwrap())
        .collect();

    // One agent, two mechanisms: guidance telling it CtxC exists, and an MCP
    // registration actually handing it the tools.
    assert_eq!(changed, ["claude-code", "claude-code-mcp"], "{changed:?}");

    assert!(
        !project.join("AGENTS.md").exists(),
        "an agent that is not here should not get a file"
    );
    assert!(
        !project.join(".cursor").exists(),
        "and neither should one whose marker is absent"
    );
}

#[test]
fn a_named_agent_installs_even_when_it_was_not_detected() {
    let sandbox = Sandbox::new("integrations-undetected");
    let project = plain_project(&sandbox, "app");

    // Setting an agent up before its first run is entirely reasonable.
    assert_success(&sandbox.run(&[
        "integrations",
        "install",
        "cursor",
        "--path",
        path_str(&project),
    ]));

    let rule = project.join(".cursor").join("rules").join("ctxc.mdc");
    assert!(rule.exists(), "{}", rule.display());
    assert!(std::fs::read_to_string(&rule)
        .unwrap()
        .contains("alwaysApply"));
}

#[test]
fn an_unknown_agent_is_refused_with_a_way_to_find_the_real_names() {
    let sandbox = Sandbox::new("integrations-unknown");
    let project = plain_project(&sandbox, "app");

    let output = sandbox.run(&[
        "integrations",
        "install",
        "emacs",
        "--path",
        path_str(&project),
    ]);

    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("no integration named `emacs`"), "{text}");
    assert!(text.contains("ctxc config agents list"), "{text}");
}

#[test]
fn installing_into_an_unregistered_directory_says_what_is_missing() {
    let sandbox = Sandbox::new("integrations-unregistered");
    let project = plain_project(&sandbox, "app");

    let output = sandbox.run(&[
        "integrations",
        "install",
        "claude-code",
        "--path",
        path_str(&project),
    ]);
    assert_success(&output);

    let text = stdout(&output);
    assert!(
        text.contains("not a registered project"),
        "guidance that tells an agent to search an unindexed project needs a caveat: {text}"
    );
}

#[test]
fn a_registered_project_is_named_in_the_guidance_it_gets() {
    let sandbox = Sandbox::new("integrations-registered");
    let project = plain_project(&sandbox, "app");

    assert_success(&sandbox.run(&["project", "add", path_str(&project)]));
    let output = sandbox.run(&[
        "integrations",
        "install",
        "claude-code",
        "--path",
        path_str(&project),
    ]);
    assert_success(&output);

    assert!(
        !stdout(&output).contains("not a registered project"),
        "{}",
        stdout(&output)
    );
    let contents = std::fs::read_to_string(project.join("CLAUDE.md")).unwrap();
    assert!(contents.contains("`app`"), "{contents}");
}

// ---------------------------------------------------------------------------
// MCP
// ---------------------------------------------------------------------------

/// Speak one MCP conversation to `ctxc mcp` and collect the answers.
///
/// The server reads newline-delimited JSON on stdin and answers on stdout, so
/// a whole session is one write and one read.
fn mcp(sandbox: &Sandbox, working_directory: &Path, messages: &[&str]) -> Vec<serde_json::Value> {
    use std::io::Write;

    let mut child = sandbox
        .command(&["mcp"])
        .current_dir(working_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the MCP server");

    {
        let mut stdin = child.stdin.take().expect("stdin is piped");
        for message in messages {
            writeln!(stdin, "{message}").expect("write a message");
        }
        // Closing stdin is how an MCP client says goodbye.
    }

    let output = child.wait_with_output().expect("the server exits");
    assert!(
        output.status.success(),
        "the server exited badly: {}",
        stderr(&output)
    );

    stdout(&output)
        .lines()
        .map(|line| serde_json::from_str(line).expect("every line is one JSON object"))
        .collect()
}

/// The text a tool call produced.
fn tool_text(response: &serde_json::Value) -> &str {
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("not a tool result: {response}"))
}

const HANDSHAKE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;

#[test]
fn the_mcp_server_completes_a_handshake_and_lists_its_tools() {
    let sandbox = Sandbox::new("mcp-handshake");
    let project = plain_project(&sandbox, "app");

    let answers = mcp(
        &sandbox,
        &project,
        &[
            HANDSHAKE,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        ],
    );

    assert_eq!(
        answers.len(),
        2,
        "the notification must not be answered: {answers:?}"
    );
    assert_eq!(answers[0]["result"]["serverInfo"]["name"], "ctxc");

    let tools: Vec<&str> = answers[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();

    for expected in [
        "ctxc_search",
        "ctxc_retrieve",
        "ctxc_compile",
        "ctxc_optimize",
        "ctxc_index",
        "ctxc_memory",
    ] {
        assert!(tools.contains(&expected), "{expected} missing: {tools:?}");
    }
}

#[test]
fn an_agent_can_index_then_search_a_project() {
    let sandbox = Sandbox::new("mcp-search");
    let project = sample_project(&sandbox);

    let answers = mcp(
        &sandbox,
        &project,
        &[
            HANDSHAKE,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ctxc_index","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ctxc_search","arguments":{"query":"authenticate"}}}"#,
        ],
    );

    assert_eq!(answers[1]["result"]["isError"], false);
    assert!(
        tool_text(&answers[1]).contains("Indexed"),
        "{:?}",
        answers[1]
    );

    assert_eq!(answers[2]["result"]["isError"], false);
    let found = tool_text(&answers[2]);
    assert!(found.contains("authenticate"), "{found}");
}

#[test]
fn searching_an_unindexed_project_says_what_to_do_about_it() {
    let sandbox = Sandbox::new("mcp-unindexed");
    let project = plain_project(&sandbox, "app");

    let answers = mcp(
        &sandbox,
        &project,
        &[
            HANDSHAKE,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ctxc_search","arguments":{"query":"anything"}}}"#,
        ],
    );

    assert_eq!(answers[1]["result"]["isError"], true);
    let text = tool_text(&answers[1]);
    assert!(
        text.contains("ctxc_index"),
        "a dead end should name the way out: {text}"
    );
}

#[test]
fn optimized_output_can_be_turned_back_into_the_original() {
    let sandbox = Sandbox::new("mcp-round-trip");
    let project = plain_project(&sandbox, "app");
    let original = "same line\n\n\nsame line\n\n\n\nsame line\nunique tail\n";

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "ctxc_optimize",
            "arguments": { "content": original, "source": "npm test" },
        },
    })
    .to_string();

    let answers = mcp(&sandbox, &project, &[HANDSHAKE, &request]);
    assert_eq!(answers[1]["result"]["isError"], false);

    let optimized = tool_text(&answers[1]);
    let reference = optimized
        .lines()
        .find_map(|line| line.strip_prefix("Original: "))
        .expect("every optimized result carries a reference");

    let recall = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "ctxc_retrieve",
            "arguments": { "reference": reference },
        },
    })
    .to_string();

    let answers = mcp(&sandbox, &project, &[HANDSHAKE, &recall]);
    assert_eq!(answers[1]["result"]["isError"], false);
    assert_eq!(
        tool_text(&answers[1]),
        original,
        "reversible means byte for byte"
    );
}

#[test]
fn memory_survives_between_sessions() {
    let sandbox = Sandbox::new("mcp-memory");
    let project = plain_project(&sandbox, "app");
    assert_success(&sandbox.run(&["project", "add", path_str(&project)]));

    mcp(
        &sandbox,
        &project,
        &[
            HANDSHAKE,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ctxc_memory","arguments":{"action":"save","key":"build","value":"Use npm run build."}}}"#,
        ],
    );

    // A separate process, which is what a new agent session is.
    let answers = mcp(
        &sandbox,
        &project,
        &[
            HANDSHAKE,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ctxc_memory","arguments":{"action":"get","key":"build"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ctxc_memory","arguments":{"action":"get","key":"never-written"}}}"#,
        ],
    );

    assert_eq!(tool_text(&answers[1]), "Use npm run build.");
    assert_eq!(
        answers[2]["result"]["isError"], true,
        "a note that was never written is not an empty note"
    );
}

#[test]
fn a_malformed_message_does_not_end_the_session() {
    let sandbox = Sandbox::new("mcp-malformed");
    let project = plain_project(&sandbox, "app");

    let answers = mcp(
        &sandbox,
        &project,
        &[
            HANDSHAKE,
            "{not json",
            r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#,
        ],
    );

    assert_eq!(answers[1]["error"]["code"], -32700);
    assert!(
        answers[2]["result"].is_object(),
        "the session carries on: {:?}",
        answers[2]
    );
}

#[test]
fn registering_the_mcp_server_writes_a_config_an_agent_can_use() {
    let sandbox = Sandbox::new("mcp-register");
    let project = plain_project(&sandbox, "app");

    assert_success(&sandbox.run(&[
        "integrations",
        "install",
        "claude-code-mcp",
        "--path",
        path_str(&project),
    ]));

    let config = std::fs::read_to_string(project.join(".mcp.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&config).unwrap();
    assert_eq!(parsed["mcpServers"]["ctxc"]["command"], "ctxc");
    assert_eq!(parsed["mcpServers"]["ctxc"]["args"][0], "mcp");
}

// ---------------------------------------------------------------------------
// Embeddings
// ---------------------------------------------------------------------------

/// A sandbox with embeddings switched on.
fn embedding_sandbox(name: &str) -> Sandbox {
    let sandbox = Sandbox::new(name);
    std::fs::write(
        sandbox.path("config.toml"),
        "[semantic]\nenabled = true\n[ranking]\nsemantic = 0.8\n",
    )
    .unwrap();
    sandbox
}

#[test]
fn indexing_embeds_files_and_skips_ones_that_have_not_changed() {
    let sandbox = embedding_sandbox("semantic-index");
    let project = sample_project(&sandbox);
    let path = path_str(&project);

    let output = sandbox.run(&["index", path, "--format", "json"]);
    assert_success(&output);
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert!(
        json["embedded"].as_u64().unwrap() > 0,
        "embeddings should be built during the index pass: {json}"
    );

    let again = sandbox.run(&["index", path, "--format", "json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&again)).unwrap();
    assert_eq!(
        json["embedded"], 0,
        "re-embedding unchanged content is wasted work: {json}"
    );
}

#[test]
fn indexing_embeds_nothing_while_embeddings_are_off() {
    let sandbox = Sandbox::new("semantic-off");
    let project = sample_project(&sandbox);

    let output = sandbox.run(&["index", path_str(&project), "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(
        json["embedded"], 0,
        "the deterministic path must not pay for a feature nobody turned on"
    );
}

#[test]
fn similar_finds_files_that_share_wording_and_says_what_that_means() {
    let sandbox = embedding_sandbox("semantic-similar");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["index", path_str(&project)]));

    let output = sandbox.run(&[
        "similar",
        "authenticate user token",
        "--path",
        path_str(&project),
    ]);
    assert_success(&output);

    let text = stdout(&output);
    assert!(text.contains("auth"), "{text}");
    assert!(
        text.contains("not shared meaning"),
        "a lexical score must never be presented as a semantic one: {text}"
    );
}

#[test]
fn similar_refuses_rather_than_returning_nothing_when_embeddings_are_off() {
    let sandbox = Sandbox::new("semantic-similar-off");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["index", path_str(&project)]));

    let output = sandbox.run(&["similar", "anything", "--path", path_str(&project)]);

    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(
        text.contains("switched off"),
        "an empty result would read as `nothing matches`: {text}"
    );
    assert!(text.contains("semantic.enabled"), "{text}");
}

#[test]
fn similar_says_when_a_project_has_not_been_embedded_yet() {
    let sandbox = embedding_sandbox("semantic-unembedded");
    let project = sample_project(&sandbox);

    // Indexed by nobody: the switch is on, but nothing has run.
    let output = sandbox.run(&["similar", "anything", "--path", path_str(&project)]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("ctxc project index"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn embedding_similarity_reaches_the_ranking_signals() {
    let sandbox = embedding_sandbox("semantic-ranking");
    let project = sample_project(&sandbox);
    let path = path_str(&project);
    assert_success(&sandbox.run(&["index", path]));

    let output = sandbox.run(&["search", "authenticate", "--path", path, "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let files = json["files"].as_array().unwrap();
    assert!(!files.is_empty(), "{json}");
    assert!(
        files
            .iter()
            .any(|file| file["signals"]["semantic"].as_f64().unwrap_or(0.0) > 0.0),
        "the semantic signal should be populated: {json}"
    );
}

#[test]
fn search_is_unchanged_when_embeddings_are_off() {
    let sandbox = Sandbox::new("semantic-ranking-off");
    let project = sample_project(&sandbox);
    let path = path_str(&project);
    assert_success(&sandbox.run(&["index", path]));

    let output = sandbox.run(&["search", "authenticate", "--path", path, "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    for file in json["files"].as_array().unwrap() {
        assert_eq!(
            file["signals"]["semantic"].as_f64().unwrap_or(0.0),
            0.0,
            "no embeddings means no semantic contribution: {file}"
        );
    }
}

#[test]
fn an_unknown_embedding_provider_is_refused_with_the_names_that_work() {
    let sandbox = Sandbox::new("semantic-bad-provider");
    std::fs::write(
        sandbox.path("config.toml"),
        "[semantic]\nenabled = true\nprovider = \"some-model\"\n",
    )
    .unwrap();
    let project = sample_project(&sandbox);

    let output = sandbox.run(&["index", path_str(&project)]);

    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("some-model"), "{text}");
    assert!(
        text.contains("hashed"),
        "a refusal should name what would have worked: {text}"
    );
}

// ---------------------------------------------------------------------------
// The grouped command surface.
//
// The behaviour these forms reach is already covered above under the older
// names; what is tested here is that the new spelling reaches it.
// ---------------------------------------------------------------------------

#[test]
fn optimize_describes_one_input_with_dry_run() {
    let sandbox = Sandbox::new("optimize-dry-run");
    let input = sandbox.path("notes.txt");
    std::fs::write(&input, SAMPLE).unwrap();

    let output = sandbox.run(&[
        "optimize",
        "--dry-run",
        path_str(&input),
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["content_type"], "plain_text");
    assert!(json["projected_tokens"].as_u64().unwrap() < json["tokens"].as_u64().unwrap());
}

#[test]
fn optimize_compiles_when_it_is_given_several_inputs() {
    let sandbox = Sandbox::new("optimize-many");
    let first = sandbox.path("first.txt");
    let second = sandbox.path("second.txt");
    std::fs::write(&first, "alpha\n").unwrap();
    std::fs::write(&second, "beta\n").unwrap();

    let output = sandbox.run(&[
        "optimize",
        path_str(&first),
        path_str(&second),
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["sections"].as_array().unwrap().len(), 2);
}

#[test]
fn optimize_runs_a_command_written_after_a_double_dash() {
    let sandbox = Sandbox::new("optimize-capture");
    let ctxc = env!("CARGO_BIN_EXE_ctxc");

    let output = sandbox.run(&["optimize", "--format", "json", "--", ctxc, "version"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["exit_code"], 0);
    assert!(
        json["source"].as_str().unwrap().contains("version"),
        "{json}"
    );
}

#[test]
fn optimize_refuses_a_dry_run_it_cannot_honour() {
    let sandbox = Sandbox::new("optimize-dry-run-refused");
    let first = sandbox.path("first.txt");
    let second = sandbox.path("second.txt");
    std::fs::write(&first, SAMPLE).unwrap();
    std::fs::write(&second, SAMPLE).unwrap();

    let output = sandbox.run(&["optimize", "--dry-run", path_str(&first), path_str(&second)]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("one input at a time"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn find_searches_a_project_the_way_search_did() {
    let sandbox = Sandbox::new("find-search");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["project", "index", path_str(&project)]));

    let output = sandbox.run(&[
        "find",
        "database",
        "--path",
        path_str(&project),
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert!(json["files"].is_array(), "{json}");
}

#[test]
fn find_recovers_a_reference_without_a_project() {
    let sandbox = Sandbox::new("find-reference");
    let input = sandbox.path("notes.txt");
    std::fs::write(&input, SAMPLE).unwrap();

    let optimized = sandbox.run(&["optimize", path_str(&input), "--format", "json"]);
    assert_success(&optimized);
    let json: serde_json::Value = serde_json::from_str(&stdout(&optimized)).unwrap();
    let reference = json["reference"].as_str().unwrap().to_string();

    let recovered = sandbox.run(&["find", &reference]);
    assert_success(&recovered);
    assert_eq!(stdout(&recovered), SAMPLE);
}

#[test]
fn project_index_and_graph_reach_the_same_work() {
    let sandbox = Sandbox::new("project-index-graph");
    let project = sample_project(&sandbox);

    let indexed = sandbox.run(&["project", "index", path_str(&project), "--format", "json"]);
    assert_success(&indexed);
    let json: serde_json::Value = serde_json::from_str(&stdout(&indexed)).unwrap();
    assert_eq!(json["indexed"], 3);

    let graph = sandbox.run(&["project", "graph", path_str(&project), "--format", "json"]);
    assert_success(&graph);
    let json: serde_json::Value = serde_json::from_str(&stdout(&graph)).unwrap();
    assert_eq!(json["edges"], 1);
}

#[test]
fn status_reports_savings_only_when_asked_for_metrics() {
    let sandbox = Sandbox::new("status-metrics");
    assert_success(&sandbox.run_with_stdin(&["optimize"], "same line\n\n\nsame line\n"));

    let output = sandbox.run(&["status", "--metrics", "--format", "json"]);
    assert_success(&output);
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["summary"]["operations"], 1, "{json}");

    // Without the flag it is still the installation report.
    let plain = sandbox.run(&["status", "--format", "json"]);
    assert_success(&plain);
    let json: serde_json::Value = serde_json::from_str(&stdout(&plain)).unwrap();
    assert!(json["daemon"].is_object(), "{json}");
}

#[test]
fn config_agents_lists_the_integrations() {
    let sandbox = Sandbox::new("config-agents");
    let project = sample_project(&sandbox);

    let output = sandbox.run(&[
        "config",
        "agents",
        "list",
        "--path",
        path_str(&project),
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert!(
        !json["integrations"].as_array().unwrap().is_empty(),
        "{json}"
    );
}

/// A hidden command that no longer dispatches is worse than one that was
/// removed outright, so each folded-in name is run for real, not just parsed.
#[test]
fn old_names_still_work() {
    let sandbox = Sandbox::new("old-names");
    let input = sandbox.path("notes.txt");
    std::fs::write(&input, SAMPLE).unwrap();

    assert_success(&sandbox.run(&["analyze", path_str(&input)]));
    assert_success(&sandbox.run(&["compile", path_str(&input), path_str(&input)]));
    assert_success(&sandbox.run(&["metrics"]));
    assert_success(&sandbox.run(&["version"]));

    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["index", path_str(&project)]));
    assert_success(&sandbox.run(&["graph", path_str(&project)]));
    assert_success(&sandbox.run(&["search", "database", "--path", path_str(&project)]));
    assert_success(&sandbox.run(&["integrations", "list", "--path", path_str(&project)]));
    assert_success(&sandbox.run(&["daemon", "status"]));
}

#[test]
fn status_reports_the_daemon_in_full_when_asked() {
    let sandbox = Sandbox::new("status-daemon-flag");

    let output = sandbox.run(&["status", "--daemon", "--format", "json"]);
    assert_success(&output);
    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(json["running"], false, "{json}");

    // The same report the removed `ctxc daemon status` printed.
    let old = sandbox.run(&["daemon", "status", "--format", "json"]);
    assert_success(&old);
    assert_eq!(stdout(&old), stdout(&output));
}

#[test]
fn init_registers_indexes_and_reports_what_to_do_next() {
    let sandbox = Sandbox::new("init");
    let project = sample_project(&sandbox);

    let output = sandbox.run(&["init", path_str(&project), "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("valid json");
    assert_eq!(json["registered"], true);
    assert_eq!(json["project"], "project");
    assert!(json["index"]["files"].as_u64().unwrap() >= 3, "{json}");
    assert!(json["index"]["symbols"].as_u64().unwrap() >= 1, "{json}");
    assert_eq!(json["daemon"], "not_started");

    // What init set up is what the other commands see.
    let listed = sandbox.run(&["project", "list", "--format", "json"]);
    assert_success(&listed);
    let json: serde_json::Value = serde_json::from_str(&stdout(&listed)).unwrap();
    assert_eq!(json["projects"].as_array().unwrap().len(), 1);

    let found = sandbox.run(&["find", "authenticate", "--path", path_str(&project)]);
    assert_success(&found);
    assert!(stdout(&found).contains("auth.ts"), "{}", stdout(&found));
}

/// Every step `init` runs is idempotent, so the command as a whole must be.
#[test]
fn init_run_twice_changes_nothing_and_still_succeeds() {
    let sandbox = Sandbox::new("init-twice");
    let project = sample_project(&sandbox);

    assert_success(&sandbox.run(&["init", path_str(&project)]));
    let output = sandbox.run(&["init", path_str(&project), "--format", "json"]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("valid json");
    assert_eq!(json["registered"], false, "the project was added twice");
    assert!(json["index"]["symbols"].as_u64().unwrap() >= 1, "{json}");

    let listed = sandbox.run(&["project", "list", "--format", "json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout(&listed)).unwrap();
    assert_eq!(json["projects"].as_array().unwrap().len(), 1);
}

#[test]
fn init_can_skip_the_work_it_is_told_to_skip() {
    let sandbox = Sandbox::new("init-skip");
    let project = sample_project(&sandbox);

    let output = sandbox.run(&[
        "init",
        path_str(&project),
        "--no-index",
        "--no-agents",
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("valid json");
    assert_eq!(json["registered"], true);
    assert!(json["index"].is_null(), "nothing should have been indexed");
    assert_eq!(json["agents"].as_array().unwrap().len(), 0);
}

#[test]
fn an_empty_search_says_what_to_try_next() {
    let sandbox = Sandbox::new("hint-search");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["project", "index", path_str(&project)]));

    let output = sandbox.run(&["find", "zzzznothinghere", "--path", path_str(&project)]);
    assert_success(&output);

    let advice = stderr(&output);
    assert!(advice.contains("--similar"), "{advice}");
    assert!(advice.contains("--limit"), "{advice}");
}

/// Advice is for people. A machine consumer must read the same bytes it always
/// did, with nothing extra on either stream.
#[test]
fn hints_never_reach_machine_output() {
    let sandbox = Sandbox::new("hint-json");
    let project = sample_project(&sandbox);
    assert_success(&sandbox.run(&["project", "index", path_str(&project)]));

    let output = sandbox.run(&[
        "find",
        "zzzznothinghere",
        "--path",
        path_str(&project),
        "--format",
        "json",
    ]);
    assert_success(&output);

    let json: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("valid json");
    assert_eq!(json["files"].as_array().unwrap().len(), 0);
    assert!(
        !stderr(&output).contains("--similar"),
        "stderr carried a hint into a machine-readable run: {}",
        stderr(&output)
    );
}

#[test]
fn status_with_nothing_indexed_points_at_init() {
    let sandbox = Sandbox::new("hint-status");
    let output = sandbox.run(&["status"]);
    assert_success(&output);

    let advice = stderr(&output);
    assert!(advice.contains("ctxc init"), "{advice}");
}

#[test]
fn completions_are_generated_for_every_supported_shell() {
    let sandbox = Sandbox::new("completions");

    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let output = sandbox.run(&["completions", shell]);
        assert_success(&output);

        let script = stdout(&output);
        assert!(script.len() > 200, "{shell} script looks empty: {script}");
        for command in ["init", "optimize", "find", "project"] {
            assert!(
                script.contains(command),
                "{shell} completions never mention `{command}`"
            );
        }
    }
}

/// The progress line is drawn with carriage returns, which belong on a
/// terminal and nowhere else. Captured output must not carry them.
#[test]
fn indexing_prints_no_progress_when_its_output_is_captured() {
    let sandbox = Sandbox::new("progress-piped");
    let project = sample_project(&sandbox);

    let output = sandbox.run(&["project", "index", path_str(&project)]);
    assert_success(&output);
    assert!(
        !stderr(&output).contains('\r'),
        "a redrawn line reached a pipe: {:?}",
        stderr(&output)
    );

    let output = sandbox.run(&[
        "project",
        "index",
        path_str(&project),
        "--force",
        "--format",
        "json",
    ]);
    assert_success(&output);
    serde_json::from_str::<serde_json::Value>(&stdout(&output)).expect("valid json");
    assert!(
        !stderr(&output).contains("files seen"),
        "{}",
        stderr(&output)
    );
}
