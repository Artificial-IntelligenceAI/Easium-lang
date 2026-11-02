//! Checks types before the program runs (D4) and gives every variable a slot. A variable takes the type of the
//! value it is made with, and keeps it.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{self, BinaryOp, ExprKind};
use crate::diagnostic::{Diagnostic, Span};
use crate::ir::{self, Arithmetic, Comparison, Type};

/// Checks a parsed program.
///
/// # Errors
///
/// The first type mistake, misspelt name, or use of something still open.
pub fn check(program: &ast::Program) -> Result<ir::Program, Diagnostic> {
    let mut checker = Checker {
        scopes: vec![HashMap::new()],
        ended: HashSet::new(),
        slot_count: 0,
    };
    let statements = checker.block(&program.statements)?;
    Ok(ir::Program {
        statements,
        slot_count: checker.slot_count,
    })
}

#[derive(Clone)]
struct Variable {
    slot: usize,
    ty: Type,
}

struct Checker {
    scopes: Vec<HashMap<String, Variable>>,
    /// Names made inside blocks that have ended: whether they still exist is open.
    ended: HashSet<String>,
    slot_count: usize,
}

impl Checker {
    fn block(&mut self, statements: &[ast::Stmt]) -> Result<Vec<ir::Stmt>, Diagnostic> {
        statements
            .iter()
            .map(|statement| self.statement(statement))
            .collect()
    }

    fn nested_block(&mut self, statements: &[ast::Stmt]) -> Result<Vec<ir::Stmt>, Diagnostic> {
        self.scopes.push(HashMap::new());
        let result = self.block(statements);
        if let Some(scope) = self.scopes.pop() {
            self.ended.extend(scope.into_keys());
        }
        result
    }

    fn statement(&mut self, statement: &ast::Stmt) -> Result<ir::Stmt, Diagnostic> {
        match statement {
            ast::Stmt::Create {
                name,
                name_span,
                value,
            } => self.create(name, *name_span, value),
            ast::Stmt::Change {
                name,
                name_span,
                value,
            } => self.change(name, *name_span, value),
            ast::Stmt::Show {
                value,
                whether,
                condition,
            } => {
                let value_span = value.span;
                let (value, ty) = self.expression(value)?;
                // D32: how a table is shown is still open.
                if ty.is_table() {
                    return Err(Diagnostic::not_decided(
                        "showing a table",
                        Some("D32"),
                        value_span,
                    ));
                }
                // D28
                if *whether && ty != Type::Boolean {
                    return Err(Diagnostic::mistake(
                        format!(
                            "`whether` needs a comparison, but this is {}",
                            ty.described()
                        ),
                        value_span,
                    ));
                }
                let condition = condition
                    .as_ref()
                    .map(|condition| self.condition(condition))
                    .transpose()?;
                Ok(ir::Stmt::Show { value, condition })
            }
            ast::Stmt::If {
                branches,
                otherwise,
            } => {
                let branches = branches
                    .iter()
                    .map(|branch| {
                        Ok((
                            self.condition(&branch.condition)?,
                            self.nested_block(&branch.body)?,
                        ))
                    })
                    .collect::<Result<_, Diagnostic>>()?;
                let otherwise = match otherwise {
                    Some(statements) => self.nested_block(statements)?,
                    None => Vec::new(),
                };
                Ok(ir::Stmt::If {
                    branches,
                    otherwise,
                })
            }
        }
    }

    // D12, D15
    fn create(
        &mut self,
        name: &str,
        name_span: Span,
        value: &ast::Expr,
    ) -> Result<ir::Stmt, Diagnostic> {
        let (value, ty) = self.expression(value)?;
        if self.lookup(name).is_some() {
            return Err(
                Diagnostic::mistake(format!("`{name}` already exists"), name_span)
                    .because("`=` makes a new variable; `is_now` changes one that exists"),
            );
        }
        if self.ended.contains(name) {
            return Err(made_in_ended_block(name, name_span));
        }
        let slot = self.slot_count;
        self.slot_count += 1;
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_owned(), Variable { slot, ty });
        }
        Ok(ir::Stmt::Assign { slot, value })
    }

    // D4, D14, D15
    fn change(
        &mut self,
        name: &str,
        name_span: Span,
        value: &ast::Expr,
    ) -> Result<ir::Stmt, Diagnostic> {
        let variable = self.variable(
            name,
            name_span,
            "a variable has to be made with `=` before `is_now` can change it",
        )?;
        let value_span = value.span;
        let (value, ty) = self.expression(value)?;
        if ty != variable.ty {
            return Err(Diagnostic::mistake(
                format!(
                    "`{name}` is {}, so it can't become {}",
                    variable.ty.described(),
                    ty.described()
                ),
                value_span,
            )
            .because("a variable keeps the type of the value it was made with"));
        }
        Ok(ir::Stmt::Assign {
            slot: variable.slot,
            value,
        })
    }

    fn condition(&mut self, condition: &ast::Expr) -> Result<ir::Expr, Diagnostic> {
        let (checked, ty) = self.expression(condition)?;
        if ty != Type::Boolean {
            return Err(Diagnostic::mistake(
                format!(
                    "an `if` needs true or false, but this is {}",
                    ty.described()
                ),
                condition.span,
            ));
        }
        Ok(checked)
    }

    fn lookup(&self, name: &str) -> Option<Variable> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).cloned())
    }

    fn variable(&self, name: &str, span: Span, why: &str) -> Result<Variable, Diagnostic> {
        if let Some(variable) = self.lookup(name) {
            return Ok(variable);
        }
        if self.ended.contains(name) {
            return Err(made_in_ended_block(name, span));
        }
        Err(Diagnostic::mistake(format!("there's no `{name}`"), span).because(why))
    }

    fn expression(&mut self, expression: &ast::Expr) -> Result<(ir::Expr, Type), Diagnostic> {
        let span = expression.span;
        let (kind, ty) = match &expression.kind {
            ExprKind::Int(value) => (ir::ExprKind::Int(*value), Type::Int),
            ExprKind::Rational(value) => (ir::ExprKind::Rational(*value), Type::Rational),
            ExprKind::Text(text) => (ir::ExprKind::Text(Rc::from(text.as_str())), Type::Text),
            ExprKind::Boolean(value) => (ir::ExprKind::Boolean(*value), Type::Boolean),
            ExprKind::Name(name) => {
                let variable = self.variable(
                    name,
                    span,
                    "a variable has to be made with `=` before it's used",
                )?;
                (ir::ExprKind::Load(variable.slot), variable.ty)
            }
            ExprKind::Not(operand) => {
                let operand = self.boolean_operand("not", operand)?;
                (ir::ExprKind::Not(Box::new(operand)), Type::Boolean)
            }
            // D26: `ask` shows text and gives back text.
            ExprKind::Ask(prompt) => {
                let (checked, ty) = self.expression(prompt)?;
                if ty != Type::Text {
                    return Err(Diagnostic::mistake(
                        format!("`ask` needs text to show, but this is {}", ty.described()),
                        prompt.span,
                    ));
                }
                (ir::ExprKind::Ask(Box::new(checked)), Type::Text)
            }
            ExprKind::Binary {
                op,
                op_span,
                left,
                right,
            } => self.binary(*op, *op_span, left, right, span)?,
            ExprKind::InOrder(items) => self.in_order(items)?,
            ExprKind::Keyed(entries) => self.keyed(entries)?,
        };
        Ok((ir::Expr { kind, span }, ty))
    }

    // D32, D33: every value in a table has the type of the first one.
    fn in_order(&mut self, items: &[ast::Expr]) -> Result<(ir::ExprKind, Type), Diagnostic> {
        let mut checked = Vec::with_capacity(items.len());
        let mut value_type: Option<Type> = None;
        for item in items {
            let (expression, ty) = self.expression(item)?;
            same_type_in_table(value_type.as_ref(), &ty, "value", item.span)?;
            value_type.get_or_insert(ty);
            checked.push(expression);
        }
        let value_type =
            value_type.ok_or_else(|| Diagnostic::bug("a table with no values", Span::new(0, 0)))?;
        Ok((
            ir::ExprKind::InOrder(checked),
            Type::InOrder(Rc::new(value_type)),
        ))
    }

    fn keyed(
        &mut self,
        entries: &[(ast::Expr, ast::Expr)],
    ) -> Result<(ir::ExprKind, Type), Diagnostic> {
        let mut checked = Vec::with_capacity(entries.len());
        let mut key_type: Option<Type> = None;
        let mut value_type: Option<Type> = None;
        for (key, value) in entries {
            let (checked_key, this_key_type) = self.expression(key)?;
            if this_key_type.is_table() {
                return Err(Diagnostic::not_decided(
                    "a table as a key",
                    Some("D32"),
                    key.span,
                ));
            }
            same_type_in_table(key_type.as_ref(), &this_key_type, "key", key.span)?;
            key_type.get_or_insert(this_key_type);
            let (checked_value, this_value_type) = self.expression(value)?;
            same_type_in_table(value_type.as_ref(), &this_value_type, "value", value.span)?;
            value_type.get_or_insert(this_value_type);
            checked.push((checked_key, checked_value));
        }
        match (key_type, value_type) {
            (Some(key_type), Some(value_type)) => Ok((
                ir::ExprKind::Keyed(checked),
                Type::Keyed(Rc::new(key_type), Rc::new(value_type)),
            )),
            _ => Err(Diagnostic::bug("a table with no entries", Span::new(0, 0))),
        }
    }

    fn boolean_operand(&mut self, word: &str, operand: &ast::Expr) -> Result<ir::Expr, Diagnostic> {
        let (checked, ty) = self.expression(operand)?;
        if ty != Type::Boolean {
            return Err(Diagnostic::mistake(
                format!(
                    "`{word}` needs true or false, but this is {}",
                    ty.described()
                ),
                operand.span,
            ));
        }
        Ok(checked)
    }

    fn binary(
        &mut self,
        op: BinaryOp,
        op_span: Span,
        left: &ast::Expr,
        right: &ast::Expr,
        span: Span,
    ) -> Result<(ir::ExprKind, Type), Diagnostic> {
        // D21
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            let checked_left = Box::new(self.boolean_operand(op.word(), left)?);
            let checked_right = Box::new(self.boolean_operand(op.word(), right)?);
            let kind = if op == BinaryOp::And {
                ir::ExprKind::And(checked_left, checked_right)
            } else {
                ir::ExprKind::Or(checked_left, checked_right)
            };
            return Ok((kind, Type::Boolean));
        }
        let (checked_left, left_type) = self.expression(left)?;
        let (checked_right, right_type) = self.expression(right)?;
        let operands = Operands {
            op,
            op_span,
            span,
            left: (left.span, left_type),
            right: (right.span, right_type),
        };
        match (arithmetic(op), comparison(op)) {
            (Some(arithmetic), _) => {
                operands.arithmetic(arithmetic, Box::new(checked_left), Box::new(checked_right))
            }
            (None, Some(comparison)) => {
                let ty = operands.comparison(comparison)?;
                // D29
                let (left, right) = match (&operands.left.1, &operands.right.1) {
                    (Type::Int, Type::Rational) => {
                        (to_rational(Box::new(checked_left)), checked_right)
                    }
                    (Type::Rational, Type::Int) => {
                        (checked_left, to_rational(Box::new(checked_right)))
                    }
                    _ => (checked_left, checked_right),
                };
                Ok((
                    ir::ExprKind::Compare(comparison, Box::new(left), Box::new(right)),
                    ty,
                ))
            }
            (None, None) => Err(Diagnostic::bug(
                "a word that is neither arithmetic nor a comparison",
                op_span,
            )),
        }
    }
}

/// The two sides of an arithmetic word or comparison, for working out its type.
struct Operands {
    op: BinaryOp,
    op_span: Span,
    span: Span,
    left: (Span, Type),
    right: (Span, Type),
}

impl Operands {
    // D7, D16
    fn arithmetic(
        &self,
        arithmetic: Arithmetic,
        left: Box<ir::Expr>,
        right: Box<ir::Expr>,
    ) -> Result<(ir::ExprKind, Type), Diagnostic> {
        match (&self.left.1, &self.right.1) {
            (Type::Int, Type::Int) if arithmetic == Arithmetic::DividedBy => {
                Ok((ir::ExprKind::IntDivide(left, right), Type::Rational))
            }
            (Type::Int, Type::Int) => Ok((
                ir::ExprKind::IntArithmetic(arithmetic, left, right),
                Type::Int,
            )),
            (Type::Rational, Type::Rational) if arithmetic == Arithmetic::Remainder => Err(
                Diagnostic::not_decided("`remainder` on rationals", Some("D16"), self.op_span),
            ),
            (Type::Rational, Type::Rational) => Ok((
                ir::ExprKind::RationalArithmetic(arithmetic, left, right),
                Type::Rational,
            )),
            (Type::Int, Type::Rational) | (Type::Rational, Type::Int)
                if arithmetic == Arithmetic::Remainder =>
            {
                Err(Diagnostic::not_decided(
                    "`remainder` on rationals",
                    Some("D16"),
                    self.op_span,
                ))
            }
            // D7, D29: an int beside a rational counts as a rational, as in `1 / 3 * 3` being `1`.
            (Type::Int, Type::Rational) => Ok((
                ir::ExprKind::RationalArithmetic(arithmetic, Box::new(to_rational(left)), right),
                Type::Rational,
            )),
            (Type::Rational, Type::Int) => Ok((
                ir::ExprKind::RationalArithmetic(arithmetic, left, Box::new(to_rational(right))),
                Type::Rational,
            )),
            // D26
            (Type::Text, Type::Text) if arithmetic == Arithmetic::Plus => Err(
                Diagnostic::not_decided("joining text with `plus`", Some("D26"), self.op_span),
            ),
            _ => Err(self.needs_numbers()),
        }
    }

    // D18, D20
    fn comparison(&self, comparison: Comparison) -> Result<Type, Diagnostic> {
        let ordering = !matches!(comparison, Comparison::Is | Comparison::IsNot);
        match (&self.left.1, &self.right.1) {
            (Type::Int | Type::Rational, Type::Int | Type::Rational) => Ok(Type::Boolean),
            (left, right) if left.is_table() || right.is_table() => Err(Diagnostic::not_decided(
                "comparing tables",
                Some("D32"),
                self.span,
            )),
            (Type::Text, Type::Text) if ordering => Err(Diagnostic::not_decided(
                format!("comparing text with `{}`", self.op.word()),
                Some("D18"),
                self.op_span,
            )),
            (left, right) if left == right && !ordering => Ok(Type::Boolean),
            _ if ordering => Err(self.needs_numbers()),
            (left, right) => Err(Diagnostic::mistake(
                format!(
                    "`{}` compares two things of the same type, but this is {} and {}",
                    self.op.word(),
                    left.described(),
                    right.described()
                ),
                self.span,
            )),
        }
    }

    fn needs_numbers(&self) -> Diagnostic {
        let (side_span, side_type) = if matches!(self.left.1, Type::Int | Type::Rational) {
            &self.right
        } else {
            &self.left
        };
        Diagnostic::mistake(
            format!(
                "`{}` needs numbers, but this is {}",
                self.op.word(),
                side_type.described()
            ),
            *side_span,
        )
    }
}

fn same_type_in_table(
    expected: Option<&Type>,
    ty: &Type,
    part: &str,
    span: Span,
) -> Result<(), Diagnostic> {
    match expected {
        Some(expected) if expected != ty => {
            if matches!(
                (expected, ty),
                (Type::Int, Type::Rational) | (Type::Rational, Type::Int)
            ) {
                return Err(Diagnostic::not_decided(
                    "a table mixing ints and rationals",
                    Some("D32"),
                    span,
                ));
            }
            Err(Diagnostic::mistake(
                format!(
                    "this table's {part}s are {}, but this is {}",
                    expected.plural(),
                    ty.described()
                ),
                span,
            )
            .because(format!(
                "every {part} in a table has the type of the first one"
            )))
        }
        _ => Ok(()),
    }
}

fn to_rational(int: Box<ir::Expr>) -> ir::Expr {
    let span = int.span;
    ir::Expr {
        kind: ir::ExprKind::IntToRational(int),
        span,
    }
}

fn made_in_ended_block(name: &str, span: Span) -> Diagnostic {
    Diagnostic::not_decided(
        format!("whether `{name}`, made inside an `if` block, exists after the block ends"),
        Some("D22"),
        span,
    )
}

fn arithmetic(op: BinaryOp) -> Option<Arithmetic> {
    match op {
        BinaryOp::Plus => Some(Arithmetic::Plus),
        BinaryOp::Minus => Some(Arithmetic::Minus),
        BinaryOp::Times => Some(Arithmetic::Times),
        BinaryOp::DividedBy => Some(Arithmetic::DividedBy),
        BinaryOp::Remainder => Some(Arithmetic::Remainder),
        _ => None,
    }
}

fn comparison(op: BinaryOp) -> Option<Comparison> {
    match op {
        BinaryOp::Is => Some(Comparison::Is),
        BinaryOp::IsNot => Some(Comparison::IsNot),
        BinaryOp::AtLeast => Some(Comparison::AtLeast),
        BinaryOp::AtMost => Some(Comparison::AtMost),
        BinaryOp::GreaterThan => Some(Comparison::GreaterThan),
        BinaryOp::LessThan => Some(Comparison::LessThan),
        _ => None,
    }
}
