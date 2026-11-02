# Redstone — design decisions

The owner decides; redstone design claude proposes and records. Each entry says what was chosen and whose idea it was.
Numbers are permanent: a decision is never renumbered. A changed decision gets a new entry that says which one it
replaces.

Identity: a friendly language, like Ruby.

Syntax in code samples below is placeholder unless a decision says otherwise.

---

## D1 — Purpose (2026-11-02)

Owner: "It's just for people who want to make things without getting into a whole new world, yk? But it would have
some programmer things, if real programmers wanna use it."

Redstone is for people who want to make things without first learning a whole new world. It also has some
programmer features, for experienced programmers who want to use it.

## D2 — What people make (2026-11-02)

Owner: "Simple stuff. Like, a project to show off. If this gets typed, this happens. But it could do big stuff too."

Redstone is aimed at small projects to show off, like "if this gets typed, this happens". It can still be used for
big projects.

## D3 — A JIT (2026-11-02)

Owner: "I did think it would have a JIT too, maybe based on Cranelift or LLVM, or both."

Redstone has a JIT.

Backend, owner: "Both, in tiers." (yes to redstone design claude's proposal). Code starts in an interpreter; code
that runs often is compiled with Cranelift; the hottest code is compiled with LLVM.

## D4 — Types: lazy inference, exact types for programmers (2026-11-02)

Owner: "most Redstone writers would probably not even know how to complain about lazy type inference. So, we could
just do lazy, and let them deal with it, and give programmers their `i32` shit".

Types are known before the program runs. A variable takes the type of the value it is given (lazy inference); there
is no whole-program inference and no proving. Programmers can write exact types, such as `i32`.

Open: what a function's parameters get when no type is written (functions are not being designed yet, owner: "We
don't go fns yet"); which type an unwritten number such as `18` gets; the names of the exact types.

## D5 — One thing per word (2026-11-02)

Owner's sketch: `show "Is Adult" if age at_least 18`, and on `at_least`: "everything else adds another thing, `if`,
the quotes, and stuff. `at_least` is just one thing, not two". Confirmed as a rule (owner: "Correct").

A space always separates two things; one thing never contains a space, except text inside quotes. A keyword or name
made of several words is joined with `_`: `at_least`, not `at least`.

## D6 — Decimals are exact; binary floats on request (2026-11-02)

Owner: "there could be a friendly way for them to get binary floats", then "Hm, yes" to redstone design claude's
reading of it.

An unwritten decimal such as `0.1` is exact: `0.1 + 0.2` is `0.3`. There is a friendly way to ask for a binary float
instead; programmers can also write the exact type (D4).

Open: the friendly spelling (proposed: `~0.1`, a type word such as `fast_number`, or `0.1 as_float`); whether a whole
number such as `18` is the same type as a decimal.

## D7 — Numbers: int, rational, float (2026-11-02)

Owner: "Yeah, approved." (yes to redstone design claude's proposal, after measuring rationals against floats on an
Apple M5: 40–85× slower when reduced after every operation, about as fast or ~6× when reduction waits until the
numbers grow).

- A whole number such as `18` is an int.
- A decimal such as `0.1` is a rational, an exact fraction (1/10), as Raku's `Rat`. `0.1 + 0.2` is `0.3`;
  `1 / 3 * 3` is `1`. This settles D6's "exact".
- `7 / 2` is the rational `3.5`, never `3`.
- A binary float comes from D6's friendly spelling (still open); programmers can write exact types such as `i32`
  and `f64` (D4).

Open: what happens when a rational's parts outgrow 64 bits (a bigint, or decay to a float as Raku's `Rat` does);
what happens when an int overflows.

## D8 — Host language: Rust (2026-11-02)

Owner: "Yup" (yes to redstone design claude's proposal). Cranelift's only first-class API is Rust, and moving a
running function between tiers (D3) is easier when every tier shares one language and one value layout.

- Front end (parser, lazy type checker, errors), interpreter, Cranelift tier, LLVM tier, and the runtime
  (rationals, bigints, text, memory) are written in Rust.
- LLVM is reached as in CyborgPL: `inkwell` first, `llvm-sys` only where inkwell lacks a call, a small C++ wrapper
  only where the C API lacks a feature.

Open: whether the standard library is written in Redstone or in Rust; which bigint library (`malachite`,
`num-bigint`, or `rug`/GMP, which is LGPL).

## D9 — Built-ins and packages, Cargo style (2026-11-02)

Owner, asked whether Redstone needs a standard library: "Do we really need a stdlib?", then "Maybe Cargo style?",
then "Yes" to redstone design claude's reading of it.

There is no standard library to import from. Redstone has a small set of built-ins that are always there (such as
`show`), living in the Rust runtime (D8). Everything else comes as packages from a registry, through one tool, as
Rust does with Cargo and crates.io. This replaces D8's open question about the standard library's language.

Open (proposed by redstone design claude, not ruled): a single file runs with no manifest; the manifest appears with
the first package added, or packages are declared at the top of the file; the one tool runs, adds, builds and
publishes. Also open: the list of built-ins; the tool's name; the file extension (not `.rs`, which is Rust's); the
registry.

## D10 — A manifest, always (2026-11-02)

Owner: "Manifest always, but we'll make it friendly". Replaces D9's proposal that a single file runs with no
manifest.

Every Redstone project has a manifest, even one with a single file and no packages. It is made friendly.

Open: how it is made friendly; who creates it; its name and format.

## D11 — Who creates the manifest: both (2026-11-02)

Owner: "It's both, bro" (to redstone design claude's two options).

`redstone new <name>` creates a project with its manifest, as `cargo new` does; `redstone run` also creates the
manifest when it is missing. The tool's name (`redstone`) is a placeholder (D9).

## D12 — A variable is created by assigning it (2026-11-02)

Owner, asked how a variable is created, answered with:

```
age = 18
show "Is adult" if age equal 18
```

A variable is created by assigning it, with no keyword: `age = 18`. With D4, a later `age = "hi"` is caught before
the program runs.

Open (proposed by redstone design claude, not ruled): the checker catches a misspelt assignment ("`agee` is set but
never used"); a programmer type is written `age: i32 = 18`. Also from the sketch, not yet confirmed: `equal` is the
equality comparison.

## D13 — `equal` compares; `=` never does (2026-11-02)

Owner: "Yup" (to redstone design claude's reading of D12's sketch).

`equal` is the equality comparison: `age equal 18`. `=` never compares, so there is no `==`.

## D14 — `is_now` changes a variable; `plus` adds (2026-11-02)

Owner's sketch, "for changing the variable":

```
age is_now age plus 1
```

`=` creates a variable (D12); `is_now` changes one that exists. `plus` is addition.

Open (proposed by redstone design claude, not ruled): `=` on a variable that already exists is an error ("`age`
already exists; use `is_now` to change it"), and `is_now` on one that does not exist is an error, which catches a
misspelt name and replaces D12's "set but never used" proposal. Also open: the other arithmetic words; whether
symbols such as `+` exist (as with `>=`, owner: "we'll see").

## D15 — `=` and `is_now` catch misspelt names (2026-11-02)

Owner: "Yes" (to D14's open proposal by redstone design claude).

`=` on a variable that already exists is an error ("`age` already exists; use `is_now` to change it"). `is_now` on a
variable that does not exist is an error ("there's no `agee`. Did you mean `age`?"). Error wording is placeholder.

## D16 — Arithmetic words (2026-11-02)

Owner: "Yes, and to the arith too. It's `remainder`" (redstone design claude proposed `-` `minus`, `*` `times`,
`/` `divided_by`, and `mod` or `remainder` for `%`).

| Operation | Word |
|---|---|
| add | `plus` (D14) |
| subtract | `minus` |
| multiply | `times` |
| divide | `divided_by` (with D7, `7 divided_by 2` is the rational `3.5`) |
| remainder | `remainder` |

Open: precedence; whether symbols such as `+` also exist.

## D17 — Maths precedence, `()` to group (2026-11-02)

Owner: "Follow maths precedence, and () grouping when needed".

Arithmetic words follow school maths: `times` and `divided_by` bind tighter than `plus` and `minus`, so
`2 plus 3 times 4` is `14`. Parentheses group: `(2 plus 3) times 4` is `20`.

Open (proposed by redstone design claude, not ruled): `remainder` binds like `times` and `divided_by`, as in most
languages; comparisons (`equal`, `at_least`) bind looser than all arithmetic; parentheses, like quotes (D5), need
no spaces around them.

## D18 — Comparison words (2026-11-02)

Owner: "Correct. `more_than` is `greater_than`" (to redstone design claude's proposed table).

| Comparison | Word |
|---|---|
| equal | `equal` (D13) |
| greater or equal | `at_least` |
| less or equal | `at_most` |
| greater | `greater_than` |
| less | `less_than` |
| not equal | `not_equal` |

## D19 — `true` and `false` (2026-11-02)

Owner: "`true`/`false`" (redstone design claude offered `true`/`false` or `yes`/`no`).

The booleans are `true` and `false`.

Open: the logic words (proposed: `and`, `or`, `not`).

## D20 — `is` and `is_not` (2026-11-02)

Owner asked: "should we change `equal` to `is` and `isn't` for `not_equal`?"; redstone design claude proposed `is`
and `is_not` (D5's `_` rule; no apostrophe for autocorrect to turn into `’`); owner: "Yes".

`is` is the equality comparison and `is_not` the inequality, replacing D13's `equal` and D18's `not_equal`:
`show "Is adult" if age is 18`.

Open (proposed by redstone design claude, not ruled): a comparison standing alone as a statement is an error
(`age is 5` on its own line: "did you mean `age is_now 5`?").

## D21 — `and`, `or`, `not` (2026-11-02)

Owner: "Yes" (to redstone design claude's proposal). The logic words are `and`, `or` and `not`:
`show "Teen" if age at_least 13 and age less_than 20`. This settles D19's open question.

## D22 — Block `if`: colons, `end of if` (2026-11-02)

Owner, asked how blocks end (redstone design claude proposed Ruby's `end` or Python's indentation):

```
if age at_least 18:
show "Adult"
show "Can vote"
else:
show "Not yet"
end of if
```

A block `if` has a colon after its condition and after `else`, and closes with `end of if`.

Open: whether indentation is ignored (the sketch has none); how `else if` is spelt; whether other blocks close the
same way (`end of while` and so on).

## D23 — Indentation is ignored; a formatter indents (2026-11-02)

Owner: "What's the problem with not spamming the space key? I mean, we could just make a formatter".

Spaces at the start of a line mean nothing; blocks are marked only by their colons and `end of …` (D22). Redstone
has a formatter that indents code.

Open: whether the formatter is part of the one tool (D9).

## D24 — `else_if` (2026-11-02)

Owner: "Yes" (to redstone design claude's recommendation; first proposed `else if`, then `else_if` on reflection).

```
if age at_least 18:
show "Adult"
else_if age at_least 13:
show "Teen"
else:
show "Kid"
end of if
```

`else_if` is one keyword (D5's `_` rule): it continues the same `if`, so the block still closes with one `end of if`.
`else if` as two words would read as `else` followed by a new `if` needing its own `end of if` (D22).

## D25 — `end_of`, not `end of` (2026-11-02)

Owner: "Wait, `end of` should be `end_of`. I mean, you can't swap that `of` for something else".

A block closes with `end_of` followed by the block's name: `end_of if`. `end` and `of` never appear apart, so by D5
they are one keyword; the block's name after it is a second thing. Replaces `end of if` in D22, D23 and D24.

```
if age at_least 18:
show "Adult"
else_if age at_least 13:
show "Teen"
else:
show "Kid"
end_of if
```

## D26 — `ask` reads input (2026-11-02)

Owner: "`ask`" (redstone design claude proposed `ask`, `input` or `read`).

`ask` shows a prompt and waits for a line to be typed; what comes back is text:

```
name = ask "What's your name?"
show "Hi " plus name
```

Open: getting a number from `ask` (proposed: `ask_number`, or a conversion); whether `plus` joins text, as in the
example.

## D27 — Implementation rules (2026-11-02)

Owner: "Yup. Pedantic. Auto commit & push" (to redstone design claude's eleven proposed rules; clippy's pedantic
level chosen over its default; commits and pushes happen without asking, instead of only when the owner says so).

The rules are in `IMPLEMENTATION-RULES.md`.

## D28 — `show whether` (2026-11-02)

Owner, on `show 1 is 1.0`: "Reads a bit weird", "reads a bit weird is the problem that needs to get fixed", then
"yes" to redstone design claude's reading (it sounds like a sentence claiming "1 is 1.0") and to `whether`, the
recommended fix (over requiring parentheses, or not showing comparisons at all).

Showing the result of a comparison takes `whether`: `show whether age at_least 18` shows `true` or `false`. Without
it, `show age at_least 18` is an error that suggests `whether`.

Open (redstone design claude's reading, not ruled): the rule covers anything built with a comparison or `and`, `or`,
`not`; a plain boolean (`show true`, `show is_adult`) needs no `whether`; `whether` anywhere but right after `show`.

## D29 — An int beside a rational counts as a rational (2026-11-02)

Owner: "yes" (to redstone design claude's question whether comparing a rational with an int should just work, as the
arithmetic already did, read from D7's example `1 / 3 * 3` being `1`).

Where an int meets a rational, in arithmetic or a comparison, the int counts as a rational: `1 plus 0.5` is `1.5`;
`half greater_than 3` compares 3.5 with 3; `1 is 1.0` is `true`. `remainder` on rationals stays open (D16).

## D30 — Comparisons bind looser than arithmetic (2026-11-02)

Owner: "yes" (to redstone design claude's question; one of D17's open proposals). As in maths, every comparison
binds looser than all arithmetic, `remainder` included: `age plus 1 at_least 18` means `(age plus 1) at_least 18`,
and `n remainder 2 is 0` means `(n remainder 2) is 0`.

## D31 — Error messages in Radium's style, friendlier (2026-11-02)

Owner: "Btw, should we copy Radium's error message style?", then "yes" to redstone design claude's proposal: copy
Radium's D116–D118 (`/Users/ts/Radium Design/design-rules/error-messages.md`) with two changes for Redstone's
audience.

- **Only what's certain.** A mistake with exactly one fix shows it, as code (`` `is not` → `is_not` ``), on a `fix:`
  line in place of `what:`. With more than one fix, only facts: `what:` is wrong and `why:`. No "did you mean".
- **The place twice** on the first line: `file:line:column` for tools (the column counts UTF-8 bytes), then
  `line N column M` for people (the column counts grapheme clusters).
- **Then the code line, a `^` pointer** under exactly the part in question (grapheme clusters), and lined-up labels.
- **Changed from Radium:** `why:` is in plain words, with no decision numbers, except on "not decided yet" messages,
  which only Redstone's own makers should meet. The first word is friendlier: `stopped:` for a mistake,
  `not decided yet:` for something still open.

```
stopped: hello.redstone:2:1, line 2 column 1
 2 | agee is_now 19
   | ^^^^
 what: there's no `agee`
 why:  a variable has to be made with `=` before `is_now` can change it
```

Replaces implementation rule 8's "a suggestion where one exists".

## D32 — Tables, Lua's improved (2026-11-02)

Owner, asked whether lists and maps should be separate: "I mean, why don't we improve it?" (Lua's tables), then "yes"
to redstone design claude's proposal.

Redstone has tables: one name and one way of writing them, as in Lua, with Lua's traps fixed.

1. **Typed, lazily (D4).** A table takes its key and value types from what it is made with, and keeps them.
2. **Two kinds, inferred from how a table is made:** *in order* (values only, at positions 1 to n, never with
   holes: removing from the middle closes the gap, so the length is always exact; stored as one contiguous array)
   and *keyed* (any key type; stored as a hash map). The same words work on both where they make sense: length,
   going through each item, checking whether something is in it.
3. **No `nil` holes.** Reading a position or key that isn't there is an error, never a silent nothing.

Open: all syntax (the sketch was `scores = table 90, 85, 70` and `ages = table "Ada": 36, "Bob": 41`); the words
for reading, changing, adding and removing; going through each item, which needs loops; whether a table can be
empty when made, and what type it gets then.

## D33 — How tables are written (2026-11-02)

Owner, after redstone design claude offered a word with commas, brackets, or a `table:` block:

```
scores = 90, 65, 70
ages = table:
"Ada": 36
"Bbn": 41
end_of table
```

("Btw, there's no `table` in the one line ver"), then "no" to making the two forms symmetric.

- **An in-order table is written on one line, its values joined by commas**, with no keyword: `scores = 90, 65, 70`.
- **A keyed table is written as a block:** `table:`, then one `key: value` per line, closed by `end_of table`
  (D25).
- Not symmetric: no one-line keyed table, no block of in-order values.

Open: a table with one item (`90` alone is an int; a trailing comma, `90,`, is easy to miss); an in-order table too
long for one line; where else commas may appear (they now make tables, so later function calls can't use bare
commas for arguments); empty tables.

## D34 — Reading from a table: `value at position`, `value at offset`, `value of` (2026-11-02)

Shaped together: redstone design claude offered `at`, brackets, or `item 1 of scores`; owner: "`show value of "Ada"
in ages`?", "`at position` & `at offset`, so it wouldn't be confusing?", "For `"Ada"`, might wanna use `of` instead
of `at`", then "I pick 3" (the `value … in` form).

```
show value at position 1 in scores     # counting from 1: the first item
show value at offset 0 in scores       # counting from 0: also the first item
show value of "Ada" in ages            # a key
name = ask "Whose age? "
show value of name in ages             # a key held in a variable
```

- **An in-order table is read by position or by offset:** `at position` counts from 1, `at offset` from 0, so each
  read says how it counts. `position` and `offset` swap, so `at position` is two things (D5).
- **A keyed table is read by key, with `of`.**
- Reading a position, offset or key that isn't there is an error (D32).

Open: changing an item (`value at position 1 in scores is_now 95`); whether `value`, `at`, `position`, `offset`,
`of` and `in` become reserved words; reading with a key of a different number type (an int key in a table keyed by
rationals).

## D35 — Changing an item: `is_now` (2026-11-02)

Owner: "yes" (to redstone design claude's proposal, reusing D14's `is_now`).

```
value at position 1 in scores is_now 95
value of "Ada" in ages is_now 37
```

The new value has the type of the table's values (D4, D32).

Open: whether another variable holding the same table sees the change (`b = a`, then changing `b`); adding a key
this way (`value of "Cy" in ages is_now 7` when `"Cy"` isn't there); changing an item in a table inside a table.

## D36 — No LLVM; Redstone optimizes its own IR for Cranelift (2026-11-02)

Owner, asked "Should we drop LLVM?", then "Yes, but I mean we could optimize the IR we give to Cranelift. Or optimize
ourselves", to redstone design claude's recommendation (LLVM is the heaviest part of D3, a third tier makes moving
between tiers much harder, and Cranelift's code is usually within a modest factor of LLVM's).

- **Two tiers:** code starts in the interpreter; code that runs often is compiled with Cranelift. Replaces D3's
  LLVM tier and D8's LLVM layers (`inkwell`, `llvm-sys`, the C++ wrapper).
- **Redstone optimizes its own IR before handing it to Cranelift**, for what Cranelift doesn't do and for what only
  Redstone knows (its types, rationals, tables).

Open: which optimizations, and in what order.

## D37 — Tables are copied, not shared (2026-11-02)

Owner: "yes" (to redstone design claude's recommendation, over sharing as in Lua, Python and Ruby).

`b = a` gives `b` its own table, as numbers and text already behave: changing an item in `b` leaves `a` as it was.
The same holds for a table put inside another table. Copies are made only when something changes (copy-on-write),
so passing a table around costs nothing until then. Settles D35's open question.

```
a = 1, 2
b = a
value at position 1 in b is_now 5
show value at position 1 in a     # 1
```

## D38 — `show items in … until` (2026-11-02)

Shaped together, after redstone design claude offered loops (`for_each`) and `show everything_inside scores`; owner:
"Actually, nvm. Let's do `show items in scores until last_item`, `show items in scores until <>`", then for the stop:
"`until value 65`", "`until current_item less_than 70`", "should there be `until before`?", and "Yes" to the set,
with `until_before` one word by D5 (as `else_if` beside `else`).

With `scores = 90, 65, 70, 65`:

```
show items in scores until last_item                          # 90 65 70 65
show items in scores until position 2                         # 90 65
show items in scores until_before position 2                  # 90
show items in scores until value 65                           # 90 65
show items in scores until_before value 65                    # 90
show items in scores until current_item less_than 70          # 90 65
show items in scores until_before current_item less_than 70   # 90
```

- Items are shown from the first, one per line. `until` includes the item where it stops; `until_before` stops just
  before it.
- The stop is `last_item`, `position`, `value` (the first item equal to it), or a test on `current_item` (the first
  item that makes it true).

Open: a stop that is never reached (a value that never appears, a test never true); whether `items in … until` is
only a `show` form or works anywhere a table can (`first_two = items in scores until position 2`); keyed tables;
stopping at an offset; loops, which this does not replace.

## D39 — Loops: `repeat … times`, `repeat until`, `for items in` (2026-11-02)

Owner: "Maybe the loop would be `for` in front?" (of D38's form), "Or maybe a more general loop outside of `show`?",
then "yes" to redstone design claude's set.

```
repeat 3 times:
    show "Hip hip hooray"
end_of repeat

repeat until lives is 0:
    …
end_of repeat

for items in scores until last_item:
    total is_now total plus current_item
end_of for
```

- **`repeat <count> times:`** runs its block that many times.
- **`repeat until <test>:`** checks the test before each round and stops when it is true; it also serves as "keep
  going while…".
- **`for items in <table> until <stop>:`** goes through an in-order table with D38's stops (`last_item`, `position`,
  `value`, a test on `current_item`) and `until`/`until_before`; `current_item` holds the item in the body.
- Each closes with `end_of repeat` or `end_of for` (D25), with a colon after its first line (D22).

Open: leaving a loop early; a count variable for `repeat … times`; naming the item for nested `for` loops (an inner
`current_item` hides the outer); repeating a negative number of times; a `for` whose stop is never reached; a
trailing `if` on lines other than `show` (D20).

## D40 — `show … 50x` (2026-11-02)

Owner:

```
show "Hello, World!" 50x
```

"means print that 50 times."

A whole number written with `x` right after it (`50x`, one thing by D5), after `show`'s value, shows the value that
many times, one per line. A trailing `if` comes after it: `show "Hi" 3x if loud`.

Open: a count held in a variable; `x` counts after anything but `show`'s value (other statements, `show items in`);
whether the value is worked out once or once per line (it matters only for `ask`).

## D41 — `repeat forever` and `stop` (2026-11-02)

Owner: "what about a `repeat forever` loop too?", then "`stop`" for the word that leaves a loop (redstone design
claude offered `stop`, `leave_loop` or `break`, and recommended `leave_loop`).

```
repeat forever:
    answer = ask "Type something (or quit): "
    stop if answer is "quit"
    show answer
end_of repeat
```

- **`repeat forever:`** runs its block until something leaves it.
- **`stop`** leaves the loop it is in, the innermost one, in any loop (`repeat … times`, `repeat until`,
  `repeat forever`, `for items in`). It takes a trailing `if`, as in the example. Outside a loop it is a mistake.

## D42 — `becomes`, not `is_now` (2026-11-02)

Owner: "`is_now` reads a bit weird, can we change it to `become`?", then "with s" (redstone design claude offered
`becomes`, which reads as a sentence, or `become`, which reads as a command).

`becomes` changes a variable or an item: `age becomes age plus 1`, `value of "Ada" in ages becomes 37`. Replaces
`is_now` in D14, D15, D20 and D35. It no longer looks like `is`.
