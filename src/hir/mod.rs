pub mod candidate;

use crate::{
    basic::create_source_map,
    context::scope::{ScopeId, Symbol},
    parse::ast,
    vfs,
};
use internment::{Arena, ArenaIntern};
use rustc_data_structures::fx::FxHashMap;
use rustc_span::SourceMap;
use std::{cell::RefCell, fmt::Display};

pub trait HirNode {}

pub struct Hir {
    pub str_arena: Arena<str>,
    pub source_map: SourceMap,
    expr_arena: Arena<Expr<'static>>,
    pattern_arena: Arena<Pattern<'static>>,
    definition_arena: Arena<Definition<'static>>,
    clause_arena: Arena<Clause<'static>>,
    param_arena: Arena<Param<'static>>,
    exprs_arena: Arena<[Expr<'static>]>,
    patterns_arena: Arena<[Pattern<'static>]>,
    definitions_arena: Arena<[Definition<'static>]>,
    clauses_arena: Arena<[Clause<'static>]>,
    params_arena: Arena<[Param<'static>]>,
    properties_arena: Arena<[Property<'static>]>,

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
    Unresolved(vfs::NodeId, ast::NodeIndex, HirId),
    UnresolvedFileScope(vfs::NodeId, HirId),
    UnresolvedPackage(vfs::NodeId),
    UnresolvedDirectoryModule(vfs::NodeId, HirId),
}

impl Hir {
    pub fn new() -> Self {
        Self {
            str_arena: Arena::new(),
            source_map: create_source_map(),
            expr_arena: Arena::new(),
            pattern_arena: Arena::new(),
            definition_arena: Arena::new(),
            clause_arena: Arena::new(),
            param_arena: Arena::new(),
            exprs_arena: Arena::new(),
            patterns_arena: Arena::new(),
            definitions_arena: Arena::new(),
            clauses_arena: Arena::new(),
            params_arena: Arena::new(),
            properties_arena: Arena::new(),
            map: RefCell::new(FxHashMap::default()),
            impls: RefCell::new(FxHashMap::default()),
        }
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

    pub fn intern_clause<'hir>(&'hir self, clause: Clause<'hir>) -> SClause<'hir> {
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
}

pub type HirId = usize;
pub type SExpr<'hir> = ArenaIntern<'hir, Expr<'hir>>;
pub type SPattern<'hir> = ArenaIntern<'hir, Pattern<'hir>>;
pub type SDefinition<'hir> = ArenaIntern<'hir, Definition<'hir>>;
pub type SClause<'hir> = ArenaIntern<'hir, Clause<'hir>>;
pub type SParam<'hir> = ArenaIntern<'hir, Param<'hir>>;
// which kind do we need?
// pub type MExpr<'hir> = ArenaIntern<'hir, [Expr<'hir>]>;
// or
// pub type MExpr<'hir> = ArenaIntern<'hir, [SExpr<'hir>]>;
pub type MExpr<'hir> = ArenaIntern<'hir, [Expr<'hir>]>;
pub type MPattern<'hir> = ArenaIntern<'hir, [Pattern<'hir>]>;
pub type MDefinition<'hir> = ArenaIntern<'hir, [Definition<'hir>]>;
pub type MClause<'hir> = ArenaIntern<'hir, [Clause<'hir>]>;
pub type MParam<'hir> = ArenaIntern<'hir, [Param<'hir>]>;
pub type MProperty<'hir> = ArenaIntern<'hir, [Property<'hir>]>;

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
        value: SExpr<'hir>,
        body: SExpr<'hir>,
    },
    Const {
        pattern: SPattern<'hir>,
        value: SExpr<'hir>,
        body: SExpr<'hir>,
    },
    Assign {
        location: SExpr<'hir>,
        value: SExpr<'hir>,
    },
    Block(MExpr<'hir>),
    ExprStatement(SExpr<'hir>),
    Break(Option<Symbol<'hir>>),
    Continue(Option<Symbol<'hir>>),
    Return(Option<SExpr<'hir>>),
    Resume(Option<SExpr<'hir>>),

    // types
    TyVoid,
    TyNoReturn,
    TyAny,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pattern<'hir> {
    Wildcard,
    Literal(SExpr<'hir>),
    Variable(SPattern<'hir>),
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
        items: MDefinition<'hir>,
        scope_id: ScopeId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Function<'hir> {
    pub kind: FnKind,
    pub name: Symbol<'hir>,
    pub clauses: MClause<'hir>,
    pub params: MParam<'hir>,
    pub body: SExpr<'hir>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FnKind {
    Normal,
    Method,
    RefMethod,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Clause<'hir> {
    TypeDecl(Symbol<'hir>),
    TypeTraitBounded(Symbol<'hir>, SExpr<'hir>),
    // if the default value is None, it means it must not be a optional parameter
    Decl(Symbol<'hir>, MPattern<'hir>, Option<SExpr<'hir>>),
    Requires,
    Ensures,
    Decreases,
    Outcomes,
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
