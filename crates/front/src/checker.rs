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

#[derive(Clone, Copy)]
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
                // D28
                if *whether && ty != Type::Boolean {
                    return Err(Diagnostic::error(
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
                Diagnostic::error(format!("`{name}` already exists"), name_span)
                    .with_hint(format!("to change it, write `{name} is_now …` (D15)")),
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
        let variable = self.variable(name, name_span)?;
        let value_span = value.span;
        let (value, ty) = self.expression(value)?;
        if ty != variable.ty {
            return Err(Diagnostic::error(
                format!(
                    "`{name}` is {}, so it can't become {}",
                    variable.ty.described(),
                    ty.described()
                ),
                value_span,
            )
            .with_hint("a variable keeps the type of the value it was made with (D4)"));
        }
        Ok(ir::Stmt::Assign {
            slot: variable.slot,
            value,
        })
    }

    fn condition(&mut self, condition: &ast::Expr) -> Result<ir::Expr, Diagnostic> {
        let (checked, ty) = self.expression(condition)?;
        if ty != Type::Boolean {
            return Err(Diagnostic::error(
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
            .find_map(|scope| scope.get(name).copied())
    }

    fn variable(&self, name: &str, span: Span) -> Result<Variable, Diagnostic> {
        if let Some(variable) = self.lookup(name) {
            return Ok(variable);
        }
        if self.ended.contains(name) {
            return Err(made_in_ended_block(name, span));
        }
        let diagnostic = Diagnostic::error(format!("there's no `{name}`"), span);
        Err(match self.closest_name(name) {
            Some(closest) => diagnostic.with_hint(format!("did you mean `{closest}`?")),
            None => diagnostic.with_hint(format!("to make it, write `{name} = …` (D12)")),
        })
    }

    fn closest_name(&self, name: &str) -> Option<String> {
        self.scopes
            .iter()
            .flat_map(HashMap::keys)
            .map(|candidate| (edit_distance(name, candidate), candidate))
            .filter(|(distance, candidate)| *distance <= 2 && *distance < candidate.len())
            .min()
            .map(|(_, candidate)| candidate.clone())
    }

    fn expression(&mut self, expression: &ast::Expr) -> Result<(ir::Expr, Type), Diagnostic> {
        let span = expression.span;
        let (kind, ty) = match &expression.kind {
            ExprKind::Int(value) => (ir::ExprKind::Int(*value), Type::Int),
            ExprKind::Rational(value) => (ir::ExprKind::Rational(*value), Type::Rational),
            ExprKind::Text(text) => (ir::ExprKind::Text(Rc::from(text.as_str())), Type::Text),
            ExprKind::Boolean(value) => (ir::ExprKind::Boolean(*value), Type::Boolean),
            ExprKind::Name(name) => {
                let variable = self.variable(name, span)?;
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
                    return Err(Diagnostic::error(
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
        };
        Ok((ir::Expr { kind, span }, ty))
    }

    fn boolean_operand(&mut self, word: &str, operand: &ast::Expr) -> Result<ir::Expr, Diagnostic> {
        let (checked, ty) = self.expression(operand)?;
        if ty != Type::Boolean {
            return Err(Diagnostic::error(
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
                Ok((
                    ir::ExprKind::Compare(
                        comparison,
                        Box::new(checked_left),
                        Box::new(checked_right),
                    ),
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
        match (self.left.1, self.right.1) {
            (Type::Int, Type::Int) if arithmetic == Arithmetic::DividedBy => {
                Ok((ir::ExprKind::IntDivide(left, right), Type::Rational))
            }
            (Type::Int, Type::Int) => Ok((
                ir::ExprKind::IntArithmetic(arithmetic, left, right),
                Type::Int,
            )),
            (Type::Rational, Type::Rational) if arithmetic == Arithmetic::Remainder => Err(
                Diagnostic::not_decided("`remainder` on rationals", "D16", self.op_span),
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
                    "D16",
                    self.op_span,
                ))
            }
            // D7: an int beside a rational counts as a rational, as in `1 / 3 * 3` being `1`.
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
                Diagnostic::not_decided("joining text with `plus`", "D26", self.op_span),
            ),
            _ => Err(self.needs_numbers()),
        }
    }

    // D18, D20
    fn comparison(&self, comparison: Comparison) -> Result<Type, Diagnostic> {
        let ordering = !matches!(comparison, Comparison::Is | Comparison::IsNot);
        match (self.left.1, self.right.1) {
            (Type::Int, Type::Int) | (Type::Rational, Type::Rational) => Ok(Type::Boolean),
            (Type::Int, Type::Rational) | (Type::Rational, Type::Int) => Err(self.mixed_numbers()),
            (Type::Text, Type::Text) if ordering => Err(Diagnostic::not_decided(
                format!("comparing text with `{}`", self.op.word()),
                "D18",
                self.op_span,
            )),
            (left, right) if left == right && !ordering => Ok(Type::Boolean),
            _ if ordering => Err(self.needs_numbers()),
            (left, right) => Err(Diagnostic::error(
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

    fn mixed_numbers(&self) -> Diagnostic {
        Diagnostic::not_decided(
            format!(
                "`{}` with an int on one side and a rational on the other",
                self.op.word()
            ),
            "D7",
            self.span,
        )
        .with_hint("write the int as a decimal, like `1.0`, to make both rationals")
    }

    fn needs_numbers(&self) -> Diagnostic {
        let (side_span, side_type) = if matches!(self.left.1, Type::Int | Type::Rational) {
            self.right
        } else {
            self.left
        };
        Diagnostic::error(
            format!(
                "`{}` needs numbers, but this is {}",
                self.op.word(),
                side_type.described()
            ),
            side_span,
        )
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
        "D22",
        span,
    )
    .with_hint(format!(
        "make `{name}` before the `if` to use it after `end_of if`"
    ))
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

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, a_character) in a.chars().enumerate() {
        let mut current = vec![i + 1];
        for (j, b_character) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(a_character != *b_character);
            current.push(substitution.min(previous[j + 1] + 1).min(current[j] + 1));
        }
        previous = current;
    }
    previous[b.len()]
}
