//! Pooled-stack continuation runtime declarations and intrinsic lowering.
use super::*;

pub(super) fn target_context_switch_ir(
    target_triple: Option<&str>,
) -> Result<&'static str, String> {
    let arch = target_triple
        .map(|triple| crate::host::tools::TargetTripleInfo::parse(triple).arch)
        .unwrap_or("x86_64");
    match arch {
        "x86_64" | "amd64" => Ok(X86_64_CTX_SWITCH_IR),
        "aarch64" | "aarch64_be" | "arm64" => Ok(AARCH64_CTX_SWITCH_IR),
        other => Err(format!(
            "host pooled-stack capturing continuations require x86_64 or aarch64 target, got {other:?}"
        )),
    }
}

const X86_64_CTX_SWITCH_IR: &str = r#"module asm ".text"
module asm ".p2align 4"
module asm "flurry_ctx_switch:"
module asm "  push %rbp"
module asm "  push %rbx"
module asm "  push %r12"
module asm "  push %r13"
module asm "  push %r14"
module asm "  push %r15"
module asm "  mov %rsp, (%rdi)"
module asm "  mov %rsi, %rsp"
module asm "  mov %rdx, %rax"
module asm "  pop %r15"
module asm "  pop %r14"
module asm "  pop %r13"
module asm "  pop %r12"
module asm "  pop %rbx"
module asm "  pop %rbp"
module asm "  ret"
module asm ".p2align 4"
module asm "flurry_ctx_trampoline:"
module asm "  mov %rax, %rdi"
module asm "  jmp *%r12"

define internal ptr @flurry_cont_init_frame(ptr %base, i64 %bytes, ptr %entry_target) {
entry:
  %end = getelementptr i8, ptr %base, i64 %bytes
  %end_i = ptrtoint ptr %end to i64
  %top_i = and i64 %end_i, -16
  %sp_i = sub i64 %top_i, 64
  %sp = inttoptr i64 %sp_i to ptr
  %s0 = getelementptr ptr, ptr %sp, i64 0
  store ptr null, ptr %s0, align 8
  %s1 = getelementptr ptr, ptr %sp, i64 1
  store ptr null, ptr %s1, align 8
  %s2 = getelementptr ptr, ptr %sp, i64 2
  store ptr null, ptr %s2, align 8
  %s3 = getelementptr ptr, ptr %sp, i64 3
  store ptr %entry_target, ptr %s3, align 8
  %s4 = getelementptr ptr, ptr %sp, i64 4
  store ptr null, ptr %s4, align 8
  %s5 = getelementptr ptr, ptr %sp, i64 5
  store ptr null, ptr %s5, align 8
  %s6 = getelementptr ptr, ptr %sp, i64 6
  store ptr @flurry_ctx_trampoline, ptr %s6, align 8
  %s7 = getelementptr ptr, ptr %sp, i64 7
  store ptr null, ptr %s7, align 8
  ret ptr %sp
}
"#;

const AARCH64_CTX_SWITCH_IR: &str = r#"module asm ".text"
module asm ".p2align 4"
module asm "flurry_ctx_switch:"
module asm "  sub sp, sp, #160"
module asm "  stp x19, x20, [sp, #0]"
module asm "  stp x21, x22, [sp, #16]"
module asm "  stp x23, x24, [sp, #32]"
module asm "  stp x25, x26, [sp, #48]"
module asm "  stp x27, x28, [sp, #64]"
module asm "  stp x29, x30, [sp, #80]"
module asm "  stp d8, d9, [sp, #96]"
module asm "  stp d10, d11, [sp, #112]"
module asm "  stp d12, d13, [sp, #128]"
module asm "  stp d14, d15, [sp, #144]"
module asm "  mov x3, sp"
module asm "  str x3, [x0]"
module asm "  mov sp, x1"
module asm "  ldp d14, d15, [sp, #144]"
module asm "  ldp d12, d13, [sp, #128]"
module asm "  ldp d10, d11, [sp, #112]"
module asm "  ldp d8, d9, [sp, #96]"
module asm "  ldp x29, x30, [sp, #80]"
module asm "  ldp x27, x28, [sp, #64]"
module asm "  ldp x25, x26, [sp, #48]"
module asm "  ldp x23, x24, [sp, #32]"
module asm "  ldp x21, x22, [sp, #16]"
module asm "  ldp x19, x20, [sp, #0]"
module asm "  add sp, sp, #160"
module asm "  mov x0, x2"
module asm "  ret"
module asm ".p2align 4"
module asm "flurry_ctx_trampoline:"
module asm "  br x19"

define internal ptr @flurry_cont_init_frame(ptr %base, i64 %bytes, ptr %entry_target) {
entry:
  %end = getelementptr i8, ptr %base, i64 %bytes
  %end_i = ptrtoint ptr %end to i64
  %top_i = and i64 %end_i, -16
  %sp_i = sub i64 %top_i, 160
  %sp = inttoptr i64 %sp_i to ptr
  %x19_p = getelementptr i8, ptr %sp, i64 0
  store ptr %entry_target, ptr %x19_p, align 8
  %x30_p = getelementptr i8, ptr %sp, i64 88
  store ptr @flurry_ctx_trampoline, ptr %x30_p, align 8
  ret ptr %sp
}
"#;

impl Emitter<'_> {
    pub(super) fn install_continuation_runtime(&mut self) -> Result<(), String> {
        let triple = self.resolved_target_triple().map(str::to_owned);
        let asm = target_context_switch_ir(triple.as_deref())?;
        self.declarations.insert(asm.into());
        self.declarations
            .insert(include_str!("continuations.ll").into());
        Ok(())
    }

    pub(super) fn continuation_intrinsic(
        &mut self,
        operation: nir::intrinsics::Operation,
        values: &[Value],
        result_ty: Ty,
    ) -> Result<Value, String> {
        use nir::intrinsics::Operation;
        if !self.checked {
            return Err("host Continuation operations require checked resource compilation".into());
        }
        self.install_continuation_runtime()?;
        self.emitted_effects.has_external_or_indirect_call = true;
        self.emitted_effects.has_trap_or_unwind = true;
        match operation {
            Operation::ContinuationResumeRaw => {
                let [cont, arg_addr, out_addr] = values else {
                    return Err("host continuation_resume_raw requires 3 operands".into());
                };
                if cont.ty != Ty::Pointer {
                    return Err("host continuation_resume_raw requires Continuation handle".into());
                }
                let Ty::Int(_, false) = arg_addr.ty else {
                    return Err("host continuation_resume_raw requires usize arg_addr".into());
                };
                if out_addr.ty != arg_addr.ty || result_ty != arg_addr.ty {
                    return Err("host continuation_resume_raw requires usize representation".into());
                }
                let raw = self.instruction(
                    Ty::Int(64, false),
                    format!(
                        "call i64 @flurry_cont_resume_raw(ptr {}, i64 {}, i64 {}, ptr %task)",
                        cont.text, arg_addr.text, out_addr.text
                    ),
                );
                if result_ty == Ty::Int(64, false) {
                    Ok(raw)
                } else {
                    Ok(self.instruction(
                        result_ty.clone(),
                        format!("trunc i64 {} to {}", raw.text, result_ty.llvm()),
                    ))
                }
            }
            Operation::ContinuationAbortRaw => {
                let [cont] = values else {
                    return Err("host continuation_abort_raw requires 1 operand".into());
                };
                if cont.ty != Ty::Pointer || result_ty != Ty::Unit {
                    return Err("host continuation_abort_raw representation mismatch".into());
                }
                self.line(format!(
                    "call void @flurry_cont_abort(ptr {}, ptr %task)",
                    cont.text
                ));
                Ok(Value::unit())
            }
            Operation::ContinuationCrossesCFrame => {
                let [cont_ptr] = values else {
                    return Err("host continuation_crosses_c_frame requires 1 operand".into());
                };
                if cont_ptr.ty != Ty::Pointer || result_ty != Ty::Bool {
                    return Err("host continuation_crosses_c_frame representation mismatch".into());
                }
                let handle = self.instruction(
                    Ty::Pointer,
                    format!("load ptr, ptr {}, align 8", cont_ptr.text),
                );
                Ok(self.instruction(
                    Ty::Bool,
                    format!("call i1 @flurry_cont_crosses_c_frame(ptr {})", handle.text),
                ))
            }
            Operation::ContinuationOriginThreadId => {
                let [cont_ptr] = values else {
                    return Err("host continuation_origin_thread_id requires 1 operand".into());
                };
                if cont_ptr.ty != Ty::Pointer || result_ty != Ty::Int(64, false) {
                    return Err("host continuation_origin_thread_id representation mismatch".into());
                }
                let handle = self.instruction(
                    Ty::Pointer,
                    format!("load ptr, ptr {}, align 8", cont_ptr.text),
                );
                Ok(self.instruction(
                    Ty::Int(64, false),
                    format!(
                        "call i64 @flurry_cont_origin_thread_id(ptr {})",
                        handle.text
                    ),
                ))
            }
            _ => Err("unsupported host continuation intrinsic".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pooled_stack_allows_resume_across_c_frame_and_traps_on_abort_across_c_frame() {
        let asm = target_context_switch_ir(Some("x86_64-pc-linux-gnu")).unwrap();
        let task_ll = include_str!("task_context.ll");
        let cont_ll = include_str!("continuations.ll");

        // Build a native test where a pooled-stack body enters a simulated C-ABI frame
        // (`@flurry_c_frame_depth` incremented), suspends (`crosses_c_frame == true`),
        // resumes cleanly when resumed, and traps when aborted across the C frame.
        let make_ir = |abort_instead_of_resume: bool| {
            let consume = if abort_instead_of_resume {
                "  call void @flurry_cont_abort(ptr %k, ptr %task)\n  ret { {}, ptr } { {} poison, ptr null }"
            } else {
                "  store i8 1, ptr %resumed_out, align 1\n  store i64 7, ptr %y_out, align 8\n  ret { {}, ptr } { {} poison, ptr null }"
            };
            format!(
                r#"declare void @llvm.trap()
{task_ll}
{asm}
{cont_ll}

define internal {{ {{}}, ptr }} @c_framed_handler(ptr %env, ptr %args, ptr %k, ptr %y_out, ptr %b_out, ptr %resumed_out, ptr %task) {{
entry:
  %cross = call i1 @flurry_cont_crosses_c_frame(ptr %k)
  br i1 %cross, label %ok_c, label %fail
fail:
  call void @llvm.trap()
  unreachable
ok_c:
  %tid = call i64 @flurry_cont_origin_thread_id(ptr %k)
  %tid_nz = icmp ne i64 %tid, 0
  br i1 %tid_nz, label %do_consume, label %fail
do_consume:
{consume}
}}

define internal {{ {{}}, ptr }} @c_framed_body(ptr %env, ptr %out_b, ptr %task) {{
entry:
  %d0 = load i64, ptr @flurry_c_frame_depth, align 8
  %d1 = add i64 %d0, 1
  store i64 %d1, ptr @flurry_c_frame_depth, align 8
  %y = alloca i64, align 8
  %aborted = call i8 @flurry_cont_suspend(ptr @c_framed_handler, ptr null, ptr null, ptr %y, i64 8)
  store i64 %d0, ptr @flurry_c_frame_depth, align 8
  %yv = load i64, ptr %y, align 8
  %sum = add i64 %yv, 35
  store i64 %sum, ptr %out_b, align 8
  ret {{ {{}}, ptr }} {{ {{}} poison, ptr null }}
}}

define i32 @main() {{
entry:
  %task = alloca %flurry.task
  call void @flurry_task_init(ptr %task, ptr null)
  %out = alloca i64, align 8
  store i64 0, ptr %out, align 8
  %ctrl = call ptr @flurry_cap_run(ptr @c_framed_body, ptr null, ptr %out, i64 8, ptr %task)
  %ok_ctrl = icmp eq ptr %ctrl, null
  %val = load i64, ptr %out, align 8
  %ok_val = icmp eq i64 %val, 42
  %ok = and i1 %ok_ctrl, %ok_val
  %bad = xor i1 %ok, true
  %code = zext i1 %bad to i32
  ret i32 %code
}}
"#
            )
        };

        let tools = crate::host::Toolchain::default();
        for opt in [
            crate::host::OptimizationLevel::O0,
            crate::host::OptimizationLevel::O3,
        ] {
            let tc = crate::host::Toolchain {
                optimization: opt,
                ..tools.clone()
            };
            let resume_out = tc.execute_test_ir(&make_ir(false)).unwrap();
            assert!(
                resume_out.status.success(),
                "resume across C frame must succeed ({opt:?})"
            );
            let abort_out = tc.execute_test_ir(&make_ir(true)).unwrap();
            assert!(
                !abort_out.status.success(),
                "abort across C frame must trap ({opt:?})"
            );
        }
    }
}
