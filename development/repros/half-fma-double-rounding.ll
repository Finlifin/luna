; LLVM 21.1.8, x86_64: direct half fma is promoted to fmaf and narrowed.
; (1 + 2^-10) * 1.5 - 2^-24 must round once to bits 0x3e01.
; This diagnostic prints result=3e02 at O0 on that backend.
; Flurry's half_fma lowering retains the residual instead; its frontend_cli
; regression checks both f16 and bf16 at O0/O3.
;
; Reproduce from the repository root:
; llc-21 -O0 -filetype=obj development/repros/half-fma-double-rounding.ll -o /tmp/half-fma.o
; cc -no-pie /tmp/half-fma.o -lm -o /tmp/half-fma
; /tmp/half-fma

@format = private constant [13 x i8] c"result=%04x\0A\00"
declare i32 @printf(ptr, ...)
declare half @llvm.fma.f16(half, half, half)

define half @half_fma(half %a, half %b, half %c) {
  %result = call half @llvm.fma.f16(half %a, half %b, half %c)
  ret half %result
}

define i32 @main() {
  %value = call half @half_fma(half bitcast (i16 15361 to half), half bitcast (i16 15872 to half), half bitcast (i16 32769 to half))
  %bits = bitcast half %value to i16
  %wide = zext i16 %bits to i32
  %written = call i32 (ptr, ...) @printf(ptr @format, i32 %wide)
  ret i32 0
}
