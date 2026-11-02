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
    /// `age becomes 19`: changes a variable (D14).
    Change {
        /// The variable's name.
        name: String,
        /// Where the name is.
        name_span: Span,
        /// Its new value.
        value: Expr,
        /// Runs only when this is true (D43).
        condition: Option<Expr>,
    },
    /// `show value`, or `show value if condition`.
    Show {
        /// What to show.
        value: Expr,
        /// Whether it was written `show whether …` (D28).
        whether: bool,
        /// How many times, from `50x` (D40); once when not written.
        count: i64,
        /// Shown only when this is true.
        condition: Option<Expr>,
    },
    /// `value at position 1 in scores becomes 95`: changes an item in a table held by a variable (D35).
    ChangeItem {
        /// The variable holding the table.
        name: String,
        /// Where the variable's name is.
        name_span: Span,
        /// Which item.
        selector: Selector,
        /// Its new value.
        value: Expr,
        /// Runs only when this is true (D43).
        condition: Option<Expr>,
    },
    /// `show items in scores until …`: shows a table's items, one per line, up to a stop (D38).
    ShowItems {
        /// The table.
        table: Expr,
        /// Where to stop.
        stop: Stop,
        /// `until` (true) shows the item where it stops; `until_before` (false) doesn't.
        inclusive: bool,
        /// Shown only when this is true.
        condition: Option<Expr>,
    },
    /// `repeat 3 times:` … `end_of repeat` (D39).
    RepeatTimes {
        /// How many times, an int.
        count: Expr,
        /// The block.
        body: Vec<Stmt>,
    },
    /// `repeat until lives is 0:` … `end_of repeat`: checks before each round (D39).
    RepeatUntil {
        /// Stops when this is true.
        test: Expr,
        /// The block.
        body: Vec<Stmt>,
    },
    /// `repeat forever:` … `end_of repeat` (D41).
    RepeatForever {
        /// The block.
        body: Vec<Stmt>,
    },
    /// `stop`: leaves the innermost loop (D41).
    Stop {
        /// Where `stop` is.
        span: Span,
        /// Leaves only when this is true.
        condition: Option<Expr>,
    },
    /// `for items in scores until …:` … `end_of for`, with `current_item` in the block (D39).
    ForItems {
        /// The table.
        table: Expr,
        /// Where to stop.
        stop: Stop,
        /// `until` (true) runs the block for the item where it stops; `until_before` (false) doesn't.
        inclusive: bool,
        /// The block.
        body: Vec<Stmt>,
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
    /// An in-order table: `90, 65, 70` (D33).
    InOrder(Vec<Expr>),
    /// A keyed table: a `table:` block of `key: value` lines (D33).
    Keyed(Vec<(Expr, Expr)>),
    /// `value at position 1 in scores` or `value at offset 0 in scores` (D34).
    ReadPosition {
        /// The table.
        table: Box<Expr>,
        /// The position or offset.
        index: Box<Expr>,
        /// How the index counts.
        counting: Counting,
    },
    /// `value of "Ada" in ages` (D34).
    ReadKey {
        /// The table.
        table: Box<Expr>,
        /// The key.
        key: Box<Expr>,
    },
}

/// Where `show items in … until` stops (D38).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// `last_item`
    LastItem,
    /// `position 2`
    Position(Expr),
    /// `value 65`: the first item equal to it.
    Value(Expr),
    /// `current_item less_than 70`: the first item that makes it true.
    Test(Expr),
}

/// Which item of a table (D34, D35).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    /// A position or offset in an in-order table.
    Index(Expr, Counting),
    /// A key in a keyed table.
    Key(Expr),
}

/// How an index into an in-order table counts (D34).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counting {
    /// `at position`: from 1.
    Position,
    /// `at offset`: from 0.
    Offset,
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
