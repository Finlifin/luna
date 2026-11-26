//! A11 `NotSized(T)` layout-domain predicate (`docs/type/propositional-constraints/atoms.md`).
//!
//! `NotSized(T)` is a representation property over valid runtime-representable types:
//! - Comptime-only types have no runtime layout and are prerequisite errors (`IllFormed`),
//!   never `true` or `false`.
//! - Unknown/opaque type parameters or unresolved heads remain `Unknown` (neither `true`
//!   nor `false`).
use crate::declaration_theory::ConstructorContracts;
use nir::{DeclarationId, Primitive, Sort, Term};
use std::{collections::BTreeSet, fmt};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NotSizedQuery {
    pub ty: Term,
}

impl fmt::Display for NotSizedQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(not-sized {})", self.ty)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotSizedStatus {
    /// Valid runtime representation whose size-dependence status is known.
    Known(bool),
    /// Comptime-only or non-representable domain: must surface as a prerequisite error.
    InvalidRepresentation(&'static str),
    /// Unresolved/opaque type: neither known fixed-size nor known unsized.
    Unknown,
}

const COMPTIME_ONLY_REASON: &str =
    "comptime-only type has no runtime representation for Type.predicates.is_not_sized";

pub(crate) fn evaluate(
    ty: &Term,
    constructors: &ConstructorContracts,
    list_heads: &BTreeSet<DeclarationId>,
) -> NotSizedStatus {
    evaluate_at(ty, constructors, list_heads, 0)
}

fn evaluate_at(
    ty: &Term,
    constructors: &ConstructorContracts,
    list_heads: &BTreeSet<DeclarationId>,
    depth: usize,
) -> NotSizedStatus {
    if depth >= 128 {
        return NotSizedStatus::Unknown;
    }
    match ty {
        Term::Primitive(
            Primitive::Type
            | Primitive::Object
            | Primitive::Integer
            | Primitive::Float
            | Primitive::Unit,
        ) => NotSizedStatus::InvalidRepresentation(COMPTIME_ONLY_REASON),
        Term::ComptimeFunction(_)
        | Term::NormalFormFunction(..)
        | Term::Lift(_)
        | Term::ClosureContextType
        | Term::EffectRowType
        | Term::ErrorRowType
        | Term::ReachabilityType
        | Term::AnyType => NotSizedStatus::InvalidRepresentation(COMPTIME_ONLY_REASON),
        Term::Function { parameters, .. } => {
            if parameters.iter().any(|param| param.sort != Sort::Runtime) {
                NotSizedStatus::InvalidRepresentation(COMPTIME_ONLY_REASON)
            } else {
                NotSizedStatus::Known(false)
            }
        }
        Term::UnsafeFunction(inner) | Term::PureFunction(inner) => {
            evaluate_at(inner, constructors, list_heads, depth + 1)
        }
        Term::Closure {
            environment,
            signature,
        } => {
            if !matches!(signature.as_ref(), Term::TraitObject { .. })
                && !matches!(
                    signature.callable_signature(),
                    Term::Function { parameters, .. }
                        if parameters.iter().all(|param| param.sort == Sort::Runtime)
                )
            {
                return NotSizedStatus::InvalidRepresentation(COMPTIME_ONLY_REASON);
            }
            evaluate_aggregate(environment.iter(), constructors, list_heads, depth + 1)
        }
        Term::Primitive(
            Primitive::Bool
            | Primitive::Char
            | Primitive::Str
            | Primitive::Symbol
            | Primitive::Void
            | Primitive::Never
            | Primitive::Continuation
            | Primitive::I8
            | Primitive::I16
            | Primitive::I32
            | Primitive::I64
            | Primitive::I128
            | Primitive::Isize
            | Primitive::U8
            | Primitive::U16
            | Primitive::U32
            | Primitive::U64
            | Primitive::U128
            | Primitive::Usize
            | Primitive::F16
            | Primitive::Bf16
            | Primitive::F32
            | Primitive::F64
            | Primitive::BitVec(_),
        ) => NotSizedStatus::Known(false),
        // Pointers and bare dynamic trait objects carry fixed target pointer representation.
        Term::Pointer(_) | Term::TraitObject { .. } => NotSizedStatus::Known(false),
        Term::Optional(inner) => evaluate_at(inner, constructors, list_heads, depth + 1),
        Term::Tuple(elements) => {
            evaluate_aggregate(elements.iter(), constructors, list_heads, depth + 1)
        }
        Term::Record(fields) => evaluate_aggregate(
            fields.iter().map(|(_, field_ty)| field_ty),
            constructors,
            list_heads,
            depth + 1,
        ),
        Term::ErrorResult { errors, result } => evaluate_aggregate(
            errors.iter().chain(std::iter::once(result.as_ref())),
            constructors,
            list_heads,
            depth + 1,
        ),
        Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments,
        } => {
            if list_heads.contains(head) {
                return NotSizedStatus::InvalidRepresentation(COMPTIME_ONLY_REASON);
            }
            if constructors.tuple_heads.contains(head) {
                if let Some(elements) =
                    nir::normal_forms::type_list_fields_with_layouts(arguments, &|id| {
                        list_heads.contains(&id)
                    })
                {
                    return evaluate_aggregate(
                        elements.iter(),
                        constructors,
                        list_heads,
                        depth + 1,
                    );
                }
            }
            if constructors.array_heads.contains(head) && arguments.len() == 2 {
                return evaluate_aggregate(
                    arguments[..1].iter(),
                    constructors,
                    list_heads,
                    depth + 1,
                );
            }
            if constructors.storage_heads.contains(head) && arguments.len() == 1 {
                return evaluate_aggregate(arguments.iter(), constructors, list_heads, depth + 1);
            }
            NotSizedStatus::Unknown
        }
        Term::Nominal {
            declaration,
            arguments,
        } => {
            if arguments.is_empty() {
                if constructors.comptime_only_nominals.contains(declaration) {
                    NotSizedStatus::InvalidRepresentation(COMPTIME_ONLY_REASON)
                } else if constructors.sized_nominals.contains(declaration) {
                    NotSizedStatus::Known(false)
                } else {
                    NotSizedStatus::Unknown
                }
            } else {
                NotSizedStatus::Unknown
            }
        }
        _ => NotSizedStatus::Unknown,
    }
}

fn evaluate_aggregate<'a>(
    fields: impl Iterator<Item = &'a Term>,
    constructors: &ConstructorContracts,
    list_heads: &BTreeSet<DeclarationId>,
    depth: usize,
) -> NotSizedStatus {
    let mut saw_unknown = false;
    let mut saw_unsized = false;
    for field in fields {
        match evaluate_at(field, constructors, list_heads, depth) {
            invalid @ NotSizedStatus::InvalidRepresentation(_) => return invalid,
            NotSizedStatus::Unknown => saw_unknown = true,
            NotSizedStatus::Known(true) => saw_unsized = true,
            NotSizedStatus::Known(false) => {}
        }
    }
    if saw_unknown {
        NotSizedStatus::Unknown
    } else {
        NotSizedStatus::Known(saw_unsized)
    }
}

/// Extract the direct field/payload types of an unconditional nullary nominal
/// declaration so `DeclarationTheory` can resolve fixed-size status regardless
/// of declaration admission order.
pub(crate) fn nullary_definition_fields(
    definition: nir::normal_forms::NominalValueDeclaration<'_>,
) -> Option<Vec<Term>> {
    if !definition.is_unconditional_nullary() {
        return None;
    }
    use nir::normal_forms::NominalValueDeclaration as D;
    Some(match definition {
        D::Struct(def) => def.fields.iter().map(|(_, ty)| ty.clone()).collect(),
        D::Enum(def) => def
            .variants
            .iter()
            .flat_map(|v| v.fields.iter().cloned())
            .collect(),
        D::Newtype(def) => vec![def.representation.clone()],
    })
}

pub(crate) fn evaluate_fields(
    fields: &[Term],
    constructors: &ConstructorContracts,
    list_heads: &BTreeSet<DeclarationId>,
) -> NotSizedStatus {
    evaluate_aggregate(fields.iter(), constructors, list_heads, 0)
}

/// Classify an unconditional nullary nominal declaration's runtime layout status
/// under the current `constructors` and `list_heads`.
pub(crate) fn evaluate_definition(
    definition: nir::normal_forms::NominalValueDeclaration<'_>,
    constructors: &ConstructorContracts,
    list_heads: &BTreeSet<DeclarationId>,
) -> NotSizedStatus {
    if !definition.is_unconditional_nullary() {
        return NotSizedStatus::Unknown;
    }
    use nir::normal_forms::NominalValueDeclaration as D;
    match definition {
        D::Struct(def) => evaluate_aggregate(
            def.fields.iter().map(|(_, ty)| ty),
            constructors,
            list_heads,
            0,
        ),
        D::Enum(def) => evaluate_aggregate(
            def.variants.iter().flat_map(|v| v.fields.iter()),
            constructors,
            list_heads,
            0,
        ),
        D::Newtype(def) => evaluate_at(&def.representation, constructors, list_heads, 0),
    }
}
