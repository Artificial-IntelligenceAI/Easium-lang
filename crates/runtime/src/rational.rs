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

    /// `self + other`, the same result as [`Rational::checked_add`] by a quicker path, for the tiers: the
    /// reference interpreter keeps the simple one (D52, rule 10). A reduced fraction is unique, so any exact method
    /// gives the same answer, and "too big" means the same in both: the reduced result doesn't fit in 64 bits.
    ///
    /// # Errors
    ///
    /// `TooBig` when the result's parts don't fit in 64 bits.
    pub fn add_quickly(self, other: Self) -> Result<Self, RationalError> {
        self.sum_quickly(other, false)
    }

    /// `self - other`, as [`Rational::checked_sub`], by a quicker path (see [`Rational::add_quickly`]).
    ///
    /// # Errors
    ///
    /// `TooBig` when the result's parts don't fit in 64 bits.
    pub fn subtract_quickly(self, other: Self) -> Result<Self, RationalError> {
        self.sum_quickly(other, true)
    }

    // D36: same denominators (tenths and tenths, ints and ints) need only the sum reduced; otherwise Knuth's
    // method keeps the numbers small: with g = gcd(b, d), t = a·(d/g) ± c·(b/g), and g2 = gcd(t, g), the result
    // is (t/g2) / ((b/g)·(d/g2)), already reduced.
    fn sum_quickly(self, other: Self, subtract: bool) -> Result<Self, RationalError> {
        let (a, b) = (self.numerator, self.denominator);
        let c = if subtract {
            other.numerator.checked_neg()
        } else {
            Some(other.numerator)
        };
        let d = other.denominator;
        // The common case stays in 64 bits, where division is one instruction; anything that would overflow
        // goes the 128-bit way.
        let narrow = c.and_then(|c| {
            if b == d {
                let numerator = a.checked_add(c)?;
                if numerator == 0 {
                    return Some(Self::from_int(0));
                }
                let divisor = gcd_euclid(numerator.unsigned_abs(), b.unsigned_abs()).cast_signed();
                return Some(Self {
                    numerator: numerator / divisor,
                    denominator: b / divisor,
                });
            }
            let common = gcd_euclid(b.unsigned_abs(), d.unsigned_abs()).cast_signed();
            let combined = a
                .checked_mul(d / common)?
                .checked_add(c.checked_mul(b / common)?)?;
            if combined == 0 {
                return Some(Self::from_int(0));
            }
            let common_again =
                gcd_euclid(combined.unsigned_abs(), common.unsigned_abs()).cast_signed();
            Some(Self {
                numerator: combined / common_again,
                denominator: (b / common).checked_mul(d / common_again)?,
            })
        });
        match narrow {
            Some(result) => Ok(result),
            None if subtract => self.checked_sub(other),
            None => self.checked_add(other),
        }
    }

    /// `self * other`, as [`Rational::checked_mul`], by a quicker path: cancelling across before multiplying
    /// leaves the product already reduced, so a product that overflows 64 bits is too big.
    ///
    /// # Errors
    ///
    /// `TooBig` when the result's parts don't fit in 64 bits.
    pub fn multiply_quickly(self, other: Self) -> Result<Self, RationalError> {
        if self.numerator == 0 || other.numerator == 0 {
            return Ok(Self::from_int(0));
        }
        let g1 = gcd_euclid(
            self.numerator.unsigned_abs(),
            other.denominator.unsigned_abs(),
        )
        .cast_signed();
        let g2 = gcd_euclid(
            other.numerator.unsigned_abs(),
            self.denominator.unsigned_abs(),
        )
        .cast_signed();
        let narrow = (|| {
            Some(Self {
                numerator: (self.numerator / g1).checked_mul(other.numerator / g2)?,
                denominator: (self.denominator / g2).checked_mul(other.denominator / g1)?,
            })
        })();
        // i64::MIN / -1 can't happen: g1 and g2 are positive.
        narrow.map_or_else(|| self.checked_mul(other), Ok)
    }

    /// `self / other`, as [`Rational::checked_div`], by a quicker path: multiplying by `other` turned over.
    ///
    /// # Errors
    ///
    /// `DivisionByZero` when `other` is zero; `TooBig` when the result's parts don't fit in 64 bits.
    pub fn divide_quickly(self, other: Self) -> Result<Self, RationalError> {
        if other.numerator == 0 {
            return Err(RationalError::DivisionByZero);
        }
        if self.numerator == 0 {
            return Ok(Self::from_int(0));
        }
        // a/b ÷ c/d = (a·d)/(b·c); cancel a with c and b with d first.
        let g1 = gcd_euclid(
            self.numerator.unsigned_abs(),
            other.numerator.unsigned_abs(),
        )
        .cast_signed();
        let g2 = gcd_euclid(
            self.denominator.unsigned_abs(),
            other.denominator.unsigned_abs(),
        )
        .cast_signed();
        let narrow = (|| {
            let numerator = (self.numerator / g1).checked_mul(other.denominator / g2)?;
            let denominator = (self.denominator / g2).checked_mul(other.numerator / g1)?;
            if denominator < 0 {
                return Some(Self {
                    numerator: numerator.checked_neg()?,
                    denominator: denominator.checked_neg()?,
                });
            }
            Some(Self {
                numerator,
                denominator,
            })
        })();
        narrow.map_or_else(|| self.checked_div(other), Ok)
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

/// Euclid's GCD on 64 bits: one-instruction divisions, few of them for the small numbers most programs use.
fn gcd_euclid(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
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

    /// The quick paths must give exactly what the simple ones give, extremes included.
    #[test]
    fn quick_paths_match_the_simple_ones() {
        let mut state = 0x1234_5678_9ABC_DEF1_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let edges = [
            0,
            1,
            -1,
            2,
            3,
            10,
            100,
            i64::MAX,
            i64::MIN,
            i64::MAX - 1,
            i64::MIN + 1,
            1 << 32,
            1 << 62,
        ];
        let pick = move |next: &mut dyn FnMut() -> u64| -> i64 {
            match next() % 4 {
                0 => edges[usize::try_from(next() % edges.len() as u64).unwrap()],
                1 => i64::try_from(next() % 1000).unwrap() - 500,
                2 => i64::try_from(next() % 100_000_000).unwrap() - 50_000_000,
                _ => next().cast_signed(),
            }
        };
        for _ in 0..2_000_000 {
            let numerator_a = pick(&mut next);
            let numerator_b = pick(&mut next);
            let denominator_a = pick(&mut next);
            let denominator_b = if next() % 3 == 0 {
                denominator_a
            } else {
                pick(&mut next)
            };
            let (Ok(x), Ok(y)) = (
                Rational::new(numerator_a.into(), denominator_a.into()),
                Rational::new(numerator_b.into(), denominator_b.into()),
            ) else {
                continue;
            };
            assert_eq!(x.add_quickly(y), x.checked_add(y), "{x} + {y}");
            assert_eq!(x.subtract_quickly(y), x.checked_sub(y), "{x} - {y}");
            assert_eq!(x.multiply_quickly(y), x.checked_mul(y), "{x} * {y}");
            assert_eq!(x.divide_quickly(y), x.checked_div(y), "{x} / {y}");
        }
    }

    #[test]
    fn order() {
        assert!(r(1, 3) < r(1, 2));
        assert!(r(-1, 2) < r(1, 3));
    }
}
