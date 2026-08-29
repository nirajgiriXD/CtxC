//! Machine- and human-readable output.
//!
//! Every command produces one value that can render itself both ways. Results
//! go to stdout; diagnostics go to stderr (see [`crate::logging`]), so piping
//! `--format json` into another tool is always safe.

use std::io::{self, Write};

use clap::ValueEnum;
use serde::Serialize;

use crate::style::{ColorChoice, Palette, Stream};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum OutputFormat {
    /// Formatted for a person reading a terminal.
    Human,
    /// A single pretty-printed JSON document.
    Json,
    /// One compact JSON document per line, for streaming consumers.
    Jsonl,
    /// No output at all; the exit code carries the result.
    Quiet,
}

/// A command result that can render itself for a person.
pub trait Render: Serialize {
    fn render_human(&self, out: &mut dyn Write) -> io::Result<()>;
}

/// Writes command results in the selected format.
pub struct Printer<W: Write> {
    format: OutputFormat,
    writer: W,
    palette: Palette,
    stderr_palette: Palette,
}

impl Printer<io::Stdout> {
    pub fn stdout(format: OutputFormat, color: ColorChoice) -> Self {
        let mut printer = Printer::new(format, io::stdout());
        printer.palette = Palette::for_stream(Stream::Stdout, color);
        printer.stderr_palette = Palette::for_stream(Stream::Stderr, color);
        printer
    }
}

impl<W: Write> Printer<W> {
    /// A printer that writes plain text. Colour is attached by
    /// [`Printer::stdout`], which knows what the streams actually are.
    pub fn new(format: OutputFormat, writer: W) -> Self {
        Printer {
            format,
            writer,
            palette: crate::style::PLAIN,
            stderr_palette: crate::style::PLAIN,
        }
    }

    /// The format this printer was built with.
    ///
    /// Commands whose product is content rather than a report — `optimize`,
    /// `compile` — branch on it, because for them stdout carries the optimized
    /// context and the summary belongs on stderr.
    pub fn format(&self) -> OutputFormat {
        self.format
    }

    /// Write raw content straight through, in every format except quiet-only
    /// contexts that handle it themselves.
    pub fn write_content(&mut self, content: &str) -> io::Result<()> {
        self.writer.write_all(content.as_bytes())?;
        if !content.is_empty() && !content.ends_with('\n') {
            writeln!(self.writer)?;
        }
        self.writer.flush()
    }

    /// Say what to do next, on stderr.
    ///
    /// An empty result is not an error, but it is rarely where someone meant
    /// to stop. The suggestion goes to stderr so that a piped stdout stays
    /// exactly what it was, and only for human output, so a machine consumer
    /// reads byte-identical documents either way.
    ///
    /// Best effort: a command's result must not depend on whether advice
    /// about it could be printed.
    pub fn hint<S: AsRef<str>>(&mut self, lines: &[S]) {
        if self.format != OutputFormat::Human || lines.is_empty() {
            return;
        }
        let mut stderr = io::stderr();
        let _ = writeln!(stderr);
        for line in lines {
            let _ = writeln!(stderr, "{}", self.stderr_palette.dim(line.as_ref()));
        }
    }

    /// Emit one result.
    pub fn emit<T: Render>(&mut self, value: &T) -> io::Result<()> {
        match self.format {
            OutputFormat::Human => {
                value.render_human(&mut self.writer)?;
            }
            OutputFormat::Json => {
                let json = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
                writeln!(self.writer, "{json}")?;
            }
            OutputFormat::Jsonl => {
                let json = serde_json::to_string(value).map_err(io::Error::other)?;
                writeln!(self.writer, "{json}")?;
            }
            OutputFormat::Quiet => {}
        }
        self.writer.flush()
    }
}

/// Format a byte count for human output.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Format a count with thousands separators, so large token numbers stay
/// readable in a terminal.
pub fn human_count(value: u32) -> String {
    group(&value.to_string())
}

/// The same, for totals that are wider than a `u32` and may be negative —
/// aggregated token counts add up past four billion, and a stage can cost
/// tokens rather than save them.
pub fn human_total(value: i64) -> String {
    let grouped = group(&value.unsigned_abs().to_string());
    if value < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

fn group(digits: &str) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (position, digit) in digits.chars().enumerate() {
        if position > 0 && (digits.len() - position) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Format a ratio in the 0.0..=1.0 range as a percentage.
pub fn human_percent(ratio: f64) -> String {
    format!("{:.1}%", ratio * 100.0)
}

/// Describe a stage's effect, which may be a cost rather than a saving.
pub fn human_delta(tokens: i32) -> String {
    if tokens < 0 {
        format!("{} added", human_count(tokens.unsigned_abs()))
    } else {
        format!("{} saved", human_count(tokens as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct Sample {
        name: &'static str,
        count: u32,
    }

    impl Render for Sample {
        fn render_human(&self, out: &mut dyn Write) -> io::Result<()> {
            writeln!(out, "{}: {}", self.name, self.count)
        }
    }

    fn emit(format: OutputFormat) -> String {
        let mut buffer = Vec::new();
        Printer::new(format, &mut buffer)
            .emit(&Sample {
                name: "contexts",
                count: 3,
            })
            .unwrap();
        String::from_utf8(buffer).unwrap()
    }

    #[test]
    fn human_output_is_rendered() {
        assert_eq!(emit(OutputFormat::Human), "contexts: 3\n");
    }

    #[test]
    fn json_is_pretty_and_jsonl_is_one_line() {
        let json = emit(OutputFormat::Json);
        assert!(json.contains("\n  \"name\": \"contexts\""), "{json}");

        let jsonl = emit(OutputFormat::Jsonl);
        assert_eq!(jsonl, "{\"name\":\"contexts\",\"count\":3}\n");
    }

    #[test]
    fn quiet_writes_nothing() {
        assert_eq!(emit(OutputFormat::Quiet), "");
    }

    #[test]
    fn counts_are_grouped() {
        assert_eq!(human_count(0), "0");
        assert_eq!(human_count(999), "999");
        assert_eq!(human_count(1_000), "1,000");
        assert_eq!(human_count(18_420), "18,420");
        assert_eq!(human_count(1_234_567), "1,234,567");
    }

    #[test]
    fn wide_totals_keep_their_sign() {
        assert_eq!(human_total(0), "0");
        assert_eq!(human_total(52_500_000), "52,500,000");
        assert_eq!(human_total(-2_000), "-2,000");
        assert_eq!(human_total(i64::MIN), "-9,223,372,036,854,775,808");
    }

    #[test]
    fn percentages_have_one_decimal() {
        assert_eq!(human_percent(0.607), "60.7%");
        assert_eq!(human_percent(0.0), "0.0%");
        assert_eq!(human_percent(1.0), "100.0%");
    }

    #[test]
    fn content_is_written_with_a_trailing_newline() {
        let mut buffer = Vec::new();
        Printer::new(OutputFormat::Human, &mut buffer)
            .write_content("no newline")
            .unwrap();
        assert_eq!(String::from_utf8(buffer).unwrap(), "no newline\n");

        let mut buffer = Vec::new();
        Printer::new(OutputFormat::Human, &mut buffer)
            .write_content("")
            .unwrap();
        assert_eq!(String::from_utf8(buffer).unwrap(), "");
    }

    #[test]
    fn byte_counts_are_scaled() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(1024 * 1024 * 3 / 2), "1.5 MB");
    }
}
