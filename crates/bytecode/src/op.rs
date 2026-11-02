//! The bytecode: typed registers and the instructions that work on them.

use std::rc::Rc;

use easium_front::Span;
use easium_front::ir::{Comparison, Counting};
use easium_runtime::{Rational, Value};

/// A register, named by the file it lives in. Booleans live in the int file as 0 or 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reg {
    /// An int.
    Int(u32),
    /// A boolean, 0 or 1 in the int file.
    Boolean(u32),
    /// A rational.
    Rational(u32),
    /// Text or a table.
    Object(u32),
}

/// The second operand of a compare-and-jump: a register, or a constant written in the program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operand {
    /// An int or boolean register.
    Register(u32),
    /// A constant.
    Constant(i64),
}

/// A piece of a `show_using_format` line: text as written, or a hole already turned into text.
#[derive(Clone, Debug)]
pub enum Piece {
    /// Text as written.
    Text(Rc<str>),
    /// An object register holding a hole's text.
    Hole(u32),
}

/// One instruction. Spans say where to point when the instruction fails.
#[derive(Clone, Debug)]
pub enum Op {
    /// `ints[dst] = value`
    IntConstant { dst: u32, value: i64 },
    /// `rationals[dst] = value`
    RationalConstant { dst: u32, value: Rational },
    /// `objects[dst] = value`
    ObjectConstant { dst: u32, value: Value },
    /// `ints[dst] = ints[src]`
    MoveInt { dst: u32, src: u32 },
    /// `rationals[dst] = rationals[src]`
    MoveRational { dst: u32, src: u32 },
    /// `objects[dst] = objects[src]`, sharing until a change copies it (D37).
    MoveObject { dst: u32, src: u32 },
    /// Checked int arithmetic.
    Add { dst: u32, a: u32, b: u32, at: Span },
    /// Checked int arithmetic.
    Subtract { dst: u32, a: u32, b: u32, at: Span },
    /// Checked int arithmetic.
    Multiply { dst: u32, a: u32, b: u32, at: Span },
    /// `remainder` on ints.
    Remainder { dst: u32, a: u32, b: u32, at: Span },
    /// Checked `ints[a] + value`, for a constant written in the program.
    AddConstant {
        dst: u32,
        a: u32,
        value: i64,
        at: Span,
    },
    /// Checked `ints[a] - value`.
    SubtractConstant {
        dst: u32,
        a: u32,
        value: i64,
        at: Span,
    },
    /// Checked `ints[a] * value`.
    MultiplyConstant {
        dst: u32,
        a: u32,
        value: i64,
        at: Span,
    },
    /// `remainder` by a positive constant.
    RemainderConstant {
        dst: u32,
        a: u32,
        value: i64,
        at: Span,
    },
    /// Compares an int or boolean with a constant.
    CompareConstant {
        op: Comparison,
        dst: u32,
        a: u32,
        value: i64,
    },
    /// The end of a counted round: `ints[counter] += 1`, then back to `to` while it is below `ints[limit]`;
    /// otherwise on to the next instruction, which goes to the loop's own test.
    CountUp { counter: u32, limit: u32, to: usize },
    /// Goes to an instruction when comparing two ints or booleans gives true.
    JumpIfCompare {
        op: Comparison,
        a: u32,
        b: Operand,
        to: usize,
    },
    /// An int divided by an int, giving a rational (D7).
    DivideInts { dst: u32, a: u32, b: u32, at: Span },
    /// An int as a rational (D29).
    IntToRational { dst: u32, src: u32 },
    /// `ints[dst] += 1`, for loop counters that can't overflow in practice.
    Increment { dst: u32 },
    /// Rational arithmetic.
    AddRationals { dst: u32, a: u32, b: u32, at: Span },
    /// Rational arithmetic.
    SubtractRationals { dst: u32, a: u32, b: u32, at: Span },
    /// Rational arithmetic.
    MultiplyRationals { dst: u32, a: u32, b: u32, at: Span },
    /// Rational arithmetic.
    DivideRationals { dst: u32, a: u32, b: u32, at: Span },
    /// Compares two ints or two booleans.
    CompareInts {
        op: Comparison,
        dst: u32,
        a: u32,
        b: u32,
    },
    /// Compares two rationals.
    CompareRationals {
        op: Comparison,
        dst: u32,
        a: u32,
        b: u32,
    },
    /// `is` or `is_not` on two objects.
    CompareObjects {
        equal: bool,
        dst: u32,
        a: u32,
        b: u32,
    },
    /// `not`
    Not { dst: u32, src: u32 },
    /// Goes to an instruction.
    Jump { to: usize },
    /// Goes to an instruction when a boolean is false.
    JumpIfFalse { condition: u32, to: usize },
    /// Goes to an instruction when a boolean is true.
    JumpIfTrue { condition: u32, to: usize },
    /// Goes to an instruction when `ints[a] >= ints[b]`.
    JumpIfAtLeast { a: u32, b: u32, to: usize },
    /// Turns any value into text, as `show` would show it.
    ToText { dst: u32, src: Reg, at: Span },
    /// Joins two pieces of text.
    Join { dst: u32, a: u32, b: u32 },
    /// Puts a `show_using_format` line together.
    Format { dst: u32, pieces: Box<[Piece]> },
    /// `ask`
    Ask { dst: u32, prompt: u32, at: Span },
    /// Shows a value `count` times, one per line.
    Show { src: Reg, count: i64, at: Span },
    /// Makes an in-order table.
    MakeInOrder { dst: u32, items: Box<[Reg]> },
    /// Starts an empty keyed table.
    StartKeyed { dst: u32 },
    /// Stops if a keyed table being made already has this key.
    CheckNewKey { table: u32, key: Reg, at: Span },
    /// Adds a key and value to a keyed table being made.
    PushKeyed { table: u32, key: Reg, value: Reg },
    /// Reads an in-order table at a position or offset.
    ReadPosition {
        dst: Reg,
        table: u32,
        index: u32,
        counting: Counting,
        at: Span,
    },
    /// Adds up an in-order table of ints or rationals (D56).
    Sum { dst: Reg, table: u32, at: Span },
    /// Reads a keyed table by key.
    ReadKey {
        dst: Reg,
        table: u32,
        key: Reg,
        at: Span,
    },
    /// Changes an item of an in-order table.
    ChangePosition {
        table: u32,
        index: u32,
        counting: Counting,
        value: Reg,
        at: Span,
    },
    /// Changes the value at a key of a keyed table.
    ChangeKey {
        table: u32,
        key: Reg,
        value: Reg,
        at: Span,
    },
    /// `ints[dst]` = the number of items in an in-order table.
    Length { dst: u32, table: u32 },
    /// Reads an in-order table at a zero-based index known to be inside it.
    Item { dst: Reg, table: u32, index: u32 },
    /// `ints[dst]` = whether the item at a zero-based index equals a value.
    ItemEquals {
        dst: u32,
        table: u32,
        index: u32,
        value: Reg,
    },
    /// `ints[dst]` = the zero-based index of a position, stopping if the table has no such position.
    StopPosition {
        dst: u32,
        table: u32,
        position: u32,
        at: Span,
    },
    /// `ints[dst]` = the zero-based index of the first item equal to a value, stopping if there is none.
    FindValue {
        dst: u32,
        table: u32,
        value: Reg,
        at: Span,
    },
    /// Shows the first `ints[end]` items of an in-order table, one per line.
    ShowItems { table: u32, end: u32, at: Span },
    /// Stops if an int is negative: a `repeat` count (D39).
    CheckCount { src: u32, at: Span },
    /// Stops: a `for` or `show items` stop that was never reached (D38).
    NeverReached { at: Span },
}
