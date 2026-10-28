//! bf16 uses i16 for storage and internal calls. LLVM 21 can quiet signaling
//! NaNs while legalizing even a bfloat identity function's arguments/return.
//! Only arithmetic crosses into LLVM's bfloat representation.
use super::*;

impl Emitter<'_> {
    fn widen_bfloat(&mut self, value: &Value) -> Value {
        let bits = self.instruction(
            Ty::Int(32, false),
            format!("zext i16 {} to i32", value.text),
        );
        let bits = self.instruction(Ty::Int(32, false), format!("shl i32 {}, 16", bits.text));
        self.instruction(Ty::Float32, format!("bitcast i32 {} to float", bits.text))
    }

    // Round finite f32 values to nearest-even, preserving infinities and
    // quieting NaNs before rounding could turn their payload into infinity.
    fn round_bfloat(&mut self, value: &Value) -> Value {
        let bits = self.instruction(
            Ty::Int(32, false),
            format!("bitcast float {} to i32", value.text),
        );
        let high = self.instruction(Ty::Int(32, false), format!("lshr i32 {}, 16", bits.text));
        let parity = self.instruction(Ty::Int(32, false), format!("and i32 {}, 1", high.text));
        let bias = self.instruction(
            Ty::Int(32, false),
            format!("add i32 {}, 32767", parity.text),
        );
        let rounded = self.instruction(
            Ty::Int(32, false),
            format!("add i32 {}, {}", bits.text, bias.text),
        );
        let upper = self.instruction(Ty::Int(32, false), format!("lshr i32 {}, 16", rounded.text));
        let magnitude = self.instruction(
            Ty::Int(32, false),
            format!("and i32 {}, 2147483647", bits.text),
        );
        let nan = self.instruction(
            Ty::Bool,
            format!("icmp ugt i32 {}, 2139095040", magnitude.text),
        );
        let quiet = self.instruction(Ty::Int(32, false), format!("or i32 {}, 64", high.text));
        let result = self.instruction(
            Ty::Int(32, false),
            format!(
                "select i1 {}, i32 {}, i32 {}",
                nan.text, quiet.text, upper.text
            ),
        );
        self.instruction(Ty::BFloat16, format!("trunc i32 {} to i16", result.text))
    }

    pub(super) fn float_compare(&mut self, predicate: &str, a: &Value, b: &Value) -> Value {
        let (a, b) = if a.ty == Ty::BFloat16 {
            (self.widen_bfloat(a), self.widen_bfloat(b))
        } else {
            (a.clone(), b.clone())
        };
        self.instruction(
            Ty::Bool,
            format!("fcmp {predicate} {} {}, {}", a.ty.llvm(), a.text, b.text),
        )
    }

    pub(super) fn bfloat_arithmetic(&mut self, operation: &str, a: &Value, b: &Value) -> Value {
        let left = self.fresh();
        let right = self.fresh();
        let result = self.fresh();
        self.line(format!("%{left} = bitcast i16 {} to bfloat", a.text));
        self.line(format!("%{right} = bitcast i16 {} to bfloat", b.text));
        self.line(format!("%{result} = {operation} bfloat %{left}, %{right}"));
        let bits = self.instruction(Ty::BFloat16, format!("bitcast bfloat %{result} to i16"));
        let magnitude =
            self.instruction(Ty::Int(16, false), format!("and i16 {}, 32767", bits.text));
        let nan = self.instruction(Ty::Bool, format!("icmp ugt i16 {}, 32640", magnitude.text));
        self.instruction(
            Ty::BFloat16,
            format!("select i1 {}, i16 32704, i16 {}", nan.text, bits.text),
        )
    }

    fn select_bfloat(&mut self, condition: &Value, yes: &Value, no: &Value) -> Value {
        self.instruction(
            Ty::BFloat16,
            format!(
                "select i1 {}, i16 {}, i16 {}",
                condition.text, yes.text, no.text
            ),
        )
    }

    pub(super) fn bfloat_intrinsic(
        &mut self,
        op: nir::intrinsics::Operation,
        values: &[Value],
    ) -> Result<Value, String> {
        use nir::intrinsics::Operation as Op;
        let a = &values[0];
        if op == Op::AbsoluteValue {
            return Ok(self.instruction(Ty::BFloat16, format!("and i16 {}, 32767", a.text)));
        }
        if op == Op::SquareRoot {
            // bf16 midpoint squares need at most 18 significant bits. A
            // correctly rounded f32 sqrt cannot cross a bf16 midpoint for a
            // bf16 input; round the widened result to nearest-even explicitly.
            let wide = self.widen_bfloat(a);
            self.declarations
                .insert("declare float @llvm.sqrt.f32(float)".into());
            let root = self.instruction(
                Ty::Float32,
                format!("call float @llvm.sqrt.f32(float {})", wide.text),
            );
            return Ok(self.round_bfloat(&root));
        }
        if op == Op::ScaleByPowerOfTwo {
            // The f32 significand and subnormal range retain all bits that
            // could affect rounding a scaled bf16 input back to bf16.
            let wide = self.widen_bfloat(a);
            let exponent = values.get(1).ok_or("bf16 ldexp requires an exponent")?;
            self.declarations
                .insert("declare float @llvm.ldexp.f32.i32(float, i32)".into());
            let result = self.instruction(
                Ty::Float32,
                format!(
                    "call float @llvm.ldexp.f32.i32(float {}, i32 {})",
                    wide.text, exponent.text
                ),
            );
            return Ok(self.round_bfloat(&result));
        }
        if matches!(
            op,
            Op::Floor | Op::Ceiling | Op::Truncate | Op::RoundEven | Op::RoundAway
        ) {
            // Widening is exact, and every integral result of a bf16 value is
            // itself representable in bf16. Thus no second rounding is needed.
            let wide = self.widen_bfloat(a);
            let symbol = format!("llvm.{}.f32", op.name());
            self.declarations
                .insert(format!("declare float @{symbol}(float)"));
            let rounded = self.instruction(
                Ty::Float32,
                format!("call float @{symbol}(float {})", wide.text),
            );
            let bits = self.instruction(
                Ty::Int(32, false),
                format!("bitcast float {} to i32", rounded.text),
            );
            let upper = self.instruction(Ty::Int(32, false), format!("lshr i32 {}, 16", bits.text));
            return Ok(self.instruction(Ty::BFloat16, format!("trunc i32 {} to i16", upper.text)));
        }
        let b = values
            .get(1)
            .ok_or("bf16 intrinsic requires two operands")?;
        if op == Op::CopySign {
            let magnitude =
                self.instruction(Ty::Int(16, false), format!("and i16 {}, 32767", a.text));
            let sign = self.instruction(Ty::Int(16, false), format!("and i16 {}, 32768", b.text));
            return Ok(self.instruction(
                Ty::BFloat16,
                format!("or i16 {}, {}", magnitude.text, sign.text),
            ));
        }
        if !matches!(
            op,
            Op::Minimum | Op::Maximum | Op::MinimumNumber | Op::MaximumNumber
        ) {
            return Err("unsupported bf16 intrinsic".into());
        }
        let minimum = matches!(op, Op::Minimum | Op::MinimumNumber);
        let ordered = self.float_compare(if minimum { "olt" } else { "ogt" }, a, b);
        let unequal = self.select_bfloat(&ordered, a, b);
        let equal = self.float_compare("oeq", a, b);
        // Equal finite encodings are identical except for the sign of zero.
        let zeros = self.instruction(
            Ty::BFloat16,
            format!(
                "{} i16 {}, {}",
                if minimum { "or" } else { "and" },
                a.text,
                b.text
            ),
        );
        let numeric = self.select_bfloat(&equal, &zeros, &unequal);
        let a_nan = self.half_float_predicate(a.clone(), Op::IsNan)?;
        let b_nan = self.half_float_predicate(b.clone(), Op::IsNan)?;
        let a_quiet = self.instruction(Ty::BFloat16, format!("or i16 {}, 64", a.text));
        let b_quiet = self.instruction(Ty::BFloat16, format!("or i16 {}, 64", b.text));
        if matches!(op, Op::Minimum | Op::Maximum) {
            let right = self.select_bfloat(&b_nan, &b_quiet, &numeric);
            Ok(self.select_bfloat(&a_nan, &a_quiet, &right))
        } else {
            let left_nan = self.select_bfloat(&b_nan, &a_quiet, b);
            let right = self.select_bfloat(&b_nan, a, &numeric);
            Ok(self.select_bfloat(&a_nan, &left_nan, &right))
        }
    }
}
