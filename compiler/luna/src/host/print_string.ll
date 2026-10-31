; Development S-expression output. Length-delimited UTF-8 includes embedded NUL.
@flurry_string_prefix = private constant [10 x i8] c"(string \22\00"
@flurry_hex = private constant [16 x i8] c"0123456789abcdef"
@flurry_symbol_prefix = private constant [10 x i8] c"(symbol \22\00"
define internal void @flurry_print_string({ ptr, i64 } %value) {
entry:
  call void @flurry_print_quoted_text({ ptr, i64 } %value, ptr @flurry_string_prefix)
  ret void
}
define internal void @flurry_print_symbol({ ptr, i64 } %value) {
entry:
  call void @flurry_print_quoted_text({ ptr, i64 } %value, ptr @flurry_symbol_prefix)
  ret void
}
define internal void @flurry_print_quoted_text({ ptr, i64 } %value, ptr %output_prefix) {
entry:
  call void @flurry_output_prefix(ptr %output_prefix)
  %data = extractvalue { ptr, i64 } %value, 0
  %length = extractvalue { ptr, i64 } %value, 1
  br label %test
test:
  %index = phi i64 [ 0, %entry ], [ %next, %advance ], [ %next2, %advance2 ]
  %done = icmp eq i64 %index, %length
  br i1 %done, label %end, label %read
read:
  %address = getelementptr i8, ptr %data, i64 %index
  %byte = load i8, ptr %address
  %char = zext i8 %byte to i32
  switch i32 %char, label %other [i32 34, label %slash i32 92, label %slash i32 10, label %newline i32 13, label %return i32 9, label %tab]
slash:
  call i32 @flurry_output_char(i32 92)
  call i32 @flurry_output_char(i32 %char)
  br label %advance
newline:
  call i32 @flurry_output_char(i32 92)
  call i32 @flurry_output_char(i32 110)
  br label %advance
return:
  call i32 @flurry_output_char(i32 92)
  call i32 @flurry_output_char(i32 114)
  br label %advance
tab:
  call i32 @flurry_output_char(i32 92)
  call i32 @flurry_output_char(i32 116)
  br label %advance
other:
  %low = icmp ult i32 %char, 32
  %del = icmp eq i32 %char, 127
  %control = or i1 %low, %del
  br i1 %control, label %escape, label %checkc1
checkc1:
  %prefix = icmp eq i32 %char, 194
  %following = add i64 %index, 1
  %available = icmp ult i64 %following, %length
  %candidate = and i1 %prefix, %available
  br i1 %candidate, label %c1, label %plain
c1:
  %cp = getelementptr i8, ptr %data, i64 %following
  %cb = load i8, ptr %cp
  %continuation = zext i8 %cb to i32
  %above = icmp uge i32 %continuation, 128
  %below = icmp ule i32 %continuation, 159
  %c1control = and i1 %above, %below
  br i1 %c1control, label %escape, label %plain
escape:
  %code = phi i32 [ %char, %other ], [ %continuation, %c1 ]
  %two = phi i1 [ false, %other ], [ true, %c1 ]
  call i32 @flurry_output_char(i32 92)
  call i32 @flurry_output_char(i32 117)
  call i32 @flurry_output_char(i32 48)
  call i32 @flurry_output_char(i32 48)
  %high = lshr i32 %code, 4
  %lowbits = and i32 %code, 15
  %hp = getelementptr [16 x i8], ptr @flurry_hex, i32 0, i32 %high
  %lp = getelementptr [16 x i8], ptr @flurry_hex, i32 0, i32 %lowbits
  %h = load i8, ptr %hp
  %l = load i8, ptr %lp
  %hi = zext i8 %h to i32
  %lo = zext i8 %l to i32
  call i32 @flurry_output_char(i32 %hi)
  call i32 @flurry_output_char(i32 %lo)
  br i1 %two, label %advance2, label %advance
plain:
  call i32 @flurry_output_char(i32 %char)
  br label %advance
advance2:
  %next2 = add i64 %index, 2
  br label %test
advance:
  %next = add i64 %index, 1
  br label %test
end:
  call i32 @flurry_output_char(i32 34)
  call i32 @flurry_output_char(i32 41)
  ret void
}
