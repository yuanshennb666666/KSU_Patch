//! Shared error type and exit-code table.
//!
//! Every failure carries the stage it happened in, the input involved, the reason, and a
//! concrete hint -- the tool is meant to be usable without reading its source.

pub const OK: u8 = 0;
pub const USAGE: u8 = 1;
pub const INPUT: u8 = 2;
pub const APK: u8 = 3;
pub const IMAGE: u8 = 4;
pub const KMI: u8 = 5;
pub const ASSET: u8 = 6;
pub const INJECT: u8 = 7;
pub const IO: u8 = 8;

pub struct Fail {
    pub exit: u8,
    pub stage: &'static str,
    pub input: String,
    pub reason: String,
    pub hint: String,
}

pub type Res<T> = Result<T, Fail>;

pub fn fail(
    exit: u8,
    stage: &'static str,
    input: impl Into<String>,
    reason: impl Into<String>,
    hint: impl Into<String>,
) -> Fail {
    Fail {
        exit,
        stage,
        input: input.into(),
        reason: reason.into(),
        hint: hint.into(),
    }
}

/// Renders a failure on stderr.
///
/// The compact `label  value` form matches the rest of the output. Colours follow stderr's
/// own terminal state rather than stdout's, so `2>err.log` never collects escape sequences
/// even when the normal report is still going to a console.
pub fn report(f: &Fail) {
    use crate::style;
    use std::io::IsTerminal;

    let color = style::on() && std::io::stderr().is_terminal();
    let paint = |f: fn(&str) -> String, s: &str| {
        if color {
            f(s)
        } else {
            s.to_string()
        }
    };
    eprintln!();
    eprintln!(
        "{}  {}",
        paint(style::bold_red, "ERROR"),
        paint(style::bold, &f.stage)
    );
    eprintln!("  {:<7}  {}", paint(style::dim, "Input"), &f.input);
    eprintln!(
        "  {:<7}  {}",
        paint(style::dim, "Cause"),
        paint(style::bold_red, &f.reason)
    );
    eprintln!(
        "  {:<7}  {}",
        paint(style::dim, "Hint"),
        paint(style::bold_yellow, &f.hint)
    );
    eprintln!(
        "  {:<7}  {}",
        paint(style::dim, "Exit"),
        paint(style::dim, &f.exit.to_string())
    );
}
