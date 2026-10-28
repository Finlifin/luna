; LLVM 21.1.8 x86_64-pc-linux-gnu, llc-21 -O0.
; Reproduce: llc-21 -O0 development/repros/bf16-signbit-llvm21.ll -o /tmp/bf16-sign.s
; Both functions must return true for bfloat bits 0x8000 (negative zero).
; Observed direct emits an unconditional zero result; widened reads the sign.
; The widened path was an initial signbit workaround. LLVM also converts
; bf16 identity arguments/returns (see identity below), quieting signaling NaNs.
; Luna now transports bf16 as i16 internally and crosses into bfloat only for
; arithmetic; sign extraction and magnitude classification use the raw bits.

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

; LLVM 21.1.8 x86_64 O0 inserts __truncsfbf2 into this identity function.
; Ordinary transport must not change a signaling NaN's payload/quiet bit.
define bfloat @identity(bfloat %value) {
  ret bfloat %value
}

define i16 @bit_identity(i16 %value) {
  ret i16 %value
}
