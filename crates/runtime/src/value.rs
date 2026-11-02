//! The values a running Redstone program holds.

use std::rc::Rc;

use crate::Rational;

/// One value: an int or a rational (D7), text, or a boolean (D19).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// A whole number, 64 bits.
    Int(i64),
    /// An exact fraction.
    Rational(Rational),
    /// Text.
    Text(Rc<str>),
    /// `true` or `false`.
    Boolean(bool),
}

impl Value {
    /// The text `show` writes for this value, or `None` where that is still open: a rational with no exact
    /// decimal, such as 1/3.
    #[must_use]
    pub fn show(&self) -> Option<String> {
        match self {
            Value::Int(value) => Some(value.to_string()),
            Value::Rational(value) => value.to_decimal(),
            Value::Text(value) => Some(value.to_string()),
            Value::Boolean(value) => Some(value.to_string()),
        }
    }
}
