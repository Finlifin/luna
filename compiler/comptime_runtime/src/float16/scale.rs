//! Power-of-two scaling with bounded shifts and one final subnormal rounding.
use super::Float16Format;

impl Float16Format {
    /// Scale by 2^power, nearest-even, without constructing that power. The
    /// entire i32 exponent range takes constant work, including extreme inputs.
    pub fn scale_pow2(self, bits: u16, power: i32) -> u16 {
        let magnitude = bits & 0x7fff;
        let sign = bits & 0x8000;
        if magnitude >= self.infinity() {
            return if magnitude == self.infinity() {
                bits
            } else {
                bits | (1 << (self.fraction_bits() - 1))
            };
        }
        if magnitude == 0 {
            return bits;
        }
        let (mantissa, exponent) = self.parts(magnitude);
        let exponent = i64::from(exponent) + i64::from(power);
        let leading = 31 - mantissa.leading_zeros();
        let normalized = exponent + i64::from(leading);
        let fraction = self.fraction_bits();
        let bias = i64::from(self.bias());
        if normalized > bias {
            return sign | self.infinity();
        }
        if normalized >= 1 - bias {
            let significand = mantissa << (fraction - leading);
            return sign
                | (((normalized + bias) as u16) << fraction)
                | ((significand as u16) & ((1 << fraction) - 1));
        }
        let shift = 1 - bias - i64::from(fraction) - exponent;
        if shift <= 0 {
            // Still subnormal, so the exact left shift cannot exceed the
            // fraction width. No rounding has occurred on this path.
            return sign | (mantissa << (-shift)) as u16;
        }
        if shift > i64::from(leading) + 1 {
            return sign;
        }
        let quotient = mantissa >> shift;
        let remainder = mantissa & ((1 << shift) - 1);
        let halfway = 1 << (shift - 1);
        let increment = remainder > halfway || (remainder == halfway && quotient & 1 != 0);
        // A rounded subnormal can carry into the minimum normal encoding.
        sign | (quotient + u32::from(increment)) as u16
    }
}
