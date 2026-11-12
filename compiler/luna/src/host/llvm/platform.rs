//! Linux host boundaries. Text is borrowed synchronously and never retained;
//! errors are captured before any other libc call can overwrite errno.
use super::*;

impl Emitter<'_> {
    pub(super) fn platform_intrinsic(
        &mut self,
        operation: nir::intrinsics::Operation,
        values: &[Value],
    ) -> Result<Value, String> {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64"))
            || self.graph.target.is_some_and(|t| t.pointer_bits() != 64)
        {
            return Err(
                "Linux host operations currently require an x86_64 Linux host and 64-bit target"
                    .into(),
            );
        }
        self.declarations.insert(include_str!("platform.ll").into());
        let arguments = values
            .iter()
            .map(|value| format!("{} {}", value.ty.llvm(), value.text))
            .collect::<Vec<_>>()
            .join(", ");
        Ok(self.instruction(
            Ty::Int(64, true),
            format!("call i64 @flurry_{}({arguments})", operation.name()),
        ))
    }
}
