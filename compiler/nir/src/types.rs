//! Immutable type cores and reference-side substitutions.
//!
//! Contracts: docs/compilation/runtime-dependent-types.md and
//! docs/type/type-identity.md. Function parameters introduce one de Bruijn
//! frame; their types can refer to earlier parameters in that frame. Holes
//! belong to a core's ordered telescope and are distinct from bound variables.

use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::sync::Arc;

mod input_binding;
pub use input_binding::{FunctionCallBinding, FunctionInputBinding, FunctionInputBuilder};

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
    /// Concrete intrinsic bit-vector width; symbolic widths require later elaboration.
    BitVec(usize),
    Type,
    /// Heterogeneous compile-time metadata; no runtime layout.
    Object,
    Bool,
    Char,
    Str,
    Symbol,
    Integer,
    Float,
    Unit,
    Void,
    Never,
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    Isize,
    Usize,
    F16,
    Bf16,
    F32,
    F64,
}

impl Primitive {
    /// Width of a readable initialized scalar value. This does not authorize
    /// reconstruction or bit writeback: Char excludes non-Unicode bit patterns.
    pub fn readable_scalar_width_in(
        self,
        target: Option<crate::target::TargetData>,
    ) -> Option<u32> {
        match self {
            Self::Char => Some(32),
            _ => self.unrestricted_scalar_width_in(target),
        }
    }

    /// Supported initialized scalar representations with no invalid bit patterns.
    /// This describes values only; access, initialization and write permissions
    /// must be established separately. Pointer-sized types need target data.
    pub fn unrestricted_scalar_width(self) -> Option<u32> {
        match self {
            Self::F16 | Self::Bf16 => Some(16),
            Self::F32 => Some(32),
            Self::F64 => Some(64),
            _ => self.fixed_integer_layout().map(|(_, width)| width),
        }
    }

    /// Representation width with explicit target facts; never guesses a host width.
    pub fn unrestricted_scalar_width_in(
        self,
        target: Option<crate::target::TargetData>,
    ) -> Option<u32> {
        match self {
            Self::Isize | Self::Usize => target.map(|target| target.pointer_bits()),
            _ => self.unrestricted_scalar_width(),
        }
    }

    /// Fixed-width integer proxies only. Pointer-sized proxies require target
    /// data and must never silently use the compiler host's pointer width.
    pub fn fixed_integer_layout(self) -> Option<(bool, u32)> {
        Some(match self {
            Self::I8 => (true, 8),
            Self::I16 => (true, 16),
            Self::I32 => (true, 32),
            Self::I64 => (true, 64),
            Self::I128 => (true, 128),
            Self::U8 => (false, 8),
            Self::U16 => (false, 16),
            Self::U32 => (false, 32),
            Self::U64 => (false, 64),
            Self::U128 => (false, 128),
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Parameter {
    pub sort: Sort,
    pub ty: Term,
}

/// Stable declaration input identity and semantic domain. The input's source
/// type and its constraints belong to the declaration contract, not this sort.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeclarationParameter {
    pub declaration: DeclarationId,
    pub sort: Sort,
}
impl fmt::Display for DeclarationParameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(declaration-parameter (identity {}) (sort {}))",
            self.declaration, self.sort
        )
    }
}
fn declaration_parameter_indices(
    parameters: &[DeclarationParameter],
) -> Result<std::collections::BTreeMap<DeclarationId, (usize, Sort)>, TypeError> {
    let mut indices = std::collections::BTreeMap::new();
    for (index, parameter) in parameters.iter().enumerate() {
        if indices
            .insert(parameter.declaration, (index, parameter.sort))
            .is_some()
        {
            return Err(TypeError::DuplicateDeclarationParameter(
                parameter.declaration,
            ));
        }
    }
    Ok(indices)
}

/// Nominal origin of an associated query, boxed in Term to keep the common
/// type representation compact. It never records a selected implementation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssociatedTypeOrigin {
    pub interface: DeclarationId,
    pub member: DeclarationId,
}

/// A stable symbolic term. Eq/Hash compare representation only. In particular,
/// two unequal terms can still require a semantic equality query.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Term {
    Primitive(Primitive),
    /// Error labels are complete Type instances, independent of effect entries.
    ErrorResult {
        errors: Vec<Term>,
        result: Box<Term>,
    },
    /// Scoped effect occurrences in order. Repetition records handler layers,
    /// not the number of requests. Use `computation` to join nested prefixes.
    Computation {
        effects: Vec<crate::effects::Operation>,
        result: Box<Term>,
    },
    /// A Type-valued member of a complete interface instance. This records a
    /// query, not an implementation choice or a nominal/injective type head.
    /// Binder substitution preserves the query; only implementation evidence
    /// may resolve it. The interface member's declared sort is checked by the
    /// producer against the interface world, separately from structural scope.
    AssociatedType {
        origin: Box<AssociatedTypeOrigin>,
        arguments: Vec<Term>,
        subject: Box<Term>,
    },
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
    /// Domain of ordered closure contexts, distinct from Type and type lists.
    ClosureContextType,
    /// A completed context's capture types in semantic order. Ordinary capture
    /// values are carried by closures, not by this type-level context value.
    ClosureContextValue(Vec<Term>),
    /// Stable context binder applied to a callable type. No inference metas
    /// live here; substitution completes this into Closure or an empty body.
    ContextualClosure {
        context: Box<Term>,
        signature: Box<Term>,
    },
    /// Structural closure environment; values and body identity are not type identity.
    Closure {
        environment: Vec<Term>,
        signature: Box<Term>,
    },
    /// Internal product view and legacy unbound frontend representation.
    /// The language's `Tuple<...Es: meta.List<Type>>` type is a `NormalForm`
    /// with one ordinary list value input; this view does not replace its identity.
    Tuple(Vec<Term>),
    /// Closed named product. Field order is retained in the representation;
    /// representation equality alone does not decide permutation equivalence.
    Record(Vec<(String, Term)>),
    /// An immutable comptime tuple value; distinct from a tuple Type.
    TupleValue(Vec<Term>),
    /// Ordered concatenation of list values in one declared domain.
    /// This residual must complete before immutable value admission.
    ListConcat {
        ty: Box<Term>,
        parts: Vec<Term>,
    },
    /// Completed structural value; field labels and order are part of identity.
    RecordValue(Vec<(String, Term)>),
    /// Closed value of an identity-bearing aggregate. Nominal fields follow
    /// declaration order; trusted normal-form collections retain element order.
    /// Some(index) distinguishes nominal enum variants. A normal-form head
    /// carries no layout authority here: admission must check its contract.
    /// Builtin Object uses ordered typed entry packets (see `crate::objects`).
    NominalValue {
        ty: Box<Term>,
        variant: Option<u32>,
        fields: Vec<Term>,
    },
    /// Frozen optional data. None marks absence at exactly this outer layer.
    OptionalValue {
        ty: Box<Term>,
        value: Option<Box<Term>>,
    },
    /// Frozen error result. Branch identity is its full label, never a layout tag.
    ErrorValue {
        ty: Box<Term>,
        label: Option<Box<Term>>,
        value: Box<Term>,
    },
    /// Language optional qualification. Every layer is observable; it is neither
    /// a nominal enum application nor a flattenable error/effect row.
    Optional(Box<Term>),
    Pointer(Box<Term>),
    /// A comptime callable. Its inner signature must be an ordinary Function.
    ComptimeFunction(Box<Term>),
    /// Opaque normal-form callable. The inner Function owns its dependent
    /// input frame; the optional index marks a collected input, independent
    /// of its value domain. It is not an ordinary comptime invocation.
    NormalFormFunction(Box<Term>, Option<usize>),
    /// Caller responsibility, independent of phase. Canonically wraps an ordinary
    /// Function, ComptimeFunction or NormalFormFunction; environments remain outside it.
    UnsafeFunction(Box<Term>),
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
    /// A definition-side rigid parameter; ordinary binder substitution cannot assign it.
    Rigid {
        declaration: DeclarationId,
        sort: Sort,
    },
    Runtime(RuntimeId),
    Unit,
    Bool(bool),
    Char(char),
    BitVec(comptime_runtime::bitvec::BitVec),
    /// Completed floating values preserve IEEE bits, not numerical equality.
    Float16(u16),
    BFloat16(u16),
    Float32(u32),
    Float64(u64),
    /// Immutable UTF-8 contents, compared by value rather than allocation identity.
    String(Arc<str>),
    Symbol(Arc<str>),
    /// Immutable identity of a named callable declaration and its instance inputs.
    /// Execution phase and responsibility come from the declaration and value domain;
    /// freezing an identity never grants permission to invoke it.
    /// This is a value, not its callable type or an unfolded normal-form head.
    FunctionValue {
        declaration: DeclarationId,
        arguments: Vec<Term>,
    },
    /// Checked reconstruction of a closure whose captured data cannot be mutated
    /// or addressed by its body. The factory owns the complete constant graph.
    ClosureSnapshot {
        factory: DeclarationId,
        arguments: Vec<Term>,
    },
    /// Canonical exact builtin integer; no machine-width truncation or store identity.
    Integer(comptime_runtime::Integer),
    Float(comptime_runtime::decimal::DecimalValue),
    /// Handle into a comptime value store. Structural identity does not replace
    /// that store's semantic value equality (e.g. Eq/Hash protocols).
    Value {
        store: u64,
        index: u64,
    },
}

impl Term {
    /// Coalesce exact repeated labels and nested qualifiers without unfolding
    /// opaque heads. Canonical label order gives equal closed rows one receiver
    /// representation, including after substitution. Semantic label equality is
    /// resolved by elaboration; sorting does not assert new type equations.
    pub fn error_result(mut errors: Vec<Term>, mut result: Term) -> Self {
        while let Self::ErrorResult {
            errors: inner,
            result: output,
        } = result
        {
            errors.extend(inner);
            result = *output;
        }
        let mut seen = std::collections::HashSet::new();
        errors.retain(|label| seen.insert(label.clone()));
        errors.sort_by_cached_key(ToString::to_string);
        if errors.is_empty() {
            result
        } else {
            Self::ErrorResult {
                errors,
                result: Box::new(result),
            }
        }
    }

    /// Rebuild a frozen result after substituting or resolving its components.
    /// An exposed inner result may flatten only with matching domain evidence.
    /// Callers meter component traversal before invoking this constructor.
    pub fn error_value(
        errors: Vec<Term>,
        result: Term,
        mut label: Option<Box<Term>>,
        mut value: Term,
    ) -> Result<Self, TypeError> {
        if errors.is_empty()
            || errors.iter().any(|ty| ty.sort() != Sort::Type)
            || result.sort() != Sort::Type
            || label.as_ref().is_some_and(|label| !errors.contains(label))
            || value.sort() == Sort::Runtime
        {
            return Err(TypeError::InvalidErrorValue);
        }
        let mut domain = result.clone();
        let mut depth = 0;
        while label.is_none() {
            if value.has_pending_error_branch() {
                let Self::ErrorValue { ty, .. } = &value else {
                    unreachable!()
                };
                let same_domain = ty.as_ref() == &domain
                    || match (ty.as_ref(), &domain) {
                        (
                            Self::ErrorResult {
                                errors: a,
                                result: ar,
                            },
                            Self::ErrorResult {
                                errors: b,
                                result: br,
                            },
                        ) => {
                            Self::error_result(a.clone(), ar.as_ref().clone())
                                == Self::error_result(b.clone(), br.as_ref().clone())
                        }
                        _ => false,
                    };
                if !same_domain {
                    return Err(TypeError::InvalidErrorValue);
                }
                break;
            }
            let Self::ErrorValue {
                ty,
                label: inner_label,
                value: inner_value,
            } = value
            else {
                break;
            };
            depth += 1;
            if depth >= 128 || *ty != domain {
                return Err(TypeError::InvalidErrorValue);
            }
            let Self::ErrorResult { errors, result } = *ty else {
                return Err(TypeError::InvalidErrorValue);
            };
            if errors.is_empty()
                || inner_label
                    .as_ref()
                    .is_some_and(|label| !errors.contains(label))
            {
                return Err(TypeError::InvalidErrorValue);
            }
            domain = *result;
            label = inner_label;
            value = *inner_value;
        }
        // A symbolic inner result has no selected branch yet. Retain its
        // original success domain until substitution exposes the value; an
        // eager flatten would reinterpret that whole result as a success leaf.
        let ty = if label.is_none() && matches!(domain, Self::ErrorResult { .. }) {
            Self::ErrorResult {
                errors,
                result: Box::new(result),
            }
        } else {
            Self::error_result(errors, result)
        };
        Ok(Self::ErrorValue {
            ty: Box::new(ty),
            label,
            value: Box::new(value),
        })
    }

    /// The outer success contains a whole result whose branch is not yet
    /// available. Consumers must not treat this residual as a selected success.
    pub fn has_pending_error_branch(&self) -> bool {
        let Self::ErrorValue {
            ty,
            label: None,
            value,
        } = self
        else {
            return false;
        };
        let Self::ErrorResult { result, .. } = ty.as_ref() else {
            return false;
        };
        matches!(result.as_ref(), Self::ErrorResult { .. })
            || (matches!(
                result.as_ref(),
                Self::Rigid { .. }
                    | Self::Bound { .. }
                    | Self::Hole { .. }
                    | Self::AssociatedType { .. }
                    | Self::NormalForm { .. }
            ) && matches!(
                value.as_ref(),
                Self::Rigid { .. }
                    | Self::Bound { .. }
                    | Self::Hole { .. }
                    | Self::Value { .. }
                    | Self::NormalForm { .. }
            ))
    }

    /// Free runtime object identities retained by this type expression.
    /// Contextual code must either substitute these consistently or keep an
    /// explicit unsupported obligation; shape equality alone cannot erase them.
    pub fn runtime_dependencies(&self) -> BTreeSet<RuntimeId> {
        let mut result = BTreeSet::new();
        collect_runtime_dependencies(self, &mut result);
        result
    }
    /// Release an owned IR tree without using the host call stack for children.
    /// Useful after rejecting untrusted/deep input. This is compiler-memory
    /// disposal, not execution of a source-language Drop implementation.
    pub fn drop_iteratively(self) {
        // Releasing a leaf must not allocate a traversal vector of its own.
        // Descendant disposal remains iterative even for rejected deep input.
        let mut pending = Vec::new();
        let mut first = Some(self);
        while let Some(term) = first.take().or_else(|| pending.pop()) {
            match term {
                Self::ErrorResult { errors, result } => {
                    pending.extend(errors);
                    pending.push(*result);
                }
                Self::Computation { effects, result } => {
                    pending.push(*result);
                    for operation in effects {
                        pending.extend(operation.arguments);
                    }
                }
                Self::AssociatedType {
                    arguments, subject, ..
                } => {
                    pending.push(*subject);
                    pending.extend(arguments);
                }
                Self::ClosureSnapshot { arguments, .. }
                | Self::FunctionValue { arguments, .. }
                | Self::NormalForm { arguments, .. }
                | Self::Nominal { arguments, .. }
                | Self::Tuple(arguments)
                | Self::TupleValue(arguments)
                | Self::ClosureContextValue(arguments) => pending.extend(arguments),
                Self::Record(fields) | Self::RecordValue(fields) => {
                    pending.extend(fields.into_iter().map(|(_, field)| field))
                }
                Self::OptionalValue { ty, value } => {
                    pending.push(*ty);
                    pending.extend(value.map(|value| *value));
                }
                Self::ErrorValue { ty, label, value } => {
                    pending.push(*ty);
                    pending.extend(label.map(|label| *label));
                    pending.push(*value);
                }
                Self::ListConcat { ty, parts: fields } | Self::NominalValue { ty, fields, .. } => {
                    pending.push(*ty);
                    pending.extend(fields);
                }
                Self::Closure {
                    environment,
                    signature,
                } => {
                    pending.push(*signature);
                    pending.extend(environment);
                }
                Self::Optional(inner)
                | Self::Pointer(inner)
                | Self::ComptimeFunction(inner)
                | Self::NormalFormFunction(inner, _)
                | Self::UnsafeFunction(inner) => pending.push(*inner),
                Self::Function { parameters, result } => {
                    pending.push(*result);
                    pending.extend(parameters.into_iter().map(|parameter| parameter.ty));
                }
                Self::ContextualClosure { context, signature } => {
                    pending.push(*context);
                    pending.push(*signature);
                }
                Self::ClosureContextType
                | Self::Primitive(_)
                | Self::Hole { .. }
                | Self::Bound { .. }
                | Self::Rigid { .. }
                | Self::Runtime(_)
                | Self::Value { .. }
                | Self::Unit
                | Self::Bool(_)
                | Self::Char(_)
                | Self::BitVec(_)
                | Self::Float16(_)
                | Self::BFloat16(_)
                | Self::Float32(_)
                | Self::Float64(_)
                | Self::String(_)
                | Self::Symbol(_)
                | Self::Float(_)
                | Self::Integer(_) => {}
            }
        }
    }

    /// Normalize an admitted collection operation without unfolding its head.
    /// Domain admission belongs to the producer; this checks representation and
    /// preserves order while substitution supplies previously unknown pieces.
    /// Like structural rewriting, callers precharge retained input structure.
    pub fn list_concat(ty: Term, parts: Vec<Term>) -> Result<Self, TypeError> {
        ty.list_element_domain()
            .ok_or(TypeError::InvalidListComponent)?;
        let mut pending = parts.into_iter().rev().collect::<Vec<_>>();
        let mut result = Vec::new();
        let mut fields = Vec::new();
        let flush = |fields: &mut Vec<Term>, result: &mut Vec<Term>| -> Result<(), TypeError> {
            if !fields.is_empty() {
                result.push(
                    crate::comptime_values::encode_admitted_list(
                        ty.clone(),
                        std::mem::take(fields),
                    )
                    .map_err(|_| TypeError::InvalidListComponent)?,
                );
            }
            Ok(())
        };
        while let Some(part) = pending.pop() {
            if !part.is_list_component(&ty) {
                return Err(TypeError::InvalidListComponent);
            }
            match part {
                Self::ListConcat { parts, .. } => pending.extend(parts.into_iter().rev()),
                Self::NominalValue {
                    fields: elements, ..
                } => {
                    fields.extend(elements);
                }
                other => {
                    flush(&mut fields, &mut result)?;
                    result.push(other);
                }
            }
        }
        flush(&mut fields, &mut result)?;
        Ok(if result.is_empty() {
            crate::comptime_values::encode_admitted_list(ty, vec![])
                .map_err(|_| TypeError::InvalidListComponent)?
        } else if result.len() == 1 {
            result.pop().unwrap()
        } else {
            Self::ListConcat {
                ty: Box::new(ty),
                parts: result,
            }
        })
    }
    fn list_element_domain(&self) -> Option<&Term> {
        match self {
            Self::NormalForm {
                sort: Sort::Type,
                arguments,
                ..
            } => match arguments.as_slice() {
                [element] if element.sort() == Sort::Type => Some(element),
                _ => None,
            },
            _ => None,
        }
    }
    fn is_list_component(&self, domain: &Term) -> bool {
        let Some(element) = domain.list_element_domain() else {
            return false;
        };
        let types = *element == Self::Primitive(Primitive::Type);
        let sort = if types {
            Sort::Type
        } else {
            Sort::ComptimeValue
        };
        match self {
            Self::NominalValue {
                ty,
                variant: None,
                fields,
            } => ty.as_ref() == domain && fields.iter().all(|field| field.sort() == sort),
            Self::ListConcat { ty, .. } => ty.as_ref() == domain,
            Self::Rigid {
                sort: Sort::ComptimeValue,
                ..
            }
            | Self::Bound {
                sort: Sort::ComptimeValue,
                ..
            }
            | Self::Hole {
                sort: Sort::ComptimeValue,
                ..
            }
            | Self::NormalForm {
                sort: Sort::ComptimeValue,
                ..
            } => true,
            _ => false,
        }
    }
    /// Empty qualification disappears; adjacent effect prefixes concatenate.
    /// Do not deduplicate occurrences or reorder labels without evidence.
    pub fn computation(mut effects: Vec<crate::effects::Operation>, mut result: Term) -> Term {
        while let Term::Computation {
            effects: inner,
            result: value,
        } = result
        {
            effects.extend(inner);
            result = *value;
        }
        if effects.is_empty() {
            result
        } else {
            Self::Computation {
                effects,
                result: Box::new(result),
            }
        }
    }
    /// The parameter/result binder, without erasing the identity of this type.
    pub fn callable_signature(&self) -> &Term {
        match self {
            Self::ComptimeFunction(signature)
            | Self::NormalFormFunction(signature, _)
            | Self::UnsafeFunction(signature) => signature.callable_signature(),
            Self::Closure { signature, .. } | Self::ContextualClosure { signature, .. } => {
                signature.callable_signature()
            }
            _ => self,
        }
    }
    pub fn is_comptime_callable(&self) -> bool {
        match self {
            Self::ComptimeFunction(_) | Self::NormalFormFunction(..) => true,
            Self::UnsafeFunction(signature) => signature.is_comptime_callable(),
            Self::Closure { signature, .. } | Self::ContextualClosure { signature, .. } => {
                signature.is_comptime_callable()
            }
            _ => false,
        }
    }
    pub fn is_normal_form_callable(&self) -> bool {
        match self {
            Self::NormalFormFunction(..) => true,
            Self::UnsafeFunction(signature)
            | Self::Closure { signature, .. }
            | Self::ContextualClosure { signature, .. } => signature.is_normal_form_callable(),
            _ => false,
        }
    }
    pub fn is_unsafe_callable(&self) -> bool {
        match self {
            Self::UnsafeFunction(_) => true,
            Self::Closure { signature, .. } | Self::ContextualClosure { signature, .. } => {
                signature.is_unsafe_callable()
            }
            _ => false,
        }
    }
    pub fn with_comptime_phase(self, comptime: bool) -> Term {
        if let Self::UnsafeFunction(signature) = self {
            return Self::UnsafeFunction(Box::new(signature.with_comptime_phase(comptime)));
        }
        if matches!(self, Self::NormalFormFunction(..)) {
            return self;
        }
        if comptime {
            Self::ComptimeFunction(Box::new(self))
        } else {
            self
        }
    }

    /// Add caller responsibility after phase qualification. A false request
    /// never strips an existing responsibility from an already formed type.
    pub fn with_unsafe_call_requirement(self, required: bool) -> Term {
        if required && !matches!(self, Self::UnsafeFunction(_)) {
            Self::UnsafeFunction(Box::new(self))
        } else {
            self
        }
    }

    /// Replace only the callable's result. Environment and phase wrappers are
    /// identity-bearing and must survive reconstructing the inner signature.
    /// This operates on an already checked type; it does not validate a new
    /// result's binder scope or discharge its formation obligations.
    pub fn with_callable_result(mut self, result: Term) -> Option<Term> {
        let mut current = &mut self;
        loop {
            match current {
                Self::Function { result: target, .. } => {
                    *target = Box::new(result);
                    return Some(self);
                }
                Self::Closure { signature, .. }
                | Self::ContextualClosure { signature, .. }
                | Self::ComptimeFunction(signature)
                | Self::NormalFormFunction(signature, _)
                | Self::UnsafeFunction(signature) => {
                    current = signature;
                }
                _ => return None,
            }
        }
    }

    /// Bind selected inputs and remove their slots from a function telescope.
    /// `None` retains an input; `Some` supplies a closed symbolic value. This
    /// only checks scope and sorts. The caller must check source value types,
    /// contracts and stage availability before erasing call arguments.
    /// Retained dependencies are renumbered without removing the function frame.
    /// A comptime-value input whose substituted type is exactly Type acquires
    /// the Type domain; the returned binding plan also refines its references.
    pub fn bind_function_inputs(&self, inputs: &[Option<Term>]) -> Result<Term, TypeError> {
        self.prepare_function_inputs(inputs)
            .map(FunctionInputBinding::into_signature)
    }

    /// Prepare a checked, reusable substitution for both the signature and
    /// expression annotations in its frame.
    pub fn prepare_function_inputs(
        &self,
        inputs: &[Option<Term>],
    ) -> Result<FunctionInputBinding, TypeError> {
        FunctionInputBinding::new(self, inputs)
    }

    /// Resolve each substituted parameter domain before refining its input
    /// sort. The trusted view sees the retained prefix's binder scope and must
    /// preserve type meaning; its result is checked for sort and scope.
    pub fn prepare_function_inputs_with_type_view<E: From<TypeError>>(
        &self,
        inputs: &[Option<Term>],
        view: impl FnMut(&Term, &[Parameter]) -> Result<Term, E>,
    ) -> Result<FunctionInputBinding, E> {
        FunctionInputBinding::with_type_view(self, inputs, view)
    }

    /// Instantiate a stable function signature in caller scope. Arguments are
    /// already-prepared symbolic values, not expressions to evaluate here.
    /// Structural checks run before semantic checks. The caller owns the
    /// candidate transaction and must discard its evidence on failure.
    pub fn instantiate_function<E>(
        &self,
        arguments: &[Term],
        mut check: impl FnMut(usize, &Term, &Term) -> Result<(), E>,
    ) -> Result<Term, CallError<E>> {
        let Self::Function { parameters, result } = self.callable_signature() else {
            return Err(CallError::NotFunction);
        };
        if parameters.len() != arguments.len() {
            return Err(CallError::Shape(TypeError::ArgumentCount {
                expected: parameters.len(),
                actual: arguments.len(),
            }));
        }
        validate(self, &[], &mut Vec::new(), true, false).map_err(CallError::Shape)?;
        for argument in arguments {
            validate(argument, &[], &mut Vec::new(), true, false).map_err(CallError::Shape)?;
        }
        let mut type_slots = BTreeSet::new();
        let mut expected_types = Vec::with_capacity(parameters.len());
        for (index, (parameter, argument)) in parameters.iter().zip(arguments).enumerate() {
            let expected =
                input_binding::replace_scope(&parameter.ty, &arguments[..index], &type_slots, true)
                    .map_err(CallError::Shape)?;
            let sort = input_binding::input_sort(parameter, &expected);
            require_sort(argument, sort).map_err(CallError::Shape)?;
            if sort != parameter.sort {
                type_slots.insert(index);
            }
            expected_types.push(expected);
        }
        for (index, (argument, expected)) in arguments.iter().zip(&expected_types).enumerate() {
            check(index, argument, expected).map_err(|error| CallError::Contract {
                parameter: index,
                error,
            })?;
        }
        input_binding::replace_scope(result, arguments, &type_slots, true).map_err(CallError::Shape)
    }

    /// Validate stable term shape and binder scope without changing its head.
    pub fn check_scope(&self, frames: &[Vec<Sort>]) -> Result<(), TypeError> {
        validate(self, &[], &mut frames.to_vec(), false, false)
    }

    pub fn sort(&self) -> Sort {
        match self {
            Self::Hole { sort, .. }
            | Self::Bound { sort, .. }
            | Self::Rigid { sort, .. }
            | Self::NormalForm { sort, .. } => *sort,
            Self::Runtime(_) => Sort::Runtime,
            Self::OptionalValue { .. }
            | Self::ErrorValue { .. }
            | Self::NominalValue { .. }
            | Self::TupleValue(_)
            | Self::ClosureContextValue(_)
            | Self::ListConcat { .. }
            | Self::RecordValue(_)
            | Self::Unit
            | Self::Bool(_)
            | Self::BitVec(_)
            | Self::Float16(_)
            | Self::BFloat16(_)
            | Self::Float32(_)
            | Self::Float64(_)
            | Self::Char(_)
            | Self::String(_)
            | Self::ClosureSnapshot { .. }
            | Self::Symbol(_)
            | Self::FunctionValue { .. }
            | Self::Float(_)
            | Self::Integer(_)
            | Self::Value { .. } => Sort::ComptimeValue,
            _ => Sort::Type,
        }
    }

    /// Complete structural context qualification without evaluating any
    /// normal-form head or changing capture order / runtime identities.
    pub fn contextual_closure(context: Term, signature: Term) -> Result<Term, TypeError> {
        if !matches!(
            signature,
            Term::Function { .. }
                | Term::ComptimeFunction(_)
                | Term::NormalFormFunction(..)
                | Term::UnsafeFunction(_)
        ) {
            return Err(TypeError::InvalidClosureSignature);
        }
        match context {
            Self::ClosureContextValue(environment) => {
                for ty in &environment {
                    require_sort(ty, Sort::Type)?;
                }
                if environment.is_empty() {
                    Ok(signature)
                } else {
                    Ok(Self::Closure {
                        environment,
                        signature: Box::new(signature),
                    })
                }
            }
            context @ (Self::Bound {
                sort: Sort::ComptimeValue,
                ..
            }
            | Self::Rigid {
                sort: Sort::ComptimeValue,
                ..
            }
            | Self::Hole {
                sort: Sort::ComptimeValue,
                ..
            }
            | Self::NormalForm {
                sort: Sort::ComptimeValue,
                ..
            }) => Ok(Self::ContextualClosure {
                context: Box::new(context),
                signature: Box::new(signature),
            }),
            _ => Err(TypeError::InvalidClosureContext),
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

    /// Replace slots of the surrounding frame without removing that frame.
    /// Used when staged inputs disappear but retained runtime slots still form
    /// a telescope. Nested binders and more distant frames keep their identity.
    pub fn rebind_scope(&self, arguments: &[Term]) -> Result<Term, TypeError> {
        self.rewrite(0, &mut |term, nesting| match term {
            Self::Bound { depth, index, sort } if *depth == nesting => {
                let value = arguments
                    .get(*index)
                    .ok_or(TypeError::MissingArgument(*index))?;
                require_sort(value, *sort)?;
                value.shift(nesting, 0)
            }
            _ => Ok(term.clone()),
        })
    }

    /// Rebind only demanded slots, preserving the surrounding frame. Missing
    /// entries are errors if referenced; no dummy type/value fills a pruned slot.
    pub fn rebind_scope_partial(&self, arguments: &[Option<Term>]) -> Result<Term, TypeError> {
        self.rewrite(0, &mut |term, nesting| match term {
            Self::Bound { depth, index, sort } if *depth == nesting => {
                let value = arguments
                    .get(*index)
                    .and_then(Option::as_ref)
                    .ok_or(TypeError::MissingArgument(*index))?;
                require_sort(value, *sort)?;
                value.shift(nesting, 0)
            }
            _ => Ok(term.clone()),
        })
    }

    /// Free declaration identities, including those under nested binders.
    /// Normal-form heads remain opaque; only their explicit arguments are read.
    pub fn rigid_parameters(
        &self,
    ) -> Result<std::collections::BTreeMap<DeclarationId, Sort>, TypeError> {
        let mut parameters = std::collections::BTreeMap::new();
        self.rewrite(0, &mut |term, _| {
            if let Self::Rigid { declaration, sort } = term {
                if let Some(previous) = parameters.insert(*declaration, *sort) {
                    if previous != *sort {
                        return Err(TypeError::WrongSort {
                            expected: previous,
                            actual: *sort,
                        });
                    }
                }
            }
            Ok(term.clone())
        })?;
        Ok(parameters)
    }

    /// Slots referenced in the outer frame, accounting for nested binders.
    pub fn substitution_parameters(
        &self,
    ) -> Result<std::collections::BTreeMap<usize, Sort>, TypeError> {
        let mut slots = std::collections::BTreeMap::new();
        self.rewrite(0, &mut |term, nesting| {
            if let Self::Bound { depth, index, sort } = term {
                if *depth == nesting {
                    if let Some(previous) = slots.insert(*index, *sort) {
                        if previous != *sort {
                            return Err(TypeError::WrongSort {
                                expected: previous,
                                actual: *sort,
                            });
                        }
                    }
                }
            }
            Ok(term.clone())
        })?;
        Ok(slots)
    }

    /// Substitute only demanded slots of a frame. Missing slots are allowed
    /// when the term does not refer to them; no arbitrary placeholder is used.
    pub fn substitute_partial(&self, arguments: &[Option<Term>]) -> Result<Term, TypeError> {
        self.rewrite(0, &mut |term, nesting| match term {
            Self::Bound { depth, index, sort } if *depth == nesting => {
                let value = arguments
                    .get(*index)
                    .and_then(Option::as_ref)
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

    /// Introduce one mixed-sort declaration frame outside this term's local
    /// binders. Existing outer references shift past the inserted frame.
    pub fn abstract_parameters(
        &self,
        parameters: &[DeclarationParameter],
    ) -> Result<Term, TypeError> {
        let indices = declaration_parameter_indices(parameters)?;
        self.shift(1, 0)?.rewrite(0, &mut |term, depth| match term {
            Self::Rigid { declaration, sort } => match indices.get(declaration) {
                Some(&(index, expected)) => {
                    if *sort != expected {
                        return Err(TypeError::WrongSort {
                            expected,
                            actual: *sort,
                        });
                    }
                    Ok(Self::Bound {
                        depth,
                        index,
                        sort: *sort,
                    })
                }
                None => Ok(term.clone()),
            },
            _ => Ok(term.clone()),
        })
    }

    /// Substitute a declared mixed-sort frame. Validate every supplied input,
    /// including unused ones, without asking the type solver to compare values.
    pub fn instantiate_parameters(
        &self,
        parameters: &[DeclarationParameter],
        arguments: &[Term],
    ) -> Result<Term, TypeError> {
        if parameters.len() != arguments.len() {
            return Err(TypeError::ArgumentCount {
                expected: parameters.len(),
                actual: arguments.len(),
            });
        }
        let indices = declaration_parameter_indices(parameters)?;
        for (parameter, argument) in parameters.iter().zip(arguments) {
            argument.check_scope(&[])?;
            require_sort(argument, parameter.sort)?;
        }
        self.rewrite(0, &mut |term, depth| match term {
            Self::Rigid { declaration, sort } => match indices.get(declaration) {
                Some(&(index, expected)) => {
                    if *sort != expected {
                        return Err(TypeError::WrongSort {
                            expected,
                            actual: *sort,
                        });
                    }
                    arguments[index].shift(depth, 0)
                }
                None => Ok(term.clone()),
            },
            _ => Ok(term.clone()),
        })
    }

    /// Internal substitution for a domain-admitted declaration instance. The
    /// caller must establish each replacement's domain before using this path.
    /// Occurrences still have to match the declaration's original sort.
    pub(crate) fn instantiate_admitted_parameters(
        &self,
        parameters: &[DeclarationParameter],
        arguments: &[Term],
    ) -> Result<Term, TypeError> {
        self.instantiate_domain_checked_parameters_in_scope(parameters, arguments, &[])
    }

    /// Structural callers establish the replacement sorts from parameter domains;
    /// completed-value callers additionally carry full domain-admission evidence.
    pub(crate) fn instantiate_domain_checked_parameters_in_scope(
        &self,
        parameters: &[DeclarationParameter],
        arguments: &[Term],
        frames: &[Vec<Sort>],
    ) -> Result<Term, TypeError> {
        if parameters.len() != arguments.len() {
            return Err(TypeError::ArgumentCount {
                expected: parameters.len(),
                actual: arguments.len(),
            });
        }
        let indices = declaration_parameter_indices(parameters)?;
        let result = self.rewrite(0, &mut |term, depth| match term {
            Self::Rigid { declaration, sort } => match indices.get(declaration) {
                Some(&(index, expected)) => {
                    if *sort != expected {
                        return Err(TypeError::WrongSort {
                            expected,
                            actual: *sort,
                        });
                    }
                    arguments[index].shift(depth, 0)
                }
                None => Ok(term.clone()),
            },
            _ => Ok(term.clone()),
        })?;
        result.check_scope(frames)?;
        Ok(result)
    }

    /// Bind selected definition-side Type parameters without unfolding heads.
    pub fn abstract_types(&self, parameters: &[DeclarationId]) -> Result<Term, TypeError> {
        let parameters: Vec<_> = parameters
            .iter()
            .map(|declaration| DeclarationParameter {
                declaration: *declaration,
                sort: Sort::Type,
            })
            .collect();
        self.abstract_parameters(&parameters)
    }
    /// Instantiate only the listed rigid Type parameters. Inputs are closed in
    /// the caller scope; unrelated rigid parameters and normal-form heads stay.
    pub fn instantiate_types(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
    ) -> Result<Term, TypeError> {
        let parameters: Vec<_> = parameters
            .iter()
            .map(|declaration| DeclarationParameter {
                declaration: *declaration,
                sort: Sort::Type,
            })
            .collect();
        self.instantiate_parameters(&parameters, arguments)
    }

    /// Substitute declaration Type inputs relative to an explicit caller scope.
    /// The ordinary closed-input API remains strict. Insert/remove a declaration
    /// frame so local binders and caller references retain distinct depths.
    pub fn instantiate_types_in_scope(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
        frames: &[Vec<Sort>],
    ) -> Result<Term, TypeError> {
        let parameters = parameters
            .iter()
            .map(|&declaration| DeclarationParameter {
                declaration,
                sort: Sort::Type,
            })
            .collect::<Vec<_>>();
        self.instantiate_parameters_in_scope(&parameters, arguments, frames)
    }

    /// Instantiate declaration inputs of declared sorts in a caller telescope.
    /// This is structural substitution, including value indices in opaque heads;
    /// it does not evaluate a head or discharge a program specification.
    pub fn instantiate_parameters_in_scope(
        &self,
        parameters: &[DeclarationParameter],
        arguments: &[Term],
        frames: &[Vec<Sort>],
    ) -> Result<Term, TypeError> {
        if parameters.len() != arguments.len() {
            return Err(TypeError::ArgumentCount {
                expected: parameters.len(),
                actual: arguments.len(),
            });
        }
        for (parameter, argument) in parameters.iter().zip(arguments) {
            argument.check_scope(frames)?;
            require_sort(argument, parameter.sort)?;
        }
        let result = self
            .abstract_parameters(parameters)?
            .substitute(arguments)?;
        result.check_scope(frames)?;
        Ok(result)
    }

    /// Instantiate declaration inputs of any semantic sort. This substitutes
    /// identities only; opaque heads remain opaque and replacement scope is checked.
    pub fn instantiate_rigids(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
    ) -> Result<Term, TypeError> {
        if parameters.len() != arguments.len() {
            return Err(TypeError::ArgumentCount {
                expected: parameters.len(),
                actual: arguments.len(),
            });
        }
        let parameters: Vec<_> = parameters
            .iter()
            .zip(arguments)
            .map(|(declaration, argument)| DeclarationParameter {
                declaration: *declaration,
                sort: argument.sort(),
            })
            .collect();
        self.instantiate_parameters(&parameters, arguments)
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
            Self::ErrorResult { errors, result } => {
                Self::error_result(many(errors, leaf)?, result.rewrite(nesting, leaf)?)
            }
            Self::Computation { effects, result } => Self::computation(
                effects
                    .iter()
                    .map(|operation| {
                        Ok(crate::effects::Operation {
                            declaration: operation.declaration,
                            arguments: many(&operation.arguments, leaf)?,
                        })
                    })
                    .collect::<Result<Vec<_>, TypeError>>()?,
                result.rewrite(nesting, leaf)?,
            ),
            Self::AssociatedType {
                origin,
                arguments,
                subject,
            } => Self::AssociatedType {
                origin: origin.clone(),
                arguments: many(arguments, leaf)?,
                subject: Box::new(subject.rewrite(nesting, leaf)?),
            },
            Self::Nominal {
                declaration,
                arguments,
            } => Self::Nominal {
                declaration: *declaration,
                arguments: many(arguments, leaf)?,
            },
            Self::ClosureSnapshot { factory, arguments } => Self::ClosureSnapshot {
                factory: *factory,
                arguments: many(arguments, leaf)?,
            },
            Self::FunctionValue {
                declaration,
                arguments,
            } => Self::FunctionValue {
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
            Self::ClosureContextValue(fields) => Self::ClosureContextValue(many(fields, leaf)?),
            Self::ContextualClosure { context, signature } => Self::contextual_closure(
                context.rewrite(nesting, leaf)?,
                signature.rewrite(nesting, leaf)?,
            )?,
            Self::Closure {
                environment,
                signature,
            } => Self::Closure {
                environment: many(environment, leaf)?,
                signature: Box::new(signature.rewrite(nesting, leaf)?),
            },
            Self::Tuple(items) => Self::Tuple(many(items, leaf)?),
            Self::Record(fields) => Self::Record(
                fields
                    .iter()
                    .map(|(name, ty)| Ok((name.clone(), ty.rewrite(nesting, leaf)?)))
                    .collect::<Result<_, TypeError>>()?,
            ),
            Self::OptionalValue { ty, value } => Self::OptionalValue {
                ty: Box::new(ty.rewrite(nesting, leaf)?),
                value: value
                    .as_ref()
                    .map(|value| value.rewrite(nesting, leaf).map(Box::new))
                    .transpose()?,
            },
            Self::ErrorValue { ty, label, value } => {
                let Self::ErrorResult { errors, result } = ty.as_ref() else {
                    return Err(TypeError::InvalidErrorValue);
                };
                Self::error_value(
                    many(errors, leaf)?,
                    result.rewrite(nesting, leaf)?,
                    label
                        .as_ref()
                        .map(|label| label.rewrite(nesting, leaf).map(Box::new))
                        .transpose()?,
                    value.rewrite(nesting, leaf)?,
                )?
            }
            Self::NominalValue {
                ty,
                variant,
                fields,
            } => Self::NominalValue {
                ty: Box::new(ty.rewrite(nesting, leaf)?),
                variant: *variant,
                fields: many(fields, leaf)?,
            },
            Self::RecordValue(fields) => Self::RecordValue(
                fields
                    .iter()
                    .map(|(name, ty)| Ok((name.clone(), ty.rewrite(nesting, leaf)?)))
                    .collect::<Result<_, TypeError>>()?,
            ),
            Self::TupleValue(items) => Self::TupleValue(many(items, leaf)?),
            Self::ListConcat { ty, parts } => {
                Self::list_concat(ty.rewrite(nesting, leaf)?, many(parts, leaf)?)?
            }
            Self::ComptimeFunction(inner) => {
                Self::ComptimeFunction(Box::new(inner.rewrite(nesting, leaf)?))
            }
            Self::NormalFormFunction(inner, variadic) => {
                Self::NormalFormFunction(Box::new(inner.rewrite(nesting, leaf)?), *variadic)
            }
            Self::UnsafeFunction(inner) => {
                Self::UnsafeFunction(Box::new(inner.rewrite(nesting, leaf)?))
            }
            Self::Optional(inner) => Self::Optional(Box::new(inner.rewrite(nesting, leaf)?)),
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

#[derive(Debug, PartialEq, Eq)]
pub enum CallError<E> {
    NotFunction,
    Shape(TypeError),
    Contract { parameter: usize, error: E },
}

impl<E: fmt::Display> fmt::Display for CallError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFunction => f.write_str("expected a function signature"),
            Self::Shape(error) => error.fmt(f),
            Self::Contract { parameter, error } => write!(f, "parameter {parameter}: {error}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for CallError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotFunction => None,
            Self::Shape(error) => Some(error),
            Self::Contract { error, .. } => Some(error),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeError {
    ContextualCodeNotCallable,
    WrongSort { expected: Sort, actual: Sort },
    MissingArgument(usize),
    ArgumentCount { expected: usize, actual: usize },
    InvalidHole(usize),
    InvalidBound { depth: u32, index: usize },
    RuntimeInCore(RuntimeId),
    DuplicateRuntime(RuntimeId),
    DuplicateDeclarationParameter(DeclarationId),
    UnboundRuntime(RuntimeId),
    BinderDepthOverflow,
    InvalidClosureSignature,
    InvalidClosureContext,
    DuplicateRecordField(String),
    InvalidNominalValueType,
    InvalidOptionalValue,
    InvalidErrorValue,
    InvalidComptimeFunctionSignature,
    InvalidNormalFormFunctionSignature,
    InvalidUnsafeFunctionSignature,
    InvalidFunctionSignature,
    InvalidListComponent,
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContextualCodeNotCallable => f.write_str("contextual code signature requires code-method elaboration, not ordinary callable erasure"),
            Self::InvalidClosureContext => f.write_str("closure qualification requires a closure context"),
            Self::InvalidListComponent => {
                f.write_str("list concatenation requires list operands")
            }
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
            Self::DuplicateRuntime(id) => write!(f, "duplicate runtime parameter {id}"),
            Self::DuplicateDeclarationParameter(id) => {
                write!(f, "duplicate declaration parameter {id}")
            }
            Self::UnboundRuntime(id) => write!(
                f,
                "runtime identity {id} is unavailable in this environment"
            ),
            Self::DuplicateRecordField(name) => write!(f, "duplicate record field {name:?}"),
            Self::InvalidOptionalValue => f.write_str("invalid frozen optional value"),
            Self::InvalidErrorValue => f.write_str("invalid frozen error result"),
            Self::InvalidNominalValueType => {
                f.write_str("frozen aggregate requires a nominal type, or an untagged normal-form type")
            }
            Self::BinderDepthOverflow => f.write_str("binder depth overflow"),
            Self::InvalidFunctionSignature => f.write_str("expected a function signature"),
            Self::InvalidUnsafeFunctionSignature => write!(f, "unsafe function requires an ordinary or comptime function signature"),
            Self::InvalidNormalFormFunctionSignature => {
                f.write_str("normal-form callable requires comptime inputs and at most one final collector")
            }
            Self::InvalidComptimeFunctionSignature => {
                f.write_str("comptime callable signature must be a function type")
            }
            Self::InvalidClosureSignature => {
                f.write_str("closure signature must be a function type")
            }
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
    /// Abstract explicitly ordered runtime dependencies out of a type and its
    /// parameter contracts. The caller supplies checked dependency types; this
    /// operation neither infers contracts nor reads runtime object contents.
    /// Repeated occurrences of one identity share one hole. No core is interned
    /// until all contracts and the body pass structural validation.
    pub fn abstract_runtime(
        &mut self,
        body: &Term,
        dependencies: &[(RuntimeId, Term)],
    ) -> Result<TypeRef, TypeError> {
        let mut positions = std::collections::BTreeMap::new();
        for (index, (id, _)) in dependencies.iter().enumerate() {
            if positions.insert(*id, index).is_some() {
                return Err(TypeError::DuplicateRuntime(*id));
            }
        }
        let mut abstract_identity = |term: &Term, _: u32| {
            if let Term::Runtime(id) = term {
                let index = positions.get(id).ok_or(TypeError::RuntimeInCore(*id))?;
                Ok(Term::Hole {
                    index: *index,
                    sort: Sort::Runtime,
                })
            } else if let Term::Hole { index, .. } = term {
                // Existing holes belong to a different telescope. Accepting
                // them here could silently capture them in the new core.
                Err(TypeError::InvalidHole(*index))
            } else {
                Ok(term.clone())
            }
        };
        let holes = dependencies
            .iter()
            .map(|(_, ty)| {
                Ok(Parameter {
                    sort: Sort::Runtime,
                    ty: ty.rewrite(0, &mut abstract_identity)?,
                })
            })
            .collect::<Result<Vec<_>, TypeError>>()?;
        let body = body.rewrite(0, &mut abstract_identity)?;
        let core = self.intern(holes, body)?;
        TypeRef::bind(
            core,
            dependencies
                .iter()
                .map(|(id, _)| Term::Runtime(*id))
                .collect(),
        )
    }

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
        // The entire substitution is part of the reference's environment,
        // including entries used only by a parameter contract. Inspecting only
        // the materialized body loses these dependencies.
        for argument in &self.arguments {
            collect_runtime_dependencies(argument, &mut result);
        }
        Ok(result)
    }

    /// Check open bound references at a use site. Frames are ordered outermost
    /// to innermost; each contains the semantic sorts of its available binders.
    /// This is shape/scope validation, not dependent contract checking.
    pub fn check_scope(&self, frames: &[Vec<Sort>]) -> Result<(), TypeError> {
        let mut frames = frames.to_vec();
        for argument in &self.arguments {
            validate(argument, &[], &mut frames, false, false)?;
        }
        Ok(())
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

fn collect_runtime_dependencies(term: &Term, result: &mut BTreeSet<RuntimeId>) {
    match term {
        Term::ErrorResult {
            errors,
            result: output,
        } => {
            for error in errors {
                collect_runtime_dependencies(error, result);
            }
            collect_runtime_dependencies(output, result);
        }
        Term::Computation {
            effects,
            result: output,
        } => {
            for operation in effects {
                for argument in &operation.arguments {
                    collect_runtime_dependencies(argument, result);
                }
            }
            collect_runtime_dependencies(output, result);
        }
        Term::AssociatedType {
            arguments, subject, ..
        } => {
            collect_runtime_dependencies(subject, result);
            for argument in arguments {
                collect_runtime_dependencies(argument, result);
            }
        }
        Term::Runtime(id) => {
            result.insert(*id);
        }
        Term::Nominal { arguments, .. }
        | Term::ClosureSnapshot { arguments, .. }
        | Term::FunctionValue { arguments, .. }
        | Term::NormalForm { arguments, .. }
        | Term::Tuple(arguments)
        | Term::ClosureContextValue(arguments)
        | Term::TupleValue(arguments) => {
            for argument in arguments {
                collect_runtime_dependencies(argument, result);
            }
        }
        Term::OptionalValue { ty, value } => {
            collect_runtime_dependencies(ty, result);
            if let Some(value) = value {
                collect_runtime_dependencies(value, result);
            }
        }
        Term::ErrorValue { ty, label, value } => {
            collect_runtime_dependencies(ty, result);
            if let Some(label) = label {
                collect_runtime_dependencies(label, result);
            }
            collect_runtime_dependencies(value, result);
        }
        Term::ListConcat { ty, parts: fields } | Term::NominalValue { ty, fields, .. } => {
            collect_runtime_dependencies(ty, result);
            for field in fields {
                collect_runtime_dependencies(field, result);
            }
        }
        Term::Record(fields) | Term::RecordValue(fields) => {
            for (_, ty) in fields {
                collect_runtime_dependencies(ty, result);
            }
        }
        Term::ContextualClosure { context, signature } => {
            collect_runtime_dependencies(context, result);
            collect_runtime_dependencies(signature, result);
        }
        Term::Closure {
            environment,
            signature,
        } => {
            for ty in environment {
                collect_runtime_dependencies(ty, result);
            }
            collect_runtime_dependencies(signature, result);
        }
        Term::Optional(inner)
        | Term::Pointer(inner)
        | Term::ComptimeFunction(inner)
        | Term::NormalFormFunction(inner, _)
        | Term::UnsafeFunction(inner) => collect_runtime_dependencies(inner, result),
        Term::Function {
            parameters,
            result: output,
        } => {
            for parameter in parameters {
                collect_runtime_dependencies(&parameter.ty, result);
            }
            collect_runtime_dependencies(output, result);
        }
        Term::ClosureContextType
        | Term::Primitive(_)
        | Term::Hole { .. }
        | Term::Bound { .. }
        | Term::Rigid { .. }
        | Term::Unit
        | Term::Bool(_)
        | Term::Float(_)
        | Term::Integer(_)
        | Term::BitVec(_)
        | Term::Float16(_)
        | Term::BFloat16(_)
        | Term::Float32(_)
        | Term::Float64(_)
        | Term::Char(_)
        | Term::String(_)
        | Term::Symbol(_)
        | Term::Value { .. } => {}
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
        Term::ErrorResult { errors, result } => {
            for ty in errors.iter().chain(std::iter::once(result.as_ref())) {
                require_sort(ty, Sort::Type)?;
                validate(ty, holes, frames, allow_free, core)?;
            }
        }
        Term::Computation { effects, result } => {
            require_sort(result, Sort::Type)?;
            validate(result, holes, frames, allow_free, core)?;
            for operation in effects {
                for argument in &operation.arguments {
                    validate(argument, holes, frames, allow_free, core)?;
                }
            }
        }
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
        Term::UnsafeFunction(signature) => {
            if !matches!(
                signature.as_ref(),
                Term::Function { .. } | Term::ComptimeFunction(_) | Term::NormalFormFunction(..)
            ) {
                return Err(TypeError::InvalidUnsafeFunctionSignature);
            }
            validate(signature, holes, frames, allow_free, core)?;
        }
        Term::NormalFormFunction(signature, variadic) => {
            let Term::Function { parameters, .. } = signature.as_ref() else {
                return Err(TypeError::InvalidNormalFormFunctionSignature);
            };
            if parameters.iter().any(|p| p.sort == Sort::Runtime)
                || variadic.is_some_and(|index| index.checked_add(1) != Some(parameters.len()))
            {
                return Err(TypeError::InvalidNormalFormFunctionSignature);
            }
            validate(signature, holes, frames, allow_free, core)?;
        }
        Term::ComptimeFunction(signature) => {
            if !matches!(signature.as_ref(), Term::Function { .. }) {
                return Err(TypeError::InvalidComptimeFunctionSignature);
            }
            validate(signature, holes, frames, allow_free, core)?;
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
        Term::ClosureContextValue(fields) => {
            for ty in fields {
                require_sort(ty, Sort::Type)?;
                validate(ty, holes, frames, allow_free, core)?;
            }
        }
        Term::ContextualClosure { context, signature } => {
            if !matches!(
                signature.as_ref(),
                Term::Function { .. }
                    | Term::ComptimeFunction(_)
                    | Term::NormalFormFunction(..)
                    | Term::UnsafeFunction(_)
            ) {
                return Err(TypeError::InvalidClosureSignature);
            }
            if !matches!(
                context.as_ref(),
                Term::ClosureContextValue(_)
                    | Term::Bound {
                        sort: Sort::ComptimeValue,
                        ..
                    }
                    | Term::Rigid {
                        sort: Sort::ComptimeValue,
                        ..
                    }
                    | Term::Hole {
                        sort: Sort::ComptimeValue,
                        ..
                    }
                    | Term::NormalForm {
                        sort: Sort::ComptimeValue,
                        ..
                    }
            ) {
                return Err(TypeError::InvalidClosureContext);
            }
            validate(context, holes, frames, allow_free, core)?;
            validate(signature, holes, frames, allow_free, core)?;
        }
        Term::Closure {
            environment,
            signature,
        } => {
            if !matches!(
                signature.as_ref(),
                Term::Function { .. }
                    | Term::ComptimeFunction(_)
                    | Term::NormalFormFunction(..)
                    | Term::UnsafeFunction(_)
            ) {
                return Err(TypeError::InvalidClosureSignature);
            }
            for ty in environment {
                require_sort(ty, Sort::Type)?;
                validate(ty, holes, frames, allow_free, core)?;
            }
            require_sort(signature, Sort::Type)?;
            validate(signature, holes, frames, allow_free, core)?;
        }
        Term::Record(fields) => {
            let mut names = BTreeSet::new();
            for (name, ty) in fields {
                if !names.insert(name) {
                    return Err(TypeError::DuplicateRecordField(name.clone()));
                }
                require_sort(ty, Sort::Type)?;
                validate(ty, holes, frames, allow_free, core)?;
            }
        }
        Term::Tuple(items) => {
            for item in items {
                require_sort(item, Sort::Type)?;
                validate(item, holes, frames, allow_free, core)?;
            }
        }
        Term::ListConcat { ty, parts } => {
            ty.list_element_domain()
                .ok_or(TypeError::InvalidListComponent)?;
            validate(ty, holes, frames, allow_free, core)?;
            for part in parts {
                if !part.is_list_component(ty) {
                    return Err(TypeError::InvalidListComponent);
                }
                validate(part, holes, frames, allow_free, core)?;
            }
        }
        Term::Optional(inner) | Term::Pointer(inner) => {
            require_sort(inner, Sort::Type)?;
            validate(inner, holes, frames, allow_free, core)?;
        }
        Term::OptionalValue { ty, value } => {
            if !matches!(ty.as_ref(), Term::Optional(_)) {
                return Err(TypeError::InvalidOptionalValue);
            }
            validate(ty, holes, frames, allow_free, core)?;
            if let Some(value) = value {
                if value.sort() == Sort::Runtime {
                    return Err(TypeError::InvalidOptionalValue);
                }
                validate(value, holes, frames, allow_free, core)?;
            }
        }
        Term::ErrorValue { ty, label, value } => {
            require_sort(ty, Sort::Type)?;
            let Term::ErrorResult { errors, .. } = ty.as_ref() else {
                return Err(TypeError::InvalidErrorValue);
            };
            if errors.is_empty() || label.as_ref().is_some_and(|label| !errors.contains(label)) {
                return Err(TypeError::InvalidErrorValue);
            }
            validate(ty, holes, frames, allow_free, core)?;
            if let Some(label) = label {
                require_sort(label, Sort::Type)?;
                validate(label, holes, frames, allow_free, core)?;
            }
            if value.sort() == Sort::Runtime {
                return Err(TypeError::InvalidErrorValue);
            }
            validate(value, holes, frames, allow_free, core)?;
        }
        Term::NominalValue {
            ty,
            variant,
            fields,
        } => {
            require_sort(ty, Sort::Type)?;
            if !matches!(ty.as_ref(), Term::Nominal { .. })
                && !(variant.is_none()
                    && matches!(
                        ty.as_ref(),
                        Term::Primitive(Primitive::Object)
                            | Term::NormalForm {
                                sort: Sort::Type,
                                ..
                            }
                    ))
            {
                return Err(TypeError::InvalidNominalValueType);
            }
            validate(ty, holes, frames, allow_free, core)?;
            if **ty == Term::Primitive(Primitive::Object)
                && fields
                    .iter()
                    .any(|field| crate::objects::entry_parts(field).is_none())
            {
                return Err(TypeError::InvalidNominalValueType);
            }
            for field in fields {
                if field.sort() == Sort::Runtime {
                    return Err(TypeError::WrongSort {
                        expected: Sort::ComptimeValue,
                        actual: Sort::Runtime,
                    });
                }
                validate(field, holes, frames, allow_free, core)?;
            }
        }
        Term::RecordValue(fields) => {
            let mut names = BTreeSet::new();
            for (name, value) in fields {
                if !names.insert(name) {
                    return Err(TypeError::DuplicateRecordField(name.clone()));
                }
                if value.sort() == Sort::Runtime {
                    return Err(TypeError::WrongSort {
                        expected: Sort::ComptimeValue,
                        actual: Sort::Runtime,
                    });
                }
                validate(value, holes, frames, allow_free, core)?;
            }
        }
        Term::TupleValue(items) => {
            for item in items {
                if item.sort() == Sort::Runtime {
                    return Err(TypeError::WrongSort {
                        expected: Sort::ComptimeValue,
                        actual: Sort::Runtime,
                    });
                }
                validate(item, holes, frames, allow_free, core)?;
            }
        }
        Term::AssociatedType {
            arguments, subject, ..
        } => {
            require_sort(subject, Sort::Type)?;
            validate(subject, holes, frames, allow_free, core)?;
            for argument in arguments {
                validate(argument, holes, frames, allow_free, core)?;
            }
        }
        Term::ClosureSnapshot { arguments, .. } => {
            for argument in arguments {
                require_sort(argument, Sort::Type)?;
                validate(argument, holes, frames, allow_free, core)?;
            }
        }
        Term::Nominal { arguments, .. }
        | Term::FunctionValue { arguments, .. }
        | Term::NormalForm { arguments, .. } => {
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
            Self::BitVec(width) => return write!(f, "(bitvec-type (width {width}))"),
            Self::Type => "Type",
            Self::Object => "Object",
            Self::Bool => "bool",
            Self::Char => "char",
            Self::Str => "str",
            Self::Symbol => "Symbol",
            Self::Integer => "Integer",
            Self::Float => "Float",
            Self::Unit => "unit",
            Self::Void => "void",
            Self::Never => "never",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::I128 => "i128",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::U128 => "u128",
            Self::Isize => "isize",
            Self::Usize => "usize",
            Self::F16 => "f16",
            Self::Bf16 => "bf16",
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
            Self::ErrorResult { errors, result } => {
                f.write_str("(error-result-type (errors")?;
                for error in errors {
                    write!(f, " {error}")?;
                }
                write!(f, ") (result {result}))")
            }
            Self::Computation { effects, result } => {
                f.write_str("(computation-type (effects")?;
                for operation in effects {
                    write!(f, " {operation}")?;
                }
                write!(f, ") (result {result}))")
            }
            Self::Primitive(p) => write!(f, "(primitive {p})"),
            Self::AssociatedType {
                origin,
                arguments,
                subject,
            } => {
                write!(
                    f,
                    "(associated-type (interface {}) (arguments ",
                    origin.interface
                )?;
                list(f, arguments)?;
                write!(f, ") (subject {subject}) (member {}))", origin.member)
            }
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
            Self::ClosureContextType => f.write_str("(closure-context-type)"),
            Self::ClosureContextValue(fields) => {
                f.write_str("(closure-context (fields ")?;
                list(f, fields)?;
                f.write_str("))")
            }
            Self::ContextualClosure { context, signature } => write!(
                f,
                "(contextual-closure (context {context}) (signature {signature}))"
            ),
            Self::Closure {
                environment,
                signature,
            } => {
                f.write_str("(closure (environment")?;
                for ty in environment {
                    write!(f, " {ty}")?;
                }
                write!(f, ") (signature {signature}))")
            }
            Self::Record(fields) => {
                f.write_str("(record-type (fields")?;
                for (name, ty) in fields {
                    f.write_str(" (field ")?;
                    crate::display::string(f, name)?;
                    write!(f, " {ty})")?;
                }
                f.write_str("))")
            }
            Self::OptionalValue { ty, value } => {
                write!(f, "(optional-value (type {ty}) ")?;
                match value {
                    Some(value) => write!(f, "(present {value}))"),
                    None => f.write_str("(absent))"),
                }
            }
            Self::ErrorValue { ty, label, value } => {
                write!(f, "(error-value (type {ty}) (branch ")?;
                match label {
                    Some(label) => write!(f, "(error {label})")?,
                    None if self.has_pending_error_branch() => f.write_str("pending-inner")?,
                    None => f.write_str("success")?,
                }
                write!(f, ") (payload {value}))")
            }
            Self::NominalValue {
                ty,
                variant,
                fields,
            } => {
                if **ty == Term::Primitive(Primitive::Object)
                    && variant.is_none()
                    && fields
                        .iter()
                        .all(|entry| crate::objects::entry_parts(entry).is_some())
                {
                    f.write_str("(object-value")?;
                    for entry in fields {
                        let (key, domain, value) = crate::objects::entry_parts(entry).unwrap();
                        f.write_str(" (entry (key ")?;
                        match key {
                            Some(key) => crate::display::string(f, key)?,
                            None => f.write_str("none")?,
                        }
                        write!(f, ") (type {domain}) (value {value}))")?;
                    }
                    return f.write_str(")");
                }
                write!(f, "(nominal-value (type {ty}) (variant ")?;
                match variant {
                    Some(index) => write!(f, "{index}")?,
                    None => f.write_str("none")?,
                }
                f.write_str(") (fields ")?;
                list(f, fields)?;
                f.write_str("))")
            }
            Self::RecordValue(fields) => {
                f.write_str("(record-value (fields")?;
                for (name, ty) in fields {
                    f.write_str(" (field ")?;
                    crate::display::string(f, name)?;
                    write!(f, " {ty})")?;
                }
                f.write_str("))")
            }
            Self::Tuple(items) => {
                f.write_str("(tuple ")?;
                list(f, items)?;
                f.write_str(")")
            }
            Self::ListConcat { ty, parts } => {
                write!(f, "(list-concat (type {ty}) (parts ")?;
                list(f, parts)?;
                f.write_str("))")
            }
            Self::TupleValue(items) => {
                f.write_str("(tuple-value ")?;
                list(f, items)?;
                f.write_str(")")
            }
            Self::ComptimeFunction(inner) => write!(f, "(comptime-function {inner})"),
            Self::NormalFormFunction(inner, variadic) => {
                f.write_str("(normal-form-function (collector ")?;
                match variadic {
                    Some(index) => write!(f, "{index}")?,
                    None => f.write_str("none")?,
                }
                write!(f, ") (signature {inner}))")
            }
            Self::UnsafeFunction(inner) => write!(f, "(unsafe-function {inner})"),
            Self::Optional(inner) => write!(f, "(optional {inner})"),
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
            Self::Unit => f.write_str("(unit-value)"),
            Self::BitVec(value) => write!(f, "{value}"),
            Self::Float16(bits) => write!(f, "(float16-bits 0x{bits:04x})"),
            Self::BFloat16(bits) => write!(f, "(bfloat16-bits 0x{bits:04x})"),
            Self::Float32(bits) => write!(f, "(float32-bits 0x{bits:08x})"),
            Self::Float64(bits) => write!(f, "(float64-bits 0x{bits:016x})"),
            Self::Integer(value) => write!(f, "{value}"),
            Self::Float(value) => write!(f, "(comptime-float {value})"),
            Self::Char(value) => write!(f, "(char {})", *value as u32),
            Self::ClosureSnapshot { factory, arguments } => {
                write!(f, "(closure-snapshot (factory {factory}) (arguments")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str("))")
            }
            Self::FunctionValue {
                declaration,
                arguments,
            } => {
                write!(f, "(function-value {declaration} (arguments")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str("))")
            }
            Self::String(value) | Self::Symbol(value) => {
                f.write_str(if matches!(self, Self::Symbol(_)) {
                    "(symbol "
                } else {
                    "(string "
                })?;
                crate::display::string(f, value)?;
                f.write_str(")")
            }
            Self::Value { store, index } => write!(f, "(value (store {store}) (index {index}))"),
        }
    }
}

impl fmt::Display for AssociatedTypeOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-origin (interface {}) (member {}))",
            self.interface, self.member
        )
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
