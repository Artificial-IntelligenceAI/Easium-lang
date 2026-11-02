//! Runs every program in `tests/programs` (rule 9) and checks that each traced decision has one.
//!
//! Each `name.redstone` has `name.out`, its expected output, and, when it should fail, `name.err`, the expected
//! error. `name.in` holds what gets typed for `ask`. With `REDSTONE_BLESS=1` set, the expected files are rewritten
//! from what actually happens, to be reviewed before committing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn programs() -> Vec<PathBuf> {
    let mut programs: Vec<PathBuf> = fs::read_dir(root().join("tests/programs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "redstone")
        })
        .collect();
    programs.sort();
    programs
}

/// Waits for a test program, killing it after `limit` so a program that never ends (a `repeat forever` with no
/// way out) fails its test instead of hanging the suite.
fn wait_at_most(child: Child, limit: Duration) -> Option<Output> {
    let id = child.id();
    let finished = Arc::new(AtomicBool::new(false));
    let timed_out = Arc::new(AtomicBool::new(false));
    let watchdog = {
        let (finished, timed_out) = (Arc::clone(&finished), Arc::clone(&timed_out));
        thread::spawn(move || {
            let start = Instant::now();
            while start.elapsed() < limit {
                if finished.load(Ordering::SeqCst) {
                    return;
                }
                thread::sleep(Duration::from_millis(20));
            }
            timed_out.store(true, Ordering::SeqCst);
            let _ = Command::new("kill").arg("-9").arg(id.to_string()).status();
        })
    };
    let output = child.wait_with_output().unwrap();
    finished.store(true, Ordering::SeqCst);
    watchdog.join().unwrap();
    (!timed_out.load(Ordering::SeqCst)).then_some(output)
}

#[test]
fn every_program_behaves_as_expected() {
    let bless = std::env::var_os("REDSTONE_BLESS").is_some();
    let mut failures = Vec::new();
    let programs = programs();
    assert!(!programs.is_empty(), "no test programs found");
    // Rule 10: each program runs in the bytecode interpreter (tier 0) and in the reference interpreter, and both
    // must match the expected files.
    let interpreters: &[&str] = if bless {
        &["reference"]
    } else {
        &["bytecode", "reference"]
    };
    for (program, interpreter) in programs.iter().flat_map(|program| {
        interpreters
            .iter()
            .map(move |interpreter| (program, *interpreter))
    }) {
        let name = program.file_name().unwrap().to_str().unwrap();
        let input = fs::read(program.with_extension("in")).unwrap_or_default();
        let mut child = Command::new(env!("CARGO_BIN_EXE_redstone"))
            .arg("run")
            .arg(name)
            .env("REDSTONE_INTERPRETER", interpreter)
            .current_dir(program.parent().unwrap())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&input).unwrap();
        let result = wait_at_most(child, Duration::from_secs(10));
        let Some(result) = result else {
            failures.push(format!(
                "{name}: still running after 10 seconds, so it was stopped"
            ));
            continue;
        };
        let stdout = String::from_utf8(result.stdout).unwrap();
        let stderr = String::from_utf8(result.stderr).unwrap();
        let (out_path, err_path) = (program.with_extension("out"), program.with_extension("err"));

        if bless {
            fs::write(&out_path, &stdout).unwrap();
            if result.status.success() {
                let _ = fs::remove_file(&err_path);
            } else {
                fs::write(&err_path, &stderr).unwrap();
            }
            continue;
        }

        let Ok(expected_out) = fs::read_to_string(&out_path) else {
            failures.push(format!("{name} ({interpreter}): no .out file"));
            continue;
        };
        let expected_err = fs::read_to_string(&err_path).ok();
        if result.status.success() != expected_err.is_none() {
            failures.push(format!(
                "{name}: expected it to {}",
                if expected_err.is_none() {
                    "succeed"
                } else {
                    "fail"
                }
            ));
        }
        if stdout != expected_out {
            failures.push(format!(
                "{name}: output\n--- expected\n{expected_out}--- got\n{stdout}"
            ));
        }
        if stderr != expected_err.clone().unwrap_or_default() {
            failures.push(format!(
                "{name}: error\n--- expected\n{}--- got\n{stderr}",
                expected_err.unwrap_or_default()
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// Rule 3 and rule 9: every decision named in a trace tag (`// D7`) has a test program named after it (`d07_…`).
#[test]
fn every_traced_decision_has_a_program() {
    let mut traced = BTreeSet::new();
    let mut sources = vec![root().join("crates")];
    while let Some(path) = sources.pop() {
        if path.is_dir() {
            sources.extend(
                fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            for line in fs::read_to_string(&path).unwrap().lines() {
                let Some((_, comment)) = line.split_once("// D") else {
                    continue;
                };
                for tag in format!("D{comment}").split([',', ':', ' ']) {
                    if let Some(number) = tag
                        .strip_prefix('D')
                        .and_then(|digits| digits.parse::<u32>().ok())
                    {
                        traced.insert(number);
                    }
                }
            }
        }
    }
    let tested: BTreeSet<u32> = programs()
        .iter()
        .filter_map(|path| {
            let name = path.file_stem()?.to_str()?;
            name.strip_prefix('d')?.split('_').next()?.parse().ok()
        })
        .collect();
    let missing: Vec<String> = traced
        .difference(&tested)
        .map(|number| format!("D{number}"))
        .collect();
    assert!(
        missing.is_empty(),
        "traced but no test program: {}",
        missing.join(", ")
    );
}
