# meta.math

`builtin.math` supplies runtime operations; `meta.math` supplies their
`pure comptime fn` counterparts. Both use the public primitive type aliases.
The comptime module is explicitly qualified as `meta.math`, so importing the
meta prelude does not replace the default runtime `math` module.

```flurry
pure comptime fn table_scale() -> f64 {
    meta.math.ldexp<f64>(meta.math.sqrt<f64>(81.0), 2)
}

const scale = table_scale();

fn has_expected_scale() -> bool {
    inline if scale == 36.0 {true} else {false}
}

pure comptime fn reversed_mask() -> u8 {
    meta.math.bit_reverse<u8>(3)
}
```

The arithmetic scalar domain matches `builtin.math`: IEEE `f32`/`f64`, fixed-width
integers, and `usize`/`isize` with an explicitly selected compilation target.
It does not convert the arbitrary-precision `Integer` or decimal `Float` to a
machine scalar implicitly. Extending those domains requires their own numeric
contracts and implementations; they are not implemented by this module yet.

| Domain | Operations |
| --- | --- |
| Floating point | `fma`, `sqrt`, `copysign`, `floor`, `ceil`, `trunc`, `roundeven`, `round`, `abs`, `sin`, `cos`, `exp`, `exp2`, `log`, `log2`, `log10`, `pow`, `powi`, `ldexp` |
| Floating selection | `minimum`, `maximum`, `minimumnum`, `maximumnum` |
| Floating classification | `isnan`, `isinf`, `isfinite`, `isnormal`, `issubnormal`, `signbit` |
| Unsigned bits | `bit_reverse`, `count_ones`, `leading_zeros`, `trailing_zeros`, `byte_swap`, `funnel_shift_left`, `funnel_shift_right`, `rotate_left`, `rotate_right` |
| Integers | `saturating_add`, `saturating_sub`, `min`, `max`, `bit_and`, `bit_or`, `bit_xor`; signed integers also support `wrapping_abs` |

`byte_swap` requires an even number of bytes. Zero leading/trailing counts
return the selected width. Shift amounts are reduced modulo that width.
Saturation clamps to the selected integer range; `wrapping_abs(INT_MIN)`
returns `INT_MIN`.

`fma` performs one rounding. `roundeven` resolves ties to even and `round`
resolves ties away from zero. Floating minimum/maximum propagate NaN and order
negative zero below positive zero; the `num` forms select a numeric operand
when only one operand is NaN. `ldexp` scales without first constructing a
potentially overflowing or underflowing power of two. Transcendental operations
currently use the compiler host's scalar math implementation; no cross-host
bit-for-bit reproducibility or equality with LLVM's transcendental results is
promised by this implementation.

The sysroot's `comptime-intrinsic.*` bindings identify the VM operations by
source declaration identity. They are separate from `host-intrinsic.*` bindings.
An unbound or runtime tag cannot execute at comptime, and a comptime tag cannot
remain in runtime code. Ordinary arguments are evaluated once in source order.

Explicit package-set drivers can supply each binding with
`--comptime-intrinsic sqrt=meta.numeric.scalar_math.target.Scalar.sqrt` (and the
corresponding flag for every other operation). They must also provide meta
with a `builtin` dependency. Sysroot users receive these bindings automatically.

The six floating classification functions also accept `f16` and `bf16`.
They classify the original representation: a half subnormal stays subnormal
for `issubnormal`, even though widening it could produce a normal value.
`signbit` observes negative zero and signed NaNs without changing the payload.
Other floating math wrappers do not yet admit half precision.
