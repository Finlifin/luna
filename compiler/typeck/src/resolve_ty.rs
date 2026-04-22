//! Resolve HIR type expressions (`Expr`) into semantic types (`Ty`).
//!
//! In the Flurry HIR, types are represented as expressions (e.g. `Path("i32")`
//! for the `i32` type, `TyPtr(inner)` for pointer types). This module
//! converts those syntactic type expressions into interned `Ty<'tcx>` values.

use hir::Package;
use hir::expr::{Expr, ExprKind};
use hir::item::ItemKind;
use ty::{AdtId, PrimTy, Ty, TyCtxt};

/// Context for resolving type expressions, carrying type parameter scope
/// and access to the package for user-defined type lookup.
pub struct TyResolutionCtx<'a, 'hir> {
    /// Type parameters in scope: (name, param_index).
    pub type_params: &'a [(String, u32)],
    /// The HIR package (for resolving struct/enum names to DefIds).
    pub package: &'a Package<'hir>,
}

/// Resolve a HIR type-expression into a semantic type (no generic context).
///
/// Returns `Ty::Error` for unrecognised or invalid type expressions.
pub fn resolve_ty_expr<'tcx>(tcx: &'tcx TyCtxt, expr: &Expr<'_>) -> Ty<'tcx> {
    resolve_ty_expr_in(tcx, expr, None)
}

/// Resolve a HIR type-expression with an optional generic context.
pub fn resolve_ty_expr_in<'tcx>(
    tcx: &'tcx TyCtxt,
    expr: &Expr<'_>,
    ctx: Option<&TyResolutionCtx<'_, '_>>,
) -> Ty<'tcx> {
    match &expr.kind {
        ExprKind::Path(path) => {
            if path.segments.len() == 1 {
                let seg = &path.segments[0];
                let name = &*seg.ident.name;

                // 1. Primitive types
                if let Some(prim) = prim_ty_from_name(name) {
                    return tcx.mk_primitive(prim);
                }
                // 2. Special built-in type names
                match name {
                    "void" | "Void" => return tcx.mk_unit(),
                    "NoReturn" => return tcx.mk_never(),
                    _ => {}
                }
                // 3. Type parameter lookup
                if let Some(ctx) = ctx {
                    for (pname, idx) in ctx.type_params {
                        if pname == name {
                            return tcx.mk_param(*idx, name.to_string());
                        }
                    }
                }
                // 4. User-defined type (struct/enum) lookup
                if let Some(ctx) = ctx {
                    for (owner_id, info) in ctx.package.owners() {
                        let item = info.node.expect_item();
                        if item.ident.name.as_str() == name {
                            if matches!(item.kind, ItemKind::Struct(..) | ItemKind::Enum(..)) {
                                let adt_id = AdtId(owner_id.def_id);
                                // Resolve generic args if present
                                let args: Vec<Ty<'tcx>> = seg
                                    .args
                                    .iter()
                                    .map(|ga| match ga {
                                        hir::common::GenericArg::Expr(e) => {
                                            resolve_ty_expr_in(tcx, e, Some(ctx))
                                        }
                                        hir::common::GenericArg::Optional(_, e) => {
                                            resolve_ty_expr_in(tcx, e, Some(ctx))
                                        }
                                    })
                                    .collect();
                                return tcx.mk_adt(adt_id, &args);
                            }
                        }
                    }
                }
            }
            tcx.mk_error()
        }

        ExprKind::TyPtr(inner) => {
            let inner_ty = resolve_ty_expr_in(tcx, inner, ctx);
            tcx.mk_ptr(inner_ty, hir::common::Mutability::Immutable)
        }

        ExprKind::TyOptional(inner) => {
            let inner_ty = resolve_ty_expr_in(tcx, inner, ctx);
            tcx.mk_optional(inner_ty)
        }

        ExprKind::TyFn(params, ret) => {
            let param_tys: Vec<_> = params
                .iter()
                .map(|p| resolve_ty_expr_in(tcx, p, ctx))
                .collect();
            let ret_ty = resolve_ty_expr_in(tcx, ret, ctx);
            tcx.mk_fn(&param_tys, ret_ty)
        }

        ExprKind::TyPlaceholder => tcx.mk_infer(),

        ExprKind::TyNoReturn => tcx.mk_never(),

        ExprKind::TyVoid => tcx.mk_unit(),

        ExprKind::Tuple(elems) => {
            let tys: Vec<_> = elems
                .iter()
                .map(|e| resolve_ty_expr_in(tcx, e, ctx))
                .collect();
            tcx.mk_tuple(&tys)
        }

        _ => tcx.mk_error(),
    }
}

/// Map primitive type name strings to `PrimTy`.
///
/// Phase 1: Comptime types (`Integer`, `Float`, `Real`) are lowered to
/// their default runtime representations (`i64`, `f64`).
fn prim_ty_from_name(name: &str) -> Option<PrimTy> {
    match name {
        // Signed integers
        "i8" | "I8" => Some(PrimTy::I8),
        "i16" | "I16" => Some(PrimTy::I16),
        "i32" | "I32" => Some(PrimTy::I32),
        "i64" | "I64" => Some(PrimTy::I64),
        "isize" | "Isize" => Some(PrimTy::Isize),
        // Unsigned integers
        "u8" | "U8" => Some(PrimTy::U8),
        "u16" | "U16" => Some(PrimTy::U16),
        "u32" | "U32" => Some(PrimTy::U32),
        "u64" | "U64" => Some(PrimTy::U64),
        "usize" | "Usize" => Some(PrimTy::Usize),
        // Floating point
        "f32" | "F32" => Some(PrimTy::F32),
        "f64" | "F64" => Some(PrimTy::F64),
        // Comptime → default runtime: Integer → i64, Float/Real → f64
        "Int" | "Integer" => Some(PrimTy::I64),
        "Float" | "Real" => Some(PrimTy::F64),
        // Other scalars
        "Bool" | "bool" => Some(PrimTy::Bool),
        "Char" | "char" => Some(PrimTy::Char),
        "String" | "str" | "Str" => Some(PrimTy::Str),
        _ => None,
    }
}
