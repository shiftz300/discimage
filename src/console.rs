//! Line-oriented diagnostics. Command results and JSON stay on stdout.
use std::{
    fmt::Arguments,
    io::{self, IsTerminal, Write},
};

pub fn phase(label: &str, message: Arguments<'_>) {
    line(label, 36, message);
}

pub fn success(message: Arguments<'_>) {
    line("done", 32, message);
}

pub fn warning(message: Arguments<'_>) {
    line("warning", 33, message);
}

pub fn error(message: Arguments<'_>) {
    line("error", 31, message);
}

fn line(label: &str, color: u8, message: Arguments<'_>) {
    let mut stderr = io::stderr().lock();
    // Basic ANSI colors are supported by Terminal.app. Keep redirected logs
    // and consoles without advertised ANSI support readable as plain text.
    let use_color = cfg!(unix)
        && stderr.is_terminal()
        && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
        && std::env::var_os("TERM").is_some_and(|value| !value.is_empty() && value != "dumb");
    if use_color {
        let _ = writeln!(stderr, "\x1b[{color}m[{label}]\x1b[0m {message}");
    } else {
        let _ = writeln!(stderr, "[{label}] {message}");
    }
}
