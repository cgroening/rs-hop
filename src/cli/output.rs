//! What a command writes and how it fails: the two output streams, the error
//! format and the exit-code contract.
//!
//! stdout carries only payload - the result a caller would pipe onwards. Every
//! hint, confirmation, warning and error goes to stderr, so `hop list | …`
//! never sees a message mixed into the data.

use std::io::{self, Write};
use std::process::ExitCode;

use sparcli::{
    Alert, Color as UiColor, Renderable, Style as SpStyle, Theme, set_theme,
};

use crate::config::Config;
use crate::domain::error::Error;
use crate::theme::{Color, GlyphVariant, Palette};
use crate::util::app_info::APP_NAME;

/// Exit code for a successful run.
pub const EXIT_OK: u8 = 0;
/// Exit code for a general runtime failure.
pub const EXIT_ERROR: u8 = 1;
/// Exit code for a usage or argument error (clap uses the same code).
pub const EXIT_USAGE: u8 = 2;
/// Exit code after `Ctrl+C`, by the shell convention of 128 + SIGINT.
pub const EXIT_INTERRUPTED: u8 = 130;

/// Where a command writes: payload to `out`, everything else to `err`.
///
/// Both are injected rather than taken from the process, so a test can drive a
/// handler and assert on the two streams separately.
pub struct Streams<'a> {
    /// The payload stream (the process's stdout in a real run).
    pub out: &'a mut dyn Write,
    /// The message stream (the process's stderr in a real run).
    pub err: &'a mut dyn Write,
}

impl Streams<'_> {
    /// Writes one payload line to stdout.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`io::Error`] if the stream cannot be written,
    /// for instance when a downstream pipe has already closed.
    pub fn line(&mut self, text: &str) -> io::Result<()> {
        writeln!(self.out, "{text}")
    }

    /// Renders a sparcli widget as a message on stderr.
    ///
    /// # Errors
    ///
    /// Returns a [`CliError`] if the stream cannot be written.
    pub fn message(&mut self, widget: &impl Renderable) -> CliResult {
        widget
            .print_to(&mut self.err)
            .map_err(|error| CliError::runtime(error.to_string()))
    }

    /// Renders a sparcli widget as payload on stdout.
    ///
    /// Reserved for widgets that *are* the result, such as the entry table.
    ///
    /// # Errors
    ///
    /// Returns a [`CliError`] if the stream cannot be written.
    pub fn payload(&mut self, widget: &impl Renderable) -> CliResult {
        widget
            .print_to(&mut self.out)
            .map_err(|error| CliError::runtime(error.to_string()))
    }
}

/// A failed command: what went wrong, and the exit code it should produce.
#[derive(Debug)]
pub struct CliError {
    /// The message, without the `hop: error:` prefix.
    message: String,
    /// The process exit code, per the contract in `README.md`.
    code: u8,
}

impl CliError {
    /// A general runtime failure (exit 1).
    pub fn runtime(message: impl Into<String>) -> Self {
        CliError {
            message: message.into(),
            code: EXIT_ERROR,
        }
    }

    /// A usage error (exit 2): a missing value, or a prompt with no terminal
    /// to show it on.
    pub fn usage(message: impl Into<String>) -> Self {
        CliError {
            message: message.into(),
            code: EXIT_USAGE,
        }
    }

    /// The exit code this failure should end the process with.
    pub fn code(&self) -> u8 {
        self.code
    }
}

impl From<Error> for CliError {
    fn from(error: Error) -> Self {
        CliError::runtime(error.to_string())
    }
}

impl From<io::Error> for CliError {
    fn from(error: io::Error) -> Self {
        CliError::runtime(error.to_string())
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// The result of a command handler.
pub type CliResult = Result<(), CliError>;

/// Reports `error` on stderr in the `hop: error: …` form and returns its exit
/// code.
///
/// The plain prefixed line is deliberate: a boxed panel cannot be grepped, and
/// wraps badly once the message outgrows the terminal.
pub fn report(error: &CliError, err: &mut dyn Write) -> ExitCode {
    let _ = writeln!(err, "{APP_NAME}: error: {error}");
    ExitCode::from(error.code)
}

/// Builds a sparcli theme from the config palette and installs it globally, so
/// CLI output shares the TUI's colors.
///
/// `NO_COLOR` and non-terminal output are handled by sparcli itself.
pub fn apply_sparcli_theme(config: &Config) {
    set_theme(sparcli_theme(&config.palette(), config.appearance.glyphs));
}

/// The sparcli theme for `palette`: the accent, the message roles, the
/// widget styles and the dimmed text (`dim`, when set).
fn sparcli_theme(palette: &Palette, glyphs: GlyphVariant) -> Theme {
    let defaults = Theme::default();
    let fg = |color: Color| SpStyle::new().fg(map_color(color));
    // Dimmed text takes the palette's `dim` once it is set, like the TUI.
    let dimmed = (palette.dim != Color::Default).then(|| fg(palette.dim));
    Theme {
        accent: map_color(palette.accent),
        title: fg(palette.accent).bold(),
        heading: fg(palette.accent).bold(),
        secondary: fg(palette.foreground_dim),
        success: fg(palette.success),
        error: fg(palette.error),
        warning: fg(palette.warning),
        info: fg(palette.info),
        hint: dimmed.unwrap_or(defaults.hint),
        selection: fg(palette.accent).bold(),
        cursor: fg(palette.foreground.readable_on(palette.cursor))
            .bg(map_color(palette.cursor)),
        diff_add: fg(palette.success),
        diff_delete: fg(palette.error),
        progress_empty: map_color(palette.foreground_muted),
        code: fg(palette.info),
        stripe: dimmed.unwrap_or(defaults.stripe),
        unicode: matches!(glyphs, GlyphVariant::Unicode),
        ..defaults
    }
}

/// Maps a resolved palette [`Color`] to sparcli's color: truecolor to `Rgb`,
/// an `ansi:` color to the same terminal color, the terminal default to
/// `Reset`.
fn map_color(color: Color) -> UiColor {
    match color {
        Color::Rgb(red, green, blue) => UiColor::Rgb(red, green, blue),
        Color::Ansi(_) => {
            UiColor::from_name(&color.to_hex()).unwrap_or(UiColor::Reset)
        }
        Color::Default => UiColor::Reset,
    }
}

/// Writes an informational message to stderr, ignoring a closed stream.
pub fn info(streams: &mut Streams, message: impl Into<String>) {
    let _ = streams.message(&Alert::info(message.into()));
}

/// Writes a success message to stderr, ignoring a closed stream.
pub fn success(streams: &mut Streams, message: impl Into<String>) {
    let _ = streams.message(&Alert::success(message.into()));
}

/// Writes a warning to stderr, ignoring a closed stream.
pub fn warning(streams: &mut Streams, message: impl Into<String>) {
    let _ = streams.message(&Alert::warning(message.into()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cli_theme_follows_the_palette_and_dim() {
        let mut config = Config::default();
        config
            .appearance
            .colors
            .insert("error".to_string(), "ansi:magenta".to_string());
        config
            .appearance
            .colors
            .insert("dim".to_string(), "#040506".to_string());
        let theme = sparcli_theme(&config.palette(), config.appearance.glyphs);
        assert_eq!(theme.error, SpStyle::new().fg(UiColor::Magenta));
        assert_eq!(theme.hint, SpStyle::new().fg(UiColor::Rgb(4, 5, 6)));
    }

    #[test]
    fn an_error_is_reported_as_a_greppable_prefixed_line() {
        let mut err: Vec<u8> = Vec::new();
        let code = report(&CliError::runtime("no such slug 'zzz'"), &mut err);
        assert_eq!(
            String::from_utf8(err).unwrap(),
            "hop: error: no such slug 'zzz'\n"
        );
        assert_eq!(code, ExitCode::from(EXIT_ERROR));
    }

    #[test]
    fn a_usage_error_carries_exit_code_two() {
        assert_eq!(CliError::usage("needs a terminal").code(), EXIT_USAGE);
        assert_eq!(CliError::runtime("boom").code(), EXIT_ERROR);
    }

    #[test]
    fn payload_and_messages_land_on_separate_streams() {
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let mut streams = Streams {
            out: &mut out,
            err: &mut err,
        };
        streams.line("payload").unwrap();
        info(&mut streams, "a hint");
        assert_eq!(String::from_utf8(out).unwrap(), "payload\n");
        assert!(String::from_utf8(err).unwrap().contains("a hint"));
    }
}
