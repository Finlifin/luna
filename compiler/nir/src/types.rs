//! Immutable type cores and reference-side substitutions.
//!
//! Contracts: docs/compilation/runtime-dependent-types.md and
//! docs/type/type-identity.md. Function parameters introduce one de Bruijn
//! frame; their types can refer to earlier parameters in that frame. Holes
//! belong to a core's ordered telescope and are distinct from bound variables.

use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::sync::Arc;

/// Semantic domain of an argument, not a source-language type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sort {
    Type,
    ComptimeValue,
    Runtime,
}

/// Package identity must include the package's build identity at the driver
/// boundary. These numeric handles are session-local, not persistent cache IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeclarationId {
    pub package: u64,
    pub local: u32,
}

/// `instance` distinguishes repeated instantiations/calls of the same binder.
/// Object contents and access paths are deliberately absent from this identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuntimeId {
    pub instance: u64,
    pub local: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Primitive {
    Type,
    Bool,
    Char,
    Str,
    Integer,
    Float,
    Unit,
    Never,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    Isize,
    Usize,
    F32,
    F64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Parameter {
    pub sort: Sort,
    pub ty: Term,
}

/// A stable symbolic term. Eq/Hash compare representation only. In particular,
/// two unequal terms can still require a semantic equality query.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Term {
    Primitive(Primitive),
    Nominal {
        declaration: DeclarationId,
        arguments: Vec<Term>,
    },
    /// No body or reduction callback is stored: substitution preserves the head.
    NormalForm {
        head: DeclarationId,
        sort: Sort,
        arguments: Vec<Term>,
    },
    Tuple(Vec<Term>),
    Pointer(Box<Term>),
    Function {
        parameters: Vec<Parameter>,
        result: Box<Term>,
    },
    /// Core-local interface slot, interpreted only through TypeRef.arguments.
    Hole {
        index: usize,
        sort: Sort,
    },
    /// Local binder reference. Depth zero names the innermost frame.
    Bound {
        depth: u32,
        index: usize,
        sort: Sort,
    },
    /// A definition-side rigid parameter; never assigned by substitution.
    Rigid {
        declaration: DeclarationId,
        sort: Sort,
    },
    Runtime(RuntimeId),
    Bool(bool),
    /// Handle into a comptime value store. Structural identity does not replace
    /// that store's semantic value equality (e.g. Eq/Hash protocols).
    Value {
        store: u64,
        index: u64,
    },
}

impl Term {
    pub fn sort(&self) -> Sort {
        match self {
            Self::Hole { sort, .. }
            | Self::Bound { sort, .. }
            | Self::Rigid { sort, .. }
            | Self::NormalForm { sort, .. } => *sort,
            Self::Runtime(_) => Sort::Runtime,
            Self::Bool(_) | Self::Value { .. } => Sort::ComptimeValue,
            _ => Sort::Type,
        }
    }

    /// Eliminate the immediately surrounding binder frame without capturing
    /// free references in replacement terms. Runtime and rigid identities stay
    /// fixed. Unused substitution entries have no effect on the result.
    pub fn substitute(&self, arguments: &[Term]) -> Result<Term, TypeError> {
        self.rewrite(0, &mut |term, nesting| match term {
            Self::Bound { depth, index, sort } if *depth == nesting => {
                let value = arguments
                    .get(*index)
                    .ok_or(TypeError::MissingArgument(*index))?;
                require_sort(value, *sort)?;
                value.shift(nesting, 0)
            }
            Self::Bound { depth, index, sort } if *depth > nesting => Ok(Self::Bound {
                depth: depth - 1,
                index: *index,
                sort: *sort,
            }),
            _ => Ok(term.clone()),
        })
    }

    fn shift(&self, amount: u32, cutoff: u32) -> Result<Term, TypeError> {
        self.rewrite(cutoff, &mut |term, nesting| match term {
            Self::Bound { depth, index, sort } if *depth >= nesting => Ok(Self::Bound {
                depth: depth
                    .checked_add(amount)
                    .ok_or(TypeError::BinderDepthOverflow)?,
                index: *index,
                sort: *sort,
            }),
            _ => Ok(term.clone()),
        })
    }

    fn rewrite(
        &self,
        nesting: u32,
        leaf: &mut impl FnMut(&Term, u32) -> Result<Term, TypeError>,
    ) -> Result<Term, TypeError> {
        let many = |terms: &[Term], leaf: &mut _| {
            terms
                .iter()
                .map(|t| t.rewrite(nesting, leaf))
                .collect::<Result<Vec<_>, _>>()
        };
        Ok(match self {
            Self::Nominal {
                declaration,
                arguments,
            } => Self::Nominal {
                declaration: *declaration,
                arguments: many(arguments, leaf)?,
            },
            Self::NormalForm {
                head,
                sort,
                arguments,
            } => Self::NormalForm {
                head: *head,
                sort: *sort,
                arguments: many(arguments, leaf)?,
            },
            Self::Tuple(items) => Self::Tuple(many(items, leaf)?),
            Self::Pointer(inner) => Self::Pointer(Box::new(inner.rewrite(nesting, leaf)?)),
            Self::Function { parameters, result } => {
                let inner = nesting
                    .checked_add(1)
                    .ok_or(TypeError::BinderDepthOverflow)?;
                Self::Function {
                    parameters: parameters
                        .iter()
                        .map(|p| {
                            Ok(Parameter {
                                sort: p.sort,
                                ty: p.ty.rewrite(inner, leaf)?,
                            })
                        })
                        .collect::<Result<_, TypeError>>()?,
                    result: Box::new(result.rewrite(inner, leaf)?),
                }
            }
            _ => leaf(self, nesting)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeError {
    WrongSort { expected: Sort, actual: Sort },
    MissingArgument(usize),
    ArgumentCount { expected: usize, actual: usize },
    InvalidHole(usize),
    InvalidBound { depth: u32, index: usize },
    RuntimeInCore(RuntimeId),
    UnboundRuntime(RuntimeId),
    BinderDepthOverflow,
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongSort { expected, actual } => write!(f, "expected {expected}, got {actual}"),
            Self::MissingArgument(i) => write!(f, "missing substitution argument {i}"),
            Self::ArgumentCount { expected, actual } => {
                write!(f, "expected {expected} arguments, got {actual}")
            }
            Self::InvalidHole(i) => write!(f, "invalid or forward core hole {i}"),
            Self::InvalidBound { depth, index } => {
                write!(f, "invalid binder reference {depth}:{index}")
            }
            Self::RuntimeInCore(id) => write!(
                f,
                "runtime identity {id} must be abstracted into a core hole"
            ),
            Self::UnboundRuntime(id) => write!(
                f,
                "runtime identity {id} is unavailable in this environment"
            ),
            Self::BinderDepthOverflow => f.write_str("binder depth overflow"),
        }
    }
}
impl std::error::Error for TypeError {}

fn require_sort(term: &Term, expected: Sort) -> Result<(), TypeError> {
    let actual = term.sort();
    if actual == expected {
        Ok(())
    } else {
        Err(TypeError::WrongSort { expected, actual })
    }
}

/// A structurally validated telescope and immutable type body. Construction is
/// private so a shared core cannot acquire a runtime object or forward hole.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TypeCore {
    holes: Vec<Parameter>,
    body: Term,
}

impl TypeCore {
    pub fn holes(&self) -> &[Parameter] {
        &self.holes
    }
    pub fn body(&self) -> &Term {
        &self.body
    }
}

#[derive(Default)]
pub struct TypeInterner {
    cores: HashSet<Arc<TypeCore>>,
}

impl TypeInterner {
    pub fn intern(
        &mut self,
        holes: Vec<Parameter>,
        body: Term,
    ) -> Result<Arc<TypeCore>, TypeError> {
        for (index, hole) in holes.iter().enumerate() {
            require_sort(&hole.ty, Sort::Type)?;
            validate(&hole.ty, &holes[..index], &mut Vec::new(), false, true)?;
        }
        require_sort(&body, Sort::Type)?;
        validate(&body, &holes, &mut Vec::new(), false, true)?;
        let core = Arc::new(TypeCore { holes, body });
        if let Some(existing) = self.cores.get(&core) {
            return Ok(existing.clone());
        }
        self.cores.insert(core.clone());
        Ok(core)
    }

    pub fn len(&self) -> usize {
        self.cores.len()
    }
    pub fn is_empty(&self) -> bool {
        self.cores.is_empty()
    }
}

/// Immutable reference-side bindings. Construction validates shape and domain;
/// full parameter type/contract checking is explicitly performed by check_with.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TypeRef {
    core: Arc<TypeCore>,
    arguments: Vec<Term>,
}

impl TypeRef {
    pub fn bind(core: Arc<TypeCore>, arguments: Vec<Term>) -> Result<Self, TypeError> {
        if arguments.len() != core.holes.len() {
            return Err(TypeError::ArgumentCount {
                expected: core.holes.len(),
                actual: arguments.len(),
            });
        }
        for (argument, hole) in arguments.iter().zip(&core.holes) {
            require_sort(argument, hole.sort)?;
            // Open surrounding binder references are allowed. Core holes in a
            // reference-side value are not: they have no unambiguous owner.
            validate(argument, &[], &mut Vec::new(), true, false)?;
        }
        Ok(Self { core, arguments })
    }

    pub fn core(&self) -> &Arc<TypeCore> {
        &self.core
    }
    pub fn arguments(&self) -> &[Term] {
        &self.arguments
    }

    pub fn materialize(&self) -> Result<Term, TypeError> {
        expand_holes(&self.core.body, &self.arguments)
    }

    /// Check each parameter against its fully substituted dependent type in
    /// telescope order. The caller supplies semantic type/contract evidence;
    /// a mere structural interning hit must not bypass that check.
    pub fn check_with<E>(
        &self,
        mut check: impl FnMut(usize, &Term, &Term) -> Result<(), E>,
    ) -> Result<(), ContractError<E>> {
        for (index, (parameter, argument)) in
            self.core.holes.iter().zip(&self.arguments).enumerate()
        {
            let expected = expand_holes(&parameter.ty, &self.arguments[..index])
                .map_err(ContractError::Shape)?;
            check(index, argument, &expected).map_err(ContractError::Contract)?;
        }
        Ok(())
    }

    /// Compose the reference with a surrounding binder substitution. This
    /// creates a new reference and never modifies the shared core or old one.
    pub fn substitute(&self, substitution: &[Term]) -> Result<Self, TypeError> {
        Self::bind(
            self.core.clone(),
            self.arguments
                .iter()
                .map(|argument| argument.substitute(substitution))
                .collect::<Result<_, _>>()?,
        )
    }

    pub fn runtime_dependencies(&self) -> Result<BTreeSet<RuntimeId>, TypeError> {
        let mut result = BTreeSet::new();
        self.materialize()?.rewrite(0, &mut |term, _| {
            if let Term::Runtime(id) = term {
                result.insert(*id);
            }
            Ok(term.clone())
        })?;
        Ok(result)
    }

    /// Availability is a separate use-site check, not part of type identity.
    /// This does not prove lifetimes, resource validity or a legal access path.
    pub fn check_runtime_bindings(&self, available: &BTreeSet<RuntimeId>) -> Result<(), TypeError> {
        for dependency in self.runtime_dependencies()? {
            if !available.contains(&dependency) {
                return Err(TypeError::UnboundRuntime(dependency));
            }
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ContractError<E> {
    Shape(TypeError),
    Contract(E),
}

fn expand_holes(term: &Term, arguments: &[Term]) -> Result<Term, TypeError> {
    term.rewrite(0, &mut |term, nesting| match term {
        Term::Hole { index, sort } => {
            let value = arguments
                .get(*index)
                .ok_or(TypeError::MissingArgument(*index))?;
            require_sort(value, *sort)?;
            value.shift(nesting, 0)
        }
        _ => Ok(term.clone()),
    })
}

fn validate(
    term: &Term,
    holes: &[Parameter],
    frames: &mut Vec<Vec<Sort>>,
    allow_free: bool,
    core: bool,
) -> Result<(), TypeError> {
    match term {
        Term::Hole { index, sort } => {
            let hole = holes.get(*index).ok_or(TypeError::InvalidHole(*index))?;
            if hole.sort != *sort {
                return Err(TypeError::WrongSort {
                    expected: hole.sort,
                    actual: *sort,
                });
            }
        }
        Term::Bound { depth, index, sort } => {
            if let Some(frame) = (*depth as usize)
                .checked_add(1)
                .and_then(|distance| frames.len().checked_sub(distance))
                .map(|i| &frames[i])
            {
                let expected = *frame.get(*index).ok_or(TypeError::InvalidBound {
                    depth: *depth,
                    index: *index,
                })?;
                if expected != *sort {
                    return Err(TypeError::WrongSort {
                        expected,
                        actual: *sort,
                    });
                }
            } else if !allow_free {
                return Err(TypeError::InvalidBound {
                    depth: *depth,
                    index: *index,
                });
            }
        }
        Term::Runtime(id) if core => return Err(TypeError::RuntimeInCore(*id)),
        Term::Function { parameters, result } => {
            frames.push(Vec::new());
            let checked = (|| {
                for parameter in parameters {
                    require_sort(&parameter.ty, Sort::Type)?;
                    validate(&parameter.ty, holes, frames, allow_free, core)?;
                    frames.last_mut().unwrap().push(parameter.sort);
                }
                require_sort(result, Sort::Type)?;
                validate(result, holes, frames, allow_free, core)
            })();
            frames.pop();
            checked?;
        }
        Term::Tuple(items) => {
            for item in items {
                require_sort(item, Sort::Type)?;
                validate(item, holes, frames, allow_free, core)?;
            }
        }
        Term::Pointer(inner) => {
            require_sort(inner, Sort::Type)?;
            validate(inner, holes, frames, allow_free, core)?;
        }
        Term::Nominal { arguments, .. } | Term::NormalForm { arguments, .. } => {
            for argument in arguments {
                validate(argument, holes, frames, allow_free, core)?;
            }
        }
        _ => {}
    }
    Ok(())
}

impl fmt::Display for Sort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Type => "type",
            Self::ComptimeValue => "comptime-value",
            Self::Runtime => "runtime",
        })
    }
}
impl fmt::Display for Primitive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Type => "Type",
            Self::Bool => "bool",
            Self::Char => "char",
            Self::Str => "str",
            Self::Integer => "Integer",
            Self::Float => "Float",
            Self::Unit => "unit",
            Self::Never => "never",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::Isize => "isize",
            Self::Usize => "usize",
            Self::F32 => "f32",
            Self::F64 => "f64",
        })
    }
}
impl fmt::Display for DeclarationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(declaration (package {}) (local {}))",
            self.package, self.local
        )
    }
}
impl fmt::Display for RuntimeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(runtime (instance {}) (local {}))",
            self.instance, self.local
        )
    }
}
fn list(f: &mut fmt::Formatter<'_>, items: &[impl fmt::Display]) -> fmt::Result {
    f.write_str("(")?;
    for (index, item) in items.iter().enumerate() {
        if index != 0 {
            f.write_str(" ")?;
        }
        write!(f, "{item}")?;
    }
    f.write_str(")")
}
impl fmt::Display for Parameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(parameter (sort {}) (type {}))", self.sort, self.ty)
    }
}
impl fmt::Display for Term {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Primitive(p) => write!(f, "(primitive {p})"),
            Self::Nominal {
                declaration,
                arguments,
            } => {
                write!(f, "(nominal (head {declaration}) (arguments ")?;
                list(f, arguments)?;
                f.write_str("))")
            }
            Self::NormalForm {
                head,
                sort,
                arguments,
            } => {
                write!(f, "(normal-form (head {head}) (sort {sort}) (arguments ")?;
                list(f, arguments)?;
                f.write_str("))")
            }
            Self::Tuple(items) => {
                f.write_str("(tuple ")?;
                list(f, items)?;
                f.write_str(")")
            }
            Self::Pointer(inner) => write!(f, "(pointer {inner})"),
            Self::Function { parameters, result } => {
                f.write_str("(function (parameters ")?;
                list(f, parameters)?;
                write!(f, ") (result {result}))")
            }
            Self::Hole { index, sort } => write!(f, "(hole (index {index}) (sort {sort}))"),
            Self::Bound { depth, index, sort } => {
                write!(f, "(bound (depth {depth}) (index {index}) (sort {sort}))")
            }
            Self::Rigid { declaration, sort } => {
                write!(f, "(rigid (id {declaration}) (sort {sort}))")
            }
            Self::Runtime(id) => write!(f, "{id}"),
            Self::Bool(value) => write!(f, "(bool {value})"),
            Self::Value { store, index } => write!(f, "(value (store {store}) (index {index}))"),
        }
    }
}
impl fmt::Display for TypeCore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(type-core (holes ")?;
        list(f, &self.holes)?;
        write!(f, ") (body {}))", self.body)
    }
}
impl fmt::Display for TypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(type-ref (core {}) (substitution ", self.core)?;
        list(f, &self.arguments)?;
        f.write_str("))")
    }
}
