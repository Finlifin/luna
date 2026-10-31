//! Exact product and sum followed by a single target-format rounding.
use super::{Float16Format, arithmetic::scale};
use crate::{Budget, Error, Integer};

impl Float16Format {
    pub fn fused_multiply_add(
        self,
        a: u16,
        b: u16,
        c: u16,
        budget: &mut Budget,
    ) -> Result<u16, Error> {
        budget.consume(1)?;
        let infinity = self.infinity();
        let nan = infinity | (1 << (self.fraction_bits() - 1));
        let x = a & 0x7fff;
        let y = b & 0x7fff;
        let z = c & 0x7fff;
        if x > infinity || y > infinity || z > infinity {
            return Ok(nan);
        }
        let product_sign = (a ^ b) & 0x8000;
        if (x == infinity && y == 0) || (y == infinity && x == 0) {
            return Ok(nan);
        }
        if x == infinity || y == infinity {
            return Ok(if z == infinity && product_sign != c & 0x8000 {
                nan
            } else {
                product_sign | infinity
            });
        }
        if z == infinity {
            return Ok(c);
        }
        let (mx, ex) = self.parts(x);
        let (my, ey) = self.parts(y);
        let (mz, ez) = self.parts(z);
        let common = (ex + ey).min(ez);
        let mut product = scale(
            &Integer::from_u128(u128::from(mx) * u128::from(my)),
            (ex + ey - common) as u32,
            budget,
        )?;
        let mut addend = scale(&Integer::from_u128(mz.into()), (ez - common) as u32, budget)?;
        if product_sign != 0 {
            product = product.negate(budget)?;
        }
        if c & 0x8000 != 0 {
            addend = addend.negate(budget)?;
        }
        let sum = product.add(&addend, budget)?;
        if sum.is_zero() {
            return Ok(product_sign & c & 0x8000);
        }
        let negative = sum.is_negative();
        let magnitude = if negative { sum.negate(budget)? } else { sum };
        self.round_ratio(
            &magnitude,
            1,
            common,
            if negative { 0x8000 } else { 0 },
            budget,
        )
    }
}
