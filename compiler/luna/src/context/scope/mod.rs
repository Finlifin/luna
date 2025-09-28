mod dump;
mod manager;
mod visitor;
pub use dump::*;
pub use manager::*;
pub use visitor::*;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

use internment::ArenaIntern;

use crate::hir::{Clause, HirId, Import, MClause, MImport};

pub type Symbol<'hir> = ArenaIntern<'hir, str>;
pub type ScopeId = usize;

#[derive(Debug)]
pub struct Scope<'hir> {
    ordered: bool,
    id: ScopeId,
    hir_id: HirId,  // 添加关联的 HIR ID
    name: Option<Symbol<'hir>>,
    parent: Option<ScopeId>,
    // 匿名作用域怎么办
    items: Vec<Item<'hir>>,
    imports: RefCell<Vec<Import<'hir>>>,
    clauses: RefCell<Vec<Clause<'hir>>>,
    externsions: (),
}

#[derive(Debug, Clone, Copy)]
pub struct Item<'hir> {
    pub symbol: Symbol<'hir>,
    pub hir_id: HirId,
    pub scope_id: Option<ScopeId>,
}

impl<'hir> Item<'hir> {
    pub fn new(symbol: Symbol<'hir>, hir_id: HirId, scope_id: Option<ScopeId>) -> Self {
        Item {
            symbol,
            hir_id,
            scope_id,
        }
    }

    /// update the hir_id locally
    pub unsafe fn update_hir_id_unchecked(&self, hir_id: HirId) {
        unsafe {
            let ptr = self as *const _ as *mut Item<'hir>;
            (*ptr).hir_id = hir_id;
        }
    }
}

impl<'hir> Scope<'hir> {
    #[inline]
    fn new(
        id: ScopeId,
        hir_id: HirId,
        name: Option<Symbol<'hir>>,
        parent: Option<ScopeId>,
        ordered: bool,
    ) -> Self {
        Scope {
            id,
            hir_id,
            parent,
            name,
            items: Vec::new(),
            ordered,
            imports: RefCell::new(vec![]),
            clauses: RefCell::new(vec![]),
            externsions: (),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ScopeError<'hir> {
    DuplicateSymbol(Symbol<'hir>),
    InvalidParentScope(ScopeId),
}
