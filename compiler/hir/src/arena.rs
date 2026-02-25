//! HIR arena – owns all arena-allocated HIR nodes.
//!
//! The [`HirArena`] is the backing store for every `&'hir` reference in the
//! HIR. It is owned by [`CompilerInstance`] and lives for the entire duration
//! of a compilation. All HIR nodes are allocated through the arena, and
//! references to them (`&'hir T`) are valid as long as the arena lives.
//!
//! Internally, the typed arenas store `T<'static>` and we transmute to/from
//! `T<'hir>`. This is sound because the arena guarantees that allocations
//! live as long as `&self`, and `'hir` is exactly that borrow lifetime.

use std::mem;

use rustc_arena_modified::typed_arena::TypedArena;

use crate::body::Param;
use crate::clause::ClauseConstraint;
use crate::common::{GenericArg, PathSegment};
use crate::expr::{Arm, Block, ClosureParam, Expr, FieldExpr, LetStmt, Stmt};
use crate::item::{FieldDef, Item, Variant};
use crate::pattern::{FieldPat, Pattern};
use crate::ty::{ClauseParam, TraitBound};

/// The HIR arena – owns the memory for all `&'hir` HIR nodes.
///
/// Created once per compilation, held by `CompilerInstance`. All HIR
/// construction goes through the `alloc_*` methods on this struct.
pub struct HirArena {
    exprs: TypedArena<Expr<'static>>,
    patterns: TypedArena<Pattern<'static>>,
    stmts: TypedArena<Stmt<'static>>,
    blocks: TypedArena<Block<'static>>,
    arms: TypedArena<Arm<'static>>,
    items: TypedArena<Item<'static>>,
    field_defs: TypedArena<FieldDef<'static>>,
    variants: TypedArena<Variant<'static>>,
    clauses: TypedArena<ClauseConstraint<'static>>,
    params: TypedArena<Param<'static>>,
    let_stmts: TypedArena<LetStmt<'static>>,
    closure_params: TypedArena<ClosureParam<'static>>,
    field_exprs: TypedArena<FieldExpr<'static>>,
    field_pats: TypedArena<FieldPat<'static>>,
    type_bounds: TypedArena<TraitBound<'static>>,
    generic_params: TypedArena<ClauseParam<'static>>,
    path_segments: TypedArena<PathSegment<'static>>,
    generic_args: TypedArena<GenericArg<'static>>,
}

impl Default for HirArena {
    fn default() -> Self {
        Self::new()
    }
}

impl HirArena {
    pub fn new() -> Self {
        HirArena {
            exprs: TypedArena::new(),
            patterns: TypedArena::new(),
            stmts: TypedArena::new(),
            blocks: TypedArena::new(),
            arms: TypedArena::new(),
            items: TypedArena::new(),
            field_defs: TypedArena::new(),
            variants: TypedArena::new(),
            clauses: TypedArena::new(),
            params: TypedArena::new(),
            let_stmts: TypedArena::new(),
            closure_params: TypedArena::new(),
            field_exprs: TypedArena::new(),
            field_pats: TypedArena::new(),
            type_bounds: TypedArena::new(),
            generic_params: TypedArena::new(),
            path_segments: TypedArena::new(),
            generic_args: TypedArena::new(),
        }
    }
}

// ── Safety note ──────────────────────────────────────────────────────────────
//
// The transmute between `T<'hir>` and `T<'static>` is sound because:
//
// 1. The arena owns the memory and guarantees it lives as long as `&self`.
// 2. `'hir` is the borrow lifetime of `&'hir self`, so any `&'hir T` we
//    hand out is guaranteed to live at least as long as the arena.
// 3. We never allow `T<'static>` references to escape with the wrong
//    lifetime — every alloc method takes `T<'hir>` and returns `&'hir T<'hir>`.

macro_rules! impl_arena_alloc {
    ($alloc:ident, $alloc_slice:ident, $field:ident, $T:ident) => {
        /// Allocate a single node.
        pub fn $alloc<'hir>(&'hir self, val: $T<'hir>) -> &'hir $T<'hir> {
            // SAFETY: see module-level safety note.
            unsafe {
                let val = mem::transmute::<$T<'hir>, $T<'static>>(val);
                let r = self.$field.alloc(val);
                mem::transmute::<&$T<'static>, &'hir $T<'hir>>(r)
            }
        }

        /// Allocate a contiguous slice of nodes.
        pub fn $alloc_slice<'hir>(
            &'hir self,
            vals: impl IntoIterator<Item = $T<'hir>>,
        ) -> &'hir [$T<'hir>] {
            // SAFETY: see module-level safety note.
            unsafe {
                let vals: Vec<$T<'static>> = vals
                    .into_iter()
                    .map(|v| mem::transmute::<$T<'hir>, $T<'static>>(v))
                    .collect();
                let r = self.$field.alloc_from_iter_reg(vals);
                mem::transmute::<&[$T<'static>], &'hir [$T<'hir>]>(r)
            }
        }
    };
}

impl HirArena {
    impl_arena_alloc!(alloc_expr, alloc_expr_slice, exprs, Expr);
    impl_arena_alloc!(alloc_pattern, alloc_pattern_slice, patterns, Pattern);
    impl_arena_alloc!(alloc_stmt, alloc_stmt_slice, stmts, Stmt);
    impl_arena_alloc!(alloc_block, alloc_block_slice, blocks, Block);
    impl_arena_alloc!(alloc_arm, alloc_arm_slice, arms, Arm);
    impl_arena_alloc!(alloc_item, alloc_item_slice, items, Item);
    impl_arena_alloc!(alloc_field_def, alloc_field_def_slice, field_defs, FieldDef);
    impl_arena_alloc!(alloc_variant, alloc_variant_slice, variants, Variant);
    impl_arena_alloc!(alloc_clause, alloc_clause_slice, clauses, ClauseConstraint);
    impl_arena_alloc!(alloc_param, alloc_param_slice, params, Param);
    impl_arena_alloc!(alloc_let_stmt, alloc_let_stmt_slice, let_stmts, LetStmt);
    impl_arena_alloc!(
        alloc_closure_param,
        alloc_closure_param_slice,
        closure_params,
        ClosureParam
    );
    impl_arena_alloc!(
        alloc_field_expr,
        alloc_field_expr_slice,
        field_exprs,
        FieldExpr
    );
    impl_arena_alloc!(alloc_field_pat, alloc_field_pat_slice, field_pats, FieldPat);
    impl_arena_alloc!(
        alloc_type_bound,
        alloc_type_bound_slice,
        type_bounds,
        TraitBound
    );
    impl_arena_alloc!(
        alloc_generic_param,
        alloc_generic_param_slice,
        generic_params,
        ClauseParam
    );
    impl_arena_alloc!(
        alloc_path_segment,
        alloc_path_segment_slice,
        path_segments,
        PathSegment
    );
    impl_arena_alloc!(
        alloc_generic_arg,
        alloc_generic_arg_slice,
        generic_args,
        GenericArg
    );
}
