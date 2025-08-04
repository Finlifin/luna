use core::slice;
use std::fmt::Display;

use rustc_span::{BytePos, SourceMap, Span};

/// Node index type, for future extensibility
pub type NodeIndex = u32;

#[derive(Debug, Clone)]
pub struct Ast {
    // 一下三个字段, 每一位元素对应一个节点
    // 节点的索引从 1 开始, 0 保留用于无效节点
    // 对于一个节点, 如 for i :: nodes[i] == Add(lhs, rhs), 则children[i] == lhs, children[i + 1] == rhs
    // 子节点也可能被映射为多个次子节点,
    // 如 for i :: nodes[i] == FunctionDef, 第二个子节点可能是一个参数列表,
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

    // exprs
    DefinitionAsExpr, // a
    StatementAsExpr,  // a
    PatternAsExpr,    // a

    LiteralExtension, // a

    ListOf,
    Tuple,
    Object,
    Lambda, // N, a, b

    BoolNot, // a

    ErrorNew,

    SelfLower,
    SelfCap,
    Null,

    EffectQualifiedType,       // a, b
    ErrorQualifiedType,        // a, b
    ReachabilityQualifiedType, // a, b
    OptionalType,              // a
    TraitObjectType,           // a
    PointerType,               // a
    ForallType,                // N, a
    ForType,                   // N, b

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
    Subtype,          // a, b
    TraitBound,       // a, b
    FieldMethodBound, // TODO
    DeclarationBound, // TODO

    Select,      // a, b
    Image,       // a, b
    Pipe,        // a, b
    PipePrepend, // a, b

    Deref,        // a
    Refer,        // a
    Await,        // a
    HandlerApply, // a, b
    TypeCast,     // a, b
    AsDyn,        // a, b

    EffectElimination, // a, N
    ErrorElimination,  // a, N
    OptionElimination, // a, b
    EffectPropagation, // a
    ErrorPropagation,  // a
    OptionPropagation, // a

    Call,        // a, N
    IndexCall,   // a, b
    ObjectCall,  // a, N
    DiamondCall, // a, N

    PostMatch,    // a, N
    PatternArm,   // a, b
    ConditionArm, // a, b
    CatchArm,     // a, b

    // statements
    ExprAsStatement, // a
    // expr
    ExprStatement, // a
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
    // return expr? if_guard?
    ReturnStatement, // a, b
    // resume expr? if_guard?
    ResumeStatement, // a, b
    // break label? if_guard?
    BreakStatement, // a, b
    // continue label? if_guard?
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
    WhileLoop, // a, b, c
    // while (: label)? expr is pattern do block
    WhileIsMatch, // a, b, c, d
    // while (: label)? expr is do { branches }
    WhileMatch, // a, b, N
    // for (: label)? pattern in expr block
    ForLoop, // a, b, c, d

    // patterns
    PatternIfGuard,              // a, b
    PatternAndIs,                // a, b, c
    PatternAsBind,               // a, b
    PatternOr,                   // a, b
    PatternOptionSome,           // a
    PatternErrorOk,              // a
    PatternError,                // a
    PatternCall,                 // a, N
    PatternObjectCall,           // a, N
    PatternDiamondCall,          // a, N
    ExprAsPattern,               // a
    PatternRangeTo,              // a
    PatternRangeToInclusive,     // a
    PatternRangeFrom,            // a
    PatternRangeFromTo,          // a, b
    PatternRangeFromToInclusive, // a, b
    PropertyPattern,             // a, b
    PatternRecord,               // N
    PatternList,                 // N
    PatternTuple,                // N
    PatternBitVecBin,            // TODO
    PatternBitVecOct,            // TODO
    PatternBitVecHex,            // TODO
    PatternAsync,                // a
    PatternNot,                  // a
    PatternTypeBind,             // a

    // items
    // fn id? ( params ) (-> return_type)? (handles eff)? clauses?  (block | = expr)
    FunctionDef, // a, N, b, c, N, d
    // fn id? <params> (-> return_type)? clauses? block
    DiamondFunctionDef, // a, B, b, N, c
    // effect id? ( params ) clauses?
    EffectDef, // a, N, b, N
    // handles eff fn ( params ) (-> return_type)? clauses? block
    HandlesDef, // a, N, b, N, c

    // struct id? clauses? block
    StructDef, // a, N, b
    // id : type (= init)?
    StructField, // a, b, c

    // enum id? clauses? block
    EnumDef, // a, N, b
    // id = pattern
    EnumVariantWithPattern, // a, b
    // id ( tuple_elements )
    EnumVariantWithTuple, // a, N
    // id { struct_fields }
    EnumVariantWithStruct, // a, N
    // id . { variants }
    EnumVariantWithSubEnum, // a, N

    // union id? clauses? block
    UnionDef, // a, N, b
    // id : type
    UnionVariant, // a, b

    // trait id? (: super)? clauses? block
    TraitDef, // a, b, N, c
    // impl type clauses? block
    ImplDef, // a, N, b
    // impl trait for type clauses? block
    ImplTraitDef, // a, b, N, c
    // extend type clauses? block
    ExtendDef, // a, N, b
    // extend trait for type clauses? block
    ExtendTraitDef, // a, b, N, c
    // derive traits for type clauses?
    DeriveDef, // N, a, N

    // typealias id (< params >) = type
    Typealias, // a, N, b
    // newtype id (< params >) = type
    Newtype, // a, N, b

    // mod id? clauses? block
    ModuleDef, // a, N, b

    // imports
    // mod id
    ModStatement, // a
    // use path
    UseStatement, // a
    // path . id
    PathSelect, // a, b
    // path . { paths }
    PathSelectMulti, // a, N
    // path . *
    PathSelectAll, // a
    // . path
    SuperPath, // a
    // not path
    ExcludePath, // a
    // @ path
    PackagePath, // a
    // path as id
    PathAsBind,

    // clauses and verification related statements
    // asserts expr
    Asserts, // a
    // assumes expr
    Assumes, // a
    // axiom expr
    Axiom, // a
    // invariant expr
    Invariant, // a
    // decreases expr
    Decreases, // a
    // TODO
    Outcomes, // TODO
    // requires expr
    Requires, // a
    // ensures expr
    Ensures,

    // id :- expr
    ClauseTraitBoundDecl, // a, b
    // id : expr
    ClauseDecl, // a, b
    // .id (: expr)? (= init)?
    ClauseOptionalDecl, // a, b, c
    // id
    ClauseTypeDecl,

    // parameters
    // .id : expr = expr
    ParamOptional, // a, b, c
    // id : expr
    ParamTyped, // a, b
    // id :- expr
    ParamTraitBound, // a, b
    // self
    ParamSelf,
    // *self
    ParamSelfRef,
    // itself
    ParamItself,
    // *itself
    ParamItselfRef,
    // ... id : expr
    ParamRestBind, // a, b

    // blocks
    // { (items | statements | ...)* }
    Block, // N
    // atomic(exprs) block
    AtomicBlock, // N, a
    // do block
    DoBlock, // a
    // async block
    AsyncBlock, // a
    // comptime block
    ComptimeBlock, // a

    // others
    FileScope, // N
    // ^expr term
    Attribute, // a, b
    // inline fn ... => AttributeSetTrue("flurry_keyword_inline", FunctionDef(...))
    AttributeSetTrue, // a, b
    // .id expr
    Property, // a, b
    // .id = expr
    PropertyAssignment, // a, b
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
    // Diamond function definitions: a, N, b, N, c (id, type_params, return_type, clauses, body)
    DiamondFunctionDefChildren,
    // Effect definitions: a, N, b, N (id, params, return_type, clauses)
    EffectDefChildren,
    // Handles definitions: a, N, b, N, c (effect, params, return_type, clauses, body)
    HandlesDefChildren,
    // Struct/Enum/Union definitions: a, N, b (id, clauses, body)
    TypeDefChildren,
    // Trait definitions: a, b, N, c (id, super_trait, clauses, body)
    TraitDefChildren,
    // Impl trait definitions: a, b, N, c (trait, type, clauses, body)
    ImplTraitDefChildren,
    // Extend trait definitions: a, b, N, c (trait, type, clauses, body)
    ExtendTraitDefChildren,
    // Derive definitions: N, a, N (traits, type, clauses)
    DeriveDefChildren,
    // Type alias/newtype: a, N, b (id, type_params, type)
    TypeAliasChildren,
}

impl NodeKind {
    pub fn node_type(&self) -> NodeType {
        use NodeKind::*;

        match self {
            // No children
            Invalid | Id | Str | Int | Real | Char | Bool | Unit | Symbol | SelfLower | SelfCap
            | Null | ParamSelf | ParamSelfRef | ParamItself | ParamItselfRef | ClauseTypeDecl
            | RangeFull => NodeType::NoChild,

            // Single child (a)
            DefinitionAsExpr
            | StatementAsExpr
            | PatternAsExpr
            | LiteralExtension
            | BoolNot
            | ErrorNew
            | OptionalType
            | TraitObjectType
            | PointerType
            | RangeTo
            | RangeToInclusive
            | RangeFrom
            | Deref
            | Refer
            | Await
            | EffectPropagation
            | ErrorPropagation
            | OptionPropagation
            | ExprAsStatement
            | ExprStatement
            | PatternOptionSome
            | PatternErrorOk
            | PatternError
            | ExprAsPattern
            | PatternRangeTo
            | PatternRangeToInclusive
            | PatternRangeFrom
            | PatternAsync
            | PatternNot
            | PatternTypeBind
            | DoBlock
            | AsyncBlock
            | ComptimeBlock
            | ModStatement
            | UseStatement
            | PathSelectAll
            | SuperPath
            | ExcludePath
            | PackagePath
            | Asserts
            | Assumes
            | Axiom
            | Invariant
            | Decreases
            | Requires => NodeType::SingleChild,

            // Double children (a, b)
            EffectQualifiedType
            | ErrorQualifiedType
            | ReachabilityQualifiedType
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
            | ClauseDecl
            | Image
            | Pipe
            | PipePrepend
            | HandlerApply
            | TypeCast
            | AsDyn
            | OptionElimination
            | Assign
            | AddAssign
            | SubAssign
            | MulAssign
            | DivAssign
            | IndexCall
            | PatternArm
            | ConditionArm
            | CatchArm
            | PatternOr
            | PatternRangeFromTo
            | PatternRangeFromToInclusive
            | PropertyPattern
            | EnumVariantWithPattern
            | StructField
            | UnionVariant
            | ClauseTraitBoundDecl
            | PathSelect
            | PathAsBind
            | ParamTyped
            | ParamTraitBound
            | ParamRestBind
            | Property
            | PropertyAssignment
            | Attribute
            | ReturnStatement
            | ResumeStatement
            | BreakStatement
            | ContinueStatement
            | AttributeSetTrue => NodeType::DoubleChildren,

            // Triple children (a, b, c)
            ConstDecl | LetDecl | IfStatement | WhileLoop | PatternIfGuard | PatternAndIs
            | PatternAsBind | ClauseOptionalDecl | ParamOptional => NodeType::TripleChildren,

            // Quadruple children (a, b, c, d)
            IfIsMatch | WhileIsMatch | ForLoop => NodeType::QuadrupleChildren,

            // Multi children (N)
            ListOf | Tuple | Object | Block | PatternRecord | PatternList | PatternTuple
            | WhenStatement | PatternBitVecBin | PatternBitVecOct | PatternBitVecHex
            | FileScope => NodeType::MultiChildren,

            // Single with multi children (a, N)
            ForallType
            | ForType
            | EffectElimination
            | ErrorElimination
            | Call
            | ObjectCall
            | DiamondCall
            | PostMatch
            | PatternCall
            | PatternObjectCall
            | PatternDiamondCall
            | IfMatch
            | EnumVariantWithTuple
            | EnumVariantWithStruct
            | EnumVariantWithSubEnum => NodeType::SingleWithMultiChildren,

            // Double with multi children (a, b, N)
            Lambda | AtomicBlock | WhileMatch => NodeType::DoubleWithMultiChildren,

            // Complex children patterns
            // fn id? ( params ) (-> return_type)? (handles eff)? clauses?  (block | = expr)
            FunctionDef => NodeType::FunctionDefChildren, // a, N, b, c, N, d
            // fn id? <params> (-> return_type)? clauses? block
            DiamondFunctionDef => NodeType::DiamondFunctionDefChildren, // a, N, b, N, c
            // effect id? ( params ) clauses?
            EffectDef => NodeType::EffectDefChildren, // a, N, b, N
            // handles eff fn ( params ) (-> return_type)? clauses? block
            HandlesDef => NodeType::HandlesDefChildren, // a, N, b, N, c
            // struct id? clauses? block
            StructDef => NodeType::TypeDefChildren, // a, N, b
            // enum id? clauses? block
            EnumDef => NodeType::TypeDefChildren, // a, N, b
            // union id? clauses? block
            UnionDef => NodeType::TypeDefChildren, // a, N, b
            // trait id? (: super)? clauses? block
            TraitDef => NodeType::TraitDefChildren, // a, b, N, c
            // impl type clauses? block
            ImplDef => NodeType::TypeDefChildren, // a, N, b
            // impl trait for type clauses? block
            ImplTraitDef => NodeType::ImplTraitDefChildren, // a, b, N, c
            // extend type clauses? block
            ExtendDef => NodeType::TypeDefChildren, // a, N, b
            // extend trait for type clauses? block
            ExtendTraitDef => NodeType::ExtendTraitDefChildren, // a, b, N, c
            // derive traits for type clauses?
            DeriveDef => NodeType::DeriveDefChildren, // N, a, N
            // typealias id (< params >) = type
            Typealias => NodeType::TypeAliasChildren, // a, N, b
            // newtype id (< params >) = type
            Newtype => NodeType::TypeAliasChildren, // a, N, b
            // mod id? clauses? block
            ModuleDef => NodeType::TypeDefChildren, // a, N, b
            // path . { paths }
            PathSelectMulti => NodeType::SingleWithMultiChildren, // a, N

            // TODO items
            FieldMethodBound | DeclarationBound | Outcomes | Ensures => NodeType::SingleChild,
        }
    }
}

impl Ast {
    pub fn get(&self, index: NodeIndex, note_type: NodeType) -> Option<NodeBuilder> {
        todo!("Implement get method for Ast with NodeDescriptor");
    }

    pub fn get_multi_child_slice(&self, slice_len_index: NodeIndex) -> Option<&[NodeIndex]> {
        if slice_len_index == 0 || slice_len_index >= self.children.len() as NodeIndex {
            return None;
        }
        let slice_len_index = slice_len_index as usize;
        let count = self.children[slice_len_index] as usize;
        Some(&self.children[slice_len_index + 1..slice_len_index + count + 1])
    }
}

impl From<NodeKind> for NodeIndex {
    fn from(kind: NodeKind) -> Self {
        kind as NodeIndex
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

                NodeType::DiamondFunctionDefChildren => {
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

                NodeType::EffectDefChildren => {
                    // a, N, b, N (id, params, return_type, clauses)
                    let children = self.get_children(node_index);
                    let id = children[0];
                    let params_node = children[1];
                    let return_type = children[2];
                    let clauses_node = children[3];

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
                        "({} {} [{}] {} [{}])",
                        kind,
                        self.dump_to_s_expression(id, source_map),
                        params_str,
                        self.dump_to_s_expression(return_type, source_map),
                        clauses_str
                    )
                }

                NodeType::HandlesDefChildren => {
                    // a, N, b, N, c (effect, params, return_type, clauses, body)
                    let children = self.get_children(node_index);
                    let effect = children[0];
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
                        self.dump_to_s_expression(effect, source_map),
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

                NodeType::DeriveDefChildren => {
                    // N, a, N (traits, type, clauses)
                    let children = self.get_children(node_index);
                    let traits_node = children[0];
                    let type_expr = children[1];
                    let clauses_node = children[2];

                    let traits = self.get_multi_child_slice(traits_node).unwrap();
                    let traits_str = traits
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
                        "({} [{}] {} [{}])",
                        kind,
                        traits_str,
                        self.dump_to_s_expression(type_expr, source_map),
                        clauses_str
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
            }
        } else {
            format!("Invalid node index: {}", node_index)
        }
    }
}
