//! The bytecode interpreter, Easium's tier 0 (D52): compiles a checked program to typed bytecode and runs it.
//! Its output and errors match the reference interpreter's exactly (rule 10).

mod compile;
mod machine;
pub mod op;

pub use compile::Code;

use std::io::{BufRead, Write};

use easium_front::Diagnostic;
use easium_front::ir::Program;
use easium_runtime::{Rational, Value};

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
    run_tiered(program, input, output, None)
}

/// Runs `program` as [`run`] does, handing loops that get hot to `tiering`'s compiler (D3, D36).
///
/// # Errors
///
/// As [`run`]; compiled loops hand any error back to tier 0, which reports it (rule 10).
pub fn run_tiered(
    program: &Program,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    tiering: Option<&mut Tiering>,
) -> Result<(), Diagnostic> {
    let code = compile::compile(program)?;
    machine::execute(&code, input, output, tiering)
}

/// How hot loops leave tier 0.
pub struct Tiering {
    /// Compiles hot loops.
    pub compiler: Box<dyn LoopCompiler>,
    /// How many times a loop goes round in tier 0 before it is compiled; 0 compiles at the first round.
    pub threshold: u32,
}

/// Compiles loops of bytecode to machine code.
pub trait LoopCompiler {
    /// Compiles the loop whose head is instruction `head` and whose jump back is instruction `back`, or gives
    /// `None` if it can't.
    fn compile(&mut self, code: &Code, head: usize, back: usize) -> Option<Box<dyn CompiledLoop>>;
}

/// A compiled loop.
pub trait CompiledLoop {
    /// Runs the loop from its head, on tier 0's registers, and returns the instruction tier 0 goes on from.
    /// Anything the compiled loop can't finish itself, such as an instruction about to fail, it leaves for tier 0
    /// by returning that instruction, so tier 0 reports the error exactly as it always would.
    fn run(&self, registers: &mut Registers<'_>) -> usize;
}

/// Tier 0's register files, lent to a compiled loop.
pub struct Registers<'a> {
    /// Ints and booleans.
    pub ints: &'a mut [i64],
    /// Rationals.
    pub rationals: &'a mut [Rational],
    /// Text and tables, which compiled loops only read.
    pub objects: &'a [Value],
}
