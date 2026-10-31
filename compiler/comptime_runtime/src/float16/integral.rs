//! Integral rounding acts directly on the source format's significand.
use super::Float16Format;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegralRounding {
    Floor,
    Ceiling,
    Truncate,
    NearestEven,
    NearestAway,
}
impl fmt::Display for IntegralRounding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(integral-rounding {})",
            match self {
                Self::Floor => "floor",
                Self::Ceiling => "ceiling",
                Self::Truncate => "truncate",
                Self::NearestEven => "nearest-even",
                Self::NearestAway => "nearest-away",
            }
        )
    }
}

impl Float16Format {
    /// Return the integral value in the same format. Preserve zero signs and
    /// infinity, and quiet NaNs without discarding their sign or payload.
    /// Constant bounded bit operations require no allocation or host floats.
    pub fn round_integral(self, bits: u16, mode: IntegralRounding) -> u16 {
        use IntegralRounding::*;
        let magnitude = bits & 0x7fff;
        let sign = bits & 0x8000;
        let fraction = self.fraction_bits();
        if magnitude >= self.infinity() {
            return if magnitude == self.infinity() {
                bits
            } else {
                bits | (1 << (fraction - 1))
            };
        }
        let exponent = i32::from(magnitude >> fraction) - self.bias();
        if magnitude == 0 || exponent >= fraction as i32 {
            return bits;
        }
        let negative = sign != 0;
        if exponent < 0 {
            let half = ((self.bias() - 1) as u16) << fraction;
            let increment = match mode {
                Floor => negative,
                Ceiling => !negative,
                Truncate => false,
                NearestEven => magnitude > half,
                NearestAway => magnitude >= half,
            };
            return sign
                | if increment {
                    (self.bias() as u16) << fraction
                } else {
                    0
                };
        }
        let shift = fraction - exponent as u32;
        let unit = 1u16 << shift;
        let remainder = magnitude & (unit - 1);
        if remainder == 0 {
            return bits;
        }
        let integral = magnitude & !(unit - 1);
        let significand = (1u16 << fraction) | (magnitude & ((1u16 << fraction) - 1));
        let odd = (significand >> shift) & 1 != 0;
        let increment = match mode {
            Floor => negative,
            Ceiling => !negative,
            Truncate => false,
            NearestEven => remainder > unit / 2 || (remainder == unit / 2 && odd),
            NearestAway => remainder >= unit / 2,
        };
        sign | (integral + if increment { unit } else { 0 })
    }
}
