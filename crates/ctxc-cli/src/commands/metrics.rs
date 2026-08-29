//! `ctxc status --metrics`
//!
//! What CtxC has actually saved, and where the saving came from. The command
//! rolls up before it reads: someone running metrics without a daemon should
//! still see everything they have done, not everything up to the last time a
//! daemon happened to be running.

use std::io::{self, Write};

use anyhow::Result;

use ctxc_metrics::report::{Breakdown, Summary, Timeseries};
use ctxc_metrics::{Granularity, MetricEvent, Metrics, Window};
use ctxc_project::{ProjectStore, Registry};
use ctxc_store::{Database, SqliteMetricsStore, SqliteProjectStore};

use crate::app::App;
use crate::cli::MetricsOptions;
use crate::output::{human_percent, human_total, Printer, Render};
use crate::style::Palette;

/// Everything `ctxc status --metrics` reports, in one document.
#[derive(Debug, serde::Serialize)]
pub struct MetricsReport {
    /// The project this covers, by name, or `None` for everything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub days: u32,
    pub summary: Summary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breakdown: Option<Breakdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeseries: Option<Timeseries>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub activity: Vec<MetricEvent>,
}

impl Render for MetricsReport {
    fn render_human(&self, out: &mut dyn Write, palette: Palette) -> io::Result<()> {
        match &self.project {
            Some(project) => writeln!(
                out,
                "{} {}",
                palette.path(project),
                palette.dim(format!("— last {} days", self.days))
            )?,
            None => writeln!(
                out,
                "{} {}",
                palette.heading("CtxC metrics"),
                palette.dim(format!("— last {} days", self.days))
            )?,
        }
        writeln!(out)?;

        if self.summary.is_empty() {
            return writeln!(
                out,
                "Nothing recorded yet. Optimize something, or start the daemon."
            );
        }

        let summary = &self.summary;
        {
            let mut field = |name: &str, value: String| -> io::Result<()> {
                writeln!(out, "{}{value}", palette.label(format!("{name:<18}")))
            };
            field(
                "Operations",
                palette
                    .number(human_total(summary.operations as i64))
                    .to_string(),
            )?;
            field(
                "Tokens in",
                palette
                    .number(human_total(summary.input_tokens))
                    .to_string(),
            )?;
            field(
                "Tokens out",
                palette
                    .number(human_total(summary.output_tokens))
                    .to_string(),
            )?;
            field(
                "Tokens saved",
                palette.good(human_total(summary.tokens_saved)).to_string(),
            )?;
            field(
                "Reduction",
                format!(
                    "{}{}",
                    palette.good(human_percent(summary.reduction_ratio)),
                    if summary.estimated {
                        format!("  {}", palette.dim("(token counts are estimates)"))
                    } else {
                        String::new()
                    }
                ),
            )?;

            if let Some(average) = summary.average_duration_ms {
                field(
                    "Latency",
                    format!(
                        "{} {}, {} {}",
                        palette.number(format!("{average:.0} ms")),
                        palette.dim("average"),
                        palette.number(format!("{} ms", human_total(summary.slowest_duration_ms))),
                        palette.dim("slowest")
                    ),
                )?;
            }
            if let Some(rate) = summary.cache_hit_rate {
                field(
                    "Cache hits",
                    palette.number(human_percent(rate)).to_string(),
                )?;
            }
            if summary.errors > 0 || summary.degradations > 0 {
                field(
                    "Errors",
                    format!(
                        "{} {}, {} {}",
                        palette.bad(human_total(summary.errors as i64)),
                        palette.dim("failed"),
                        palette.warn(human_total(summary.degradations as i64)),
                        palette.dim("degraded")
                    ),
                )?;
            }

            match &summary.estimated_cost_saved {
                Some(cost) => field(
                    "Cost saved",
                    format!(
                        "{}  {}",
                        palette.good(cost.to_display()),
                        palette.dim(format!("(estimate, {} rates)", cost.model))
                    ),
                )?,
                None => field(
                    "Cost saved",
                    format!(
                        "{}  {}",
                        palette.dim("not estimated"),
                        palette.dim("(set metrics.cost_per_million_input_tokens)")
                    ),
                )?,
            }
        }

        let savings = &summary.savings_by_stage;
        if savings.total() != 0 {
            writeln!(out)?;
            writeln!(out, "{}", palette.heading("Savings by stage:"))?;
            for (stage, tokens) in savings.labelled() {
                if tokens != 0 {
                    let total = human_total(tokens);
                    let total = if tokens < 0 {
                        palette.warn(total)
                    } else {
                        palette.good(total)
                    };
                    writeln!(out, "  {:<16}{total}", palette.label(stage))?;
                }
            }
        }

        if let Some(breakdown) = &self.breakdown {
            writeln!(out)?;
            writeln!(out, "{}", palette.heading("By operation:"))?;
            for row in &breakdown.by_operation {
                writeln!(
                    out,
                    "  {:<14}{:>9} {}  {:>14} {}  {:>7}",
                    palette.symbol(row.operation.as_str()),
                    palette.number(human_total(row.operations as i64)),
                    palette.dim("runs"),
                    palette.good(human_total(row.tokens_saved)),
                    palette.dim("saved"),
                    palette.good(human_percent(row.reduction_ratio)),
                )?;
            }
        }

        if let Some(series) = &self.timeseries {
            writeln!(out)?;
            writeln!(
                out,
                "{}",
                palette.heading(format!("By {}:", series.granularity.as_str()))
            )?;
            for point in &series.points {
                writeln!(
                    out,
                    "  {}  {:>9} {}  {:>14} {}",
                    palette.dim(point.bucket_start.to_rfc3339()),
                    palette.number(human_total(point.operations as i64)),
                    palette.dim("runs"),
                    palette.good(human_total(point.tokens_saved)),
                    palette.dim("saved"),
                )?;
            }
        }

        if !self.activity.is_empty() {
            writeln!(out)?;
            writeln!(out, "{}", palette.heading("Recent activity:"))?;
            // When, what, to which input, and what it was worth. The source is
            // the column a reader scans for, so it is the one painted as a path.
            for event in &self.activity {
                writeln!(
                    out,
                    "  {}  {:<10}{:<28}{:>12} {}",
                    palette.dim(event.recorded_at.to_rfc3339()),
                    palette.symbol(event.operation.as_str()),
                    palette.path(truncate(&event.source, 26)),
                    palette.good(human_total(event.tokens_saved())),
                    palette.dim("saved"),
                )?;
            }
        }

        Ok(())
    }
}

/// Keep a source label from wrapping the line it belongs to.
fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_owned();
    }
    let kept: String = value.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}

pub fn run<W: Write>(app: &App, options: &MetricsOptions, printer: &mut Printer<W>) -> Result<()> {
    let database = app.open_database()?;

    let project = match &options.project {
        Some(reference) => {
            let store = SqliteProjectStore::new(&database);
            Some(Registry::new(&store).resolve(reference)?)
        }
        None => None,
    };
    let project_id = project
        .as_ref()
        .map(|project| project.id.as_str().to_owned());

    let store = SqliteMetricsStore::new(&database);
    let metrics = Metrics::new(&store, &app.config);

    // Aggregates are what every report reads, so bring them up to date first.
    // A CLI-only installation has no daemon to have done it.
    metrics.maintain()?;

    let window = Window::last_days(options.days);
    let scope = project_id.as_deref();

    let report = MetricsReport {
        project: project.as_ref().map(|project| project.name.clone()),
        days: options.days,
        summary: metrics.summary(window, scope)?,
        breakdown: options
            .breakdown
            .then(|| metrics.breakdown(window, scope))
            .transpose()?,
        timeseries: options
            .by
            .map(|granularity| metrics.timeseries(granularity.into(), window, scope))
            .transpose()?,
        activity: match options.activity {
            0 => Vec::new(),
            limit => metrics.activity(scope, limit)?,
        },
    };

    printer.emit(&report)?;
    Ok(())
}

/// The project a root belongs to, when it is registered.
///
/// Indexing and searching work on any directory, registered or not, so this
/// answers "which project should this be attributed to?" and accepts `None`.
/// A lookup failure is not worth failing the command over — it costs the
/// attribution, nothing else.
pub fn project_for_root(database: &Database, root_key: &str) -> Option<String> {
    let store = SqliteProjectStore::new(database);
    match store.project_by_path(root_key) {
        Ok(project) => project.map(|project| project.id.as_str().to_owned()),
        Err(err) => {
            tracing::debug!(error = %err, "could not attribute the operation to a project");
            None
        }
    }
}

/// How a timeseries is bucketed, as the CLI spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum Bucket {
    Hour,
    Day,
}

impl From<Bucket> for Granularity {
    fn from(bucket: Bucket) -> Self {
        match bucket {
            Bucket::Hour => Granularity::Hour,
            Bucket::Day => Granularity::Day,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_sources_are_shortened_rather_than_wrapped() {
        assert_eq!(truncate("git status", 26), "git status");
        assert_eq!(truncate(&"x".repeat(30), 5), "xxxx…");
    }

    #[test]
    fn cli_buckets_map_onto_granularities() {
        assert_eq!(Granularity::from(Bucket::Hour), Granularity::Hour);
        assert_eq!(Granularity::from(Bucket::Day), Granularity::Day);
    }
}
