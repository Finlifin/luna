; Linux x86_64 libc ABI adapter. No readonly/readnone/speculatable attributes:
; every call can observe or change process state. No input pointer escapes.
declare ptr @__errno_location()
declare i64 @write(i32, ptr, i64)
declare i64 @read(i32, ptr, i64)
declare i32 @open(ptr, i32, ...)
declare i32 @close(i32)
declare i32 @unlink(ptr)
declare i32 @getpid()
declare ptr @memchr(ptr, i32, i64)
declare i32 @posix_memalign(ptr, i64, i64)
declare void @free(ptr)

define internal i64 @flurry_linux_error() {
entry:
  %pointer = call ptr @__errno_location()
  %code = load i32, ptr %pointer
  %wide = zext i32 %code to i64
  %negative = sub i64 0, %wide
  ret i64 %negative
}

define internal i64 @flurry_linux_write(i32 %fd, { ptr, i64 } %text, i64 %offset) {
entry:
  %data = extractvalue { ptr, i64 } %text, 0
  %length = extractvalue { ptr, i64 } %text, 1
  %valid = icmp ule i64 %offset, %length
  br i1 %valid, label %bounded, label %invalid
invalid:
  ret i64 -22
bounded:
  %remaining = sub i64 %length, %offset
  ; Linux caps one transfer at MAX_RW_COUNT. Avoid size_t/ssize_t overflow.
  %large = icmp ugt i64 %remaining, 2147479552
  %count = select i1 %large, i64 2147479552, i64 %remaining
  %start = getelementptr i8, ptr %data, i64 %offset
  %result = call i64 @write(i32 %fd, ptr %start, i64 %count)
  %failed = icmp slt i64 %result, 0
  br i1 %failed, label %error, label %done
error:
  %reason = call i64 @flurry_linux_error()
  ret i64 %reason
done:
  ret i64 %result
}

define internal i64 @flurry_linux_write_byte(i32 %fd, i8 %byte) {
entry:
  %storage = alloca i8
  store i8 %byte, ptr %storage
  %result = call i64 @write(i32 %fd, ptr %storage, i64 1)
  %failed = icmp slt i64 %result, 0
  br i1 %failed, label %error, label %done
error:
  %reason = call i64 @flurry_linux_error()
  ret i64 %reason
done:
  ret i64 %result
}

; 0..255 is a byte, 256 is EOF, negative values are -errno.
define internal i64 @flurry_linux_read_byte(i32 %fd) {
entry:
  %storage = alloca i8
  %result = call i64 @read(i32 %fd, ptr %storage, i64 1)
  %failed = icmp slt i64 %result, 0
  br i1 %failed, label %error, label %check
error:
  %reason = call i64 @flurry_linux_error()
  ret i64 %reason
check:
  %eof = icmp eq i64 %result, 0
  br i1 %eof, label %end, label %byte
end:
  ret i64 256
byte:
  %value = load i8, ptr %storage
  %wide = zext i8 %value to i64
  ret i64 %wide
}

; Linux paths are limited to 4096 bytes including the terminator. Reject
; embedded NUL rather than silently operating on a different path.
define internal i64 @flurry_linux_path({ ptr, i64 } %path, i32 %flags, i32 %mode, i1 %remove) {
entry:
  %data = extractvalue { ptr, i64 } %path, 0
  %length = extractvalue { ptr, i64 } %path, 1
  %long = icmp uge i64 %length, 4096
  br i1 %long, label %too_long, label %check
 too_long:
  ret i64 -36
check:
  %nul = call ptr @memchr(ptr %data, i32 0, i64 %length)
  %contains = icmp ne ptr %nul, null
  br i1 %contains, label %invalid, label %copy
invalid:
  ret i64 -22
copy:
  %buffer = alloca [4096 x i8]
  call void @llvm.memcpy.p0.p0.i64(ptr %buffer, ptr %data, i64 %length, i1 false)
  %end = getelementptr i8, ptr %buffer, i64 %length
  store i8 0, ptr %end
  br i1 %remove, label %unlink, label %open
unlink:
  %removed = call i32 @unlink(ptr %buffer)
  br label %join
open:
  %opened = call i32 (ptr, i32, ...) @open(ptr %buffer, i32 %flags, i32 %mode)
  br label %join
join:
  %result = phi i32 [ %removed, %unlink ], [ %opened, %open ]
  %failed = icmp slt i32 %result, 0
  br i1 %failed, label %error, label %done
error:
  %reason = call i64 @flurry_linux_error()
  ret i64 %reason
done:
  %wide = sext i32 %result to i64
  ret i64 %wide
}
define internal i64 @flurry_linux_open({ ptr, i64 } %path, i32 %flags, i32 %mode) {
entry:
  %result = call i64 @flurry_linux_path({ ptr, i64 } %path, i32 %flags, i32 %mode, i1 false)
  ret i64 %result
}
define internal i64 @flurry_linux_unlink({ ptr, i64 } %path) {
entry:
  %result = call i64 @flurry_linux_path({ ptr, i64 } %path, i32 0, i32 0, i1 true)
  ret i64 %result
}
define internal i64 @flurry_linux_close(i32 %fd) {
entry:
  ; Never retry close: Linux releases the descriptor even on EINTR.
  %result = call i32 @close(i32 %fd)
  %failed = icmp slt i32 %result, 0
  br i1 %failed, label %error, label %done
error:
  %reason = call i64 @flurry_linux_error()
  ret i64 %reason
done:
  ret i64 0
}
define internal i64 @flurry_linux_getpid() {
entry:
  %pid = call i32 @getpid()
  %wide = zext i32 %pid to i64
  ret i64 %wide
}
define internal i64 @flurry_linux_alloc(i64 %size, i64 %align) {
entry:
  %zero_size = icmp eq i64 %size, 0
  %zero_align = icmp eq i64 %align, 0
  %bad_pair = or i1 %zero_size, %zero_align
  br i1 %bad_pair, label %fail, label %check_pow2
check_pow2:
  %align_m1 = sub i64 %align, 1
  %rem = and i64 %align, %align_m1
  %not_pow2 = icmp ne i64 %rem, 0
  br i1 %not_pow2, label %fail, label %clamp_align
clamp_align:
  %small = icmp ult i64 %align, 8
  %eff_align = select i1 %small, i64 8, i64 %align
  %slot = alloca ptr
  store ptr null, ptr %slot
  %rc = call i32 @posix_memalign(ptr %slot, i64 %eff_align, i64 %size)
  %ok = icmp eq i32 %rc, 0
  br i1 %ok, label %success, label %fail
success:
  %raw = load ptr, ptr %slot
  %addr = ptrtoint ptr %raw to i64
  ret i64 %addr
fail:
  ret i64 0
}
define internal void @flurry_linux_dealloc(i64 %addr, i64 %_size, i64 %_align) {
entry:
  %is_null = icmp eq i64 %addr, 0
  br i1 %is_null, label %done, label %release
release:
  %raw = inttoptr i64 %addr to ptr
  call void @free(ptr %raw)
  br label %done
done:
  ret void
}
define internal i64 @flurry_linux_write_raw(i32 %fd, i64 %addr, i64 %len) {
entry:
  %empty = icmp eq i64 %len, 0
  br i1 %empty, label %done_zero, label %check_ptr
done_zero:
  ret i64 0
check_ptr:
  %is_null = icmp eq i64 %addr, 0
  br i1 %is_null, label %fault, label %transfer
fault:
  ret i64 -14
transfer:
  %large = icmp ugt i64 %len, 2147479552
  %count = select i1 %large, i64 2147479552, i64 %len
  %raw = inttoptr i64 %addr to ptr
  %result = call i64 @write(i32 %fd, ptr %raw, i64 %count)
  %failed = icmp slt i64 %result, 0
  br i1 %failed, label %error, label %done
error:
  %reason = call i64 @flurry_linux_error()
  ret i64 %reason
done:
  ret i64 %result
}
define internal i64 @flurry_linux_read_raw(i32 %fd, i64 %addr, i64 %len) {
entry:
  %empty = icmp eq i64 %len, 0
  br i1 %empty, label %done_zero, label %check_ptr
done_zero:
  ret i64 0
check_ptr:
  %is_null = icmp eq i64 %addr, 0
  br i1 %is_null, label %fault, label %transfer
fault:
  ret i64 -14
transfer:
  %large = icmp ugt i64 %len, 2147479552
  %count = select i1 %large, i64 2147479552, i64 %len
  %raw = inttoptr i64 %addr to ptr
  %result = call i64 @read(i32 %fd, ptr %raw, i64 %count)
  %failed = icmp slt i64 %result, 0
  br i1 %failed, label %error, label %done
error:
  %reason = call i64 @flurry_linux_error()
  ret i64 %reason
done:
  ret i64 %result
}
