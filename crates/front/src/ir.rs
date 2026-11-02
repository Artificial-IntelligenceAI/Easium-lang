//! The checked program: every type known, every variable a numbered slot (D4). The interpreter runs this; the JIT
//! tiers will compile it (D3).

use std::rc::Rc;

use redstone_runtime::Rational;

pub use crate::ast::Counting;
use crate::diagnostic::Span;

/// A checked program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    /// The statements, in order.
    pub statements: Vec<Stmt>,
    /// How many variable slots the program uses.
    pub slot_count: usize,
}

/// The type of a value (D4, D7, D19, D32).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    /// A 64-bit whole number.
    Int,
    /// An exact fraction.
    Rational,
    /// Text.
    Text,
    /// `true` or `false`.
    Boolean,
    /// An in-order table, and the type of its values.
    InOrder(Rc<Type>),
    /// A keyed table, and the types of its keys and values.
    Keyed(Rc<Type>, Rc<Type>),
}

impl Type {
    /// The type named in a sentence: "an int", "text", "an in-order table of ints".
    #[must_use]
    pub fn described(&self) -> String {
        match self {
            Type::Int => "an int".to_owned(),
            Type::Rational => "a rational".to_owned(),
            Type::Text => "text".to_owned(),
            Type::Boolean => "a boolean".to_owned(),
            Type::InOrder(value) => format!("an in-order table of {}", value.plural()),
            Type::Keyed(key, value) => {
                format!("a keyed table from {} to {}", key.plural(), value.plural())
            }
        }
    }

    /// The type named as many: "ints", "text".
    #[must_use]
    pub fn plural(&self) -> String {
        match self {
            Type::Int => "ints".to_owned(),
            Type::Rational => "rationals".to_owned(),
            Type::Text => "text".to_owned(),
            Type::Boolean => "booleans".to_owned(),
            Type::InOrder(value) => format!("in-order tables of {}", value.plural()),
            Type::Keyed(key, value) => {
                format!("keyed tables from {} to {}", key.plural(), value.plural())
            }
        }
    }

    /// Whether this is a table type.
    #[must_use]
    pub fn is_table(&self) -> bool {
        matches!(self, Type::InOrder(_) | Type::Keyed(..))
    }
}

/// One checked statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    /// Stores a value in a slot: both `=` and `is_now`.
    Assign {
        /// The variable's slot.
        slot: usize,
        /// The value.
        value: Expr,
    },
    /// Shows a value, if the condition holds.
    Show {
        /// What to show.
        value: Expr,
        /// How many times (D40).
        count: i64,
        /// Shown only when this is true.
        condition: Option<Expr>,
    },
    /// Shows a table's items, one per line, up to a stop (D38).
    ShowItems {
        /// The in-order table.
        table: Expr,
        /// Where to stop.
        stop: Stop,
        /// Whether the item where it stops is shown.
        inclusive: bool,
        /// Shown only when this is true.
        condition: Option<Expr>,
    },
    /// Runs a block a number of times (D39).
    RepeatTimes {
        /// How many times.
        count: Expr,
        /// The block.
        body: Vec<Stmt>,
    },
    /// Runs a block until a test, checked before each round, is true (D39).
    RepeatUntil {
        /// The test.
        test: Expr,
        /// The block.
        body: Vec<Stmt>,
    },
    /// Runs a block until a `stop` leaves it (D41).
    RepeatForever {
        /// The block.
        body: Vec<Stmt>,
    },
    /// Leaves the innermost loop, if the condition holds (D41).
    Stop {
        /// Leaves only when this is true.
        condition: Option<Expr>,
    },
    /// Runs a block for each item of an in-order table, up to a stop (D39).
    ForItems {
        /// The table.
        table: Expr,
        /// Where to stop.
        stop: Stop,
        /// Whether the block runs for the item where it stops.
        inclusive: bool,
        /// The slot holding `current_item`.
        current_item: usize,
        /// The block.
        body: Vec<Stmt>,
    },
    /// Changes one item of the table in a slot (D35).
    ChangeItem {
        /// The slot holding the table.
        slot: usize,
        /// Which item.
        selector: Selector,
        /// Its new value.
        value: Expr,
        /// Where the table's name is, for errors while running.
        name_span: Span,
    },
    /// Runs the first block whose condition holds, or the `else` block.
    If {
        /// Each condition and its block, in order.
        branches: Vec<(Expr, Vec<Stmt>)>,
        /// The `else` block; empty when there is none.
        otherwise: Vec<Stmt>,
    },
}

/// Where `show items in … until` stops, checked (D38).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// After the last item.
    LastItem,
    /// At a position, an int.
    Position(Expr),
    /// At the first item equal to this.
    Value(Expr),
    /// At the first item that makes the test true; `current_item` is in the slot.
    Test {
        /// The test.
        test: Expr,
        /// The slot `current_item` is read from.
        current_item: usize,
    },
}

/// Which item of a table, checked (D34, D35).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    /// A position or offset, an int.
    Index(Expr, Counting),
    /// A key.
    Key(Expr),
}

/// A checked expression, with where it is, for errors while running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    /// What it computes.
    pub kind: ExprKind,
    /// Where it is.
    pub span: Span,
}

/// What a checked expression computes; each operation knows its operand types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    /// An int.
    Int(i64),
    /// A rational.
    Rational(Rational),
    /// Text.
    Text(Rc<str>),
    /// A boolean.
    Boolean(bool),
    /// A variable's value.
    Load(usize),
    /// Arithmetic on two ints, giving an int; never `divided_by`.
    IntArithmetic(Arithmetic, Box<Expr>, Box<Expr>),
    /// `divided_by` on two ints, giving a rational (D7).
    IntDivide(Box<Expr>, Box<Expr>),
    /// An int used where a rational is needed, beside a rational in arithmetic (D7).
    IntToRational(Box<Expr>),
    /// Arithmetic on two rationals, giving a rational; never `remainder`.
    RationalArithmetic(Arithmetic, Box<Expr>, Box<Expr>),
    /// A comparison of two values of the same type; ordering only on ints or rationals.
    Compare(Comparison, Box<Expr>, Box<Expr>),
    /// `and`
    And(Box<Expr>, Box<Expr>),
    /// `or`
    Or(Box<Expr>, Box<Expr>),
    /// `not`
    Not(Box<Expr>),
    /// `ask`, with its prompt (D26).
    Ask(Box<Expr>),
    /// Makes an in-order table (D33).
    InOrder(Vec<Expr>),
    /// Makes a keyed table (D33).
    Keyed(Vec<(Expr, Expr)>),
    /// Reads an in-order table by position or offset (D34).
    ReadPosition {
        /// The table.
        table: Box<Expr>,
        /// The position or offset, an int.
        index: Box<Expr>,
        /// How the index counts.
        counting: Counting,
    },
    /// Reads a keyed table by key (D34).
    ReadKey {
        /// The table.
        table: Box<Expr>,
        /// The key.
        key: Box<Expr>,
    },
}

/// An arithmetic word (D16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arithmetic {
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
}

/// A comparison word (D18, D20).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
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
}
