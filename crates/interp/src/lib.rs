//! The interpreter, Redstone's first tier (D3): runs a checked program directly. It is the reference the Cranelift
//! and LLVM tiers are tested against (rule 10).

use std::cmp::Ordering;
use std::io::{BufRead, Write};
use std::rc::Rc;

use redstone_front::ir::{Arithmetic, Comparison, Expr, ExprKind, Program, Stmt};
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
            Stmt::Show { value, condition } => {
                if let Some(condition) = condition
                    && !self.boolean(condition)?
                {
                    return Ok(());
                }
                self.show(value)
            }
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

    fn show(&mut self, value: &Expr) -> Result<(), Diagnostic> {
        // D7: how a rational with no exact decimal is shown is still open.
        let text = self.evaluate(value)?.show().ok_or_else(|| {
            Diagnostic::not_decided(
                "showing a rational with no exact decimal, like 1/3",
                Some("D7"),
                value.span,
            )
        })?;
        writeln!(self.output, "{text}").map_err(|error| output_failed(&error, value.span))
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

fn type_bug(span: Span) -> Diagnostic {
    Diagnostic::bug("a value had a type the checker ruled out", span)
}

fn output_failed(error: &std::io::Error, span: Span) -> Diagnostic {
    Diagnostic::mistake(format!("couldn't write the output: {error}"), span)
}
