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
