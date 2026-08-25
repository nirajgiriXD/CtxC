//! `ctxc optimize --dry-run <input>`

use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context as _, Result};

use ctxc_engine::{Analysis, Engine};
use ctxc_metrics::{MetricEvent, Operation};

use crate::app::App;
use crate::output::{human_count, human_delta, human_percent, Printer, Render};

impl Render for Analysis {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
        writeln!(out, "{}", self.source)?;
        writeln!(out)?;
        writeln!(out, "Type:        {}", self.content_type.as_str())?;
        writeln!(out, "Size:        {} bytes", human_count(self.bytes as u32))?;
        writeln!(out, "Lines:       {}", human_count(self.lines))?;
        writeln!(
            out,
            "Fragments:   {}{}",
            human_count(self.fragments),
            match self.duplicate_fragments {
                0 => String::new(),
                duplicates => format!("  ({} duplicated)", human_count(duplicates)),
            }
        )?;
        writeln!(
            out,
            "Tokens:      {}{}",
            human_count(self.tokens),
            if self.estimated { "  (estimated)" } else { "" }
        )?;
        writeln!(out)?;
        writeln!(out, "Optimizer:   {}", self.optimizer)?;
        writeln!(
            out,
            "Projected:   {} tokens  ({} smaller)",
            human_count(self.projected_tokens),
            human_percent(self.projected_reduction)
        )?;

        let savings = &self.projected_savings_by_stage;
        if savings.total() != 0 {
            writeln!(out)?;
            writeln!(out, "Savings by stage:")?;
            for (stage, tokens) in [
                ("filtering", savings.filtering),
                ("deduplication", savings.deduplication),
                ("compression", savings.compression),
                ("selection", savings.selection),
            ] {
                if tokens != 0 {
                    writeln!(out, "  {stage:<16}{}", human_delta(tokens))?;
                }
            }
        }
        Ok(())
    }
}

pub fn run<W: Write>(
    app: &App,
    input: Option<&Path>,
    from: Option<&str>,
    printer: &mut Printer<W>,
) -> Result<()> {
    let context = super::input::load_from(input, from)?;

    let engine = Engine::from_config(&app.config);
    let started = std::time::Instant::now();
    let analysis = engine
        .analyze(&context)
        .context("failed to analyze input")?;

    // Analysis reports what optimization *would* save, so it is recorded with
    // the projection rather than an actual reduction — the operation name keeps
    // the two apart in every report.
    let mut event = MetricEvent::new(Operation::Analyze, &analysis.source)
        .with_tokens(analysis.tokens, analysis.projected_tokens)
        .took(started.elapsed());
    event.optimizer = Some(analysis.optimizer.clone());
    event.savings_by_stage = analysis.projected_savings_by_stage;
    event.estimated = analysis.estimated;
    app.record(event);

    printer.emit(&analysis)?;
    Ok(())
}
