use super::*;
use crate::{
    context::{
        CompilerContext,
        scope::{Item, ScopeId},
    },
    hir::{Definition, Expr, Hir, HirMapping, Module, SDefinition, Struct},
    parse::ast::{self, Ast},
    vfs::{self, NodeIdExt, Vfs},
};

impl<'hir, 'ctx, 'vfs> LoweringContext<'hir, 'ctx, 'vfs> {}