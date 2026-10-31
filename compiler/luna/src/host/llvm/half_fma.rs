//! LLVM 21 promotes half fma to f32, which can double-round. Preserve the
//! exact sum's side of a target midpoint using TwoSum and round-to-odd.
use super::*;

impl Emitter<'_> {
    pub(super) fn half_fma(&mut self, inputs: &[Value]) -> Value {
        let ty = inputs[0].ty.clone();
        let mut wide = Vec::with_capacity(3);
        for input in inputs {
            let input = if ty == Ty::BFloat16 {
                self.widen_bfloat(input)
            } else {
                input.clone()
            };
            wide.push(self.instruction(
                Ty::Float64,
                format!("fpext {} {} to double", input.ty.llvm(), input.text),
            ));
        }
        // A product of half operands is exact in f64 (at most 22 significant
        // bits, exponents within [-266, 256]). No finite intermediate here
        // can overflow or underflow f64, including TwoSum's residual.
        let product = self.double_binary("fmul", &wide[0], &wide[1]);
        let sum = self.double_binary("fadd", &product, &wide[2]);
        let bv = self.double_binary("fsub", &sum, &product);
        let av = self.double_binary("fsub", &sum, &bv);
        let br = self.double_binary("fsub", &wide[2], &bv);
        let ar = self.double_binary("fsub", &product, &av);
        let residual = self.double_binary("fadd", &ar, &br);

        // An exact target midpoint has a zero low bit in the wide encoding.
        // If the wide sum rounded there, nudge it toward the exact residual.
        // Keeping odd sums and exact sums unchanged yields round-to-odd;
        // the final fptrunc then performs the sole target rounding.
        let bits = self.instruction(
            Ty::Int(64, false),
            format!("bitcast double {} to i64", sum.text),
        );
        let parity = self.instruction(Ty::Int(64, false), format!("and i64 {}, 1", bits.text));
        let even = self.instruction(Ty::Bool, format!("icmp eq i64 {}, 0", parity.text));
        let inexact = self.instruction(Ty::Bool, format!("fcmp one double {}, 0.0", residual.text));
        let correct = self.instruction(Ty::Bool, format!("and i1 {}, {}", even.text, inexact.text));
        let negative = self.instruction(Ty::Bool, format!("icmp slt i64 {}, 0", bits.text));
        let above = self.instruction(Ty::Bool, format!("fcmp ogt double {}, 0.0", residual.text));
        let increase_bits = self.instruction(
            Ty::Bool,
            format!("xor i1 {}, {}", negative.text, above.text),
        );
        let delta = self.instruction(
            Ty::Int(64, false),
            format!("select i1 {}, i64 1, i64 -1", increase_bits.text),
        );
        let adjacent = self.instruction(
            Ty::Int(64, false),
            format!("add i64 {}, {}", bits.text, delta.text),
        );
        let adjusted = self.instruction(
            Ty::Int(64, false),
            format!(
                "select i1 {}, i64 {}, i64 {}",
                correct.text, adjacent.text, bits.text
            ),
        );
        let value = self.instruction(
            Ty::Float64,
            format!("bitcast i64 {} to double", adjusted.text),
        );
        let result = if ty == Ty::BFloat16 {
            // Convert directly from f64: routing through f32 would discard
            // the residual information preserved above. bfloat is transient;
            // ordinary bf16 transport remains i16.
            let narrow = self.fresh();
            self.line(format!(
                "%{narrow} = fptrunc double {} to bfloat",
                value.text
            ));
            self.instruction(ty.clone(), format!("bitcast bfloat %{narrow} to i16"))
        } else {
            self.instruction(ty.clone(), format!("fptrunc double {} to half", value.text))
        };
        let nan = self.instruction(
            Ty::Bool,
            format!("fcmp uno double {}, {}", sum.text, sum.text),
        );
        let canonical = if ty == Ty::BFloat16 {
            "32704"
        } else {
            "bitcast (i16 32256 to half)"
        };
        self.instruction(
            ty.clone(),
            format!(
                "select i1 {}, {} {canonical}, {} {}",
                nan.text,
                ty.llvm(),
                ty.llvm(),
                result.text
            ),
        )
    }

    fn double_binary(&mut self, op: &str, a: &Value, b: &Value) -> Value {
        self.instruction(Ty::Float64, format!("{op} double {}, {}", a.text, b.text))
    }
}
