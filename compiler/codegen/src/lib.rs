//! LLVM code generation from MIR.
//!
//! Translates MIR into LLVM IR using `llvm-sys`, then compiles and links
//! against libc using C ABI for basic infrastructure (printf, etc.).

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::ptr;

#[allow(deprecated)] // LLVMBuildGlobalStringPtr → LLVMBuildGlobalString TBD
use llvm_sys::core::*;
use llvm_sys::prelude::*;
use llvm_sys::target::*;
use llvm_sys::target_machine::*;
use llvm_sys::LLVMIntPredicate;
use llvm_sys::LLVMRealPredicate;

use hir::common::BinOp;
use mir::*;
use ty::{AdtId, PrimTy, TyCtxt, TyKind};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn c_str(s: &str) -> CString {
    CString::new(s).expect("CString::new failed")
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Result of LLVM codegen – owns the LLVM module and can write it.
pub struct CodegenResult {
    context: LLVMContextRef,
    module: LLVMModuleRef,
}

impl CodegenResult {
    /// Dump the LLVM IR as a string.
    pub fn dump_ir(&self) -> String {
        unsafe {
            let raw = LLVMPrintModuleToString(self.module);
            let s = CStr::from_ptr(raw).to_string_lossy().into_owned();
            LLVMDisposeMessage(raw);
            s
        }
    }

    /// Write the LLVM IR to a file.
    pub fn write_ir(&self, path: &str) -> Result<(), String> {
        let c_path = c_str(path);
        unsafe {
            let mut err_msg: *mut i8 = ptr::null_mut();
            let rc = LLVMPrintModuleToFile(self.module, c_path.as_ptr(), &mut err_msg);
            if rc != 0 {
                let msg = CStr::from_ptr(err_msg).to_string_lossy().into_owned();
                LLVMDisposeMessage(err_msg);
                return Err(msg);
            }
        }
        Ok(())
    }

    /// Write the module as an object file (.o).
    pub fn write_object(&self, path: &str) -> Result<(), String> {
        unsafe {
            LLVM_InitializeAllTargetInfos();
            LLVM_InitializeAllTargets();
            LLVM_InitializeAllTargetMCs();
            LLVM_InitializeAllAsmParsers();
            LLVM_InitializeAllAsmPrinters();

            let triple = LLVMGetDefaultTargetTriple();
            let mut target: LLVMTargetRef = ptr::null_mut();
            let mut err_msg: *mut i8 = ptr::null_mut();

            if LLVMGetTargetFromTriple(triple, &mut target, &mut err_msg) != 0 {
                let msg = CStr::from_ptr(err_msg).to_string_lossy().into_owned();
                LLVMDisposeMessage(err_msg);
                LLVMDisposeMessage(triple);
                return Err(msg);
            }

            let cpu = c_str("generic");
            let features = c_str("");

            let target_machine = LLVMCreateTargetMachine(
                target,
                triple,
                cpu.as_ptr(),
                features.as_ptr(),
                LLVMCodeGenOptLevel::LLVMCodeGenLevelDefault,
                LLVMRelocMode::LLVMRelocPIC,
                LLVMCodeModel::LLVMCodeModelDefault,
            );

            let c_path = c_str(path);
            let mut err_msg: *mut i8 = ptr::null_mut();
            let rc = LLVMTargetMachineEmitToFile(
                target_machine,
                self.module,
                c_path.as_ptr() as *mut _,
                LLVMCodeGenFileType::LLVMObjectFile,
                &mut err_msg,
            );

            LLVMDisposeTargetMachine(target_machine);
            LLVMDisposeMessage(triple);

            if rc != 0 {
                let msg = CStr::from_ptr(err_msg).to_string_lossy().into_owned();
                LLVMDisposeMessage(err_msg);
                return Err(msg);
            }
        }
        Ok(())
    }
}

impl Drop for CodegenResult {
    fn drop(&mut self) {
        unsafe {
            LLVMDisposeModule(self.module);
            LLVMContextDispose(self.context);
        }
    }
}

// ── Codegen context ──────────────────────────────────────────────────────────

struct CodegenCtx<'a> {
    context: LLVMContextRef,
    module: LLVMModuleRef,
    builder: LLVMBuilderRef,
    tcx: &'a TyCtxt,

    /// Map from function name → LLVM function value.
    functions: HashMap<String, LLVMValueRef>,
    /// Map from ADT id → LLVM struct type.
    struct_types: HashMap<AdtId, LLVMTypeRef>,
}

impl<'a> CodegenCtx<'a> {
    fn new(tcx: &'a TyCtxt) -> Self {
        unsafe {
            let context = LLVMContextCreate();
            let module_name = c_str("luna_module");
            let module = LLVMModuleCreateWithNameInContext(module_name.as_ptr(), context);
            let builder = LLVMCreateBuilderInContext(context);

            CodegenCtx {
                context,
                module,
                builder,
                tcx,
                functions: HashMap::new(),
                struct_types: HashMap::new(),
            }
        }
    }

    // ── LLVM type mapping ────────────────────────────────────────────────

    fn llvm_ty(&self, ty: &ty::Ty<'_>) -> LLVMTypeRef {
        unsafe {
            match ty.kind() {
                TyKind::Primitive(prim) => self.llvm_prim_ty(*prim),
                TyKind::Unit => LLVMVoidTypeInContext(self.context),
                TyKind::Never => LLVMVoidTypeInContext(self.context),
                TyKind::Ref(_, _) | TyKind::Ptr(_, _) => {
                    LLVMPointerTypeInContext(self.context, 0)
                }
                TyKind::Adt(adt_id, _) => {
                    if let Some(&sty) = self.struct_types.get(adt_id) {
                        sty
                    } else {
                        let name = c_str(&format!("adt_{}", adt_id.0.index()));
                        LLVMStructCreateNamed(self.context, name.as_ptr())
                    }
                }
                TyKind::Tuple(elems) => {
                    let mut elem_tys: Vec<LLVMTypeRef> =
                        elems.iter().map(|t| self.llvm_ty(t)).collect();
                    LLVMStructTypeInContext(
                        self.context,
                        elem_tys.as_mut_ptr(),
                        elem_tys.len() as u32,
                        0,
                    )
                }
                TyKind::Array(elem, len) => {
                    let elem_ty = self.llvm_ty(elem);
                    LLVMArrayType2(elem_ty, *len)
                }
                TyKind::Fn(_, _) => LLVMPointerTypeInContext(self.context, 0),
                _ => LLVMInt64TypeInContext(self.context),
            }
        }
    }

    fn llvm_prim_ty(&self, prim: PrimTy) -> LLVMTypeRef {
        unsafe {
            match prim {
                PrimTy::I8 | PrimTy::U8 => LLVMInt8TypeInContext(self.context),
                PrimTy::I16 | PrimTy::U16 => LLVMInt16TypeInContext(self.context),
                PrimTy::I32 | PrimTy::U32 => LLVMInt32TypeInContext(self.context),
                PrimTy::I64 | PrimTy::U64 => LLVMInt64TypeInContext(self.context),
                PrimTy::Isize | PrimTy::Usize => LLVMInt64TypeInContext(self.context),
                PrimTy::F32 => LLVMFloatTypeInContext(self.context),
                PrimTy::F64 => LLVMDoubleTypeInContext(self.context),
                PrimTy::Bool => LLVMInt1TypeInContext(self.context),
                PrimTy::Char => LLVMInt32TypeInContext(self.context),
                PrimTy::Str => LLVMPointerTypeInContext(self.context, 0),
            }
        }
    }

    fn is_void_ty(&self, ty: &ty::Ty<'_>) -> bool {
        matches!(ty.kind(), TyKind::Unit | TyKind::Never)
    }

    // ── Struct type definitions ──────────────────────────────────────────

    fn define_struct_types(&mut self, bodies: &[mir::Body<'_>]) {
        // Collect (AdtId → first concrete generic args seen).
        let mut adt_args: HashMap<AdtId, Vec<ty::Ty<'_>>> = HashMap::new();
        for body in bodies {
            for decl in &body.local_decls {
                self.collect_adt_args(&decl.ty, &mut adt_args);
            }
        }

        for (adt_id, args) in &adt_args {
            if let Some(def) = self.tcx.adt_def(*adt_id) {
                let name = c_str(&def.name);
                unsafe {
                    let sty = LLVMStructCreateNamed(self.context, name.as_ptr());
                    // Substitute type params in field types with concrete args.
                    let subst_args: Vec<ty::Ty<'_>> = args.to_vec();
                    let mut field_tys: Vec<LLVMTypeRef> = def
                        .fields
                        .iter()
                        .map(|f| {
                            if subst_args.is_empty() {
                                self.llvm_ty(&f.ty)
                            } else {
                                let subst_ty = self.tcx.subst(f.ty, &subst_args);
                                self.llvm_ty(&subst_ty)
                            }
                        })
                        .collect();
                    LLVMStructSetBody(
                        sty,
                        field_tys.as_mut_ptr(),
                        field_tys.len() as u32,
                        0,
                    );
                    self.struct_types.insert(*adt_id, sty);
                }
            }
        }
    }

    fn collect_adt_args<'b>(
        &self,
        ty: &ty::Ty<'b>,
        seen: &mut HashMap<AdtId, Vec<ty::Ty<'b>>>,
    ) {
        match ty.kind() {
            TyKind::Adt(adt_id, args) => {
                seen.entry(*adt_id).or_insert_with(|| args.to_vec());
                for arg in args.iter() {
                    self.collect_adt_args(arg, seen);
                }
            }
            TyKind::Ref(inner, _) | TyKind::Ptr(inner, _) | TyKind::Optional(inner) => {
                self.collect_adt_args(inner, seen);
            }
            TyKind::Tuple(elems) => {
                for elem in elems.iter() {
                    self.collect_adt_args(elem, seen);
                }
            }
            TyKind::Array(elem, _) | TyKind::Slice(elem) => {
                self.collect_adt_args(elem, seen);
            }
            TyKind::Fn(params, ret) => {
                for p in params.iter() {
                    self.collect_adt_args(p, seen);
                }
                self.collect_adt_args(ret, seen);
            }
            _ => {}
        }
    }

    // ── Declare printf ───────────────────────────────────────────────────

    fn declare_printf(&mut self) {
        unsafe {
            let name = c_str("printf");
            let ptr_ty = LLVMPointerTypeInContext(self.context, 0);
            let i32_ty = LLVMInt32TypeInContext(self.context);
            let fn_ty = LLVMFunctionType(i32_ty, [ptr_ty].as_mut_ptr(), 1, 1);
            let func = LLVMAddFunction(self.module, name.as_ptr(), fn_ty);
            self.functions.insert("printf".to_string(), func);
        }
    }

    // ── Function codegen ─────────────────────────────────────────────────

    fn codegen_function(&mut self, body: &mir::Body<'_>) {
        let fn_name = body.local_decls[0]
            .name
            .as_deref()
            .unwrap_or("unknown");

        let ret_ty = &body.local_decls[0].ty;
        let is_void = self.is_void_ty(ret_ty);

        let llvm_ret_ty = if is_void {
            unsafe { LLVMVoidTypeInContext(self.context) }
        } else {
            self.llvm_ty(ret_ty)
        };

        let mut param_tys: Vec<LLVMTypeRef> = (1..=body.arg_count)
            .map(|i| self.llvm_ty(&body.local_decls[i].ty))
            .collect();

        let fn_ty = unsafe {
            LLVMFunctionType(llvm_ret_ty, param_tys.as_mut_ptr(), param_tys.len() as u32, 0)
        };

        let c_name = c_str(fn_name);
        let function = unsafe { LLVMAddFunction(self.module, c_name.as_ptr(), fn_ty) };
        self.functions.insert(fn_name.to_string(), function);
    }

    fn codegen_function_body(&mut self, body: &mir::Body<'_>) {
        let fn_name = body.local_decls[0]
            .name
            .as_deref()
            .unwrap_or("unknown");

        let function = self.functions[fn_name];
        let ret_ty = &body.local_decls[0].ty;
        let is_void = self.is_void_ty(ret_ty);

        // Create entry block
        let entry_bb = unsafe {
            LLVMAppendBasicBlockInContext(self.context, function, c_str("entry").as_ptr())
        };

        // Create basic blocks for each MIR basic block
        let mut bb_map: Vec<LLVMBasicBlockRef> = Vec::new();
        for i in 0..body.basic_blocks.len() {
            let name = c_str(&format!("bb{}", i));
            let bb = unsafe {
                LLVMAppendBasicBlockInContext(self.context, function, name.as_ptr())
            };
            bb_map.push(bb);
        }

        // Entry block: allocate all locals and branch to bb0
        unsafe { LLVMPositionBuilderAtEnd(self.builder, entry_bb) };

        let mut locals: Vec<LLVMValueRef> = Vec::new();

        // _0 = return place
        let ret_alloca = if !is_void {
            let alloca = unsafe {
                LLVMBuildAlloca(self.builder, self.llvm_ty(ret_ty), c_str("_0").as_ptr())
            };
            locals.push(alloca);
            Some(alloca)
        } else {
            locals.push(ptr::null_mut());
            None
        };

        // _1 .. _arg_count = parameters (allocate + store)
        for i in 1..=body.arg_count {
            let decl = &body.local_decls[i];
            let name = c_str(&format!("_{}", i));
            let alloca =
                unsafe { LLVMBuildAlloca(self.builder, self.llvm_ty(&decl.ty), name.as_ptr()) };
            let param = unsafe { LLVMGetParam(function, (i - 1) as u32) };
            unsafe { LLVMBuildStore(self.builder, param, alloca) };
            locals.push(alloca);
        }

        // remaining locals = temporaries
        for i in (body.arg_count + 1)..body.local_decls.len() {
            let decl = &body.local_decls[i];
            if self.is_void_ty(&decl.ty) {
                locals.push(ptr::null_mut());
            } else {
                let name = c_str(&format!("_{}", i));
                let alloca = unsafe {
                    LLVMBuildAlloca(self.builder, self.llvm_ty(&decl.ty), name.as_ptr())
                };
                locals.push(alloca);
            }
        }

        // Branch from entry to bb0
        if !bb_map.is_empty() {
            unsafe { LLVMBuildBr(self.builder, bb_map[0]) };
        }

        // Codegen each basic block
        for (bb_idx, bb_data) in body.basic_blocks.iter().enumerate() {
            unsafe { LLVMPositionBuilderAtEnd(self.builder, bb_map[bb_idx]) };

            for stmt in &bb_data.statements {
                self.codegen_statement(stmt, body, &locals);
            }

            if let Some(ref term) = bb_data.terminator {
                self.codegen_terminator(term, body, &locals, &bb_map, ret_alloca, is_void);
            }
        }
    }

    fn codegen_statement(
        &self,
        stmt: &Statement<'_>,
        body: &mir::Body<'_>,
        locals: &[LLVMValueRef],
    ) {
        match &stmt.kind {
            StatementKind::Assign(place, rvalue) => {
                // Skip unit assignments
                if let Rvalue::Use(Operand::Constant(Constant {
                    kind: ConstKind::Unit,
                    ..
                })) = rvalue
                {
                    return;
                }

                let val = self.codegen_rvalue(rvalue, body, locals);
                let dst = self.codegen_place_ptr(place, body, locals);
                if !dst.is_null() && !val.is_null() {
                    unsafe { LLVMBuildStore(self.builder, val, dst) };
                }
            }
            StatementKind::Nop => {}
        }
    }

    fn codegen_terminator(
        &self,
        term: &Terminator<'_>,
        body: &mir::Body<'_>,
        locals: &[LLVMValueRef],
        bb_map: &[LLVMBasicBlockRef],
        ret_alloca: Option<LLVMValueRef>,
        is_void: bool,
    ) {
        match &term.kind {
            TerminatorKind::Goto { target } => unsafe {
                LLVMBuildBr(self.builder, bb_map[target.index()]);
            },

            TerminatorKind::Return => unsafe {
                if is_void {
                    LLVMBuildRetVoid(self.builder);
                } else if let Some(ret_ptr) = ret_alloca {
                    let val = LLVMBuildLoad2(
                        self.builder,
                        self.llvm_ty(&body.local_decls[0].ty),
                        ret_ptr,
                        c_str("ret").as_ptr(),
                    );
                    LLVMBuildRet(self.builder, val);
                } else {
                    LLVMBuildRetVoid(self.builder);
                }
            },

            TerminatorKind::Unreachable => unsafe {
                LLVMBuildUnreachable(self.builder);
            },

            TerminatorKind::SwitchInt { discr, targets } => {
                let cond = self.codegen_operand(discr, body, locals);

                if targets.values.len() == 1 {
                    let (val, target) = &targets.values[0];
                    let cmp = if *val == 1 {
                        // boolean true check — cond is already i1
                        cond
                    } else {
                        let const_val = unsafe {
                            LLVMConstInt(LLVMTypeOf(cond), *val as u64, 0)
                        };
                        unsafe {
                            LLVMBuildICmp(
                                self.builder,
                                LLVMIntPredicate::LLVMIntEQ,
                                cond,
                                const_val,
                                c_str("cmp").as_ptr(),
                            )
                        }
                    };
                    unsafe {
                        LLVMBuildCondBr(
                            self.builder,
                            cmp,
                            bb_map[target.index()],
                            bb_map[targets.otherwise.index()],
                        );
                    }
                } else {
                    let switch = unsafe {
                        LLVMBuildSwitch(
                            self.builder,
                            cond,
                            bb_map[targets.otherwise.index()],
                            targets.values.len() as u32,
                        )
                    };
                    for (val, target) in &targets.values {
                        let const_val = unsafe {
                            LLVMConstInt(LLVMTypeOf(cond), *val as u64, 0)
                        };
                        unsafe {
                            LLVMAddCase(switch, const_val, bb_map[target.index()]);
                        }
                    }
                }
            }

            TerminatorKind::Call {
                func,
                args,
                destination,
                target,
            } => {
                let callee = self.resolve_callee(func);
                let mut llvm_args: Vec<LLVMValueRef> = args
                    .iter()
                    .map(|a| self.codegen_operand(a, body, locals))
                    .collect();

                let fn_ty = self.get_callee_fn_type(func);

                // Determine if call returns void
                let dest_idx = destination.local.index();
                let call_is_void = dest_idx < body.local_decls.len()
                    && self.is_void_ty(&body.local_decls[dest_idx].ty);

                let name = if call_is_void {
                    c_str("")
                } else {
                    c_str("call")
                };

                let call_val = unsafe {
                    LLVMBuildCall2(
                        self.builder,
                        fn_ty,
                        callee,
                        llvm_args.as_mut_ptr(),
                        llvm_args.len() as u32,
                        name.as_ptr(),
                    )
                };

                // Store result
                if !call_is_void {
                    let dst = self.codegen_place_ptr(destination, body, locals);
                    if !dst.is_null() {
                        unsafe { LLVMBuildStore(self.builder, call_val, dst) };
                    }
                }

                if let Some(target) = target {
                    unsafe {
                        LLVMBuildBr(self.builder, bb_map[target.index()]);
                    }
                }
            }
        }
    }

    fn resolve_callee(&self, func: &Operand<'_>) -> LLVMValueRef {
        match func {
            Operand::Constant(Constant {
                kind: ConstKind::FnName(name),
                ..
            }) => {
                if let Some(&f) = self.functions.get(name.as_str()) {
                    f
                } else {
                    // Declare as external
                    let ret_ty = unsafe { LLVMInt64TypeInContext(self.context) };
                    let fn_ty = unsafe { LLVMFunctionType(ret_ty, ptr::null_mut(), 0, 1) };
                    let c_name = c_str(name);
                    unsafe { LLVMAddFunction(self.module, c_name.as_ptr(), fn_ty) }
                }
            }
            _ => ptr::null_mut(),
        }
    }

    fn get_callee_fn_type(&self, func: &Operand<'_>) -> LLVMTypeRef {
        match func {
            Operand::Constant(Constant {
                kind: ConstKind::FnName(name),
                ..
            }) => {
                if let Some(&f) = self.functions.get(name.as_str()) {
                    unsafe { LLVMGlobalGetValueType(f) }
                } else {
                    unsafe {
                        let ret_ty = LLVMInt64TypeInContext(self.context);
                        LLVMFunctionType(ret_ty, ptr::null_mut(), 0, 1)
                    }
                }
            }
            _ => unsafe {
                let ret_ty = LLVMInt64TypeInContext(self.context);
                LLVMFunctionType(ret_ty, ptr::null_mut(), 0, 1)
            },
        }
    }

    // ── Operand / Rvalue codegen ─────────────────────────────────────────

    fn codegen_operand(
        &self,
        op: &Operand<'_>,
        body: &mir::Body<'_>,
        locals: &[LLVMValueRef],
    ) -> LLVMValueRef {
        match op {
            Operand::Copy(place) | Operand::Move(place) => {
                self.codegen_place_load(place, body, locals)
            }
            Operand::Constant(c) => self.codegen_constant(c),
        }
    }

    fn codegen_constant(&self, c: &Constant<'_>) -> LLVMValueRef {
        unsafe {
            match &c.kind {
                ConstKind::Int(v) => {
                    let ty = self.llvm_ty(&c.ty);
                    LLVMConstInt(ty, *v as u64, 1)
                }
                ConstKind::Float(v) => {
                    let ty = self.llvm_ty(&c.ty);
                    LLVMConstReal(ty, *v)
                }
                ConstKind::Bool(v) => {
                    LLVMConstInt(LLVMInt1TypeInContext(self.context), *v as u64, 0)
                }
                ConstKind::Char(v) => {
                    LLVMConstInt(LLVMInt32TypeInContext(self.context), *v as u64, 0)
                }
                ConstKind::Str(s) => {
                    let c_s = c_str(s);
                    LLVMBuildGlobalStringPtr(self.builder, c_s.as_ptr(), c_str("str").as_ptr())
                }
                ConstKind::FnName(name) => {
                    if let Some(&f) = self.functions.get(name.as_str()) {
                        f
                    } else {
                        LLVMConstInt(LLVMInt64TypeInContext(self.context), 0, 0)
                    }
                }
                ConstKind::FnDef(_) => {
                    LLVMConstInt(LLVMInt64TypeInContext(self.context), 0, 0)
                }
                ConstKind::Unit => LLVMGetUndef(LLVMInt1TypeInContext(self.context)),
                ConstKind::Null => {
                    let ptr_ty = LLVMPointerTypeInContext(self.context, 0);
                    LLVMConstNull(ptr_ty)
                }
            }
        }
    }

    fn codegen_rvalue(
        &self,
        rv: &Rvalue<'_>,
        body: &mir::Body<'_>,
        locals: &[LLVMValueRef],
    ) -> LLVMValueRef {
        match rv {
            Rvalue::Use(op) => self.codegen_operand(op, body, locals),

            Rvalue::BinaryOp(op, lhs, rhs) => {
                let l = self.codegen_operand(lhs, body, locals);
                let r = self.codegen_operand(rhs, body, locals);
                let is_float = self.operand_is_float(lhs, body);
                let is_signed = self.operand_is_signed(lhs, body);
                self.codegen_binop(*op, l, r, is_float, is_signed)
            }

            Rvalue::UnaryOp(op, operand) => {
                let v = self.codegen_operand(operand, body, locals);
                let is_float = self.operand_is_float(operand, body);
                match op {
                    hir::common::UnOp::Neg => {
                        if is_float {
                            unsafe { LLVMBuildFNeg(self.builder, v, c_str("fneg").as_ptr()) }
                        } else {
                            unsafe { LLVMBuildNeg(self.builder, v, c_str("neg").as_ptr()) }
                        }
                    }
                    hir::common::UnOp::Not => unsafe {
                        LLVMBuildNot(self.builder, v, c_str("not").as_ptr())
                    },
                }
            }

            Rvalue::Ref(place) => self.codegen_place_ptr(place, body, locals),

            Rvalue::Aggregate(kind, ops) => {
                let operands: Vec<LLVMValueRef> = ops
                    .iter()
                    .map(|o| self.codegen_operand(o, body, locals))
                    .collect();

                match kind {
                    AggregateKind::Adt(def_id) => {
                        let adt_id = AdtId(*def_id);
                        if let Some(&sty) = self.struct_types.get(&adt_id) {
                            let mut agg = unsafe { LLVMGetUndef(sty) };
                            for (i, val) in operands.iter().enumerate() {
                                agg = unsafe {
                                    LLVMBuildInsertValue(
                                        self.builder,
                                        agg,
                                        *val,
                                        i as u32,
                                        c_str("field").as_ptr(),
                                    )
                                };
                            }
                            agg
                        } else {
                            unsafe { LLVMGetUndef(LLVMInt64TypeInContext(self.context)) }
                        }
                    }
                    AggregateKind::Tuple | AggregateKind::Array => {
                        if operands.is_empty() {
                            return unsafe {
                                LLVMGetUndef(LLVMInt1TypeInContext(self.context))
                            };
                        }
                        let mut tys: Vec<LLVMTypeRef> = operands
                            .iter()
                            .map(|v| unsafe { LLVMTypeOf(*v) })
                            .collect();
                        let sty = unsafe {
                            LLVMStructTypeInContext(
                                self.context,
                                tys.as_mut_ptr(),
                                tys.len() as u32,
                                0,
                            )
                        };
                        let mut agg = unsafe { LLVMGetUndef(sty) };
                        for (i, val) in operands.iter().enumerate() {
                            agg = unsafe {
                                LLVMBuildInsertValue(
                                    self.builder,
                                    agg,
                                    *val,
                                    i as u32,
                                    c_str("elem").as_ptr(),
                                )
                            };
                        }
                        agg
                    }
                }
            }
        }
    }

    fn codegen_binop(
        &self,
        op: BinOp,
        lhs: LLVMValueRef,
        rhs: LLVMValueRef,
        is_float: bool,
        is_signed: bool,
    ) -> LLVMValueRef {
        unsafe {
            if is_float {
                match op {
                    BinOp::Add => LLVMBuildFAdd(self.builder, lhs, rhs, c_str("fadd").as_ptr()),
                    BinOp::Sub => LLVMBuildFSub(self.builder, lhs, rhs, c_str("fsub").as_ptr()),
                    BinOp::Mul => LLVMBuildFMul(self.builder, lhs, rhs, c_str("fmul").as_ptr()),
                    BinOp::Div => LLVMBuildFDiv(self.builder, lhs, rhs, c_str("fdiv").as_ptr()),
                    BinOp::Rem => LLVMBuildFRem(self.builder, lhs, rhs, c_str("frem").as_ptr()),
                    BinOp::Eq => LLVMBuildFCmp(self.builder, LLVMRealPredicate::LLVMRealOEQ, lhs, rhs, c_str("feq").as_ptr()),
                    BinOp::Ne => LLVMBuildFCmp(self.builder, LLVMRealPredicate::LLVMRealONE, lhs, rhs, c_str("fne").as_ptr()),
                    BinOp::Lt => LLVMBuildFCmp(self.builder, LLVMRealPredicate::LLVMRealOLT, lhs, rhs, c_str("flt").as_ptr()),
                    BinOp::Gt => LLVMBuildFCmp(self.builder, LLVMRealPredicate::LLVMRealOGT, lhs, rhs, c_str("fgt").as_ptr()),
                    BinOp::Le => LLVMBuildFCmp(self.builder, LLVMRealPredicate::LLVMRealOLE, lhs, rhs, c_str("fle").as_ptr()),
                    BinOp::Ge => LLVMBuildFCmp(self.builder, LLVMRealPredicate::LLVMRealOGE, lhs, rhs, c_str("fge").as_ptr()),
                    _ => LLVMGetUndef(LLVMTypeOf(lhs)),
                }
            } else {
                match op {
                    BinOp::Add => LLVMBuildAdd(self.builder, lhs, rhs, c_str("add").as_ptr()),
                    BinOp::Sub => LLVMBuildSub(self.builder, lhs, rhs, c_str("sub").as_ptr()),
                    BinOp::Mul => LLVMBuildMul(self.builder, lhs, rhs, c_str("mul").as_ptr()),
                    BinOp::Div if is_signed => LLVMBuildSDiv(self.builder, lhs, rhs, c_str("sdiv").as_ptr()),
                    BinOp::Div => LLVMBuildUDiv(self.builder, lhs, rhs, c_str("udiv").as_ptr()),
                    BinOp::Rem if is_signed => LLVMBuildSRem(self.builder, lhs, rhs, c_str("srem").as_ptr()),
                    BinOp::Rem => LLVMBuildURem(self.builder, lhs, rhs, c_str("urem").as_ptr()),
                    BinOp::Eq => LLVMBuildICmp(self.builder, LLVMIntPredicate::LLVMIntEQ, lhs, rhs, c_str("eq").as_ptr()),
                    BinOp::Ne => LLVMBuildICmp(self.builder, LLVMIntPredicate::LLVMIntNE, lhs, rhs, c_str("ne").as_ptr()),
                    BinOp::Lt if is_signed => LLVMBuildICmp(self.builder, LLVMIntPredicate::LLVMIntSLT, lhs, rhs, c_str("slt").as_ptr()),
                    BinOp::Lt => LLVMBuildICmp(self.builder, LLVMIntPredicate::LLVMIntULT, lhs, rhs, c_str("ult").as_ptr()),
                    BinOp::Gt if is_signed => LLVMBuildICmp(self.builder, LLVMIntPredicate::LLVMIntSGT, lhs, rhs, c_str("sgt").as_ptr()),
                    BinOp::Gt => LLVMBuildICmp(self.builder, LLVMIntPredicate::LLVMIntUGT, lhs, rhs, c_str("ugt").as_ptr()),
                    BinOp::Le if is_signed => LLVMBuildICmp(self.builder, LLVMIntPredicate::LLVMIntSLE, lhs, rhs, c_str("sle").as_ptr()),
                    BinOp::Le => LLVMBuildICmp(self.builder, LLVMIntPredicate::LLVMIntULE, lhs, rhs, c_str("ule").as_ptr()),
                    BinOp::Ge if is_signed => LLVMBuildICmp(self.builder, LLVMIntPredicate::LLVMIntSGE, lhs, rhs, c_str("sge").as_ptr()),
                    BinOp::Ge => LLVMBuildICmp(self.builder, LLVMIntPredicate::LLVMIntUGE, lhs, rhs, c_str("uge").as_ptr()),
                    BinOp::And => LLVMBuildAnd(self.builder, lhs, rhs, c_str("and").as_ptr()),
                    BinOp::Or => LLVMBuildOr(self.builder, lhs, rhs, c_str("or").as_ptr()),
                    BinOp::BitAnd => LLVMBuildAnd(self.builder, lhs, rhs, c_str("band").as_ptr()),
                    BinOp::BitOr => LLVMBuildOr(self.builder, lhs, rhs, c_str("bor").as_ptr()),
                    BinOp::BitXor => LLVMBuildXor(self.builder, lhs, rhs, c_str("bxor").as_ptr()),
                    BinOp::Shl => LLVMBuildShl(self.builder, lhs, rhs, c_str("shl").as_ptr()),
                    BinOp::Shr if is_signed => LLVMBuildAShr(self.builder, lhs, rhs, c_str("ashr").as_ptr()),
                    BinOp::Shr => LLVMBuildLShr(self.builder, lhs, rhs, c_str("lshr").as_ptr()),
                }
            }
        }
    }

    // ── Place codegen ────────────────────────────────────────────────────

    /// Get a pointer (alloca) to a Place, following projections with GEP.
    fn codegen_place_ptr(
        &self,
        place: &Place,
        body: &mir::Body<'_>,
        locals: &[LLVMValueRef],
    ) -> LLVMValueRef {
        let idx = place.local.index();
        if idx >= locals.len() {
            return ptr::null_mut();
        }

        let mut current_ptr = locals[idx];
        if current_ptr.is_null() {
            return ptr::null_mut();
        }

        let mut cur_ty: Option<ty::Ty<'_>> = if idx < body.local_decls.len() {
            Some(body.local_decls[idx].ty)
        } else {
            None
        };

        for proj in &place.projection {
            match proj {
                PlaceElem::Field(field_idx) => {
                    if let Some(ty) = cur_ty {
                        let llvm_struct_ty = self.llvm_ty(&ty);
                        current_ptr = unsafe {
                            LLVMBuildStructGEP2(
                                self.builder,
                                llvm_struct_ty,
                                current_ptr,
                                *field_idx,
                                c_str("fptr").as_ptr(),
                            )
                        };
                        // Update type to field type
                        cur_ty = match ty.kind() {
                            TyKind::Adt(adt_id, args) => self
                                .tcx
                                .adt_def(*adt_id)
                                .and_then(|def| {
                                    def.fields.get(*field_idx as usize).map(|f| {
                                        if args.is_empty() {
                                            f.ty
                                        } else {
                                            self.tcx.subst(f.ty, args)
                                        }
                                    })
                                }),
                            TyKind::Tuple(elems) => {
                                elems.get(*field_idx as usize).copied()
                            }
                            _ => None,
                        };
                    }
                }
                PlaceElem::Deref => {
                    if let Some(ty) = cur_ty {
                        let load_ty = self.llvm_ty(&ty);
                        current_ptr = unsafe {
                            LLVMBuildLoad2(
                                self.builder,
                                load_ty,
                                current_ptr,
                                c_str("deref").as_ptr(),
                            )
                        };
                        cur_ty = match ty.kind() {
                            TyKind::Ref(inner, _) | TyKind::Ptr(inner, _) => Some(*inner),
                            _ => None,
                        };
                    }
                }
                PlaceElem::Index(local) => {
                    if let Some(ty) = cur_ty {
                        let index_val = unsafe {
                            LLVMBuildLoad2(
                                self.builder,
                                LLVMInt64TypeInContext(self.context),
                                locals[local.index()],
                                c_str("idx").as_ptr(),
                            )
                        };
                        let zero = unsafe {
                            LLVMConstInt(LLVMInt64TypeInContext(self.context), 0, 0)
                        };
                        let arr_ty = self.llvm_ty(&ty);
                        let mut indices = [zero, index_val];
                        current_ptr = unsafe {
                            LLVMBuildGEP2(
                                self.builder,
                                arr_ty,
                                current_ptr,
                                indices.as_mut_ptr(),
                                2,
                                c_str("iptr").as_ptr(),
                            )
                        };
                        cur_ty = match ty.kind() {
                            TyKind::Array(elem, _) | TyKind::Slice(elem) => Some(*elem),
                            _ => None,
                        };
                    }
                }
            }
        }

        current_ptr
    }

    /// Load a value from a Place.
    fn codegen_place_load(
        &self,
        place: &Place,
        body: &mir::Body<'_>,
        locals: &[LLVMValueRef],
    ) -> LLVMValueRef {
        let ptr = self.codegen_place_ptr(place, body, locals);
        if ptr.is_null() {
            return unsafe { LLVMGetUndef(LLVMInt64TypeInContext(self.context)) };
        }

        let ty = self.place_ty(place, body);
        if self.is_void_ty(&ty) {
            return unsafe { LLVMGetUndef(LLVMInt1TypeInContext(self.context)) };
        }
        let llvm_ty = self.llvm_ty(&ty);
        unsafe { LLVMBuildLoad2(self.builder, llvm_ty, ptr, c_str("load").as_ptr()) }
    }

    /// Compute the semantic type of a Place by following projections.
    fn place_ty<'b>(&self, place: &Place, body: &'b mir::Body<'_>) -> ty::Ty<'b> {
        let idx = place.local.index();
        let mut ty = if idx < body.local_decls.len() {
            body.local_decls[idx].ty
        } else {
            return unsafe {
                std::mem::transmute::<ty::Ty<'_>, ty::Ty<'b>>(
                    self.tcx.mk_primitive(PrimTy::I64),
                )
            };
        };

        for proj in &place.projection {
            match proj {
                PlaceElem::Field(field_idx) => {
                    ty = match ty.kind() {
                        TyKind::Adt(adt_id, args) => {
                            if let Some(def) = self.tcx.adt_def(*adt_id) {
                                if let Some(field) = def.fields.get(*field_idx as usize) {
                                    let raw_ty = unsafe {
                                        std::mem::transmute::<ty::Ty<'static>, ty::Ty<'b>>(
                                            field.ty,
                                        )
                                    };
                                    if args.is_empty() {
                                        raw_ty
                                    } else {
                                        let substed = self.tcx.subst(raw_ty, args);
                                        unsafe {
                                            std::mem::transmute::<ty::Ty<'_>, ty::Ty<'b>>(
                                                substed,
                                            )
                                        }
                                    }
                                } else {
                                    ty
                                }
                            } else {
                                ty
                            }
                        }
                        TyKind::Tuple(elems) => {
                            if let Some(&elem_ty) = elems.get(*field_idx as usize) {
                                elem_ty
                            } else {
                                ty
                            }
                        }
                        _ => ty,
                    };
                }
                PlaceElem::Deref => {
                    ty = match ty.kind() {
                        TyKind::Ref(inner, _) | TyKind::Ptr(inner, _) => *inner,
                        _ => ty,
                    };
                }
                PlaceElem::Index(_) => {
                    ty = match ty.kind() {
                        TyKind::Array(elem, _) | TyKind::Slice(elem) => *elem,
                        _ => ty,
                    };
                }
            }
        }

        ty
    }

    // ── Type queries on operands ─────────────────────────────────────────

    fn operand_is_float(&self, op: &Operand<'_>, body: &mir::Body<'_>) -> bool {
        match op {
            Operand::Copy(place) | Operand::Move(place) => {
                let ty = self.place_ty(place, body);
                matches!(ty.kind(), TyKind::Primitive(p) if p.is_float())
            }
            Operand::Constant(c) => matches!(c.kind, ConstKind::Float(_)),
        }
    }

    fn operand_is_signed(&self, op: &Operand<'_>, body: &mir::Body<'_>) -> bool {
        match op {
            Operand::Copy(place) | Operand::Move(place) => {
                let ty = self.place_ty(place, body);
                match ty.kind() {
                    TyKind::Primitive(p) => p.is_signed_int(),
                    _ => true,
                }
            }
            Operand::Constant(c) => matches!(c.kind, ConstKind::Int(_)),
        }
    }

    // ── Test main ────────────────────────────────────────────────────────

    fn codegen_test_main(&mut self, bodies: &[mir::Body<'_>]) {
        let has_main = bodies.iter().any(|b| {
            b.local_decls[0]
                .name
                .as_deref()
                .is_some_and(|n| n == "main")
        });
        if has_main {
            return;
        }

        let i32_ty = unsafe { LLVMInt32TypeInContext(self.context) };
        let main_ty = unsafe { LLVMFunctionType(i32_ty, ptr::null_mut(), 0, 0) };
        let main_fn = unsafe {
            LLVMAddFunction(self.module, c_str("main").as_ptr(), main_ty)
        };

        let entry = unsafe {
            LLVMAppendBasicBlockInContext(self.context, main_fn, c_str("entry").as_ptr())
        };
        unsafe { LLVMPositionBuilderAtEnd(self.builder, entry) };

        let printf_fn = self.functions["printf"];

        for body in bodies {
            let name = body.local_decls[0]
                .name
                .as_deref()
                .unwrap_or("unknown");

            let ret_ty = &body.local_decls[0].ty;
            if self.is_void_ty(ret_ty) {
                continue;
            }

            let callee = self.functions[name];
            let callee_fn_ty = unsafe { LLVMGlobalGetValueType(callee) };

            let test_vals: &[i64] = match body.arg_count {
                0 => &[],
                1 => &[10],
                2 => &[3, 4],
                3 => &[1, 2, 3],
                4 => &[1, 5, 4, 2],
                _ => continue,
            };

            let mut args: Vec<LLVMValueRef> = Vec::new();
            for (i, &val) in test_vals.iter().enumerate() {
                let param_idx = i + 1;
                if param_idx < body.local_decls.len() {
                    let param_ty = self.llvm_ty(&body.local_decls[param_idx].ty);
                    args.push(unsafe { LLVMConstInt(param_ty, val as u64, 1) });
                }
            }

            let call_result = unsafe {
                LLVMBuildCall2(
                    self.builder,
                    callee_fn_ty,
                    callee,
                    args.as_mut_ptr(),
                    args.len() as u32,
                    c_str("result").as_ptr(),
                )
            };

            let is_float_ret = matches!(ret_ty.kind(), TyKind::Primitive(p) if p.is_float());
            let fmt_spec = if is_float_ret { "%f" } else { "%ld" };

            let args_str = test_vals
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let fmt = format!("{}({}) = {}\n", name, args_str, fmt_spec);
            let c_fmt = c_str(&fmt);
            let fmt_str = unsafe {
                LLVMBuildGlobalStringPtr(self.builder, c_fmt.as_ptr(), c_str("fmt").as_ptr())
            };

            // Extend bool to i64 for printf, or f32 to f64
            let print_val = if matches!(ret_ty.kind(), TyKind::Primitive(PrimTy::Bool)) {
                unsafe {
                    LLVMBuildZExt(
                        self.builder,
                        call_result,
                        LLVMInt64TypeInContext(self.context),
                        c_str("bext").as_ptr(),
                    )
                }
            } else if is_float_ret && matches!(ret_ty.kind(), TyKind::Primitive(PrimTy::F32)) {
                unsafe {
                    LLVMBuildFPExt(
                        self.builder,
                        call_result,
                        LLVMDoubleTypeInContext(self.context),
                        c_str("fext").as_ptr(),
                    )
                }
            } else {
                call_result
            };

            // printf(fmt, result)
            let printf_ty = unsafe {
                let ptr_ty = LLVMPointerTypeInContext(self.context, 0);
                LLVMFunctionType(i32_ty, [ptr_ty].as_mut_ptr(), 1, 1)
            };
            let mut printf_args = [fmt_str, print_val];
            unsafe {
                LLVMBuildCall2(
                    self.builder,
                    printf_ty,
                    printf_fn,
                    printf_args.as_mut_ptr(),
                    2,
                    c_str("").as_ptr(),
                );
            }
        }

        // return 0
        unsafe {
            LLVMBuildRet(self.builder, LLVMConstInt(i32_ty, 0, 0));
        }
    }
}

impl Drop for CodegenCtx<'_> {
    fn drop(&mut self) {
        unsafe {
            LLVMDisposeBuilder(self.builder);
            // context and module are NOT freed here — ownership may be
            // transferred to CodegenResult via mem::forget.
        }
    }
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Generate LLVM IR from MIR bodies. Returns a `CodegenResult` that can
/// be used to emit IR, object files, or link.
pub fn codegen_llvm(bodies: &[mir::Body<'_>], tcx: &TyCtxt) -> CodegenResult {
    let mut ctx = CodegenCtx::new(tcx);

    // Define struct types
    ctx.define_struct_types(bodies);

    // Declare printf (libc)
    ctx.declare_printf();

    // First pass: declare all functions (forward declarations)
    for body in bodies {
        ctx.codegen_function(body);
    }

    // Second pass: generate function bodies
    for body in bodies {
        ctx.codegen_function_body(body);
    }

    // Generate test main if no user main
    ctx.codegen_test_main(bodies);

    let result = CodegenResult {
        context: ctx.context,
        module: ctx.module,
    };

    // Prevent CodegenCtx destructor from freeing context/module — ownership
    // transferred to CodegenResult.
    std::mem::forget(ctx);

    result
}
