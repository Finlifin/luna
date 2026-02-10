use std::fmt::Display;

pub mod ast_visitor;
use rustc_span::{SourceMap, Span};

/// Node index type, for future extensibility
pub type NodeIndex = u32;

#[derive(Debug, Clone)]
pub struct Ast {
    // 以下三个字段, 每一位元素对应一个节点
    // 节点的索引从 1 开始, 0 保留用于无效节点
    // 对于一个节点, 如 for i :: nodes[i] == Add(lhs, rhs), 则children[i] == lhs, children[i + 1] == rhs
    // 子节点也可能被映射为多个次子节点,
    // 如 for i :: nodes[i] == Function, 第二个子节点可能是一个参数列表,
    // 即 children[i + 1] == param_count_index, 则
    // children[param_count_index] == param_count, children[param_count_index + 1 .. param_count_index + param_count] == params
    pub nodes: Vec<NodeKind>,
    pub spans: Vec<Span>,
    pub children_start: Vec<NodeIndex>,

    pub children: Vec<NodeIndex>,

    pub root: NodeIndex, // 根节点索引
}

impl Ast {
    pub fn new() -> Self {
        let mut result = Ast {
            nodes: Vec::new(),
            spans: Vec::new(),
            children_start: Vec::new(),
            children: Vec::new(),
            root: 0,
        };

        result.nodes.push(NodeKind::Invalid);
        result.spans.push(Span::default());
        result.children_start.push(0);
        result.children.push(0); // 添加一个无效的子节点索引
        result
    }

    pub fn add_node(&mut self, descriptor: NodeBuilder) -> NodeIndex {
        let children_indexes: Vec<_> = descriptor
            .children
            .iter()
            .map(|child| {
                match child {
                    Child::Single(child_index) => *child_index,
                    Child::Multiple(child_indices) => {
                        let len_index = self.children.len() as NodeIndex;
                        // 先存储子节点数量
                        self.children.push(child_indices.len() as NodeIndex);
                        // 然后存储所有子节点索引
                        self.children.extend_from_slice(&child_indices);
                        len_index
                    }
                }
            })
            .collect();

        let node_index = self.nodes.len() as NodeIndex;
        let children_start_pos = self.children.len() as NodeIndex;

        self.children.extend(children_indexes);
        // 添加节点信息
        self.nodes.push(descriptor.kind);
        self.spans.push(descriptor.span);
        self.children_start.push(children_start_pos);
        node_index
    }

    /// 获取节点的子节点
    pub fn get_children(&self, node_index: NodeIndex) -> &[NodeIndex] {
        if node_index == 0 || node_index > self.nodes.len() as NodeIndex {
            return &[];
        }
        let start = self.children_start[node_index as usize] as usize;
        // 计算结束位置
        let end = self.children.len();
        &self.children[start..end]
    }

    /// 获取节点类型
    pub fn get_node_kind(&self, node_index: NodeIndex) -> Option<NodeKind> {
        if node_index == 0 || node_index > self.nodes.len() as NodeIndex {
            return None;
        }
        Some(self.nodes[node_index as usize])
    }

    pub fn get_node(&self, node_index: NodeIndex) -> Option<(NodeKind, Span, &[NodeIndex])> {
        if node_index == 0 || node_index > self.nodes.len() as NodeIndex {
            return None;
        }
        Some((
            self.nodes[node_index as usize],
            self.spans[node_index as usize],
            self.get_children(node_index),
        ))
    }

    /// 获取节点的 span
    pub fn get_span(&self, node_index: NodeIndex) -> Option<Span> {
        if node_index == 0 || node_index > self.nodes.len() as NodeIndex {
            return None;
        }
        Some(self.spans[node_index as usize])
    }
}

pub struct NodeBuilder {
    kind: NodeKind,
    span: Span,
    children: Vec<Child>,
}

pub enum Child {
    Single(NodeIndex),        // 单个子节点
    Multiple(Vec<NodeIndex>), // 多个子节点
}

impl NodeBuilder {
    pub fn new(kind: NodeKind, span: Span) -> Self {
        NodeBuilder {
            kind,
            span,
            children: Vec::new(),
        }
    }

    pub fn add_single_child(mut self, child: NodeIndex) -> Self {
        self.children.push(Child::single(child));
        self
    }

    pub fn add_multiple_children(mut self, children: Vec<NodeIndex>) -> Self {
        self.children.push(Child::multiple(children));
        self
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = span;
        self
    }

    pub fn with_children(mut self, children: Vec<Child>) -> Self {
        self.children = children;
        self
    }

    pub fn with_node_kind(mut self, kind: NodeKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn build(self, ast: &mut Ast) -> NodeIndex {
        ast.add_node(self)
    }
}

impl Child {
    fn single(index: NodeIndex) -> Self {
        Child::Single(index)
    }

    fn multiple(indices: Vec<NodeIndex>) -> Self {
        Child::Multiple(indices)
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Invalid = 0,

    Id,
    Str,
    Int,
    Real,
    Char,
    Bool,
    Unit,
    Symbol,
    Wildcard, // _

    // exprs
    LiteralExtension, // a, b

    ListOf,
    Tuple,
    Object,
    Lambda, // a, b, N (return_type, body, params)

    BoolNot,  // a
    Negative, // a (prefix `-expr`)

    ErrorNew, // a

    SelfLower,
    SelfCap,
    Null,
    Undefined,

    EffectQualifiedType,       // a, b
    ErrorQualifiedType,        // a, b
    ReachabilityQualifiedType, // a, b
    ClosureQualifiedType,      // a, b (^closure_expr type_expr)
    OptionalType,              // a
    TraitObjectType,           // a
    PointerType,               // a
    LiftType,                  // a (lift expr)
    ForallType,                // a, N (body_expr, params)
    ForType,                   // a, N (body_expr, params)
    // fn_type -> pure? comptime? inline? (unsafe|spec|verified)? (extern "ABI")? fn(parameter_type*)
    // Children: flags_u32, abi_node, N (modifier_flags, abi_str_node, parameter_types)
    // flags_u32 is a raw u32 bitmask stored in the children slot (not a real node index).
    FnType,

    BoolForall, // a, N (body_expr, type_bound_params)
    BoolExists, // a, N (body_expr, type_bound_params)

    RangeFull,
    RangeTo,              // a
    RangeToInclusive,     // a
    RangeFrom,            // a
    RangeFromTo,          // a, b
    RangeFromToInclusive, // a, b

    Add,         // a, b
    Sub,         // a, b
    Mul,         // a, b
    Div,         // a, b
    Mod,         // a, b
    AddAdd,      // a, b
    BoolEq,      // a, b
    BoolNotEq,   // a, b
    BoolAnd,     // a, b
    BoolOr,      // a, b
    BoolGt,      // a, b
    BoolGtEq,    // a, b
    BoolLt,      // a, b
    BoolLtEq,    // a, b
    BoolImplies, // a, b
    BoolMatches, // a, b

    Arrow, // a, b

    TypedWith,        // a, b
    Subtype,          // a, b (TODO: needs `<:` token)
    TraitBound,       // a, b
    FieldMethodBound, // TODO
    DeclarationBound, // TODO

    Select,      // a, b (expr . id)
    TakeView,    // a, b (expr ' id)
    Pipe,        // a, b
    PipePrepend, // a, b

    Deref,              // a
    Refer,              // a
    Await,              // a
    HandlerApplication, // a, b
    TypeCast,           // a, b
    DynCast,            // a, b

    EffectElimination, // a, N
    ErrorElimination,  // a, N
    OptionElimination, // a, b
    EffectPropagation, // a
    ErrorPropagation,  // a
    OptionPropagation, // a

    Application,           // application -> expr(argument*), a, N
    IndexApplication,      // index_application -> expr[expr], a, b
    ExtendedApplication,   // extended_application -> expr{(property|expr)*}, a, N
    NormalFormApplication, // normal_form_application -> expr<argument*>, a, N

    PostMatch,    // a, N (expr match { case_arm* })
    PostLambda,   // a, b (expr do (lambda | block | expr))
    CaseArm,      // a, b
    ConditionArm, // a, b
    CatchArm,     // a, b

    // statements
    // expr
    ExprStatement, // a
    // inline statement_expr
    InlineStatement, // a
    // expr = expr
    Assign, // a, b
    // expr += expr
    AddAssign, // a, b
    // expr -= expr
    SubAssign, // a, b
    // expr *= expr
    MulAssign, // a, b
    // expr /= expr
    DivAssign, // a, b
    // const pattern (: type)? = expr
    ConstDecl, // a, b, c
    // let pattern (: type)? = expr
    LetDecl, // a, b, c
    // return expr? (while expr)?
    ReturnStatement, // a, b
    // resume expr? (while expr)?
    ResumeStatement, // a, b
    // break label? (while expr)?
    BreakStatement, // a, b
    // continue label? (while expr)?
    ContinueStatement, // a, b
    // if condition block else?
    IfStatement, // a, b, c
    // if expr is pattern do block else?
    IfIsMatch, // a, b, c, d
    // if expr is do { branches }
    IfMatch, // a, N
    // when { condition branches }
    WhenStatement, // N
    // while (: label)? condition block
    WhileStatement, // a, b, c
    // while (: label)? expr is pattern do block
    WhileIsMatch, // a, b, c, d
    // while (: label)? expr is do { branches }
    WhileMatch, // a, b, N
    // for (: label)? pattern in expr block
    ForStatement, // a, b, c, d

    // patterns
    IfGuardPattern,               // a, b
    AndIsPattern,                 // a, b, c
    AsBindPattern,                // a, b
    OrPattern,                    // a, b
    OptionSomePattern,            // a
    ErrorOkPattern,               // a
    ErrorPattern,                 // a
    RefPattern,                   // a (ref pattern)
    ApplicationPattern,           // application_pattern -> pattern(pattern*), a, N
    ExtendedApplicationPattern,   // extended_application_pattern -> pattern{...}, a, N
    NormalFormApplicationPattern, // normal_form_application_pattern -> pattern<expr*>, a, N
    ExprAsPattern,                // a (< expr >)
    RangeToPattern,               // a
    RangeToInclusivePattern,      // a
    RangeFromPattern,             // a
    RangeFromToPattern,           // a, b
    RangeFromToInclusivePattern,  // a, b
    PropertyPattern,              // a, b (id: pattern)
    ListRestPattern,              // a (...id)
    StructPattern,                // N
    ListPattern,                  // N
    TuplePattern,                 // N
    BitVecBinPattern,             // TODO
    BitVecOctPattern,             // TODO
    BitVecHexPattern,             // TODO
    AsyncPattern,                 // a
    NotPattern,                   // a
    TypeBindPattern,              // a

    // items
    // pure? comptime? inline? (unsafe|spec|verified|atomic)? (extern "ABI")?
    //   fn id? ( params ) (-> return_type)? (handles eff)? clauses? (block | = expr)
    Function, // a, N, b, c, N, d
    // pure comptime fn id<params> (-> return_type)? clauses? (block | = expr)
    NormalFormDef, // a, N, b, N, c
    // async? effect id (params) (-> return_type)? clauses? (block | = expr)
    AlgebraicEffect, // a, N, b, N, c
    // (id :)? type_expr
    ResultWithId, // a, b

    // struct id clauses? { (property | struct_field | definition | statement)* }
    StructDef, // a, N, b
    // id : type (= default_expr)?
    StructField, // a, b, c

    // enum id clauses? { (property | enum_variant | definition | statement)* }
    EnumDef, // a, N, b
    // id: pattern
    PatternEnumVariant, // a, b
    // id = expr
    ExprEnumVariant, // a, b
    // id (expr*)
    TupleEnumVariant, // a, N
    // id { struct_field* }
    StructEnumVariant, // a, N
    // id.{ enum_variant* }
    SubEnumEnumVariant, // a, N

    // union id clauses? { (property | union_variant | definition | statement)* }
    UnionDef, // a, N, b
    // id : type
    UnionVariant, // a, b

    // trait id (:- expr)? clauses? { (assoc_decl | definition | statement)* }
    TraitDef, // a, b, N, c
    // assoc id(<parameter*>)?: expr (= default)? clauses?
    AssocDecl, // a, N, b, c, N (id, params, type, default, clauses)
    // impl expr clauses? { (definition | statement)* }
    ImplDef, // a, N, b
    // impl expr for expr clauses? { (assoc_decl | definition | statement)* }
    ImplTraitDef, // a, b, N, c
    // extend expr clauses? { (definition | statement)* }
    ExtendDef, // a, N, b
    // extend expr for expr clauses? { (assoc_decl | definition | statement)* }
    ExtendTraitDef, // a, b, N, c
    // derive expr for expr clauses?
    DeriveDef, // a, b, N

    // case id (parameter*) (-> result)? clauses? block
    CaseDef, // a, N, b, N, c

    // typealias id(<parameter*>)? = expr
    TypealiasDef, // a, N, b
    // newtype id(<parameter*>)? = expr
    NewtypeDef, // a, N, b
    // const id (: expr)? = expr
    ConstDef, // a, b, c

    // mod id { (definition | statement)* }
    ModuleDef, // a, b
    // test id? block
    TestDef, // a, b

    // imports
    // mod id
    ModStatement, // a
    // use path
    UseStatement, // a
    // path . id
    ProjectionPath, // a, b
    // path . { paths }
    ProjectionMultiPath, // a, N
    // path . *
    ProjectionAllPath, // a
    // . path
    SuperPath, // a
    // @ path
    PackagePath, // a
    // path as id
    PathAsBind,

    // clauses and verification related statements
    // asserts(: label_id)? expr
    Asserts, // a, b
    // assumes(: label_id)? expr
    Assumes, // a, b
    // axiom(: label_id)? expr
    Axiom, // a, b
    // invariant(: label_id)? expr
    Invariant, // a, b
    // decreases(: label_id)? expr
    Decreases, // a, b
    // TODO
    Outcomes, // TODO
    // requires(: label_id)? expr
    Requires, // a, b
    // ensures(: label_id)? expr
    Ensures, // a, b

    // id :- expr
    TraitBoundDeclClause, // a, b
    // id : expr
    TypeBoundDeclClause, // a, b
    // .id : expr = default
    OptionalDeclClause, // a, b, c
    // ...id : expr
    VarargDeclClause, // a, b
    // quote decl_clause
    QuoteDeclClause, // a
    // id
    TypeDeclClause,

    // parameters
    // .id : expr = expr
    OptionalParam, // a, b, c
    // id : expr
    TypeBoundParam, // a, b
    // id :- expr
    TraitBoundParam, // a, b
    // self
    SelfParam,
    // *self
    SelfRefParam,
    // itself
    ItselfParam,
    // *itself
    ItselfRefParam,
    // ...id : expr
    VarargParam, // a, b
    // comptime parameter
    ComptimeParam, // a
    // error parameter
    ErrorParam, // a
    // catch parameter
    CatchParam, // a
    // lambda parameter
    LambdaParam, // a
    // implicit parameter
    ImplicitParam, // a
    // quote parameter
    QuoteParam, // a
    // assoc parameter_type
    AssocParam, // a
    // ^expr parameter
    AttrParam, // a, b (attr_expr, inner_param)

    // blocks
    // { (items | statements | ...)* }
    Block, // N
    // atomic(id*) { statement* }
    AtomicBlock, // N, a
    // do { statement* }
    DoBlock, // a
    // async { statement* }
    AsyncBlock, // a
    // unsafe { statement* }
    UnsafeBlock, // a
    // comptime { statement* }
    ComptimeBlock, // a

    // others
    FileScope, // N
    // ^expr definition
    Attribute, // a, b
    // keyword modifier -> AttributeSetTrue("flurry_kw_...", definition)
    AttributeSetTrue, // a, b
    // property -> .id expr
    Property, // a, b
    // optional_arg -> .id = expr
    OptionalArg, // a, b
    // extend_arg -> ... expr
    ExtendArg, // a
}

// FnType modifier flags (packed into a u32 stored as children[0]).
pub const FN_MOD_PURE: u32 = 1 << 0;
pub const FN_MOD_COMPTIME: u32 = 1 << 1;
pub const FN_MOD_INLINE: u32 = 1 << 2;
pub const FN_MOD_UNSAFE: u32 = 1 << 3;
pub const FN_MOD_SPEC: u32 = 1 << 4;
pub const FN_MOD_VERIFIED: u32 = 1 << 5;
pub const FN_MOD_EXTERN: u32 = 1 << 6;
pub const FN_MOD_ATOMIC: u32 = 1 << 7; // only for function definitions, not fn_type

pub fn fn_mod_flags_to_string(flags: u32) -> String {
    let mut parts = Vec::new();
    if flags & FN_MOD_PURE != 0 {
        parts.push("pure");
    }
    if flags & FN_MOD_COMPTIME != 0 {
        parts.push("comptime");
    }
    if flags & FN_MOD_INLINE != 0 {
        parts.push("inline");
    }
    if flags & FN_MOD_UNSAFE != 0 {
        parts.push("unsafe");
    }
    if flags & FN_MOD_SPEC != 0 {
        parts.push("spec");
    }
    if flags & FN_MOD_VERIFIED != 0 {
        parts.push("verified");
    }
    if flags & FN_MOD_EXTERN != 0 {
        parts.push("extern");
    }
    if flags & FN_MOD_ATOMIC != 0 {
        parts.push("atomic");
    }
    parts.join(" ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    // No children
    NoChild,
    // Single child: a
    SingleChild,
    // Two children: a, b
    DoubleChildren,
    // Three children: a, b, c
    TripleChildren,
    // Four children: a, b, c, d
    QuadrupleChildren,
    // Multiple children: N
    MultiChildren,
    // Single child with multi children: a, N
    SingleWithMultiChildren,
    // Two children with multi children: a, b, N
    DoubleWithMultiChildren,
    // Three children with multi children: a, b, c, N
    TripleWithMultiChildren,

    // Complex patterns for specific node types
    // Function definitions: a, N, b, c, N, d (id, params, return_type, handles_effect, clauses, body)
    FunctionDefChildren,
    // Normal form / Case definitions: a, N, b, N, c (id, params, return_type, clauses, body)
    NormalFormDefChildren,
    // Effect definitions: a, N, b, N, c (id, params, return_type, clauses, body)
    AlgebraicEffectChildren,
    // Struct/Enum/Union definitions: a, N, b (id, clauses, body)
    TypeDefChildren,
    // Trait definitions: a, b, N, c (id, super_trait, clauses, body)
    TraitDefChildren,
    // Impl trait definitions: a, b, N, c (trait, type, clauses, body)
    ImplTraitDefChildren,
    // Extend trait definitions: a, b, N, c (trait, type, clauses, body)
    ExtendTraitDefChildren,
    // Type alias/newtype/const_def: a, N, b (id, type_params, type) or (id, type, init)
    TypeAliasChildren,
    // Assoc decl: a, N, b, c, N (id, params, type, default, clauses)
    AssocDeclChildren,
    // FnType: flags_u32, abi_node, N (modifier_flags, abi_str_node, parameter_types)
    // flags_u32 is NOT a node index but a raw bitmask.
    FnTypeChildren,
}

impl NodeKind {
    pub fn node_type(&self) -> NodeType {
        use NodeKind::*;

        match self {
            // No children
            Invalid | Id | Str | Int | Real | Char | Bool | Unit | Symbol | Wildcard
            | SelfLower | SelfCap | Null | Undefined | SelfParam | SelfRefParam | ItselfParam
            | ItselfRefParam | TypeDeclClause | RangeFull => NodeType::NoChild,

            // Single child (a)
            BoolNot
            | Negative
            | ErrorNew
            | OptionalType
            | TraitObjectType
            | PointerType
            | LiftType
            | RangeTo
            | RangeToInclusive
            | RangeFrom
            | Deref
            | Refer
            | Await
            | EffectPropagation
            | ErrorPropagation
            | OptionPropagation
            | ExprStatement
            | InlineStatement
            | OptionSomePattern
            | ErrorOkPattern
            | ErrorPattern
            | RefPattern
            | ExprAsPattern
            | RangeToPattern
            | RangeToInclusivePattern
            | RangeFromPattern
            | ListRestPattern
            | AsyncPattern
            | NotPattern
            | TypeBindPattern
            | DoBlock
            | AsyncBlock
            | UnsafeBlock
            | ComptimeBlock
            | ComptimeParam
            | ErrorParam
            | CatchParam
            | LambdaParam
            | ImplicitParam
            | QuoteParam
            | AssocParam
            | QuoteDeclClause
            | ModStatement
            | UseStatement
            | ProjectionAllPath
            | SuperPath
            | PackagePath
            | ExtendArg => NodeType::SingleChild,

            // Double children (a, b)
            LiteralExtension
            | EffectQualifiedType
            | ErrorQualifiedType
            | ReachabilityQualifiedType
            | ClosureQualifiedType
            | RangeFromTo
            | RangeFromToInclusive
            | Add
            | Sub
            | Mul
            | Div
            | Mod
            | AddAdd
            | BoolEq
            | BoolNotEq
            | BoolAnd
            | BoolOr
            | BoolGt
            | BoolGtEq
            | BoolLt
            | BoolLtEq
            | BoolImplies
            | BoolMatches
            | Arrow
            | TypedWith
            | Subtype
            | TraitBound
            | Select
            | TypeBoundDeclClause
            | TakeView
            | Pipe
            | PipePrepend
            | HandlerApplication
            | TypeCast
            | DynCast
            | OptionElimination
            | PostLambda
            | Assign
            | AddAssign
            | SubAssign
            | MulAssign
            | DivAssign
            | IndexApplication
            | CaseArm
            | ConditionArm
            | CatchArm
            | OrPattern
            | RangeFromToPattern
            | RangeFromToInclusivePattern
            | PropertyPattern
            | PatternEnumVariant
            | ExprEnumVariant
            | VarargDeclClause
            | UnionVariant
            | TraitBoundDeclClause
            | ProjectionPath
            | PathAsBind
            | TypeBoundParam
            | TraitBoundParam
            | VarargParam
            | AttrParam
            | Property
            | OptionalArg
            | ModuleDef
            | TestDef
            | Attribute
            | ReturnStatement
            | ResumeStatement
            | BreakStatement
            | ContinueStatement
            | ResultWithId
            | AttributeSetTrue
            | Asserts
            | Assumes
            | Axiom
            | Invariant
            | Decreases
            | Requires
            | Ensures => NodeType::DoubleChildren,

            // Triple children (a, b, c)
            ConstDecl | ConstDef | LetDecl | IfStatement | WhileStatement | IfGuardPattern
            | AndIsPattern | AsBindPattern | OptionalDeclClause | OptionalParam | StructField => {
                NodeType::TripleChildren
            }

            // Quadruple children (a, b, c, d)
            IfIsMatch | WhileIsMatch | ForStatement => NodeType::QuadrupleChildren,

            // Multi children (N)
            ListOf | Tuple | Object | Block | StructPattern | ListPattern | TuplePattern
            | WhenStatement | BitVecBinPattern | BitVecOctPattern | BitVecHexPattern
            | FileScope => NodeType::MultiChildren,

            // Single with multi children (a, N)
            ForallType
            | ForType
            | BoolForall
            | BoolExists
            | EffectElimination
            | ErrorElimination
            | Application
            | ExtendedApplication
            | NormalFormApplication
            | PostMatch
            | ApplicationPattern
            | ExtendedApplicationPattern
            | NormalFormApplicationPattern
            | IfMatch
            | TupleEnumVariant
            | StructEnumVariant
            | SubEnumEnumVariant => NodeType::SingleWithMultiChildren,

            // Double with multi children (a, b, N)
            Lambda | AtomicBlock | WhileMatch => NodeType::DoubleWithMultiChildren,

            // Complex children patterns
            Function => NodeType::FunctionDefChildren, // a, N, b, c, N, d
            NormalFormDef | CaseDef => NodeType::NormalFormDefChildren, // a, N, b, N, c
            AlgebraicEffect => NodeType::AlgebraicEffectChildren, // a, N, b, N, c
            StructDef | EnumDef | UnionDef | ImplDef | ExtendDef => NodeType::TypeDefChildren, // a, N, b
            TraitDef => NodeType::TraitDefChildren, // a, b, N, c
            ImplTraitDef => NodeType::ImplTraitDefChildren, // a, b, N, c
            ExtendTraitDef => NodeType::ExtendTraitDefChildren, // a, b, N, c
            DeriveDef => NodeType::DoubleWithMultiChildren, // a, b, N
            TypealiasDef | NewtypeDef => NodeType::TypeAliasChildren, // a, N, b
            AssocDecl => NodeType::AssocDeclChildren, // a, N, b, c, N
            FnType => NodeType::FnTypeChildren,     // flags_u32, abi_node, N
            ProjectionMultiPath => NodeType::SingleWithMultiChildren, // a, N

            // TODO items
            FieldMethodBound | DeclarationBound | Outcomes => NodeType::SingleChild,
        }
    }
}

impl Ast {
    pub fn get_multi_child_slice(&self, slice_len_index: NodeIndex) -> Option<&[NodeIndex]> {
        if slice_len_index == 0 || slice_len_index >= self.children.len() as NodeIndex {
            return None;
        }
        let slice_len_index = slice_len_index as usize;
        let count = self.children[slice_len_index] as usize;
        Some(&self.children[slice_len_index + 1..slice_len_index + count + 1])
    }
}

impl Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Ast {
    pub fn source_content(&self, node_index: NodeIndex, source_map: &SourceMap) -> Option<String> {
        if node_index == 0 {
            return None;
        }
        if let Some(span) = self.get_span(node_index) {
            // let span = span.with_lo(self.start_pos + span.lo());
            let source_file = source_map.lookup_source_file(span.lo());
            if let Some(content) = &source_file.src {
                let byte_start = (span.lo().0 - source_file.start_pos.0) as usize;
                let byte_end = (span.hi().0 - source_file.start_pos.0) as usize;
                Some(content[byte_start..byte_end].trim().to_string())
            } else {
                eprintln!("Error: Source file content not available");
                None
            }
        } else {
            None
        }
    }

    // TODO: 记得改进unwarp
    pub fn dump_to_s_expression(&self, node_index: NodeIndex, source_map: &SourceMap) -> String {
        if node_index == 0 {
            return "(<invalid node>)".to_string();
        }
        if let Some(kind) = self.get_node_kind(node_index) {
            match kind.node_type() {
                NodeType::NoChild => {
                    let source_file =
                        source_map.lookup_source_file(self.get_span(node_index).unwrap().lo());

                    let source_content = match &source_file.src {
                        Some(content) => content.as_str(),
                        None => {
                            eprintln!("Error: Source file content not available");
                            return "<invalid source>".to_string();
                        }
                    };

                    let byte_start = (self.get_span(node_index).unwrap().lo().0
                        - source_file.start_pos.0) as usize;
                    let byte_end = (self.get_span(node_index).unwrap().hi().0
                        - source_file.start_pos.0) as usize;
                    format!("({} {})", kind, source_content[byte_start..byte_end].trim())
                }
                NodeType::SingleChild => {
                    let children = self.get_children(node_index);
                    let child_index = children[0];
                    format!(
                        "({} {})",
                        kind,
                        self.dump_to_s_expression(child_index, source_map)
                    )
                }
                NodeType::DoubleChildren => {
                    let children = self.get_children(node_index);
                    format!(
                        "({} {} {})",
                        kind,
                        self.dump_to_s_expression(children[0], source_map),
                        self.dump_to_s_expression(children[1], source_map)
                    )
                }
                NodeType::TripleChildren => {
                    let children = self.get_children(node_index);
                    format!(
                        "({} {} {} {})",
                        kind,
                        self.dump_to_s_expression(children[0], source_map),
                        self.dump_to_s_expression(children[1], source_map),
                        self.dump_to_s_expression(children[2], source_map)
                    )
                }
                NodeType::QuadrupleChildren => {
                    let children = self.get_children(node_index);
                    format!(
                        "({} {} {} {} {})",
                        kind,
                        self.dump_to_s_expression(children[0], source_map),
                        self.dump_to_s_expression(children[1], source_map),
                        self.dump_to_s_expression(children[2], source_map),
                        self.dump_to_s_expression(children[3], source_map)
                    )
                }
                NodeType::MultiChildren => {
                    let elements = self.get_children(node_index)[0];
                    let child_nodes = self.get_multi_child_slice(elements).unwrap();
                    let children_str = child_nodes
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!("({} {})", kind, children_str)
                }
                NodeType::SingleWithMultiChildren => {
                    let children = self.get_children(node_index);
                    let first_child = children[0];
                    let multi_children_node = children[1];
                    let multi_children = self.get_multi_child_slice(multi_children_node).unwrap();
                    let multi_children_str = multi_children
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!(
                        "({} {} {})",
                        kind,
                        self.dump_to_s_expression(first_child, source_map),
                        multi_children_str
                    )
                }
                NodeType::DoubleWithMultiChildren => {
                    let children = self.get_children(node_index);
                    let first_child = children[0];
                    let second_child = children[1];
                    let multi_children_node = children[2];
                    let multi_children = self.get_multi_child_slice(multi_children_node).unwrap();
                    let multi_children_str = multi_children
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!(
                        "({} {} {} {})",
                        kind,
                        self.dump_to_s_expression(first_child, source_map),
                        self.dump_to_s_expression(second_child, source_map),
                        multi_children_str
                    )
                }
                NodeType::TripleWithMultiChildren => {
                    let children = self.get_children(node_index);
                    let first_child = children[0];
                    let second_child = children[1];
                    let third_child = children[2];
                    let multi_children_node = children[3];
                    let multi_children = self.get_multi_child_slice(multi_children_node).unwrap();
                    let multi_children_str = multi_children
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!(
                        "({} {} {} {} {})",
                        kind,
                        self.dump_to_s_expression(first_child, source_map),
                        self.dump_to_s_expression(second_child, source_map),
                        self.dump_to_s_expression(third_child, source_map),
                        multi_children_str
                    )
                }

                // Complex children patterns
                NodeType::FunctionDefChildren => {
                    // a, N, b, c, N, d (id, params, return_type, handles_effect, clauses, body)
                    let children = self.get_children(node_index);
                    let id = children[0];
                    let params_node = children[1];
                    let return_type = children[2];
                    let handles_effect = children[3];
                    let clauses_node = children[4];
                    let body = children[5];

                    let params = self.get_multi_child_slice(params_node).unwrap();
                    let params_str = params
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let clauses = self.get_multi_child_slice(clauses_node).unwrap();
                    let clauses_str = clauses
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");

                    format!(
                        "({} {} [{}] {} {} [{}] {})",
                        kind,
                        self.dump_to_s_expression(id, source_map),
                        params_str,
                        self.dump_to_s_expression(return_type, source_map),
                        self.dump_to_s_expression(handles_effect, source_map),
                        clauses_str,
                        self.dump_to_s_expression(body, source_map)
                    )
                }

                NodeType::NormalFormDefChildren => {
                    // a, N, b, N, c (id, type_params, return_type, clauses, body)
                    let children = self.get_children(node_index);
                    let id = children[0];
                    let type_params_node = children[1];
                    let return_type = children[2];
                    let clauses_node = children[3];
                    let body = children[4];

                    let type_params = self.get_multi_child_slice(type_params_node).unwrap();
                    let type_params_str = type_params
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let clauses = self.get_multi_child_slice(clauses_node).unwrap();
                    let clauses_str = clauses
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");

                    format!(
                        "({} {} <{}> {} [{}] {})",
                        kind,
                        self.dump_to_s_expression(id, source_map),
                        type_params_str,
                        self.dump_to_s_expression(return_type, source_map),
                        clauses_str,
                        self.dump_to_s_expression(body, source_map)
                    )
                }

                NodeType::AlgebraicEffectChildren => {
                    // a, N, b, N, c (id, params, return_type, clauses, body)
                    let children = self.get_children(node_index);
                    let id = children[0];
                    let params_node = children[1];
                    let return_type = children[2];
                    let clauses_node = children[3];
                    let body = children[4];

                    let params = self.get_multi_child_slice(params_node).unwrap();
                    let params_str = params
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let clauses = self.get_multi_child_slice(clauses_node).unwrap();
                    let clauses_str = clauses
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");

                    format!(
                        "({} {} [{}] {} [{}] {})",
                        kind,
                        self.dump_to_s_expression(id, source_map),
                        params_str,
                        self.dump_to_s_expression(return_type, source_map),
                        clauses_str,
                        self.dump_to_s_expression(body, source_map)
                    )
                }

                NodeType::TypeDefChildren => {
                    // a, N, b (id, clauses, body)
                    let children = self.get_children(node_index);
                    let id = children[0];
                    let clauses_node = children[1];
                    let body = children[2];

                    let clauses = self.get_multi_child_slice(clauses_node).unwrap();
                    let clauses_str = clauses
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");

                    format!(
                        "({} {} [{}] {})",
                        kind,
                        self.dump_to_s_expression(id, source_map),
                        clauses_str,
                        self.dump_to_s_expression(body, source_map)
                    )
                }

                NodeType::TraitDefChildren => {
                    // a, b, N, c (id, super_trait, clauses, body)
                    let children = self.get_children(node_index);
                    let id = children[0];
                    let super_trait = children[1];
                    let clauses_node = children[2];
                    let body = children[3];

                    let clauses = self.get_multi_child_slice(clauses_node).unwrap();
                    let clauses_str = clauses
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");

                    format!(
                        "({} {} {} [{}] {})",
                        kind,
                        self.dump_to_s_expression(id, source_map),
                        self.dump_to_s_expression(super_trait, source_map),
                        clauses_str,
                        self.dump_to_s_expression(body, source_map)
                    )
                }

                NodeType::ImplTraitDefChildren | NodeType::ExtendTraitDefChildren => {
                    // a, b, N, c (trait, type, clauses, body)
                    let children = self.get_children(node_index);
                    let trait_expr = children[0];
                    let type_expr = children[1];
                    let clauses_node = children[2];
                    let body = children[3];

                    let clauses = self.get_multi_child_slice(clauses_node).unwrap();
                    let clauses_str = clauses
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");

                    format!(
                        "({} {} {} [{}] {})",
                        kind,
                        self.dump_to_s_expression(trait_expr, source_map),
                        self.dump_to_s_expression(type_expr, source_map),
                        clauses_str,
                        self.dump_to_s_expression(body, source_map)
                    )
                }

                NodeType::TypeAliasChildren => {
                    // a, N, b (id, type_params, type)
                    let children = self.get_children(node_index);
                    let id = children[0];
                    let type_params_node = children[1];
                    let type_expr = children[2];

                    let type_params = self.get_multi_child_slice(type_params_node).unwrap();
                    let type_params_str = type_params
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");

                    format!(
                        "({} {} <{}> {})",
                        kind,
                        self.dump_to_s_expression(id, source_map),
                        type_params_str,
                        self.dump_to_s_expression(type_expr, source_map)
                    )
                }

                NodeType::FnTypeChildren => {
                    // flags_u32, abi_node, N (modifier_flags, abi_str_node, parameter_types)
                    let children = self.get_children(node_index);
                    let flags = children[0]; // raw u32 bitmask, NOT a node index
                    let abi_node = children[1];
                    let params_node = children[2];

                    let mods_str = fn_mod_flags_to_string(flags);
                    let params = self.get_multi_child_slice(params_node).unwrap();
                    let params_str = params
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");

                    if abi_node != 0 {
                        let abi = self.dump_to_s_expression(abi_node, source_map);
                        format!("(FnType [{}] {} [{}])", mods_str, abi, params_str)
                    } else if !mods_str.is_empty() {
                        format!("(FnType [{}] [{}])", mods_str, params_str)
                    } else {
                        format!("(FnType [{}])", params_str)
                    }
                }

                NodeType::AssocDeclChildren => {
                    // a, N, b, c, N (id, params, type, default, clauses)
                    let children = self.get_children(node_index);
                    let id = children[0];
                    let params_node = children[1];
                    let type_expr = children[2];
                    let default_expr = children[3];
                    let clauses_node = children[4];

                    let params = self.get_multi_child_slice(params_node).unwrap();
                    let params_str = params
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let clauses = self.get_multi_child_slice(clauses_node).unwrap();
                    let clauses_str = clauses
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");

                    format!(
                        "({} {} <{}> {} {} [{}])",
                        kind,
                        self.dump_to_s_expression(id, source_map),
                        params_str,
                        self.dump_to_s_expression(type_expr, source_map),
                        self.dump_to_s_expression(default_expr, source_map),
                        clauses_str
                    )
                }
            }
        } else {
            format!("Invalid node index: {}", node_index)
        }
    }
}
