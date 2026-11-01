//! Base layer of the BRAINLLM compiler: source positions, diagnostics and the BrainScript lexer.
//!
//! Everything here is hand-written: no third-party parsing crates (SPEC: zero parsing deps).

pub mod codes;
mod diag;
pub mod json;
mod lexer;
mod span;

pub use json::Json;
pub use diag::{push_json_str, Diag, Edit, Severity};
pub use lexer::{Lexer, LexerState, Tok, Token};
pub use span::{LineIndex, Span};

/// Writes `line` and its newline to stderr in one call. Stderr has no buffer, so `eprintln!` makes one write per piece of
/// its format string, and a log collector that stamps each write splits the line (`error:` alone, then the message).
pub fn write_stderr_line(line: &str) {
    use std::io::Write;
    let mut text = String::with_capacity(line.len() + 1);
    text.push_str(line);
    text.push('\n');
    let _ = std::io::stderr().lock().write_all(text.as_bytes());
}
