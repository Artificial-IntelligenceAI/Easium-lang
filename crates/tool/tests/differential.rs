//! Rule 10, tested broadly: random Redstone programs run in the reference interpreter, in tier 0, and in tier 0
//! with every loop compiled by Cranelift, and all three must show the same output and the same error.
//!
//! The programs stick to what is decided, with every operation in parentheses so no open precedence question
//! stops them early. Errors are expected (overflow, a third that can't be shown, a missing position) and must
//! match too. Set `REDSTONE_FUZZ_COUNT` to run more programs, `REDSTONE_FUZZ_SEED` to start elsewhere.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fmt::Write as _;

/// A small, fixed pseudo-random sequence, so a failure can be run again from its seed.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, limit: u64) -> u64 {
        self.next() % limit
    }

    fn chance(&mut self, in_ten: u64) -> bool {
        self.below(10) < in_ten
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[usize::try_from(self.below(items.len() as u64)).unwrap()]
    }
}

/// What is in scope where an expression is being written.
#[derive(Clone, Copy, Default)]
struct Scope {
    in_repeat: bool,
    in_for: bool,
    in_loop: bool,
}

struct Generator {
    random: Random,
    table_length: u64,
    /// The table's first items, for stops that are reached.
    items: Vec<u64>,
    lines: Vec<String>,
}

impl Generator {
    fn line(&mut self, depth: usize, text: &str) {
        self.lines.push(format!("{}{text}", "    ".repeat(depth)));
    }

    fn int(&mut self, scope: Scope, depth: u32) -> String {
        if depth == 0 || self.random.chance(3) {
            let mut leaves = vec!["a", "b", "c", "small", "small", "position"];
            if scope.in_repeat {
                leaves.push("current_round");
            }
            if scope.in_for {
                leaves.push("current_item");
            }
            return match self.random.pick(&leaves) {
                "small" => self.random.below(12).to_string(),
                // Now and then one past the end, to test the error.
                "position" => {
                    let past_end = u64::from(self.random.below(15) == 0);
                    format!(
                        "value at position {} in t",
                        1 + self.random.below(self.table_length) + past_end
                    )
                }
                name => name.to_owned(),
            };
        }
        let left = self.int(scope, depth - 1);
        match self.random.below(8) {
            0..=2 => format!("({left} plus {})", self.int(scope, depth - 1)),
            3 => format!("({left} minus {})", self.int(scope, depth - 1)),
            4 | 5 => format!("({left} times {})", self.int(scope, depth - 1)),
            _ => format!("({left} remainder {})", 1 + self.random.below(7)),
        }
    }

    fn rational(&mut self, scope: Scope, depth: u32) -> String {
        if depth == 0 || self.random.chance(3) {
            return match self.random.below(4) {
                0 => "r".to_owned(),
                1 => "s".to_owned(),
                2 => self
                    .random
                    .pick(&["0.5", "0.25", "1.5", "0.1", "2.0", "0.125"])
                    .to_owned(),
                _ => format!(
                    "({} divided_by {})",
                    self.int(scope, 1),
                    1 + self.random.below(8)
                ),
            };
        }
        let left = self.rational(scope, depth - 1);
        let right = if self.random.chance(3) {
            self.int(scope, 1)
        } else {
            self.rational(scope, depth - 1)
        };
        let word = self.random.pick(&["plus", "minus", "times", "plus"]);
        format!("({left} {word} {right})")
    }

    fn boolean(&mut self, scope: Scope, depth: u32) -> String {
        if depth == 0 || self.random.chance(3) {
            return match self.random.below(4) {
                0 => "p".to_owned(),
                1 => "q".to_owned(),
                2 => {
                    let word = self.comparison();
                    format!("({} {word} {})", self.int(scope, 1), self.int(scope, 1))
                }
                _ => {
                    let word = self.comparison();
                    format!(
                        "({} {word} {})",
                        self.rational(scope, 1),
                        self.rational(scope, 1)
                    )
                }
            };
        }
        match self.random.below(3) {
            0 => format!(
                "({} and {})",
                self.boolean(scope, depth - 1),
                self.boolean(scope, depth - 1)
            ),
            1 => format!(
                "({} or {})",
                self.boolean(scope, depth - 1),
                self.boolean(scope, depth - 1)
            ),
            _ => format!("(not {})", self.boolean(scope, depth - 1)),
        }
    }

    fn comparison(&mut self) -> &'static str {
        self.random.pick(&[
            "is",
            "is_not",
            "less_than",
            "greater_than",
            "at_least",
            "at_most",
        ])
    }

    fn trailing_if(&mut self, scope: Scope) -> String {
        if self.random.chance(2) {
            format!(" if {}", self.boolean(scope, 1))
        } else {
            String::new()
        }
    }

    fn statements(&mut self, scope: Scope, depth: usize, count: u64) {
        for _ in 0..count {
            self.statement(scope, depth);
        }
    }

    fn loop_statement(&mut self, scope: Scope, depth: usize) {
        match self.random.below(4) {
            0 | 1 => {
                let count = self.random.below(6);
                self.line(depth, &format!("repeat {count} times:"));
                let inner = Scope {
                    in_repeat: true,
                    in_loop: true,
                    ..scope
                };
                let statements = 1 + self.random.below(3);
                self.statements(inner, depth + 1, statements);
                self.line(depth, "end_of repeat");
            }
            2 => {
                // Value and test stops are mostly ones the original items reach; changes to the table and the
                // odd miss still test the "never reached" error.
                let item =
                    self.items[usize::try_from(self.random.below(self.table_length)).unwrap()];
                let stop = match self.random.below(5) {
                    0 | 1 => "until last_item".to_owned(),
                    2 => format!(
                        "until_before position {}",
                        1 + self.random.below(self.table_length)
                    ),
                    3 => format!("until current_item at_least {item}"),
                    _ => format!("until_before value {item}"),
                };
                self.line(depth, &format!("for items in t {stop}:"));
                let inner = Scope {
                    in_for: true,
                    in_loop: true,
                    ..scope
                };
                let statements = 1 + self.random.below(3);
                self.statements(inner, depth + 1, statements);
                self.line(depth, "end_of for");
            }
            _ => {
                let limit = self.random.below(6);
                self.line(
                    depth,
                    &format!("repeat until current_round greater_than {limit}:"),
                );
                let inner = Scope {
                    in_repeat: true,
                    in_loop: true,
                    ..scope
                };
                let statements = 1 + self.random.below(3);
                self.statements(inner, depth + 1, statements);
                self.line(depth, "end_of repeat");
            }
        }
    }

    fn statement(&mut self, scope: Scope, depth: usize) {
        let nested = depth < 3;
        match self.random.below(if nested { 14 } else { 9 }) {
            0 | 1 => {
                let target = self.random.pick(&["a", "b", "c"]);
                let value = self.int(scope, 2);
                let condition = self.trailing_if(scope);
                self.line(depth, &format!("{target} becomes {value}{condition}"));
            }
            2 => {
                let target = self.random.pick(&["r", "s"]);
                let value = self.rational(scope, 2);
                let condition = self.trailing_if(scope);
                self.line(depth, &format!("{target} becomes {value}{condition}"));
            }
            3 => {
                let target = self.random.pick(&["p", "q"]);
                let value = self.boolean(scope, 2);
                self.line(depth, &format!("{target} becomes {value}"));
            }
            4 => {
                let shown = match self.random.below(4) {
                    0 => self.int(scope, 2),
                    1 => self.rational(scope, 1),
                    2 => format!("whether {}", self.boolean(scope, 2)),
                    _ => format!("{} {}x", self.int(scope, 1), self.random.below(3)),
                };
                self.line(depth, &format!("show {shown}"));
            }
            5 => {
                let (int, rational) = (self.int(scope, 1), self.rational(scope, 1));
                self.line(
                    depth,
                    &format!("show_using_format \"i=<{int}> r=<{rational}> p=<p>\""),
                );
            }
            6 => {
                let position = 1 + self.random.below(self.table_length);
                let value = self.int(scope, 1);
                self.line(
                    depth,
                    &format!("value at position {position} in t becomes {value}"),
                );
            }
            7 if scope.in_loop => {
                let condition = self.boolean(scope, 1);
                self.line(depth, &format!("stop if {condition}"));
            }
            7 | 8 => {
                let joined = format!(
                    "{} <-- \" \" <-- {}",
                    self.int(scope, 1),
                    self.random.pick(&["p", "a", "r"])
                );
                self.line(depth, &format!("line becomes {joined}"));
                self.line(depth, "show line");
            }
            9..=12 => self.loop_statement(scope, depth),
            _ => {
                let condition = self.boolean(scope, 2);
                self.line(depth, &format!("if {condition}:"));
                let statements = 1 + self.random.below(2);
                self.statements(scope, depth + 1, statements);
                if self.random.chance(4) {
                    let other = self.boolean(scope, 1);
                    self.line(depth, &format!("else_if {other}:"));
                    self.statements(scope, depth + 1, 1);
                }
                if self.random.chance(5) {
                    self.line(depth, "else:");
                    self.statements(scope, depth + 1, 1);
                }
                self.line(depth, "end_of if");
            }
        }
    }
}

fn program(seed: u64) -> String {
    let mut generator = Generator {
        random: Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1),
        table_length: 0,
        items: Vec::new(),
        lines: Vec::new(),
    };
    generator.table_length = 2 + generator.random.below(4);
    generator.items = (0..generator.table_length)
        .map(|_| generator.random.below(10))
        .collect();
    let items = generator
        .items
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let a = generator.random.below(20);
    let b = generator.random.below(20);
    let c = generator.random.below(20);
    for line in [
        format!("a = {a}"),
        format!("b = {b}"),
        format!("c = {c}"),
        "r = 0.5".to_owned(),
        "s = 1.25".to_owned(),
        "p = true".to_owned(),
        "q = false".to_owned(),
        format!("t = {items}"),
        "line = \"\"".to_owned(),
    ] {
        generator.line(0, &line);
    }
    let statements = 4 + generator.random.below(8);
    generator.statements(Scope::default(), 0, statements);
    let mut source = generator.lines.join("\n");
    source.push('\n');
    source
}

/// Runs a program one way: its output, then its error as it would be shown, if any.
fn run(source: &str, way: &str) -> String {
    let mut output = Vec::new();
    let mut input = std::io::Cursor::new(Vec::new());
    let result = redstone_front::compile(source).and_then(|program| match way {
        "reference" => redstone_interp::run(&program, &mut input, &mut output),
        "tier 0" => redstone_bytecode::run(&program, &mut input, &mut output),
        _ => {
            let mut tiering =
                redstone_cranelift::Cranelift::new().map(|compiler| redstone_bytecode::Tiering {
                    compiler: Box::new(compiler),
                    threshold: 0,
                });
            redstone_bytecode::run_tiered(&program, &mut input, &mut output, tiering.as_mut())
        }
    });
    let mut shown = String::from_utf8(output).unwrap();
    if let Err(diagnostic) = result {
        let _ = write!(
            shown,
            "--- error ---\n{}",
            diagnostic.render(source, "fuzz.redstone")
        );
    }
    shown
}

#[test]
fn every_way_of_running_agrees() {
    let count: u64 = std::env::var("REDSTONE_FUZZ_COUNT")
        .ok()
        .and_then(|count| count.parse().ok())
        .unwrap_or(400);
    let start: u64 = std::env::var("REDSTONE_FUZZ_SEED")
        .ok()
        .and_then(|seed| seed.parse().ok())
        .unwrap_or(1);
    let mut checked_errors = 0;
    for seed in start..start + count {
        let source = program(seed);
        let reference = run(&source, "reference");
        assert!(
            redstone_front::compile(&source).is_ok(),
            "seed {seed}: the generator wrote a program that doesn't check:\n{source}\n{reference}"
        );
        if reference.contains("--- error ---") {
            checked_errors += 1;
        }
        if std::env::var_os("REDSTONE_FUZZ_STATS").is_some() && seed < start + 3 {
            eprintln!("=== seed {seed}\n{source}--- runs to ---\n{reference}");
        }
        for way in ["tier 0", "cranelift"] {
            let other = run(&source, way);
            assert!(
                other == reference,
                "seed {seed}: {way} differs from the reference\n--- program ---\n{source}--- reference ---\n{reference}--- {way} ---\n{other}"
            );
        }
    }
    if std::env::var_os("REDSTONE_FUZZ_STATS").is_some() {
        eprintln!("{checked_errors} of {count} programs ended in an error");
    }
    // The generator should reach errors as well as clean runs, or the errors go untested.
    assert!(checked_errors > 0, "no program ended in an error");
    assert!(checked_errors < count, "every program ended in an error");
}
