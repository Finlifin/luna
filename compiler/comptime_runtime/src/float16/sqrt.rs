//! Correctly rounded square roots from exact comparisons of squared midpoints.
use super::Float16Format;
use std::cmp::Ordering;

// Compare nonnegative dyadics without expanding large binary exponents.
fn compare(a: u64, ea: i32, b: u64, eb: i32) -> Ordering {
    if a == 0 || b == 0 {
        return a.cmp(&b);
    }
    let la = (64 - a.leading_zeros()) as i32;
    let lb = (64 - b.leading_zeros()) as i32;
    (la + ea).cmp(&(lb + eb)).then_with(|| {
        // Equal leading-bit positions imply a shift bounded by significand
        // widths (at most 24 bits here), regardless of the source exponent.
        let exponent = ea.min(eb);
        (u128::from(a) << (ea - exponent)).cmp(&(u128::from(b) << (eb - exponent)))
    })
}

impl Float16Format {
    /// Nearest-even square root, with signed zeros preserved. Negative nonzero
    /// inputs produce canonical NaN; NaN inputs retain sign/payload and quiet.
    /// At most 15 search steps, each using fixed-width integer operations.
    pub fn square_root(self, bits: u16) -> u16 {
        let magnitude = bits & 0x7fff;
        let quiet = 1 << (self.fraction_bits() - 1);
        if magnitude > self.infinity() {
            return bits | quiet;
        }
        if magnitude == 0 {
            return bits;
        }
        if bits & 0x8000 != 0 {
            return self.infinity() | quiet;
        }
        if magnitude == self.infinity() {
            return bits;
        }
        let (input, exponent) = self.parts(bits);
        let mut lower = 0;
        let mut upper = self.infinity();
        while lower < upper {
            let middle = lower + (upper - lower) / 2;
            let (significand, e) = self.parts(middle);
            // The midpoint above each finite encoding is (2*m+1)*2^(e-1),
            // including subnormals and transitions to a new binade.
            let midpoint = u64::from(2 * significand + 1);
            match compare(u64::from(input), exponent, midpoint * midpoint, 2 * e - 2) {
                Ordering::Less => upper = middle,
                Ordering::Equal if middle & 1 == 0 => upper = middle,
                _ => lower = middle + 1,
            }
        }
        lower
    }
}
