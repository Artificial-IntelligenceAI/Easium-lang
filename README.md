# Easium

A friendly language, like Ruby: for people who want to make things without getting into a whole new world, with
some programmer things for programmers who want to use it (D1). Small projects to show off, that can still grow
(D2).

```
name = ask "What's your name? "
show_using_format "Hi <name>!"

scores = 90, 65, 70
total = 0
for items in scores until last_item:
    total becomes total plus current_item
end_of for
show "Good average" if total divided_by 3 at_least 70
```

Every design choice is the owner's and is recorded, numbered, in [`DECISIONS.md`](DECISIONS.md). The rules for the
code that implements them are in [`IMPLEMENTATION-RULES.md`](IMPLEMENTATION-RULES.md).

## Running

Needs Rust 1.96 or newer.

```
cargo run --release -p easium-tool -- run program.easium
```

The tool's name (`easium`) and the `.easium` extension are placeholders; the manifest (D10) isn't built yet,
so a file runs on its own.

## How a program runs

1. **Front end** (`crates/front`): lexer, parser, and the checker, which works out every type before the program
   runs (D4) and says what's wrong in Radium's message style (D31). Anything still open in `DECISIONS.md` stops with
   "not decided yet" instead of a quiet default (rule 2).
2. **Tier 0** (`crates/bytecode`): the checked program becomes typed bytecode, with separate register files for
   ints and booleans, rationals, and text and tables, and runs (D52).
3. **Cranelift** (`crates/cranelift`): a loop that goes round 1000 times in tier 0 is compiled to machine code and
   runs on tier 0's own registers (D3, D36). Anything a compiled loop can't finish, such as an instruction about
   to fail, goes back to tier 0 at that instruction, so the error is always tier 0's.
4. **Reference interpreter** (`crates/interp`): a simple tree-walking interpreter, not used to run programs; the
   other two are tested against it (rule 10).

`crates/runtime` holds the values: ints, exact rationals (D7), text, booleans and tables (D32). `crates/tool` is
the command and the tests.

Settings for checking by hand:

| Variable | Effect |
|---|---|
| `EASIUM_INTERPRETER=reference` | run in the reference interpreter |
| `EASIUM_JIT=off` | keep every loop in tier 0 |
| `EASIUM_JIT=always` | compile every loop at its first round |

## Testing

```
cargo test
```

- **`tests/programs`**: one small program per file, named after the decision it covers (`d34_read.easium`),
  with its expected output (`.out`), error (`.err`) and typed input (`.in`). Each runs three ways (tier 0, tier 0
  with every loop compiled, and the reference), and all three must match. `EASIUM_BLESS=1 cargo test` rewrites
  the expected files from the reference, to review before committing.
- **The differential fuzzer** (`crates/tool/tests/differential.rs`): random programs from the decided syntax, run
  the same three ways. `EASIUM_FUZZ_COUNT` and `EASIUM_FUZZ_SEED` run more.
- A test fails the build when code names a decision (`// D7`) that has no test program (rule 3).

`cargo clippy --all-targets -- -D warnings` runs at the pedantic level (rule 4).
