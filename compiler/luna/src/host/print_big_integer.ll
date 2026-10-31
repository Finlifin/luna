; Decimal conversion using 32-bit limbs, independent of target byte order.
; The caller supplies a nonempty magnitude array and an exclusive buffer end.
define internal ptr @flurry_decimal_limbs(ptr %limbs, i32 %count, ptr %end, i1 %negative) {
entry:
  store i8 0, ptr %end
  br label %digit
digit:
  %cursor = phi ptr [ %end, %entry ], [ %previous, %emit ]
  br label %word
word:
  %index = phi i32 [ %count, %digit ], [ %slot, %word ]
  %carry = phi i64 [ 0, %digit ], [ %remainder, %word ]
  %any = phi i32 [ 0, %digit ], [ %combined, %word ]
  %slot = sub i32 %index, 1
  %address = getelementptr i32, ptr %limbs, i32 %slot
  %part = load i32, ptr %address
  %low = zext i32 %part to i64
  %high = shl i64 %carry, 32
  %number = or i64 %high, %low
  %quotient = udiv i64 %number, 10
  %remainder = urem i64 %number, 10
  %q = trunc i64 %quotient to i32
  store i32 %q, ptr %address
  %combined = or i32 %any, %q
  %morewords = icmp ne i32 %slot, 0
  br i1 %morewords, label %word, label %emit
emit:
  %byte = trunc i64 %remainder to i8
  %ascii = add i8 %byte, 48
  %previous = getelementptr i8, ptr %cursor, i32 -1
  store i8 %ascii, ptr %previous
  %more = icmp ne i32 %combined, 0
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
