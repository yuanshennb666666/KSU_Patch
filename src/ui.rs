//! Terminal presentation.
//!
//! The output is deliberately plain ASCII: Windows `cmd.exe` often runs in a legacy code
//! page, where box-drawing characters and check marks turn into mojibake. The layout is the
//! one common to modern CLI tools: a short title, numbered steps, aligned `label  value`
//! rows, and a final summary block. Colour is added by [`crate::style`] only when stdout is
//! a terminal.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::style;

/// Width of a horizontal rule.
pub const WIDTH: usize = 64;
/// Left indent used for fields inside a step.
const INDENT: &str = "      ";

static VERBOSE: AtomicBool = AtomicBool::new(false);

/// Select the detailed eight-step trace (`--verbose`); the default is one summary screen.
pub fn set_verbose(on: bool) {
    VERBOSE.store(on, Ordering::Relaxed);
}

pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

pub fn rule() {
    println!("{}", style::dim(&"-".repeat(WIDTH)));
}

/// Program title and one-line description.
pub fn header(version: &str, subtitle: &str) {
    println!("{} {}", style::bold_cyan("ksupatch"), style::dim(version));
    if verbose() {
        println!("{}", style::dim(subtitle));
        rule();
    }
}

/// Start a numbered step.
pub fn step(n: u8, total: u8, title: &str) {
    if !verbose() {
        return;
    }
    println!();
    println!(
        "{} {}",
        style::cyan(&format!("[{n}/{total}]")),
        style::bold(title)
    );
}

/// One `label  value` row inside a step.
pub fn field(label: &str, value: impl std::fmt::Display) {
    if !verbose() {
        return;
    }
    println!("{INDENT}{}  {}", style::dim(&format!("{label:<14}")), value);
}

/// A dim explanatory line inside a step.
pub fn note(text: &str) {
    if !verbose() {
        return;
    }
    println!("{INDENT}{}", style::dim(text));
}

/// A warning line inside a step.
pub fn warn(text: &str) {
    println!(
        "{INDENT}{} {}",
        style::bold_yellow("!"),
        style::yellow(text)
    );
}

/// One row of the final summary.
pub fn item(label: &str, value: impl std::fmt::Display) {
    println!("  {}  {}", style::dim(&format!("{label:<9}")), value);
}

/// The closing summary block, used by `--verbose`.
pub fn done(title: &str, rows: &[(&str, String)]) {
    println!();
    rule();
    println!("{}", style::bold_green(title));
    for (label, value) in rows {
        item(label, value);
    }
    rule();
}

/// The default one-screen summary: aligned labels, an empty label starts a new group.
pub fn summary(rows: &[(&str, String)]) {
    println!();
    for (label, value) in rows {
        if label.is_empty() {
            println!();
        } else {
            println!("  {}  {}", style::dim(&format!("{label:<8}")), value);
        }
    }
}

/// A group separated integer, e.g. `1,462,272`.
pub fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// A human-readable size without the byte count, for embedding in a sentence.
pub fn size(n: usize) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let value = n as f64;
    if value >= MIB {
        format!("{:.2} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{n} bytes")
    }
}

/// A human-readable size that still carries the exact byte count.
pub fn bytes(n: usize) -> String {
    if n >= 1024 {
        format!("{} ({} bytes)", size(n), group(n))
    } else {
        format!("{} bytes", group(n))
    }
}
