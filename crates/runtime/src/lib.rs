//! Redstone's runtime: the values a running program holds (D8).

mod rational;
mod value;

pub use rational::{Rational, RationalError};
pub use value::{Table, Value};
