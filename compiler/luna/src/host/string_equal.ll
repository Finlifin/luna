; Compare bytes, not storage addresses. Empty strings never dereference a pointer.
define internal i1 @flurry_string_equal({ ptr, i64 } %a, { ptr, i64 } %b) {
entry:
  %alen = extractvalue { ptr, i64 } %a, 1
  %blen = extractvalue { ptr, i64 } %b, 1
  %same = icmp eq i64 %alen, %blen
  br i1 %same, label %start, label %no
start:
  %ap = extractvalue { ptr, i64 } %a, 0
  %bp = extractvalue { ptr, i64 } %b, 0
  br label %test
test:
  %index = phi i64 [ 0, %start ], [ %next, %byte ]
  %end = icmp eq i64 %index, %alen
  br i1 %end, label %yes, label %byte
byte:
  %ax = getelementptr i8, ptr %ap, i64 %index
  %bx = getelementptr i8, ptr %bp, i64 %index
  %av = load i8, ptr %ax
  %bv = load i8, ptr %bx
  %equal = icmp eq i8 %av, %bv
  %next = add i64 %index, 1
  br i1 %equal, label %test, label %no
yes:
  ret i1 true
no:
  ret i1 false
}
