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

/// Renders a failure as a block, on stderr.
///
/// The whole block is red, frame included: an error block that frames itself in red but
/// leaves its body uncoloured reads as two mismatched halves.
pub fn report(f: &Fail) {
    use crate::style;
    let line = "-".repeat(72);
    eprintln!();
    eprintln!("{}", style::red(&line));
    eprintln!("{}", style::bold_red("ERROR"));
    eprintln!("{}", style::red(&line));
    eprintln!("  {} {}", style::dim_red("stage  :"), style::red(&f.stage));
    eprintln!("  {} {}", style::dim_red("input  :"), style::red(&f.input));
    eprintln!("  {} {}", style::dim_red("reason :"), style::bold_red(&f.reason));
    eprintln!("  {} {}", style::dim_red("hint   :"), style::red(&f.hint));
    eprintln!(
        "  {} {}",
        style::dim_red("exit   :"),
        style::red(&f.exit.to_string())
    );
    eprintln!("{}", style::red(&line));
}
