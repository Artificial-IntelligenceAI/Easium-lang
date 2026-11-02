//! Rationals: exact fractions, as Raku's `Rat`.

use std::cmp::Ordering;
use std::fmt;

/// An exact fraction whose numerator and denominator each fit in 64 bits.
///
/// Always reduced, with a positive denominator, so equal rationals have equal parts.
// D7
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rational {
    numerator: i64,
    denominator: i64,
}

/// Why an operation on rationals has no rational result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RationalError {
    /// The divisor was zero.
    DivisionByZero,
    /// The reduced numerator or denominator doesn't fit in 64 bits. What happens then is still open.
    TooBig,
}

impl Rational {
    /// Builds `numerator / denominator`, reduced.
    ///
    /// # Errors
    ///
    /// `DivisionByZero` when `denominator` is zero; `TooBig` when the reduced parts don't fit in 64 bits.
    pub fn new(numerator: i128, denominator: i128) -> Result<Self, RationalError> {
        if denominator == 0 {
            return Err(RationalError::DivisionByZero);
        }
        let divisor = gcd(numerator.unsigned_abs(), denominator.unsigned_abs());
        let divisor = i128::try_from(divisor).map_err(|_| RationalError::TooBig)?;
        let (mut numerator, mut denominator) = (numerator / divisor, denominator / divisor);
        if denominator < 0 {
            numerator = numerator.checked_neg().ok_or(RationalError::TooBig)?;
            denominator = denominator.checked_neg().ok_or(RationalError::TooBig)?;
        }
        Ok(Self {
            numerator: i64::try_from(numerator).map_err(|_| RationalError::TooBig)?,
            denominator: i64::try_from(denominator).map_err(|_| RationalError::TooBig)?,
        })
    }

    /// The rational equal to an int.
    #[must_use]
    pub fn from_int(value: i64) -> Self {
        Self {
            numerator: value,
            denominator: 1,
        }
    }

    /// The numerator, after reducing.
    #[must_use]
    pub fn numerator(self) -> i64 {
        self.numerator
    }

    /// The denominator, after reducing; always positive.
    #[must_use]
    pub fn denominator(self) -> i64 {
        self.denominator
    }

    /// `self + other`.
    ///
    /// # Errors
    ///
    /// `TooBig` when the result's parts don't fit in 64 bits.
    pub fn checked_add(self, other: Self) -> Result<Self, RationalError> {
        let (a, b, c, d) = self.wide(other);
        let numerator = (a * d).checked_add(c * b).ok_or(RationalError::TooBig)?;
        Self::new(numerator, b * d)
    }

    /// `self - other`.
    ///
    /// # Errors
    ///
    /// `TooBig` when the result's parts don't fit in 64 bits.
    pub fn checked_sub(self, other: Self) -> Result<Self, RationalError> {
        let (a, b, c, d) = self.wide(other);
        let numerator = (a * d).checked_sub(c * b).ok_or(RationalError::TooBig)?;
        Self::new(numerator, b * d)
    }

    /// `self * other`.
    ///
    /// # Errors
    ///
    /// `TooBig` when the result's parts don't fit in 64 bits.
    pub fn checked_mul(self, other: Self) -> Result<Self, RationalError> {
        let (a, b, c, d) = self.wide(other);
        Self::new(a * c, b * d)
    }

    /// `self / other`.
    ///
    /// # Errors
    ///
    /// `DivisionByZero` when `other` is zero; `TooBig` when the result's parts don't fit in 64 bits.
    pub fn checked_div(self, other: Self) -> Result<Self, RationalError> {
        let (a, b, c, d) = self.wide(other);
        Self::new(a * d, b * c)
    }

    /// The exact decimal form, such as `3.5`, `-0.25` or `2`, or `None` when there is none, as for 1/3.
    #[must_use]
    pub fn to_decimal(self) -> Option<String> {
        let mut rest = self.denominator.unsigned_abs();
        let mut twos = 0_u32;
        while rest.is_multiple_of(2) {
            rest /= 2;
            twos += 1;
        }
        let mut fives = 0_u32;
        while rest.is_multiple_of(5) {
            rest /= 5;
            fives += 1;
        }
        if rest != 1 {
            return None;
        }
        let denominator = u128::from(self.denominator.unsigned_abs());
        let magnitude = u128::from(self.numerator.unsigned_abs());
        let mut text = String::new();
        if self.numerator < 0 {
            text.push('-');
        }
        text.push_str(&(magnitude / denominator).to_string());
        let places = twos.max(fives);
        if places > 0 {
            text.push('.');
            // Long division: the remainder stays below the denominator, so times ten it fits easily.
            let mut remainder = magnitude % denominator;
            for _ in 0..places {
                remainder *= 10;
                text.push_str(&(remainder / denominator).to_string());
                remainder %= denominator;
            }
        }
        Some(text)
    }

    fn wide(self, other: Self) -> (i128, i128, i128, i128) {
        (
            i128::from(self.numerator),
            i128::from(self.denominator),
            i128::from(other.numerator),
            i128::from(other.denominator),
        )
    }
}

impl Ord for Rational {
    fn cmp(&self, other: &Self) -> Ordering {
        // Denominators are positive, so cross-multiplying keeps the order; i64 × i64 fits in i128.
        let (a, b, c, d) = self.wide(*other);
        (a * d).cmp(&(c * b))
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.numerator, self.denominator)
    }
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{Rational, RationalError};

    fn r(numerator: i128, denominator: i128) -> Rational {
        Rational::new(numerator, denominator).unwrap()
    }

    #[test]
    fn reduces_with_a_positive_denominator() {
        assert_eq!(r(2, 4), r(1, 2));
        assert_eq!(r(1, -2).numerator(), -1);
        assert_eq!(r(1, -2).denominator(), 2);
        assert_eq!(r(0, -5), Rational::from_int(0));
    }

    #[test]
    fn tenths_add_exactly() {
        assert_eq!(r(1, 10).checked_add(r(2, 10)).unwrap(), r(3, 10));
    }

    #[test]
    fn a_third_times_three_is_one() {
        assert_eq!(
            r(1, 3).checked_mul(Rational::from_int(3)).unwrap(),
            Rational::from_int(1)
        );
    }

    #[test]
    fn errors() {
        assert_eq!(Rational::new(1, 0), Err(RationalError::DivisionByZero));
        assert_eq!(
            r(1, 2).checked_div(Rational::from_int(0)),
            Err(RationalError::DivisionByZero)
        );
        let big = Rational::from_int(i64::MAX);
        assert_eq!(big.checked_add(big), Err(RationalError::TooBig));
    }

    #[test]
    fn decimals() {
        assert_eq!(r(7, 2).to_decimal().as_deref(), Some("3.5"));
        assert_eq!(r(-1, 4).to_decimal().as_deref(), Some("-0.25"));
        assert_eq!(r(1, 8).to_decimal().as_deref(), Some("0.125"));
        assert_eq!(Rational::from_int(2).to_decimal().as_deref(), Some("2"));
        assert_eq!(r(1, 3).to_decimal(), None);
        assert_eq!(r(1, 6).to_decimal(), None);
    }

    #[test]
    fn order() {
        assert!(r(1, 3) < r(1, 2));
        assert!(r(-1, 2) < r(1, 3));
    }
}
