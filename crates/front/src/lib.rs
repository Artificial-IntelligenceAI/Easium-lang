//! Easium's front end: source text to a checked program (D8).

pub mod ast;
pub mod checker;
pub mod diagnostic;
pub mod ir;
pub mod lexer;
pub mod parser;

pub use diagnostic::{Diagnostic, Kind, Span};

/// Lexes, parses and checks `source`.
///
/// # Errors
///
/// The first problem found, as a diagnostic to show.
pub fn compile(source: &str) -> Result<ir::Program, Diagnostic> {
    let tokens = lexer::lex(source)?;
    let program = parser::parse(&tokens)?;
    checker::check(&program)
}
