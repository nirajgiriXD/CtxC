//! Terminal styling for human output.
//!
//! Colour separates one kind of thing from another — a path from the content
//! quoted under it, a label from the number beside it — so the roles here are
//! named for what they mark, not for the colour they happen to use. A renderer
//! asks for [`Palette::path`], never for "cyan", and the palette stays the one
//! place where that mapping lives.
//!
//! Every role degrades to plain text. A [`Palette`] carries one bit, and when
//! it is off each helper writes exactly what an uncoloured build would have
//! written, so `--format json`, a redirected stdout, and `NO_COLOR` all keep
//! producing byte-identical documents.

use std::fmt::{self, Display};
use std::io::IsTerminal;

use anstyle::{AnsiColor, Color, Style};
use clap::ValueEnum;

/// When to colour human output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
#[value(rename_all = "lower")]
pub enum ColorChoice {
    /// Colour when the stream is a terminal that wants it.
    #[default]
    Auto,
    /// Colour even when the stream is a file or a pipe.
    Always,
    /// Never colour.
    Never,
}

/// Which stream a palette is being built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    fn is_terminal(self) -> bool {
        match self {
            Stream::Stdout => std::io::stdout().is_terminal(),
            Stream::Stderr => std::io::stderr().is_terminal(),
        }
    }
}

/// The set of roles a renderer can paint with.
///
/// `Copy`, so a renderer can take one by value at the top of `render_human`
/// and still hand `out` out mutably on every line below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Palette {
    color: bool,
}

/// A palette that never colours: the default for captured output and tests.
pub const PLAIN: Palette = Palette { color: false };

impl Palette {
    /// Decide colour for one stream.
    ///
    /// `auto` asks three questions in the order a user expects them answered:
    /// an explicit `NO_COLOR` wins over everything, `CLICOLOR_FORCE` colours a
    /// pipe on purpose, and otherwise the stream has to be a terminal that
    /// claims to understand ANSI. On Windows the console is switched into
    /// virtual-terminal mode first, because there the answer depends on
    /// whether anyone asked for it.
    pub fn for_stream(stream: Stream, choice: ColorChoice) -> Palette {
        let color = match choice {
            ColorChoice::Never => false,
            ColorChoice::Always => {
                enable_windows_ansi();
                true
            }
            ColorChoice::Auto => {
                if anstyle_query::no_color() {
                    false
                } else if anstyle_query::clicolor_force() {
                    enable_windows_ansi();
                    true
                } else {
                    stream.is_terminal()
                        && anstyle_query::term_supports_color()
                        && enable_windows_ansi()
                }
            }
        };
        Palette { color }
    }

    /// A section title, or the first line of a report.
    pub fn heading<T: Display>(self, value: T) -> Paint<T> {
        self.paint(value, Style::new().bold())
    }

    /// The name of a field, to the left of its value.
    pub fn label<T: Display>(self, value: T) -> Paint<T> {
        self.paint(value, Style::new().dimmed())
    }

    /// A file, directory, or project path: the thing content belongs to.
    pub fn path<T: Display>(self, value: T) -> Paint<T> {
        self.paint(value, fg(AnsiColor::Cyan).bold())
    }

    /// A symbol name: a function, a type, or whatever else the parser found.
    pub fn symbol<T: Display>(self, value: T) -> Paint<T> {
        self.paint(value, fg(AnsiColor::Blue))
    }

    /// A count, size, or duration worth reading off quickly.
    pub fn number<T: Display>(self, value: T) -> Paint<T> {
        self.paint(value, Style::new().bold())
    }

    /// A `ctxc://` reference, or an id that can be pasted back in.
    pub fn reference<T: Display>(self, value: T) -> Paint<T> {
        self.paint(value, fg(AnsiColor::Magenta))
    }

    /// A command the reader is being told to run.
    pub fn command<T: Display>(self, value: T) -> Paint<T> {
        self.paint(value, fg(AnsiColor::Cyan))
    }

    /// Something that worked, or a saving.
    pub fn good<T: Display>(self, value: T) -> Paint<T> {
        self.paint(value, fg(AnsiColor::Green))
    }

    /// Something that works but deserves a look.
    pub fn warn<T: Display>(self, value: T) -> Paint<T> {
        self.paint(value, fg(AnsiColor::Yellow))
    }

    /// Something that failed.
    pub fn bad<T: Display>(self, value: T) -> Paint<T> {
        self.paint(value, fg(AnsiColor::Red).bold())
    }

    /// Secondary detail: a score, a gutter, an aside in parentheses.
    pub fn dim<T: Display>(self, value: T) -> Paint<T> {
        self.paint(value, Style::new().dimmed())
    }

    fn paint<T: Display>(self, value: T, style: Style) -> Paint<T> {
        Paint {
            value,
            style: if self.color { style } else { Style::new() },
        }
    }
}

/// One styled value, rendered when it is written.
pub struct Paint<T> {
    value: T,
    style: Style,
}

impl<T: Display> Display for Paint<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.style.is_plain() {
            return Display::fmt(&self.value, f);
        }
        // Width and alignment belong to the value, not to the escapes around
        // it, so a caller's `{:<28}` still lines its column up with colour on.
        let rendered = self.value.to_string();
        write!(f, "{}", self.style.render())?;
        Display::fmt(&rendered, f)?;
        write!(f, "{}", self.style.render_reset())
    }
}

const fn fg(color: AnsiColor) -> Style {
    Style::new().fg_color(Some(Color::Ansi(color)))
}

/// Put the Windows console into virtual-terminal mode, and say whether ANSI
/// can be used. Always true elsewhere.
fn enable_windows_ansi() -> bool {
    anstyle_query::windows::enable_ansi_colors().unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colored() -> Palette {
        Palette { color: true }
    }

    #[test]
    fn a_plain_palette_writes_the_value_and_nothing_else() {
        assert_eq!(PLAIN.path("src/main.rs").to_string(), "src/main.rs");
        assert_eq!(PLAIN.number(42).to_string(), "42");
        assert_eq!(PLAIN.heading("x").to_string(), "x");
    }

    #[test]
    fn a_colored_palette_wraps_the_value_in_escapes() {
        let painted = colored().path("src/main.rs").to_string();
        assert!(painted.starts_with('\u{1b}'), "{painted:?}");
        assert!(painted.contains("src/main.rs"), "{painted:?}");
        assert!(painted.ends_with('m'), "{painted:?}");
    }

    #[test]
    fn roles_are_distinguishable_from_one_another() {
        let palette = colored();
        let path = palette.path("x").to_string();
        let symbol = palette.symbol("x").to_string();
        let bad = palette.bad("x").to_string();
        assert_ne!(path, symbol);
        assert_ne!(path, bad);
        assert_ne!(symbol, bad);
    }

    #[test]
    fn padding_measures_the_value_not_the_escapes() {
        assert_eq!(format!("{:<6}|", PLAIN.label("ab")), "ab    |");
        let padded = format!("{:<6}|", colored().label("ab"));
        assert_eq!(padded.matches(' ').count(), 4, "{padded:?}");
    }

    #[test]
    fn never_beats_a_terminal_and_always_beats_a_pipe() {
        assert_eq!(
            Palette::for_stream(Stream::Stdout, ColorChoice::Never),
            PLAIN
        );
        assert_eq!(
            Palette::for_stream(Stream::Stdout, ColorChoice::Always),
            colored()
        );
    }
}
