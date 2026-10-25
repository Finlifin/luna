; Development result-output status, separate from language errors and effects.
; Track immediate writes as well as the final buffered flush: a successful
; later write must not erase an earlier failure. Only the entry printer uses it.
@flurry_output_failed = internal global i1 false
declare i32 @putchar(i32)
declare i32 @fflush(ptr)
define internal void @flurry_output_record(i32 %result) {
entry:
  %failed = icmp slt i32 %result, 0
  %previous = load i1, ptr @flurry_output_failed
  %any = or i1 %previous, %failed
  store i1 %any, ptr @flurry_output_failed
  ret void
}
define internal i32 @flurry_output_char(i32 %char) {
entry:
  %result = call i32 @putchar(i32 %char)
  call void @flurry_output_record(i32 %result)
  ret i32 %result
}
define internal void @flurry_output_prefix(ptr %format) {
entry:
  %result = call i32 (ptr, ...) @printf(ptr %format)
  call void @flurry_output_record(i32 %result)
  ret void
}
define internal i32 @flurry_output_finish() {
entry:
  ; A null stream flushes buffered output without assuming FILE layout or
  ; a platform-specific exported stdout symbol.
  %result = call i32 @fflush(ptr null)
  call void @flurry_output_record(i32 %result)
  %failed = load i1, ptr @flurry_output_failed
  %status = zext i1 %failed to i32
  ret i32 %status
}
