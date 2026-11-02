//! Everything unsafe in the Cranelift tier (rule 5), and why.
//!
//! - **Calling a compiled loop.** Cranelift hands back a code pointer; calling it needs `unsafe`. Each loop is
//!   compiled for one bytecode program and reads and writes int registers only at indices checked against that
//!   program's register files when it was compiled; [`Compiled::run`] checks the files lent to it are that size
//!   before calling, and lets tier 0 run the loop otherwise.
//! - **The helpers compiled loops call.** They get raw pointers from the compiled code, always with a length, and
//!   turn them back into slices; every read and write then goes through safe, bounds-checked indexing, so a wrong
//!   index can only make a helper report failure, never touch memory outside the registers.
//!
//! A helper never writes when it fails, so tier 0 can run the same instruction again and report the error
//! exactly as it always does (rule 10).
#![allow(unsafe_code)]

use std::cmp::Ordering;

use redstone_bytecode::{CompiledLoop, Registers};
use redstone_runtime::{Rational, Table, Value};

/// A compiled loop's machine code: `(ints, rationals, rationals' length, objects, objects' length)` to the
/// instruction tier 0 goes on from.
pub(crate) type LoopFunction =
    extern "C" fn(*mut i64, *mut Rational, i64, *const Value, i64) -> i64;

/// A compiled loop and the register file sizes it was compiled for.
pub(crate) struct Compiled {
    function: LoopFunction,
    /// The loop's rational constants, which its code copies from; owned here so they live as long as the code.
    _constants: Box<[Rational]>,
    ints: usize,
    rationals: usize,
    objects: usize,
    head: usize,
}

impl Compiled {
    /// Wraps a pointer from `JITModule::get_finalized_function` for a function with [`LoopFunction`]'s
    /// signature, compiled for register files of these sizes.
    pub(crate) fn new(
        pointer: *const u8,
        constants: Box<[Rational]>,
        ints: usize,
        rationals: usize,
        objects: usize,
        head: usize,
    ) -> Self {
        // SAFETY: the pointer is a finished Cranelift function whose signature was declared to match
        // `LoopFunction`; it stays valid while the `JITModule` that made it lives, and the module lives in the
        // `Cranelift` compiler, which outlives every run of tier 0 it serves.
        let function = unsafe { std::mem::transmute::<*const u8, LoopFunction>(pointer) };
        Self {
            function,
            _constants: constants,
            ints,
            rationals,
            objects,
            head,
        }
    }
}

impl CompiledLoop for Compiled {
    fn run(&self, registers: &mut Registers<'_>) -> usize {
        if registers.ints.len() != self.ints
            || registers.rationals.len() != self.rationals
            || registers.objects.len() != self.objects
        {
            return self.head;
        }
        let rationals_length = i64::try_from(registers.rationals.len()).unwrap_or(0);
        let objects_length = i64::try_from(registers.objects.len()).unwrap_or(0);
        let resume = (self.function)(
            registers.ints.as_mut_ptr(),
            registers.rationals.as_mut_ptr(),
            rationals_length,
            registers.objects.as_ptr(),
            objects_length,
        );
        usize::try_from(resume).unwrap_or(self.head)
    }
}

fn rationals<'a>(pointer: *mut Rational, length: i64) -> &'a mut [Rational] {
    let length = usize::try_from(length).unwrap_or(0);
    // SAFETY: the compiled loop passes tier 0's rational register file and its exact length, lent mutably for
    // the whole call, and nothing else touches it while a helper runs.
    unsafe { std::slice::from_raw_parts_mut(pointer, length) }
}

fn objects<'a>(pointer: *const Value, length: i64) -> &'a [Value] {
    let length = usize::try_from(length).unwrap_or(0);
    // SAFETY: the compiled loop passes tier 0's object register file and its exact length, lent for the whole
    // call; compiled loops and helpers only read it.
    unsafe { std::slice::from_raw_parts(pointer, length) }
}

fn index(value: i64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// The in-order table in an object register, if that's what it holds.
fn in_order(objects: &[Value], table: i64) -> Option<&[Value]> {
    match objects.get(index(table))? {
        Value::Table(contents) => match &**contents {
            Table::InOrder(values) => Some(values),
            Table::Keyed(_) => None,
        },
        _ => None,
    }
}

/// `rationals[dst] = *constant`, a constant from the compiled loop's own pool.
pub(crate) extern "C" fn rational_copy(
    rationals_pointer: *mut Rational,
    rationals_length: i64,
    dst: i64,
    constant: *const Rational,
) -> i64 {
    // SAFETY: `constant` points into the pool of the compiled loop calling this, built before its code and kept
    // alive with it in `Compiled`, so it points at a valid, aligned `Rational`.
    let value = unsafe { constant.read() };
    match rationals(rationals_pointer, rationals_length).get_mut(index(dst)) {
        Some(slot) => {
            *slot = value;
            0
        }
        None => 1,
    }
}

/// `rationals[dst] = rationals[src]`.
pub(crate) extern "C" fn rational_move(
    rationals_pointer: *mut Rational,
    rationals_length: i64,
    dst: i64,
    src: i64,
) -> i64 {
    let file = rationals(rationals_pointer, rationals_length);
    let Some(value) = file.get(index(src)).copied() else {
        return 1;
    };
    match file.get_mut(index(dst)) {
        Some(slot) => {
            *slot = value;
            0
        }
        None => 1,
    }
}

/// `rationals[dst] = value`, an int as a rational.
pub(crate) extern "C" fn rational_from_int(
    rationals_pointer: *mut Rational,
    rationals_length: i64,
    dst: i64,
    value: i64,
) -> i64 {
    match rationals(rationals_pointer, rationals_length).get_mut(index(dst)) {
        Some(slot) => {
            *slot = Rational::from_int(value);
            0
        }
        None => 1,
    }
}

/// `rationals[dst] = a / b` for two ints; fails, writing nothing, on division by zero or overflow.
pub(crate) extern "C" fn rational_divide_ints(
    rationals_pointer: *mut Rational,
    rationals_length: i64,
    dst: i64,
    a: i64,
    b: i64,
) -> i64 {
    let file = rationals(rationals_pointer, rationals_length);
    match (Rational::new(a.into(), b.into()), file.get_mut(index(dst))) {
        (Ok(value), Some(slot)) => {
            *slot = value;
            0
        }
        _ => 1,
    }
}

/// `rationals[dst] = rationals[a] op rationals[b]`, `op` 0 to 3 for plus, minus, times, divided by; fails, writing
/// nothing, on division by zero or a result too big.
pub(crate) extern "C" fn rational_arithmetic(
    rationals_pointer: *mut Rational,
    rationals_length: i64,
    op: i64,
    dst: i64,
    a: i64,
    b: i64,
) -> i64 {
    let file = rationals(rationals_pointer, rationals_length);
    let (Some(left), Some(right)) = (file.get(index(a)).copied(), file.get(index(b)).copied())
    else {
        return 1;
    };
    let result = match op {
        0 => left.add_quickly(right),
        1 => left.subtract_quickly(right),
        2 => left.multiply_quickly(right),
        3 => left.divide_quickly(right),
        _ => return 1,
    };
    match (result, file.get_mut(index(dst))) {
        (Ok(value), Some(slot)) => {
            *slot = value;
            0
        }
        _ => 1,
    }
}

/// How `rationals[a]` compares with `rationals[b]`: -1, 0 or 1; 2 if a register is missing.
pub(crate) extern "C" fn rational_compare(
    rationals_pointer: *mut Rational,
    rationals_length: i64,
    a: i64,
    b: i64,
) -> i64 {
    let file = rationals(rationals_pointer, rationals_length);
    match (file.get(index(a)), file.get(index(b))) {
        (Some(left), Some(right)) => match left.cmp(right) {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        },
        _ => 2,
    }
}

/// How many items the in-order table in `objects[table]` has; -1 if it isn't one.
pub(crate) extern "C" fn table_length(
    objects_pointer: *const Value,
    objects_length: i64,
    table: i64,
) -> i64 {
    in_order(objects(objects_pointer, objects_length), table)
        .map_or(-1, |values| i64::try_from(values.len()).unwrap_or(-1))
}

/// Writes the int or boolean at zero-based `item` of the in-order table in `objects[table]` to `out`; fails,
/// writing nothing, if there's no such item.
pub(crate) extern "C" fn table_int(
    objects_pointer: *const Value,
    objects_length: i64,
    table: i64,
    item: i64,
    out: *mut i64,
) -> i64 {
    let value = match in_order(objects(objects_pointer, objects_length), table)
        .and_then(|values| values.get(index(item)))
    {
        Some(Value::Int(value)) => *value,
        Some(Value::Boolean(value)) => i64::from(*value),
        _ => return 1,
    };
    // SAFETY: `out` is the address of an 8-byte, 8-aligned stack slot in the calling compiled loop.
    unsafe { out.write(value) };
    0
}

/// Writes the rational at zero-based `item` of the in-order table in `objects[table]` to `rationals[dst]`; fails,
/// writing nothing, if there's no such item.
pub(crate) extern "C" fn table_rational(
    objects_pointer: *const Value,
    objects_length: i64,
    rationals_pointer: *mut Rational,
    rationals_length: i64,
    table: i64,
    item: i64,
    dst: i64,
) -> i64 {
    let value = match in_order(objects(objects_pointer, objects_length), table)
        .and_then(|values| values.get(index(item)))
    {
        Some(Value::Rational(value)) => *value,
        _ => return 1,
    };
    match rationals(rationals_pointer, rationals_length).get_mut(index(dst)) {
        Some(slot) => {
            *slot = value;
            0
        }
        None => 1,
    }
}
