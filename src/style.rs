//! Terminal colours.
//!
//! Colour is decided once, on stdout: on when stdout is a terminal, off when it is a file or
//! a pipe, so a redirected log never collects escape sequences. `NO_COLOR` (the usual
//! convention: set to anything to disable) and an explicit `--color` override both win over
//! that guess.
//!
//! No dependency is used. On Windows the console only interprets escape sequences once
//! virtual-terminal processing is switched on, which is one small kernel32 call.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Auto,
    Always,
    Never,
}

static ON: AtomicBool = AtomicBool::new(false);

pub fn init(choice: Choice) -> Result<(), String> {
    let on = match choice {
        Choice::Always => true,
        Choice::Never => false,
        Choice::Auto => {
            if std::env::var_os("NO_COLOR").is_some() {
                false
            } else if !std::io::stdout().is_terminal() {
                false
            } else {
                enable_virtual_terminal()
            }
        }
    };
    ON.store(on, Ordering::Relaxed);
    Ok(())
}

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

fn wrap(code: &str, s: &str) -> String {
    if on() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn bold(s: &str) -> String {
    wrap("1", s)
}
pub fn dim(s: &str) -> String {
    wrap("2", s)
}
pub fn red(s: &str) -> String {
    wrap("31", s)
}
pub fn dim_red(s: &str) -> String {
    wrap("2;31", s)
}
pub fn green(s: &str) -> String {
    wrap("32", s)
}
pub fn yellow(s: &str) -> String {
    wrap("33", s)
}
pub fn heading(s: &str) -> String {
    wrap("1;36", s)
}
pub fn bold_green(s: &str) -> String {
    wrap("1;32", s)
}
pub fn bold_red(s: &str) -> String {
    wrap("1;31", s)
}

/// Turn on ANSI escape processing for the console attached to stdout.
#[cfg(windows)]
fn enable_virtual_terminal() -> bool {
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    const INVALID_HANDLE_VALUE: isize = -1;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(which: u32) -> *mut core::ffi::c_void;
        fn GetConsoleMode(handle: *mut core::ffi::c_void, mode: *mut u32) -> i32;
        fn SetConsoleMode(handle: *mut core::ffi::c_void, mode: u32) -> i32;
    }

    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle.is_null() || handle as isize == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut mode = 0u32;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return false;
        }
        SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

#[cfg(not(windows))]
fn enable_virtual_terminal() -> bool {
    true
}
