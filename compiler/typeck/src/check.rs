//! Type checking pass – traverses HIR and assigns `Ty<'tcx>` to every node.

use hir::body::Body;
use hir::common::{BinOp, LitKind, UnOp};
use hir::expr::{Arg, ExprKind, StmtKind};
use hir::item::{FnParamTy, ItemKind};
use hir::{Expr, Package};
use ty::{AdtDef, AdtId, AdtKind, FieldDef, PrimTy, Ty, TyCtxt, TyKind};

use crate::resolve_ty::{TyResolutionCtx, resolve_ty_expr, resolve_ty_expr_in};

/// Run type checking on every item in a package.
///
/// After this returns, the `TyCtxt` is populated with:
/// - `def_ty(def_id)` for every function definition
/// - `node_ty(hir_id)` for every expression in every body
pub fn typeck_package<'hir>(package: &Package<'hir>, tcx: &TyCtxt) {
    for (owner_id, info) in package.owners() {
        let item = info.node.expect_item();
        match &item.kind {
            ItemKind::Fn(sig, body_id) => {
                // Resolve parameter types.
                let param_tys: Vec<Ty<'_>> = sig
                    .decl
                    .inputs
                    .iter()
                    .map(|p| match p {
                        FnParamTy::Typed(_, ty_expr, _, _) => resolve_ty_expr(tcx, ty_expr),
                        FnParamTy::Optional { ty, .. } => resolve_ty_expr(tcx, ty),
                        FnParamTy::Variadic(_, ty_expr, _, _) => resolve_ty_expr(tcx, ty_expr),
                    })
                    .collect();

                // Resolve return type.
                let ret_ty = sig
                    .decl
                    .output
                    .map(|e| resolve_ty_expr(tcx, e))
                    .unwrap_or_else(|| tcx.mk_unit());

                // Register the function signature as a Fn type.
                let fn_ty = tcx.mk_fn(&param_tys, ret_ty);
                tcx.register_def_ty(owner_id.def_id, fn_ty);

                // Type-check the body.
                if let Some(body) = package.body(*body_id) {
                    let mut checker = FnChecker {
                        tcx,
                        package,
                        ret_ty,
                        param_tys: &param_tys,
                        locals: Vec::new(),
                    };
                    checker.check_body(body);
                }
            }
            ItemKind::Struct(struct_def, _constraints) => {
                // Extract type parameters from clause_params.
                let type_params: Vec<(String, u32)> = struct_def
                    .clause_params
                    .iter()
                    .enumerate()
                    .map(|(i, cp)| (cp.ident.name.to_string(), i as u32))
                    .collect();
                let param_names: Vec<String> = type_params.iter().map(|(n, _)| n.clone()).collect();

                // Build resolution context with type params and package.
                let res_ctx = TyResolutionCtx {
                    type_params: &type_params,
                    package,
                };

                let mut fields = Vec::new();
                for (i, field) in struct_def.fields.iter().enumerate() {
                    let field_ty = resolve_ty_expr_in(tcx, &field.ty, Some(&res_ctx));
                    tcx.register_node_ty(field.hir_id, field_ty);
                    let field_ty_static =
                        unsafe { std::mem::transmute::<ty::Ty<'_>, ty::Ty<'static>>(field_ty) };
                    fields.push(FieldDef {
                        name: field.ident.to_string(),
                        index: i as u32,
                        ty: field_ty_static,
                    });
                }

                let adt_id = AdtId(owner_id.def_id);
                let adt_def = AdtDef {
                    name: item.ident.to_string(),
                    kind: AdtKind::Struct,
                    fields,
                    type_params: param_names.clone(),
                };
                tcx.register_adt_def(adt_id, adt_def);

                // Register the struct type with param types as args.
                let param_tys: Vec<Ty<'_>> = type_params
                    .iter()
                    .map(|(name, idx)| tcx.mk_param(*idx, name.clone()))
                    .collect();
                let adt_ty = tcx.mk_adt(adt_id, &param_tys);
                tcx.register_def_ty(owner_id.def_id, adt_ty);
            }
            ItemKind::Mod(_) | ItemKind::Use(_) | ItemKind::Err => {}
            // TODO: Enum, Trait, Impl, TypeAlias
            _ => {}
        }
    }
}

/// Per-function type checking context.
struct FnChecker<'a, 'tcx> {
    tcx: &'tcx TyCtxt,
    package: &'a Package<'a>,
    ret_ty: Ty<'tcx>,
    param_tys: &'a [Ty<'tcx>],
    /// Local variable names → their types. Used for path resolution.
    locals: Vec<(String, Ty<'tcx>)>,
}

impl<'a, 'tcx> FnChecker<'a, 'tcx> {
    fn check_body(&mut self, body: &Body<'_>) {
        // Register parameter types and track param names.
        for (i, param) in body.params.iter().enumerate() {
            if let Some(&ty) = self.param_tys.get(i) {
                self.tcx.register_node_ty(param.hir_id, ty);
                // Extract param name from pattern.
                if let hir::PatternKind::Binding(_, ident, _) = &param.pat.kind {
                    self.locals.push((ident.name.to_string(), ty));
                }
            }
        }

        // Check the body expression.
        let body_ty = self.check_expr(body.value);

        // The body's inferred type should match the declared return type.
        // For now, just record it (type mismatch diagnostics come later).
        let _ = (body_ty, self.ret_ty);
    }

    fn check_expr(&mut self, expr: &Expr<'_>) -> Ty<'tcx> {
        let ty = self.infer_expr(expr);
        self.tcx.register_node_ty(expr.hir_id, ty);
        ty
    }

    fn infer_expr(&mut self, expr: &Expr<'_>) -> Ty<'tcx> {
        match &expr.kind {
            // ── Literals ─────────────────────────────────────────────────
            ExprKind::Lit(lit) => match &lit.kind {
                LitKind::Integer(_) => self.tcx.mk_primitive(PrimTy::I64),
                LitKind::Float(_) => self.tcx.mk_primitive(PrimTy::F64),
                LitKind::String(_) => self.tcx.mk_primitive(PrimTy::Str),
                LitKind::Bool(_) => self.tcx.mk_primitive(PrimTy::Bool),
                LitKind::Char(_) => self.tcx.mk_primitive(PrimTy::Char),
                LitKind::Symbol(_) => self.tcx.mk_primitive(PrimTy::Str),
            },

            // ── Path (variable / function reference) ─────────────────────
            ExprKind::Path(path) => {
                if path.segments.len() == 1 {
                    let name = path.segments[0].ident.name.as_str();
                    // Look up in locals (reverse for shadowing).
                    for (n, ty) in self.locals.iter().rev() {
                        if n == name {
                            return *ty;
                        }
                    }
                    // Look up in top-level definitions.
                    for (oid, info) in self.package.owners() {
                        let it = info.node.expect_item();
                        if it.ident.name.as_str() == name {
                            if let Some(ty) = self.tcx.def_ty(oid.def_id) {
                                return ty;
                            }
                        }
                    }
                }
                self.tcx.mk_infer()
            }

            // ── Binary operations ────────────────────────────────────────
            ExprKind::Binary(op, lhs, rhs) => {
                let lhs_ty = self.check_expr(lhs);
                let rhs_ty = self.check_expr(rhs);
                match op {
                    BinOp::Eq
                    | BinOp::Ne
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::Le
                    | BinOp::Ge
                    | BinOp::And
                    | BinOp::Or => self.tcx.mk_primitive(PrimTy::Bool),
                    _ => {
                        // Arithmetic: result type is same as operands.
                        // Prefer the non-infer type if one is inferred.
                        if lhs_ty.is_infer() { rhs_ty } else { lhs_ty }
                    }
                }
            }

            // ── Unary operations ─────────────────────────────────────────
            ExprKind::Unary(op, operand) => {
                let operand_ty = self.check_expr(operand);
                match op {
                    UnOp::Not => self.tcx.mk_primitive(PrimTy::Bool),
                    UnOp::Neg => operand_ty,
                }
            }

            // ── Call ─────────────────────────────────────────────────────
            ExprKind::Call(callee, args) => {
                let callee_ty = self.check_expr(callee);
                for arg in args.iter() {
                    match arg {
                        Arg::Positional(e)
                        | Arg::Named(_, e)
                        | Arg::Expand(e)
                        | Arg::Implicit(e) => {
                            self.check_expr(e);
                        }
                        Arg::DependencyCatch(_, e) => {
                            self.check_expr(e);
                        }
                    }
                }
                // Extract return type from function type.
                match callee_ty.kind() {
                    ty::TyKind::Fn(_, ret) => *ret,
                    _ => self.tcx.mk_infer(),
                }
            }

            // ── If expression ────────────────────────────────────────────
            ExprKind::If(cond, then_block, else_expr) => {
                self.check_expr(cond);
                let then_ty = self.check_block(then_block);
                if let Some(else_e) = else_expr {
                    let else_ty = self.check_expr(else_e);
                    // Both branches should have the same type.
                    if then_ty.is_infer() { else_ty } else { then_ty }
                } else {
                    self.tcx.mk_unit()
                }
            }

            // ── Block ────────────────────────────────────────────────────
            ExprKind::Block(block) => self.check_block(block),

            // ── Return ───────────────────────────────────────────────────
            ExprKind::Return(val) => {
                if let Some(e) = val {
                    self.check_expr(e);
                }
                self.tcx.mk_never()
            }

            // ── Assign ───────────────────────────────────────────────────
            ExprKind::Assign(lhs, rhs) => {
                self.check_expr(lhs);
                self.check_expr(rhs);
                self.tcx.mk_unit()
            }

            // ── Tuple ────────────────────────────────────────────────────
            ExprKind::Tuple(elems) => {
                let tys: Vec<_> = elems.iter().map(|e| self.check_expr(e)).collect();
                self.tcx.mk_tuple(&tys)
            }

            // ── Field access ─────────────────────────────────────────────
            ExprKind::Field(base, field_ident) => {
                let base_ty = self.check_expr(base);
                // If base is an ADT, look up the field type from its definition.
                if let ty::TyKind::Adt(adt_id, _) = base_ty.kind() {
                    if let Some(adt_def) = self.tcx.adt_def(*adt_id) {
                        let field_name = field_ident.name.as_str();
                        if let Some(idx) = adt_def.field_index(field_name) {
                            // Look up field type from the struct item's fields.
                            for (_owner_id, info) in self.package.owners() {
                                let it = info.node.expect_item();
                                if let ItemKind::Struct(sdef, _) = &it.kind {
                                    if it.ident.name.as_str() == adt_def.name {
                                        if let Some(f) = sdef.fields.get(idx as usize) {
                                            return resolve_ty_expr(self.tcx, &f.ty);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                self.tcx.mk_infer()
            }

            // ── Ref / Deref ──────────────────────────────────────────────
            ExprKind::Ref(inner) => {
                let inner_ty = self.check_expr(inner);
                // Produce a pointer type (matches `*T` field declarations).
                self.tcx.mk_ptr(inner_ty, hir::Mutability::Immutable)
            }
            ExprKind::Deref(inner) => {
                let inner_ty = self.check_expr(inner);
                match inner_ty.kind() {
                    ty::TyKind::Ref(t, _) | ty::TyKind::Ptr(t, _) => *t,
                    _ => self.tcx.mk_infer(),
                }
            }

            // ── Match ────────────────────────────────────────────────────
            ExprKind::Match(scrutinee, arms) => {
                self.check_expr(scrutinee);
                let mut result_ty = self.tcx.mk_infer();
                for arm in arms.iter() {
                    let arm_ty = self.check_expr(arm.body);
                    if result_ty.is_infer() {
                        result_ty = arm_ty;
                    }
                }
                result_ty
            }

            // ── Struct literal ───────────────────────────────────────────
            ExprKind::StructLit(path, fields) => {
                // Check all field expressions first.
                let field_tys: Vec<_> = fields
                    .iter()
                    .map(|f| (f.ident.name.to_string(), self.check_expr(f.expr)))
                    .collect();

                // Resolve path to ADT type by name lookup.
                let name = path
                    .segments
                    .last()
                    .map(|s| s.ident.name.as_str())
                    .unwrap_or("");
                for (owner_id, info) in self.package.owners() {
                    let it = info.node.expect_item();
                    if matches!(it.kind, ItemKind::Struct(..)) && it.ident.name.as_str() == name {
                        let adt_id = AdtId(owner_id.def_id);
                        if let Some(def) = self.tcx.adt_def(adt_id) {
                            if def.type_params.is_empty() {
                                return self.tcx.mk_adt(adt_id, &[]);
                            }
                            // Infer generic args from field values.
                            let mut inferred: Vec<Ty<'tcx>> =
                                vec![self.tcx.mk_infer(); def.type_params.len()];
                            for field_def in &def.fields {
                                if let Some((_, val_ty)) =
                                    field_tys.iter().find(|(n, _)| *n == field_def.name)
                                {
                                    self.infer_param_from(field_def.ty, *val_ty, &mut inferred);
                                }
                            }
                            return self.tcx.mk_adt(adt_id, &inferred);
                        }
                        return self.tcx.mk_adt(adt_id, &[]);
                    }
                }
                self.tcx.mk_infer()
            }

            // ── Array ────────────────────────────────────────────────────
            ExprKind::Array(elems) => {
                let mut elem_ty = self.tcx.mk_infer();
                for e in elems.iter() {
                    let t = self.check_expr(e);
                    if elem_ty.is_infer() {
                        elem_ty = t;
                    }
                }
                let len = elems.len() as u64;
                self.tcx.mk_array(elem_ty, len)
            }

            // ── Null / Undefined ─────────────────────────────────────────
            ExprKind::Null => {
                // null is a null pointer — `*const _`
                let inner = self.tcx.mk_infer();
                self.tcx.mk_ptr(inner, hir::Mutability::Immutable)
            }
            ExprKind::Undefined => self.tcx.mk_infer(),

            // ── Cast ─────────────────────────────────────────────────────
            ExprKind::Cast(_expr, ty_expr) => resolve_ty_expr(self.tcx, ty_expr),

            // ── Type expressions (when used as values) ───────────────────
            ExprKind::TyPtr(_)
            | ExprKind::TyOptional(_)
            | ExprKind::TyFn(_, _)
            | ExprKind::TyPlaceholder
            | ExprKind::TyNoReturn
            | ExprKind::TyVoid
            | ExprKind::TyAny => {
                // Type expressions — their "value" is the type itself.
                resolve_ty_expr(self.tcx, expr)
            }

            _ => self.tcx.mk_infer(),
        }
    }

    fn check_block(&mut self, block: &hir::Block<'_>) -> Ty<'tcx> {
        for stmt in block.stmts.iter() {
            match &stmt.kind {
                StmtKind::Let(let_stmt) => {
                    let init_ty = let_stmt
                        .init
                        .map(|e| self.check_expr(e))
                        .unwrap_or_else(|| self.tcx.mk_infer());

                    let declared_ty = let_stmt.ty.map(|e| resolve_ty_expr(self.tcx, e));

                    let binding_ty = declared_ty.unwrap_or(init_ty);
                    self.tcx.register_node_ty(let_stmt.hir_id, binding_ty);

                    // Track the local variable for path resolution.
                    if let hir::PatternKind::Binding(_, ident, _) = &let_stmt.pat.kind {
                        self.locals.push((ident.name.to_string(), binding_ty));
                    }
                }
                StmtKind::Semi(e) | StmtKind::Expr(e) => {
                    self.check_expr(e);
                }
                StmtKind::Item(_) => {}
            }
        }

        // Block result is the tail expression, or unit.
        block
            .expr
            .map(|e| self.check_expr(e))
            .unwrap_or_else(|| self.tcx.mk_unit())
    }

    /// Try to infer concrete types for type parameters by matching a
    /// field's declared type (which may contain `Param`) against the
    /// actual value type.
    fn infer_param_from(&self, declared: Ty<'_>, actual: Ty<'tcx>, inferred: &mut [Ty<'tcx>]) {
        match declared.kind() {
            TyKind::Param(p) => {
                let idx = p.index as usize;
                if idx < inferred.len() && inferred[idx].is_infer() {
                    inferred[idx] = actual;
                }
            }
            TyKind::Ptr(inner, _) | TyKind::Ref(inner, _) => {
                if let TyKind::Ptr(a_inner, _) | TyKind::Ref(a_inner, _) = actual.kind() {
                    self.infer_param_from(*inner, *a_inner, inferred);
                }
            }
            TyKind::Adt(_, args) => {
                if let TyKind::Adt(_, a_args) = actual.kind() {
                    for (d, a) in args.iter().zip(a_args.iter()) {
                        self.infer_param_from(*d, *a, inferred);
                    }
                }
            }
            _ => {}
        }
    }
}
