pub mod candidate;
pub mod display;
mod dump;
pub mod macros;

use crate::{
    basic::create_source_map,
    context::scope::{ScopeId, ScopeManager, Symbol},
    intrinsic::Intrinsic,
    parse::ast,
    vfs,
};
use core::panic;
use internment::{Arena, ArenaIntern};
use rustc_data_structures::fx::FxHashMap;
use rustc_span::SourceMap;
use std::collections::HashSet;
use std::{cell::RefCell, fmt::Display, mem};

// there are lots of unsafe methods in this module,
// make sure each item that requires 'hir come from the same Hir instance
pub struct Hir {
    pub str_arena: Arena<str>,
    pub source_map: SourceMap,
    pub empty: Empty<'static>,
    pub singleton: Singleton<'static>,
    pub preserved_expr_ids: FxHashMap<Symbol<'static>, SExpr<'static>>,
    pub preserved_pattern_ids: FxHashMap<Symbol<'static>, SPattern<'static>>,
    expr_arena: Arena<Expr<'static>>,
    pattern_arena: Arena<Pattern<'static>>,
    definition_arena: Arena<Definition<'static>>,
    clause_arena: Arena<Clause<'static>>,
    param_arena: Arena<Param<'static>>,
    exprs_arena: Arena<[Expr<'static>]>,
    patterns_arena: Arena<[Pattern<'static>]>,
    definitions_arena: Arena<[Definition<'static>]>,
    imports: Arena<[Import<'static>]>,
    clauses_arena: Arena<[Clause<'static>]>,
    params_arena: Arena<[Param<'static>]>,
    properties_arena: Arena<[Property<'static>]>,
    ids_arena: Arena<[HirId]>,

    map: RefCell<FxHashMap<HirId, HirMapping<'static>>>,
    impls: RefCell<FxHashMap<SExpr<'static>, Vec<HirId>>>,
}

impl Display for Hir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.map.borrow())
    }
}

// owner == 0 if the Hir is not owned by any specific context
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HirMapping<'hir> {
    Expr(SExpr<'hir>, HirId),
    Pattern(SPattern<'hir>, HirId),
    Definition(SDefinition<'hir>, HirId),
    Param(SParam<'hir>, HirId),
    Clause(SClause<'hir>, HirId),

    Package,
    BuiltinPackage,

    Unresolved(vfs::NodeId, ast::NodeIndex, HirId),
    UnresolvedFileScope(vfs::NodeId, HirId),
    UnresolvedPackage(vfs::NodeId),
    UnresolvedDirectoryModule(vfs::NodeId, HirId),

    Invalid,
}

pub struct Empty<'hir> {
    pub exprs: MExpr<'hir>,
    pub patterns: MPattern<'hir>,
    pub definitions: MDefinition<'hir>,
    pub clauses: MClause<'hir>,
    pub params: MParam<'hir>,
    pub ids: MId<'hir>,
}

/// The most frequent expressions
pub struct Singleton<'hir> {
    // exprs
    pub undefined_: SExpr<'hir>,
    pub null_: SExpr<'hir>,
    pub self_: SExpr<'hir>,
    pub self_type: SExpr<'hir>,
    pub any_: SExpr<'hir>,
    pub any_type: SExpr<'hir>,
    pub unit_: SExpr<'hir>,
    pub noreturn_type: SExpr<'hir>,
    pub void_type: SExpr<'hir>,
    pub bool_type: SExpr<'hir>,

    // patterns
    pub wildcard: SPattern<'hir>,
    pub null_pattern: SPattern<'hir>,
}

impl Hir {
    pub fn new() -> Self {
        let str_arena = Arena::new();
        let source_map = create_source_map();
        let expr_arena = Arena::new();
        let pattern_arena = Arena::new();
        let definition_arena = Arena::new();
        let clause_arena = Arena::new();
        let param_arena = Arena::new();
        let exprs_arena = Arena::new();
        let patterns_arena = Arena::new();
        let definitions_arena = Arena::new();
        let imports = Arena::new();
        let clauses_arena = Arena::new();
        let params_arena = Arena::new();
        let properties_arena = Arena::new();
        let ids_arena = Arena::new();

        let empty_exprs = unsafe { mem::transmute(exprs_arena.intern_vec(vec![])) };
        let empty_patterns = unsafe { mem::transmute(patterns_arena.intern_vec(vec![])) };
        let empty_definitions = unsafe { mem::transmute(definitions_arena.intern_vec(vec![])) };
        let empty_clauses = unsafe { mem::transmute(clauses_arena.intern_vec(vec![])) };
        let empty_params = unsafe { mem::transmute(params_arena.intern_vec(vec![])) };
        let empty_ids = unsafe { mem::transmute(ids_arena.intern_vec(vec![])) };

        let empty = Empty {
            exprs: empty_exprs,
            patterns: empty_patterns,
            definitions: empty_definitions,
            clauses: empty_clauses,
            params: empty_params,
            ids: empty_ids,
        };

        let intern_expr = |e: Expr<'static>| unsafe {
            mem::transmute(Arena::<Expr<'static>>::intern(&expr_arena, e))
        };
        let intern_pattern = |p: Pattern<'static>| unsafe {
            mem::transmute(Arena::<Pattern<'static>>::intern(&pattern_arena, p))
        };
        let singleton = Singleton {
            undefined_: intern_expr(Expr::Undefined),
            null_: intern_expr(Expr::Null),
            self_: intern_expr(Expr::SelfVal),
            self_type: intern_expr(Expr::TySelf),
            any_: intern_expr(Expr::Any),
            any_type: intern_expr(Expr::TyAny),
            unit_: intern_expr(Expr::Unit),
            noreturn_type: intern_expr(Expr::TyNoReturn),
            void_type: intern_expr(Expr::TyVoid),
            bool_type: intern_expr(Expr::TyBool),

            // patterns
            wildcard: intern_pattern(Pattern::Wildcard),
            null_pattern: intern_pattern(Pattern::Null),
        };

        let mut preserved_expr_ids = FxHashMap::default();
        let intern_str = |s: &str| unsafe { mem::transmute(Arena::<str>::intern(&str_arena, s)) };
        preserved_expr_ids.insert(intern_str("undefined"), singleton.undefined_);
        preserved_expr_ids.insert(intern_str("null"), singleton.null_);
        preserved_expr_ids.insert(intern_str("self"), singleton.self_);
        preserved_expr_ids.insert(intern_str("Self"), singleton.self_type);
        preserved_expr_ids.insert(intern_str("any"), singleton.any_);
        preserved_expr_ids.insert(intern_str("Any"), singleton.any_type);
        preserved_expr_ids.insert(intern_str("NoReturn"), singleton.noreturn_type);
        preserved_expr_ids.insert(intern_str("void"), singleton.void_type);
        preserved_expr_ids.insert(intern_str("bool"), singleton.bool_type);

        let mut preserved_pattern_ids = FxHashMap::default();
        preserved_pattern_ids.insert(intern_str("_"), singleton.unit_);

        let result = Self {
            str_arena,
            source_map,
            expr_arena,
            pattern_arena,
            definition_arena,
            clause_arena,
            param_arena,
            exprs_arena,
            patterns_arena,
            definitions_arena,
            imports,
            clauses_arena,
            params_arena,
            properties_arena,
            ids_arena,
            empty,
            singleton,
            preserved_expr_ids,
            preserved_pattern_ids: FxHashMap::default(),
            map: RefCell::new(FxHashMap::default()),
            impls: RefCell::new(FxHashMap::default()),
        };
        let _invalid_hir_id_0 = result.put(HirMapping::Invalid);
        result
    }

    pub fn put(&self, value: HirMapping<'_>) -> HirId {
        let mut map = self.map.borrow_mut();
        let id = map.len();
        let static_value: HirMapping<'static> = unsafe { std::mem::transmute(value) };
        map.insert(id, static_value);
        id
    }

    pub fn remove(&self, id: HirId) {
        self.map.borrow_mut().remove(&id);
    }

    pub fn put_impl(&self, expr: SExpr<'_>, id: HirId) {
        let mut impls = self.impls.borrow_mut();
        let static_expr: SExpr<'static> = unsafe { std::mem::transmute(expr) };
        impls.entry(static_expr).or_insert_with(Vec::new).push(id);
    }

    pub fn put_clause<'hir>(&'hir self, clause: Clause<'_>) -> (HirId, SClause<'hir>) {
        let mut map = self.map.borrow_mut();
        let id = map.len();
        // replace the clause with id
        match clause {
            Clause::TypeDecl { symbol, owner, .. } => {
                let clause = self.intern_clause(Clause::TypeDecl {
                    symbol,
                    owner,
                    self_id: id,
                });
                let static_clause: SClause<'static> = unsafe { std::mem::transmute(clause) };
                map.insert(id, HirMapping::Clause(static_clause, id));
                (id, static_clause)
            }
            Clause::TypeTraitBounded {
                symbol,
                owner,
                trait_bound,
                ..
            } => {
                let clause = self.intern_clause(Clause::TypeTraitBounded {
                    symbol,
                    owner,
                    trait_bound,
                    self_id: id,
                });
                let static_clause: SClause<'static> = unsafe { std::mem::transmute(clause) };
                map.insert(id, HirMapping::Clause(static_clause, id));
                (id, static_clause)
            }
            Clause::Decl {
                symbol,
                ty,
                default,
                owner,
                ..
            } => {
                let clause = self.intern_clause(Clause::Decl {
                    symbol,
                    ty,
                    default,
                    owner,
                    self_id: id,
                });
                let static_clause: SClause<'static> = unsafe { std::mem::transmute(clause) };
                map.insert(id, HirMapping::Clause(static_clause, id));
                (id, static_clause)
            }
            _ => {
                panic!("Unimplemented clause type: {:?}", clause)
            }
        }
    }

    pub fn get_impl(&self, expr: SExpr<'_>) -> Option<Vec<HirId>> {
        self.impls.borrow().get(&expr).cloned()
    }

    pub fn update(&self, id: HirId, value: HirMapping<'_>) {
        let static_value: HirMapping<'static> = unsafe { std::mem::transmute(value) };
        self.map.borrow_mut().insert(id, static_value);
    }

    pub fn get(&self, id: HirId) -> Option<HirMapping<'static>> {
        self.map.borrow().get(&id).copied()
    }

    #[inline]
    pub fn source_map(&self) -> &SourceMap {
        &self.source_map
    }

    pub fn intern_str<'hir>(&'hir self, s: &str) -> Symbol<'hir> {
        unsafe { std::mem::transmute(self.str_arena.intern(s)) }
    }

    pub fn intern_expr<'hir>(&'hir self, expr: Expr<'hir>) -> SExpr<'hir> {
        unsafe {
            // 将expr转换为static生命周期版本进行存储
            let static_expr: Expr<'static> = std::mem::transmute(expr);
            std::mem::transmute(self.expr_arena.intern(static_expr))
        }
    }

    pub fn intern_pattern<'hir>(&'hir self, pattern: Pattern<'hir>) -> SPattern<'hir> {
        unsafe {
            let static_pattern: Pattern<'static> = std::mem::transmute(pattern);
            std::mem::transmute(self.pattern_arena.intern(static_pattern))
        }
    }

    pub fn intern_definition<'hir>(&'hir self, def: Definition<'hir>) -> SDefinition<'hir> {
        unsafe {
            let static_def: Definition<'static> = std::mem::transmute(def);
            std::mem::transmute(self.definition_arena.intern(static_def))
        }
    }

    // 如果需要intern一个clause, 请使用put_clause
    fn intern_clause<'hir>(&'hir self, clause: Clause<'hir>) -> SClause<'hir> {
        unsafe {
            let static_clause: Clause<'static> = std::mem::transmute(clause);
            std::mem::transmute(self.clause_arena.intern(static_clause))
        }
    }

    pub fn intern_param<'hir>(&'hir self, param: Param<'hir>) -> SParam<'hir> {
        unsafe {
            let static_param: Param<'static> = std::mem::transmute(param);
            std::mem::transmute(self.param_arena.intern(static_param))
        }
    }

    // 使用专门的vec arena来处理数组
    pub fn intern_exprs<'hir>(&'hir self, exprs: Vec<Expr<'hir>>) -> MExpr<'hir> {
        unsafe {
            let static_exprs: Vec<Expr<'static>> = std::mem::transmute(exprs);
            std::mem::transmute(self.exprs_arena.intern_vec(static_exprs))
        }
    }

    pub fn intern_patterns<'hir>(&'hir self, patterns: Vec<Pattern<'hir>>) -> MPattern<'hir> {
        unsafe {
            let static_patterns: Vec<Pattern<'static>> = std::mem::transmute(patterns);
            std::mem::transmute(self.patterns_arena.intern_vec(static_patterns))
        }
    }

    pub fn intern_definitions<'hir>(&'hir self, defs: Vec<Definition<'hir>>) -> MDefinition<'hir> {
        unsafe {
            let static_defs: Vec<Definition<'static>> = std::mem::transmute(defs);
            std::mem::transmute(self.definitions_arena.intern_vec(static_defs))
        }
    }

    pub fn intern_clauses<'hir>(&'hir self, clauses: Vec<Clause<'hir>>) -> MClause<'hir> {
        unsafe {
            let static_clauses: Vec<Clause<'static>> = std::mem::transmute(clauses);
            std::mem::transmute(self.clauses_arena.intern_vec(static_clauses))
        }
    }

    pub fn intern_params<'hir>(&'hir self, params: Vec<Param<'hir>>) -> MParam<'hir> {
        unsafe {
            let static_params: Vec<Param<'static>> = std::mem::transmute(params);
            std::mem::transmute(self.params_arena.intern_vec(static_params))
        }
    }

    pub fn intern_properties<'hir>(&'hir self, properties: Vec<Property<'hir>>) -> MProperty<'hir> {
        unsafe {
            let static_properties: Vec<Property<'static>> = std::mem::transmute(properties);
            std::mem::transmute(self.properties_arena.intern_vec(static_properties))
        }
    }

    pub fn intern_ids<'hir>(&'hir self, ids: Vec<HirId>) -> MId<'hir> {
        unsafe {
            let static_ids: Vec<HirId> = std::mem::transmute(ids);
            std::mem::transmute(self.ids_arena.intern_vec(static_ids))
        }
    }
}

pub type HirId = usize;
pub type SExpr<'hir> = ArenaIntern<'hir, Expr<'hir>>;
pub type SPattern<'hir> = ArenaIntern<'hir, Pattern<'hir>>;
pub type SDefinition<'hir> = ArenaIntern<'hir, Definition<'hir>>;
pub type SClause<'hir> = ArenaIntern<'hir, Clause<'hir>>;
pub type SParam<'hir> = ArenaIntern<'hir, Param<'hir>>;
pub type MExpr<'hir> = ArenaIntern<'hir, [Expr<'hir>]>;
pub type MPattern<'hir> = ArenaIntern<'hir, [Pattern<'hir>]>;
pub type MDefinition<'hir> = ArenaIntern<'hir, [Definition<'hir>]>;
pub type MClause<'hir> = ArenaIntern<'hir, [Clause<'hir>]>;
pub type MParam<'hir> = ArenaIntern<'hir, [Param<'hir>]>;
pub type MProperty<'hir> = ArenaIntern<'hir, [Property<'hir>]>;
pub type MImport<'hir> = ArenaIntern<'hir, [Import<'hir>]>;
pub type MId<'hir> = ArenaIntern<'hir, [HirId]>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Expr<'hir> {
    Ref(HirId),

    IntLiteral(i64),
    BoolLiteral(bool),
    RealLiteral(i32, u32),
    StrLiteral(Symbol<'hir>),
    CharLiteral(char),
    SymbolLiteral(Symbol<'hir>),

    Null,
    Undefined,
    Unit,
    Any,
    SelfVal,

    List(MExpr<'hir>),
    Tuple(MExpr<'hir>),
    Object(MExpr<'hir>, MProperty<'hir>),
    Range {
        from: SExpr<'hir>,
        to: SExpr<'hir>,
        inclusive: bool,
    },
    Pattern(SPattern<'hir>),

    FnApply {
        callee: SExpr<'hir>,
        args: MExpr<'hir>,
        optional_args: MProperty<'hir>,
    },
    NormalFormFnApply {
        callee: SExpr<'hir>,
        args: MExpr<'hir>,
        optional_args: MProperty<'hir>,
    },
    FnObjectApply {
        callee: SExpr<'hir>,
        elements: MExpr<'hir>,
        properties: MProperty<'hir>,
    },
    UnaryApply {
        expr: SExpr<'hir>,
        op: UnaryOp,
    },
    BinaryApply {
        left: SExpr<'hir>,
        right: SExpr<'hir>,
        op: BinaryOp,
    },
    ObjectApply {
        callee: SExpr<'hir>,
        args: MExpr<'hir>,
        optional_args: MProperty<'hir>,
        object: SExpr<'hir>,
    },
    Index(SExpr<'hir>, SExpr<'hir>),
    Matches(SExpr<'hir>, SPattern<'hir>),

    // statements
    If {
        condition: SExpr<'hir>,
        then_branch: SExpr<'hir>,
        else_branch: Option<SExpr<'hir>>,
    },
    When {
        conditions: MExpr<'hir>,
        branches: MExpr<'hir>,
    },
    Match {
        subject: SExpr<'hir>,
        arms: MPattern<'hir>,
    },
    While {
        condition: SExpr<'hir>,
        body: SExpr<'hir>,
    },
    For {
        pattern: SPattern<'hir>,
        iterable: SExpr<'hir>,
        body: SExpr<'hir>,
    },
    Let {
        pattern: SPattern<'hir>,
        ty: SExpr<'hir>,
        init: SExpr<'hir>,
    },
    Const {
        pattern: SPattern<'hir>,
        ty: SExpr<'hir>,
        init: SExpr<'hir>,
    },
    Assign {
        location: SExpr<'hir>,
        value: SExpr<'hir>,
    },
    Block(BlockKind, MExpr<'hir>),
    ExprStatement(SExpr<'hir>),
    Break(Option<Symbol<'hir>>),
    Continue(Option<Symbol<'hir>>),
    Return(Option<SExpr<'hir>>),
    Resume(Option<SExpr<'hir>>),

    // types
    TyVoid,
    TyNoReturn,
    TyAny,
    TySelf,
    TyInteger,
    TyReal,
    TyChar,
    TySymbol,
    TyObject,
    TyStr,
    TyBool,
    TyInt(u8, bool),
    TyFloat(u8),
    TyOptional(SExpr<'hir>),
    TyTuple(MExpr<'hir>),
    TyPointer(SExpr<'hir>),
    TyArray(SExpr<'hir>, SExpr<'hir>),
    TyScheme(MParam<'hir>, SExpr<'hir>),

    TyNamed(Symbol<'hir>, SDefinition<'hir>),
    TyAlias(Symbol<'hir>, SExpr<'hir>),

    // Pending variants(they are not final)
    Select(SExpr<'hir>, Symbol<'hir>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Normal,
    Do,
    Comptime,
    Unsafe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pattern<'hir> {
    Ref(HirId),
    Wildcard,
    Literal(SExpr<'hir>),
    Symbol(Symbol<'hir>),

    Null,
    Some(SPattern<'hir>),

    Ok(SPattern<'hir>),
    Err(SPattern<'hir>),

    Range {
        from: SPattern<'hir>,
        to: SPattern<'hir>,
        inclusive: bool,
    },

    TupleDestructure(MPattern<'hir>),
    CallDestructure(SExpr<'hir>, MPattern<'hir>),
    // NormalFormDestructure(SExpr<'hir>, MPattern<'hir>),
    ListDestructure(MPattern<'hir>),
    ObjectDestructure(MPattern<'hir>),
    ObjectCallDestructure(SExpr<'hir>, MPattern<'hir>),
    Property(Symbol<'hir>, SPattern<'hir>),

    Variable(Symbol<'hir>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Definition<'hir> {
    Module(Module<'hir>),
    Struct(Struct<'hir>),
    StructField(Symbol<'hir>, SExpr<'hir>, Option<SExpr<'hir>>),
    Enum(Enum<'hir>),
    EnumVariant(Symbol<'hir>),
    EnumVariantWithStruct(Symbol<'hir>, MDefinition<'hir>),
    EnumVariantWithTuple(Symbol<'hir>, MExpr<'hir>),
    EnumVariantWithPattern(Symbol<'hir>, SPattern<'hir>),
    EnumVariantWithSubEnum(Symbol<'hir>, MDefinition<'hir>),
    Function(Function<'hir>),
    FileScope {
        name: Symbol<'hir>,
        items: MDefinition<'hir>,
        scope_id: ScopeId,
    },
    Package {
        name: Symbol<'hir>,
        scope_id: ScopeId,
    },
    Intrinsic(Intrinsic),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Function<'hir> {
    pub kind: FnKind,
    pub name: Symbol<'hir>,
    pub clauses: MId<'hir>,
    pub params: MId<'hir>,
    pub return_type: SExpr<'hir>,
    pub body: SExpr<'hir>,
    pub body_scope: ScopeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FnKind {
    Normal,
    Method,
    RefMethod,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Clause<'hir> {
    TypeDecl {
        symbol: Symbol<'hir>,
        owner: HirId,
        self_id: HirId,
    },
    TypeTraitBounded {
        symbol: Symbol<'hir>,
        trait_bound: HirId,
        owner: HirId,
        self_id: HirId,
    },
    // if the default value is None, it means it must not be a optional parameter
    Decl {
        symbol: Symbol<'hir>,
        ty: HirId,
        default: Option<HirId>,
        owner: HirId,
        self_id: HirId,
    },
    Requires,
    Ensures,
    Decreases,
    Outcomes,
}

impl<'hir> Clause<'hir> {
    pub fn self_id(&self) -> Option<HirId> {
        match self {
            Clause::TypeDecl { self_id, .. } => Some(*self_id),
            Clause::Decl { self_id, .. } => Some(*self_id),
            Clause::TypeTraitBounded { self_id, .. } => Some(*self_id),
            _ => None,
        }
    }

    pub fn owner(&self) -> Option<HirId> {
        match self {
            Clause::TypeDecl { owner, .. } => Some(*owner),
            Clause::Decl { owner, .. } => Some(*owner),
            Clause::TypeTraitBounded { owner, .. } => Some(*owner),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Param<'hir> {
    // if the default value is None, it means it must not be a optional parameter
    Itself { is_ref: bool },
    Typed(Symbol<'hir>, SExpr<'hir>, Option<SExpr<'hir>>),
    AutoCollectToTuple(Symbol<'hir>, SExpr<'hir>),
    AutoCollectToObject(Symbol<'hir>, SExpr<'hir>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Module<'hir> {
    pub name: Symbol<'hir>,
    pub clauses: MClause<'hir>,
    pub scope_id: ScopeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Struct<'hir> {
    pub name: Symbol<'hir>,
    pub clauses: MClause<'hir>,
    pub fields: MDefinition<'hir>,
    pub scope_id: ScopeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Enum<'hir> {
    pub name: Symbol<'hir>,
    pub clauses: MClause<'hir>,
    pub variants: MDefinition<'hir>,
    pub items: MDefinition<'hir>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    Add,
    Sub,
    Div,
    Mul,
    Mod,

    BoolAnd,
    BoolOr,
    BoolEq,
    BoolNotEq,
    BoolGt,
    BoolLt,
    BoolGtEq,
    BoolLtEq,
    BoolImplies,
    BoolTypedWith,
    BoolTraitBound,

    AddAdd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    Neg,
    Not,
    Refer,
    Deref,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Property<'hir> {
    pub name: Symbol<'hir>,
    pub value: SExpr<'hir>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Import<'hir> {
    All(ScopeId),
    Multi(ScopeId, Vec<Symbol<'hir>>),
    Single(ScopeId, Symbol<'hir>),
    Alias {
        scope_id: ScopeId,
        alias: Symbol<'hir>,
        original: Symbol<'hir>,
    },
}

#[test]
fn test_function() {
    let hir = Hir::new();
    // 23 + 23 + 34
    let expr1 = hir.intern_expr(Expr::IntLiteral(23));
    let expr2 = hir.intern_expr(Expr::IntLiteral(23));
    let expr3 = hir.intern_expr(Expr::IntLiteral(34));
    let add_expr1 = hir.intern_expr(Expr::BinaryApply {
        left: expr1,
        right: expr2,
        op: BinaryOp::Add,
    });
    let add_expr2 = hir.intern_expr(Expr::BinaryApply {
        left: add_expr1,
        right: expr3,
        op: BinaryOp::Add,
    });

    let expr4 = hir.intern_expr(Expr::IntLiteral(23));
    let expr5 = hir.intern_expr(Expr::IntLiteral(34));
    let add_expr3 = hir.intern_expr(Expr::BinaryApply {
        left: expr4,
        right: expr4,
        op: BinaryOp::Add,
    });
    let add_expr4 = hir.intern_expr(Expr::BinaryApply {
        left: add_expr3,
        right: expr5,
        op: BinaryOp::Add,
    });

    assert_eq!(add_expr2, add_expr4);
    assert_eq!(expr1, expr2);
    assert_eq!(expr1, expr4);
    assert_ne!(expr1, expr3);
}
