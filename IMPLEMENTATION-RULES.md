# Easium — implementation rules

The rules for the code that implements Easium (D27). Proposed by easium design claude; owner: "Yup. Pedantic.
Auto commit & push".

## Process

1. **Commit and push as the work is done**, without asking (owner: "Auto commit & push"). Commit messages follow
   Soclets/CyborgPL: `kind.area.topic: sentence`, kinds such as `decision`, `design`, `feature`, `fix`, `test`,
   `house`; the owner's words in `(owner: …)` where a decision is theirs. Commits end with
   `Co-Authored-By: easium design claude <noreply@anthropic.com>`.
2. **Open means not implemented.** Anything still open in `DECISIONS.md` gives a "not decided yet" error, never a
   quiet default.
3. **Trace tags.** Code and tests name the decision they carry out (`// D7`), so nothing drifts from the log.

## Code

4. **`rustfmt` with default settings; `clippy` at its pedantic level, warnings as errors.**
5. **`unsafe` only in named modules**, each with a written reason.
6. **No `unwrap()` or `expect()` outside tests.** A failure inside Easium is reported as Easium's bug, not the
   user's, never as a Rust panic dump.
7. **One vocabulary.** A concept has the same name in `DECISIONS.md`, the code and the error messages: "rational",
   never "fraction" in one place and `Rat` in another.

## Errors

8. **Every user-facing error follows D31** (Radium's style: only what's certain, the place twice, `what:`, `why:`, `fix:`), and has a test.

## Testing

9. **Tests are small Easium programs**, one file each, with their expected output and any typed input for `ask`.
   Every implemented decision has at least one.
10. **The reference interpreter is the reference (D52).** The bytecode interpreter (tier 0) and the Cranelift tier
    (D36) give the same output and the same errors as the reference interpreter on every test program, with
    Easium's own optimizations on and off. The reference stays simple: no optimizations in it.

## Layout

11. **A Cargo workspace, one crate per part:** front end, runtime, the reference interpreter, the bytecode
    interpreter, Easium's optimizer, the Cranelift tier, and the tool.
