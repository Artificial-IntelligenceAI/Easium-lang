//! The program as written, before checking.

use redstone_runtime::Rational;

use crate::diagnostic::Span;

/// A whole program: its statements, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    /// The statements, in order.
    pub statements: Vec<Stmt>,
}

/// One statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    /// `age = 18`: makes a variable (D12).
    Create {
        /// The variable's name.
        name: String,
        /// Where the name is.
        name_span: Span,
        /// Its first value.
        value: Expr,
    },
    /// `age is_now 19`: changes a variable (D14).
    Change {
        /// The variable's name.
        name: String,
        /// Where the name is.
        name_span: Span,
        /// Its new value.
        value: Expr,
    },
    /// `show value`, or `show value if condition`.
    Show {
        /// What to show.
        value: Expr,
        /// Shown only when this is true.
        condition: Option<Expr>,
    },
    /// A block `if`, closed by `end_of if` (D22, D24, D25).
    If {
        /// The `if` and each `else_if`, in order.
        branches: Vec<Branch>,
        /// The `else` block, if there is one.
        otherwise: Option<Vec<Stmt>>,
    },
}

/// A condition and the block it guards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// The condition.
    pub condition: Expr,
    /// The block run when it is true.
    pub body: Vec<Stmt>,
}

/// An expression, with where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    /// What it is.
    pub kind: ExprKind,
    /// Where it is.
    pub span: Span,
}

/// What an expression is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    /// A whole number.
    Int(i64),
    /// A decimal, which is a rational.
    Rational(Rational),
    /// Text.
    Text(String),
    /// `true` or `false`.
    Boolean(bool),
    /// A variable.
    Name(String),
    /// Two operands and a word between them.
    Binary {
        /// The word.
        op: BinaryOp,
        /// Where the word is.
        op_span: Span,
        /// The left operand.
        left: Box<Expr>,
        /// The right operand.
        right: Box<Expr>,
    },
    /// `not operand`.
    Not(Box<Expr>),
    /// `ask prompt` (D26).
    Ask(Box<Expr>),
}

/// A word between two operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    /// `plus`
    Plus,
    /// `minus`
    Minus,
    /// `times`
    Times,
    /// `divided_by`
    DividedBy,
    /// `remainder`
    Remainder,
    /// `is`
    Is,
    /// `is_not`
    IsNot,
    /// `at_least`
    AtLeast,
    /// `at_most`
    AtMost,
    /// `greater_than`
    GreaterThan,
    /// `less_than`
    LessThan,
    /// `and`
    And,
    /// `or`
    Or,
}

impl BinaryOp {
    /// How the word is written.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            BinaryOp::Plus => "plus",
            BinaryOp::Minus => "minus",
            BinaryOp::Times => "times",
            BinaryOp::DividedBy => "divided_by",
            BinaryOp::Remainder => "remainder",
            BinaryOp::Is => "is",
            BinaryOp::IsNot => "is_not",
            BinaryOp::AtLeast => "at_least",
            BinaryOp::AtMost => "at_most",
            BinaryOp::GreaterThan => "greater_than",
            BinaryOp::LessThan => "less_than",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
        }
    }
}
