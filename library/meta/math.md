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

The arithmetic scalar domain matches `builtin.math`: IEEE `f32`/`f64`,
`f16`/`bf16` for the operations detailed below, fixed-width integers, and
`usize`/`isize` with an explicitly selected compilation target.
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
`abs`, `copysign`, `minimum`, `maximum`, `minimumnum`, and `maximumnum`
also accept both half formats. Sign operations preserve all non-sign bits,
including signaling NaN payloads. Extrema order negative zero below positive
zero; propagating variants return NaN if either operand is NaN, while number
variants choose the number when exactly one operand is NaN.

`floor`, `ceil`, `trunc`, `roundeven`, and `round` also accept both half
formats and return the original floating type. Ties go to even for
`roundeven` and away from zero for `round`. Zero signs and infinities are
preserved; NaNs remain NaNs. The VM rounds the source significand directly
with bounded integer operations.

`sqrt` also accepts both formats and rounds to nearest-even. It preserves
negative zero and positive infinity, and produces NaN for negative nonzero
inputs and NaNs. NaN payloads are not part of this interface contract. The
VM compares the input with exact squared midpoints in a bounded binary
search; it does not first round through a host floating type.

`ldexp` accepts both half formats with an `i32` exponent, including its
minimum and maximum values. It rounds subnormal results to nearest-even,
returns signed infinity on overflow and signed zero on complete underflow,
and preserves zeros and infinities for any exponent. NaNs remain NaNs.
The VM adjusts exponents with bounded integer work and rounds only at the
final subnormal boundary.

`fma` also accepts f16/bf16 and rounds the exact product plus addend only
once. The VM uses budgeted exact integer arithmetic, including cases where
the intermediate product would overflow the target format but the fused
result is finite. Opposite infinite terms and zero times infinity produce
NaN. Signed zeros follow nearest-even fused arithmetic; NaN payloads are
not an interface promise. Other floating math wrappers do not yet admit
half precision.

Public half-format coverage is exercised through both `sysroot-meta-math`
and `native-math` in `development/library_check.py`. These paths cover the
supported arithmetic, rounding, selection and classification wrappers,
including signed zeros, smallest subnormals, overflow, invalid square roots,
and a binary16 fused result whose unfused product would overflow. The
`library_meta_math` source tests additionally check exact result-format bits;
this coverage does not extend the admitted domain of transcendental or
`powi` wrappers to half formats.
