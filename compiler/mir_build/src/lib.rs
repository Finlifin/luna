//! MIR builder – lowers typed HIR into MIR.
//!
//! For each function definition in the HIR package, the builder creates a
//! MIR `Body` with basic blocks, statements, and terminators.

use hir::body::Body as HirBody;
use hir::common::LitKind;
use hir::expr::{Arg, ExprKind, StmtKind};
use hir::item::{FnParamTy, ItemKind};
use hir::{Expr, Package};
use mir::*;
use ty::{AdtId, PrimTy, Ty, TyCtxt, TyKind};

/// Lower all function bodies in the HIR package to MIR.
///
/// Returns a `Vec<mir::Body>` — one per function definition.
pub fn build_mir<'tcx>(package: &Package<'_>, tcx: &'tcx TyCtxt) -> Vec<mir::Body<'tcx>> {
    let mut bodies = Vec::new();

    for (owner_id, info) in package.owners() {
        let item = info.node.expect_item();
        if let ItemKind::Fn(sig, body_id) = &item.kind {
            if let Some(hir_body) = package.body(*body_id) {
                let fn_ty = tcx.def_ty(owner_id.def_id);
                let (param_tys, ret_ty) = match fn_ty.map(|t| t.kind()) {
                    Some(TyKind::Fn(params, ret)) => (params.to_vec(), *ret),
                    _ => {
                        // Fallback: resolve from signature
                        let params: Vec<_> = sig
                            .decl
                            .inputs
                            .iter()
                            .map(|p| match p {
                                FnParamTy::Typed(_, ty_expr, _, _) => tcx
                                    .node_ty(ty_expr.hir_id)
                                    .unwrap_or_else(|| tcx.mk_infer()),
                                _ => tcx.mk_infer(),
                            })
                            .collect();
                        let ret = sig
                            .decl
                            .output
                            .and_then(|e| tcx.node_ty(e.hir_id))
                            .unwrap_or_else(|| tcx.mk_unit());
                        (params, ret)
                    }
                };

                let mut builder = Builder::new(tcx, package, owner_id.def_id, &param_tys, ret_ty);

                // Add parameter names
                for (i, param) in sig.decl.inputs.iter().enumerate() {
                    let name = match param {
                        FnParamTy::Typed(ident, _, _, _)
                        | FnParamTy::Optional { ident, .. }
                        | FnParamTy::Variadic(ident, _, _, _) => ident.name.to_string(),
                    };
                    // Local _0 is return, params are _1.._n
                    builder.body.local_decls[i + 1].name = Some(name.clone());
                    builder.param_names.push(name);
                }

                // Set the function name on the return-place local for display
                builder.body.local_decls[0].name = Some(item.ident.name.to_string());

                builder.lower_body(hir_body);
                bodies.push(builder.body);
            }
        }
    }

    bodies
}

/// Per-function MIR builder.
struct Builder<'a, 'tcx> {
    tcx: &'tcx TyCtxt,
    package: &'a Package<'a>,
    body: mir::Body<'tcx>,
    /// The current block we are appending to.
    current_block: BasicBlock,
    /// Parameter names (for variable lookup).
    param_names: Vec<String>,
    /// Local variable names → Local index (for `let` bindings & params).
    locals: Vec<(String, Local)>,
    /// Phantom to keep the HirBody lifetime alive.
    _phantom: std::marker::PhantomData<&'a ()>,
}

impl<'a, 'tcx> Builder<'a, 'tcx> {
    fn new(
        tcx: &'tcx TyCtxt,
        package: &'a Package<'a>,
        def_id: hir::LocalDefId,
        param_tys: &[Ty<'tcx>],
        ret_ty: Ty<'tcx>,
    ) -> Self {
        let arg_count = param_tys.len();
        let mut body = mir::Body::new(def_id, arg_count);

        // _0: return place
        body.push_local(LocalDecl {
            ty: ret_ty,
            name: None,
        });

        // _1 .. _n: parameters
        for ty in param_tys {
            body.push_local(LocalDecl {
                ty: *ty,
                name: None,
            });
        }

        // Create the entry block (bb0).
        let entry = body.new_block();

        Builder {
            tcx,
            package,
            body,
            current_block: entry,
            param_names: Vec::new(),
            locals: Vec::new(),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Create a fresh temporary local.
    fn temp(&mut self, ty: Ty<'tcx>) -> Local {
        self.body.push_local(LocalDecl { ty, name: None })
    }

    /// Create a named local.
    fn named_local(&mut self, name: &str, ty: Ty<'tcx>) -> Local {
        let local = self.body.push_local(LocalDecl {
            ty,
            name: Some(name.to_string()),
        });
        self.locals.push((name.to_string(), local));
        local
    }

    /// Push a statement to the current block.
    fn push_stmt(&mut self, kind: StatementKind<'tcx>) {
        self.body.basic_blocks[self.current_block.index()]
            .statements
            .push(Statement { kind });
    }

    /// Set the terminator for the current block.
    fn terminate(&mut self, kind: TerminatorKind<'tcx>) {
        self.body.basic_blocks[self.current_block.index()].terminator = Some(Terminator { kind });
    }

    /// Look up a variable name and return its Local.
    fn resolve_name(&self, name: &str) -> Option<Local> {
        // Search locals in reverse (inner scopes shadow outer)
        for (n, local) in self.locals.iter().rev() {
            if n == name {
                return Some(*local);
            }
        }
        // Search parameters
        for (i, param_name) in self.param_names.iter().enumerate() {
            if param_name == name {
                // Parameters are _1.._n
                return Some(Local::new((i + 1) as u32));
            }
        }
        None
    }

    /// Resolve a field name to its index within the base expression's ADT.
    fn resolve_field_index(&self, base: &Expr<'_>, field: &hir::Ident) -> u32 {
        let base_ty = self.tcx.node_ty(base.hir_id);
        if let Some(ty) = base_ty {
            if let TyKind::Adt(adt_id, _) = ty.kind() {
                if let Some(def) = self.tcx.adt_def(*adt_id) {
                    let name = field.name.to_string();
                    if let Some(idx) = def.field_index(&name) {
                        return idx;
                    }
                }
            }
        }
        0 // fallback
    }

    fn lower_body(&mut self, hir_body: &HirBody<'_>) {
        let result = self.lower_expr(hir_body.value);
        // Store result into return place
        self.push_stmt(StatementKind::Assign(
            Place::return_place(),
            Rvalue::Use(result),
        ));
        self.terminate(TerminatorKind::Return);
    }

    /// Lower an HIR expression, returning an `Operand` representing the value.
    fn lower_expr(&mut self, expr: &Expr<'_>) -> Operand<'tcx> {
        match &expr.kind {
            // ── Literals ─────────────────────────────────────────────
            ExprKind::Lit(lit) => {
                let (ty, kind) = match &lit.kind {
                    LitKind::Integer(v) => (self.tcx.mk_primitive(PrimTy::I64), ConstKind::Int(*v)),
                    LitKind::Float(v) => (self.tcx.mk_primitive(PrimTy::F64), ConstKind::Float(*v)),
                    LitKind::Bool(v) => (self.tcx.mk_primitive(PrimTy::Bool), ConstKind::Bool(*v)),
                    LitKind::Char(v) => (self.tcx.mk_primitive(PrimTy::Char), ConstKind::Char(*v)),
                    LitKind::String(v) => (
                        self.tcx.mk_primitive(PrimTy::Str),
                        ConstKind::Str(v.clone()),
                    ),
                    LitKind::Symbol(v) => (
                        self.tcx.mk_primitive(PrimTy::Str),
                        ConstKind::Str(v.to_string()),
                    ),
                };
                Operand::Constant(Constant { ty, kind })
            }

            // ── Path (variable reference) ────────────────────────────
            ExprKind::Path(path) => {
                if path.segments.len() == 1 {
                    let name = &*path.segments[0].ident.name;
                    if let Some(local) = self.resolve_name(name) {
                        return Operand::Copy(Place::local(local));
                    }
                    // Could be a function reference — check def_types
                    // For now, create a fn-def constant if we have a type
                    // (simplified: we don't have DefId lookup from path yet)
                }
                // Unresolved path — emit a placeholder
                let ty = self.tcx.mk_infer();
                Operand::Constant(Constant {
                    ty,
                    kind: ConstKind::Unit,
                })
            }

            // ── Binary operation ─────────────────────────────────────
            ExprKind::Binary(op, lhs, rhs) => {
                let lhs_op = self.lower_expr(lhs);
                let rhs_op = self.lower_expr(rhs);
                let result_ty = self
                    .tcx
                    .node_ty(expr.hir_id)
                    .unwrap_or_else(|| self.tcx.mk_infer());
                let tmp = self.temp(result_ty);
                self.push_stmt(StatementKind::Assign(
                    Place::local(tmp),
                    Rvalue::BinaryOp(*op, lhs_op, rhs_op),
                ));
                Operand::Copy(Place::local(tmp))
            }

            // ── Unary operation ──────────────────────────────────────
            ExprKind::Unary(op, operand) => {
                let operand = self.lower_expr(operand);
                let result_ty = self
                    .tcx
                    .node_ty(expr.hir_id)
                    .unwrap_or_else(|| self.tcx.mk_infer());
                let tmp = self.temp(result_ty);
                self.push_stmt(StatementKind::Assign(
                    Place::local(tmp),
                    Rvalue::UnaryOp(*op, operand),
                ));
                Operand::Copy(Place::local(tmp))
            }

            // ── Call ─────────────────────────────────────────────────
            ExprKind::Call(callee, args) => {
                // Special-case: if callee is a path, treat it as a function name.
                let func = if let ExprKind::Path(path) = &callee.kind {
                    if path.segments.len() == 1 {
                        let name = path.segments[0].ident.name.to_string();
                        let fn_ty = self
                            .tcx
                            .node_ty(callee.hir_id)
                            .unwrap_or_else(|| self.tcx.mk_infer());
                        Operand::Constant(Constant {
                            ty: fn_ty,
                            kind: ConstKind::FnName(name),
                        })
                    } else {
                        self.lower_expr(callee)
                    }
                } else {
                    self.lower_expr(callee)
                };

                let mir_args: Vec<_> = args
                    .iter()
                    .map(|arg| match arg {
                        Arg::Positional(e)
                        | Arg::Named(_, e)
                        | Arg::Expand(e)
                        | Arg::Implicit(e) => self.lower_expr(e),
                        Arg::DependencyCatch(_, e) => self.lower_expr(e),
                    })
                    .collect();

                let ret_ty = self
                    .tcx
                    .node_ty(expr.hir_id)
                    .unwrap_or_else(|| self.tcx.mk_infer());
                let dest = self.temp(ret_ty);
                let next_bb = self.body.new_block();

                self.terminate(TerminatorKind::Call {
                    func,
                    args: mir_args,
                    destination: Place::local(dest),
                    target: Some(next_bb),
                });

                self.current_block = next_bb;
                Operand::Copy(Place::local(dest))
            }

            // ── If expression ────────────────────────────────────────
            ExprKind::If(cond, then_block, else_expr) => {
                let cond_op = self.lower_expr(cond);

                // Allocate result local — try node_ty first, fall back to
                // the function's return type as a heuristic.
                let result_ty = self
                    .tcx
                    .node_ty(expr.hir_id)
                    .filter(|t| !t.is_unit() && !t.is_infer())
                    .unwrap_or(self.body.local_decls[Local::RETURN_PLACE.index()].ty);
                let result_local = self.temp(result_ty);

                let then_bb = self.body.new_block();
                let else_bb = self.body.new_block();
                let join_bb = self.body.new_block();

                // Switch on condition: true (1) → then, otherwise → else
                self.terminate(TerminatorKind::SwitchInt {
                    discr: cond_op,
                    targets: SwitchTargets {
                        values: vec![(1, then_bb)],
                        otherwise: else_bb,
                    },
                });

                // ── Then block ───────────────────────────────
                self.current_block = then_bb;
                let then_val = self.lower_block(then_block);
                self.push_stmt(StatementKind::Assign(
                    Place::local(result_local),
                    Rvalue::Use(then_val),
                ));
                self.terminate(TerminatorKind::Goto { target: join_bb });

                // ── Else block ───────────────────────────────
                self.current_block = else_bb;
                if let Some(else_e) = else_expr {
                    let else_val = self.lower_expr(else_e);
                    self.push_stmt(StatementKind::Assign(
                        Place::local(result_local),
                        Rvalue::Use(else_val),
                    ));
                } else {
                    // No else: result is unit
                    self.push_stmt(StatementKind::Assign(
                        Place::local(result_local),
                        Rvalue::Use(Operand::Constant(Constant {
                            ty: self.tcx.mk_unit(),
                            kind: ConstKind::Unit,
                        })),
                    ));
                }
                self.terminate(TerminatorKind::Goto { target: join_bb });

                self.current_block = join_bb;
                Operand::Copy(Place::local(result_local))
            }

            // ── Block expression ─────────────────────────────────────
            ExprKind::Block(block) => self.lower_block(block),

            // ── Return ───────────────────────────────────────────────
            ExprKind::Return(val) => {
                let operand = if let Some(e) = val {
                    self.lower_expr(e)
                } else {
                    Operand::Constant(Constant {
                        ty: self.tcx.mk_unit(),
                        kind: ConstKind::Unit,
                    })
                };
                self.push_stmt(StatementKind::Assign(
                    Place::return_place(),
                    Rvalue::Use(operand),
                ));
                self.terminate(TerminatorKind::Return);
                // After a return, create an unreachable continuation block
                let dead_bb = self.body.new_block();
                self.current_block = dead_bb;
                Operand::Constant(Constant {
                    ty: self.tcx.mk_never(),
                    kind: ConstKind::Unit,
                })
            }

            // ── Assign ───────────────────────────────────────────────
            ExprKind::Assign(lhs, rhs) => {
                let rhs_op = self.lower_expr(rhs);
                let place = self.lower_place(lhs);
                self.push_stmt(StatementKind::Assign(place, Rvalue::Use(rhs_op)));
                Operand::Constant(Constant {
                    ty: self.tcx.mk_unit(),
                    kind: ConstKind::Unit,
                })
            }

            // ── Tuple ────────────────────────────────────────────────
            ExprKind::Tuple(elems) => {
                let ops: Vec<_> = elems.iter().map(|e| self.lower_expr(e)).collect();
                let ty = self
                    .tcx
                    .node_ty(expr.hir_id)
                    .unwrap_or_else(|| self.tcx.mk_unit());
                let tmp = self.temp(ty);
                self.push_stmt(StatementKind::Assign(
                    Place::local(tmp),
                    Rvalue::Aggregate(AggregateKind::Tuple, ops),
                ));
                Operand::Copy(Place::local(tmp))
            }

            // ── Array ────────────────────────────────────────────────
            ExprKind::Array(elems) => {
                let ops: Vec<_> = elems.iter().map(|e| self.lower_expr(e)).collect();
                let ty = self
                    .tcx
                    .node_ty(expr.hir_id)
                    .unwrap_or_else(|| self.tcx.mk_infer());
                let tmp = self.temp(ty);
                self.push_stmt(StatementKind::Assign(
                    Place::local(tmp),
                    Rvalue::Aggregate(AggregateKind::Array, ops),
                ));
                Operand::Copy(Place::local(tmp))
            }

            // ── Ref ──────────────────────────────────────────────────
            ExprKind::Ref(inner) => {
                let place = self.lower_place(inner);
                let ty = self
                    .tcx
                    .node_ty(expr.hir_id)
                    .unwrap_or_else(|| self.tcx.mk_infer());
                let tmp = self.temp(ty);
                self.push_stmt(StatementKind::Assign(Place::local(tmp), Rvalue::Ref(place)));
                Operand::Copy(Place::local(tmp))
            }

            // ── Field access ─────────────────────────────────────────
            ExprKind::Field(base, ident) => {
                let base_op = self.lower_expr(base);
                // We need the base in a place; if it's a Copy of a place, use that.
                let base_place = match &base_op {
                    Operand::Copy(p) | Operand::Move(p) => p.clone(),
                    _ => {
                        // Materialize the base into a temp.
                        let base_ty = self
                            .tcx
                            .node_ty(base.hir_id)
                            .unwrap_or_else(|| self.tcx.mk_infer());
                        let tmp = self.temp(base_ty);
                        self.push_stmt(StatementKind::Assign(
                            Place::local(tmp),
                            Rvalue::Use(base_op),
                        ));
                        Place::local(tmp)
                    }
                };
                let field_idx = self.resolve_field_index(base, ident);
                let mut place = base_place;
                place.projection.push(PlaceElem::Field(field_idx));

                let ty = self
                    .tcx
                    .node_ty(expr.hir_id)
                    .unwrap_or_else(|| self.tcx.mk_infer());
                let tmp = self.temp(ty);
                self.push_stmt(StatementKind::Assign(
                    Place::local(tmp),
                    Rvalue::Use(Operand::Copy(place)),
                ));
                Operand::Copy(Place::local(tmp))
            }

            // ── Struct literal ────────────────────────────────────────
            ExprKind::StructLit(path, fields) => {
                let ty = self
                    .tcx
                    .node_ty(expr.hir_id)
                    .unwrap_or_else(|| self.tcx.mk_infer());

                // Resolve the struct's def_id from the type.
                let def_id = match ty.kind() {
                    TyKind::Adt(adt_id, _) => adt_id.0,
                    _ => {
                        // Fallback: look up by name in the package.
                        let name = path
                            .segments
                            .last()
                            .map(|s| s.ident.name.to_string())
                            .unwrap_or_default();
                        let mut found = None;
                        for (oid, info) in self.package.owners() {
                            let it = info.node.expect_item();
                            if matches!(it.kind, ItemKind::Struct(..))
                                && it.ident.name.to_string() == name
                            {
                                found = Some(oid.def_id);
                                break;
                            }
                        }
                        found.unwrap_or(hir::LocalDefId::new(0))
                    }
                };

                // Order field operands according to the AdtDef field order.
                let adt_def = self.tcx.adt_def(AdtId(def_id));
                let mut ops = Vec::new();
                if let Some(ref def) = adt_def {
                    for field_def in &def.fields {
                        // Find the matching field expression.
                        let field_expr = fields
                            .iter()
                            .find(|f| f.ident.name.to_string() == field_def.name);
                        if let Some(fe) = field_expr {
                            ops.push(self.lower_expr(fe.expr));
                        } else {
                            ops.push(Operand::Constant(Constant {
                                ty: self.tcx.mk_infer(),
                                kind: ConstKind::Unit,
                            }));
                        }
                    }
                } else {
                    // No AdtDef — just lower fields in source order.
                    for f in fields.iter() {
                        ops.push(self.lower_expr(f.expr));
                    }
                }

                let tmp = self.temp(ty);
                self.push_stmt(StatementKind::Assign(
                    Place::local(tmp),
                    Rvalue::Aggregate(AggregateKind::Adt(def_id), ops),
                ));
                Operand::Copy(Place::local(tmp))
            }

            // ── Null literal ──────────────────────────────────────────
            ExprKind::Null => {
                let ty = self
                    .tcx
                    .node_ty(expr.hir_id)
                    .unwrap_or_else(|| {
                        let inner = self.tcx.mk_infer();
                        self.tcx.mk_ptr(inner, hir::common::Mutability::Immutable)
                    });
                Operand::Constant(Constant {
                    ty,
                    kind: ConstKind::Null,
                })
            }

            // ── Everything else: produce a placeholder ───────────────
            _ => {
                let ty = self
                    .tcx
                    .node_ty(expr.hir_id)
                    .unwrap_or_else(|| self.tcx.mk_infer());
                Operand::Constant(Constant {
                    ty,
                    kind: ConstKind::Unit,
                })
            }
        }
    }

    /// Lower an HIR expression into a `Place` (for the LHS of assignments).
    fn lower_place(&self, expr: &Expr<'_>) -> Place {
        match &expr.kind {
            ExprKind::Path(path) => {
                if path.segments.len() == 1 {
                    let name = &*path.segments[0].ident.name;
                    if let Some(local) = self.resolve_name(name) {
                        return Place::local(local);
                    }
                }
                // Fallback: return place (not ideal, but won't crash)
                Place::return_place()
            }
            ExprKind::Deref(inner) => {
                let mut place = self.lower_place(inner);
                place.projection.push(PlaceElem::Deref);
                place
            }
            ExprKind::Field(base, ident) => {
                let mut place = self.lower_place(base);
                let field_idx = self.resolve_field_index(base, ident);
                place.projection.push(PlaceElem::Field(field_idx));
                place
            }
            _ => Place::return_place(),
        }
    }

    /// Lower an HIR block, returning the operand for the block's value.
    fn lower_block(&mut self, block: &hir::Block<'_>) -> Operand<'tcx> {
        let stmt_count = block.stmts.len();

        for (i, stmt) in block.stmts.iter().enumerate() {
            let is_last = i + 1 == stmt_count && block.expr.is_none();

            match &stmt.kind {
                StmtKind::Let(let_stmt) => {
                    let ty = self
                        .tcx
                        .node_ty(let_stmt.hir_id)
                        .unwrap_or_else(|| self.tcx.mk_infer());

                    // Determine the binding name from pattern.
                    let name = match &let_stmt.pat.kind {
                        hir::PatternKind::Binding(_, ident, _) => ident.name.to_string(),
                        _ => format!("_let{}", self.body.local_decls.len()),
                    };

                    let local = self.named_local(&name, ty);

                    if let Some(init) = let_stmt.init {
                        let init_op = self.lower_expr(init);
                        self.push_stmt(StatementKind::Assign(
                            Place::local(local),
                            Rvalue::Use(init_op),
                        ));
                    }
                }
                StmtKind::Expr(e) if is_last => {
                    // Last expression statement with no block tail — use as value.
                    return self.lower_expr(e);
                }
                StmtKind::Semi(e) if is_last => {
                    // Last semi statement with no block tail — use as value.
                    // (AST lowering sometimes puts tail expressions as Semi.)
                    return self.lower_expr(e);
                }
                StmtKind::Semi(e) | StmtKind::Expr(e) => {
                    self.lower_expr(e);
                }
                StmtKind::Item(_) => {}
            }
        }

        if let Some(tail) = block.expr {
            self.lower_expr(tail)
        } else {
            Operand::Constant(Constant {
                ty: self.tcx.mk_unit(),
                kind: ConstKind::Unit,
            })
        }
    }
}
