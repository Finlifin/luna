use std::fmt::Display;

use rustc_span::Span;

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
        let node_index = self.nodes.len() as NodeIndex;
        // 记录当前节点的子节点在 children 数组中的起始位置
        let children_start_pos = self.children.len() as NodeIndex;

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
                        self.children.extend(child_indices.iter().cloned());
                        len_index
                    }
                }
            })
            .collect();

        children_indexes
            .iter()
            .for_each(|child_index| self.children.push(*child_index));
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

enum Child {
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
    DefinitionAsExpr,
    StatementAsExpr,
    PatternAsExpr,

    IntExtension,
    RealExtension,
    CharExtension,
    StrExtension,

    ListOf,
    Tuple,
    Object,
    Lambda,

    BoolNot,

    SelfLower,
    SelfCap,
    Null,

    EffectQualifiedType,
    ErrorQualifiedType,
    ReachabilityQualifiedType,
    OptionalType,
    TraitObjectType,
    PointerType,
    ForallType,
    ForType,

    RangeTo,
    RangeToInclusive,
    RangeFrom,
    RangeFromTo,
    RangeFromToInclusive,

    Add,
    Sub,
    Mul,
    Div,
    Mod,
    AddAdd,
    BoolEq,
    BoolNotEq,
    BoolAnd,
    BoolOr,
    BoolGt,
    BoolGtEq,
    BoolLt,
    BoolLtEq,
    BoolImplies,
    BoolMatches,

    Arrow, // for function types or arrow expressions

    TypedWith,
    Subtype,
    TraitBound,
    FieldMethodBound,
    DeclarationBound,

    Select,
    Image,
    Pipe,
    PipePrepend,

    Deref,
    Refer,
    Await,
    HandlerApply,
    TypeCast,
    AsDyn,

    EffectElimination,
    ErrorElimination,
    OptionElimination,
    EffectPropagation,
    ErrorPropagation,
    OptionPropagation,

    Call,
    IndexCall,
    ObjectCall,
    DiamondCall,

    PostMatch,
    PatternArm,
    ConditionArm,
    CatchArm,

    // statements
    ExprAsStatement,
    ExprStatement,
    ConstDecl,
    LetDecl,
    ReturnStatement,
    ResumeStatement,
    BreakStatement,
    ContinueStatement,
    IfStatement,
    IfIsMatch,
    IfMatch,
    WhenStatement,
    WhileLoop,
    WhileIsMatch,
    WhileMatch,
    ForLoop,

    // patterns
    PatternIfGuard,
    PatternAndIs,
    PatternAsBind,
    PatternOr,
    PatternOptionSome,
    PatternErrorOk,
    PatternError,
    PatternCall,
    PatternObjectCall,
    PatternDiamondCall,
    PatternFromExpr,
    PatternRangeTo,
    PatternRangeToInclusive,
    PatternRangeFrom,
    PatternRangeFromTo,
    PatternRangeFromToInclusive,
    PropertyPattern,
    PatternRecord,
    PatternList,
    PatternTuple,
    PatternBitVecBin,
    PatternBitVecOct,
    PatternBitVecHex,
    PatternAsync,
    PatternNot,
    PatternTypeBind,

    // items
    FunctionDef,
    EffectDef,
    HandlesDef,

    StructDef,
    StructField,

    EnumDef,
    EnumVariantWithPattern,
    EnumVariantWithTuple,
    EnumVariantWithStruct,
    EnumVariantWithSubEnum,

    UnionDef,
    UnionVariant,

    TraitDef,
    ImplDef,
    ExtendDef,
    DeriveDef,

    Typealias,
    Newtype,

    ModuleDef,

    // imports
    ModStatement,
    UseStatement,
    PathSelect,
    PathSelectMulti,
    PathSelectAll,
    SuperPath,
    ExcludePath,
    PackagePath,
    PathAsBind,

    // clauses and verification related statements
    Asserts,
    Assumes,
    Axiom,
    Invariant,
    Decreases,
    Outcomes,
    Requires,
    Ensures,

    ClauseTraitBoundDecl,
    ClauseDecl,
    ClauseOptionalDecl,
    ClauseTypeDecl,

    // parameters
    ParamOptional,
    ParamTyped,
    ParamTraitBound,
    ParamId,
    ParamOptionalId,
    ParamSelf,
    ParamSelfRef,
    ParamItself,
    ParamItselfRef,
    ParamRestBind,

    // blocks
    Block,
    AtomicBlock,
    DoBlock,
    AsyncBlock,
    ComptimeBlock,

    // others
    FileScope,
    Attribute,
    AttributeSetTrue,
    Property,
    PropertyAssignment,

    Multi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    NoChild,
    // a
    SingleChild,
    // a, b
    DoubleChildren,
    // N
    MultiChildren,
    // a, N
    SingleWithMultiChildren,
    // a, b, N
    DoubleWithMultiChildren,
}

impl NodeKind {
    pub fn node_type(&self) -> NodeType {
        use NodeKind::*;

        match self {
            Invalid => NodeType::NoChild,

            Id | Str | Int | Real | Char | Bool | Unit | SelfCap | SelfLower | Null => {
                NodeType::NoChild
            }

            // Single child
            DefinitionAsExpr
            | StatementAsExpr
            | PatternAsExpr
            | IntExtension
            | RealExtension
            | CharExtension
            | StrExtension
            | Symbol
            | BoolNot
            | EffectQualifiedType
            | ErrorQualifiedType
            | ReachabilityQualifiedType
            | OptionalType
            | TraitObjectType
            | PointerType
            | ForallType
            | ForType
            | RangeTo
            | RangeToInclusive
            | RangeFrom
            | RangeFromTo
            | RangeFromToInclusive
            | Deref
            | Refer
            | Await
            | HandlerApply
            | TypeCast
            | AsDyn
            | EffectElimination
            | ErrorElimination
            | OptionElimination
            | EffectPropagation
            | ErrorPropagation
            | OptionPropagation
            | ExprAsStatement
            | ExprStatement
            | ReturnStatement
            | ResumeStatement
            | BreakStatement
            | ContinueStatement
            | PatternIfGuard
            | PatternAndIs
            | PatternAsBind
            | PatternOptionSome
            | PatternErrorOk
            | PatternError
            | PatternFromExpr
            | PatternRangeTo
            | PatternRangeToInclusive
            | PatternRangeFrom
            | PatternRangeFromTo
            | PatternRangeFromToInclusive
            | PatternAsync
            | PatternNot
            | PatternTypeBind
            | Block
            | AtomicBlock
            | DoBlock
            | AsyncBlock
            | ComptimeBlock => NodeType::SingleChild,

            // Double children
            Add | Sub | Mul | Div | Mod | AddAdd | BoolEq | BoolNotEq | BoolAnd | BoolOr
            | BoolGt | BoolGtEq | BoolLt | BoolLtEq | BoolImplies | BoolMatches | Arrow
            | TypedWith | Subtype | TraitBound | FieldMethodBound | DeclarationBound | Select
            | Image | Pipe | PipePrepend | IndexCall | PostMatch | PatternArm | ConditionArm
            | CatchArm | ConstDecl | LetDecl | IfStatement | IfIsMatch | IfMatch
            | WhenStatement | WhileLoop | WhileIsMatch | WhileMatch | ForLoop | PatternOr
            | StructField | UnionVariant | Typealias | Newtype | PathSelect | PathAsBind
            | ParamOptional | ParamTyped | ParamTraitBound | Property | PropertyAssignment => {
                NodeType::DoubleChildren
            }

            // Multi children
            ListOf | Tuple | Object | Lambda | PatternRecord | PatternList | PatternTuple
            | PatternBitVecBin | PatternBitVecOct | PatternBitVecHex | StructDef | EnumDef
            | UnionDef | TraitDef | ImplDef | ExtendDef | DeriveDef | ModuleDef | UseStatement
            | PathSelectMulti | PathSelectAll | Multi => NodeType::MultiChildren,

            // Single with multi children
            Call
            | ObjectCall
            | DiamondCall
            | PatternCall
            | PatternObjectCall
            | PatternDiamondCall
            | FunctionDef
            | EffectDef
            | HandlesDef
            | EnumVariantWithPattern
            | EnumVariantWithTuple
            | EnumVariantWithStruct
            | EnumVariantWithSubEnum
            | ModStatement
            | SuperPath
            | ExcludePath
            | PackagePath
            | Asserts
            | Assumes
            | Axiom
            | Invariant
            | Decreases
            | Outcomes
            | Requires
            | Ensures
            | PropertyPattern => NodeType::SingleWithMultiChildren,

            // Double with multi children
            ClauseTraitBoundDecl | ClauseDecl | ClauseOptionalDecl | ClauseTypeDecl => {
                NodeType::DoubleWithMultiChildren
            }

            // No child (remaining)
            ParamId | ParamOptionalId | ParamSelf | ParamSelfRef | ParamItself | ParamItselfRef
            | ParamRestBind | FileScope | Attribute | AttributeSetTrue => NodeType::NoChild,
        }
    }
}

impl Ast {
    pub fn get(&self, index: NodeIndex, shape: NodeType) -> Option<NodeBuilder> {
        todo!("Implement get method for Ast with NodeDescriptor");
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
    // TODO: 添加对source map的访问来展开span
    pub fn dump_to_s_expression(&self, node_index: NodeIndex, source_map: ()) -> String {
        if let Some(kind) = self.get_node_kind(node_index) {
            match kind.node_type() {
                NodeType::NoChild => {
                    format!("({} TODO)", kind)
                }
                NodeType::SingleChild => {
                    let child_index = self.get_children(node_index)[0];
                    // self.dump_to_s_expression(child_index, source_map)
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
                NodeType::MultiChildren => {
                    let count = self.get_children(node_index)[0];
                    let children = &self.get_children(node_index)[1..count as usize + 1];
                    let children = children
                        .iter()
                        .map(|&child_index| self.dump_to_s_expression(child_index, source_map))
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!("({} {})", kind, children)
                }
                _ => {
                    format!("(TODO node type: {:?})", kind.node_type())
                }
            }
        } else {
            format!("Invalid node index: {}", node_index)
        }
    }
}
