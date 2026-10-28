; LLVM 21.1.8 x86_64-pc-linux-gnu, llc-21 -O0.
; Reproduce: llc-21 -O0 development/repros/bf16-signbit-llvm21.ll -o /tmp/bf16-sign.s
; Both functions must return true for bfloat bits 0x8000 (negative zero).
; Observed direct emits an unconditional zero result; widened reads the sign.
; Luna uses exact widening only for bf16 signbit. Magnitude classification
; must retain the original format because widening changes subnormal classes.

define i1 @direct(bfloat %value) {
  %slot = alloca bfloat
  store bfloat %value, ptr %slot
  %loaded = load bfloat, ptr %slot
  %raw = bitcast bfloat %loaded to i16
  %negative = icmp slt i16 %raw, 0
  ret i1 %negative
}
define i1 @widened(bfloat %value) {
  %slot = alloca bfloat
  store bfloat %value, ptr %slot
  %loaded = load bfloat, ptr %slot
  %wide = fpext bfloat %loaded to float
  %raw = bitcast float %wide to i32
  %negative = icmp slt i32 %raw, 0
  ret i1 %negative
}
