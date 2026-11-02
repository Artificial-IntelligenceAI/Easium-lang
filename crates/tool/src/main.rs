//! The `easium` tool (D9, D11). Its name is a placeholder; for now it only runs a file.

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    // Rule 6: a crash inside Easium is Easium's bug, never shown as a bare Rust panic.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("Easium's bug, not yours: Easium crashed ({info}). Please report it.");
    }));
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.as_slice() {
        [command, path] if command == "run" => run(path),
        _ => {
            eprintln!("usage: easium run <file>");
            ExitCode::from(2)
        }
    }
}

fn run(path: &str) -> ExitCode {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("stopped: couldn't read `{path}`: {error}");
            return ExitCode::FAILURE;
        }
    };
    // Every project has a manifest and `run` makes it when missing (D10, D11), but its format is still open.
    if io::stderr().is_terminal() {
        eprintln!(
            "note: the manifest's format isn't decided yet (D10), so `{path}` runs on its own"
        );
    }
    let program = match easium_front::compile(&source) {
        Ok(program) => program,
        Err(diagnostic) => {
            eprint!("{}", diagnostic.render(&source, path));
            return ExitCode::FAILURE;
        }
    };
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    // D52: the bytecode interpreter is tier 0; the reference interpreter is there to check it against.
    let result = if std::env::var_os("EASIUM_INTERPRETER").is_some_and(|name| name == "reference") {
        easium_interp::run(&program, &mut input, &mut output)
    } else {
        let mut tiering = tiering();
        easium_bytecode::run_tiered(&program, &mut input, &mut output, tiering.as_mut())
    };
    let _ = output.flush();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(diagnostic) => {
            eprint!("{}", diagnostic.render(&source, path));
            ExitCode::FAILURE
        }
    }
}

/// D36: hot loops go to Cranelift. `EASIUM_JIT=off` keeps everything in tier 0; `EASIUM_JIT=always` compiles
/// every loop at its first round, which is how the tests run them all through Cranelift (rule 10).
fn tiering() -> Option<easium_bytecode::Tiering> {
    let setting = std::env::var("EASIUM_JIT").unwrap_or_default();
    let threshold = match setting.as_str() {
        "off" => return None,
        "always" => 0,
        _ => 1000,
    };
    let compiler = easium_cranelift::Cranelift::new()?;
    Some(easium_bytecode::Tiering {
        compiler: Box::new(compiler),
        threshold,
    })
}
