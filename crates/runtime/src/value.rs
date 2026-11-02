//! The values a running Easium program holds.

use std::rc::Rc;

use crate::Rational;

/// One value: an int or a rational (D7), text, a boolean (D19), or a table (D32).
///
/// Laid out as a C struct of a one-byte tag and its payload, so the Cranelift tier can read ints and booleans out of
/// tables in place (D36); it checks the layout before relying on it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[repr(C, u8)]
pub enum Value {
    /// A whole number, 64 bits.
    Int(i64),
    /// An exact fraction.
    Rational(Rational),
    /// Text.
    Text(Rc<str>),
    /// `true` or `false`.
    Boolean(bool),
    /// A table.
    Table(Rc<Table>),
}

/// A table's contents, in one of its two kinds (D32).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Table {
    /// Values at positions 1 to n, with no holes.
    InOrder(Vec<Value>),
    /// Keys and their values, in the order they were written.
    Keyed(Vec<(Value, Value)>),
}

impl Value {
    /// The text `show` writes for this value, or `None` where that is still open: a rational with no exact
    /// decimal, such as 1/3, or a table.
    #[must_use]
    pub fn show(&self) -> Option<String> {
        match self {
            Value::Int(value) => Some(value.to_string()),
            Value::Rational(value) => value.to_decimal(),
            Value::Text(value) => Some(value.to_string()),
            Value::Boolean(value) => Some(value.to_string()),
            Value::Table(_) => None,
        }
    }
}
