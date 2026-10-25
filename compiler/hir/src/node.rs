//! Unified HIR node enum.
//!
//! [`Node`] is a tagged reference to any arena-allocated HIR node.  It is
//! used as the element type in the per-owner [`OwnerNodes`](crate::owner::OwnerNodes)
//! table, which is the primary way to look up a node by [`HirId`].

use crate::ClauseParam;
use crate::body::BodyParam;
use crate::clause::ClauseConstraint;
use crate::decl::LetDecl;
use crate::expr::{Block, Expr};
use crate::item::{FieldDef, Item, Variant};
use crate::pattern::{Pattern, PatternArm};

#[derive(Debug, Clone)]
pub enum Node<'hir> {
    Item(&'hir Item<'hir>),
    Pattern(&'hir Pattern<'hir>),
    Expr(&'hir Expr<'hir>),
    Block(&'hir Block<'hir>),
    PatternArm(&'hir PatternArm<'hir>),
    FieldDef(&'hir FieldDef<'hir>),
    Variant(&'hir Variant<'hir>),
    ClauseConstraint(&'hir ClauseConstraint<'hir>),
    Param(&'hir BodyParam<'hir>),
    ClauseParam(&'hir ClauseParam<'hir>),
    LetDecl(&'hir LetDecl<'hir>),
}

impl Node<'_> {
    pub fn hir_id(&self) -> crate::HirId {
        match self {
            Node::Item(node) => node.hir_id(),
            Node::Pattern(node) => node.hir_id,
            Node::Expr(node) => node.hir_id,
            Node::Block(node) => node.hir_id,
            Node::PatternArm(node) => node.hir_id,
            Node::FieldDef(node) => node.hir_id,
            Node::Variant(node) => node.hir_id,
            Node::ClauseConstraint(node) => node.hir_id,
            Node::Param(node) => node.hir_id,
            Node::ClauseParam(node) => node.hir_id,
            Node::LetDecl(node) => node.hir_id,
        }
    }
    pub fn kind_name(&self) -> &'static str {
        match self {
            Node::Item(_) => "item",
            Node::Expr(_) => "expression",
            Node::Pattern(_) => "pattern",
            Node::Block(_) => "block",
            Node::PatternArm(_) => "match arm",
            Node::FieldDef(_) => "field definition",
            Node::Variant(_) => "enum variant",
            Node::ClauseConstraint(_) => "clause constraint",
            Node::Param(_) => "parameter",
            Node::ClauseParam(_) => "clause parameter",
            Node::LetDecl(_) => "let declaration",
        }
    }
}
