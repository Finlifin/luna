//! Raw host memory intrinsic lowering for LLVM IR.
use super::*;

fn usize_bits(value: &Value) -> Result<u32, String> {
    match value.ty {
        Ty::Int(bits, false) => Ok(bits),
        _ => Err("host raw memory operation requires unsigned usize integer representation".into()),
    }
}

impl Emitter<'_> {
    pub(super) fn memory_intrinsic(
        &mut self,
        operation: nir::intrinsics::Operation,
        values: &[Value],
        result_ty: Ty,
    ) -> Result<Value, String> {
        use nir::intrinsics::Operation;
        match operation {
            Operation::MemRawRead => {
                let [addr] = values else {
                    return Err("host mem_raw_read requires 1 argument".into());
                };
                let _ = usize_bits(addr)?;
                if result_ty == Ty::Unit {
                    return Ok(Value::unit());
                }
                let ptr = self.instruction(
                    Ty::Pointer,
                    format!("inttoptr {} {} to ptr", addr.ty.llvm(), addr.text),
                );
                let align_suffix = if matches!(result_ty, Ty::PackedRecord(_)) {
                    ", align 1"
                } else {
                    ""
                };
                Ok(self.instruction(
                    result_ty.clone(),
                    format!("load {}, ptr {}{align_suffix}", result_ty.llvm(), ptr.text),
                ))
            }
            Operation::MemRawWrite => {
                let [addr, val] = values else {
                    return Err("host mem_raw_write requires 2 arguments".into());
                };
                let _ = usize_bits(addr)?;
                if result_ty != Ty::Unit {
                    return Err("host mem_raw_write requires unit result".into());
                }
                if val.ty == Ty::Unit {
                    return Ok(Value::unit());
                }
                let ptr = self.instruction(
                    Ty::Pointer,
                    format!("inttoptr {} {} to ptr", addr.ty.llvm(), addr.text),
                );
                let align_suffix = if matches!(val.ty, Ty::PackedRecord(_)) {
                    ", align 1"
                } else {
                    ""
                };
                self.line(format!(
                    "store {} {}, ptr {}{align_suffix}",
                    val.ty.llvm(),
                    val.text,
                    ptr.text
                ));
                Ok(Value::unit())
            }
            Operation::MemCopy | Operation::MemCopyNonOverlapping => {
                let [dst, src, count] = values else {
                    return Err("host mem_copy requires 3 arguments".into());
                };
                let bits = usize_bits(count)?;
                if dst.ty != count.ty || src.ty != count.ty || result_ty != Ty::Unit {
                    return Err("host mem_copy representation mismatch".into());
                }
                let intrinsic = if matches!(operation, Operation::MemCopy) {
                    "llvm.memmove"
                } else {
                    "llvm.memcpy"
                };
                self.declarations.insert(format!(
                    "declare void @{intrinsic}.p0.p0.i{bits}(ptr, ptr, i{bits}, i1)"
                ));
                let non_zero = self.instruction(
                    Ty::Bool,
                    format!("icmp ne {} {}, 0", count.ty.llvm(), count.text),
                );
                let copy_bb = self.fresh();
                let done_bb = self.fresh();
                self.line(format!(
                    "br i1 {}, label %{copy_bb}, label %{done_bb}",
                    non_zero.text
                ));
                self.label(&copy_bb);
                let dst_ptr = self.instruction(
                    Ty::Pointer,
                    format!("inttoptr {} {} to ptr", dst.ty.llvm(), dst.text),
                );
                let src_ptr = self.instruction(
                    Ty::Pointer,
                    format!("inttoptr {} {} to ptr", src.ty.llvm(), src.text),
                );
                self.line(format!(
                    "call void @{intrinsic}.p0.p0.i{bits}(ptr {}, ptr {}, {} {}, i1 false)",
                    dst_ptr.text,
                    src_ptr.text,
                    count.ty.llvm(),
                    count.text
                ));
                self.branch(&done_bb);
                self.label(&done_bb);
                Ok(Value::unit())
            }
            Operation::MemSetBytes => {
                let [dst, byte, count] = values else {
                    return Err("host mem_set_bytes requires 3 arguments".into());
                };
                let bits = usize_bits(count)?;
                if dst.ty != count.ty || byte.ty != Ty::Int(8, false) || result_ty != Ty::Unit {
                    return Err("host mem_set_bytes representation mismatch".into());
                }
                self.declarations.insert(format!(
                    "declare void @llvm.memset.p0.i{bits}(ptr, i8, i{bits}, i1)"
                ));
                let non_zero = self.instruction(
                    Ty::Bool,
                    format!("icmp ne {} {}, 0", count.ty.llvm(), count.text),
                );
                let set_bb = self.fresh();
                let done_bb = self.fresh();
                self.line(format!(
                    "br i1 {}, label %{set_bb}, label %{done_bb}",
                    non_zero.text
                ));
                self.label(&set_bb);
                let dst_ptr = self.instruction(
                    Ty::Pointer,
                    format!("inttoptr {} {} to ptr", dst.ty.llvm(), dst.text),
                );
                self.line(format!(
                    "call void @llvm.memset.p0.i{bits}(ptr {}, i8 {}, {} {}, i1 false)",
                    dst_ptr.text,
                    byte.text,
                    count.ty.llvm(),
                    count.text
                ));
                self.branch(&done_bb);
                self.label(&done_bb);
                Ok(Value::unit())
            }
            Operation::MemCompareBytes => {
                let [lhs, rhs, count] = values else {
                    return Err("host mem_compare_bytes requires 3 arguments".into());
                };
                let bits = usize_bits(count)?;
                if lhs.ty != count.ty || rhs.ty != count.ty || result_ty != Ty::Int(32, true) {
                    return Err("host mem_compare_bytes representation mismatch".into());
                }
                self.declarations
                    .insert(format!("declare i32 @memcmp(ptr, ptr, i{bits})"));
                let slot = self.slot(Ty::Int(32, true));
                self.store(
                    &slot,
                    &Value {
                        ty: Ty::Int(32, true),
                        text: "0".into(),
                    },
                )?;
                let non_zero = self.instruction(
                    Ty::Bool,
                    format!("icmp ne {} {}, 0", count.ty.llvm(), count.text),
                );
                let cmp_bb = self.fresh();
                let done_bb = self.fresh();
                self.line(format!(
                    "br i1 {}, label %{cmp_bb}, label %{done_bb}",
                    non_zero.text
                ));
                self.label(&cmp_bb);
                let lhs_ptr = self.instruction(
                    Ty::Pointer,
                    format!("inttoptr {} {} to ptr", lhs.ty.llvm(), lhs.text),
                );
                let rhs_ptr = self.instruction(
                    Ty::Pointer,
                    format!("inttoptr {} {} to ptr", rhs.ty.llvm(), rhs.text),
                );
                let cmp = self.instruction(
                    Ty::Int(32, true),
                    format!(
                        "call i32 @memcmp(ptr {}, ptr {}, {} {})",
                        lhs_ptr.text,
                        rhs_ptr.text,
                        count.ty.llvm(),
                        count.text
                    ),
                );
                self.store(&slot, &cmp)?;
                self.branch(&done_bb);
                self.label(&done_bb);
                Ok(self.load(&slot))
            }
            _ => Err(format!(
                "operation `{}` is not a raw memory intrinsic",
                operation.name()
            )),
        }
    }
}
