//! The interpreter, Redstone's first tier (D3): runs a checked program directly. It is the reference the Cranelift
//! tier is tested against (rule 10).

use std::cmp::Ordering;
use std::io::{BufRead, Write};
use std::rc::Rc;

use redstone_front::ir::{
    Arithmetic, Comparison, Counting, Expr, ExprKind, Program, Selector, Stmt, Stop,
};
use redstone_front::{Diagnostic, Span};
use redstone_runtime::{Rational, RationalError, Table, Value};

/// Runs `program`, reading typed lines for `ask` from `input` and writing to `output`.
///
/// # Errors
///
/// A problem found while running, such as something still open in the design.
pub fn run(
    program: &Program,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<(), Diagnostic> {
    let mut machine = Machine {
        slots: vec![Value::Boolean(false); program.slot_count],
        input,
        output,
    };
    machine.block(&program.statements)
}

struct Machine<'io> {
    slots: Vec<Value>,
    input: &'io mut dyn BufRead,
    output: &'io mut dyn Write,
}

impl Machine<'_> {
    fn block(&mut self, statements: &[Stmt]) -> Result<(), Diagnostic> {
        statements
            .iter()
            .try_for_each(|statement| self.statement(statement))
    }

    fn statement(&mut self, statement: &Stmt) -> Result<(), Diagnostic> {
        match statement {
            Stmt::Assign { slot, value } => {
                let result = self.evaluate(value)?;
                let target = self
                    .slots
                    .get_mut(*slot)
                    .ok_or_else(|| Diagnostic::bug("a variable has no slot", value.span))?;
                *target = result;
                Ok(())
            }
            Stmt::Show {
                value,
                count,
                condition,
            } => {
                if let Some(condition) = condition
                    && !self.boolean(condition)?
                {
                    return Ok(());
                }
                // D40: the value is worked out once and shown `count` times.
                let shown = self.evaluate(value)?;
                for _ in 0..*count {
                    self.write_shown(&shown, value.span)?;
                }
                Ok(())
            }
            Stmt::ChangeItem {
                slot,
                selector,
                value,
                name_span,
            } => self.change_item(*slot, selector, value, *name_span),
            Stmt::ShowItems {
                table,
                stop,
                inclusive,
                condition,
            } => {
                if let Some(condition) = condition
                    && !self.boolean(condition)?
                {
                    return Ok(());
                }
                self.show_items(table, stop, *inclusive)
            }
            Stmt::RepeatTimes { count, body } => {
                let Value::Int(times) = self.evaluate(count)? else {
                    return Err(type_bug(count.span));
                };
                if times < 0 {
                    return Err(Diagnostic::not_decided(
                        "repeating a negative number of times",
                        Some("D39"),
                        count.span,
                    ));
                }
                for _ in 0..times {
                    self.block(body)?;
                }
                Ok(())
            }
            Stmt::RepeatUntil { test, body } => {
                while !self.boolean(test)? {
                    self.block(body)?;
                }
                Ok(())
            }
            Stmt::ForItems {
                table,
                stop,
                inclusive,
                current_item,
                body,
            } => self.for_items(table, stop, *inclusive, *current_item, body),
            Stmt::If {
                branches,
                otherwise,
            } => {
                for (condition, body) in branches {
                    if self.boolean(condition)? {
                        return self.block(body);
                    }
                }
                self.block(otherwise)
            }
        }
    }

    // D35
    fn change_item(
        &mut self,
        slot: usize,
        selector: &Selector,
        value: &Expr,
        name_span: Span,
    ) -> Result<(), Diagnostic> {
        let target = match selector {
            Selector::Index(index, counting) => {
                let Value::Int(index_value) = self.evaluate(index)? else {
                    return Err(type_bug(index.span));
                };
                Target::Index(index_value, *counting, index.span)
            }
            Selector::Key(key) => Target::Key(self.evaluate(key)?, key.span),
        };
        let new_value = self.evaluate(value)?;
        let Some(Value::Table(table)) = self.slots.get_mut(slot) else {
            return Err(type_bug(name_span));
        };
        // D37: tables are copied, not shared, so a table something else also holds is copied before it changes.
        let contents = Rc::make_mut(table);
        match (contents, target) {
            (Table::InOrder(values), Target::Index(index_value, counting, span)) => {
                let length = values.len();
                let item = index_slot(values, index_value, counting)
                    .ok_or_else(|| missing_index(index_value, counting, length, span))?;
                *item = new_value;
                Ok(())
            }
            (Table::Keyed(pairs), Target::Key(key, span)) => {
                let item = pairs
                    .iter_mut()
                    .find(|(existing, _)| *existing == key)
                    .map(|(_, value)| value)
                    .ok_or_else(|| {
                        Diagnostic::not_decided("adding a key by changing it", Some("D35"), span)
                    })?;
                *item = new_value;
                Ok(())
            }
            _ => Err(type_bug(name_span)),
        }
    }

    fn write_shown(&mut self, value: &Value, span: Span) -> Result<(), Diagnostic> {
        // D7: how a rational with no exact decimal is shown is still open.
        let text = value.show().ok_or_else(|| {
            Diagnostic::not_decided(
                "showing a rational with no exact decimal, like 1/3",
                Some("D7"),
                span,
            )
        })?;
        writeln!(self.output, "{text}").map_err(|error| output_failed(&error, span))
    }

    // D39: the table is read once, at the start; a value or test stop is checked as each item comes up.
    fn for_items(
        &mut self,
        table: &Expr,
        stop: &Stop,
        inclusive: bool,
        current_item: usize,
        body: &[Stmt],
    ) -> Result<(), Diagnostic> {
        let Value::Table(contents) = self.evaluate(table)? else {
            return Err(type_bug(table.span));
        };
        let Table::InOrder(values) = &*contents else {
            return Err(type_bug(table.span));
        };
        let count = match stop {
            Stop::LastItem => Some(values.len()),
            Stop::Position(position) => {
                let Value::Int(index) = self.evaluate(position)? else {
                    return Err(type_bug(position.span));
                };
                let stop_index = zero_based(index, Counting::Position)
                    .filter(|zero_based| *zero_based < values.len())
                    .ok_or_else(|| {
                        missing_index(index, Counting::Position, values.len(), position.span)
                    })?;
                Some(stop_index + 1)
            }
            Stop::Value(_) | Stop::Test { .. } => None,
        };
        if let Some(count) = count {
            let count = if inclusive {
                count
            } else {
                count.saturating_sub(1)
            };
            for item in values.iter().take(count) {
                self.set_slot(current_item, item.clone(), table.span)?;
                self.block(body)?;
            }
            return Ok(());
        }
        for item in values {
            self.set_slot(current_item, item.clone(), table.span)?;
            let reached = match stop {
                Stop::Value(value) => *item == self.evaluate(value)?,
                Stop::Test { test, .. } => self.boolean(test)?,
                Stop::LastItem | Stop::Position(_) => false,
            };
            if reached && !inclusive {
                return Ok(());
            }
            self.block(body)?;
            if reached {
                return Ok(());
            }
        }
        let span = match stop {
            Stop::Value(expression)
            | Stop::Test {
                test: expression, ..
            } => expression.span,
            Stop::LastItem | Stop::Position(_) => table.span,
        };
        Err(never_reached(span))
    }

    fn set_slot(&mut self, slot: usize, value: Value, span: Span) -> Result<(), Diagnostic> {
        let target = self
            .slots
            .get_mut(slot)
            .ok_or_else(|| Diagnostic::bug("a variable has no slot", span))?;
        *target = value;
        Ok(())
    }

    // D38: works out where to stop first, then shows the items before it, one per line.
    fn show_items(&mut self, table: &Expr, stop: &Stop, inclusive: bool) -> Result<(), Diagnostic> {
        let Value::Table(contents) = self.evaluate(table)? else {
            return Err(type_bug(table.span));
        };
        let Table::InOrder(values) = &*contents else {
            return Err(type_bug(table.span));
        };
        let stop_index = match stop {
            Stop::LastItem => values.len().saturating_sub(1),
            Stop::Position(position) => {
                let Value::Int(index) = self.evaluate(position)? else {
                    return Err(type_bug(position.span));
                };
                zero_based(index, Counting::Position)
                    .filter(|zero_based| *zero_based < values.len())
                    .ok_or_else(|| {
                        missing_index(index, Counting::Position, values.len(), position.span)
                    })?
            }
            Stop::Value(value) => {
                let wanted = self.evaluate(value)?;
                values
                    .iter()
                    .position(|item| *item == wanted)
                    .ok_or_else(|| never_reached(value.span))?
            }
            Stop::Test { test, current_item } => {
                let mut found = None;
                for (index, item) in values.iter().enumerate() {
                    let slot = self
                        .slots
                        .get_mut(*current_item)
                        .ok_or_else(|| Diagnostic::bug("`current_item` has no slot", test.span))?;
                    *slot = item.clone();
                    if self.boolean(test)? {
                        found = Some(index);
                        break;
                    }
                }
                found.ok_or_else(|| never_reached(test.span))?
            }
        };
        let end = if inclusive {
            stop_index + 1
        } else {
            stop_index
        };
        for item in values.iter().take(end) {
            self.write_shown(item, table.span)?;
        }
        Ok(())
    }

    fn boolean(&mut self, expression: &Expr) -> Result<bool, Diagnostic> {
        match self.evaluate(expression)? {
            Value::Boolean(value) => Ok(value),
            _ => Err(type_bug(expression.span)),
        }
    }

    fn evaluate(&mut self, expression: &Expr) -> Result<Value, Diagnostic> {
        let span = expression.span;
        match &expression.kind {
            ExprKind::Int(value) => Ok(Value::Int(*value)),
            ExprKind::Rational(value) => Ok(Value::Rational(*value)),
            ExprKind::Text(text) => Ok(Value::Text(Rc::clone(text))),
            ExprKind::Boolean(value) => Ok(Value::Boolean(*value)),
            ExprKind::Load(slot) => self
                .slots
                .get(*slot)
                .cloned()
                .ok_or_else(|| Diagnostic::bug("a variable has no slot", span)),
            ExprKind::IntArithmetic(arithmetic, left, right) => {
                let (left, right) = (self.evaluate(left)?, self.evaluate(right)?);
                match (left, right) {
                    (Value::Int(left), Value::Int(right)) => {
                        int_arithmetic(*arithmetic, left, right, span).map(Value::Int)
                    }
                    _ => Err(type_bug(span)),
                }
            }
            // D7: an int divided by an int is a rational.
            ExprKind::IntDivide(left, right) => match (self.evaluate(left)?, self.evaluate(right)?)
            {
                (Value::Int(left), Value::Int(right)) => Rational::new(left.into(), right.into())
                    .map(Value::Rational)
                    .map_err(|error| rational_error(error, span)),
                _ => Err(type_bug(span)),
            },
            ExprKind::IntToRational(int) => match self.evaluate(int)? {
                Value::Int(int) => Ok(Value::Rational(Rational::from_int(int))),
                _ => Err(type_bug(span)),
            },
            ExprKind::RationalArithmetic(arithmetic, left, right) => {
                match (self.evaluate(left)?, self.evaluate(right)?) {
                    (Value::Rational(left), Value::Rational(right)) => {
                        rational_arithmetic(*arithmetic, left, right, span).map(Value::Rational)
                    }
                    _ => Err(type_bug(span)),
                }
            }
            ExprKind::Compare(comparison, left, right) => {
                let (left, right) = (self.evaluate(left)?, self.evaluate(right)?);
                compare(*comparison, &left, &right, span).map(Value::Boolean)
            }
            ExprKind::And(left, right) => {
                Ok(Value::Boolean(self.boolean(left)? && self.boolean(right)?))
            }
            ExprKind::Or(left, right) => {
                Ok(Value::Boolean(self.boolean(left)? || self.boolean(right)?))
            }
            ExprKind::Not(operand) => Ok(Value::Boolean(!self.boolean(operand)?)),
            ExprKind::Ask(prompt) => self.ask(prompt),
            // D32, D33
            ExprKind::InOrder(items) => {
                let values = items
                    .iter()
                    .map(|item| self.evaluate(item))
                    .collect::<Result<_, _>>()?;
                Ok(Value::Table(Rc::new(Table::InOrder(values))))
            }
            ExprKind::Keyed(entries) => self.keyed(entries),
            ExprKind::ReadPosition {
                table,
                index,
                counting,
            } => self.read_position(table, index, *counting, span),
            ExprKind::ReadKey { table, key } => self.read_key(table, key, span),
        }
    }

    fn keyed(&mut self, entries: &[(Expr, Expr)]) -> Result<Value, Diagnostic> {
        let mut pairs: Vec<(Value, Value)> = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let key_value = self.evaluate(key)?;
            if pairs.iter().any(|(existing, _)| *existing == key_value) {
                return Err(Diagnostic::not_decided(
                    "the same key twice in one table",
                    Some("D32"),
                    key.span,
                ));
            }
            let value = self.evaluate(value)?;
            pairs.push((key_value, value));
        }
        Ok(Value::Table(Rc::new(Table::Keyed(pairs))))
    }

    // D32, D34: reading what isn't there is an error.
    fn read_position(
        &mut self,
        table: &Expr,
        index: &Expr,
        counting: Counting,
        span: Span,
    ) -> Result<Value, Diagnostic> {
        let (Value::Table(contents), Value::Int(index_value)) =
            (self.evaluate(table)?, self.evaluate(index)?)
        else {
            return Err(type_bug(span));
        };
        let Table::InOrder(values) = &*contents else {
            return Err(type_bug(span));
        };
        zero_based(index_value, counting)
            .and_then(|zero_based| values.get(zero_based))
            .cloned()
            .ok_or_else(|| missing_index(index_value, counting, values.len(), index.span))
    }

    fn read_key(&mut self, table: &Expr, key: &Expr, span: Span) -> Result<Value, Diagnostic> {
        let Value::Table(contents) = self.evaluate(table)? else {
            return Err(type_bug(span));
        };
        let Table::Keyed(pairs) = &*contents else {
            return Err(type_bug(span));
        };
        let key_value = self.evaluate(key)?;
        pairs
            .iter()
            .find(|(existing, _)| *existing == key_value)
            .map(|(_, value)| value.clone())
            .ok_or_else(|| {
                Diagnostic::mistake(
                    format!("there's no key {} in this table", described_key(&key_value)),
                    key.span,
                )
            })
    }

    // D26: `ask` shows its prompt and waits for a line; what comes back is text.
    fn ask(&mut self, prompt: &Expr) -> Result<Value, Diagnostic> {
        let Value::Text(prompt_text) = self.evaluate(prompt)? else {
            return Err(type_bug(prompt.span));
        };
        write!(self.output, "{prompt_text}")
            .and_then(|()| self.output.flush())
            .map_err(|error| output_failed(&error, prompt.span))?;
        let mut line = String::new();
        let read = self.input.read_line(&mut line).map_err(|error| {
            Diagnostic::mistake(
                format!("couldn't read what was typed: {error}"),
                prompt.span,
            )
        })?;
        if read == 0 {
            return Err(Diagnostic::mistake(
                "`ask` found nothing more to read",
                prompt.span,
            ));
        }
        let line = line.strip_suffix('\n').unwrap_or(&line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        Ok(Value::Text(Rc::from(line)))
    }
}

// D7, D16
fn int_arithmetic(
    arithmetic: Arithmetic,
    left: i64,
    right: i64,
    span: Span,
) -> Result<i64, Diagnostic> {
    let result = match arithmetic {
        Arithmetic::Plus => left.checked_add(right),
        Arithmetic::Minus => left.checked_sub(right),
        Arithmetic::Times => left.checked_mul(right),
        Arithmetic::Remainder => {
            if right == 0 {
                return Err(division_by_zero(span));
            }
            if left < 0 || right < 0 {
                return Err(Diagnostic::not_decided(
                    "`remainder` with a negative number",
                    Some("D16"),
                    span,
                ));
            }
            left.checked_rem(right)
        }
        Arithmetic::DividedBy => return Err(type_bug(span)),
    };
    result.ok_or_else(|| {
        Diagnostic::not_decided("what happens when an int overflows", Some("D7"), span)
    })
}

fn rational_arithmetic(
    arithmetic: Arithmetic,
    left: Rational,
    right: Rational,
    span: Span,
) -> Result<Rational, Diagnostic> {
    let result = match arithmetic {
        Arithmetic::Plus => left.checked_add(right),
        Arithmetic::Minus => left.checked_sub(right),
        Arithmetic::Times => left.checked_mul(right),
        Arithmetic::DividedBy => left.checked_div(right),
        Arithmetic::Remainder => return Err(type_bug(span)),
    };
    result.map_err(|error| rational_error(error, span))
}

fn compare(
    comparison: Comparison,
    left: &Value,
    right: &Value,
    span: Span,
) -> Result<bool, Diagnostic> {
    let ordering = match (left, right) {
        (Value::Int(left), Value::Int(right)) => Some(left.cmp(right)),
        (Value::Rational(left), Value::Rational(right)) => Some(left.cmp(right)),
        _ => None,
    };
    match (comparison, ordering) {
        (Comparison::Is, _) => Ok(left == right),
        (Comparison::IsNot, _) => Ok(left != right),
        (Comparison::AtLeast, Some(ordering)) => Ok(ordering != Ordering::Less),
        (Comparison::AtMost, Some(ordering)) => Ok(ordering != Ordering::Greater),
        (Comparison::GreaterThan, Some(ordering)) => Ok(ordering == Ordering::Greater),
        (Comparison::LessThan, Some(ordering)) => Ok(ordering == Ordering::Less),
        (_, None) => Err(type_bug(span)),
    }
}

fn rational_error(error: RationalError, span: Span) -> Diagnostic {
    match error {
        RationalError::DivisionByZero => division_by_zero(span),
        RationalError::TooBig => Diagnostic::not_decided(
            "what happens when a rational's parts outgrow 64 bits",
            Some("D7"),
            span,
        ),
    }
}

fn division_by_zero(span: Span) -> Diagnostic {
    Diagnostic::not_decided("what dividing by zero does", Some("D16"), span)
}

/// An item to change, its selector already worked out.
enum Target {
    Index(i64, Counting, Span),
    Key(Value, Span),
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

fn index_slot(values: &mut [Value], index: i64, counting: Counting) -> Option<&mut Value> {
    zero_based(index, counting).and_then(|zero_based| values.get_mut(zero_based))
}

// D32, D34: reading or changing what isn't there is an error.
fn missing_index(index: i64, counting: Counting, length: usize, span: Span) -> Diagnostic {
    let (word, first) = match counting {
        Counting::Position => ("position", 1),
        Counting::Offset => ("offset", 0),
    };
    let last = i64::try_from(length).unwrap_or(i64::MAX) - 1 + first;
    Diagnostic::mistake(format!("there's no {word} {index} in this table"), span)
        .because(format!("this table has {word}s {first} to {last}"))
}

fn never_reached(span: Span) -> Diagnostic {
    Diagnostic::not_decided("a stop that's never reached", Some("D38"), span)
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

fn type_bug(span: Span) -> Diagnostic {
    Diagnostic::bug("a value had a type the checker ruled out", span)
}

fn output_failed(error: &std::io::Error, span: Span) -> Diagnostic {
    Diagnostic::mistake(format!("couldn't write the output: {error}"), span)
}
