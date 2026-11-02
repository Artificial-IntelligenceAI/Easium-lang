//! Compiles a checked program to bytecode. Every value is worked out in the same order as in the reference
//! interpreter, so the output and the errors match it exactly (rule 10).

use easium_front::ir::{
    Arithmetic, Comparison, Expr, ExprKind, FormatPart, Program, Selector, Stmt, Stop, Type,
};
use easium_front::{Diagnostic, Span};
use easium_runtime::Value;

use crate::op::{Op, Operand, Piece, Reg};

/// A compiled program: its instructions and how big each register file is.
pub struct Code {
    /// The instructions.
    pub ops: Vec<Op>,
    /// Registers in the int file, which also holds booleans.
    pub ints: usize,
    /// Registers in the rational file.
    pub rationals: usize,
    /// Registers in the object file: text and tables.
    pub objects: usize,
}

/// Compiles a checked program.
///
/// # Errors
///
/// Only Easium's own bugs: a program the checker passed always compiles.
pub(crate) fn compile(program: &Program) -> Result<Code, Diagnostic> {
    let mut compiler = Compiler {
        ops: Vec::new(),
        slots: Vec::with_capacity(program.slot_types.len()),
        next: Files::default(),
        most: Files::default(),
        loop_exits: Vec::new(),
    };
    for ty in &program.slot_types {
        let reg = compiler.temp(ty);
        compiler.slots.push(reg);
    }
    compiler.block(&program.statements)?;
    Ok(Code {
        ops: compiler.ops,
        ints: compiler.most.ints as usize,
        rationals: compiler.most.rationals as usize,
        objects: compiler.most.objects as usize,
    })
}

/// How many registers of each file are in use.
#[derive(Clone, Copy, Default)]
struct Files {
    ints: u32,
    rationals: u32,
    objects: u32,
}

struct Compiler {
    ops: Vec<Op>,
    /// Each variable slot's register.
    slots: Vec<Reg>,
    /// The next free register in each file; statements give back what they used.
    next: Files,
    /// The most registers each file ever needed.
    most: Files,
    /// For each loop being compiled, the jumps that leave it, to point at its end once known (D41).
    loop_exits: Vec<Vec<usize>>,
}

impl Compiler {
    fn emit(&mut self, op: Op) -> usize {
        self.ops.push(op);
        self.ops.len() - 1
    }

    fn here(&self) -> usize {
        self.ops.len()
    }

    /// Points a jump emitted earlier at `to`.
    fn patch(&mut self, jump: usize, target: usize) {
        if let Some(
            Op::Jump { to }
            | Op::JumpIfFalse { to, .. }
            | Op::JumpIfTrue { to, .. }
            | Op::JumpIfAtLeast { to, .. }
            | Op::JumpIfCompare { to, .. },
        ) = self.ops.get_mut(jump)
        {
            *to = target;
        }
    }

    fn temp_int(&mut self) -> u32 {
        let reg = self.next.ints;
        self.next.ints += 1;
        self.most.ints = self.most.ints.max(self.next.ints);
        reg
    }

    fn temp_rational(&mut self) -> u32 {
        let reg = self.next.rationals;
        self.next.rationals += 1;
        self.most.rationals = self.most.rationals.max(self.next.rationals);
        reg
    }

    fn temp_object(&mut self) -> u32 {
        let reg = self.next.objects;
        self.next.objects += 1;
        self.most.objects = self.most.objects.max(self.next.objects);
        reg
    }

    fn temp(&mut self, ty: &Type) -> Reg {
        match ty {
            Type::Int => Reg::Int(self.temp_int()),
            Type::Boolean => Reg::Boolean(self.temp_int()),
            Type::Rational => Reg::Rational(self.temp_rational()),
            Type::Text | Type::InOrder(_) | Type::Keyed(..) => Reg::Object(self.temp_object()),
        }
    }

    fn slot(&self, slot: usize, at: Span) -> Result<Reg, Diagnostic> {
        self.slots
            .get(slot)
            .copied()
            .ok_or_else(|| Diagnostic::bug("a variable has no slot", at))
    }

    fn move_into(&mut self, dst: Reg, src: Reg) {
        if dst == src {
            return;
        }
        let op = match (dst, src) {
            (Reg::Int(dst) | Reg::Boolean(dst), Reg::Int(src) | Reg::Boolean(src)) => {
                Op::MoveInt { dst, src }
            }
            (Reg::Rational(dst), Reg::Rational(src)) => Op::MoveRational { dst, src },
            (Reg::Object(dst), Reg::Object(src)) => Op::MoveObject { dst, src },
            _ => return,
        };
        self.emit(op);
    }

    fn block(&mut self, statements: &[Stmt]) -> Result<(), Diagnostic> {
        for statement in statements {
            let mark = self.next;
            self.statement(statement)?;
            self.next = mark;
        }
        Ok(())
    }

    /// A trailing `if` (D43): jumps past the statement when false. Returns the jump to point past it.
    fn skip_unless(&mut self, condition: Option<&Expr>) -> Result<Option<usize>, Diagnostic> {
        let Some(condition) = condition else {
            return Ok(None);
        };
        Ok(Some(self.jump_on(condition, false)?))
    }

    /// Emits a jump taken when `condition` is `when`, to be pointed later. A comparison of ints or booleans becomes
    /// one compare-and-jump instruction.
    fn jump_on(&mut self, condition: &Expr, when: bool) -> Result<usize, Diagnostic> {
        if let ExprKind::Compare(op, left, right) = &condition.kind
            && matches!(left.ty, Type::Int | Type::Boolean)
            && matches!(right.ty, Type::Int | Type::Boolean)
        {
            let (Reg::Int(a) | Reg::Boolean(a)) = self.expression(left, None)? else {
                return Err(Diagnostic::bug(
                    "an int comparison on non-int registers",
                    condition.span,
                ));
            };
            let b = match constant_int(right) {
                Some(value) => Operand::Constant(value),
                None => match self.expression(right, None)? {
                    Reg::Int(b) | Reg::Boolean(b) => Operand::Register(b),
                    _ => {
                        return Err(Diagnostic::bug(
                            "an int comparison on non-int registers",
                            condition.span,
                        ));
                    }
                },
            };
            let op = if when { *op } else { negated(*op) };
            return Ok(self.emit(Op::JumpIfCompare { op, a, b, to: 0 }));
        }
        let condition = self.boolean(condition)?;
        Ok(self.emit(if when {
            Op::JumpIfTrue { condition, to: 0 }
        } else {
            Op::JumpIfFalse { condition, to: 0 }
        }))
    }

    fn end_skip(&mut self, skip: Option<usize>) {
        if let Some(skip) = skip {
            let here = self.here();
            self.patch(skip, here);
        }
    }

    fn statement(&mut self, statement: &Stmt) -> Result<(), Diagnostic> {
        match statement {
            Stmt::Assign {
                slot,
                value,
                condition,
            } => {
                let skip = self.skip_unless(condition.as_ref())?;
                let target = self.slot(*slot, value.span)?;
                let result = self.expression(value, Some(target))?;
                self.move_into(target, result);
                self.end_skip(skip);
            }
            Stmt::Show {
                value,
                count,
                condition,
            } => {
                let skip = self.skip_unless(condition.as_ref())?;
                let src = self.expression(value, None)?;
                self.emit(Op::Show {
                    src,
                    count: *count,
                    at: value.span,
                });
                self.end_skip(skip);
            }
            Stmt::ShowFormat {
                parts,
                count,
                condition,
            } => {
                let skip = self.skip_unless(condition.as_ref())?;
                self.show_format(parts, *count)?;
                self.end_skip(skip);
            }
            Stmt::ChangeItem {
                slot,
                selector,
                value,
                name_span,
                condition,
            } => {
                let skip = self.skip_unless(condition.as_ref())?;
                self.change_item(*slot, selector, value, *name_span)?;
                self.end_skip(skip);
            }
            Stmt::ShowItems {
                table,
                stop,
                inclusive,
                condition,
            } => {
                let skip = self.skip_unless(condition.as_ref())?;
                self.show_items(table, stop, *inclusive)?;
                self.end_skip(skip);
            }
            Stmt::RepeatTimes { .. } | Stmt::RepeatUntil { .. } | Stmt::RepeatForever { .. } => {
                self.repeat(statement)?;
            }
            Stmt::ForItems {
                table,
                stop,
                inclusive,
                current_item,
                body,
            } => self.for_items(table, stop, *inclusive, *current_item, body)?,
            Stmt::Stop { condition } => {
                let skip = self.skip_unless(condition.as_ref())?;
                let exit = self.emit(Op::Jump { to: 0 });
                self.loop_exits
                    .last_mut()
                    .ok_or_else(|| Diagnostic::bug("a `stop` outside a loop", Span::new(0, 0)))?
                    .push(exit);
                self.end_skip(skip);
            }
            Stmt::If {
                branches,
                otherwise,
            } => {
                let mut ends = Vec::with_capacity(branches.len());
                for (condition, body) in branches {
                    let next = self.jump_on(condition, false)?;
                    self.block(body)?;
                    ends.push(self.emit(Op::Jump { to: 0 }));
                    let here = self.here();
                    self.patch(next, here);
                }
                self.block(otherwise)?;
                let here = self.here();
                for end in ends {
                    self.patch(end, here);
                }
            }
        }
        Ok(())
    }

    fn show_format(&mut self, parts: &[FormatPart], count: i64) -> Result<(), Diagnostic> {
        let mut pieces = Vec::with_capacity(parts.len());
        for part in parts {
            pieces.push(match part {
                FormatPart::Text(text) => Piece::Text(text.clone()),
                FormatPart::Hole(hole) => {
                    let value = self.expression(hole, None)?;
                    Piece::Hole(self.text_of(value, hole.span))
                }
            });
        }
        let dst = self.temp_object();
        self.emit(Op::Format {
            dst,
            pieces: pieces.into_boxed_slice(),
        });
        self.emit(Op::Show {
            src: Reg::Object(dst),
            count,
            at: Span::new(0, 0),
        });
        Ok(())
    }

    fn change_item(
        &mut self,
        slot: usize,
        selector: &Selector,
        value: &Expr,
        name_span: Span,
    ) -> Result<(), Diagnostic> {
        let Reg::Object(table) = self.slot(slot, name_span)? else {
            return Err(Diagnostic::bug(
                "a table in a non-object register",
                name_span,
            ));
        };
        match selector {
            Selector::Index(index, counting) => {
                let index_reg = self.int(index)?;
                let value = self.expression(value, None)?;
                self.emit(Op::ChangePosition {
                    table,
                    index: index_reg,
                    counting: *counting,
                    value,
                    at: index.span,
                });
            }
            Selector::Key(key) => {
                let key_reg = self.expression(key, None)?;
                let value = self.expression(value, None)?;
                self.emit(Op::ChangeKey {
                    table,
                    key: key_reg,
                    value,
                    at: key.span,
                });
            }
        }
        Ok(())
    }

    // D38: works out where to stop first, then shows the items before it.
    fn show_items(&mut self, table: &Expr, stop: &Stop, inclusive: bool) -> Result<(), Diagnostic> {
        let table_reg = self.object(table)?;
        let end = self.temp_int();
        match stop {
            Stop::LastItem => {
                self.emit(Op::Length {
                    dst: end,
                    table: table_reg,
                });
                if !inclusive {
                    self.decrement(end);
                }
            }
            Stop::Position(position) => {
                let position_reg = self.int(position)?;
                self.emit(Op::StopPosition {
                    dst: end,
                    table: table_reg,
                    position: position_reg,
                    at: position.span,
                });
                if inclusive {
                    self.emit(Op::Increment { dst: end });
                }
            }
            Stop::Value(value) => {
                let value_reg = self.expression(value, None)?;
                self.emit(Op::FindValue {
                    dst: end,
                    table: table_reg,
                    value: value_reg,
                    at: value.span,
                });
                if inclusive {
                    self.emit(Op::Increment { dst: end });
                }
            }
            Stop::Test { test, current_item } => {
                let item = self.slot(*current_item, test.span)?;
                let length = self.temp_int();
                self.emit(Op::Length {
                    dst: length,
                    table: table_reg,
                });
                self.emit(Op::IntConstant { dst: end, value: 0 });
                let top = self.here();
                let to_never = self.emit(Op::JumpIfAtLeast {
                    a: end,
                    b: length,
                    to: 0,
                });
                self.emit(Op::Item {
                    dst: item,
                    table: table_reg,
                    index: end,
                });
                let reached = self.boolean(test)?;
                let to_found = self.emit(Op::JumpIfTrue {
                    condition: reached,
                    to: 0,
                });
                self.emit(Op::Increment { dst: end });
                self.emit(Op::Jump { to: top });
                let never = self.emit(Op::NeverReached { at: test.span });
                self.patch(to_never, never);
                let found = self.here();
                self.patch(to_found, found);
                if inclusive {
                    self.emit(Op::Increment { dst: end });
                }
            }
        }
        self.emit(Op::ShowItems {
            table: table_reg,
            end,
            at: table.span,
        });
        Ok(())
    }

    fn decrement(&mut self, reg: u32) {
        let one = self.temp_int();
        self.emit(Op::IntConstant { dst: one, value: 1 });
        self.emit(Op::Subtract {
            dst: reg,
            a: reg,
            b: one,
            at: Span::new(0, 0),
        });
    }

    // D39, D50, D51: a hidden round counter, copied into `current_round` before each round.
    fn repeat(&mut self, statement: &Stmt) -> Result<(), Diagnostic> {
        let round = self.temp_int();
        let (current_round, body, limit, test) = match statement {
            Stmt::RepeatTimes {
                count,
                current_round,
                body,
            } => {
                let evaluated = self.int(count)?;
                let limit = self.temp_int();
                self.emit(Op::MoveInt {
                    dst: limit,
                    src: evaluated,
                });
                self.emit(Op::CheckCount {
                    src: limit,
                    at: count.span,
                });
                self.emit(Op::Increment { dst: limit });
                (*current_round, body, Some(limit), None)
            }
            Stmt::RepeatUntil {
                test,
                current_round,
                body,
            } => (*current_round, body, None, Some(test)),
            Stmt::RepeatForever {
                current_round,
                body,
            } => (*current_round, body, None, None),
            _ => {
                return Err(Diagnostic::bug(
                    "a `repeat` that isn't one",
                    Span::new(0, 0),
                ));
            }
        };
        let current_round = self.slot(current_round, Span::new(0, 0))?;
        self.emit(Op::IntConstant {
            dst: round,
            value: 1,
        });
        let top = self.here();
        self.loop_exits.push(Vec::new());
        if let Some(limit) = limit {
            let exit = self.emit(Op::JumpIfAtLeast {
                a: round,
                b: limit,
                to: 0,
            });
            self.current_exits()?.push(exit);
        }
        self.move_into(current_round, Reg::Int(round));
        if let Some(test) = test {
            let exit = self.jump_on(test, true)?;
            self.current_exits()?.push(exit);
        }
        self.block(body)?;
        self.emit(Op::Increment { dst: round });
        self.emit(Op::Jump { to: top });
        self.close_loop();
        Ok(())
    }

    fn current_exits(&mut self) -> Result<&mut Vec<usize>, Diagnostic> {
        self.loop_exits
            .last_mut()
            .ok_or_else(|| Diagnostic::bug("no loop to leave", Span::new(0, 0)))
    }

    fn close_loop(&mut self) {
        let here = self.here();
        if let Some(exits) = self.loop_exits.pop() {
            for exit in exits {
                self.patch(exit, here);
            }
        }
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
        let evaluated = self.object(table)?;
        // A copy of the table, so changes made in the block don't change what is gone through (D37).
        let table_reg = self.temp_object();
        self.emit(Op::MoveObject {
            dst: table_reg,
            src: evaluated,
        });
        let item = self.slot(current_item, table.span)?;
        let index = self.temp_int();
        let limit = self.temp_int();
        if let Stop::Position(position) = stop {
            let position_reg = self.int(position)?;
            self.emit(Op::StopPosition {
                dst: limit,
                table: table_reg,
                position: position_reg,
                at: position.span,
            });
            self.emit(Op::Increment { dst: limit });
        } else {
            self.emit(Op::Length {
                dst: limit,
                table: table_reg,
            });
        }
        let counted = matches!(stop, Stop::LastItem | Stop::Position(_));
        if counted && !inclusive {
            self.decrement(limit);
        }
        self.emit(Op::IntConstant {
            dst: index,
            value: 0,
        });
        let top = self.here();
        self.loop_exits.push(Vec::new());
        let to_end = self.emit(Op::JumpIfAtLeast {
            a: index,
            b: limit,
            to: 0,
        });
        self.emit(Op::Item {
            dst: item,
            table: table_reg,
            index,
        });
        let reached = self.reached(stop, table_reg, index)?;
        if let Some(reached) = reached
            && !inclusive
        {
            let exit = self.emit(Op::JumpIfTrue {
                condition: reached,
                to: 0,
            });
            self.current_exits()?.push(exit);
        }
        self.block(body)?;
        if let Some(reached) = reached
            && inclusive
        {
            let exit = self.emit(Op::JumpIfTrue {
                condition: reached,
                to: 0,
            });
            self.current_exits()?.push(exit);
        }
        self.emit(Op::Increment { dst: index });
        self.emit(Op::Jump { to: top });
        if counted {
            let end = self.here();
            self.patch(to_end, end);
        } else {
            // D38: going past the last item without reaching the stop.
            let span = match stop {
                Stop::Value(expression)
                | Stop::Test {
                    test: expression, ..
                } => expression.span,
                Stop::LastItem | Stop::Position(_) => table.span,
            };
            let never = self.emit(Op::NeverReached { at: span });
            self.patch(to_end, never);
        }
        self.close_loop();
        Ok(())
    }

    /// For a `for` with a value or test stop: works out whether the current item reaches it, into a register the
    /// block can't change.
    fn reached(
        &mut self,
        stop: &Stop,
        table_reg: u32,
        index: u32,
    ) -> Result<Option<u32>, Diagnostic> {
        Ok(match stop {
            Stop::Value(value) => {
                let value_reg = self.expression(value, None)?;
                let reached = self.temp_int();
                self.emit(Op::ItemEquals {
                    dst: reached,
                    table: table_reg,
                    index,
                    value: value_reg,
                });
                Some(reached)
            }
            Stop::Test { test, .. } => {
                let tested = self.boolean(test)?;
                let reached = self.temp_int();
                self.emit(Op::MoveInt {
                    dst: reached,
                    src: tested,
                });
                Some(reached)
            }
            Stop::LastItem | Stop::Position(_) => None,
        })
    }

    fn int(&mut self, expression: &Expr) -> Result<u32, Diagnostic> {
        match self.expression(expression, None)? {
            Reg::Int(reg) => Ok(reg),
            _ => Err(Diagnostic::bug(
                "an int in a non-int register",
                expression.span,
            )),
        }
    }

    fn boolean(&mut self, expression: &Expr) -> Result<u32, Diagnostic> {
        match self.expression(expression, None)? {
            Reg::Boolean(reg) => Ok(reg),
            _ => Err(Diagnostic::bug(
                "a boolean in a non-boolean register",
                expression.span,
            )),
        }
    }

    fn rational(&mut self, expression: &Expr) -> Result<u32, Diagnostic> {
        match self.expression(expression, None)? {
            Reg::Rational(reg) => Ok(reg),
            _ => Err(Diagnostic::bug(
                "a rational in a non-rational register",
                expression.span,
            )),
        }
    }

    fn object(&mut self, expression: &Expr) -> Result<u32, Diagnostic> {
        match self.expression(expression, None)? {
            Reg::Object(reg) => Ok(reg),
            _ => Err(Diagnostic::bug(
                "an object in a non-object register",
                expression.span,
            )),
        }
    }

    /// The register holding a value as text: text as it is, anything else turned into text as `show` would.
    fn text_of(&mut self, value: Reg, at: Span) -> u32 {
        if let Reg::Object(reg) = value {
            return reg;
        }
        let dst = self.temp_object();
        self.emit(Op::ToText {
            dst,
            src: value,
            at,
        });
        dst
    }

    /// Where the result goes: `into` when given and of the right file, else a new register.
    fn destination(&mut self, into: Option<Reg>, ty: &Type) -> Reg {
        match into {
            Some(reg) if fits(reg, ty) => reg,
            _ => self.temp(ty),
        }
    }
}

/// Whether a register's file holds values of a type.
fn fits(reg: Reg, ty: &Type) -> bool {
    matches!(
        (reg, ty),
        (Reg::Int(_), Type::Int)
            | (Reg::Boolean(_), Type::Boolean)
            | (Reg::Rational(_), Type::Rational)
            | (
                Reg::Object(_),
                Type::Text | Type::InOrder(_) | Type::Keyed(..)
            )
    )
}

impl Compiler {
    /// Compiles an expression, returning the register holding its value. `into` is a hint for where to put it;
    /// the last instruction writes there only after it has read everything it needs.
    fn expression(&mut self, expression: &Expr, into: Option<Reg>) -> Result<Reg, Diagnostic> {
        let at = expression.span;
        let ty = &expression.ty;
        match &expression.kind {
            ExprKind::Int(value) => Ok(self.constant(into, ty, Value::Int(*value))),
            ExprKind::Rational(value) => Ok(self.constant(into, ty, Value::Rational(*value))),
            ExprKind::Text(text) => Ok(self.constant(into, ty, Value::Text(text.clone()))),
            ExprKind::Boolean(value) => Ok(self.constant(into, ty, Value::Boolean(*value))),
            ExprKind::Load(slot) => self.slot(*slot, at),
            ExprKind::IntArithmetic(..)
            | ExprKind::IntDivide(..)
            | ExprKind::IntToRational(..)
            | ExprKind::RationalArithmetic(..) => self.arithmetic(expression, into),
            ExprKind::Compare(comparison, left, right) => {
                self.compare(*comparison, left, right, into, at)
            }
            ExprKind::And(..) | ExprKind::Or(..) | ExprKind::Not(..) => {
                self.logic(expression, into)
            }
            ExprKind::Join(..) | ExprKind::Ask(..) => self.text(expression, into),
            ExprKind::InOrder(..)
            | ExprKind::Keyed(..)
            | ExprKind::ReadPosition { .. }
            | ExprKind::ReadKey { .. } => self.table(expression, into),
        }
    }

    fn constant(&mut self, into: Option<Reg>, ty: &Type, value: Value) -> Reg {
        let dst = self.destination(into, ty);
        self.emit_constant(dst, value);
        dst
    }

    fn arithmetic(&mut self, expression: &Expr, into: Option<Reg>) -> Result<Reg, Diagnostic> {
        let at = expression.span;
        let ty = &expression.ty;
        match &expression.kind {
            ExprKind::IntArithmetic(arithmetic, left, right)
                if constant_operand(*arithmetic, left, right).is_some() =>
            {
                let Some((variable, value)) = constant_operand(*arithmetic, left, right) else {
                    return Err(Diagnostic::bug("a constant operand that went away", at));
                };
                let a = self.int(variable)?;
                let Reg::Int(dst) = self.destination(into, ty) else {
                    return Err(Diagnostic::bug("an int result in a non-int register", at));
                };
                self.emit(match arithmetic {
                    Arithmetic::Plus => Op::AddConstant { dst, a, value, at },
                    Arithmetic::Minus => Op::SubtractConstant { dst, a, value, at },
                    Arithmetic::Times => Op::MultiplyConstant { dst, a, value, at },
                    _ => Op::RemainderConstant { dst, a, value, at },
                });
                Ok(Reg::Int(dst))
            }
            ExprKind::IntArithmetic(arithmetic, left, right) => {
                let (a, b) = (self.int(left)?, self.int(right)?);
                let Reg::Int(dst) = self.destination(into, ty) else {
                    return Err(Diagnostic::bug("an int result in a non-int register", at));
                };
                let op = match arithmetic {
                    Arithmetic::Plus => Op::Add { dst, a, b, at },
                    Arithmetic::Minus => Op::Subtract { dst, a, b, at },
                    Arithmetic::Times => Op::Multiply { dst, a, b, at },
                    Arithmetic::Remainder => Op::Remainder { dst, a, b, at },
                    Arithmetic::DividedBy => {
                        return Err(Diagnostic::bug("int division giving an int", at));
                    }
                };
                self.emit(op);
                Ok(Reg::Int(dst))
            }
            ExprKind::IntDivide(left, right) => {
                let (a, b) = (self.int(left)?, self.int(right)?);
                let dst = self.rational_destination(into, ty, at)?;
                self.emit(Op::DivideInts { dst, a, b, at });
                Ok(Reg::Rational(dst))
            }
            ExprKind::IntToRational(int) => {
                let src = self.int(int)?;
                let dst = self.rational_destination(into, ty, at)?;
                self.emit(Op::IntToRational { dst, src });
                Ok(Reg::Rational(dst))
            }
            ExprKind::RationalArithmetic(arithmetic, left, right) => {
                let (a, b) = (self.rational(left)?, self.rational(right)?);
                let dst = self.rational_destination(into, ty, at)?;
                let op = match arithmetic {
                    Arithmetic::Plus => Op::AddRationals { dst, a, b, at },
                    Arithmetic::Minus => Op::SubtractRationals { dst, a, b, at },
                    Arithmetic::Times => Op::MultiplyRationals { dst, a, b, at },
                    Arithmetic::DividedBy => Op::DivideRationals { dst, a, b, at },
                    Arithmetic::Remainder => {
                        return Err(Diagnostic::bug("`remainder` on rationals", at));
                    }
                };
                self.emit(op);
                Ok(Reg::Rational(dst))
            }
            _ => Err(Diagnostic::bug("arithmetic that isn't", at)),
        }
    }

    fn rational_destination(
        &mut self,
        into: Option<Reg>,
        ty: &Type,
        at: Span,
    ) -> Result<u32, Diagnostic> {
        match self.destination(into, ty) {
            Reg::Rational(dst) => Ok(dst),
            _ => Err(Diagnostic::bug(
                "a rational result in a non-rational register",
                at,
            )),
        }
    }

    fn object_destination(
        &mut self,
        into: Option<Reg>,
        ty: &Type,
        at: Span,
    ) -> Result<u32, Diagnostic> {
        match self.destination(into, ty) {
            Reg::Object(dst) => Ok(dst),
            _ => Err(Diagnostic::bug(
                "an object result in a non-object register",
                at,
            )),
        }
    }

    fn logic(&mut self, expression: &Expr, into: Option<Reg>) -> Result<Reg, Diagnostic> {
        let at = expression.span;
        match &expression.kind {
            ExprKind::And(left, right) | ExprKind::Or(left, right) => {
                // Not into `into`: the right side may read the variable being set.
                let dst = self.temp_int();
                let a = self.boolean(left)?;
                self.emit(Op::MoveInt { dst, src: a });
                let condition = dst;
                let skip = if matches!(expression.kind, ExprKind::And(..)) {
                    self.emit(Op::JumpIfFalse { condition, to: 0 })
                } else {
                    self.emit(Op::JumpIfTrue { condition, to: 0 })
                };
                let b = self.boolean(right)?;
                self.emit(Op::MoveInt { dst, src: b });
                let here = self.here();
                self.patch(skip, here);
                Ok(Reg::Boolean(dst))
            }
            ExprKind::Not(operand) => {
                let src = self.boolean(operand)?;
                let Reg::Boolean(dst) = self.destination(into, &expression.ty) else {
                    return Err(Diagnostic::bug(
                        "a boolean result in a non-boolean register",
                        at,
                    ));
                };
                self.emit(Op::Not { dst, src });
                Ok(Reg::Boolean(dst))
            }
            _ => Err(Diagnostic::bug("logic that isn't", at)),
        }
    }

    fn text(&mut self, expression: &Expr, into: Option<Reg>) -> Result<Reg, Diagnostic> {
        let at = expression.span;
        match &expression.kind {
            // D48, D49
            ExprKind::Join(left, right) => {
                let left_value = self.expression(left, None)?;
                let a = self.text_of(left_value, left.span);
                let right_value = self.expression(right, None)?;
                let b = self.text_of(right_value, right.span);
                let dst = self.object_destination(into, &expression.ty, at)?;
                self.emit(Op::Join { dst, a, b });
                Ok(Reg::Object(dst))
            }
            ExprKind::Ask(prompt) => {
                let prompt_reg = self.object(prompt)?;
                let dst = self.object_destination(into, &expression.ty, at)?;
                self.emit(Op::Ask {
                    dst,
                    prompt: prompt_reg,
                    at: prompt.span,
                });
                Ok(Reg::Object(dst))
            }
            _ => Err(Diagnostic::bug("text that isn't", at)),
        }
    }

    fn table(&mut self, expression: &Expr, into: Option<Reg>) -> Result<Reg, Diagnostic> {
        let at = expression.span;
        let ty = &expression.ty;
        match &expression.kind {
            ExprKind::InOrder(items) => {
                let mut regs = Vec::with_capacity(items.len());
                for item in items {
                    regs.push(self.expression(item, None)?);
                }
                let dst = self.object_destination(into, ty, at)?;
                self.emit(Op::MakeInOrder {
                    dst,
                    items: regs.into_boxed_slice(),
                });
                Ok(Reg::Object(dst))
            }
            ExprKind::Keyed(entries) => {
                // Not into `into`: the table is built up while keys and values are worked out.
                let dst = self.temp_object();
                self.emit(Op::StartKeyed { dst });
                for (key, value) in entries {
                    let key_reg = self.expression(key, None)?;
                    self.emit(Op::CheckNewKey {
                        table: dst,
                        key: key_reg,
                        at: key.span,
                    });
                    let value_reg = self.expression(value, None)?;
                    self.emit(Op::PushKeyed {
                        table: dst,
                        key: key_reg,
                        value: value_reg,
                    });
                }
                Ok(Reg::Object(dst))
            }
            ExprKind::ReadPosition {
                table,
                index,
                counting,
            } => {
                let table_reg = self.object(table)?;
                let index_reg = self.int(index)?;
                let dst = self.destination(into, ty);
                self.emit(Op::ReadPosition {
                    dst,
                    table: table_reg,
                    index: index_reg,
                    counting: *counting,
                    at: index.span,
                });
                Ok(dst)
            }
            ExprKind::ReadKey { table, key } => {
                let table_reg = self.object(table)?;
                let key_reg = self.expression(key, None)?;
                let dst = self.destination(into, ty);
                self.emit(Op::ReadKey {
                    dst,
                    table: table_reg,
                    key: key_reg,
                    at: key.span,
                });
                Ok(dst)
            }
            _ => Err(Diagnostic::bug("a table expression that isn't", at)),
        }
    }

    fn emit_constant(&mut self, dst: Reg, value: Value) {
        let op = match (dst, value) {
            (Reg::Int(dst), Value::Int(value)) => Op::IntConstant { dst, value },
            (Reg::Boolean(dst), Value::Boolean(value)) => Op::IntConstant {
                dst,
                value: i64::from(value),
            },
            (Reg::Rational(dst), Value::Rational(value)) => Op::RationalConstant { dst, value },
            (Reg::Object(dst), value) => Op::ObjectConstant { dst, value },
            _ => return,
        };
        self.emit(op);
    }

    fn compare(
        &mut self,
        op: Comparison,
        left: &Expr,
        right: &Expr,
        into: Option<Reg>,
        at: Span,
    ) -> Result<Reg, Diagnostic> {
        if matches!(left.ty, Type::Int | Type::Boolean)
            && let Some(value) = constant_int(right)
            && let Reg::Int(a) | Reg::Boolean(a) = self.expression(left, None)?
        {
            let Reg::Boolean(dst) = self.destination(into, &Type::Boolean) else {
                return Err(Diagnostic::bug(
                    "a boolean result in a non-boolean register",
                    at,
                ));
            };
            self.emit(Op::CompareConstant { op, dst, a, value });
            return Ok(Reg::Boolean(dst));
        }
        let a = self.expression(left, None)?;
        let b = self.expression(right, None)?;
        let Reg::Boolean(dst) = self.destination(into, &Type::Boolean) else {
            return Err(Diagnostic::bug(
                "a boolean result in a non-boolean register",
                at,
            ));
        };
        let instruction = match (a, b) {
            (Reg::Int(a) | Reg::Boolean(a), Reg::Int(b) | Reg::Boolean(b)) => {
                Op::CompareInts { op, dst, a, b }
            }
            (Reg::Rational(a), Reg::Rational(b)) => Op::CompareRationals { op, dst, a, b },
            (Reg::Object(a), Reg::Object(b))
                if matches!(op, Comparison::Is | Comparison::IsNot) =>
            {
                Op::CompareObjects {
                    equal: op == Comparison::Is,
                    dst,
                    a,
                    b,
                }
            }
            _ => return Err(Diagnostic::bug("a comparison the checker ruled out", at)),
        };
        self.emit(instruction);
        Ok(Reg::Boolean(dst))
    }
}

/// The value of an int or boolean written as a constant in the program.
fn constant_int(expression: &Expr) -> Option<i64> {
    match expression.kind {
        ExprKind::Int(value) => Some(value),
        ExprKind::Boolean(value) => Some(i64::from(value)),
        _ => None,
    }
}

/// For int arithmetic with a constant written on one side, the other side and the constant: on the right for
/// any word, on the left only for `plus` and `times`, which don't care about order. `remainder` takes only a positive
/// constant, so its zero and negative cases stay in the general instruction.
fn constant_operand<'e>(
    arithmetic: Arithmetic,
    left: &'e Expr,
    right: &'e Expr,
) -> Option<(&'e Expr, i64)> {
    match (arithmetic, constant_int(left), constant_int(right)) {
        (Arithmetic::Remainder, _, Some(value)) if value > 0 => Some((left, value)),
        (Arithmetic::Remainder | Arithmetic::DividedBy, _, _) => None,
        (_, _, Some(value)) => Some((left, value)),
        (Arithmetic::Plus | Arithmetic::Times, Some(value), None) => Some((right, value)),
        _ => None,
    }
}

/// The comparison that is true exactly when `op` is false.
fn negated(op: Comparison) -> Comparison {
    match op {
        Comparison::Is => Comparison::IsNot,
        Comparison::IsNot => Comparison::Is,
        Comparison::AtLeast => Comparison::LessThan,
        Comparison::LessThan => Comparison::AtLeast,
        Comparison::AtMost => Comparison::GreaterThan,
        Comparison::GreaterThan => Comparison::AtMost,
    }
}
