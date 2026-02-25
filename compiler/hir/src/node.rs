//! Unified HIR node enum.

use crate::body::Param;
use crate::clause::ClauseConstraint;
use crate::expr::{Arm, Block, Expr, LetStmt, Stmt};
use crate::item::{FieldDef, Item, Variant};
use crate::pattern::Pattern;
use crate::ty::ClauseParam;

#[derive(Debug, Clone)]
pub enum Node<'hir> {
    Item(&'hir Item<'hir>),
    Expr(&'hir Expr<'hir>),
    Pattern(&'hir Pattern<'hir>),
    Stmt(&'hir Stmt<'hir>),
    Block(&'hir Block<'hir>),
    Arm(&'hir Arm<'hir>),
    FieldDef(&'hir FieldDef<'hir>),
    Variant(&'hir Variant<'hir>),
    ClauseConstraint(&'hir ClauseConstraint<'hir>),
    Param(&'hir Param<'hir>),
    ClauseParam(&'hir ClauseParam<'hir>),
    LetStmt(&'hir LetStmt<'hir>),
}

impl Node<'_> {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Node::Item(_) => "item",
            Node::Expr(_) => "expression",
            Node::Pattern(_) => "pattern",
            Node::Stmt(_) => "statement",
            Node::Block(_) => "block",
            Node::Arm(_) => "match arm",
            Node::FieldDef(_) => "field definition",
            Node::Variant(_) => "enum variant",
            Node::ClauseConstraint(_) => "clause constraint",
            Node::Param(_) => "parameter",
            Node::ClauseParam(_) => "clause parameter",
            Node::LetStmt(_) => "let statement",
        }
    }
}
