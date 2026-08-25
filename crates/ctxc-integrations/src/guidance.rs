//! What CtxC tells an agent about itself.
//!
//! This is the entire payload of every integration, so it is worth being blunt
//! about what it is: instructions an agent will read as if a person wrote them.
//! It should therefore say what CtxC is *for*, give commands that work, and be
//! honest about the parts that are estimates or lossy. An agent that trusts a
//! token count as exact, or that does not know optimization is reversible, will
//! make worse decisions than one told nothing at all.
//!
//! It deliberately does not tell the agent to run CtxC on everything. An agent
//! is a reader, not a pipeline; the useful advice is *when* reaching for CtxC
//! beats reading files directly.

/// Bumped when the text changes, so an installed block can be recognised as
/// out of date without diffing prose.
pub const VERSION: u32 = 2;

/// The guidance CtxC writes into an agent's instruction file.
///
/// `project` names the project as CtxC knows it, which is what makes the block
/// readable when an agent has several open.
pub fn body(project: &str) -> String {
    format!(
        "\
## Context lookup (CtxC)

CtxC is a local context compiler available in `{project}`. Prefer these over
opening files at random or pasting whole command output.

If a command below reports that the project has not been indexed, run
`ctxc project index .` once and try again.

**Find the code relevant to a task.** This searches text, symbols, and the
dependency graph together, then prints the best of it inside a token budget:

```
ctxc find \"authentication timeout\" --compile
```

Without `--compile` it lists the files and why each one matched, which is
cheaper when you only need to decide what to open.

**Shrink noisy output before reading it.** Build logs, test runs, and diffs are
mostly repetition:

```
git diff | ctxc optimize
ctxc optimize -- npm test
```

**Get the original back.** Optimization removes content on purpose, and every
optimized output ends with a reference to what it came from:

```
ctxc find ctxc://context/<id>
```

Use it whenever a detail looks like it was dropped, rather than guessing at what
was there.

**What to trust.** Token counts CtxC reports are estimates from a heuristic, not
the target model's tokenizer — treat them as approximate. Search results are
ranked, not authoritative: if the answer is not among them, the index may be
stale, and `ctxc project index .` refreshes it.

CtxC is local. It reads this project and writes to a database on this machine;
it sends nothing anywhere.\
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_is_named_so_a_reader_knows_which_one() {
        let text = body("acme-web");
        assert!(text.contains("`acme-web`"), "{text}");
    }

    #[test]
    fn it_never_claims_the_project_is_already_indexed() {
        // Integrations install into projects CtxC has never seen. Telling an
        // agent the index exists when it does not sends it down a dead end.
        let text = body("acme-web");
        assert!(
            !text.contains("is indexed by CtxC"),
            "the guidance must not assert something install cannot know: {text}"
        );
        assert!(text.contains("has not been indexed"), "{text}");
    }

    #[test]
    fn every_command_it_suggests_is_one_ctxc_has() {
        let text = body("demo");
        for command in ["ctxc find", "ctxc optimize", "ctxc project index"] {
            assert!(text.contains(command), "{command} is missing: {text}");
        }
    }

    #[test]
    fn it_says_the_numbers_are_estimates() {
        let text = body("demo");
        assert!(
            text.contains("estimates"),
            "an agent told a token count is exact will plan around a wrong number"
        );
        assert!(
            text.contains("ctxc://context/"),
            "an agent that cannot undo optimization will treat it as lossy and avoid it"
        );
    }

    #[test]
    fn it_does_not_promise_anything_about_the_network() {
        let text = body("demo");
        assert!(text.contains("sends nothing anywhere"), "{text}");
    }
}
