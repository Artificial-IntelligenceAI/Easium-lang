//! The `redstone` tool (D9, D11). Its name is a placeholder; for now it only runs a file.

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    // Rule 6: a crash inside Redstone is Redstone's bug, never shown as a bare Rust panic.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("Redstone's bug, not yours: Redstone crashed ({info}). Please report it.");
    }));
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.as_slice() {
        [command, path] if command == "run" => run(path),
        _ => {
            eprintln!("usage: redstone run <file>");
            ExitCode::from(2)
        }
    }
}

fn run(path: &str) -> ExitCode {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("error: couldn't read `{path}`: {error}");
            return ExitCode::FAILURE;
        }
    };
    // Every project has a manifest and `run` makes it when missing (D10, D11), but its format is still open.
    if io::stderr().is_terminal() {
        eprintln!(
            "note: the manifest's format isn't decided yet (D10), so `{path}` runs on its own"
        );
    }
    let program = match redstone_front::compile(&source) {
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
    let result = redstone_interp::run(&program, &mut input, &mut output);
    let _ = output.flush();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(diagnostic) => {
            eprint!("{}", diagnostic.render(&source, path));
            ExitCode::FAILURE
        }
    }
}
