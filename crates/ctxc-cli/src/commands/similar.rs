//! `ctxc similar`
//!
//! Find the files closest to a piece of text, by embedding similarity rather
//! than by keyword. Useful for "what else looks like this?" — near-duplicate
//! code, the other three places a pattern was copied to, the file a bug report
//! is probably about.
//!
//! Every result here carries what its number is worth. With the embedder CtxC
//! ships, similarity is *lexical*: it finds shared wording, not shared meaning.
//! Saying so is not a disclaimer, it is the difference between a user reading
//! an empty result as "there is nothing" and as "nothing that uses these
//! words".

use std::io::{self, Write};
use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use ctxc_engine::{index, ProjectEmbedder, Similar};
use ctxc_semantic::SemanticOptions;
use ctxc_store::SqliteEmbeddingStore;

use crate::app::App;
use crate::error::CliError;
use crate::output::{human_percent, Printer, Render};

/// What `ctxc similar` found.
#[derive(Debug, Serialize)]
pub struct SimilarReport {
    pub query: String,
    pub root: String,
    #[serde(flatten)]
    pub found: Similar,
}

impl Render for SimilarReport {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        if self.found.files.is_empty() {
            writeln!(out, "Nothing in {} is close to that.", self.root)?;
            writeln!(out)?;
            return writeln!(out, "Note: {}.", self.found.fidelity.caveat());
        }

        for (path, similarity) in &self.found.files {
            writeln!(out, "  {:<52}{}", path, human_percent(*similarity as f64))?;
        }

        writeln!(out)?;
        writeln!(
            out,
            "{} result(s), scored by the `{}` embedder.",
            self.found.files.len(),
            self.found.provider
        )?;
        // Shown every time, not only when results are thin. A person who reads
        // "94%" and thinks it means "says the same thing" will trust the wrong
        // file.
        writeln!(out, "Note: {}.", self.found.fidelity.caveat())
    }
}

pub fn run<W: Write>(
    app: &App,
    query: &str,
    root: &Path,
    limit: usize,
    printer: &mut Printer<W>,
) -> Result<()> {
    let options = SemanticOptions::from_config(&app.config);
    if !options.enabled {
        return Err(CliError::new("embeddings are switched off")
            .with_hint("set semantic.enabled = true, then run `ctxc index` to build them")
            .into());
    }

    let database = app.open_database()?;
    let store = SqliteEmbeddingStore::new(&database);
    let key = index::root_key(root);

    let Some(embedder) = ProjectEmbedder::from_options(&options, &store, &key)? else {
        // Unreachable while the check above stands, but the two conditions are
        // not the same thing and tying them together would be a bug waiting.
        return Err(CliError::new("embeddings are switched off").into());
    };

    if embedder.count()? == 0 {
        return Err(
            CliError::new(format!("{} has no embeddings", root.display()))
                .with_hint("run `ctxc index` to build them")
                .into(),
        );
    }

    // A floor, because everything is a little similar to everything and a weak
    // match presented as a result reads as evidence.
    let found = embedder.nearest(query, limit, 0.15)?;

    printer.emit(&SimilarReport {
        query: query.to_owned(),
        root: key,
        found,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::OutputFormat;
    use ctxc_semantic::Fidelity;

    fn render(found: Similar) -> String {
        let mut buffer = Vec::new();
        Printer::new(OutputFormat::Human, &mut buffer)
            .emit(&SimilarReport {
                query: "authenticate".into(),
                root: "/work/app".into(),
                found,
            })
            .unwrap();
        String::from_utf8(buffer).unwrap()
    }

    #[test]
    fn results_always_say_what_their_numbers_are_worth() {
        let text = render(Similar {
            fidelity: Fidelity::Lexical,
            provider: "hashed".into(),
            files: vec![("src/auth.rs".into(), 0.94)],
        });

        assert!(text.contains("src/auth.rs"), "{text}");
        assert!(text.contains("94.0%"), "{text}");
        assert!(
            text.contains("not shared meaning"),
            "a reader must not take a lexical score for a semantic one: {text}"
        );
    }

    #[test]
    fn an_empty_result_explains_itself_rather_than_reading_as_nothing_exists() {
        let text = render(Similar {
            fidelity: Fidelity::Lexical,
            provider: "hashed".into(),
            files: Vec::new(),
        });

        assert!(text.contains("Nothing in /work/app"), "{text}");
        assert!(
            text.contains("not shared meaning"),
            "the caveat matters most when there are no results: {text}"
        );
    }

    #[test]
    fn a_model_backed_result_says_something_different() {
        let text = render(Similar {
            fidelity: Fidelity::Semantic,
            provider: "some-model".into(),
            files: vec![("src/auth.rs".into(), 0.8)],
        });

        assert!(text.contains("local model"), "{text}");
        assert!(!text.contains("not shared meaning"), "{text}");
    }
}
