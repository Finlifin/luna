//! HIR pattern types.

use rustc_span::Span;

use crate::Expr;
use crate::common::{BindingMode, Ident, Path};
use crate::hir_id::HirId;

#[derive(Debug, Clone, PartialEq)]
pub struct Pattern<'hir> {
    pub hir_id: HirId,
    pub kind: PatternKind<'hir>,
    pub span: Span,
}

/// 带控制流的模式语法，比如`and_is`和`if_guard`，会被去糖为多层的模式匹配表达式
#[derive(Debug, Clone, PartialEq)]
pub enum PatternKind<'hir> {
    Wild,

    Binding(BindingMode, Ident, Option<&'hir Pattern<'hir>>),

    Const(&'hir Expr<'hir>),
    // `<comptime_expr>`，在最后会被编译器求值为一个常量值
    Comptime(&'hir Expr<'hir>),

    // `(pat1, pat2, …)`
    Tuple(&'hir [Pattern<'hir>]),
    // 这里指的是完全没有限定类型的结构体模式，需要类型推导来确定匹配的结构体类型
    // `{ field1, field2, field3: pat3, .. }`
    // 如果 field没有指定模式，则默认使用 `field: _` 来匹配
    Struct(Path<'hir>, &'hir [FieldPat<'hir>], bool),
    // 用于匹配数组、切片、迭代器等类型的模式
    List(&'hir [Pattern<'hir>], Option<&'hir Pattern<'hir>>),

    // `.NetErr.Timeout(...)`
    AppTuple(Path<'hir>, &'hir [Pattern<'hir>], PathExaustiveness),
    // `.NetErr.Timeout { ... }`
    AppStruct(Path<'hir>, &'hir [FieldPat<'hir>], PathExaustiveness),

    // `some_value?`
    OptionSome(&'hir Pattern<'hir>),
    // `null`
    OptionNull,

    // `ok_result!`
    ErrorOk(&'hir Pattern<'hir>),
    // `error err_pattern`
    ErrorErr(&'hir Pattern<'hir>),

    Or(&'hir [Pattern<'hir>]),
    Ref(&'hir Pattern<'hir>),
    Path(Path<'hir>),
    Range(
        Option<&'hir super::expr::Expr<'hir>>,
        Option<&'hir super::expr::Expr<'hir>>,
        BoundType,
    ),

    // TODO
    Async,
    BitVec,

    Invalid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatternArm<'hir> {
    pub hir_id: HirId,
    pub pat: Pattern<'hir>,
    pub body: &'hir Expr<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PathExaustiveness {
    NonExhaustive,
    Exhaustive,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BoundType {
    Inclusive,
    Exclusive,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldPat<'hir> {
    pub ident: Ident,
    pub pat: Pattern<'hir>,
    pub span: Span,
}
