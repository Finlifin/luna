; Development entry adapter. The caller provides 41 bytes: 39 digits, sign, NUL.
; Magnitude is unsigned so the minimum signed i128 does not overflow.
define internal ptr @flurry_decimal_i128(i128 %value, i1 %signed, ptr %buffer) {
entry:
  %below_zero = icmp slt i128 %value, 0
  %negative = and i1 %signed, %below_zero
  %negated = sub i128 0, %value
  %magnitude = select i1 %negative, i128 %negated, i128 %value
  %end = getelementptr i8, ptr %buffer, i32 40
  store i8 0, ptr %end
  br label %digit

digit:
  %remaining = phi i128 [ %magnitude, %entry ], [ %quotient, %digit ]
  %cursor = phi ptr [ %end, %entry ], [ %previous, %digit ]
  %quotient = udiv i128 %remaining, 10
  %remainder = urem i128 %remaining, 10
  %byte = trunc i128 %remainder to i8
  %ascii = add i8 %byte, 48
  %previous = getelementptr i8, ptr %cursor, i32 -1
  store i8 %ascii, ptr %previous
  %more = icmp ne i128 %quotient, 0
  br i1 %more, label %digit, label %sign

sign:
  br i1 %negative, label %minus, label %done

minus:
  %start = getelementptr i8, ptr %previous, i32 -1
  store i8 45, ptr %start
  br label %done

done:
  %text = phi ptr [ %previous, %sign ], [ %start, %minus ]
  ret ptr %text
}
