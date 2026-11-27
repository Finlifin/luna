//! Linux host boundaries. Text is borrowed synchronously and never retained;
//! errors are captured before any other libc call can overwrite errno.
use super::*;

impl Emitter<'_> {
    pub(super) fn platform_intrinsic(
        &mut self,
        operation: nir::intrinsics::Operation,
        values: &[Value],
    ) -> Result<Value, String> {
        if self.graph.target.is_some_and(|t| t.pointer_bits() != 64) {
            return Err("Linux host operations require a 64-bit target layout".into());
        }
        if let Some(triple) = self.resolved_target_triple() {
            let info = crate::host::tools::TargetTripleInfo::parse(triple);
            if !info.is_linux || !info.is_64bit_arch {
                return Err(format!(
                    "Linux host operations require a 64-bit Linux target triple, got {triple:?}"
                ));
            }
        }
        self.emitted_effects.has_external_or_indirect_call = true;
        self.declarations
            .insert("declare void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1)".into());
        self.declarations
            .insert("declare i32 @posix_memalign(ptr, i64, i64)".into());
        self.declarations.insert("declare void @free(ptr)".into());
        self.declarations.insert(include_str!("platform.ll").into());
        let arguments = values
            .iter()
            .map(|value| format!("{} {}", value.ty.llvm(), value.text))
            .collect::<Vec<_>>()
            .join(", ");
        match operation {
            nir::intrinsics::Operation::LinuxDealloc => {
                self.line(format!(
                    "call void @flurry_{}({arguments})",
                    operation.name()
                ));
                Ok(Value::unit())
            }
            nir::intrinsics::Operation::LinuxAlloc => Ok(self.instruction(
                Ty::Int(64, false),
                format!("call i64 @flurry_{}({arguments})", operation.name()),
            )),
            _ => Ok(self.instruction(
                Ty::Int(64, true),
                format!("call i64 @flurry_{}({arguments})", operation.name()),
            )),
        }
    }
}
