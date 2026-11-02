//! The bytecode interpreter, Redstone's tier 0 (D52): compiles a checked program to typed bytecode and runs it.
//! Its output and errors match the reference interpreter's exactly (rule 10).

mod compile;
mod machine;
mod op;

use std::io::{BufRead, Write};

use redstone_front::Diagnostic;
use redstone_front::ir::Program;

/// Runs `program`, reading typed lines for `ask` from `input` and writing to `output`.
///
/// # Errors
///
/// A problem found while running, such as something still open in the design.
pub fn run(
    program: &Program,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<(), Diagnostic> {
    let code = compile::compile(program)?;
    machine::execute(&code, input, output)
}
