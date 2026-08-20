//! Tool output optimizers.
//!
//! Command output is where an agent's context budget actually goes: a build
//! log, a test run, a `git status`. What is worth removing depends on the tool,
//! so this is a registry of profiles rather than one clever filter, and a
//! generic profile always exists so an unrecognised command still gets the
//! basics.
//!
//! Profiles only ever drop lines they *recognise* as noise — progress spam,
//! per-test success lines, the hints git prints to teach you git. Anything a
//! profile does not recognise passes through untouched, because the cost of
//! dropping a real error is far higher than the saving from dropping a hint.

use std::sync::Arc;

use ctxc_context::fragment::SplitStrategy;
use ctxc_core::optimization::{OptimizedContext, Stage};
use ctxc_core::{ContentType, Context, ContextSource, TokenBudget, Tokenizer};

use crate::error::Result;
use crate::log::strip_ansi;
use crate::stages::{deduplicate, trim_and_drop_blanks, StageRunner};
use crate::ContextOptimizer;

/// What one family of commands needs done to its output.
#[derive(Debug, Clone, Copy)]
pub struct ToolProfile {
    /// Reported as the optimizer name.
    pub name: &'static str,
    /// Whether this profile handles a command line.
    pub matches: fn(&str) -> bool,
    /// Whether a line carries nothing worth sending to a model.
    pub is_noise: fn(&str) -> bool,
}

impl ToolProfile {
    /// Profiles in routing order: specific first, generic last.
    pub const REGISTRY: [ToolProfile; 4] = [
        ToolProfile::GIT,
        ToolProfile::TEST,
        ToolProfile::BUILD,
        ToolProfile::GENERIC,
    ];

    /// `git`, whose output is mostly instructions on how to use git.
    pub const GIT: ToolProfile = ToolProfile {
        name: "git",
        matches: |command| first_word(command) == "git",
        is_noise: is_git_noise,
    };

    /// Test runners, where the passing tests are the part nobody needs.
    pub const TEST: ToolProfile = ToolProfile {
        name: "test",
        matches: is_test_command,
        is_noise: is_test_noise,
    };

    /// Build tools, whose progress chatter dwarfs their diagnostics.
    pub const BUILD: ToolProfile = ToolProfile {
        name: "build",
        matches: is_build_command,
        is_noise: is_build_noise,
    };

    /// Anything else that was captured from a command.
    pub const GENERIC: ToolProfile = ToolProfile {
        name: "tool",
        matches: |_| true,
        is_noise: |_| false,
    };
}

/// Optimizer for the output of one family of commands.
pub struct ToolOutputOptimizer {
    tokenizer: Arc<dyn Tokenizer>,
    profile: ToolProfile,
}

impl ToolOutputOptimizer {
    pub fn new(tokenizer: Arc<dyn Tokenizer>, profile: ToolProfile) -> Self {
        ToolOutputOptimizer { tokenizer, profile }
    }

    /// One optimizer per registered profile, in routing order.
    pub fn registry(tokenizer: Arc<dyn Tokenizer>) -> Vec<ToolOutputOptimizer> {
        ToolProfile::REGISTRY
            .iter()
            .map(|profile| ToolOutputOptimizer::new(Arc::clone(&tokenizer), *profile))
            .collect()
    }

    pub fn profile(&self) -> ToolProfile {
        self.profile
    }
}

impl ContextOptimizer for ToolOutputOptimizer {
    fn name(&self) -> &'static str {
        self.profile.name
    }

    fn supports(&self, context: &Context) -> bool {
        // JSON output is better served by the optimizer that can parse it.
        if context.metadata.content_type == ContentType::Json {
            return false;
        }
        match &context.metadata.source {
            ContextSource::Command { command } => (self.profile.matches)(command),
            _ => false,
        }
    }

    fn optimize(&self, context: &Context, budget: Option<TokenBudget>) -> Result<OptimizedContext> {
        let is_noise = self.profile.is_noise;
        let mut runner = StageRunner::new(&*self.tokenizer, context, SplitStrategy::Lines);

        runner.apply(Stage::Filtering, |fragments| {
            for item in fragments.iter_mut() {
                item.content = strip_ansi(&item.content);
            }
            fragments.retain(|item| !is_noise(item.content.trim()));
            trim_and_drop_blanks(fragments);
        });

        runner.apply(Stage::Deduplication, |fragments| {
            deduplicate(fragments, collapse_whitespace, Some(annotate_repeats))
        });

        runner.select(budget);
        Ok(runner.finish(self.name(), context))
    }
}

fn annotate_repeats(content: &str, repeats: usize) -> String {
    format!("{content}  (repeated {repeats} times)")
}

/// Dedup identity for tool output: exact text, whitespace collapsed.
///
/// Deliberately stricter than the log optimizer's: command output is full of
/// file names and counts that differ in exactly one number, and collapsing
/// those would lose the part that matters.
fn collapse_whitespace(content: &str) -> String {
    content.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn first_word(command: &str) -> &str {
    command.split_whitespace().next().unwrap_or("")
}

/// The subcommand, e.g. `test` in `cargo test --all`.
fn second_word(command: &str) -> &str {
    command.split_whitespace().nth(1).unwrap_or("")
}

fn is_test_command(command: &str) -> bool {
    let program = first_word(command);
    let subcommand = second_word(command);

    matches!(
        program,
        "pytest" | "jest" | "vitest" | "gotestsum" | "phpunit" | "rspec"
    ) || (program == "cargo" && subcommand == "test")
        || (program == "go" && subcommand == "test")
        || (matches!(program, "npm" | "pnpm" | "yarn" | "bun") && subcommand == "test")
}

fn is_build_command(command: &str) -> bool {
    let program = first_word(command);
    let subcommand = second_word(command);

    matches!(
        program,
        "make" | "gradle" | "mvn" | "tsc" | "webpack" | "vite" | "docker"
    ) || (program == "cargo" && matches!(subcommand, "build" | "check" | "clippy" | "run"))
        || (program == "go" && matches!(subcommand, "build" | "vet"))
        || (matches!(program, "npm" | "pnpm" | "yarn" | "bun")
            && matches!(subcommand, "install" | "ci" | "run" | "build"))
}

/// Git spends most of its output telling you which git command to run next.
fn is_git_noise(line: &str) -> bool {
    line.starts_with("(use \"git ")
        || line.starts_with("(commit or discard")
        || line == "no changes added to commit (use \"git add\" and/or \"git commit -a\")"
        || line.starts_with("nothing added to commit but untracked files present")
}

/// A passing test is worth one line in a summary, not one line each.
fn is_test_noise(line: &str) -> bool {
    // Rust: `test module::name ... ok`
    if line.starts_with("test ") && line.ends_with(" ... ok") {
        return true;
    }
    // Jest and vitest tick marks, pytest's PASSED, Go's ok/--- PASS.
    line.starts_with("\u{2713} ")
        || line.starts_with("PASS ")
        || line.ends_with(" PASSED")
        || line.starts_with("--- PASS")
        || line.starts_with("=== RUN")
        || is_build_noise(line)
}

/// Build progress that says only "still working".
fn is_build_noise(line: &str) -> bool {
    const PREFIXES: [&str; 8] = [
        "Compiling ",
        "Downloading ",
        "Downloaded ",
        "Updating ",
        "Fresh ",
        "Installing ",
        "Blocking ",
        "Adding ",
    ];
    PREFIXES.iter().any(|prefix| line.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_core::HeuristicTokenizer;

    fn optimizer(profile: ToolProfile) -> ToolOutputOptimizer {
        ToolOutputOptimizer::new(Arc::new(HeuristicTokenizer::new()), profile)
    }

    fn captured(command: &str, output: &str) -> Context {
        Context::new(
            ContextSource::Command {
                command: command.into(),
            },
            ContentType::Terminal,
            output,
        )
    }

    #[test]
    fn profiles_claim_their_own_commands() {
        assert!(optimizer(ToolProfile::GIT).supports(&captured("git status", "x")));
        assert!(!optimizer(ToolProfile::GIT).supports(&captured("cargo test", "x")));

        assert!(optimizer(ToolProfile::TEST).supports(&captured("cargo test --all", "x")));
        assert!(optimizer(ToolProfile::TEST).supports(&captured("pytest -q", "x")));
        assert!(optimizer(ToolProfile::BUILD).supports(&captured("cargo build", "x")));
        assert!(optimizer(ToolProfile::BUILD).supports(&captured("npm run build", "x")));
    }

    #[test]
    fn the_generic_profile_claims_any_command() {
        assert!(optimizer(ToolProfile::GENERIC).supports(&captured("some-tool --flag", "x")));
    }

    #[test]
    fn no_profile_claims_content_that_did_not_come_from_a_command() {
        let file = Context::new(ContextSource::Stdin, ContentType::Terminal, "x");
        for profile in ToolProfile::REGISTRY {
            assert!(!optimizer(profile).supports(&file), "{}", profile.name);
        }
    }

    #[test]
    fn json_output_is_left_to_the_json_optimizer() {
        let json = Context::new(
            ContextSource::Command {
                command: "curl -s https://example.test/api".into(),
            },
            ContentType::Json,
            "{\"a\":1}",
        );
        for profile in ToolProfile::REGISTRY {
            assert!(!optimizer(profile).supports(&json), "{}", profile.name);
        }
    }

    #[test]
    fn git_hints_are_dropped_and_changes_are_kept() {
        let output = "On branch main\n\
                      Changes not staged for commit:\n\
                      \x20 (use \"git add <file>...\" to update what will be committed)\n\
                      \x20 (use \"git restore <file>...\" to discard changes)\n\
                      \tmodified:   src/main.rs\n\
                      \n\
                      no changes added to commit (use \"git add\" and/or \"git commit -a\")\n";

        let optimized = optimizer(ToolProfile::GIT)
            .optimize(&captured("git status", output), None)
            .unwrap();

        assert!(optimized.content.contains("modified:   src/main.rs"));
        assert!(optimized.content.contains("On branch main"));
        assert!(!optimized.content.contains("use \\\"git add"));
        assert!(!optimized.content.contains("(use "));
        assert!(optimized.result.savings_by_stage.filtering > 0);
    }

    #[test]
    fn passing_tests_are_dropped_and_failures_are_kept() {
        let output = "running 3 tests\n\
                      test config::parses ... ok\n\
                      test config::rejects_unknown ... ok\n\
                      test store::roundtrip ... FAILED\n\
                      \n\
                      failures:\n\
                      \x20   store::roundtrip\n\
                      test result: FAILED. 2 passed; 1 failed\n";

        let optimized = optimizer(ToolProfile::TEST)
            .optimize(&captured("cargo test", output), None)
            .unwrap();

        assert!(optimized.content.contains("store::roundtrip ... FAILED"));
        assert!(optimized.content.contains("test result: FAILED"));
        assert!(!optimized.content.contains("config::parses"));
    }

    #[test]
    fn build_progress_is_dropped_and_diagnostics_are_kept() {
        let output = "Compiling serde v1.0.0\n\
                      Compiling ctxc-core v0.1.0\n\
                      warning: unused variable: `x`\n\
                      error[E0308]: mismatched types\n\
                      Finished dev profile in 3.4s\n";

        let optimized = optimizer(ToolProfile::BUILD)
            .optimize(&captured("cargo build", output), None)
            .unwrap();

        assert!(optimized.content.contains("error[E0308]"));
        assert!(optimized.content.contains("warning: unused variable"));
        assert!(optimized.content.contains("Finished dev profile"));
        assert!(!optimized.content.contains("Compiling"));
    }

    #[test]
    fn colour_codes_are_stripped() {
        let output = "\u{1b}[32mok\u{1b}[0m: everything fine\n";
        let optimized = optimizer(ToolProfile::GENERIC)
            .optimize(&captured("some-tool", output), None)
            .unwrap();

        assert_eq!(optimized.content, "ok: everything fine\n");
    }

    #[test]
    fn repeats_collapse_with_a_count() {
        let output =
            "warning: unused import\nwarning: unused import\nwarning: unused import\ndone\n";
        let optimized = optimizer(ToolProfile::GENERIC)
            .optimize(&captured("some-tool", output), None)
            .unwrap();

        assert_eq!(
            optimized.content,
            "warning: unused import  (repeated 3 times)\ndone\n"
        );
    }

    #[test]
    fn lines_differing_by_a_number_are_kept_apart() {
        let output = "processed file1.txt\nprocessed file2.txt\nprocessed file3.txt\n";
        let optimized = optimizer(ToolProfile::GENERIC)
            .optimize(&captured("some-tool", output), None)
            .unwrap();

        assert_eq!(optimized.content, output, "file names must survive");
    }

    #[test]
    fn unrecognised_lines_are_never_dropped() {
        let output = "something nobody wrote a rule for\nand another thing\n";
        let optimized = optimizer(ToolProfile::GIT)
            .optimize(&captured("git weird-subcommand", output), None)
            .unwrap();

        assert_eq!(optimized.content, output);
    }

    #[test]
    fn a_budget_applies_to_tool_output_too() {
        let output: String = (0..300)
            .map(|index| format!("distinct diagnostic number {index} here\n"))
            .collect();
        let optimized = optimizer(ToolProfile::GENERIC)
            .optimize(&captured("some-tool", &output), Some(TokenBudget::new(40)))
            .unwrap();

        assert!(optimized.result.optimized_tokens <= 40);
        assert_eq!(
            optimized.result.savings_by_stage.total(),
            optimized.result.tokens_saved()
        );
    }
}
