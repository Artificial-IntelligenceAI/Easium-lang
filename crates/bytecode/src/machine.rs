//! Runs bytecode. Every instruction reads all it needs before it writes, so a register can be both read and
//! written by one instruction.

use std::cmp::Ordering;
use std::io::{BufRead, Write};
use std::rc::Rc;

use easium_front::ir::{Comparison, Counting};
use easium_front::{Diagnostic, Span};
use easium_runtime::{Rational, RationalError, Table, Value};

use crate::compile::Code;
use crate::op::{Op, Piece, Reg};
use crate::{CompiledLoop, Registers, Tiering};

/// Runs compiled bytecode, handing hot loops to `tiering` when given (D3, D36).
///
/// # Errors
///
/// A problem found while running, the same as the reference interpreter would find (rule 10).
pub fn execute(
    code: &Code,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    tiering: Option<&mut Tiering>,
) -> Result<(), Diagnostic> {
    let mut machine = Machine {
        ints: vec![0; code.ints],
        rationals: vec![Rational::from_int(0); code.rationals],
        objects: vec![Value::Boolean(false); code.objects],
        input,
        output,
    };
    let mut loops: Vec<LoopState> = Vec::new();
    if tiering.is_some() {
        loops.resize_with(code.ops.len(), || LoopState::Counting(0));
    }
    machine.run(code, tiering, &mut loops)
}

/// What tier 0 knows about the loop whose head is at an instruction.
enum LoopState {
    /// Not hot yet: how many times a jump has gone back to it.
    Counting(u32),
    /// Compiled; runs from its head and says where to go on from.
    Compiled(Box<dyn CompiledLoop>),
    /// The compiler couldn't take it; stays in tier 0.
    Interpreted,
}

struct Machine<'io> {
    ints: Vec<i64>,
    rationals: Vec<Rational>,
    objects: Vec<Value>,
    input: &'io mut dyn BufRead,
    output: &'io mut dyn Write,
}

impl Machine<'_> {
    /// Called on a jump back to `head` from `back`: counts the loop, compiles it once hot, and runs it compiled
    /// when it can. Returns where to go on from.
    fn back_edge(
        &mut self,
        code: &Code,
        head: usize,
        back: usize,
        tiering: &mut Tiering,
        loops: &mut [LoopState],
    ) -> usize {
        let Some(state) = loops.get_mut(head) else {
            return head;
        };
        if let LoopState::Counting(count) = state {
            *count += 1;
            if *count > tiering.threshold {
                *state = match tiering.compiler.compile(code, head, back) {
                    Some(compiled) => LoopState::Compiled(compiled),
                    None => LoopState::Interpreted,
                };
            }
        }
        match state {
            LoopState::Compiled(compiled) => compiled.run(&mut Registers {
                ints: &mut self.ints,
                rationals: &mut self.rationals,
                objects: &self.objects,
            }),
            LoopState::Counting(_) | LoopState::Interpreted => head,
        }
    }

    fn run(
        &mut self,
        code: &Code,
        mut tiering: Option<&mut Tiering>,
        loops: &mut [LoopState],
    ) -> Result<(), Diagnostic> {
        let ops = &code.ops;
        let mut pc = 0;
        while let Some(op) = ops.get(pc) {
            pc += 1;
            match op {
                Op::Jump { to } => {
                    pc = match tiering.as_deref_mut() {
                        Some(tiering) if *to < pc => {
                            self.back_edge(code, *to, pc - 1, tiering, loops)
                        }
                        _ => *to,
                    };
                }
                Op::JumpIfFalse { condition, to } => {
                    if self.ints[*condition as usize] == 0 {
                        pc = *to;
                    }
                }
                Op::JumpIfTrue { condition, to } => {
                    if self.ints[*condition as usize] != 0 {
                        pc = *to;
                    }
                }
                Op::JumpIfAtLeast { a, b, to } => {
                    if self.ints[*a as usize] >= self.ints[*b as usize] {
                        pc = *to;
                    }
                }
                // The most frequent instructions run here, without a call; errors go out through cold paths.
                Op::MoveInt { dst, src } => self.ints[*dst as usize] = self.ints[*src as usize],
                Op::IntConstant { dst, value } => self.ints[*dst as usize] = *value,
                Op::Increment { dst } => {
                    let reg = &mut self.ints[*dst as usize];
                    *reg = reg.wrapping_add(1);
                }
                Op::Add { dst, a, b, at } => {
                    match self.ints[*a as usize].checked_add(self.ints[*b as usize]) {
                        Some(result) => self.ints[*dst as usize] = result,
                        None => return Err(overflow(*at)),
                    }
                }
                Op::CompareInts { op, dst, a, b } => {
                    let ordering = self.ints[*a as usize].cmp(&self.ints[*b as usize]);
                    self.ints[*dst as usize] = i64::from(compared(*op, ordering));
                }
                Op::Item { dst, table, index } => {
                    let index = self.ints[*index as usize];
                    let item = match &self.objects[*table as usize] {
                        Value::Table(contents) => match &**contents {
                            Table::InOrder(values) => usize::try_from(index)
                                .ok()
                                .and_then(|index| values.get(index)),
                            Table::Keyed(_) => None,
                        },
                        _ => None,
                    };
                    match (item, *dst) {
                        (Some(Value::Int(value)), Reg::Int(reg)) => {
                            self.ints[reg as usize] = *value;
                        }
                        (Some(item), dst) => {
                            let item = item.clone();
                            self.store(dst, item, Span::new(0, 0))?;
                        }
                        (None, _) => return Err(item_bug()),
                    }
                }
                Op::AddRationals { dst, a, b, at } => {
                    let result = self.rationals[*a as usize]
                        .add_quickly(self.rationals[*b as usize])
                        .map_err(|error| rational_error(error, *at))?;
                    self.rationals[*dst as usize] = result;
                }
                Op::RationalConstant { dst, value } => self.rationals[*dst as usize] = *value,
                Op::MoveRational { dst, src } => {
                    self.rationals[*dst as usize] = self.rationals[*src as usize];
                }
                _ => self.step(op)?,
            }
        }
        Ok(())
    }

    fn int(&self, reg: u32) -> i64 {
        self.ints[reg as usize]
    }

    fn set_int(&mut self, reg: u32, value: i64) {
        self.ints[reg as usize] = value;
    }

    fn rational(&self, reg: u32) -> Rational {
        self.rationals[reg as usize]
    }

    fn set_rational(&mut self, reg: u32, value: Rational) {
        self.rationals[reg as usize] = value;
    }

    fn object(&self, reg: u32) -> &Value {
        &self.objects[reg as usize]
    }

    fn set_object(&mut self, reg: u32, value: Value) {
        self.objects[reg as usize] = value;
    }

    fn value(&self, reg: Reg) -> Value {
        match reg {
            Reg::Int(reg) => Value::Int(self.int(reg)),
            Reg::Boolean(reg) => Value::Boolean(self.int(reg) != 0),
            Reg::Rational(reg) => Value::Rational(self.rational(reg)),
            Reg::Object(reg) => self.object(reg).clone(),
        }
    }

    fn store(&mut self, reg: Reg, value: Value, at: Span) -> Result<(), Diagnostic> {
        match (reg, value) {
            (Reg::Int(reg), Value::Int(value)) => self.set_int(reg, value),
            (Reg::Boolean(reg), Value::Boolean(value)) => self.set_int(reg, i64::from(value)),
            (Reg::Rational(reg), Value::Rational(value)) => self.set_rational(reg, value),
            (Reg::Object(reg), value @ (Value::Text(_) | Value::Table(_))) => {
                self.set_object(reg, value);
            }
            _ => return Err(type_bug(at)),
        }
        Ok(())
    }

    fn in_order(&self, reg: u32, at: Span) -> Result<&[Value], Diagnostic> {
        match self.object(reg) {
            Value::Table(table) => match &**table {
                Table::InOrder(values) => Ok(values),
                Table::Keyed(_) => Err(type_bug(at)),
            },
            _ => Err(type_bug(at)),
        }
    }

    fn keyed(&self, reg: u32, at: Span) -> Result<&[(Value, Value)], Diagnostic> {
        match self.object(reg) {
            Value::Table(table) => match &**table {
                Table::Keyed(pairs) => Ok(pairs),
                Table::InOrder(_) => Err(type_bug(at)),
            },
            _ => Err(type_bug(at)),
        }
    }

    /// A table to change, copied first if something else also holds it (D37).
    fn table_mut(&mut self, reg: u32, at: Span) -> Result<&mut Table, Diagnostic> {
        match &mut self.objects[reg as usize] {
            Value::Table(table) => Ok(Rc::make_mut(table)),
            _ => Err(type_bug(at)),
        }
    }

    fn write_line(&mut self, text: &str, at: Span) -> Result<(), Diagnostic> {
        writeln!(self.output, "{text}").map_err(|error| output_failed(&error, at))
    }

    #[allow(clippy::too_many_lines)]
    fn step(&mut self, op: &Op) -> Result<(), Diagnostic> {
        match op {
            Op::IntConstant { dst, value } => self.set_int(*dst, *value),
            Op::RationalConstant { dst, value } => self.set_rational(*dst, *value),
            Op::ObjectConstant { dst, value } => self.set_object(*dst, value.clone()),
            Op::MoveInt { dst, src } => self.set_int(*dst, self.int(*src)),
            Op::MoveRational { dst, src } => self.set_rational(*dst, self.rational(*src)),
            Op::MoveObject { dst, src } => self.set_object(*dst, self.object(*src).clone()),
            Op::Add { dst, a, b, at } => {
                let result = self
                    .int(*a)
                    .checked_add(self.int(*b))
                    .ok_or_else(|| overflow(*at))?;
                self.set_int(*dst, result);
            }
            Op::Subtract { dst, a, b, at } => {
                let result = self
                    .int(*a)
                    .checked_sub(self.int(*b))
                    .ok_or_else(|| overflow(*at))?;
                self.set_int(*dst, result);
            }
            Op::Multiply { dst, a, b, at } => {
                let result = self
                    .int(*a)
                    .checked_mul(self.int(*b))
                    .ok_or_else(|| overflow(*at))?;
                self.set_int(*dst, result);
            }
            Op::Remainder { dst, a, b, at } => {
                let (left, right) = (self.int(*a), self.int(*b));
                if right == 0 {
                    return Err(division_by_zero(*at));
                }
                if left < 0 || right < 0 {
                    return Err(Diagnostic::not_decided(
                        "`remainder` with a negative number",
                        Some("D16"),
                        *at,
                    ));
                }
                let result = left.checked_rem(right).ok_or_else(|| overflow(*at))?;
                self.set_int(*dst, result);
            }
            Op::DivideInts { dst, a, b, at } => {
                let result = Rational::new(self.int(*a).into(), self.int(*b).into())
                    .map_err(|error| rational_error(error, *at))?;
                self.set_rational(*dst, result);
            }
            Op::IntToRational { dst, src } => {
                self.set_rational(*dst, Rational::from_int(self.int(*src)));
            }
            Op::Increment { dst } => self.set_int(*dst, self.int(*dst).wrapping_add(1)),
            Op::AddRationals { dst, a, b, at } => {
                let result = self
                    .rational(*a)
                    .add_quickly(self.rational(*b))
                    .map_err(|error| rational_error(error, *at))?;
                self.set_rational(*dst, result);
            }
            Op::SubtractRationals { dst, a, b, at } => {
                let result = self
                    .rational(*a)
                    .subtract_quickly(self.rational(*b))
                    .map_err(|error| rational_error(error, *at))?;
                self.set_rational(*dst, result);
            }
            Op::MultiplyRationals { dst, a, b, at } => {
                let result = self
                    .rational(*a)
                    .multiply_quickly(self.rational(*b))
                    .map_err(|error| rational_error(error, *at))?;
                self.set_rational(*dst, result);
            }
            Op::DivideRationals { dst, a, b, at } => {
                let result = self
                    .rational(*a)
                    .divide_quickly(self.rational(*b))
                    .map_err(|error| rational_error(error, *at))?;
                self.set_rational(*dst, result);
            }
            Op::CompareInts { op, dst, a, b } => {
                let result = compared(*op, self.int(*a).cmp(&self.int(*b)));
                self.set_int(*dst, i64::from(result));
            }
            Op::CompareRationals { op, dst, a, b } => {
                let result = compared(*op, self.rational(*a).cmp(&self.rational(*b)));
                self.set_int(*dst, i64::from(result));
            }
            Op::CompareObjects { equal, dst, a, b } => {
                let same = self.object(*a) == self.object(*b);
                self.set_int(*dst, i64::from(same == *equal));
            }
            Op::Not { dst, src } => self.set_int(*dst, i64::from(self.int(*src) == 0)),
            Op::ToText { dst, src, at } => {
                let text = shown_text(&self.value(*src), *at)?;
                self.set_object(*dst, Value::Text(Rc::from(text)));
            }
            Op::Join { dst, a, b } => {
                let joined = match (self.object(*a), self.object(*b)) {
                    (Value::Text(a), Value::Text(b)) => format!("{a}{b}"),
                    _ => return Err(type_bug(Span::new(0, 0))),
                };
                self.set_object(*dst, Value::Text(Rc::from(joined)));
            }
            Op::Format { dst, pieces } => {
                let mut text = String::new();
                for piece in pieces {
                    match piece {
                        Piece::Text(piece) => text.push_str(piece),
                        Piece::Hole(reg) => match self.object(*reg) {
                            Value::Text(hole) => text.push_str(hole),
                            _ => return Err(type_bug(Span::new(0, 0))),
                        },
                    }
                }
                self.set_object(*dst, Value::Text(Rc::from(text)));
            }
            Op::Ask { dst, prompt, at } => {
                let line = self.ask(*prompt, *at)?;
                self.set_object(*dst, Value::Text(Rc::from(line)));
            }
            Op::Show { src, count, at } => {
                if *count > 0 {
                    let text = shown_text(&self.value(*src), *at)?;
                    for _ in 0..*count {
                        self.write_line(&text, *at)?;
                    }
                }
            }
            Op::MakeInOrder { dst, items } => {
                let values = items.iter().map(|item| self.value(*item)).collect();
                self.set_object(*dst, Value::Table(Rc::new(Table::InOrder(values))));
            }
            Op::StartKeyed { dst } => {
                self.set_object(*dst, Value::Table(Rc::new(Table::Keyed(Vec::new()))));
            }
            Op::CheckNewKey { table, key, at } => {
                let key = self.value(*key);
                if self
                    .keyed(*table, *at)?
                    .iter()
                    .any(|(existing, _)| *existing == key)
                {
                    return Err(Diagnostic::not_decided(
                        "the same key twice in one table",
                        Some("D32"),
                        *at,
                    ));
                }
            }
            Op::PushKeyed { table, key, value } => {
                let pair = (self.value(*key), self.value(*value));
                match self.table_mut(*table, Span::new(0, 0))? {
                    Table::Keyed(pairs) => pairs.push(pair),
                    Table::InOrder(_) => return Err(type_bug(Span::new(0, 0))),
                }
            }
            Op::ReadPosition {
                dst,
                table,
                index,
                counting,
                at,
            } => {
                let index = self.int(*index);
                let values = self.in_order(*table, *at)?;
                let item = zero_based(index, *counting)
                    .and_then(|zero_based| values.get(zero_based))
                    .cloned()
                    .ok_or_else(|| missing_index(index, *counting, values.len(), *at))?;
                self.store(*dst, item, *at)?;
            }
            Op::ReadKey {
                dst,
                table,
                key,
                at,
            } => {
                let key = self.value(*key);
                let item = self
                    .keyed(*table, *at)?
                    .iter()
                    .find(|(existing, _)| *existing == key)
                    .map(|(_, value)| value.clone())
                    .ok_or_else(|| {
                        Diagnostic::mistake(
                            format!("there's no key {} in this table", described_key(&key)),
                            *at,
                        )
                    })?;
                self.store(*dst, item, *at)?;
            }
            Op::ChangePosition {
                table,
                index,
                counting,
                value,
                at,
            } => {
                let (index, new_value) = (self.int(*index), self.value(*value));
                let Table::InOrder(values) = self.table_mut(*table, *at)? else {
                    return Err(type_bug(*at));
                };
                let length = values.len();
                let item = zero_based(index, *counting)
                    .and_then(|zero_based| values.get_mut(zero_based))
                    .ok_or_else(|| missing_index(index, *counting, length, *at))?;
                *item = new_value;
            }
            Op::ChangeKey {
                table,
                key,
                value,
                at,
            } => {
                let (key, new_value) = (self.value(*key), self.value(*value));
                let Table::Keyed(pairs) = self.table_mut(*table, *at)? else {
                    return Err(type_bug(*at));
                };
                let item = pairs
                    .iter_mut()
                    .find(|(existing, _)| *existing == key)
                    .map(|(_, value)| value)
                    .ok_or_else(|| {
                        Diagnostic::not_decided("adding a key by changing it", Some("D35"), *at)
                    })?;
                *item = new_value;
            }
            Op::Length { dst, table } => {
                let length = self.in_order(*table, Span::new(0, 0))?.len();
                self.set_int(*dst, i64::try_from(length).unwrap_or(i64::MAX));
            }
            Op::Item { dst, table, index } => {
                let index = usize::try_from(self.int(*index)).unwrap_or(usize::MAX);
                let item = self
                    .in_order(*table, Span::new(0, 0))?
                    .get(index)
                    .cloned()
                    .ok_or_else(|| {
                        Diagnostic::bug("an item past the end of a table", Span::new(0, 0))
                    })?;
                self.store(*dst, item, Span::new(0, 0))?;
            }
            Op::ItemEquals {
                dst,
                table,
                index,
                value,
            } => {
                let index = usize::try_from(self.int(*index)).unwrap_or(usize::MAX);
                let value = self.value(*value);
                let equal = self.in_order(*table, Span::new(0, 0))?.get(index) == Some(&value);
                self.set_int(*dst, i64::from(equal));
            }
            Op::StopPosition {
                dst,
                table,
                position,
                at,
            } => {
                let position = self.int(*position);
                let length = self.in_order(*table, *at)?.len();
                let index = zero_based(position, Counting::Position)
                    .filter(|zero_based| *zero_based < length)
                    .ok_or_else(|| missing_index(position, Counting::Position, length, *at))?;
                self.set_int(*dst, i64::try_from(index).unwrap_or(i64::MAX));
            }
            Op::FindValue {
                dst,
                table,
                value,
                at,
            } => {
                let value = self.value(*value);
                let index = self
                    .in_order(*table, *at)?
                    .iter()
                    .position(|item| *item == value)
                    .ok_or_else(|| never_reached(*at))?;
                self.set_int(*dst, i64::try_from(index).unwrap_or(i64::MAX));
            }
            Op::ShowItems { table, end, at } => {
                let end = usize::try_from(self.int(*end)).unwrap_or(0);
                let lines = self
                    .in_order(*table, *at)?
                    .iter()
                    .take(end)
                    .cloned()
                    .collect::<Vec<_>>();
                for item in &lines {
                    let text = shown_text(item, *at)?;
                    self.write_line(&text, *at)?;
                }
            }
            Op::CheckCount { src, at } => {
                if self.int(*src) < 0 {
                    return Err(Diagnostic::not_decided(
                        "repeating a negative number of times",
                        Some("D39"),
                        *at,
                    ));
                }
            }
            Op::NeverReached { at } => return Err(never_reached(*at)),
            Op::Jump { .. }
            | Op::JumpIfFalse { .. }
            | Op::JumpIfTrue { .. }
            | Op::JumpIfAtLeast { .. } => {
                return Err(Diagnostic::bug("a jump run as a step", Span::new(0, 0)));
            }
        }
        Ok(())
    }

    // D26: `ask` shows its prompt and waits for a line; what comes back is text.
    fn ask(&mut self, prompt: u32, at: Span) -> Result<String, Diagnostic> {
        let Value::Text(prompt_text) = self.object(prompt).clone() else {
            return Err(type_bug(at));
        };
        write!(self.output, "{prompt_text}")
            .and_then(|()| self.output.flush())
            .map_err(|error| output_failed(&error, at))?;
        let mut line = String::new();
        let read = self.input.read_line(&mut line).map_err(|error| {
            Diagnostic::mistake(format!("couldn't read what was typed: {error}"), at)
        })?;
        if read == 0 {
            return Err(Diagnostic::mistake("`ask` found nothing more to read", at));
        }
        let line = line.strip_suffix('\n').unwrap_or(&line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        Ok(line.to_owned())
    }
}

fn compared(op: Comparison, ordering: Ordering) -> bool {
    match op {
        Comparison::Is => ordering == Ordering::Equal,
        Comparison::IsNot => ordering != Ordering::Equal,
        Comparison::AtLeast => ordering != Ordering::Less,
        Comparison::AtMost => ordering != Ordering::Greater,
        Comparison::GreaterThan => ordering == Ordering::Greater,
        Comparison::LessThan => ordering == Ordering::Less,
    }
}

#[cold]
fn item_bug() -> Diagnostic {
    Diagnostic::bug("an item past the end of a table", Span::new(0, 0))
}

#[cold]
fn overflow(at: Span) -> Diagnostic {
    Diagnostic::not_decided("what happens when an int overflows", Some("D7"), at)
}

#[cold]
fn rational_error(error: RationalError, at: Span) -> Diagnostic {
    match error {
        RationalError::DivisionByZero => division_by_zero(at),
        RationalError::TooBig => Diagnostic::not_decided(
            "what happens when a rational's parts outgrow 64 bits",
            Some("D7"),
            at,
        ),
    }
}

fn division_by_zero(at: Span) -> Diagnostic {
    Diagnostic::not_decided("what dividing by zero does", Some("D16"), at)
}

fn zero_based(index: i64, counting: Counting) -> Option<usize> {
    let first = match counting {
        Counting::Position => 1,
        Counting::Offset => 0,
    };
    index
        .checked_sub(first)
        .and_then(|zero_based| usize::try_from(zero_based).ok())
}

// D32, D34: reading or changing what isn't there is an error.
fn missing_index(index: i64, counting: Counting, length: usize, at: Span) -> Diagnostic {
    let (word, first) = match counting {
        Counting::Position => ("position", 1),
        Counting::Offset => ("offset", 0),
    };
    let last = i64::try_from(length).unwrap_or(i64::MAX) - 1 + first;
    Diagnostic::mistake(format!("there's no {word} {index} in this table"), at)
        .because(format!("this table has {word}s {first} to {last}"))
}

// D7: how a rational with no exact decimal is shown is still open.
fn shown_text(value: &Value, at: Span) -> Result<String, Diagnostic> {
    value.show().ok_or_else(|| {
        Diagnostic::not_decided(
            "showing a rational with no exact decimal, like 1/3",
            Some("D7"),
            at,
        )
    })
}

fn never_reached(at: Span) -> Diagnostic {
    Diagnostic::not_decided("a stop that's never reached", Some("D38"), at)
}

fn described_key(key: &Value) -> String {
    match key {
        Value::Text(text) => format!("\"{text}\""),
        Value::Rational(rational) => rational
            .to_decimal()
            .unwrap_or_else(|| rational.to_string()),
        other => other.show().unwrap_or_default(),
    }
}

fn type_bug(at: Span) -> Diagnostic {
    Diagnostic::bug("a value had a type the checker ruled out", at)
}

fn output_failed(error: &std::io::Error, at: Span) -> Diagnostic {
    Diagnostic::mistake(format!("couldn't write the output: {error}"), at)
}
