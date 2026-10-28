//! Concrete execution of the supported NIR value/control-flow fragment.
//!
//! The caller must authorize phase and settle semantic/resource obligations.
//! This engine is not a stage checker and is not automatically invoked by the
//! frontend. `for_comptime` additionally checks the supported function-phase
//! boundary; it does not replace resource verification. No external
//! IO or normal-form unfolding are implemented. Direct handlers execute through
//! NIR and validated IIR occurrence environments; scheduled effect cleanup is
//! not yet admitted. Limits are resource outcomes, not language type errors.
mod admission;
mod cleanup;
mod dynamic_instances;
mod environment;
mod field_access;
mod iir_execution;
mod invocation_contracts;
mod list_index;
mod list_operations;
mod memory;
mod normal_inputs;
mod object_edits;
mod object_queries;
#[cfg(test)]
mod object_values;
mod objects;
pub use objects::ObjectProperty;
mod generated_enums;
pub use generated_enums::EnumPublicationError;
mod generated_structs;
pub use generated_structs::{
    DefinitionAdmissionSession, PreparedEnumAdmission, PreparedStructAdmission,
    StructAdmissionSession, StructPublicationError,
};
mod definition_diagnostics;
pub use definition_diagnostics::DefinitionBuildError;
mod enum_build;
pub use enum_build::{EnumBuildRequest, PreparedEnumBuild};
mod struct_build;
pub use struct_build::{PreparedStructBuild, StructBuildRequest};
mod reflection;
pub use reflection::{
    CallableReflection, EnumReflection, ReflectedField, ReflectedVariant, StructReflection,
};
#[cfg(test)]
mod scoped_work;
mod selected_types;
mod type_list_index;
pub use comptime_runtime::object::{Entry as ObjectEntry, Object};
pub use memory::{OwnedGraphCheckpoint, PointerValue, Storage};

use nir::program::InterfaceInstance;
use nir::program::{Expr, ExprKind as K, Literal, LocalId, Package, SourceRange};
use nir::{DeclarationId, Primitive, Term};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use environment::Environment;
// Invocation-local contextual interpretation choices; no active state is stored
// in the owned NIR or reused across calls.
enum PreparedNormalInput {
    Value(Value),
    TypeTuple(Option<DeclarationId>),
    ListType(Term),
}
/// Invocation-only input transport. Live graphs retain their storage identity;
/// normal terms are reconstructed in the invocation's checked environment.
/// This transport is neither a type identity nor a serializable IR node.
pub enum InvocationValue {
    Normal(Term),
    Live(Value),
}

/// Check completion and scope of an already domain-admitted immutable input.
/// Type and value inputs retain their semantic sort. This checks representation,
/// not the declaration's parameter domains or permission to invoke a callable.
pub fn validate_snapshot_term(
    term: &Term,
    budget: &mut comptime_runtime::Budget,
    parameters: &BTreeSet<DeclarationId>,
) -> Result<(), ErrorKind> {
    nir::type_computations::charge_term(term, &mut budget.steps)
        .map_err(|_| ErrorKind::StepsExhausted)?;
    term.check_scope(&[]).map_err(|_| ErrorKind::Type)?;
    let work =
        scoped_term_value_work_with_layouts(term, &BTreeMap::new(), parameters, |_, _, _| None)
            .ok_or(ErrorKind::Type)?;
    budget.consume(work).map_err(|_| ErrorKind::StepsExhausted)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// One optional layer. An absent outer value is distinct from a present absent payload.
    Optional {
        ty: Term,
        value: Option<Box<Value>>,
    },
    /// Tagged result data, with no continuation. None is the success branch.
    ErrorResult {
        ty: Term,
        error: Option<usize>,
        value: Box<Value>,
    },
    Pointer(PointerValue),
    /// Interpreter-internal addressable local storage; never a source value.
    Storage(Storage),
    /// Type description; abstract constituents require an authorized comptime scope.
    Type(Term),
    /// Opaque declaration Type input, admitted only by a comptime scope.
    TypeParameter(DeclarationId),
    /// Invocation-scoped opaque data, never a completed runtime value.
    AbstractValue {
        declaration: DeclarationId,
        ty: Term,
    },
    /// Immutable meta.List<Type>; distinct from a tuple of Type values.
    /// Abstract elements require the same scope as individual Type descriptions.
    TypeList(Vec<Term>),
    /// Immutable homogeneous comptime data with its original list type identity.
    List {
        ty: Term,
        values: Vec<Value>,
    },
    Integer(comptime_runtime::Integer),
    BitVec(comptime_runtime::bitvec::BitVec),
    Float(comptime_runtime::decimal::DecimalValue),
    Float32(u32),
    Float64(u64),
    SignedInteger(i128),
    UnsignedInteger(u128),
    Unit,
    Variant {
        arguments: Vec<Term>,
        declaration: DeclarationId,
        index: usize,
        fields: Vec<Value>,
    },
    Bool(bool),
    Char(char),
    String(Arc<str>),
    Symbol(Arc<str>),
    Tuple(Vec<Value>),
    /// A named structural product; labels survive host calls and storage paths.
    AnonymousRecord(Vec<(String, Value)>),
    /// Invocation-local ordered metadata, distinct from an anonymous record.
    /// No normal-term encoding or runtime representation is admitted yet.
    Object(Object<ObjectValue>),
    Record {
        arguments: Vec<Term>,
        declaration: DeclarationId,
        fields: Vec<Value>,
    },
    Newtype {
        ty: Term,
        value: Box<Value>,
    },
    Function(DeclarationId),
    /// A runtime declaration transported at comptime, without an executable body.
    RuntimeFunction {
        declaration: DeclarationId,
        arguments: Vec<Term>,
    },
    RuntimeTraitMethod(Box<TraitMethodIdentity>),
    Closure(Arc<Closure>),
}
/// Preserve the original entry domain, including opaque normal-form identity.
/// Constructing this host representation does not establish its validity.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectValue {
    pub ty: Term,
    pub value: Value,
}
impl fmt::Display for ObjectValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(object-value (type {}) (value {}))",
            self.ty, self.value
        )
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct TraitMethodIdentity {
    pub interface: InterfaceInstance,
    pub subject: Term,
    pub member: DeclarationId,
    pub method_arguments: Vec<Term>,
}
impl fmt::Display for TraitMethodIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(runtime-trait-method {} (subject {}) (member {}) (method-arguments",
            self.interface, self.subject, self.member
        )?;
        for argument in &self.method_arguments {
            write!(f, " {argument}")?;
        }
        f.write_str("))")
    }
}
impl Value {
    /// Inspect the owned invocation graph without copying storage or executing
    /// callbacks. Shared closures and storage are visited once. Weak pointers
    /// are leaves: inspecting a value does not dereference borrowed memory.
    pub fn visit_owned_graph(
        &self,
        fuel: &mut u64,
        visitor: &mut impl FnMut(&Value, &mut u64) -> Result<(), ErrorKind>,
    ) -> Result<(), ErrorKind> {
        fn visit(
            value: &Value,
            fuel: &mut u64,
            depth: usize,
            closures: &mut BTreeSet<usize>,
            storage: &mut BTreeSet<usize>,
            visitor: &mut impl FnMut(&Value, &mut u64) -> Result<(), ErrorKind>,
        ) -> Result<(), ErrorKind> {
            *fuel = fuel.checked_sub(1).ok_or(ErrorKind::StepsExhausted)?;
            if depth >= 128 {
                return Err(ErrorKind::DepthExhausted);
            }
            match value {
                Value::Closure(closure) if !closures.insert(Arc::as_ptr(closure) as usize) => {
                    return Ok(());
                }
                Value::Storage(cell) if !storage.insert(Arc::as_ptr(&cell.0) as usize) => {
                    return Ok(());
                }
                _ => {}
            }
            visitor(value, fuel)?;
            match value {
                Value::Storage(cell) => {
                    let payload = cell.0.try_lock().map_err(|_| ErrorKind::InvalidPointer)?;
                    visit(&payload, fuel, depth + 1, closures, storage, visitor)?;
                }
                Value::Closure(closure) => {
                    for child in closure.environment.values() {
                        visit(child, fuel, depth + 1, closures, storage, visitor)?;
                    }
                }
                Value::List { values: fields, .. }
                | Value::Tuple(fields)
                | Value::Record { fields, .. }
                | Value::Variant { fields, .. } => {
                    for child in fields {
                        visit(child, fuel, depth + 1, closures, storage, visitor)?;
                    }
                }
                Value::AnonymousRecord(fields) => {
                    for (_, child) in fields {
                        visit(child, fuel, depth + 1, closures, storage, visitor)?;
                    }
                }
                Value::Object(object) => {
                    for entry in object.entries() {
                        visit(
                            &entry.value().value,
                            fuel,
                            depth + 1,
                            closures,
                            storage,
                            visitor,
                        )?;
                    }
                }
                Value::Optional { value, .. } => {
                    if let Some(value) = value {
                        visit(value, fuel, depth + 1, closures, storage, visitor)?;
                    }
                }
                Value::ErrorResult { value, .. } | Value::Newtype { value, .. } => {
                    visit(value, fuel, depth + 1, closures, storage, visitor)?
                }
                _ => {}
            }
            Ok(())
        }
        visit(
            self,
            fuel,
            0,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
            visitor,
        )
    }

    /// Bound copying of immutable data supplied by the host before cloning it.
    /// This checks representation only; nominal formation and argument types
    /// still belong to the invocation checker. It does not freeze storage.
    pub fn immutable_copy_work(&self) -> Option<usize> {
        self.scoped_copy_work(&BTreeSet::new())
    }
    fn scoped_copy_work(&self, parameters: &BTreeSet<DeclarationId>) -> Option<usize> {
        self.scoped_copy_work_with_evidence(parameters, &BTreeMap::new())
    }
    fn scoped_copy_work_with_evidence(
        &self,
        parameters: &BTreeSet<DeclarationId>,
        value_parameters: &BTreeMap<DeclarationId, Term>,
    ) -> Option<usize> {
        // Keep only the bounded traversal depth on the call stack. The old
        // worklist allocated one entry per sibling on every immutable scan.
        // Count pending siblings separately to preserve its admission bound
        // and visit in the same reverse-child order without copying references.
        fn visit(
            value: &Value,
            depth: usize,
            work: &mut usize,
            pending: &mut usize,
            parameters: &BTreeSet<DeclarationId>,
            value_parameters: &BTreeMap<DeclarationId, Term>,
        ) -> Option<()> {
            *pending = pending.checked_sub(1)?;
            if depth >= 128 {
                return None;
            }
            *work = work.checked_add(1)?;
            let extra = match value {
                Value::Type(ty)
                | Value::List { ty, .. }
                | Value::Optional { ty, .. }
                | Value::ErrorResult { ty, .. }
                | Value::Newtype { ty, .. }
                | Value::AbstractValue { ty, .. } => scoped_type_value_work_with_evidence(
                    ty,
                    &BTreeMap::new(),
                    parameters,
                    value_parameters,
                    |_, _, _| None,
                )?,
                Value::TypeList(types) => {
                    if types.len() > 100_000 {
                        return None;
                    }
                    types.iter().try_fold(types.len(), |n, ty| {
                        let total = n.checked_add(scoped_type_value_work_with_evidence(
                            ty,
                            &BTreeMap::new(),
                            parameters,
                            value_parameters,
                            |_, _, _| None,
                        )?)?;
                        (total <= 100_000).then_some(total)
                    })?
                }
                Value::Float(value) => value
                    .digits()
                    .len()
                    .saturating_add(value.exponent().bit_len().div_ceil(32)),
                Value::Integer(value) => value.bit_len().div_ceil(32),
                Value::BitVec(value) => value.words().len(),
                Value::String(value) | Value::Symbol(value) => value.len(),
                Value::Record { arguments, .. } | Value::Variant { arguments, .. } => {
                    if arguments.len() > 100_000 {
                        return None;
                    }
                    arguments.iter().try_fold(0usize, |n, ty| {
                        let total = n.checked_add(scoped_term_value_work_with_evidence(
                            ty,
                            &BTreeMap::new(),
                            parameters,
                            value_parameters,
                            |_, _, _| None,
                        )?)?;
                        (total <= 100_000).then_some(total)
                    })?
                }
                Value::AnonymousRecord(fields) => {
                    if fields.len() > 100_000 {
                        return None;
                    }
                    fields.iter().try_fold(0usize, |n, (name, _)| {
                        let total = n.checked_add(name.len())?;
                        (total <= 100_000).then_some(total)
                    })?
                }
                Value::Object(object) => {
                    if object.entries().len() > 100_000 {
                        return None;
                    }
                    object.entries().iter().try_fold(0usize, |n, entry| {
                        let ty_work = scoped_type_value_work_with_evidence(
                            &entry.value().ty,
                            &BTreeMap::new(),
                            parameters,
                            value_parameters,
                            |_, _, _| None,
                        )?;
                        let total = n
                            .checked_add(entry.key().map_or(0, str::len))?
                            .checked_add(ty_work)?;
                        (total <= 100_000).then_some(total)
                    })?
                }
                Value::Pointer(_)
                | Value::Storage(_)
                | Value::Closure(_)
                | Value::Function(_)
                | Value::RuntimeFunction { .. }
                | Value::RuntimeTraitMethod(_) => return None,
                _ => 0,
            };
            *work = work.checked_add(extra)?;
            let children = match value {
                Value::List { values: fields, .. }
                | Value::Record { fields, .. }
                | Value::Variant { fields, .. }
                | Value::Tuple(fields) => fields.len(),
                Value::AnonymousRecord(fields) => fields.len(),
                Value::Object(object) => object.entries().len(),
                Value::Optional { value, .. } => usize::from(value.is_some()),
                Value::ErrorResult { .. } | Value::Newtype { .. } => 1,
                _ => 0,
            };
            if work.saturating_add(*pending).saturating_add(children) > 100_000 {
                return None;
            }

            *pending = pending.checked_add(children)?;
            let mut child = |value: &Value| {
                visit(
                    value,
                    depth + 1,
                    work,
                    pending,
                    parameters,
                    value_parameters,
                )
            };
            match value {
                Value::Optional { value, .. } => {
                    if let Some(value) = value {
                        child(value)?;
                    }
                }
                Value::ErrorResult { value, .. } | Value::Newtype { value, .. } => child(value)?,
                Value::List { values: fields, .. }
                | Value::Record { fields, .. }
                | Value::Variant { fields, .. }
                | Value::Tuple(fields) => {
                    for value in fields.iter().rev() {
                        child(value)?;
                    }
                }
                Value::AnonymousRecord(fields) => {
                    for (_, value) in fields.iter().rev() {
                        child(value)?;
                    }
                }
                Value::Object(object) => {
                    for entry in object.entries().iter().rev() {
                        child(&entry.value().value)?;
                    }
                }
                _ => {}
            }
            Some(())
        }
        let mut work = 0;
        visit(self, 0, &mut work, &mut 1, parameters, value_parameters)?;
        Some(work)
    }

    /// Freeze supported immutable scalars and aggregates. Nominal values retain
    /// their declared type and shape; reconstruction checks declaration layouts.
    /// Known-head metadata and source formation contracts are checked separately.
    pub fn into_normal_term(
        self,
        budget: &mut comptime_runtime::Budget,
    ) -> Result<Term, ErrorKind> {
        self.into_scoped_normal_term(budget, &BTreeSet::new())
    }
    /// Freeze an invocation input, retaining named callable declaration identities
    /// inside immutable aggregates. Captured environments remain live values;
    /// this path never serializes their storage or execution state.
    pub fn into_scoped_invocation_term(
        self,
        budget: &mut comptime_runtime::Budget,
        parameters: &BTreeSet<DeclarationId>,
    ) -> Result<Term, ErrorKind> {
        self.into_scoped_term(budget, parameters, &BTreeMap::new(), true)
    }
    /// Validate immutable runtime declaration identity for capture snapshots.
    /// This does not turn it into a callable comptime value or a normal term.
    pub fn validate_runtime_identity_snapshot(
        &self,
        budget: &mut comptime_runtime::Budget,
        parameters: &BTreeSet<DeclarationId>,
    ) -> Result<(), ErrorKind> {
        let terms: Vec<&Term> = match self {
            Self::RuntimeFunction { arguments, .. } => {
                budget
                    .consume(arguments.len().saturating_add(1))
                    .map_err(|_| ErrorKind::StepsExhausted)?;
                arguments.iter().collect()
            }
            Self::RuntimeTraitMethod(identity) => {
                budget
                    .consume(
                        identity
                            .interface
                            .arguments
                            .len()
                            .saturating_add(identity.method_arguments.len())
                            .saturating_add(2),
                    )
                    .map_err(|_| ErrorKind::StepsExhausted)?;
                std::iter::once(&identity.subject)
                    .chain(identity.interface.arguments.iter())
                    .chain(identity.method_arguments.iter())
                    .collect()
            }
            _ => return Err(ErrorKind::Type),
        };
        for term in terms {
            validate_snapshot_term(term, budget, parameters)?;
        }
        Ok(())
    }
    /// Freeze a substitution value within an explicitly supplied Type scope.
    /// Immutable export must use `into_normal_term`, which remains closed.
    pub fn into_scoped_normal_term(
        self,
        budget: &mut comptime_runtime::Budget,
        parameters: &BTreeSet<DeclarationId>,
    ) -> Result<Term, ErrorKind> {
        self.into_scoped_term(budget, parameters, &BTreeMap::new(), false)
    }

    /// Reify a residual substitution under explicit declaration-domain authority.
    /// This is not a completed constant or cache-key export.
    pub fn into_abstract_invocation_term(
        self,
        budget: &mut comptime_runtime::Budget,
        type_parameters: &BTreeSet<DeclarationId>,
        value_parameters: &BTreeMap<DeclarationId, Term>,
    ) -> Result<Term, ErrorKind> {
        self.into_scoped_term(budget, type_parameters, value_parameters, true)
    }

    fn into_scoped_term(
        self,
        budget: &mut comptime_runtime::Budget,
        parameters: &BTreeSet<DeclarationId>,
        value_parameters: &BTreeMap<DeclarationId, Term>,
        invocation: bool,
    ) -> Result<Term, ErrorKind> {
        fn charge(budget: &mut comptime_runtime::Budget, work: usize) -> Result<(), ErrorKind> {
            budget.consume(work).map_err(|_| ErrorKind::StepsExhausted)
        }
        fn freeze_integer(
            value: &comptime_runtime::Integer,
            budget: &mut comptime_runtime::Budget,
        ) -> Result<Term, ErrorKind> {
            let limbs = value.bit_len().div_ceil(32);
            if limbs > budget.max_limbs {
                return Err(ErrorKind::RepresentationSizeLimit);
            }
            charge(budget, limbs)?;
            Ok(Term::Integer(value.clone()))
        }
        fn visit(
            value: &Value,
            depth: usize,
            budget: &mut comptime_runtime::Budget,
            parameters: &BTreeSet<DeclarationId>,
            value_parameters: &BTreeMap<DeclarationId, Term>,
            invocation: bool,
        ) -> Result<Term, ErrorKind> {
            if depth > 128 {
                return Err(ErrorKind::DepthExhausted);
            }
            let instance = match value {
                Value::Function(declaration) => Some((*declaration, &[][..])),
                Value::RuntimeFunction {
                    declaration,
                    arguments,
                } => Some((*declaration, arguments.as_slice())),
                Value::Closure(closure)
                    if closure.environment.is_empty() && closure.capture_order.is_empty() =>
                {
                    closure.declaration_instance()
                }
                _ => None,
            };
            if invocation {
                if let Some((declaration, arguments)) = instance {
                    budget.consume(1).map_err(|_| ErrorKind::StepsExhausted)?;
                    for argument in arguments {
                        nir::type_computations::charge_term(argument, &mut budget.steps)
                            .map_err(|_| ErrorKind::StepsExhausted)?;
                        argument.check_scope(&[]).map_err(|_| ErrorKind::Type)?;
                        // A well-scoped value argument may still contain an
                        // unresolved rigid/hole or pending list operation.
                        // Frozen invocation identities must contain completed
                        // arguments, regardless of their semantic sort.
                        let work = scoped_term_value_work_with_layouts(
                            argument,
                            &BTreeMap::new(),
                            parameters,
                            |_, _, _| None,
                        )
                        .ok_or(ErrorKind::Type)?;
                        budget
                            .consume(work)
                            .map_err(|_| ErrorKind::StepsExhausted)?;
                    }
                    return Ok(Term::FunctionValue {
                        declaration,
                        arguments: arguments.to_vec(),
                    });
                }
            }
            charge(budget, 1)?;
            Ok(match value {
                Value::AbstractValue { declaration, ty } => {
                    if parameters.contains(declaration)
                        || *ty == Term::Primitive(Primitive::Type)
                        || value_parameters.get(declaration) != Some(ty)
                    {
                        return Err(ErrorKind::AbstractValueUnresolved);
                    }
                    let work = scoped_type_value_work(ty, &BTreeMap::new(), parameters)
                        .ok_or(ErrorKind::Type)?;
                    charge(budget, work)?;
                    Term::Rigid {
                        declaration: *declaration,
                        sort: nir::Sort::ComptimeValue,
                    }
                }
                Value::TypeParameter(declaration) if parameters.contains(declaration) => {
                    Term::Rigid {
                        declaration: *declaration,
                        sort: nir::Sort::Type,
                    }
                }
                Value::Type(ty) => {
                    let work = if invocation {
                        scoped_type_value_work_with_evidence(
                            ty,
                            &BTreeMap::new(),
                            parameters,
                            value_parameters,
                            |_, _, _| None,
                        )
                    } else {
                        scoped_type_value_work(ty, &BTreeMap::new(), parameters)
                    }
                    .ok_or(ErrorKind::Type)?;
                    charge(budget, work)?;
                    ty.clone()
                }
                Value::Object(object) => {
                    charge(budget, object.entries().len().saturating_mul(4))?;
                    let mut entries = Vec::with_capacity(object.entries().len());
                    for entry in object.entries() {
                        if let Some(key) = entry.key() {
                            charge(budget, key.len())?;
                        }
                        let ObjectValue { ty, value } = entry.value();
                        let work = if invocation {
                            scoped_type_value_work_with_evidence(
                                ty,
                                &BTreeMap::new(),
                                parameters,
                                value_parameters,
                                |_, _, _| None,
                            )
                        } else {
                            scoped_type_value_work(ty, &BTreeMap::new(), parameters)
                        }
                        .ok_or(ErrorKind::Type)?;
                        charge(budget, work)?;
                        let domain = ty.clone();
                        let value = visit(
                            value,
                            depth + 1,
                            budget,
                            parameters,
                            value_parameters,
                            invocation,
                        )?;
                        entries.push(nir::objects::entry(entry.key(), domain, value));
                    }
                    nir::objects::value(entries)
                }
                Value::List { ty, values } => {
                    let Term::NormalForm {
                        sort: nir::Sort::Type,
                        arguments,
                        ..
                    } = ty
                    else {
                        return Err(ErrorKind::Type);
                    };
                    if !matches!(arguments.as_slice(), [element]
                        if element.sort() == nir::Sort::Type && *element != Term::Primitive(Primitive::Type))
                    {
                        return Err(ErrorKind::Type);
                    }
                    let work = if invocation {
                        scoped_type_value_work_with_evidence(
                            ty,
                            &BTreeMap::new(),
                            parameters,
                            value_parameters,
                            |_, _, _| None,
                        )
                    } else {
                        scoped_type_value_work(ty, &BTreeMap::new(), parameters)
                    }
                    .ok_or(ErrorKind::Type)?;
                    charge(budget, work)?;
                    charge(budget, values.len())?;
                    ty.check_scope(&[]).map_err(|_| ErrorKind::Type)?;
                    Term::NominalValue {
                        ty: Box::new(ty.clone()),
                        variant: None,
                        fields: values
                            .iter()
                            .map(|value| {
                                visit(
                                    value,
                                    depth + 1,
                                    budget,
                                    parameters,
                                    value_parameters,
                                    invocation,
                                )
                            })
                            .collect::<Result<_, _>>()?,
                    }
                }
                Value::TypeList(types) => {
                    // Charge width before traversing or allocating a frozen list.
                    charge(budget, types.len())?;
                    for ty in types {
                        let work = if invocation {
                            scoped_type_value_work_with_evidence(
                                ty,
                                &BTreeMap::new(),
                                parameters,
                                value_parameters,
                                |_, _, _| None,
                            )
                        } else {
                            scoped_type_value_work(ty, &BTreeMap::new(), parameters)
                        }
                        .ok_or(ErrorKind::Type)?;
                        charge(budget, work)?;
                    }
                    Term::TypeListValue(types.clone())
                }
                Value::Unit => Term::Unit,
                Value::Bool(value) => Term::Bool(*value),
                Value::Char(value) => Term::Char(*value),
                Value::Float32(bits) => Term::Float32(*bits),
                Value::Float64(bits) => Term::Float64(*bits),
                Value::BitVec(value) => {
                    if value.words().len() > budget.max_limbs {
                        return Err(ErrorKind::RepresentationSizeLimit);
                    }
                    charge(budget, value.words().len())?;
                    Term::BitVec(value.clone())
                }
                Value::Symbol(value) => {
                    charge(budget, value.len())?;
                    Term::Symbol(value.clone())
                }
                Value::String(value) => {
                    charge(budget, value.len())?;
                    Term::String(value.clone())
                }
                Value::UnsignedInteger(value) => {
                    freeze_integer(&comptime_runtime::Integer::from_u128(*value), budget)?
                }
                Value::SignedInteger(value) => {
                    freeze_integer(&comptime_runtime::Integer::from_i128(*value), budget)?
                }
                Value::Float(value) => {
                    let exponent_limbs = value.exponent().bit_len().div_ceil(32);
                    if exponent_limbs > budget.max_limbs {
                        return Err(ErrorKind::RepresentationSizeLimit);
                    }
                    charge(budget, value.digits().len().saturating_add(exponent_limbs))?;
                    comptime_runtime::Integer::parse_literal(value.digits(), budget).map_err(
                        |error| match error {
                            comptime_runtime::Error::SizeLimit => {
                                ErrorKind::RepresentationSizeLimit
                            }
                            comptime_runtime::Error::BudgetExhausted => ErrorKind::StepsExhausted,
                            _ => ErrorKind::Type,
                        },
                    )?;
                    Term::Float(value.clone())
                }
                Value::Integer(value) => freeze_integer(value, budget)?,
                Value::Optional { ty, value } => {
                    if !matches!(ty, Term::Optional(_)) {
                        return Err(ErrorKind::Type);
                    }
                    let work = scoped_type_value_work_with_evidence(
                        ty,
                        &BTreeMap::new(),
                        parameters,
                        value_parameters,
                        |_, _, _| None,
                    )
                    .ok_or(ErrorKind::Type)?;
                    charge(budget, work)?;
                    Term::OptionalValue {
                        ty: Box::new(ty.clone()),
                        value: value
                            .as_ref()
                            .map(|value| {
                                visit(
                                    value,
                                    depth + 1,
                                    budget,
                                    parameters,
                                    value_parameters,
                                    invocation,
                                )
                                .map(Box::new)
                            })
                            .transpose()?,
                    }
                }
                Value::ErrorResult { ty, error, value } => {
                    let Term::ErrorResult { errors, result: _ } = ty else {
                        return Err(ErrorKind::Type);
                    };
                    let work = if invocation {
                        scoped_type_value_work_with_evidence(
                            ty,
                            &BTreeMap::new(),
                            parameters,
                            value_parameters,
                            |_, _, _| None,
                        )
                    } else {
                        scoped_type_value_work(ty, &BTreeMap::new(), parameters)
                    }
                    .ok_or(ErrorKind::Type)?;
                    charge(budget, work)?;
                    if errors.is_empty() {
                        return Err(ErrorKind::Type);
                    }
                    ty.check_scope(&[]).map_err(|_| ErrorKind::Type)?;
                    let label = error
                        .map(|index| {
                            errors
                                .get(index)
                                .cloned()
                                .map(Box::new)
                                .ok_or(ErrorKind::Type)
                        })
                        .transpose()?;
                    let frozen = Term::ErrorValue {
                        ty: Box::new(ty.clone()),
                        label,
                        value: Box::new(visit(
                            value,
                            depth + 1,
                            budget,
                            parameters,
                            value_parameters,
                            invocation,
                        )?),
                    };
                    let work = scoped_term_value_work_with_evidence(
                        &frozen,
                        &BTreeMap::new(),
                        parameters,
                        value_parameters,
                        |_, _, _| None,
                    )
                    .ok_or(ErrorKind::Type)?;
                    charge(budget, work)?;
                    frozen
                }
                Value::Newtype { ty, value } => {
                    if !matches!(ty, Term::Nominal { .. }) {
                        return Err(ErrorKind::Type);
                    }
                    let work = if invocation {
                        scoped_type_value_work_with_evidence(
                            ty,
                            &BTreeMap::new(),
                            parameters,
                            value_parameters,
                            |_, _, _| None,
                        )
                    } else {
                        scoped_type_value_work(ty, &BTreeMap::new(), parameters)
                    }
                    .ok_or(ErrorKind::Type)?;
                    charge(budget, work.saturating_add(1))?;
                    Term::NominalValue {
                        ty: Box::new(ty.clone()),
                        variant: None,
                        fields: vec![visit(
                            value,
                            depth + 1,
                            budget,
                            parameters,
                            value_parameters,
                            invocation,
                        )?],
                    }
                }
                Value::Record {
                    declaration,
                    arguments,
                    fields,
                }
                | Value::Variant {
                    declaration,
                    arguments,
                    fields,
                    ..
                } => {
                    charge(budget, arguments.len().saturating_add(fields.len()))?;
                    for argument in arguments {
                        let work = if invocation {
                            scoped_term_value_work_with_evidence(
                                argument,
                                &BTreeMap::new(),
                                parameters,
                                value_parameters,
                                |_, _, _| None,
                            )
                        } else {
                            scoped_term_value_work_with_layouts(
                                argument,
                                &BTreeMap::new(),
                                parameters,
                                |_, _, _| None,
                            )
                        }
                        .ok_or(ErrorKind::Type)?;
                        charge(budget, work)?;
                    }
                    let ty = Term::Nominal {
                        declaration: *declaration,
                        arguments: arguments.clone(),
                    };
                    Term::NominalValue {
                        ty: Box::new(ty),
                        variant: match value {
                            Value::Variant { index, .. } => Some(
                                u32::try_from(*index)
                                    .map_err(|_| ErrorKind::RepresentationSizeLimit)?,
                            ),
                            _ => None,
                        },
                        fields: fields
                            .iter()
                            .map(|field| {
                                visit(
                                    field,
                                    depth + 1,
                                    budget,
                                    parameters,
                                    value_parameters,
                                    invocation,
                                )
                            })
                            .collect::<Result<_, _>>()?,
                    }
                }
                Value::AnonymousRecord(fields) => {
                    let mut names = std::collections::BTreeSet::new();
                    Term::RecordValue(
                        fields
                            .iter()
                            .map(|(name, value)| {
                                charge(budget, name.len())?;
                                if !names.insert(name.clone()) {
                                    return Err(ErrorKind::Type);
                                }
                                Ok((
                                    name.clone(),
                                    visit(
                                        value,
                                        depth + 1,
                                        budget,
                                        parameters,
                                        value_parameters,
                                        invocation,
                                    )?,
                                ))
                            })
                            .collect::<Result<_, _>>()?,
                    )
                }
                Value::Tuple(values) => Term::TupleValue(
                    values
                        .iter()
                        .map(|value| {
                            visit(
                                value,
                                depth + 1,
                                budget,
                                parameters,
                                value_parameters,
                                invocation,
                            )
                        })
                        .collect::<Result<_, _>>()?,
                ),
                _ => return Err(ErrorKind::Unsupported),
            })
        }
        let result = visit(&self, 0, budget, parameters, value_parameters, invocation);
        // Even rejected host data must be released without recursively dropping
        // a potentially unbounded aggregate tail. Cleanup is not fuel-limited.
        let mut pending = vec![self];
        while let Some(value) = pending.pop() {
            match value {
                Value::Tuple(values) => pending.extend(values),
                Value::List { ty, values } => {
                    ty.drop_iteratively();
                    pending.extend(values);
                }
                Value::Record {
                    fields, arguments, ..
                }
                | Value::Variant {
                    fields, arguments, ..
                } => {
                    pending.extend(fields);
                    for argument in arguments {
                        argument.drop_iteratively();
                    }
                }
                Value::AnonymousRecord(fields) => {
                    pending.extend(fields.into_iter().map(|(_, value)| value))
                }
                Value::Object(object) => {
                    for entry in object.into_entries() {
                        let typed = entry.into_value();
                        typed.ty.drop_iteratively();
                        pending.push(typed.value);
                    }
                }
                Value::Type(ty) => ty.drop_iteratively(),
                Value::Optional { ty, value } => {
                    ty.drop_iteratively();
                    if let Some(value) = value {
                        pending.push(*value);
                    }
                }
                Value::ErrorResult { ty, value, .. } | Value::Newtype { ty, value } => {
                    ty.drop_iteratively();
                    pending.push(*value);
                }
                Value::TypeList(types)
                | Value::RuntimeFunction {
                    arguments: types, ..
                } => {
                    for ty in types {
                        ty.drop_iteratively();
                    }
                }
                Value::RuntimeTraitMethod(method) => {
                    method.subject.drop_iteratively();
                    for ty in method
                        .interface
                        .arguments
                        .into_iter()
                        .chain(method.method_arguments)
                    {
                        ty.drop_iteratively();
                    }
                }
                _ => {}
            }
        }
        result
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Closure {
    definition_source: Option<SourceRange>,
    origin: Option<DeclarationId>,
    instance_arguments: Option<Vec<Term>>,
    parameters: Vec<LocalId>,
    environment: Environment,
    capture_order: Vec<LocalId>,
    body: Arc<Expr>,
    ty: Term,
}
impl Closure {
    pub fn parameters(&self) -> &[LocalId] {
        &self.parameters
    }
    pub fn body(&self) -> &Arc<Expr> {
        &self.body
    }
    /// The checked callable type, including capture domains and phase.
    pub fn ty(&self) -> &Term {
        &self.ty
    }
    /// Capture slots in semantic field order, independent of caller locals.
    /// Slots may be Storage cells: constant materializers must snapshot their
    /// current values rather than persist these interpreter handles.
    pub fn captured_values(&self) -> impl ExactSizeIterator<Item = (LocalId, &Value)> {
        self.capture_order
            .iter()
            .map(|id| (*id, &self.environment[id]))
    }
    /// A declaration instance can be restored without serializing an execution
    /// environment or treating an arbitrary lambda as a named function.
    pub fn declaration_instance(&self) -> Option<(DeclarationId, &[Term])> {
        Some((self.origin?, self.instance_arguments.as_deref()?))
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub steps: u64,
    pub depth: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            steps: 100_000,
            depth: 128,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidDefinition(DefinitionBuildError),
    BuildMetadataUnavailable {
        owner: u64,
        provider: Option<u64>,
        alias: String,
    },
    DivisionByZero,
    IndexOutOfBounds,
    MachineOverflow,
    PhaseMismatch,
    InvalidBitWidth,
    ProgramContractRejected,
    TypeContractRejected,
    TypeContractUnresolved,
    AbstractValueUnresolved,
    StepsExhausted,
    RepresentationSizeLimit,
    DepthExhausted,
    UnknownFunction,
    MissingBinding,
    InvalidCallPlan,
    ExpiredPointer,
    InvalidPointer,
    UnmaterializedDispatch,
    Arity,
    Type,
    Unsupported,
    InvalidControlFlow,
    NonExhaustiveMatch,
    InvalidEffectEnvironment,
    EffectControlTransfer,
    EffectValueConsumed,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub source: Option<SourceRange>,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ErrorKind::InvalidDefinition(reason) => write!(f, "interpretation {reason}")?,
            _ => write!(f, "interpretation {:?}", self.kind)?,
        }
        if self.kind == ErrorKind::StepsExhausted {
            f.write_str(" (step budget exhausted)")?;
        }
        if let Some(source) = self.source {
            write!(f, " at {source}")?;
        }
        Ok(())
    }
}
impl std::error::Error for Error {}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    CheckPredicate {
        source: SourceRange,
        passed: bool,
    },
    InstantiateFunction {
        declaration: DeclarationId,
        arguments: Vec<Term>,
    },
    EnterFunction(DeclarationId),
    CreateClosure {
        source: SourceRange,
        captures: usize,
    },
}
#[derive(Clone, Debug)]
enum Flow {
    Value(Value),
    Return(Value),
    Break(LocalId),
    Continue(LocalId),
    EffectResume(u64, Value),
}
enum SelectedBitRange {
    Range(usize, usize),
    Control(Flow),
}
// Prepared actual values are owned here; the cleanup frame remains installed
// across preparation and invocation. Preparation temporaries do not occupy
// every recursive caller's host stack while the callee executes.
struct InterfaceExit {
    conditions: Vec<Expr>,
    result_binding: Option<LocalId>,
    environment: Environment,
    result_slot: usize,
}
enum PreparedCall {
    Invoke {
        interface_exit: Option<Box<InterfaceExit>>,
        callee: Value,
        arguments: Vec<Value>,
        context: Option<(iir::InstanceKey, Vec<iir::cleanup::ContextKey>)>,
    },
    Control(Flow),
}

macro_rules! value {
    ($result:expr) => {
        match $result? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        }
    };
}

mod invocation_budget;

mod effects;

/// Result from the compiler's type-proposition solver. Unknown never authorizes
/// construction. Program-state/resource specifications use a separate system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContractOutcome {
    Satisfied,
    Rejected,
    Unknown,
}

enum TypeSolver<'a> {
    Unmetered(&'a dyn Fn(&nir::program::TypePredicate) -> ContractOutcome),
    Budgeted(&'a dyn Fn(&nir::program::TypePredicate, &mut u64) -> ContractOutcome),
}

enum PointerCopyPermission<'a> {
    Checked(&'a dyn Fn(&Term) -> bool),
    Budgeted(&'a dyn Fn(&Term, &mut u64) -> bool),
}

pub struct Interpreter<'a> {
    enum_builder:
        Option<&'a dyn Fn(&EnumBuildRequest, &mut u64) -> Result<PreparedEnumBuild<'a>, Error>>,
    struct_builder:
        Option<&'a dyn Fn(&StructBuildRequest, &mut u64) -> Result<PreparedStructBuild<'a>, Error>>,
    declaration_world: Option<&'a Package>,
    target: Option<nir::target::TargetData>,
    effect_declarations: BTreeMap<DeclarationId, &'a nir::effects::Declaration>,
    effect_state: effects::State,
    effect_values: Option<&'a iir::effect_values::Plan<'a>>,
    graph: Option<&'a iir::Package>,
    cleanup_schedule: Option<&'a iir::cleanup::Schedule>,
    cleanup_scopes: HashMap<&'a iir::InstanceKey, Vec<&'a iir::cleanup::FunctionSchedule>>,
    // Borrowed from the validated, immutable schedule. Preserve all sites at
    // each source position, including distinct contextual instances.
    cleanup_sites:
        HashMap<(u32, u32), Vec<(&'a iir::cleanup::FunctionSchedule, &'a iir::cleanup::Site)>>,
    cleanup_effect_context: HashMap<nir::effects::Operation, bool>,
    cleanup_frame: Option<iir::InstanceKey>,
    cleanup_failed: bool,
    prepared_frames: Vec<cleanup::PreparedFrame>,
    residual_temporaries: Vec<(SourceRange, memory::Storage)>,
    pattern_binding_orders: BTreeMap<(u32, u32), Vec<LocalId>>,
    cleanup_context: Vec<iir::cleanup::ContextKey>,
    comptime_only: bool,
    abstract_type_parameters: BTreeSet<DeclarationId>,
    abstract_value_parameters: BTreeMap<DeclarationId, Term>,
    type_solver: Option<TypeSolver<'a>>,
    pointer_copy: Option<PointerCopyPermission<'a>>,
    declaration_parameter_sorts: BTreeMap<DeclarationId, nir::Sort>,
    declaration_parameter_domains: BTreeMap<DeclarationId, Term>,
    method_parameters: HashMap<(DeclarationId, DeclarationId), Vec<LocalId>>,
    // None records an ambiguous member identity across different interfaces.
    method_origins: HashMap<DeclarationId, Option<DeclarationId>>,
    method_contracts: HashMap<
        (DeclarationId, DeclarationId),
        (
            Vec<nir::DeclarationParameter>,
            &'a nir::program::MethodContract,
        ),
    >,
    implemented_interfaces: HashMap<(InterfaceInstance, Term), DeclarationId>,
    assumed_interfaces: HashSet<(InterfaceInstance, Term)>,
    method_bodies: HashMap<(InterfaceInstance, Term, DeclarationId), (DeclarationId, Vec<Term>)>,
    associated_values: HashMap<(InterfaceInstance, Term, DeclarationId), &'a Expr>,
    associated_initializers: HashMap<(InterfaceInstance, Term, DeclarationId), &'a Expr>,
    active_associated_initializers: HashSet<(InterfaceInstance, Term, DeclarationId)>,
    instances: HashMap<(DeclarationId, Vec<Term>), Arc<Closure>>,
    functions: BTreeMap<DeclarationId, &'a nir::program::Function>,
    runtime_function_identities: BTreeMap<DeclarationId, &'a nir::program::RuntimeFunctionIdentity>,
    normal_forms: BTreeMap<DeclarationId, &'a nir::normal_forms::Head>,
    structs: BTreeMap<DeclarationId, generated_structs::StructDefinition<'a>>,
    newtypes: BTreeMap<DeclarationId, &'a nir::newtypes::Newtype>,
    alias_contracts: BTreeMap<DeclarationId, &'a nir::type_aliases::Contract>,
    enums: BTreeMap<DeclarationId, generated_enums::EnumDefinition<'a>>,
    budget: Budget,
    depth: usize,
    events: Vec<Event>,
}
impl<'a> Interpreter<'a> {
    fn input_binders(&self, parameters: &[DeclarationId]) -> Vec<nir::DeclarationParameter> {
        parameters
            .iter()
            .map(|id| nir::DeclarationParameter {
                declaration: *id,
                sort: self
                    .declaration_parameter_sorts
                    .get(id)
                    .copied()
                    .unwrap_or(nir::Sort::Type),
            })
            .collect()
    }
    pub fn target(&self) -> Option<nir::target::TargetData> {
        self.target
    }

    fn integer_layout(&self, primitive: Primitive) -> Option<(bool, u32)> {
        match self.target {
            Some(target) => target.integer_layout(primitive),
            None => primitive.fixed_integer_layout(),
        }
    }

    pub fn new(package: &'a Package, budget: Budget) -> Result<Self, Error> {
        Self::with_assumptions(package, &[], budget)
    }
    fn with_assumptions(
        package: &'a Package,
        assumptions: &[nir::program::TraitRequirement],
        mut budget: Budget,
    ) -> Result<Self, Error> {
        invocation_budget::charge_world_tables(package, &mut budget)?;
        // Build this once before checking body-free runtime identities. Looking
        // through the source Vec for each identity makes world setup quadratic.
        let mut functions = BTreeMap::new();
        for function in &package.functions {
            if functions.insert(function.declaration, function).is_some() {
                return Err(Error {
                    kind: ErrorKind::InvalidCallPlan,
                    source: None,
                });
            }
        }
        let mut runtime_function_identities = BTreeMap::new();
        for identity in &package.runtime_function_identities {
            if !matches!(
                identity.signature.callable_signature(),
                Term::Function { .. }
            ) || identity.signature.is_comptime_callable()
                || functions.contains_key(&identity.declaration)
                || runtime_function_identities
                    .insert(identity.declaration, identity)
                    .is_some()
            {
                return Err(Error {
                    kind: ErrorKind::InvalidCallPlan,
                    source: None,
                });
            }
        }
        let mut effect_declarations = BTreeMap::new();
        for effect in &package.effects {
            if !matches!(effect.signature, Term::Function { .. })
                || effect_declarations
                    .insert(effect.declaration, effect)
                    .is_some()
            {
                return Err(Error {
                    kind: ErrorKind::InvalidCallPlan,
                    source: None,
                });
            }
        }
        let mut structs = BTreeMap::new();
        let mut enums = BTreeMap::new();
        for definition in &package.enums {
            if definition.validate_encoding_in(package.target).is_err() {
                return Err(Error {
                    kind: ErrorKind::InvalidCallPlan,
                    source: None,
                });
            }
            if enums
                .insert(
                    definition.declaration,
                    generated_enums::EnumDefinition::Borrowed(definition),
                )
                .is_some()
            {
                return Err(Error {
                    kind: ErrorKind::InvalidCallPlan,
                    source: None,
                });
            }
        }
        for definition in &package.structs {
            if structs
                .insert(
                    definition.declaration,
                    generated_structs::StructDefinition::Borrowed(definition),
                )
                .is_some()
            {
                return Err(Error {
                    kind: ErrorKind::InvalidCallPlan,
                    source: None,
                });
            }
        }
        let invalid = || Error {
            kind: ErrorKind::InvalidCallPlan,
            source: None,
        };
        nir::normal_forms::validate(package).map_err(|_| invalid())?;
        let completed_tuple_heads = package
            .normal_forms
            .iter()
            .filter(|metadata| metadata.role == nir::normal_forms::Role::Tuple)
            .map(|metadata| metadata.declaration)
            .collect::<BTreeSet<_>>();
        let mut traits = BTreeMap::new();
        for interface in &package.traits {
            if traits.insert(interface.declaration, interface).is_some() {
                return Err(invalid());
            }
            let mut names = BTreeSet::new();
            let mut members = BTreeSet::new();
            for member in &interface.methods {
                if !matches!(
                    member.signature.ordinary(),
                    Ok(Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_))
                ) {
                    return Err(invalid());
                }
                let Ok(Term::Function { parameters, .. }) = member.signature.callable_signature()
                else {
                    return Err(invalid());
                };
                if member.parameter_bindings.len() != parameters.len()
                    || member
                        .parameter_bindings
                        .iter()
                        .collect::<BTreeSet<_>>()
                        .len()
                        != parameters.len()
                {
                    return Err(invalid());
                }
                if !names.insert(&member.name) || !members.insert(member.declaration) {
                    return Err(invalid());
                }
                if member.default.is_some_and(|body| {
                    !functions.contains_key(&body)
                        && !runtime_function_identities.contains_key(&body)
                }) {
                    return Err(invalid());
                }
            }
        }
        let mut assumed_interfaces = HashSet::new();
        if assumptions.len() as u64 > budget.steps {
            return Err(Error {
                kind: ErrorKind::StepsExhausted,
                source: None,
            });
        }
        let mut pending: Vec<_> = assumptions
            .iter()
            .cloned()
            .map(|premise| (premise, 0usize))
            .collect();
        let mut premise_fuel = budget.steps;
        let mut trait_parameter_fuel = usize::try_from(budget.steps).unwrap_or(usize::MAX);
        let declaration_parameter_domains = package
            .declaration_parameter_domains(&mut trait_parameter_fuel)
            .map_err(|_| invalid())?;
        let declaration_parameter_sorts: BTreeMap<_, _> = declaration_parameter_domains
            .iter()
            .map(|(id, domain)| {
                (
                    *id,
                    if *domain == Term::Primitive(Primitive::Type) {
                        nir::Sort::Type
                    } else {
                        nir::Sort::ComptimeValue
                    },
                )
            })
            .collect();
        let input_binders = |parameters: &[DeclarationId]| {
            parameters
                .iter()
                .map(|id| nir::DeclarationParameter {
                    declaration: *id,
                    sort: declaration_parameter_sorts
                        .get(id)
                        .copied()
                        .unwrap_or(nir::Sort::Type),
                })
                .collect::<Vec<_>>()
        };
        while let Some((premise, depth)) = pending.pop() {
            if depth >= 64 {
                return Err(Error {
                    kind: ErrorKind::DepthExhausted,
                    source: None,
                });
            }
            if premise_fuel == 0 {
                return Err(Error {
                    kind: ErrorKind::StepsExhausted,
                    source: None,
                });
            }
            premise_fuel -= 1;
            let contract = traits.get(&premise.interface).ok_or_else(invalid)?;
            if contract.type_parameters.len() != premise.interface_arguments.len() {
                return Err(invalid());
            }
            if !assumed_interfaces.insert((premise.interface_instance(), premise.subject.clone())) {
                continue;
            }
            let parameters = contract
                .binders_with_self(&mut trait_parameter_fuel)
                .map_err(|_| invalid())?;
            let arguments: Vec<_> = std::iter::once(premise.subject.clone())
                .chain(premise.interface_arguments)
                .collect();
            if pending
                .len()
                .saturating_add(contract.capabilities.len())
                .saturating_add(contract.parents.len()) as u64
                > premise_fuel
            {
                return Err(Error {
                    kind: ErrorKind::StepsExhausted,
                    source: None,
                });
            }
            for capability in &contract.capabilities {
                pending.push((
                    capability
                        .instantiate_parameters(&parameters, &arguments)
                        .map_err(|_| invalid())?,
                    depth + 1,
                ));
            }
            for parent in &contract.parents {
                let parent = parent
                    .instantiate_parameters(&parameters, &arguments)
                    .map_err(|_| invalid())?;
                pending.push((
                    nir::program::TraitRequirement {
                        interface: parent.declaration,
                        interface_arguments: parent.arguments,
                        subject: premise.subject.clone(),
                    },
                    depth + 1,
                ));
            }
        }
        let mut method_bodies = HashMap::new();
        for interface in traits.values() {
            if interface
                .parents
                .iter()
                .any(|parent| !traits.contains_key(&parent.declaration))
            {
                return Err(invalid());
            }
        }
        let mut worlds = HashMap::new();
        let mut implementations = BTreeMap::new();
        for implementation in &package.implementations {
            if !implementation.phases.runtime && !implementation.phases.comptime {
                return Err(invalid());
            }
            if implementations
                .insert(implementation.declaration, implementation)
                .is_some()
            {
                return Err(invalid());
            }
            let concrete =
                implementation.type_parameters.is_empty() && implementation.requirements.is_empty();
            let Some(interface) = implementation.interface_instance() else {
                if implementation.phases.comptime {
                    return Err(invalid());
                }
                continue;
            };
            let contract = traits.get(&interface.declaration).ok_or_else(invalid)?;
            if interface.arguments.len() != contract.type_parameters.len() {
                return Err(invalid());
            }
            if concrete {
                contract
                    .validate_arguments_with_layouts(
                        &implementation.subject,
                        &interface.arguments,
                        &[],
                        &package.traits,
                        &BTreeMap::new(),
                        &mut trait_parameter_fuel,
                        |_, _, _| false,
                        Some(package),
                    )
                    .map_err(|_| invalid())?;
            }
            if concrete
                && worlds
                    .insert(
                        (interface.clone(), implementation.subject.clone()),
                        implementation.declaration,
                    )
                    .is_some()
            {
                return Err(invalid());
            }
            let mut supplied = BTreeMap::new();
            for (name, body) in &implementation.methods {
                if supplied.insert(name, body).is_some()
                    || (!functions.contains_key(body)
                        && !runtime_function_identities.contains_key(body))
                {
                    return Err(invalid());
                }
            }
            for member in &contract.methods {
                if !implementation.phases.contains(member.phase) {
                    if supplied.contains_key(&member.name) {
                        return Err(invalid());
                    }
                    continue;
                }
                let body = **supplied.get(&member.name).ok_or_else(invalid)?;
                let (phase, type_parameters) = if let Some(function) = functions.get(&body) {
                    (function.phase, &function.type_parameters)
                } else {
                    (
                        nir::program::FunctionPhase::Runtime,
                        &runtime_function_identities[&body].type_parameters,
                    )
                };
                if phase == nir::program::FunctionPhase::NormalForm
                    || (phase == nir::program::FunctionPhase::Comptime)
                        != member.signature.is_comptime_callable()
                {
                    return Err(invalid());
                }
                let expected_parameters = if member.default == Some(body) {
                    contract.parameters_with_self()
                } else {
                    implementation.type_parameters.clone()
                };
                if type_parameters.len() != expected_parameters.len() + member.type_parameters.len()
                {
                    return Err(invalid());
                }
                if !type_parameters.starts_with(&expected_parameters)
                    && !nir::function_parameters::equivalent_prefix_domains(
                        &expected_parameters,
                        &type_parameters[..expected_parameters.len()],
                        &declaration_parameter_domains,
                        &mut trait_parameter_fuel,
                    )
                    .map_err(|_| Error {
                        kind: ErrorKind::StepsExhausted,
                        source: None,
                    })?
                {
                    return Err(invalid());
                }
                if concrete {
                    let arguments = if member.default == Some(body) {
                        std::iter::once(implementation.subject.clone())
                            .chain(interface.arguments.iter().cloned())
                            .collect()
                    } else {
                        vec![]
                    };
                    method_bodies.insert(
                        (
                            interface.clone(),
                            implementation.subject.clone(),
                            member.declaration,
                        ),
                        (body, arguments),
                    );
                }
            }
        }
        let mut associated_values = HashMap::new();
        let mut associated_initializers = HashMap::new();
        for instance in &package.dispatch_instances {
            for (member, value) in &instance.associated {
                if associated_values
                    .insert(
                        (
                            instance.interface_instance(),
                            instance.subject.clone(),
                            *member,
                        ),
                        value,
                    )
                    .is_some()
                {
                    return Err(invalid());
                }
            }
        }

        for instance in &package.dispatch_instances {
            for (member, initializer) in &instance.associated_initializers {
                let key = (
                    instance.interface_instance(),
                    instance.subject.clone(),
                    *member,
                );
                if associated_values.contains_key(&key)
                    || associated_initializers
                        .insert(key, initializer.expression())
                        .is_some()
                {
                    return Err(invalid());
                }
            }
        }

        let mut instance_keys = HashSet::new();
        for instance in &package.dispatch_instances {
            if !instance_keys.insert((instance.interface_instance(), instance.subject.clone())) {
                return Err(invalid());
            }
            let implementation = implementations
                .get(&instance.implementation)
                .copied()
                .ok_or_else(invalid)?;
            nir::validate_declaration_arguments_in(
                &implementation.type_parameters,
                &implementation.parameter_types,
                &instance.arguments,
                &mut trait_parameter_fuel,
                package.target,
            )
            .map_err(|_| invalid())?;
            if implementation
                .interface_instance()
                .ok_or_else(invalid)?
                .instantiate_parameters(
                    &input_binders(&implementation.type_parameters),
                    &instance.arguments,
                )
                .map_err(|_| invalid())?
                != instance.interface_instance()
                || implementation
                    .subject
                    .instantiate_parameters(
                        &input_binders(&implementation.type_parameters),
                        &instance.arguments,
                    )
                    .map_err(|_| invalid())?
                    != instance.subject
            {
                return Err(invalid());
            }
            let contract = traits.get(&instance.interface).ok_or_else(invalid)?;
            for (member, initializer) in &instance.associated_initializers {
                let member_contract = contract
                    .associated
                    .iter()
                    .find(|m| m.declaration == *member)
                    .ok_or_else(invalid)?;
                if member_contract.phase == nir::program::TraitPhase::Comptime
                    && !implementation.phases.comptime
                {
                    return Err(invalid());
                }
                let inputs: Vec<_> = std::iter::once(instance.subject.clone())
                    .chain(instance.interface_arguments.iter().cloned())
                    .collect();
                let expected = member_contract
                    .ty
                    .instantiate_parameters(
                        &contract
                            .binders_with_self(&mut trait_parameter_fuel)
                            .map_err(|_| invalid())?,
                        &inputs,
                    )
                    .map_err(|_| invalid())?;
                if !selected_types::same(
                    &initializer.expression().ty,
                    &expected,
                    &associated_values,
                ) {
                    return Err(invalid());
                }
                if implementation.associated.iter().any(|(id, _)| id == member) {
                    return Err(invalid());
                }
                let mut originals = implementation
                    .associated_initializers
                    .iter()
                    .filter(|(id, _)| id == member);
                let (_, original) = originals.next().ok_or_else(invalid)?;
                if originals.next().is_some() {
                    return Err(invalid());
                }
                let mut fuel = usize::try_from(budget.steps).unwrap_or(usize::MAX);
                let specialized = original
                    .instantiate(
                        &input_binders(&implementation.type_parameters),
                        &instance.arguments,
                        &mut fuel,
                    )
                    .map_err(|_| invalid())?;
                if specialized != *initializer {
                    return Err(invalid());
                }
            }
            for (member, value) in &instance.associated {
                let member_contract = contract
                    .associated
                    .iter()
                    .find(|m| m.declaration == *member)
                    .ok_or_else(invalid)?;
                if member_contract.phase == nir::program::TraitPhase::Comptime
                    && !implementation.phases.comptime
                {
                    return Err(invalid());
                }
                let member_inputs: Vec<_> = std::iter::once(instance.subject.clone())
                    .chain(instance.interface_arguments.iter().cloned())
                    .collect();
                let member_type = member_contract
                    .ty
                    .instantiate_parameters(
                        &contract
                            .binders_with_self(&mut trait_parameter_fuel)
                            .map_err(|_| invalid())?,
                        &member_inputs,
                    )
                    .map_err(|_| invalid())?;
                if !selected_types::same(&value.ty, &member_type, &associated_values) {
                    return Err(invalid());
                }
                let mut originals = implementation
                    .associated
                    .iter()
                    .filter(|(id, _)| id == member);
                let (_, original) = originals.next().ok_or_else(invalid)?;
                // Payloads are completed values or closed snapshot graphs.
                // Reconstruction cannot invoke arbitrary code or capture the
                // invoking function's lexical environment.
                let mut value_fuel = usize::try_from(budget.steps).unwrap_or(usize::MAX);
                nir::completed_values::validate_with_declarations_in(
                    value,
                    package.newtypes.iter(),
                    &|head| completed_tuple_heads.contains(&head),
                    &|id, fuel| nir::completed_values::lookup_nominal(package, id, fuel),
                    package.target,
                    &mut value_fuel,
                )
                .map_err(|_| invalid())?;
                nir::completed_values::validate_with_declarations_in(
                    original,
                    package.newtypes.iter(),
                    &|head| completed_tuple_heads.contains(&head),
                    &|id, fuel| nir::completed_values::lookup_nominal(package, id, fuel),
                    package.target,
                    &mut value_fuel,
                )
                .map_err(|_| invalid())?;
                nir::copy_budget::charge_substitution(
                    original,
                    &input_binders(&implementation.type_parameters),
                    &instance.arguments,
                    &mut value_fuel,
                )
                .map_err(|_| invalid())?;
                if originals.next().is_some()
                    || original
                        .instantiate_parameters(
                            &input_binders(&implementation.type_parameters),
                            &instance.arguments,
                        )
                        .map_err(|_| invalid())?
                        != *value
                {
                    return Err(invalid());
                }
            }

            // An explicit dispatch instance contains demanded members only.
            // It supersedes the legacy concrete-implementation lookup, so an
            // omitted conditional member cannot be recovered from that table.
            let instance_interface = instance.interface_instance();
            method_bodies.retain(|(interface, subject, _), _| {
                interface != &instance_interface || subject != &instance.subject
            });
            let mut method_keys = HashSet::new();
            for method in &instance.methods {
                let member = contract
                    .methods
                    .iter()
                    .find(|m| m.declaration == method.member)
                    .ok_or_else(invalid)?;
                if !implementation.phases.contains(member.phase) {
                    return Err(invalid());
                }
                if member.type_parameters.len() != method.method_arguments.len()
                    || !method_keys.insert((method.member, method.method_arguments.clone()))
                {
                    return Err(invalid());
                }
                let expected_body = implementation
                    .methods
                    .iter()
                    .find(|(name, _)| name == &member.name)
                    .ok_or_else(invalid)?
                    .1;
                let prefix_arguments: Vec<_> = if member.default == Some(expected_body) {
                    std::iter::once(instance.subject.clone())
                        .chain(instance.interface_arguments.iter().cloned())
                        .collect()
                } else {
                    instance.arguments.clone()
                };
                let expected_arguments: Vec<_> = prefix_arguments
                    .iter()
                    .cloned()
                    .chain(method.method_arguments.iter().cloned())
                    .collect();
                if method.member != member.declaration
                    || method.body != expected_body
                    || method.arguments != expected_arguments
                {
                    return Err(invalid());
                }
                let ty = if let Some(function) = functions.get(&method.body) {
                    function
                        .signature
                        .instantiate_parameters(
                            &input_binders(&function.type_parameters),
                            &method.arguments,
                        )
                        .map_err(|_| invalid())?
                        .with_comptime_phase(
                            function.phase == nir::program::FunctionPhase::Comptime,
                        )
                        .with_unsafe_call_requirement(function.is_unsafe)
                } else if let Some(identity) = runtime_function_identities.get(&method.body) {
                    identity
                        .signature
                        .instantiate_parameters(
                            &input_binders(&identity.type_parameters),
                            &method.arguments,
                        )
                        .map_err(|_| invalid())?
                } else {
                    return Err(invalid());
                };
                let required = member
                    .signature
                    .instantiate_parameters(
                        &contract
                            .binders_with_self(&mut trait_parameter_fuel)
                            .map_err(|_| invalid())?
                            .into_iter()
                            .chain(input_binders(&member.type_parameters))
                            .collect::<Vec<_>>(),
                        &std::iter::once(instance.subject.clone())
                            .chain(instance.interface_arguments.iter().cloned())
                            .chain(method.method_arguments.iter().cloned())
                            .collect::<Vec<_>>(),
                    )
                    .map_err(|_| invalid())?;
                if !selected_types::same(
                    &ty,
                    required.ordinary().map_err(|_| invalid())?,
                    &associated_values,
                ) {
                    return Err(invalid());
                }
                let key = (
                    instance.interface_instance(),
                    instance.subject.clone(),
                    member.declaration,
                );
                let value = (method.body, prefix_arguments);
                if method_bodies.get(&key).is_some_and(|old| old != &value) {
                    return Err(invalid());
                }
                method_bodies.insert(key, value);
            }
            let key = (instance.interface_instance(), instance.subject.clone());
            if worlds
                .get(&key)
                .is_some_and(|old| *old != instance.implementation)
            {
                return Err(invalid());
            }
            worlds.insert(key, instance.implementation);
        }
        for instance in &package.dispatch_instances {
            let implementation = implementations
                .get(&instance.implementation)
                .copied()
                .ok_or_else(invalid)?;
            for capability in &implementation.capabilities {
                let capability = capability
                    .instantiate_parameters(
                        &input_binders(&implementation.type_parameters),
                        &instance.arguments,
                    )
                    .map_err(|_| invalid())?;
                if !worlds
                    .contains_key(&(capability.interface_instance(), capability.subject.clone()))
                    && !assumed_interfaces
                        .contains(&(capability.interface_instance(), capability.subject))
                {
                    return Err(invalid());
                }
            }
        }
        for (interface, subject) in worlds.keys().chain(assumed_interfaces.iter()) {
            let contract = traits[&interface.declaration];
            let parameters = contract
                .binders_with_self(&mut trait_parameter_fuel)
                .map_err(|_| invalid())?;
            for capability in &contract.capabilities {
                let capability = capability
                    .instantiate_parameters(
                        &parameters,
                        &std::iter::once(subject.clone())
                            .chain(interface.arguments.iter().cloned())
                            .collect::<Vec<_>>(),
                    )
                    .map_err(|_| invalid())?;
                if !worlds
                    .contains_key(&(capability.interface_instance(), capability.subject.clone()))
                    && !assumed_interfaces
                        .contains(&(capability.interface_instance(), capability.subject))
                {
                    return Err(invalid());
                }
            }
            let arguments: Vec<_> = std::iter::once(subject.clone())
                .chain(interface.arguments.iter().cloned())
                .collect();
            for parent in &contract.parents {
                let parent = parent
                    .instantiate_parameters(&parameters, &arguments)
                    .map_err(|_| invalid())?;
                if !worlds.contains_key(&(parent.clone(), subject.clone()))
                    && !assumed_interfaces.contains(&(parent, subject.clone()))
                {
                    return Err(invalid());
                }
            }
        }
        let method_parameters = package
            .traits
            .iter()
            .flat_map(|t| {
                t.methods
                    .iter()
                    .map(move |m| ((t.declaration, m.declaration), m.parameter_bindings.clone()))
            })
            .collect();
        let mut method_contracts = HashMap::new();
        let mut method_origins = HashMap::new();
        for contract in &package.traits {
            let parameters = contract
                .binders_with_self(&mut trait_parameter_fuel)
                .map_err(|_| invalid())?;
            for member in &contract.methods {
                method_origins
                    .entry(member.declaration)
                    .and_modify(|origin| *origin = None)
                    .or_insert(Some(contract.declaration));
                let mut binders = parameters.clone();
                binders.extend(input_binders(&member.type_parameters));
                method_contracts.insert(
                    (contract.declaration, member.declaration),
                    (binders, member),
                );
            }
        }
        let remaining_steps = premise_fuel
            .checked_sub(budget.steps.saturating_sub(trait_parameter_fuel as u64))
            .ok_or(Error {
                kind: ErrorKind::StepsExhausted,
                source: None,
            })?;
        Ok(Self {
            declaration_world: Some(package),
            effect_declarations,
            effect_state: effects::State::default(),
            effect_values: None,
            graph: None,
            target: package.target,
            cleanup_schedule: None,
            cleanup_scopes: HashMap::new(),
            cleanup_sites: HashMap::new(),
            cleanup_effect_context: HashMap::new(),
            cleanup_frame: None,
            cleanup_failed: false,
            prepared_frames: vec![],
            residual_temporaries: vec![],
            pattern_binding_orders: BTreeMap::new(),
            cleanup_context: vec![],
            comptime_only: false,
            abstract_type_parameters: BTreeSet::new(),
            abstract_value_parameters: BTreeMap::new(),
            struct_builder: None,
            enum_builder: None,
            type_solver: None,
            pointer_copy: None,
            method_parameters,
            declaration_parameter_sorts,
            declaration_parameter_domains,
            method_contracts,
            method_origins,
            implemented_interfaces: worlds,
            assumed_interfaces,
            method_bodies,
            associated_values,
            associated_initializers,
            active_associated_initializers: HashSet::new(),
            instances: HashMap::new(),
            functions,
            runtime_function_identities,
            normal_forms: package
                .normal_forms
                .iter()
                .map(|head| (head.declaration, head))
                .collect(),
            structs,
            alias_contracts: package
                .alias_contracts
                .iter()
                .map(|contract| (contract.declaration, contract))
                .collect(),
            newtypes: package
                .newtypes
                .iter()
                .map(|d| (d.declaration, d))
                .collect(),
            enums,
            budget: Budget {
                steps: remaining_steps,
                // Bound native recursion independently of a caller-supplied
                // budget: call frames include contracts and cleanup contexts.
                // This guard must fire on the supported 2 MiB host stack.
                depth: budget.depth.min(64),
            },
            depth: 0,
            events: Vec::new(),
        })
    }
    /// A phase-checked entrypoint for concrete first-order comptime functions.
    /// Resource/effect verification remains the caller's separate obligation.
    pub fn for_comptime(package: &'a Package, budget: Budget) -> Result<Self, Error> {
        Self::for_comptime_with_assumptions(package, &[], budget)
    }
    /// Evaluate under caller-established trait premises. Premises discharge
    /// obligations only; they provide neither method bodies nor associated values.
    /// They belong to this interpreter and never mutate the source package.
    pub fn for_comptime_with_assumptions(
        package: &'a Package,
        assumptions: &[nir::program::TraitRequirement],
        budget: Budget,
    ) -> Result<Self, Error> {
        // Reject an undersized world allowance before phase validation walks
        // declarations. Actual charging occurs once in with_assumptions.
        let mut phase_allowance = budget;
        invocation_budget::charge_world_tables(package, &mut phase_allowance)?;
        nir::phases::validate(package).map_err(|error| Error {
            kind: ErrorKind::PhaseMismatch,
            source: Some(error.source),
        })?;
        let mut interpreter = Self::with_assumptions(package, assumptions, budget)?;
        interpreter.comptime_only = true;
        Ok(interpreter)
    }
    /// Prepare a comptime world using a caller-owned construction allowance.
    /// Success reports the remaining steps. Failure (including unwinding) keeps
    /// the allowance reserved: a failed constructor cannot report a trustworthy
    /// remainder. This is conservative reservation, not exact failed-work timing.
    /// Depth configuration is unchanged. After executing the returned VM, callers
    /// must synchronize remaining_steps again; this does not share a live counter.
    pub fn for_comptime_in_budget(
        package: &'a Package,
        assumptions: &[nir::program::TraitRequirement],
        budget: &mut Budget,
    ) -> Result<Self, Error> {
        let reserved = *budget;
        budget.steps = 0;
        let interpreter = Self::for_comptime_with_assumptions(package, assumptions, reserved)?;
        budget.steps = interpreter.remaining_steps();
        Ok(interpreter)
    }
    /// Supply the host's type theory without coupling the interpreter to the
    /// elaborator. The callback receives an instantiated type predicate only.
    pub fn with_type_solver(
        mut self,
        solver: &'a dyn Fn(&nir::program::TypePredicate) -> ContractOutcome,
    ) -> Self {
        self.type_solver = Some(TypeSolver::Unmetered(solver));
        self
    }

    /// Charge host proof work against this invocation, without granting the
    /// callback authority to replenish its execution budget.
    pub fn with_budgeted_type_solver(
        mut self,
        solver: &'a dyn Fn(&nir::program::TypePredicate, &mut u64) -> ContractOutcome,
    ) -> Self {
        self.type_solver = Some(TypeSolver::Budgeted(solver));
        self
    }

    fn solve_type_predicate(&mut self, predicate: &nir::program::TypePredicate) -> ContractOutcome {
        match &self.type_solver {
            Some(TypeSolver::Unmetered(solver)) => solver(predicate),
            Some(TypeSolver::Budgeted(solver)) => {
                if self.budget.steps == 0 {
                    return ContractOutcome::Unknown;
                }
                let mut remaining = self.budget.steps;
                let result = solver(predicate, &mut remaining);
                self.budget.steps = self.budget.steps.min(remaining);
                result
            }
            None => ContractOutcome::Unknown,
        }
    }

    /// Host-checked Copy delivery, including representation and Drop legality.
    /// Raw interpreters retain intrinsic-only pointer reads and replacement.
    pub fn with_pointer_copy_permission(mut self, permission: &'a dyn Fn(&Term) -> bool) -> Self {
        self.pointer_copy = Some(PointerCopyPermission::Checked(permission));
        self
    }

    /// Acquire resource evidence using this invocation's remaining execution
    /// fuel. Proof work is charged before the transfer or its continuation.
    pub fn with_budgeted_pointer_copy_permission(
        mut self,
        permission: &'a dyn Fn(&Term, &mut u64) -> bool,
    ) -> Self {
        self.pointer_copy = Some(PointerCopyPermission::Budgeted(permission));
        self
    }

    fn permits_pointer_copy(&mut self, ty: &Term) -> bool {
        if nir::program::intrinsic_pointer_value(ty) {
            return true;
        }
        match &self.pointer_copy {
            Some(PointerCopyPermission::Checked(permission)) => permission(ty),
            Some(PointerCopyPermission::Budgeted(permission)) => {
                let mut remaining = self.budget.steps;
                let allowed = permission(ty, &mut remaining);
                self.budget.steps = self.budget.steps.min(remaining);
                allowed
            }
            None => false,
        }
    }

    /// Permit opaque Type inputs belonging to this normalization scope. This
    /// does not grant layout, pattern discrimination or normal-value export.
    pub fn with_abstract_type_parameters(
        mut self,
        parameters: &[DeclarationId],
    ) -> Result<Self, Error> {
        self.authorize_abstract_type_parameters(parameters)?;
        Ok(self)
    }

    /// Extend scope atomically while keeping failed preparation costs visible.
    pub fn authorize_abstract_type_parameters(
        &mut self,
        parameters: &[DeclarationId],
    ) -> Result<(), Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        self.integer_operation(None, |budget| budget.consume(parameters.len()))?;
        if parameters
            .iter()
            .any(|id| self.abstract_value_parameters.contains_key(id))
        {
            return Err(self.failure(ErrorKind::Type, None));
        }
        self.abstract_type_parameters
            .extend(parameters.iter().copied());
        Ok(())
    }

    /// Grant explicit declaration-domain evidence for opaque comptime data.
    /// These values may be transported but cannot authorize a branch decision
    /// or a completed snapshot. No authority is inferred from an input value.
    pub fn with_abstract_value_parameters(
        mut self,
        parameters: &[(DeclarationId, Term)],
    ) -> Result<Self, Error> {
        self.authorize_abstract_value_parameters(parameters)?;
        Ok(self)
    }

    /// Validate the entire batch before publishing any new value authority.
    /// Errors retain consumed fuel but leave the previous scope unchanged.
    pub fn authorize_abstract_value_parameters(
        &mut self,
        parameters: &[(DeclarationId, Term)],
    ) -> Result<(), Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        self.integer_operation(None, |budget| budget.consume(parameters.len()))?;
        let mut additions = BTreeMap::new();
        for (declaration, ty) in parameters {
            let work = self
                .type_description_work(ty)
                .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
            self.integer_operation(None, |budget| budget.consume(work))?;
            if self.abstract_type_parameters.contains(declaration)
                || *ty == Term::Primitive(Primitive::Type)
                || self
                    .abstract_value_parameters
                    .get(declaration)
                    .is_some_and(|old| old != ty)
                || additions.get(declaration).is_some_and(|old| *old != ty)
            {
                return Err(self.failure(ErrorKind::Type, None));
            }
            additions.insert(*declaration, ty);
        }
        self.abstract_value_parameters
            .extend(additions.into_iter().map(|(id, ty)| (id, ty.clone())));
        Ok(())
    }
    fn type_description_work(&self, ty: &Term) -> Option<usize> {
        scoped_type_value_work_with_evidence_in(
            ty,
            &self.normal_forms,
            &self.abstract_type_parameters,
            &self.abstract_value_parameters,
            |ty, variant, fuel| self.normal_value_fields(ty, variant, fuel),
            self.target,
        )
    }
    fn normal_value_fields(
        &self,
        ty: &Term,
        variant: Option<u32>,
        fuel: &mut usize,
    ) -> Option<Vec<Term>> {
        use nir::normal_forms::instantiate_value_fields;
        nir::type_computations::charge_term(ty, fuel).ok()?;
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return None;
        };
        if let Some(graph) = self.graph {
            for nominal in &graph.nominals {
                nir::type_computations::charge_term(&nominal.ty, fuel).ok()?;
                if nominal.ty != *ty {
                    continue;
                }
                return match (&nominal.definition, variant) {
                    (iir::NominalDefinition::Enum(definition), Some(index)) => {
                        instantiate_value_fields(
                            &[],
                            &[],
                            &[],
                            definition.variants.get(index as usize)?.fields.iter(),
                            fuel,
                        )
                    }
                    (iir::NominalDefinition::Newtype(definition), None) => {
                        instantiate_value_fields(
                            &[],
                            &[],
                            &[],
                            std::iter::once(&definition.representation),
                            fuel,
                        )
                    }
                    (iir::NominalDefinition::Struct(definition), None) => instantiate_value_fields(
                        &[],
                        &[],
                        &[],
                        definition.fields.iter().map(|(_, ty)| ty),
                        fuel,
                    ),
                    _ => None,
                };
            }
            return None;
        }
        if let Some(index) = variant {
            let definition = self.enums.get(declaration)?;
            instantiate_value_fields(
                &definition.type_parameters,
                &definition.parameter_types,
                arguments,
                definition.variants.get(index as usize)?.fields.iter(),
                fuel,
            )
        } else if let Some(definition) = self.newtypes.get(declaration) {
            Some(vec![
                definition
                    .representation_budgeted(arguments, &[], fuel)
                    .ok()?,
            ])
        } else {
            let definition = self.structs.get(declaration)?;
            instantiate_value_fields(
                &definition.type_parameters,
                &definition.parameter_types,
                arguments,
                definition.fields.iter().map(|(_, ty)| ty),
                fuel,
            )
        }
    }
    fn type_description_term(&self, value: Value, source: SourceRange) -> Result<Term, Error> {
        match value {
            Value::Type(ty) => Ok(ty),
            Value::TypeParameter(declaration)
                if self.abstract_type_parameters.contains(&declaration) =>
            {
                Ok(Term::Rigid {
                    declaration,
                    sort: nir::Sort::Type,
                })
            }
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }
    pub fn events(&self) -> &[Event] {
        &self.events
    }
    /// Account for host-side argument preparation against this invocation's
    /// budget. Hosts can consume fuel here but cannot replenish it.
    pub fn charge_preparation(&mut self, work: usize) -> Result<(), Error> {
        let work =
            u64::try_from(work).map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
        self.budget.steps = self
            .budget
            .steps
            .checked_sub(work)
            .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, None))?;
        Ok(())
    }
    pub fn remaining_steps(&self) -> u64 {
        self.budget.steps
    }
    fn charge_copy_work(
        &mut self,
        source: Option<SourceRange>,
        check: impl FnOnce(&mut usize) -> Result<(), nir::copy_budget::CopyBudgetExhausted>,
    ) -> Result<(), Error> {
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = check(&mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        result.map_err(|reason| {
            self.failure(
                match reason {
                    nir::copy_budget::CopyBudgetExhausted::Steps => ErrorKind::StepsExhausted,
                    nir::copy_budget::CopyBudgetExhausted::Depth => ErrorKind::DepthExhausted,
                },
                source,
            )
        })
    }
    fn copy_invocation_condition(
        &mut self,
        condition: &Expr,
        definition: &nir::program::Function,
        function: &Value,
    ) -> Result<Expr, Error> {
        let arguments = match function {
            Value::Closure(closure) if self.graph.is_none() => {
                closure.instance_arguments.as_deref()
            }
            _ => None,
        };
        let binders = if arguments.is_some() {
            self.input_binders(&definition.type_parameters)
        } else {
            Vec::new()
        };
        self.charge_copy_work(Some(condition.source), |fuel| {
            nir::copy_budget::charge_substitution(
                condition,
                &binders,
                arguments.unwrap_or(&[]),
                fuel,
            )
        })?;
        match arguments {
            Some(arguments) => condition
                .instantiate_parameters(&binders, arguments)
                .map_err(|_| self.failure(ErrorKind::Type, Some(condition.source))),
            None => Ok(condition.clone()),
        }
    }
    pub fn invoke(
        &mut self,
        function: DeclarationId,
        arguments: Vec<Value>,
    ) -> Result<Value, Error> {
        if self.graph.is_some() {
            return self.invoke_instance(
                &iir::InstanceKey {
                    declaration: function,
                    arguments: vec![],
                },
                arguments,
            );
        }
        self.invoke_value(Value::Function(function), arguments)
    }
    /// Invoke explicit declaration inputs through the same specialization and
    /// contract path used by source FunctionInstance expressions. No binder or
    /// obligation is erased to make a template look monomorphic.
    pub fn invoke_instantiated(
        &mut self,
        function: DeclarationId,
        types: Vec<Term>,
        arguments: Vec<Value>,
    ) -> Result<Value, Error> {
        if self.graph.is_some() {
            return self.invoke_instance(
                &iir::InstanceKey {
                    declaration: function,
                    arguments: types,
                },
                arguments,
            );
        }
        self.tick(None)?;
        for ty in &types {
            let work = scoped_term_value_work_with_evidence_in(
                ty,
                &self.normal_forms,
                &self.abstract_type_parameters,
                &self.abstract_value_parameters,
                |ty, variant, fuel| self.normal_value_fields(ty, variant, fuel),
                self.target,
            )
            .ok_or_else(|| self.failure(ErrorKind::Type, None))? as u64;
            self.budget.steps = self
                .budget
                .steps
                .checked_sub(work)
                .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, None))?;
        }

        let definition = *self
            .functions
            .get(&function)
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
        let binders = self.input_binders(&definition.type_parameters);
        if binders.len() != types.len() {
            return Err(self.failure(ErrorKind::Type, None));
        }
        self.charge_copy_work(None, |fuel| {
            nir::copy_budget::charge_type_substitution(
                &definition.signature,
                &binders,
                &types,
                fuel,
            )
        })?;
        let signature = definition
            .signature
            .instantiate_parameters(&self.input_binders(&definition.type_parameters), &types)
            .map_err(|_| self.failure(ErrorKind::Type, None))?
            .with_comptime_phase(definition.phase == nir::program::FunctionPhase::Comptime)
            .with_unsafe_call_requirement(definition.is_unsafe);
        let expression = Expr {
            source: definition.body.source,
            ty: signature,
            kind: K::FunctionInstance {
                declaration: function,
                arguments: types,
            },
        };
        let value = match self.eval_function_instance(&expression, &mut Environment::new())? {
            Flow::Value(value) => value,
            _ => return Err(self.failure(ErrorKind::InvalidControlFlow, Some(expression.source))),
        };
        self.invoke_value(value, arguments)
    }

    fn concrete_value(value: Value, source: SourceRange) -> Result<Value, Error> {
        if matches!(value, Value::AbstractValue { .. }) {
            Err(Error {
                kind: ErrorKind::AbstractValueUnresolved,
                source: Some(source),
            })
        } else {
            Ok(value)
        }
    }
    fn failure(&self, kind: ErrorKind, source: Option<SourceRange>) -> Error {
        Error { kind, source }
    }
    fn tick(&mut self, source: Option<SourceRange>) -> Result<(), Error> {
        if self.budget.steps == 0 {
            return Err(self.failure(ErrorKind::StepsExhausted, source));
        }
        self.budget.steps -= 1;
        if self.depth >= self.budget.depth {
            return Err(self.failure(ErrorKind::DepthExhausted, source));
        }
        Ok(())
    }
    fn invoke_value(&mut self, function: Value, arguments: Vec<Value>) -> Result<Value, Error> {
        self.invoke_value_scoped(function, arguments, None)
    }
    fn invoke_value_scoped(
        &mut self,
        function: Value,
        arguments: Vec<Value>,
        context: Option<(iir::InstanceKey, Vec<iir::cleanup::ContextKey>)>,
    ) -> Result<Value, Error> {
        if matches!(
            function,
            Value::RuntimeFunction { .. } | Value::RuntimeTraitMethod(_)
        ) {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        let origin = match &function {
            Value::Function(id) => Some(*id),
            Value::Closure(closure) => closure.origin,
            _ => None,
        };
        if origin
            .and_then(|id| self.functions.get(&id))
            .is_some_and(|f| f.phase == nir::program::FunctionPhase::NormalForm)
        {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        if self.comptime_only {
            let declared_comptime = |id| {
                self.invocation_definition(id, &function)
                    .is_some_and(|f| f.phase == nir::program::FunctionPhase::Comptime)
            };
            let authorized = match &function {
                Value::Function(id) => declared_comptime(*id),
                Value::Closure(closure) => {
                    closure.ty.is_comptime_callable()
                        && closure.origin.is_none_or(declared_comptime)
                }
                _ => false,
            };
            if !authorized {
                return Err(self.failure(ErrorKind::PhaseMismatch, None));
            }
        }
        if self.cleanup_schedule.is_some() && context.is_none() {
            let named_instance = match &function {
                Value::Function(declaration) => Some((*declaration, &[][..])),
                Value::Closure(closure) => closure.origin.map(|declaration| {
                    (
                        declaration,
                        closure.instance_arguments.as_deref().unwrap_or(&[]),
                    )
                }),
                _ => None,
            };
            if let Some((declaration, arguments)) = named_instance {
                for key in &self.cleanup_schedule.unwrap().borrowed_callable_functions {
                    self.tick(None)?;
                    if key.declaration == declaration && key.arguments == arguments {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                    }
                }
            }
            if let Some(declaration) = origin {
                let definition = self
                    .invocation_definition(declaration, &function)
                    .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
                if matches!(definition.signature.callable_signature(), Term::Function {parameters,..} if parameters.iter().any(|p|matches!(p.ty.callable_signature(),Term::Function {..})))
                {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                }
            }
        }
        self.tick(None)?;
        self.effect_charge(self.effect_state.environment.len(), None)?;
        let next_effect_identity = self
            .effect_state
            .next_identity
            .checked_add(1)
            .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, None))?;
        let previous_effect_invocation = self.effect_state.invocation;
        let previous_effect_resume = self.effect_state.resume.take();
        let previous_effect_environment = self.effect_state.environment.clone();
        self.effect_state.next_identity = next_effect_identity;
        self.effect_state.invocation = next_effect_identity;
        if self.depth == 0 {
            self.cleanup_failed = false;
            self.effect_state.transfer = None;
        }
        self.depth += 1;
        let previous_cleanup_context = std::mem::take(&mut self.cleanup_context);
        let previous_residuals = std::mem::take(&mut self.residual_temporaries);
        let previous_pattern_orders = std::mem::take(&mut self.pattern_binding_orders);
        let previous_prepared_frames = std::mem::take(&mut self.prepared_frames);
        let previous_cleanup_frame = self.cleanup_frame.take();
        self.cleanup_frame = if self.cleanup_schedule.is_some() {
            origin.map(|declaration| iir::InstanceKey {
                declaration,
                arguments: match &function {
                    Value::Closure(closure) => {
                        closure.instance_arguments.clone().unwrap_or_default()
                    }
                    _ => vec![],
                },
            })
        } else {
            None
        };
        if let Some((instance, context)) = context {
            self.cleanup_frame = Some(instance);
            self.cleanup_context = context;
        }
        let result = (|| {
            let (parameters, mut environment, body, signature) = match &function {
                Value::Function(id) => {
                    let definition = *self
                        .functions
                        .get(id)
                        .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
                    if !definition.type_parameters.is_empty()
                        || (!self.comptime_only
                            && (!definition.requirements.is_empty()
                                || !definition.capabilities.is_empty()))
                    {
                        return Err(self.failure(ErrorKind::Type, None));
                    }
                    (
                        definition.parameters.as_slice(),
                        Environment::with_capacity(definition.parameters.len()),
                        &definition.body,
                        &definition.signature,
                    )
                }
                Value::Closure(closure) => {
                    let signature = match &closure.ty {
                        Term::Closure { signature, .. } => signature.as_ref(),
                        ty => ty,
                    };
                    (
                        closure.parameters.as_slice(),
                        self.clone_invocation_environment(
                            &closure.environment,
                            closure.body.source,
                        )?,
                        closure.body.as_ref(),
                        signature,
                    )
                }
                _ => return Err(self.failure(ErrorKind::Type, None)),
            };
            let Term::Function {
                parameters: types,
                result: result_type,
            } = signature.callable_signature()
            else {
                return Err(self.failure(ErrorKind::Type, None));
            };
            let type_scope = self.bind_invocation_inputs(
                parameters,
                arguments,
                signature,
                &mut environment,
                body.source,
            )?;
            if self.comptime_only {
                if let Some(id) = origin {
                    let definition = self
                        .invocation_definition(id, &function)
                        .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
                    let generic_arguments = match &function {
                        _ if self.graph.is_some() => &[][..],
                        Value::Closure(closure) => {
                            closure.instance_arguments.as_deref().unwrap_or(&[])
                        }
                        _ => &[],
                    };
                    for requirement in &definition.requirements {
                        let binders = self.input_binders(&definition.type_parameters);
                        self.charge_copy_work(Some(body.source), |fuel| {
                            nir::copy_budget::charge_predicate_substitution(
                                requirement,
                                &binders,
                                generic_arguments,
                                fuel,
                            )
                        })?;
                        let requirement = requirement
                            .instantiate_parameters(
                                &self.input_binders(&definition.type_parameters),
                                generic_arguments,
                            )
                            .map_err(|_| self.failure(ErrorKind::Type, Some(body.source)))?;
                        let requirement = self
                            .bind_invocation_contract(
                                &requirement,
                                type_scope.as_ref(),
                                body.source,
                                0,
                            )?
                            .ok_or_else(|| {
                                self.failure(ErrorKind::TypeContractUnresolved, Some(body.source))
                            })?;
                        self.check_type_contract(&requirement, body.source)?;
                    }
                    self.check_deferred_capabilities(
                        definition,
                        generic_arguments,
                        type_scope.as_ref(),
                        body.source,
                    )?;
                }
            }
            let result_type = match &type_scope {
                Some(scope) => scope.rebind(result_type),
                None => result_type.substitute_partial(&vec![None; types.len()]),
            }
            .map_err(|_| self.failure(ErrorKind::Type, None))?;
            result_type
                .check_scope(&[])
                .map_err(|_| self.failure(ErrorKind::Type, None))?;
            if let Term::Computation { effects, .. } = &result_type {
                self.check_effect_environment(effects, None)?;
            } else {
                self.effect_state.environment.clear();
            }
            let specialized_body = if let Some(scope) = &type_scope {
                let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                let before = fuel;
                let checked = iir::visit::expression(body, &mut fuel, |_, _| Ok(()));
                self.budget.steps -= (before - fuel) as u64;
                checked.map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(body.source)))?;
                let mut body = body.clone();
                self.specialize_input_types(&mut body, scope)?;
                Some(body)
            } else {
                None
            };
            let body = specialized_body.as_ref().unwrap_or(body);
            let origin = match &function {
                Value::Function(id) => Some(*id),
                Value::Closure(closure) => closure.origin,
                _ => None,
            };
            if let Some(id) = origin {
                let definition = self
                    .invocation_definition(id, &function)
                    .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
                for snapshot in &definition.entry_snapshots {
                    let value = self.local_value(&environment, snapshot.parameter, None)?;
                    environment.insert(snapshot.binding, value);
                }
                for condition in &definition.preconditions {
                    let mut condition =
                        self.copy_invocation_condition(condition, definition, &function)?;
                    if let Some(scope) = &type_scope {
                        self.specialize_input_types(&mut condition, scope)?;
                    }
                    let check = Expr {
                        source: condition.source,
                        ty: Term::Primitive(Primitive::Void),
                        kind: K::CheckPredicate(Box::new(condition)),
                    };
                    match self.eval(&check, &mut environment)? {
                        Flow::Value(Value::Unit) => {}
                        _ => {
                            return Err(
                                self.failure(ErrorKind::InvalidControlFlow, Some(check.source))
                            );
                        }
                    }
                }
                self.events.push(Event::EnterFunction(id));
            }
            let (value, returned) = match self.eval(body, &mut environment)? {
                Flow::Value(value) => (value, false),
                Flow::Return(value) => (value, true),
                _ => return Err(self.failure(ErrorKind::InvalidControlFlow, Some(body.source))),
            };
            let delivered_type = if let Term::Computation { result, .. } = &result_type {
                result.as_ref()
            } else {
                &result_type
            };
            if !self.has_type(&value, delivered_type) {
                return Err(self.failure(ErrorKind::Type, Some(body.source)));
            }
            if self.cleanup_schedule.is_some() {
                if !returned {
                    self.check_scheduled_result(&value, body.source, &mut environment)?;
                }
            } else if let Some(id) = origin {
                let definition = self
                    .invocation_definition(id, &function)
                    .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
                if !definition.postconditions.is_empty()
                    && let Some(binding) = definition.result_binding
                {
                    environment.insert(binding, value.clone());
                }
                for condition in &definition.postconditions {
                    let mut condition =
                        self.copy_invocation_condition(condition, definition, &function)?;
                    if let Some(scope) = &type_scope {
                        self.specialize_input_types(&mut condition, scope)?;
                    }
                    let check = Expr {
                        source: condition.source,
                        ty: Term::Primitive(Primitive::Void),
                        kind: K::CheckPredicate(Box::new(condition)),
                    };
                    match self.eval(&check, &mut environment)? {
                        Flow::Value(Value::Unit) => {}
                        _ => {
                            return Err(
                                self.failure(ErrorKind::InvalidControlFlow, Some(check.source))
                            );
                        }
                    }
                }
            }
            if !returned {
                self.cleanup_site(
                    body.source,
                    iir::cleanup::Boundary::FunctionEnd,
                    &mut environment,
                )?;
            }
            Ok(value)
        })();
        self.effect_state.invocation = previous_effect_invocation;
        self.effect_state.resume = previous_effect_resume;
        self.effect_state.environment = previous_effect_environment;
        self.cleanup_frame = previous_cleanup_frame;
        self.prepared_frames = previous_prepared_frames;
        self.residual_temporaries = previous_residuals;
        self.pattern_binding_orders = previous_pattern_orders;
        self.cleanup_context = previous_cleanup_context;
        self.depth -= 1;
        result
    }
    // Finish binding before recursive body evaluation so its temporary values
    // do not accumulate in every invocation frame on small host stacks.
    #[inline(never)]
    fn bind_invocation_inputs(
        &mut self,
        parameters: &[LocalId],
        arguments: Vec<Value>,
        signature: &Term,
        environment: &mut Environment,
        source: SourceRange,
    ) -> Result<Option<nir::FunctionInputBinding>, Error> {
        let Term::Function {
            parameters: types, ..
        } = signature.callable_signature()
        else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        if parameters.len() != arguments.len() || types.len() != arguments.len() {
            return Err(self.failure(ErrorKind::Arity, Some(source)));
        }
        let mut type_inputs = vec![None; types.len()];
        let mut type_scope: Option<nir::FunctionInputBinding> = None;
        for (index, ((binding, argument), parameter)) in
            parameters.iter().zip(arguments).zip(types).enumerate()
        {
            let representation = match &argument {
                Value::Storage(storage) => self.read_owned_value(storage)?,
                value => self.checkpoint_representation(value, source)?,
            };
            let expected = match &type_scope {
                Some(scope) => scope.rebind(&parameter.ty),
                None => parameter.ty.substitute_partial(&type_inputs),
            }
            .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            expected
                .check_scope(&[])
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            if !self.has_type(&representation, &expected) {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            }
            if self.comptime_only {
                self.admit_embedded_type_descriptions(&representation, 0)?;
            }
            if parameter.sort == nir::Sort::Type
                || (parameter.sort == nir::Sort::ComptimeValue
                    && expected == Term::Primitive(Primitive::Type))
            {
                let ty = match &representation {
                    Value::Type(ty) => ty.clone(),
                    Value::TypeParameter(declaration)
                        if self.abstract_type_parameters.contains(declaration) =>
                    {
                        Term::Rigid {
                            declaration: *declaration,
                            sort: nir::Sort::Type,
                        }
                    }
                    _ => return Err(self.failure(ErrorKind::Type, Some(source))),
                };
                type_inputs[index] = Some(ty);
                type_scope = Some(
                    signature
                        .prepare_function_inputs(&type_inputs)
                        .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?,
                );
            } else if parameter.sort == nir::Sort::ComptimeValue
                && self.normal_input_is_data(&representation, source)?
            {
                // Checked data inputs may remain symbolic in a generic
                // invocation. Preserve their authorized declaration identity
                // in dependent inputs, results and body annotations.
                type_inputs[index] = Some(self.residual_input_term(representation, source)?);
                type_scope = Some(
                    signature
                        .prepare_function_inputs(&type_inputs)
                        .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?,
                );
            }
            environment.insert(*binding, argument);
        }
        Ok(type_scope)
    }

    fn specialize_input_types(
        &mut self,
        expression: &mut Expr,
        scope: &nir::FunctionInputBinding,
    ) -> Result<(), Error> {
        let source = expression.source;
        expression.rewrite_types(&mut |ty| {
            self.tick(Some(source))?;
            let ty = scope
                .rebind(ty)
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            ty.check_scope(&[])
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            Ok(ty)
        })
    }
    fn has_type(&self, value: &Value, ty: &Term) -> bool {
        self.has_type_at(value, ty, 0)
    }
    fn has_type_at(&self, value: &Value, ty: &Term, depth: usize) -> bool {
        if let (Some(graph), Value::RuntimeTraitMethod(identity)) = (self.graph, value) {
            return graph
                .template_method_type(
                    &identity.interface,
                    &identity.subject,
                    identity.member,
                    &identity.method_arguments,
                )
                .is_some_and(|logical| logical == ty);
        }
        if let (
            Some(graph),
            Value::RuntimeFunction {
                declaration,
                arguments,
            },
        ) = (self.graph, value)
        {
            return graph.templates.iter().any(|table| {
                table.logical_type() == ty
                    && table
                        .targets()
                        .iter()
                        .any(|key| key.declaration == *declaration && &key.arguments == arguments)
            });
        }
        if let Value::AbstractValue {
            declaration,
            ty: domain,
        } = value
        {
            return self.comptime_only
                && domain == ty
                && self.abstract_value_parameters.get(declaration) == Some(domain);
        }

        if depth >= 128 {
            return false;
        }
        if let Value::Optional { ty: actual, value } = value {
            let Term::Optional(payload) = ty else {
                return false;
            };
            return actual == ty
                && value
                    .as_ref()
                    .is_none_or(|value| self.has_type_at(value, payload, depth + 1));
        }
        if let Value::ErrorResult {
            ty: actual,
            error,
            value,
        } = value
        {
            if actual != ty {
                return false;
            }
            let Term::ErrorResult { errors, result } = ty else {
                return false;
            };
            let payload = match error {
                None => result.as_ref(),
                Some(index) => {
                    let Some(label) = errors.get(*index) else {
                        return false;
                    };
                    label
                }
            };
            return self.has_type_at(value, payload, depth + 1);
        }
        if let Value::Newtype { ty: actual, value } = value {
            if actual != ty {
                return false;
            }
            let Term::Nominal {
                declaration,
                arguments,
            } = ty
            else {
                return false;
            };
            if let Some(graph) = self.graph {
                return graph
                    .nominals
                    .iter()
                    .find(|n| n.ty == *ty)
                    .is_some_and(|n| {
                        if let iir::NominalDefinition::Newtype(d) = &n.definition {
                            self.has_type_at(value, &d.representation, depth + 1)
                        } else {
                            false
                        }
                    });
            }
            return self.newtypes.get(declaration).is_some_and(|d| {
                d.validate_completed_arguments_with_evidence_in(
                    arguments,
                    &mut 100_000,
                    |ty, variant, fuel| self.admitted_nominal_layout(ty, variant, fuel),
                    |value, domain, _| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if self.abstract_value_parameters.get(declaration)==Some(domain)), self.target)
                .is_ok()
                    && d.parameter_binders().is_ok_and(|parameters| {
                        self.capabilities_hold_parameters(&d.capabilities, &parameters, arguments)
                    })
                    && d.representation_budgeted(arguments, &[], &mut 100_000)
                        .is_ok_and(|representation| {
                            self.has_type_at(value, &representation, depth + 1)
                        })
            });
        }
        if matches!(ty, Term::AssociatedType { .. }) {
            return selected_types::view(ty, &self.associated_values)
                .is_ok_and(|ty| self.has_type_at(value, &ty, depth + 1));
        }
        if matches!(ty, Term::Nominal { .. }) && !self.associated_values.is_empty() {
            let Ok(selected) = selected_types::view(ty, &self.associated_values) else {
                return false;
            };
            if selected != *ty {
                return self.has_type_at(value, &selected, depth + 1);
            }
        }
        if let Some(graph) = self.graph {
            if let Term::Nominal {
                declaration,
                arguments,
            } = ty
            {
                let Some(nominal) = graph.nominals.iter().find(|n| &n.ty == ty) else {
                    return false;
                };
                return match (value, &nominal.definition) {
                    (
                        Value::Record {
                            declaration: actual,
                            arguments: args,
                            fields,
                        },
                        iir::NominalDefinition::Struct(s),
                    ) => {
                        actual == declaration
                            && args == arguments
                            && fields.len() == s.fields.len()
                            && fields
                                .iter()
                                .zip(&s.fields)
                                .all(|(v, (_, t))| self.has_type_at(v, t, depth + 1))
                    }
                    (
                        Value::Variant {
                            declaration: actual,
                            arguments: args,
                            index,
                            fields,
                        },
                        iir::NominalDefinition::Enum(e),
                    ) => {
                        actual == declaration
                            && args == arguments
                            && e.variants.get(*index).is_some_and(|variant| {
                                fields.len() == variant.fields.len()
                                    && fields
                                        .iter()
                                        .zip(&variant.fields)
                                        .all(|(v, t)| self.has_type_at(v, t, depth + 1))
                            })
                    }
                    _ => false,
                };
            }
        }
        match (value, ty) {
            (Value::Pointer(pointer), Term::Pointer(inner)) => {
                selected_types::same(&pointer.pointee, inner, &self.associated_values)
            }
            (
                Value::Variant {
                    declaration,
                    arguments: actual_arguments,
                    index,
                    fields,
                },
                Term::Nominal {
                    declaration: expected,
                    arguments,
                },
            ) => {
                declaration == expected
                    && actual_arguments == arguments
                    && self
                        .enums
                        .get(declaration)
                        .filter(|definition| {
                            nir::validate_declaration_arguments_with_evidence_in(
                                &definition.type_parameters,
                                &definition.parameter_types,
                                arguments,
                                &mut 100_000,
                                |ty, variant, fuel| self.admitted_nominal_layout(ty, variant, fuel),
                    |value, domain, _| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if self.abstract_value_parameters.get(declaration)==Some(domain)), self.target)
                            .is_ok()
                                && definition.parameter_binders().is_ok_and(|parameters| {
                                    self.capabilities_hold_parameters(
                                        &definition.capabilities,
                                        &parameters,
                                        arguments,
                                    )
                                })
                        })
                        .and_then(|definition| {
                            nir::normal_forms::instantiate_value_fields(
                                &definition.type_parameters,
                                &definition.parameter_types,
                                arguments,
                                definition.variants.get(*index)?.fields.iter(),
                                &mut 100_000,
                            )
                        })
                        .is_some_and(|types| {
                            fields.len() == types.len()
                                && fields
                                    .iter()
                                    .zip(&types)
                                    .all(|(value, ty)| self.has_type_at(value, ty, depth + 1))
                        })
            }
            (Value::BitVec(value), Term::Primitive(Primitive::BitVec(width))) => {
                value.width() == *width
            }
            (Value::Object(object), Term::Primitive(Primitive::Object)) => {
                self.comptime_only && object.entries().iter().all(|entry| {
                    let typed = entry.value();
                    self.type_description_work(&typed.ty).is_some()
                        && self.has_type_at(&typed.value, &typed.ty, depth + 1)
                })
            }
            (Value::List { ty: actual, values }, ty) => {
                self.comptime_only && actual == ty && values.len() <= 100_000
                    && self.list_element(ty).is_some_and(|element|
                        element != &Term::Primitive(Primitive::Type)
                        && values.iter().all(|value| self.has_type_at(value, element, depth + 1)))
            }
            (Value::TypeList(elements), ty) => {
                self.is_type_list_input(ty)
                    && elements.len() <= 100_000
                    && elements
                        .iter()
                        .all(|element| self.type_description_work(element).is_some())
            }
            (Value::TypeParameter(id), Term::Primitive(Primitive::Type)) => {
                self.abstract_type_parameters.contains(id)
            }
            (Value::Type(value), Term::Primitive(Primitive::Type)) => {
                self.type_description_work(value).is_some()
            }
            (Value::Float(_), Term::Primitive(Primitive::Float))
            | (Value::Integer(_), Term::Primitive(Primitive::Integer))
            | (Value::Float32(_), Term::Primitive(Primitive::F32))
            | (Value::Float64(_), Term::Primitive(Primitive::F64))
            | (Value::Unit, Term::Primitive(Primitive::Void))
            | (Value::Bool(_), Term::Primitive(Primitive::Bool))
            | (Value::Char(_), Term::Primitive(Primitive::Char))
            | (Value::String(_), Term::Primitive(Primitive::Str))
            | (Value::Symbol(_), Term::Primitive(Primitive::Symbol)) => true,
            (Value::SignedInteger(value), Term::Primitive(primitive)) => self
                .integer_layout(*primitive)
                .is_some_and(|(signed, width)| {
                    signed
                        && comptime_runtime::machine::MachineInteger::from_signed(
                            *value,
                            width as u16,
                        )
                        .is_ok()
                }),
            (Value::UnsignedInteger(value), Term::Primitive(primitive)) => self
                .integer_layout(*primitive)
                .is_some_and(|(signed, width)| {
                    !signed
                        && comptime_runtime::machine::MachineInteger::from_unsigned(
                            *value,
                            width as u16,
                        )
                        .is_ok()
                }),
            (Value::AnonymousRecord(values), Term::Record(types)) => {
                let mut names = std::collections::BTreeSet::new();
                values.len() == types.len()
                    && values
                        .iter()
                        .zip(types)
                        .all(|((name, value), (expected, ty))| {
                            name == expected
                                && names.insert(name)
                                && self.has_type_at(value, ty, depth + 1)
                        })
            }
            (Value::Tuple(values), ty @ Term::NormalForm { head, .. }) => self
                .normal_forms
                .get(head)
                .and_then(|metadata| metadata.tuple_elements(ty))
                .is_some_and(|types| {
                    values.len() == types.len()
                        && values
                            .iter()
                            .zip(types)
                            .all(|(value, ty)| self.has_type_at(value, ty, depth + 1))
                }),
            (Value::Tuple(values), Term::Tuple(types)) => {
                values.len() == types.len()
                    && values
                        .iter()
                        .zip(types)
                        .all(|(v, t)| self.has_type_at(v, t, depth + 1))
            }
            (
                Value::Record {
                    declaration,
                    arguments: actual_arguments,
                    fields,
                },
                Term::Nominal {
                    declaration: expected,
                    arguments,
                },
            ) if declaration == expected && actual_arguments == arguments => {
                self.structs.get(declaration).is_some_and(|s| {
                    if nir::validate_declaration_arguments_with_evidence_in(
                        &s.type_parameters,
                        &s.parameter_types,
                        arguments,
                        &mut 100_000,
                        |ty, variant, fuel| self.admitted_nominal_layout(ty, variant, fuel),
                    |value, domain, _| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if self.abstract_value_parameters.get(declaration)==Some(domain)), self.target)
                    .is_err()
                        || !s.parameter_binders().is_ok_and(|parameters| {
                            self.capabilities_hold_parameters(
                                &s.capabilities,
                                &parameters,
                                arguments,
                            )
                        })
                    {
                        return false;
                    }
                    nir::normal_forms::instantiate_value_fields(
                        &s.type_parameters,
                        &s.parameter_types,
                        arguments,
                        s.fields.iter().map(|(_, ty)| ty),
                        &mut 100_000,
                    )
                    .is_some_and(|types| {
                        types.len() == fields.len()
                            && types
                                .iter()
                                .zip(fields)
                                .all(|(ty, value)| self.has_type_at(value, ty, depth + 1))
                    })
                })
            }
            (Value::Function(id), ty @ (Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_))) => {
                self.functions.get(id).is_some_and(|f| {
                    f.type_parameters.is_empty()
                        && f.phase != nir::program::FunctionPhase::NormalForm
                        && (self.comptime_only
                            || (f.requirements.is_empty() && f.capabilities.is_empty()))
                        && (f.phase == nir::program::FunctionPhase::Comptime) == ty.is_comptime_callable()
                        && f.is_unsafe == ty.is_unsafe_callable()
                        && selected_types::same(
                            &f.signature,
                            ty.callable_signature(),
                            &self.associated_values,
                        )
                })
            }
            (
                Value::RuntimeFunction {
                    declaration,
                    arguments,
                },
                ty,
            ) => self
                .runtime_function_identities
                .get(declaration)
                .is_some_and(|identity| {
                    self.validate_function_inputs(
                        &identity.type_parameters,
                        arguments,
                        &mut 100_000,
                    )
                    .is_ok()
                        && identity
                            .signature
                            .instantiate_parameters(
                                &self.input_binders(&identity.type_parameters),
                                arguments,
                            )
                            .is_ok_and(|signature| {
                                selected_types::same(&signature, ty, &self.associated_values)
                            })
                }),
            (Value::RuntimeTraitMethod(identity), ty) => {
                let TraitMethodIdentity {
                    interface,
                    subject,
                    member,
                    method_arguments,
                } = identity.as_ref();
                self.method_bodies
                    .contains_key(&(interface.clone(), subject.clone(), *member))
                    && self
                        .method_contracts
                        .get(&(interface.declaration, *member))
                        .is_some_and(|(parameters, contract)| {
                            let arguments: Vec<_> = std::iter::once(subject.clone())
                                .chain(interface.arguments.iter().cloned())
                                .chain(method_arguments.iter().cloned())
                                .collect();
                            let ids: Vec<_> = parameters
                                .iter()
                                .map(|parameter| parameter.declaration)
                                .collect();
                            self.validate_function_inputs(&ids, &arguments, &mut 100_000)
                                .is_ok()
                                && !contract.signature.is_comptime_callable()
                                && (contract.program.preconditions.is_empty()
                                    || (self.comptime_only && matches!(contract.signature.callable_signature(), Ok(Term::Function { parameters, .. })
                                        if parameters.iter().any(|p| p.sort != nir::Sort::Runtime))))
                                && contract
                                    .signature
                                    .instantiate_parameters(parameters, &arguments)
                                    .is_ok_and(|signature| {
                                        signature.ordinary().is_ok_and(|signature| {
                                            selected_types::same(
                                                signature,
                                                ty,
                                                &self.associated_values,
                                            )
                                        })
                                    })
                        })
            }
            (Value::Closure(closure), ty) => {
                selected_types::same(&closure.ty, ty, &self.associated_values)
            }
            _ => false,
        }
    }
    fn clone_invocation_environment(
        &mut self,
        environment: &Environment,
        source: SourceRange,
    ) -> Result<Environment, Error> {
        self.charge_preparation(environment.len())?;
        environment
            .iter()
            .map(|(id, value)| {
                self.checkpoint_representation(value, source)
                    .map(|value| (*id, value))
            })
            .collect()
    }

    fn local_value(
        &mut self,
        environment: &Environment,
        id: LocalId,
        source: Option<SourceRange>,
    ) -> Result<Value, Error> {
        let location = source.unwrap_or(SourceRange { start: 0, end: 0 });
        match environment
            .get(&id)
            .ok_or_else(|| self.failure(ErrorKind::MissingBinding, source))?
        {
            Value::Storage(storage) => {
                // Read the representation, retaining shared nested identities.
                // Ownership is checked by the actual access/transfer operation;
                // a projection may legally read one field of a partial owner.
                let value = storage
                    .0
                    .try_lock()
                    .map_err(|_| self.failure(ErrorKind::InvalidPointer, source))?;
                self.checkpoint_representation(&value, location)
            }
            value => self.checkpoint_representation(value, location),
        }
    }
    fn eval_pointer(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        if let Some(result) = self.eval_list_append(expr, environment) {
            return result;
        }
        if nir::program::pointer_operation_error_in(expr, self.target).is_some() {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let result = match &expr.kind {
            K::AddressOf(place) => {
                let mut base = place.as_ref();
                let mut fields = vec![];
                loop {
                    match &base.kind {
                        K::Field {
                            value,
                            declaration,
                            index,
                        } => {
                            self.tick(Some(expr.source))?;
                            fields.push(memory::Projection::Record(*declaration, *index));
                            base = value;
                        }
                        K::AnonymousField { value, name, index } => {
                            self.tick(Some(expr.source))?;
                            fields.push(memory::Projection::AnonymousRecord(name.clone(), *index));
                            base = value;
                        }
                        _ => break,
                    }
                }
                fields.reverse();
                let K::Local(id) = base.kind else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let value = environment
                    .get_mut(&id)
                    .ok_or_else(|| self.failure(ErrorKind::MissingBinding, Some(expr.source)))?;
                if !matches!(value, Value::Storage(_)) {
                    *value = Value::Storage(Storage::new(value.clone()));
                }
                let Value::Storage(storage) = value else {
                    unreachable!()
                };
                let pointer = storage.pointer(place.ty.clone(), fields);
                let valid = pointer
                    .inspect(|target| self.has_type(target, &place.ty))
                    .map_err(|kind| self.failure(kind, Some(expr.source)))?;
                if !valid {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                Value::Pointer(pointer)
            }
            K::PointerAnonymousField {
                pointer,
                name,
                index,
            } => {
                let Value::Pointer(pointer) = value!(self.eval(pointer, environment)) else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let Term::Pointer(pointee) = &expr.ty else {
                    unreachable!()
                };
                Value::Pointer(
                    pointer
                        .project_anonymous(name, *index, (**pointee).clone())
                        .map_err(|kind| self.failure(kind, Some(expr.source)))?,
                )
            }
            K::PointerField {
                pointer,
                declaration,
                index,
            } => {
                let Value::Pointer(pointer) = value!(self.eval(pointer, environment)) else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let Term::Pointer(pointee) = &expr.ty else {
                    unreachable!()
                };
                Value::Pointer(
                    pointer
                        .project(*declaration, *index, (**pointee).clone())
                        .map_err(|kind| self.failure(kind, Some(expr.source)))?,
                )
            }
            K::ReadPointer(pointer) => {
                let Value::Pointer(pointer) = value!(self.eval(pointer, environment)) else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let immutable_list = self.comptime_only
                    && (self.list_element(&expr.ty).is_some() || self.is_type_list_input(&expr.ty));
                if pointer.pointee != expr.ty
                    || (!immutable_list && !self.permits_pointer_copy(&expr.ty))
                {
                    return Err(self.failure(ErrorKind::Unsupported, Some(expr.source)));
                }
                let value = pointer
                    .inspect(|value| self.copy_representation(value, expr.source, 0))
                    .map_err(|kind| self.failure(kind, Some(expr.source)))??;
                if immutable_list {
                    self.validate_immutable_input(&value, &expr.ty)?;
                } else if !self.has_type(&value, &expr.ty) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                value
            }
            K::WritePointer { pointer, value } => {
                let Value::Pointer(pointer) = value!(self.eval(pointer, environment)) else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let value = value!(self.eval(value, environment));
                let immutable_list = self.comptime_only
                    && (self.list_element(&pointer.pointee).is_some()
                        || self.is_type_list_input(&pointer.pointee));
                if !immutable_list && !self.permits_pointer_copy(&pointer.pointee) {
                    return Err(self.failure(ErrorKind::Unsupported, Some(expr.source)));
                }
                if immutable_list {
                    // Validate both representations before mutation. Admission
                    // is budgeted and cannot invoke user code or follow pointers.
                    self.validate_immutable_input(&value, &pointer.pointee)?;
                    pointer
                        .inspect(|old| self.validate_immutable_input(old, &pointer.pointee))
                        .map_err(|kind| self.failure(kind, Some(expr.source)))??;
                } else if !self.has_type(&value, &pointer.pointee) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                pointer
                    .write(value, |old| self.has_type(old, &pointer.pointee))
                    .map_err(|kind| self.failure(kind, Some(expr.source)))?;
                Value::Unit
            }
            _ => unreachable!(),
        };
        Ok(Flow::Value(result))
    }
    fn eval(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        self.eval_in_position(expr, environment, false)
    }
    fn eval_in_position(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
        direct_callee: bool,
    ) -> Result<Flow, Error> {
        self.tick(Some(expr.source))?;
        self.depth += 1;
        let result = if self.graph.is_some()
            && matches!(
                expr.kind,
                K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. }
            ) {
            self.eval_iir_callable(expr)
        } else if direct_callee && matches!(expr.kind, K::TraitMethod { .. }) {
            self.eval_trait_method(expr, environment, true)
        } else {
            self.eval_inner(expr, environment)
        };
        self.depth -= 1;
        self.finish_evaluation(expr, environment, result)
    }
    // Mapped computation evaluation must use the same delivery/control hooks
    // as ordinary evaluation. Transparent effect wrappers share their child's
    // source identity and must not deliver or retire that value a second time.
    fn finish_evaluation(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
        result: Result<Flow, Error>,
    ) -> Result<Flow, Error> {
        // Most comptime evaluation has no scheduled runtime cleanup. Avoid
        // dispatching both delivery hooks per instruction in that mode; effect
        // transfers still need their scope handler even without a schedule.
        if self.cleanup_schedule.is_none()
            && !matches!(&result, Err(error) if error.kind == ErrorKind::EffectControlTransfer)
        {
            return result;
        }
        let mut result = if matches!(&result,Err(error) if error.kind==ErrorKind::EffectControlTransfer)
            && self.effect_transfer_targets_scope()?
        {
            let flow = self.effect_state.transfer.take().unwrap().1;
            match &flow {
                Flow::Return(value) => {
                    self.check_scheduled_result(value, expr.source, environment)?;
                    self.cleanup_site(
                        expr.source,
                        iir::cleanup::Boundary::EffectReturn,
                        environment,
                    )?;
                }
                Flow::Break(id) => self.cleanup_site(
                    expr.source,
                    iir::cleanup::Boundary::EffectBreak(*id),
                    environment,
                )?,
                Flow::Continue(id) => self.cleanup_site(
                    expr.source,
                    iir::cleanup::Boundary::EffectContinue(*id),
                    environment,
                )?,
                _ => {}
            }
            Ok(flow)
        } else {
            result
        };
        if matches!(&result, Ok(Flow::Value(_)))
            && !matches!(
                expr.kind,
                K::Block { .. }
                    | K::CallPreparation { .. }
                    | K::Return(_)
                    | K::EffectComputation { .. }
                    | K::EffectRun { .. }
            )
        {
            if let Ok(Flow::Value(value)) = &mut result {
                self.cleanup_delivery(expr, environment, value)?;
            }
            if !expr.shares_discard_temporary_boundary() {
                self.cleanup_site(
                    expr.source,
                    iir::cleanup::Boundary::TemporaryEnd,
                    environment,
                )?;
            }
        }
        result
    }
    fn capabilities_hold_parameters(
        &self,
        requirements: &[nir::program::TraitRequirement],
        parameters: &[nir::DeclarationParameter],
        arguments: &[Term],
    ) -> bool {
        requirements.iter().all(|requirement| {
            requirement
                .instantiate_parameters(parameters, arguments)
                .is_ok_and(|mut requirement| {
                    if self.assumed_interfaces.contains(&(
                        requirement.interface_instance(),
                        requirement.subject.clone(),
                    )) {
                        return true;
                    }
                    let Ok(subject) =
                        selected_types::view(&requirement.subject, &self.associated_values)
                    else {
                        return false;
                    };
                    requirement.subject = subject;
                    for argument in &mut requirement.interface_arguments {
                        let Ok(selected) = selected_types::view(argument, &self.associated_values)
                        else {
                            return false;
                        };
                        *argument = selected;
                    }
                    self.implemented_interfaces.contains_key(&(
                        requirement.interface_instance(),
                        requirement.subject.clone(),
                    )) || self
                        .assumed_interfaces
                        .contains(&(requirement.interface_instance(), requirement.subject))
                })
        })
    }

    fn integer_operation<T>(
        &mut self,
        source: Option<SourceRange>,
        operation: impl FnOnce(&mut comptime_runtime::Budget) -> Result<T, comptime_runtime::Error>,
    ) -> Result<T, Error> {
        let steps = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut budget = comptime_runtime::Budget {
            steps,
            max_limbs: 4096,
        };
        let result = operation(&mut budget);
        self.budget.steps -= (steps - budget.steps) as u64;
        result.map_err(|error| {
            self.failure(
                match error {
                    comptime_runtime::Error::BudgetExhausted => ErrorKind::StepsExhausted,
                    comptime_runtime::Error::SizeLimit => ErrorKind::RepresentationSizeLimit,
                    comptime_runtime::Error::DivisionByZero => ErrorKind::DivisionByZero,
                    _ => ErrorKind::Type,
                },
                source,
            )
        })
    }
    fn integer_literal(
        &mut self,
        spelling: &str,
        source: Option<SourceRange>,
    ) -> Result<comptime_runtime::Integer, Error> {
        self.integer_operation(source, |budget| {
            let (negative, digits) = spelling
                .strip_prefix('-')
                .map_or((false, spelling), |s| (true, s));
            let value = comptime_runtime::Integer::parse_literal(digits, budget)?;
            if negative {
                budget.consume(value.bit_len().div_ceil(32).saturating_add(1))?;
                Ok(value.negated())
            } else {
                Ok(value)
            }
        })
    }
    fn bit_operation<T>(
        &mut self,
        source: SourceRange,
        operation: impl FnOnce(
            &mut comptime_runtime::Budget,
        ) -> Result<T, comptime_runtime::bitvec::Error>,
    ) -> Result<T, Error> {
        let steps = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut budget = comptime_runtime::Budget {
            steps,
            max_limbs: 4096,
        };
        let result = operation(&mut budget);
        self.budget.steps -= (steps - budget.steps) as u64;
        result.map_err(|error| {
            self.failure(
                match error {
                    comptime_runtime::bitvec::Error::Resource(
                        comptime_runtime::Error::BudgetExhausted,
                    ) => ErrorKind::StepsExhausted,
                    comptime_runtime::bitvec::Error::Resource(
                        comptime_runtime::Error::SizeLimit,
                    ) => ErrorKind::RepresentationSizeLimit,
                    _ => ErrorKind::Type,
                },
                Some(source),
            )
        })
    }
    // Keep the recursive dispatcher small. A monolithic debug-build match
    // reserves stack for temporaries from every arm at each recursive level,
    // exhausting the host stack before the logical depth budget can fire.
    fn eval_inner(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        match &expr.kind {
            K::OptionalPresent(_) | K::OptionalAbsent => self.eval_optional(expr, environment),
            K::EffectResume(_)
            | K::EffectHandle { .. }
            | K::EffectRun { .. }
            | K::EffectComputation { .. }
            | K::EffectRequest { .. } => self.eval_effect(expr, environment),
            K::Unit | K::Local(_) | K::Function(_) | K::Break(_) | K::Continue(_) => {
                self.eval_leaf(expr, environment)
            }
            K::ComptimeValue(..) | K::TypeValue(..) | K::TypeListValue(..) => {
                self.eval_type_value(expr, environment)
            }
            K::NormalFormType { .. } => self.eval_normal_form_type(expr, environment),
            K::FunctionType { .. } | K::FunctionResultType { .. } => {
                self.eval_function_type(expr, environment)
            }
            K::OptionalType(..) | K::PointerType(..) | K::ClosureType { .. } => {
                self.eval_environment_type(expr, environment)
            }
            K::NominalType { .. } => self.eval_nominal_type(expr, environment),
            K::CheckTypeAlias { .. } => self.eval_nominal_type(expr, environment),
            K::ScalarEqual(..)
            | K::ScalarLessEqual(..)
            | K::ScalarLess(..)
            | K::ScalarGreater(..)
            | K::ScalarGreaterEqual(..) => self.eval_scalar_comparison(expr, environment),
            K::TypeEqual(..) => self.eval_type_equal(expr, environment),
            K::TemplateEqual(..) => self.eval_template_equal(expr, environment),
            K::ListLength(_) | K::ListConcat(..) => self.eval_list_operation(expr, environment),
            K::BuildStruct(request) => self.eval_struct_build(expr, request, environment),
            K::BuildEnum(request) => self.eval_enum_build(expr, request, environment),
            K::EnumStructure { .. } => self.eval_enum_structure(expr, environment),
            K::ComptimeFunctionInstance { .. } => {
                self.eval_comptime_function_instance(expr, environment)
            }
            K::ObjectEdit { .. } => self.eval_object_edit(expr, environment),
            K::ObjectQuery { .. } => self.eval_object_query(expr, environment),
            K::StructFieldTypes(_) | K::StructFieldNames(_) => {
                self.eval_struct_fields(expr, environment)
            }
            K::TypeListIndex(..) => self.eval_type_list_index(expr, environment),
            K::TupleType(..) => self.eval_tuple_type(expr, environment),
            K::TypeList(..) => self.eval_type_list(expr, environment),
            K::BitVecType(..) => self.eval_bit_vec_type(expr, environment),
            K::Literal(..) => self.eval_literal(expr, environment),
            K::BuildMetadata {
                owner,
                selector,
                entries,
            } => {
                if !self.comptime_only {
                    return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
                }
                let Value::String(alias) = value!(self.eval(selector, environment)) else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                for (name, provider, value) in entries {
                    self.integer_operation(Some(expr.source), |budget| {
                        budget.consume(name.len().min(alias.len()).saturating_add(1))
                    })?;
                    if name == alias.as_ref() {
                        let value = value.as_ref().ok_or_else(|| {
                            self.failure(
                                ErrorKind::BuildMetadataUnavailable {
                                    owner: *owner,
                                    provider: Some(*provider),
                                    alias: alias.to_string(),
                                },
                                Some(expr.source),
                            )
                        })?;
                        return self
                            .decode_immutable_input(value, &expr.ty)
                            .map(Flow::Value)
                            .map_err(|mut error| {
                                error.source = Some(expr.source);
                                error
                            });
                    }
                }
                Err(self.failure(
                    ErrorKind::BuildMetadataUnavailable {
                        owner: *owner,
                        provider: None,
                        alias: alias.to_string(),
                    },
                    Some(expr.source),
                ))
            }
            K::Variant { .. } => self.eval_variant(expr, environment),
            K::Record { .. } | K::AnonymousRecord { .. } => self.eval_record(expr, environment),
            K::Object { .. } => self.eval_object(expr, environment),
            K::Field { .. } | K::AnonymousField { .. } => self.eval_field(expr, environment),
            K::AddressOf(_)
            | K::ReadPointer(_)
            | K::PointerAnonymousField { .. }
            | K::PointerField { .. }
            | K::WritePointer { .. } => self.eval_pointer(expr, environment),
            K::TraitMethod { .. } => self.eval_trait_method(expr, environment, false),
            K::AssociatedValue { .. } => self.eval_associated_value(expr),
            K::FunctionInstance { .. } => self.eval_function_instance(expr, environment),
            K::Closure { .. } => self.eval_closure(expr, environment),
            K::Tuple(..) => self.eval_tuple(expr, environment),
            K::Match { .. } => self.eval_match(expr, environment),
            K::Block { .. } | K::CallPreparation { .. } => self.eval_block(expr, environment),
            K::LetPattern { .. } => self.eval_let_pattern(expr, environment),
            K::BitWritePointer { .. } => self.eval_bit_write_pointer(expr, environment),
            K::BitAssign { .. } | K::BitUpdate { .. } => self.eval_bit_assign(expr, environment),
            K::Assign { .. } => self.eval_assign(expr, environment),
            K::Loop { .. } => self.eval_loop(expr, environment),
            K::Let { .. } => self.eval_let(expr, environment),
            K::RequirePredicate(..) | K::EnsurePredicate(..) | K::CheckPredicate(..) => {
                self.eval_check_predicate(expr, environment)
            }
            K::Intrinsic { .. } => Err(self.failure(ErrorKind::Unsupported, Some(expr.source))),
            K::UnsafeScope(body) => self.eval(body, environment),
            K::Discard(..) => self.eval_discard(expr, environment),
            K::Return(..) => self.eval_return(expr, environment),
            K::If { .. } => self.eval_if(expr, environment),
            K::BitSlice { .. } | K::BitIndex(..) | K::BitWindow { .. } => {
                self.eval_bit_slice(expr, environment)
            }
            K::BitEqual(..) => self.eval_bit_equal(expr, environment),
            K::FloatNegate(..) | K::FloatBinary { .. } => {
                self.eval_float_arithmetic(expr, environment)
            }
            K::IntegerNegate(..) => self.eval_integer_negate(expr, environment),
            K::IntegerBinary { .. } => self.eval_integer_binary(expr, environment),
            K::BitConvert(..) => self.eval_bit_convert(expr, environment),
            K::ErrorConvert { .. } => self.eval_error_convert(expr, environment),
            K::NewtypeConvert(..) => self.eval_newtype_convert(expr, environment),
            K::BitConcat(..) => self.eval_bit_concat(expr, environment),
            K::EnumName(operand) => {
                if expr.ty != Term::Primitive(Primitive::Str)
                    || !matches!(operand.ty, Term::Nominal { .. })
                {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let Term::Nominal { declaration, .. } = &operand.ty else {
                    unreachable!()
                };
                let is_enum = if let Some(graph) = self.graph {
                    graph.nominals.iter().any(|nominal| {
                        nominal.ty == operand.ty
                            && matches!(nominal.definition, iir::NominalDefinition::Enum(_))
                    })
                } else {
                    self.enums.contains_key(declaration)
                };
                if !is_enum {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let value = value!(self.eval(operand, environment));
                if !self.has_type(&value, &operand.ty) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                self.retain_residual(operand.source, &value)?;
                let Value::Variant {
                    declaration,
                    index,
                    arguments,
                    ..
                } = value
                else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let definition = if let Some(graph) = self.graph {
                    graph
                        .nominals
                        .iter()
                        .find(|nominal| {
                            matches!(&nominal.ty,
                        Term::Nominal { declaration: actual, arguments: actual_args }
                            if *actual == declaration && *actual_args == arguments)
                        })
                        .and_then(|nominal| match &nominal.definition {
                            iir::NominalDefinition::Enum(definition) => {
                                Some(generated_enums::EnumDefinition::Borrowed(definition))
                            }
                            _ => None,
                        })
                } else {
                    self.enums.get(&declaration).cloned()
                };
                let name = definition
                    .as_ref()
                    .and_then(|definition| definition.variants.get(index))
                    .map(|variant| variant.name.as_str())
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
                self.budget.steps = self
                    .budget
                    .steps
                    .checked_sub(name.len() as u64)
                    .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, Some(expr.source)))?;
                Ok(Flow::Value(Value::String(name.into())))
            }
            K::SymbolName(operand) => {
                let never = Term::Primitive(Primitive::Never);
                let expected = if operand.ty == never {
                    never.clone()
                } else {
                    Term::Primitive(Primitive::Str)
                };
                if expr.ty != expected
                    || (operand.ty != never && operand.ty != Term::Primitive(Primitive::Symbol))
                {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let Value::Symbol(name) = value!(self.eval(operand, environment)) else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                Ok(Flow::Value(Value::String(name)))
            }
            K::TextQuery { subject, query } => {
                if !query.validate(&subject.ty, &expr.ty) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let Value::Tuple(values) = value!(self.eval(subject, environment)) else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let text = match values.first() {
                    Some(Value::String(text)) if !query.is_symbol() => text,
                    Some(Value::Symbol(text)) if query.is_symbol() => text,
                    _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                };
                match query {
                    nir::program::TextQueryKind::ByteLength
                    | nir::program::TextQueryKind::SymbolByteLength
                        if values.len() == 1 =>
                    {
                        Ok(Flow::Value(Value::UnsignedInteger(text.len() as u128)))
                    }
                    nir::program::TextQueryKind::Byte | nir::program::TextQueryKind::SymbolByte
                        if values.len() == 2 =>
                    {
                        let Value::UnsignedInteger(index) = values[1] else {
                            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                        };
                        if index > u64::MAX as u128 {
                            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                        }
                        let byte = usize::try_from(index)
                            .ok()
                            .and_then(|index| text.as_bytes().get(index));
                        Ok(Flow::Value(Value::Optional {
                            ty: expr.ty.clone(),
                            value: byte
                                .map(|byte| Box::new(Value::UnsignedInteger(u128::from(*byte)))),
                        }))
                    }
                    _ => Err(self.failure(ErrorKind::Type, Some(expr.source))),
                }
            }
            K::Snapshot(value) => {
                if value.ty != expr.ty {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                self.eval(value, environment)
            }
            K::IntegerRepresentation(operand) => {
                if !nir::program::integer_representation_valid(&operand.ty, &expr.ty) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let value = value!(self.eval(operand, environment));
                let Term::Primitive(primitive) = operand.ty else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let (_, width) = self
                    .integer_layout(primitive)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
                if !self.has_type(&value, &operand.ty) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let bits = match value {
                    Value::UnsignedInteger(value) => value,
                    Value::SignedInteger(value) => value as u128,
                    _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                };
                Ok(Flow::Value(Value::UnsignedInteger(
                    bits & (u128::MAX >> (128 - width)),
                )))
            }
            K::TakeBits(..) => self.eval_take_bits(expr, environment),
            K::Not(..) => self.eval_not(expr, environment),
            K::And(..) | K::Or(..) => self.eval_and(expr, environment),
            K::Call { .. } => self.eval_call(expr, environment),
        }
    }
    #[inline(never)]
    fn eval_leaf(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::Unit => Value::Unit,
            K::Local(id) => self.local_value(environment, *id, Some(expr.source))?,
            K::Function(id) if self.runtime_function_identities.contains_key(id) => {
                Value::RuntimeFunction {
                    declaration: *id,
                    arguments: vec![],
                }
            }
            K::Function(id) => Value::Function(*id),
            K::Break(identity) => {
                self.cleanup_site(expr.source, iir::cleanup::Boundary::Break, environment)?;
                return Ok(Flow::Break(*identity));
            }
            K::Continue(identity) => {
                self.cleanup_site(expr.source, iir::cleanup::Boundary::Continue, environment)?;
                return Ok(Flow::Continue(*identity));
            }
            _ => unreachable!("expression dispatcher selected an incompatible leaf"),
        };
        Ok(Flow::Value(result))
    }
    #[inline(never)]
    fn eval_normal_form_type(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::NormalFormType {
            plan, arguments, ..
        } = &expr.kind
        else {
            unreachable!()
        };
        if let Some(plan) = plan {
            if !plan.valid(arguments.len())
                || plan
                    .local_bindings()
                    .unwrap()
                    .iter()
                    .any(|id| environment.contains_key(id))
            {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            }
        }
        let result = self.eval_normal_form_type_inner(expr, environment);
        if let Some(plan) = plan {
            for binding in plan.local_bindings().unwrap() {
                environment.remove(&binding);
            }
        }
        result
    }
    fn eval_normal_form_type_inner(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::NormalFormType {
            head,
            arguments,
            plan,
        } = &expr.kind
        else {
            unreachable!()
        };
        let metadata = *self
            .normal_forms
            .get(head)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
        let expected_result = if arguments
            .iter()
            .any(nir::program::NormalFormArgument::is_diverging)
            || plan
                .iter()
                .flat_map(|p| &p.preparations)
                .any(|p| p.expression.ty == Term::Primitive(Primitive::Never))
        {
            Primitive::Never
        } else {
            Primitive::Type
        };
        if expr.ty != Term::Primitive(expected_result)
            || arguments.len() != metadata.parameters.len()
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        self.integer_operation(Some(expr.source), |budget| budget.consume(arguments.len()))?;
        if plan.is_some() {
            nir::copy_budget::charge_type_rewrite(expr, &mut self.budget.steps).map_err(
                |error| {
                    self.failure(
                        match error {
                            nir::copy_budget::CopyBudgetExhausted::Steps => {
                                ErrorKind::StepsExhausted
                            }
                            nir::copy_budget::CopyBudgetExhausted::Depth => {
                                ErrorKind::DepthExhausted
                            }
                        },
                        Some(expr.source),
                    )
                },
            )?;
        }
        let mut values = vec![None; arguments.len()];
        let mut pending: Vec<
            Option<(
                Box<nir::program::NormalFormArgument>,
                normal_inputs::StagedNormalInput,
            )>,
        > = (0..arguments.len()).map(|_| None).collect();
        let mut pending_slots = BTreeSet::new();
        let mut type_parameters = Vec::new();
        let mut type_arguments = Vec::new();
        let order: Vec<_> = plan
            .as_ref()
            .map(|plan| plan.order.clone())
            .unwrap_or_else(|| (0..arguments.len()).collect());
        let mut preparations = plan.iter().flat_map(|p| &p.preparations).peekable();
        for (position, slot) in order.into_iter().enumerate() {
            while preparations.peek().is_some_and(|p| p.before == position) {
                let preparation = preparations.next().unwrap();
                value!(self.eval(&preparation.expression, environment));
            }
            let parameter = &metadata.parameters[slot];
            let mut instantiated = plan.as_ref().map(|_| Box::new(arguments[slot].clone()));
            if let Some(input) = &mut instantiated {
                self.specialize_contextual_input(input, &type_parameters, &type_arguments)?;
            }
            let argument = instantiated.as_deref().unwrap_or(&arguments[slot]);
            let expected = self.instantiate_contextual_domain(
                &parameter.ty,
                &type_parameters,
                &type_arguments,
                expr.source,
            )?;
            let unresolved = metadata
                .parameters
                .iter()
                .any(|p| expected.mentions_rigid(p.declaration));
            if unresolved && plan.is_some() {
                // Box before staging: contextual literal identities stay valid
                // while the pending input moves through this invocation.
                let argument = instantiated.take().unwrap();
                let staged = match self.stage_normal_argument(&argument, environment, 0)? {
                    std::ops::ControlFlow::Continue(value) => value,
                    std::ops::ControlFlow::Break(flow) => return Ok(flow),
                };
                pending[slot] = Some((argument, staged));
                pending_slots.insert(slot);
                continue;
            }
            let mut literals = BTreeMap::new();
            self.prepare_normal_argument(argument, &expected, &mut literals, 0)?;
            let value = value!(self.eval_normal_argument(argument, &mut literals, environment));
            if let Some(plan) = plan {
                environment.insert(plan.bindings[slot], value.clone());
            }
            let term = self.residual_input_term(value, argument.source())?;
            type_parameters.push(nir::DeclarationParameter {
                declaration: parameter.declaration,
                sort: if parameter.kind == nir::normal_forms::Kind::Type {
                    nir::Sort::Type
                } else {
                    nir::Sort::ComptimeValue
                },
            });
            type_arguments.push(term.clone());
            values[slot] = Some(term);
            for index in pending_slots.iter().copied().collect::<Vec<_>>() {
                self.tick(Some(expr.source))?;
                let expected = self.instantiate_contextual_domain(
                    &metadata.parameters[index].ty,
                    &type_parameters,
                    &type_arguments,
                    expr.source,
                )?;
                if metadata
                    .parameters
                    .iter()
                    .any(|p| expected.mentions_rigid(p.declaration))
                {
                    continue;
                }
                let (mut input, staged) = pending[index].take().unwrap();
                pending_slots.remove(&index);
                self.specialize_contextual_input(&mut input, &type_parameters, &type_arguments)?;
                let mut literals = BTreeMap::new();
                self.prepare_normal_argument(&input, &expected, &mut literals, 0)?;
                let value =
                    self.finish_staged_normal_argument(staged, &mut literals, input.source(), 0)?;
                environment.insert(plan.as_ref().unwrap().bindings[index], value.clone());
                let term = self.residual_input_term(value, input.source())?;
                type_parameters.push(nir::DeclarationParameter {
                    declaration: metadata.parameters[index].declaration,
                    sort: if metadata.parameters[index].kind == nir::normal_forms::Kind::Type {
                        nir::Sort::Type
                    } else {
                        nir::Sort::ComptimeValue
                    },
                });
                type_arguments.push(term.clone());
                values[index] = Some(term);
            }
        }
        for preparation in preparations {
            value!(self.eval(&preparation.expression, environment));
        }
        let arguments = values
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
        Ok(Flow::Value(self.complete_normal_type(
            *head,
            arguments,
            expr.source,
        )?))
    }

    fn contextual_tuple_type(
        &mut self,
        head: Option<DeclarationId>,
        elements: Vec<Term>,
        source: SourceRange,
    ) -> Result<Value, Error> {
        match head {
            Some(head) => {
                self.complete_normal_type(head, vec![Term::TypeListValue(elements)], source)
            }
            None => Ok(Value::Type(Term::Tuple(elements))),
        }
    }

    fn complete_normal_type(
        &mut self,
        head: DeclarationId,
        arguments: Vec<Term>,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let metadata = *self
            .normal_forms
            .get(&head)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        let ty = Term::NormalForm {
            head,
            sort: nir::Sort::Type,
            arguments,
        };
        let work = self
            .type_description_work(&ty)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        self.integer_operation(Some(source), |budget| budget.consume(work))?;
        let Term::NormalForm {
            arguments: values, ..
        } = &ty
        else {
            unreachable!()
        };
        let parameters: Vec<_> = metadata.parameters.iter().map(|p| p.declaration).collect();
        for requirement in &metadata.requirements {
            self.tick(Some(source))?;
            let predicate = requirement
                .instantiate_rigids(&parameters, values)
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            self.check_type_contract(&predicate, source)?;
        }
        Ok(Value::Type(ty))
    }

    /// Only data with a structural normal-input representation is frozen here.
    /// Callable/storage identities continue to use their ordinary environment
    /// binding; an aggregate containing one must not acquire a partial snapshot.
    fn normal_input_is_data(&mut self, value: &Value, source: SourceRange) -> Result<bool, Error> {
        let mut pending = vec![(value, 0usize)];
        while let Some((value, depth)) = pending.pop() {
            self.tick(Some(source))?;
            if depth > 128 {
                return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
            }
            let width = match value {
                Value::List { values, .. }
                | Value::Tuple(values)
                | Value::Record { fields: values, .. }
                | Value::Variant { fields: values, .. } => values.len(),
                Value::Newtype { .. } => 1,
                Value::AnonymousRecord(fields) => fields.len(),
                Value::AbstractValue { declaration, ty }
                    if self.abstract_value_parameters.get(declaration) == Some(ty) =>
                {
                    continue;
                }
                Value::Unit
                | Value::Bool(_)
                | Value::Char(_)
                | Value::String(_)
                | Value::Symbol(_)
                | Value::UnsignedInteger(_)
                | Value::SignedInteger(_)
                | Value::Float(_)
                | Value::Integer(_)
                | Value::Float32(_)
                | Value::Float64(_)
                | Value::BitVec(_)
                | Value::Type(_)
                | Value::TypeParameter(_)
                | Value::TypeList(_) => continue,
                _ => return Ok(false),
            };
            // Charge width before growing the traversal stack. Freezing checks
            // payload sizes, labels and completed embedded types separately.
            self.integer_operation(Some(source), |budget| budget.consume(width))?;
            match value {
                Value::List { values, .. }
                | Value::Tuple(values)
                | Value::Record { fields: values, .. }
                | Value::Variant { fields: values, .. } => {
                    pending.extend(values.iter().rev().map(|value| (value, depth + 1)))
                }
                Value::Newtype { value, .. } => pending.push((value, depth + 1)),
                Value::AnonymousRecord(fields) => {
                    pending.extend(fields.iter().rev().map(|(_, value)| (value, depth + 1)))
                }
                _ => unreachable!(),
            }
        }
        Ok(true)
    }

    // Type construction may retain authorized value binders during a generic
    // invocation. Completed input export keeps its separate closed rules.
    fn residual_input_term(&mut self, value: Value, source: SourceRange) -> Result<Term, Error> {
        let steps = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut budget = comptime_runtime::Budget {
            steps,
            max_limbs: 4096,
        };
        let term = value.into_abstract_invocation_term(
            &mut budget,
            &self.abstract_type_parameters,
            &self.abstract_value_parameters,
        );
        self.budget.steps -= (steps - budget.steps) as u64;
        term.map_err(|kind| self.failure(kind, Some(source)))
    }

    fn append_normal_list_item(
        &mut self,
        ty: &Term,
        types: &mut Vec<Term>,
        values: &mut Vec<Value>,
        spread: bool,
        value: Value,
        source: SourceRange,
    ) -> Result<(), Error> {
        let type_elements = self.is_type_list_input(ty);
        if spread {
            self.validate_immutable_input(&value, ty)?;
            match value {
                Value::TypeList(elements) if type_elements => {
                    self.integer_operation(Some(source), |budget| budget.consume(elements.len()))?;
                    if types.len().saturating_add(elements.len()) > 100_000 {
                        return Err(self.failure(ErrorKind::RepresentationSizeLimit, Some(source)));
                    }
                    types.extend(elements);
                }
                Value::List {
                    values: elements, ..
                } if !type_elements => {
                    self.integer_operation(Some(source), |budget| budget.consume(elements.len()))?;
                    if values.len().saturating_add(elements.len()) > 100_000 {
                        return Err(self.failure(ErrorKind::RepresentationSizeLimit, Some(source)));
                    }
                    values.extend(elements);
                }
                _ => return Err(self.failure(ErrorKind::Type, Some(source))),
            }
        } else if type_elements {
            types.push(self.type_description_term(value, source)?);
        } else {
            values.push(value);
        }
        if values.len().max(types.len()) > 100_000 {
            return Err(self.failure(ErrorKind::RepresentationSizeLimit, Some(source)));
        }
        Ok(())
    }

    fn finish_normal_list(
        &mut self,
        ty: Term,
        types: Vec<Term>,
        values: Vec<Value>,
    ) -> Result<Value, Error> {
        let value = if self.is_type_list_input(&ty) {
            Value::TypeList(types)
        } else {
            Value::List {
                ty: ty.clone(),
                values,
            }
        };
        self.validate_immutable_input(&value, &ty)?;
        Ok(value)
    }

    // The addresses are invocation-local cache identities only, never
    // dereferenced. Nodes are borrowed from immutable NIR throughout execution.
    fn prepare_normal_argument(
        &mut self,
        input: &nir::program::NormalFormArgument,
        expected: &Term,
        literals: &mut BTreeMap<usize, PreparedNormalInput>,
        depth: usize,
    ) -> Result<(), Error> {
        use nir::program::NormalFormArgument as A;
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(input.source())));
        }
        self.tick(Some(input.source()))?;
        match input {
            A::Unit { source } => {
                let value = match expected {
                    Term::Primitive(Primitive::Type) => {
                        Value::Type(Term::Primitive(Primitive::Void))
                    }
                    _ => {
                        self.check_type_contract(
                            &nir::program::TypePredicate::Equal(
                                Term::Primitive(Primitive::Void),
                                expected.clone(),
                            ),
                            *source,
                        )?;
                        Value::Unit
                    }
                };
                literals.insert(
                    input as *const A as usize,
                    PreparedNormalInput::Value(value),
                );
            }
            A::TypeListSpread(value) | A::Value(value) => {
                if value.ty != *expected && value.ty != Term::Primitive(Primitive::Never) {
                    self.check_type_contract(
                        &nir::program::TypePredicate::Equal(value.ty.clone(), expected.clone()),
                        value.source,
                    )?;
                }
            }
            A::Integer {
                source,
                spelling,
                negative,
            } => {
                let integer = self.integer_literal(spelling, Some(*source))?;
                let integer = if *negative {
                    self.integer_operation(Some(*source), |budget| integer.negate(budget))?
                } else {
                    integer
                };
                let value = match expected {
                    Term::Primitive(Primitive::Integer) => Value::Integer(integer),
                    Term::Primitive(Primitive::BitVec(width)) => Value::BitVec(
                        self.bit_operation(*source, |budget| integer.to_bitvec(*width, budget))?,
                    ),
                    Term::Primitive(primitive) if self.integer_layout(*primitive).is_some() => {
                        let (signed, bits) = self.integer_layout(*primitive).unwrap();
                        if signed {
                            Value::SignedInteger(
                                integer
                                    .to_signed(bits)
                                    .map_err(|_| self.failure(ErrorKind::Type, Some(*source)))?,
                            )
                        } else {
                            Value::UnsignedInteger(
                                integer
                                    .to_unsigned(bits)
                                    .map_err(|_| self.failure(ErrorKind::Type, Some(*source)))?,
                            )
                        }
                    }
                    _ => return Err(self.failure(ErrorKind::Type, Some(*source))),
                };
                literals.insert(
                    input as *const A as usize,
                    PreparedNormalInput::Value(value),
                );
            }
            A::If {
                condition,
                then_input,
                else_input,
                ..
            } => {
                if !matches!(
                    condition.ty,
                    Term::Primitive(Primitive::Bool | Primitive::Never)
                ) {
                    return Err(self.failure(ErrorKind::Type, Some(condition.source)));
                }
                self.prepare_normal_argument(then_input, expected, literals, depth + 1)?;
                self.prepare_normal_argument(else_input, expected, literals, depth + 1)?;
            }
            A::TypeList { source, items } => {
                if items.len() > 100_000 {
                    return Err(self.failure(ErrorKind::RepresentationSizeLimit, Some(*source)));
                }
                self.integer_operation(Some(*source), |budget| {
                    let before = budget.steps;
                    let mut fuel = before;
                    if nir::type_computations::charge_term(expected, &mut fuel).is_err() {
                        return budget.consume(before.saturating_add(1));
                    }
                    budget.consume(before - fuel)
                })?;
                let element = self
                    .list_element(expected)
                    .cloned()
                    .or_else(|| {
                        self.is_type_list_input(expected)
                            .then_some(Term::Primitive(Primitive::Type))
                    })
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(*source)))?;
                self.integer_operation(Some(*source), |budget| budget.consume(items.len()))?;
                literals.insert(
                    input as *const A as usize,
                    PreparedNormalInput::ListType(expected.clone()),
                );
                for item in items {
                    let element_type = if matches!(item, A::TypeListSpread(_)) {
                        expected
                    } else {
                        &element
                    };
                    self.prepare_normal_argument(item, element_type, literals, depth + 1)?;
                }
            }
            A::Tuple {
                source,
                type_head,
                items,
            } => {
                if *expected == Term::Primitive(Primitive::Type) {
                    for item in items {
                        self.prepare_normal_argument(item, expected, literals, depth + 1)?;
                    }
                    if let Some(head) = type_head {
                        if !self
                            .normal_forms
                            .get(head)
                            .is_some_and(|metadata| metadata.role == nir::normal_forms::Role::Tuple)
                        {
                            return Err(self.failure(ErrorKind::Type, Some(*source)));
                        }
                    }
                    literals.insert(
                        input as *const A as usize,
                        PreparedNormalInput::TypeTuple(*type_head),
                    );
                    return Ok(());
                }
                let Some(types) = self.tuple_input_elements(expected) else {
                    return Err(self.failure(ErrorKind::Type, Some(*source)));
                };
                if types.len() != items.len() {
                    return Err(self.failure(ErrorKind::Arity, Some(*source)));
                }
                for (item, ty) in items.iter().zip(types) {
                    self.prepare_normal_argument(item, ty, literals, depth + 1)?;
                }
            }
            A::Match { arms, .. } => {
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        if !matches!(
                            guard.ty,
                            Term::Primitive(Primitive::Bool | Primitive::Never)
                        ) {
                            return Err(self.failure(ErrorKind::Type, Some(guard.source)));
                        }
                    }
                    self.prepare_normal_argument(&arm.body, expected, literals, depth + 1)?;
                }
            }
            A::Block { result, .. } => {
                if let Some(result) = result {
                    self.prepare_normal_argument(result, expected, literals, depth + 1)?;
                } else if !input.is_diverging() {
                    self.check_type_contract(
                        &nir::program::TypePredicate::Equal(
                            Term::Primitive(Primitive::Void),
                            expected.clone(),
                        ),
                        input.source(),
                    )?;
                }
            }
        }
        Ok(())
    }

    #[inline(never)]
    fn eval_normal_argument(
        &mut self,
        input: &nir::program::NormalFormArgument,
        literals: &mut BTreeMap<usize, PreparedNormalInput>,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        use nir::program::NormalFormArgument as A;
        self.tick(Some(input.source()))?;
        self.depth += 1;
        let result = (|| match input {
            A::TypeListSpread(value) | A::Value(value) => self.eval(value, environment),
            A::Integer { .. } | A::Unit { .. } => literals
                .remove(&(input as *const A as usize))
                .and_then(|prepared| match prepared {
                    PreparedNormalInput::Value(value) => Some(Flow::Value(value)),
                    PreparedNormalInput::TypeTuple(_) | PreparedNormalInput::ListType(_) => None,
                })
                .ok_or_else(|| self.failure(ErrorKind::Type, Some(input.source()))),
            A::If {
                condition,
                then_input,
                else_input,
                ..
            } => {
                let Value::Bool(condition) = Self::concrete_value(
                    value!(self.eval(condition, environment)),
                    condition.source,
                )?
                else {
                    return Err(self.failure(ErrorKind::Type, Some(condition.source)));
                };
                self.eval_normal_argument(
                    if condition { then_input } else { else_input },
                    literals,
                    environment,
                )
            }
            A::TypeList { items, .. } => {
                let Some(PreparedNormalInput::ListType(ty)) =
                    literals.remove(&(input as *const A as usize))
                else {
                    return Err(self.failure(ErrorKind::Type, Some(input.source())));
                };
                let mut values = Vec::new();
                let mut types = Vec::new();
                for item in items {
                    let value = value!(self.eval_normal_argument(item, literals, environment));
                    self.append_normal_list_item(
                        &ty,
                        &mut types,
                        &mut values,
                        matches!(item, A::TypeListSpread(_)),
                        value,
                        item.source(),
                    )?;
                }
                self.finish_normal_list(ty, types, values).map(Flow::Value)
            }
            A::Tuple { items, .. } => {
                let mut values = Vec::new();
                for item in items {
                    values.push(value!(self.eval_normal_argument(
                        item,
                        literals,
                        environment
                    )));
                }
                if let Some(PreparedNormalInput::TypeTuple(head)) =
                    literals.remove(&(input as *const A as usize))
                {
                    let types = values
                        .into_iter()
                        .map(|value| self.type_description_term(value, input.source()))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(Flow::Value(self.contextual_tuple_type(
                        head,
                        types,
                        input.source(),
                    )?))
                } else {
                    Ok(Flow::Value(Value::Tuple(values)))
                }
            }
            A::Match {
                source,
                value,
                arms,
            } => {
                let value = value!(self.eval(value, environment));
                let existing = environment.snapshot();
                for arm in arms {
                    let flow = (|| {
                        self.tick(Some(*source))?;
                        match self.match_arm_guard(
                            &arm.pattern,
                            arm.guard.as_ref(),
                            &value,
                            environment,
                        )? {
                            Flow::Value(Value::Bool(false)) => return Ok(None),
                            Flow::Value(Value::Bool(true)) => {}
                            flow => return Ok(Some(flow)),
                        }
                        self.eval_normal_argument(&arm.body, literals, environment)
                            .map(Some)
                    })();
                    environment.retain(|id, _| existing.contains(id));
                    if let Some(flow) = flow? {
                        return Ok(flow);
                    }
                }
                Err(self.failure(ErrorKind::NonExhaustiveMatch, Some(*source)))
            }
            A::Block {
                statements, result, ..
            } => {
                let existing = environment.snapshot();
                let flow = (|| {
                    for statement in statements {
                        value!(self.eval(statement, environment));
                    }
                    if let Some(result) = result {
                        self.eval_normal_argument(result, literals, environment)
                    } else {
                        Ok(Flow::Value(Value::Unit))
                    }
                })();
                environment.retain(|id, _| existing.contains(id));
                flow
            }
        })();
        self.depth -= 1;
        if input.is_diverging() && matches!(&result, Ok(Flow::Value(_))) {
            return Err(self.failure(ErrorKind::Type, Some(input.source())));
        }
        result
    }

    fn check_type_contract(
        &mut self,
        predicate: &nir::program::TypePredicate,
        source: SourceRange,
    ) -> Result<(), Error> {
        let normalized;
        let predicate = if predicate.trivial_truth().is_none() && !self.associated_values.is_empty()
        {
            let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
            let result =
                selected_types::predicate(predicate, &self.associated_values, &mut fuel, 0);
            self.budget.steps = fuel as u64;
            normalized = result.map_err(|kind| self.failure(kind, Some(source)))?;
            &normalized
        } else {
            predicate
        };
        let outcome = match predicate.trivial_truth() {
            Some(true) => ContractOutcome::Satisfied,
            Some(false) => ContractOutcome::Rejected,
            None => self.solve_type_predicate(predicate),
        };
        match outcome {
            ContractOutcome::Satisfied => Ok(()),
            ContractOutcome::Rejected => {
                Err(self.failure(ErrorKind::TypeContractRejected, Some(source)))
            }
            ContractOutcome::Unknown => {
                Err(self.failure(ErrorKind::TypeContractUnresolved, Some(source)))
            }
        }
    }

    #[inline(never)]
    fn eval_type_value(
        &mut self,
        expr: &Expr,
        _environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::ComptimeValue(value) => {
                let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                let before = fuel;
                let checked =
                    nir::comptime_values::validate_in(value, &expr.ty, self.target, &mut fuel);
                self.budget.steps -= (before - fuel) as u64;
                checked.map_err(|error| {
                    self.failure(
                        if error == nir::comptime_values::Error::Budget {
                            ErrorKind::StepsExhausted
                        } else {
                            ErrorKind::Type
                        },
                        Some(expr.source),
                    )
                })?;
                self.normal_pattern_value(value, &expr.ty)?
            }
            K::TypeListValue(types) => {
                if !self.is_type_list_input(&expr.ty) || types.len() > 100_000 {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                for ty in types {
                    let work = self
                        .type_description_work(ty)
                        .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
                    self.integer_operation(Some(expr.source), |budget| budget.consume(work))?;
                }
                let value = Value::TypeList(types.clone());
                self.validate_immutable_input(&value, &expr.ty)?;
                value
            }
            K::TypeValue(ty) => {
                if expr.ty != Term::Primitive(Primitive::Type) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                if let Term::Rigid {
                    declaration,
                    sort: nir::Sort::Type,
                } = ty
                {
                    if self.abstract_type_parameters.contains(declaration) {
                        self.tick(Some(expr.source))?;
                        return Ok(Flow::Value(Value::TypeParameter(*declaration)));
                    }
                }
                // Structural scope permits transporting residual identities;
                // their nominal domains still need independent admission.
                if !self.abstract_value_parameters.is_empty() {
                    self.admit_type(ty, 0)?;
                }
                if let Some(work) = self.type_description_work(ty) {
                    self.integer_operation(Some(expr.source), |budget| budget.consume(work))?;
                    return Ok(Flow::Value(Value::Type(ty.clone())));
                }
                let ty = self.selected_type_view(ty, Some(expr.source))?;
                let work = self
                    .type_description_work(&ty)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
                self.integer_operation(Some(expr.source), |budget| budget.consume(work))?;
                Value::Type(ty.clone())
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_function_type(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let fields: Vec<&Expr> = match &expr.kind {
            K::FunctionType { parameters, .. } => parameters.iter().collect(),
            K::FunctionResultType { signature, result } => vec![signature, result],
            _ => unreachable!(),
        };
        let expected = if fields
            .iter()
            .any(|field| field.ty == Term::Primitive(Primitive::Never))
        {
            Primitive::Never
        } else {
            Primitive::Type
        };
        if expr.ty != Term::Primitive(expected)
            || fields.iter().any(|field| {
                !matches!(
                    field.ty,
                    Term::Primitive(Primitive::Type | Primitive::Never)
                )
            })
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        self.integer_operation(Some(expr.source), |budget| {
            budget.consume(fields.len().saturating_add(1))
        })?;
        let ty = match &expr.kind {
            K::FunctionType {
                comptime,
                comptime_parameters,
                parameters,
            } => {
                if comptime_parameters.len() != parameters.len() {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let mut types = Vec::with_capacity(parameters.len());
                for (parameter, staged) in parameters.iter().zip(comptime_parameters) {
                    let value = value!(self.eval(parameter, environment));
                    let ty = self.type_description_term(value, parameter.source)?;
                    types.push(nir::Parameter {
                        sort: if *staged {
                            if ty == Term::Primitive(Primitive::Type) {
                                nir::Sort::Type
                            } else {
                                nir::Sort::ComptimeValue
                            }
                        } else {
                            nir::Sort::Runtime
                        },
                        ty,
                    });
                }
                Term::Function {
                    parameters: types,
                    result: Box::new(Term::Primitive(Primitive::Void)),
                }
                .with_comptime_phase(*comptime)
            }
            K::FunctionResultType { signature, result } => {
                let value = value!(self.eval(signature, environment));
                let input = self.type_description_term(value, signature.source)?;
                if !matches!(input.callable_signature(), Term::Function { .. }) {
                    return Err(self.failure(ErrorKind::Type, Some(signature.source)));
                }
                let value = value!(self.eval(result, environment));
                let output = self.type_description_term(value, result.source)?;
                input
                    .with_callable_result(output)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(signature.source)))?
            }
            _ => unreachable!(),
        };
        Ok(Flow::Value(Value::Type(ty)))
    }

    #[inline(never)]
    fn eval_environment_type(
        &mut self,
        expr: &Expr,
        scope: &mut Environment,
    ) -> Result<Flow, Error> {
        let (fields, signature): (&[Expr], &Expr) = match &expr.kind {
            K::OptionalType(inner) | K::PointerType(inner) => (&[], inner),
            K::ClosureType {
                environment,
                signature,
            } => (environment, signature),
            _ => unreachable!(),
        };
        let expected = if fields
            .iter()
            .chain(std::iter::once(signature))
            .any(|field| field.ty == Term::Primitive(Primitive::Never))
        {
            Primitive::Never
        } else {
            Primitive::Type
        };
        if expr.ty != Term::Primitive(expected)
            || fields
                .iter()
                .chain(std::iter::once(signature))
                .any(|field| {
                    !matches!(
                        field.ty,
                        Term::Primitive(Primitive::Type | Primitive::Never)
                    )
                })
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        self.integer_operation(Some(expr.source), |budget| {
            budget.consume(fields.len().saturating_add(2))
        })?;
        let mut environment = Vec::with_capacity(fields.len());
        for field in fields {
            let value = value!(self.eval(field, scope));
            let ty = self.type_description_term(value, field.source)?;
            environment.push(ty);
        }
        let value = value!(self.eval(signature, scope));
        let body = self.type_description_term(value, signature.source)?;
        let ty = if matches!(expr.kind, K::OptionalType(_)) {
            Term::Optional(Box::new(body))
        } else if matches!(expr.kind, K::PointerType(_)) {
            Term::Pointer(Box::new(body))
        } else if environment.is_empty() {
            body
        } else {
            if !matches!(
                body,
                Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_)
            ) {
                return Err(self.failure(ErrorKind::Type, Some(signature.source)));
            }
            Term::Closure {
                environment,
                signature: Box::new(body),
            }
        };
        Ok(Flow::Value(Value::Type(ty)))
    }

    #[inline(never)]
    fn eval_nominal_type(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let (K::NominalType {
            arguments,
            preparations,
            ..
        }
        | K::CheckTypeAlias {
            arguments,
            preparations,
            ..
        }) = &expr.kind
        else {
            unreachable!()
        };
        self.integer_operation(Some(expr.source), |budget| {
            budget.consume(arguments.len().saturating_add(preparations.len()))
        })?;
        for preparation in preparations {
            if let K::LetPattern {
                pattern: nir::program::Pattern::Tuple(fields),
                ..
            } = &preparation.expression.kind
            {
                self.integer_operation(Some(expr.source), |budget| budget.consume(fields.len()))?;
            }
        }
        let locals = expr
            .kind
            .nominal_local_bindings()
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
        if locals.iter().any(|id| environment.contains_key(id)) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let result = self.eval_nominal_type_inner(expr, environment);
        for binding in locals {
            environment.remove(&binding);
        }
        result
    }

    fn instantiate_contextual_domain(
        &mut self,
        ty: &Term,
        parameters: &[nir::DeclarationParameter],
        types: &[Term],
        source: SourceRange,
    ) -> Result<Term, Error> {
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut remaining = before;
        let result = (|| {
            if parameters.len() != types.len() {
                return Err(nir::type_computations::Error::Scope(
                    nir::TypeError::ArgumentCount {
                        expected: parameters.len(),
                        actual: types.len(),
                    },
                ));
            }
            nir::type_computations::charge_term(ty, &mut remaining)?;
            remaining = remaining
                .checked_sub(parameters.len())
                .ok_or(nir::type_computations::Error::Budget)?;
            let referenced = ty
                .rigid_parameters()
                .map_err(nir::type_computations::Error::Scope)?;
            let mut selected_parameters = Vec::new();
            let mut selected_types = Vec::new();
            for (parameter, value) in parameters.iter().zip(types) {
                if referenced.contains_key(&parameter.declaration) {
                    nir::type_computations::charge_term(value, &mut remaining)?;
                    selected_parameters.push(parameter.clone());
                    selected_types.push(value.clone());
                }
            }
            nir::type_computations::instantiate_parameters_bounded(
                ty,
                &selected_parameters,
                &selected_types,
                &[],
                &mut remaining,
            )
        })();
        self.budget.steps -= (before - remaining) as u64;
        result.map_err(|error| {
            self.failure(
                if matches!(error, nir::type_computations::Error::Budget) {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                },
                Some(source),
            )
        })
    }

    fn specialize_contextual_input(
        &mut self,
        input: &mut nir::program::NormalFormArgument,
        parameters: &[nir::DeclarationParameter],
        types: &[Term],
    ) -> Result<(), Error> {
        let source = input.source();
        let mut rewrite =
            |ty: &Term| self.instantiate_contextual_domain(ty, parameters, types, source);
        for pattern in input.patterns_mut() {
            pattern.rewrite_types(&mut rewrite)?;
        }
        for expression in input.expressions_mut() {
            expression.rewrite_types(&mut rewrite)?;
        }
        Ok(())
    }

    fn eval_nominal_type_inner(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let (K::NominalType {
            preparations,
            bindings,
            order,
            declaration,
            arguments,
        }
        | K::CheckTypeAlias {
            preparations,
            bindings,
            order,
            declaration,
            arguments,
            ..
        }) = &expr.kind
        else {
            unreachable!()
        };
        let alias = if matches!(expr.kind, K::CheckTypeAlias { .. }) {
            Some(
                self.alias_contracts
                    .get(declaration)
                    .copied()
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?,
            )
        } else {
            None
        };
        let struct_definition = self.structs.get(declaration).cloned();
        let enum_definition = self.enums.get(declaration).cloned();
        let (parameters, domains, requirements, capabilities) = if let Some(def) = alias {
            (
                def.parameter_binders(),
                &def.parameter_types,
                &def.requirements,
                &def.capabilities,
            )
        } else if let Some(def) = struct_definition.as_ref() {
            (
                def.parameter_binders(),
                &def.parameter_types,
                &def.requirements,
                &def.capabilities,
            )
        } else if let Some(def) = enum_definition.as_ref() {
            (
                def.parameter_binders(),
                &def.parameter_types,
                &def.requirements,
                &def.capabilities,
            )
        } else if let Some(def) = self.newtypes.get(declaration).copied() {
            (
                def.parameter_binders(),
                &def.parameter_types,
                &def.requirements,
                &def.capabilities,
            )
        } else {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        };
        let parameters =
            parameters.map_err(|_| self.failure(ErrorKind::Type, Some(expr.source)))?;
        let expected = if arguments
            .iter()
            .any(nir::program::NormalFormArgument::is_diverging)
            || preparations
                .iter()
                .any(|p| p.expression.ty == Term::Primitive(Primitive::Never))
        {
            Primitive::Never
        } else if let K::CheckTypeAlias { body, .. } = &expr.kind {
            match body {
                None => Primitive::Void,
                Some(body) if body.ty == Term::Primitive(Primitive::Type) => Primitive::Type,
                Some(body) if body.ty == Term::Primitive(Primitive::Never) => Primitive::Never,
                _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
            }
        } else {
            Primitive::Type
        };
        if arguments.len() != parameters.len() || expr.ty != Term::Primitive(expected) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        self.integer_operation(Some(expr.source), |budget| {
            budget.consume(arguments.len().saturating_add(1))
        })?;
        if let Some(order) = order {
            self.integer_operation(Some(expr.source), |budget| budget.consume(order.len()))?;
            if order.len() != arguments.len() {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            }
            let mut seen = vec![false; arguments.len()];
            for &slot in order {
                if slot >= seen.len() || seen[slot] {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                seen[slot] = true;
            }
        }
        let specialize = bindings.is_some()
            || domains.iter().any(|domain| {
                parameters
                    .iter()
                    .any(|parameter| domain.mentions_rigid(parameter.declaration))
            });
        if specialize {
            for argument in arguments {
                nir::copy_budget::charge_argument_type_rewrite(argument, &mut self.budget.steps)
                    .map_err(|error| {
                        self.failure(
                            match error {
                                nir::copy_budget::CopyBudgetExhausted::Steps => {
                                    ErrorKind::StepsExhausted
                                }
                                nir::copy_budget::CopyBudgetExhausted::Depth => {
                                    ErrorKind::DepthExhausted
                                }
                            },
                            Some(expr.source),
                        )
                    })?;
            }
        }
        let mut specialized = specialize.then(|| arguments.clone());
        let mut types = parameters
            .iter()
            .map(|parameter| Term::Rigid {
                declaration: parameter.declaration,
                sort: parameter.sort,
            })
            .collect::<Vec<_>>();
        let mut ready = BTreeSet::new();
        let mut pending = (0..arguments.len()).map(|_| None).collect::<Vec<_>>();
        let mut pending_slots = BTreeSet::new();
        let mut preparations = preparations.iter().peekable();
        for position in 0..arguments.len() {
            while preparations.peek().is_some_and(|p| p.before == position) {
                value!(self.eval(&preparations.next().unwrap().expression, environment));
            }
            let slot = order.as_ref().map_or(position, |order| order[position]);
            if let Some(specialized) = &mut specialized {
                self.specialize_contextual_input(&mut specialized[slot], &parameters, &types)?;
            }
            let argument = specialized
                .as_ref()
                .map_or(&arguments[slot], |inputs| &inputs[slot]);
            if parameters[slot].sort == nir::Sort::Type
                && let nir::program::NormalFormArgument::Value(value) = argument
                && !matches!(
                    value.ty,
                    Term::Primitive(Primitive::Type | Primitive::Never)
                )
            {
                return Err(self.failure(ErrorKind::Type, Some(value.source)));
            }
            let domain = domains
                .get(slot)
                .unwrap_or(&Term::Primitive(Primitive::Type));
            let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
            let mut remaining = before;
            let expected = nir::type_computations::instantiate_parameters_bounded(
                domain,
                &parameters,
                &types,
                &[],
                &mut remaining,
            );
            self.budget.steps -= (before - remaining) as u64;
            let expected = expected.map_err(|error| {
                self.failure(
                    if matches!(error, nir::type_computations::Error::Budget) {
                        ErrorKind::StepsExhausted
                    } else {
                        ErrorKind::Type
                    },
                    Some(argument.source()),
                )
            })?;
            let unresolved = parameters.iter().enumerate().any(|(i, parameter)| {
                !ready.contains(&i) && expected.mentions_rigid(parameter.declaration)
            });
            if unresolved {
                pending_slots.insert(slot);
                pending[slot] = Some(
                    match self.stage_normal_argument(argument, environment, 0)? {
                        std::ops::ControlFlow::Continue(staged) => staged,
                        std::ops::ControlFlow::Break(flow) => return Ok(flow),
                    },
                );
            } else {
                if alias.is_some()
                    && let nir::program::NormalFormArgument::Value(value) = argument
                    && value.ty != expected
                    && value.ty != Term::Primitive(Primitive::Never)
                {
                    return Err(self.failure(ErrorKind::Type, Some(value.source)));
                }
                let mut literals = BTreeMap::new();
                self.prepare_normal_argument(argument, &expected, &mut literals, 0)?;
                let value = value!(self.eval_normal_argument(argument, &mut literals, environment));
                if let Some(bindings) = bindings {
                    let copy = self.checkpoint_representation(&value, argument.source())?;
                    environment.insert(bindings[slot], copy);
                }
                types[slot] = self.residual_input_term(value, argument.source())?;
                ready.insert(slot);
            }
            // Captures are stored after own inputs. Resolve pending inputs as
            // soon as their domains become available, without replaying effects.
            loop {
                let mut progress = false;
                self.integer_operation(Some(expr.source), |budget| {
                    budget.consume(pending_slots.len())
                })?;
                for index in pending_slots.iter().copied().collect::<Vec<_>>() {
                    let domain = domains
                        .get(index)
                        .unwrap_or(&Term::Primitive(Primitive::Type));
                    let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                    let mut remaining = before;
                    let expected = nir::type_computations::instantiate_parameters_bounded(
                        domain,
                        &parameters,
                        &types,
                        &[],
                        &mut remaining,
                    );
                    self.budget.steps -= (before - remaining) as u64;
                    let expected = expected.map_err(|error| {
                        self.failure(
                            if matches!(error, nir::type_computations::Error::Budget) {
                                ErrorKind::StepsExhausted
                            } else {
                                ErrorKind::Type
                            },
                            Some(arguments[index].source()),
                        )
                    })?;
                    if parameters.iter().enumerate().any(|(i, parameter)| {
                        !ready.contains(&i) && expected.mentions_rigid(parameter.declaration)
                    }) {
                        continue;
                    }
                    if let Some(specialized) = &mut specialized {
                        self.specialize_contextual_input(
                            &mut specialized[index],
                            &parameters,
                            &types,
                        )?;
                    }
                    let input = specialized
                        .as_ref()
                        .map_or(&arguments[index], |inputs| &inputs[index]);
                    if alias.is_some()
                        && let nir::program::NormalFormArgument::Value(value) = input
                        && value.ty != expected
                        && value.ty != Term::Primitive(Primitive::Never)
                    {
                        return Err(self.failure(ErrorKind::Type, Some(value.source)));
                    }
                    let mut literals = BTreeMap::new();
                    self.prepare_normal_argument(input, &expected, &mut literals, 0)?;
                    pending_slots.remove(&index);
                    let staged = pending[index].take().unwrap();
                    let value = self.finish_staged_normal_argument(
                        staged,
                        &mut literals,
                        input.source(),
                        0,
                    )?;
                    if let Some(bindings) = bindings {
                        let copy = self.checkpoint_representation(&value, input.source())?;
                        environment.insert(bindings[index], copy);
                    }
                    types[index] = self.residual_input_term(value, input.source())?;
                    ready.insert(index);
                    progress = true;
                }
                if !progress {
                    break;
                }
            }
        }
        for preparation in preparations {
            value!(self.eval(&preparation.expression, environment));
        }
        if ready.len() != arguments.len() {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut remaining = before;
        let ids = parameters.iter().map(|p| p.declaration).collect::<Vec<_>>();
        // Captured newtype binders follow their own capture-first admission
        // order; do not replace it with the ordinary declaration telescope.
        let valid = if let Some(def) = self.newtypes.get(declaration).filter(|_| alias.is_none()) {
            def.validate_completed_arguments_with_evidence_in(
                &types,
                &mut remaining,
                |ty, variant, fuel| {
                    self.normal_value_fields(ty, variant, fuel).ok_or_else(|| {
                        if *fuel == 0 {
                            nir::NominalAdmissionError::Budget
                        } else {
                            nir::NominalAdmissionError::Invalid(
                                "nominal value layout unavailable".into(),
                            )
                        }
                    })
                },
                |value, domain, _| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if self.abstract_value_parameters.get(declaration)==Some(domain)), self.target)
        } else {
            nir::validate_declaration_arguments_with_evidence_in(
                &ids,
                domains,
                &types,
                &mut remaining,
                |ty, variant, fuel| {
                    self.normal_value_fields(ty, variant, fuel).ok_or_else(|| {
                        if *fuel == 0 {
                            nir::NominalAdmissionError::Budget
                        } else {
                            nir::NominalAdmissionError::Invalid(
                                "nominal value layout unavailable".into(),
                            )
                        }
                    })
                },
                |value, domain, _| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if self.abstract_value_parameters.get(declaration)==Some(domain)),
                self.target,
            )
        };
        self.budget.steps -= (before - remaining) as u64;
        valid.map_err(|error| {
            self.failure(
                match error {
                    nir::NominalAdmissionError::Budget => ErrorKind::StepsExhausted,
                    nir::NominalAdmissionError::Invalid(_) => ErrorKind::Type,
                },
                Some(expr.source),
            )
        })?;
        for requirement in requirements {
            self.tick(Some(expr.source))?;
            let predicate = requirement
                .instantiate_parameters(&parameters, &types)
                .map_err(|_| self.failure(ErrorKind::Type, Some(expr.source)))?;
            self.check_type_contract(&predicate, expr.source)?;
        }
        if !self.capabilities_hold_parameters(capabilities, &parameters, &types) {
            return Err(self.failure(ErrorKind::TypeContractUnresolved, Some(expr.source)));
        }
        if let K::CheckTypeAlias { body, .. } = &expr.kind {
            return if let Some(body) = body {
                nir::copy_budget::charge_type_rewrite(body, &mut self.budget.steps).map_err(
                    |error| {
                        self.failure(
                            match error {
                                nir::copy_budget::CopyBudgetExhausted::Steps => {
                                    ErrorKind::StepsExhausted
                                }
                                nir::copy_budget::CopyBudgetExhausted::Depth => {
                                    ErrorKind::DepthExhausted
                                }
                            },
                            Some(body.source),
                        )
                    },
                )?;
                let mut specialized = (**body).clone();
                specialized.rewrite_types(&mut |ty| {
                    self.instantiate_contextual_domain(ty, &parameters, &types, body.source)
                })?;
                self.eval(&specialized, environment)
            } else {
                Ok(Flow::Value(Value::Unit))
            };
        }
        Ok(Flow::Value(Value::Type(Term::Nominal {
            declaration: *declaration,
            arguments: types,
        })))
    }

    #[inline(never)]
    fn eval_scalar_comparison(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        use nir::program::IntegerOp as Comparison;
        use std::cmp::Ordering::*;
        let (left, right, op) = match &expr.kind {
            K::ScalarEqual(left, right) => (left, right, Comparison::Equal),
            K::ScalarLessEqual(left, right) => (left, right, Comparison::LessEqual),
            K::ScalarLess(left, right) => (left, right, Comparison::Less),
            K::ScalarGreater(left, right) => (left, right, Comparison::Greater),
            K::ScalarGreaterEqual(left, right) => (left, right, Comparison::GreaterEqual),
            _ => unreachable!(),
        };
        let never = Term::Primitive(Primitive::Never);
        let result_ty = if left.ty == never || right.ty == never {
            never.clone()
        } else {
            Term::Primitive(Primitive::Bool)
        };
        let supported = |ty: &Term| matches!(ty, Term::Primitive(p) if matches!(p, Primitive::Bool | Primitive::Char | Primitive::Str | Primitive::Symbol | Primitive::Float | Primitive::F32 | Primitive::F64 | Primitive::Never) || self.integer_layout(*p).is_some());
        if expr.ty != result_ty
            || !supported(&left.ty)
            || !supported(&right.ty)
            || (op != Comparison::Equal
                && [left, right].iter().any(|value| {
                    matches!(
                        value.ty,
                        Term::Primitive(Primitive::Bool | Primitive::Str | Primitive::Symbol)
                    )
                }))
            || (left.ty != never && right.ty != never && left.ty != right.ty)
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let a = value!(self.eval(left, environment));
        let b = value!(self.eval(right, environment));
        if !self.has_type(&a, &left.ty) || !self.has_type(&b, &right.ty) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        if matches!(a, Value::AbstractValue { .. }) || matches!(b, Value::AbstractValue { .. }) {
            return Err(self.failure(ErrorKind::AbstractValueUnresolved, Some(expr.source)));
        }
        let compare = |ordering| match op {
            Comparison::Equal => matches!(ordering, Some(Equal)),
            Comparison::Less => matches!(ordering, Some(Less)),
            Comparison::LessEqual => matches!(ordering, Some(Less | Equal)),
            Comparison::Greater => matches!(ordering, Some(Greater)),
            Comparison::GreaterEqual => matches!(ordering, Some(Greater | Equal)),
            _ => unreachable!(),
        };
        let equal = match (&a, &b) {
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Char(a), Value::Char(b)) => compare(a.partial_cmp(b)),
            (Value::SignedInteger(a), Value::SignedInteger(b)) => compare(a.partial_cmp(b)),
            (Value::UnsignedInteger(a), Value::UnsignedInteger(b)) => compare(a.partial_cmp(b)),
            (Value::Float(a), Value::Float(b)) => compare(Some(
                self.integer_operation(Some(expr.source), |budget| a.compare(b, budget))?,
            )),
            (Value::Float32(a), Value::Float32(b)) => {
                compare(f32::from_bits(*a).partial_cmp(&f32::from_bits(*b)))
            }
            (Value::Float64(a), Value::Float64(b)) => {
                compare(f64::from_bits(*a).partial_cmp(&f64::from_bits(*b)))
            }
            (Value::String(a), Value::String(b)) | (Value::Symbol(a), Value::Symbol(b)) => {
                self.integer_operation(Some(expr.source), |budget| {
                    budget.consume(a.len().min(b.len()).saturating_add(1))
                })?;
                a == b
            }
            _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
        };
        Ok(Flow::Value(Value::Bool(equal)))
    }

    #[inline(never)]
    fn eval_type_list_index(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        if self.comptime_only
            && self.cleanup_schedule.is_none()
            && let K::TypeListIndex(values, index) = &expr.kind
            && self.is_type_list_input(&values.ty)
            && let K::Local(binding) = values.kind
            && matches!(index.kind, K::Literal(Literal::Integer(_)) | K::Local(_))
        {
            return self.eval_local_type_list_index(expr, values, binding, index, environment);
        }
        let K::TypeListIndex(values, index) = &expr.kind else {
            unreachable!()
        };
        let element_type = self
            .list_element(&values.ty)
            .cloned()
            .or_else(|| {
                self.is_type_list_input(&values.ty)
                    .then_some(Term::Primitive(Primitive::Type))
            })
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
        let result_type = if index.ty == Term::Primitive(Primitive::Never) {
            Term::Primitive(Primitive::Never)
        } else {
            element_type
        };
        if expr.ty != result_type
            || !matches!(
                index.ty,
                Term::Primitive(Primitive::Integer | Primitive::Never)
            )
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        if self.comptime_only
            && self.cleanup_schedule.is_none()
            && let K::Local(binding) = values.kind
            && matches!(index.kind, K::Literal(Literal::Integer(_)) | K::Local(_))
        {
            return self.eval_local_list_index(expr, values, binding, index, environment);
        }
        let list = value!(self.eval(values, environment));
        let length = match &list {
            Value::TypeList(elements) => elements.len(),
            Value::List { values, .. } => values.len(),
            _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
        };
        let Value::Integer(position) = value!(self.eval(index, environment)) else {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        };
        let position = position
            .to_u128()
            .ok()
            .and_then(|index| usize::try_from(index).ok())
            .filter(|index| *index < length)
            .ok_or_else(|| self.failure(ErrorKind::IndexOutOfBounds, Some(expr.source)))?;
        if let Value::List { mut values, .. } = list {
            return Ok(Flow::Value(values.swap_remove(position)));
        }
        let Value::TypeList(mut elements) = list else {
            unreachable!()
        };
        let result = elements.swap_remove(position);
        let work = self
            .type_description_work(&result)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
        self.integer_operation(Some(expr.source), |budget| budget.consume(work))?;
        Ok(Flow::Value(Value::Type(result)))
    }

    #[inline(never)]
    fn eval_type_equal(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::TypeEqual(left, right) = &expr.kind else {
            unreachable!()
        };
        let expected = if [left, right]
            .iter()
            .any(|field| field.ty == Term::Primitive(Primitive::Never))
        {
            Primitive::Never
        } else {
            Primitive::Bool
        };
        if expr.ty != Term::Primitive(expected)
            || [left, right].iter().any(|field| {
                !matches!(
                    field.ty,
                    Term::Primitive(Primitive::Type | Primitive::Never)
                )
            })
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let left = value!(self.eval(left, environment));
        let left = self.type_description_term(left, expr.source)?;
        let right = value!(self.eval(right, environment));
        let right = self.type_description_term(right, expr.source)?;
        let work = self
            .type_description_work(&left)
            .and_then(|a| {
                self.type_description_work(&right)
                    .and_then(|b| a.checked_add(b))
            })
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
        self.integer_operation(Some(expr.source), |budget| budget.consume(work))?;
        let predicate = nir::program::TypePredicate::Equal(left, right);
        if predicate.trivial_truth() == Some(true) {
            return Ok(Flow::Value(Value::Bool(true)));
        }
        // Failure to entail equality is not evidence of inequality. Ask for
        // a proof of its negation separately; Unknown must not select a branch.
        if self.type_solver.is_some() {
            self.tick(Some(expr.source))?;
            if self.solve_type_predicate(&predicate) == ContractOutcome::Satisfied {
                return Ok(Flow::Value(Value::Bool(true)));
            }
            self.tick(Some(expr.source))?;
            if self.solve_type_predicate(&nir::program::TypePredicate::Not(Box::new(predicate)))
                == ContractOutcome::Satisfied
            {
                return Ok(Flow::Value(Value::Bool(false)));
            }
        }
        Err(self.failure(ErrorKind::TypeContractUnresolved, Some(expr.source)))
    }

    #[inline(never)]
    fn eval_type_list(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::TypeList(fields) = &expr.kind else {
            unreachable!()
        };
        if fields.len() > 100_000 {
            return Err(self.failure(ErrorKind::RepresentationSizeLimit, Some(expr.source)));
        }
        let diverging = fields
            .iter()
            .any(|field| field.ty == Term::Primitive(Primitive::Never));
        let element_type = self.list_element(&expr.ty).cloned().or_else(|| {
            self.is_type_list_input(&expr.ty)
                .then_some(Term::Primitive(Primitive::Type))
        });
        if (diverging && expr.ty != Term::Primitive(Primitive::Never))
            || (!diverging && element_type.is_none())
            || fields.iter().any(|field| {
                field.ty != Term::Primitive(Primitive::Never)
                    && !diverging
                    && Some(&field.ty) != element_type.as_ref()
            })
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        self.integer_operation(Some(expr.source), |budget| {
            budget.consume(fields.len().saturating_add(1))
        })?;
        if diverging {
            for field in fields {
                let _ = value!(self.eval(field, environment));
            }
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        if element_type != Some(Term::Primitive(Primitive::Type)) {
            let mut values = Vec::with_capacity(fields.len());
            for field in fields {
                values.push(value!(self.eval(field, environment)));
            }
            let value = Value::List {
                ty: expr.ty.clone(),
                values,
            };
            self.validate_immutable_input(&value, &expr.ty)?;
            return Ok(Flow::Value(value));
        }
        let mut types = Vec::with_capacity(fields.len());
        for field in fields {
            let value = value!(self.eval(field, environment));
            types.push(self.type_description_term(value, field.source)?);
        }
        let value = Value::TypeList(types);
        self.validate_immutable_input(&value, &expr.ty)?;
        Ok(Flow::Value(value))
    }

    #[inline(never)]
    fn eval_tuple_type(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::TupleType(fields) = &expr.kind else {
            unreachable!()
        };
        let expected = if fields
            .iter()
            .any(|field| field.ty == Term::Primitive(Primitive::Never))
        {
            Primitive::Never
        } else {
            Primitive::Type
        };
        if expr.ty != Term::Primitive(expected)
            || fields.iter().any(|field| {
                !matches!(
                    field.ty,
                    Term::Primitive(Primitive::Type | Primitive::Never)
                )
            })
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        self.integer_operation(Some(expr.source), |budget| {
            budget.consume(fields.len().saturating_add(1))
        })?;
        let mut types = Vec::with_capacity(fields.len());
        for field in fields {
            let value = value!(self.eval(field, environment));
            let ty = self.type_description_term(value, field.source)?;
            types.push(ty);
        }
        Ok(Flow::Value(Value::Type(Term::Tuple(types))))
    }

    #[inline(never)]
    fn eval_bit_vec_type(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::BitVecType(width) => {
                let expected = if width.ty == Term::Primitive(Primitive::Never) {
                    Primitive::Never
                } else {
                    Primitive::Type
                };
                if expr.ty != Term::Primitive(expected)
                    || !matches!(
                        width.ty,
                        Term::Primitive(Primitive::Integer | Primitive::Never)
                    )
                {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let Value::Integer(width) = value!(self.eval(width, environment)) else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                self.integer_operation(Some(expr.source), |budget| {
                    budget.consume(width.bit_len().div_ceil(32).saturating_add(1))
                })?;
                let width = width
                    .to_u128()
                    .ok()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| self.failure(ErrorKind::InvalidBitWidth, Some(expr.source)))?;
                Value::Type(Term::Primitive(Primitive::BitVec(width)))
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_literal(&mut self, expr: &Expr, _environment: &mut Environment) -> Result<Flow, Error> {
        if let K::Literal(Literal::PendingInteger { spelling, negative }) = &expr.kind {
            let Term::Primitive(domain) = expr.ty else {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            };
            let target = self.target;
            let literal = self.integer_operation(Some(expr.source), |budget| {
                nir::integer_formation::form(spelling, *negative, domain, target, budget)
            })?;
            return self.eval_literal(
                &Expr {
                    source: expr.source,
                    ty: expr.ty.clone(),
                    kind: K::Literal(literal),
                },
                _environment,
            );
        }
        if let K::Literal(Literal::PendingFloat { spelling, negative }) = &expr.kind {
            let Term::Primitive(domain @ (Primitive::Float | Primitive::F32 | Primitive::F64)) =
                expr.ty
            else {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            };
            let literal = self.integer_operation(Some(expr.source), |budget| {
                nir::float_formation::FloatFormation::parse(spelling, *negative, budget)?
                    .form(domain, budget)
            })?;
            return self.eval_literal(
                &Expr {
                    source: expr.source,
                    ty: expr.ty.clone(),
                    kind: K::Literal(literal),
                },
                _environment,
            );
        }
        if let K::Literal(literal) = &expr.kind {
            if !literal.has_primitive_type_in(&expr.ty, self.target) {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            }
        }
        let result = match &expr.kind {
            K::Literal(literal) => match literal {
                Literal::Integer(spelling) => {
                    Value::Integer(self.integer_literal(spelling, Some(expr.source))?)
                }
                Literal::BitVec { width, words } => {
                    Value::BitVec(self.bit_operation(expr.source, |budget| {
                        comptime_runtime::bitvec::BitVec::from_words(*width, words, budget)
                    })?)
                }
                Literal::Float(spelling) => {
                    Value::Float(self.integer_operation(Some(expr.source), |budget| {
                        comptime_runtime::decimal::DecimalValue::parse_literal(spelling, budget)
                    })?)
                }
                Literal::Float32(bits) => Value::Float32(*bits),
                Literal::Float64(bits) => Value::Float64(*bits),
                Literal::SignedInteger(v) => {
                    let value = Value::SignedInteger(*v);
                    value
                }
                Literal::UnsignedInteger(v) => {
                    let value = Value::UnsignedInteger(*v);
                    value
                }
                Literal::Bool(v) => Value::Bool(*v),
                Literal::Char(v) => Value::Char(*v),
                Literal::String(v) | Literal::Symbol(v) => {
                    self.integer_operation(Some(expr.source), |budget| {
                        budget.consume(v.len().saturating_add(1))
                    })?;
                    let text = Arc::from(v.as_str());
                    if matches!(literal, Literal::String(_)) {
                        Value::String(text)
                    } else {
                        Value::Symbol(text)
                    }
                }
                _ => return Err(self.failure(ErrorKind::Unsupported, Some(expr.source))),
            },
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_variant(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        self.with_prepared_frame(expr.source, |this| {
            let result = match &expr.kind {
                K::Variant {
                    declaration,
                    index,
                    fields,
                } => {
                    let mut values = Vec::new();
                    for (slot, field) in fields.iter().enumerate() {
                        let value = value!(this.eval(field, environment));
                        this.retain_prepared(slot, field.source, &value)?;
                        values.push(value);
                    }
                    let selected = if this.associated_values.is_empty() {
                        None
                    } else {
                        Some(this.selected_type_view(&expr.ty, Some(expr.source))?)
                    };
                    let Term::Nominal { arguments, .. } = selected.as_ref().unwrap_or(&expr.ty)
                    else {
                        return Err(this.failure(ErrorKind::Type, Some(expr.source)));
                    };
                    let value = Value::Variant {
                        arguments: arguments.clone(),
                        declaration: *declaration,
                        index: *index,
                        fields: values,
                    };
                    if !this.has_type(&value, &expr.ty) {
                        return Err(this.failure(ErrorKind::Type, Some(expr.source)));
                    }
                    value
                }
                _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
            };
            Ok(Flow::Value(result))
        })
    }

    #[inline(never)]
    fn eval_record(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        self.with_prepared_frame(expr.source, |this| {
            let result = match &expr.kind {
                K::AnonymousRecord { fields } => {
                    let mut values = Vec::new();
                    for (slot, field) in fields.iter().enumerate() {
                        let value = value!(this.eval(field, environment));
                        this.retain_prepared(slot, field.source, &value)?;
                        values.push(value);
                    }
                    let Term::Record(types) = &expr.ty else {
                        return Err(this.failure(ErrorKind::Type, Some(expr.source)));
                    };
                    if types.len() != values.len() {
                        return Err(this.failure(ErrorKind::Type, Some(expr.source)));
                    }
                    let value = Value::AnonymousRecord(
                        types
                            .iter()
                            .map(|(name, _)| name.clone())
                            .zip(values)
                            .collect(),
                    );
                    if !this.has_type(&value, &expr.ty) {
                        return Err(this.failure(ErrorKind::Type, Some(expr.source)));
                    }
                    value
                }
                K::Record {
                    declaration,
                    fields,
                } => {
                    let mut values = Vec::new();
                    for (slot, field) in fields.iter().enumerate() {
                        let value = value!(this.eval(field, environment));
                        this.retain_prepared(slot, field.source, &value)?;
                        values.push(value);
                    }
                    let selected = if this.associated_values.is_empty() {
                        None
                    } else {
                        Some(this.selected_type_view(&expr.ty, Some(expr.source))?)
                    };
                    let Term::Nominal { arguments, .. } = selected.as_ref().unwrap_or(&expr.ty)
                    else {
                        return Err(this.failure(ErrorKind::Type, Some(expr.source)));
                    };
                    let value = Value::Record {
                        arguments: arguments.clone(),
                        declaration: *declaration,
                        fields: values,
                    };
                    if !this.has_type(&value, &expr.ty) {
                        return Err(this.failure(ErrorKind::Type, Some(expr.source)));
                    }
                    value
                }
                _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
            };
            Ok(Flow::Value(result))
        })
    }

    #[inline(never)]
    fn eval_field(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        if let Some(value) = self.eval_local_field(expr, environment) {
            return value;
        }
        let result = match &expr.kind {
            K::AnonymousField { value, name, index } => {
                let representation = value!(self.eval(value, environment));
                if !matches!(
                    value.kind,
                    K::Field { .. } | K::AnonymousField { .. } | K::Local(_)
                ) {
                    self.retain_residual(value.source, &representation)?;
                }
                let Value::AnonymousRecord(fields) = representation else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let (actual, value) = fields
                    .get(*index)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
                if actual != name || !self.has_type(value, &expr.ty) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                value.clone()
            }
            K::Field {
                value,
                declaration,
                index,
            } => {
                let representation = value!(self.eval(value, environment));
                if !matches!(
                    value.kind,
                    K::Field { .. } | K::AnonymousField { .. } | K::Local(_)
                ) {
                    self.retain_residual(value.source, &representation)?;
                }
                let Value::Record {
                    declaration: actual,
                    fields,
                    ..
                } = representation
                else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                if actual != *declaration {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                fields
                    .get(*index)
                    .cloned()
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_associated_value(&mut self, expr: &Expr) -> Result<Flow, Error> {
        let K::AssociatedValue {
            interface,
            subject,
            member,
        } = &expr.kind
        else {
            unreachable!()
        };
        let selected_subject = self.selected_type_view(subject, Some(expr.source))?;
        let selected_arguments = interface
            .arguments
            .iter()
            .map(|argument| self.selected_type_view(argument, Some(expr.source)))
            .collect::<Result<Vec<_>, _>>()?;
        let selected_interface = InterfaceInstance {
            declaration: interface.declaration,
            arguments: selected_arguments,
        };
        let key = (selected_interface, selected_subject, *member);
        let selected = *self
            .associated_values
            .get(&key)
            .or_else(|| {
                self.comptime_only
                    .then(|| self.associated_initializers.get(&key))
                    .flatten()
            })
            .ok_or_else(|| self.failure(ErrorKind::UnmaterializedDispatch, Some(expr.source)))?;
        if !selected_types::same(&selected.ty, &expr.ty, &self.associated_values) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        // Associated values cannot read a caller's lexical environment.
        let initializing = self.associated_initializers.contains_key(&key);
        if initializing && !self.active_associated_initializers.insert(key.clone()) {
            return Err(self.failure(ErrorKind::InvalidControlFlow, Some(expr.source)));
        }
        let result = self.eval(selected, &mut Environment::default());
        if initializing {
            self.active_associated_initializers.remove(&key);
        }
        let result = result?;
        let Flow::Value(value) = result else {
            return Err(self.failure(ErrorKind::InvalidControlFlow, Some(expr.source)));
        };
        if !self.has_type(&value, &expr.ty) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        Ok(Flow::Value(value))
    }
    #[inline(never)]
    fn eval_trait_method(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
        direct_callee: bool,
    ) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::TraitMethod {
                method_arguments,
                interface,
                interface_arguments,
                member,
                subject,
            } => {
                let (parameters, contract) = self
                    .method_contracts
                    .get(&(*interface, *member))
                    .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(expr.source)))?;
                let parameters = parameters.clone();
                let contract = *contract;
                // Raw NIR can check an immediate interface call, but cannot
                // erase its program domain into an implementation function
                // value. IIR transports the identity and specializes the call.
                let retained_template = self.comptime_only
                    && !contract.signature.is_comptime_callable()
                    && matches!(contract.signature.callable_signature(), Ok(Term::Function { parameters, .. })
                        if parameters.iter().any(|p| p.sort != nir::Sort::Runtime));
                if !direct_callee
                    && !contract.program.preconditions.is_empty()
                    && !retained_template
                {
                    return Err(self.failure(ErrorKind::Unsupported, Some(expr.source)));
                }
                if method_arguments.len() != contract.type_parameters.len() {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, Some(expr.source)));
                }
                let contract_arguments: Vec<_> = std::iter::once(subject.clone())
                    .chain(interface_arguments.iter().cloned())
                    .chain(method_arguments.iter().cloned())
                    .collect();
                if !self.capabilities_hold_parameters(
                    &contract.capabilities,
                    &parameters,
                    &contract_arguments,
                ) {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, Some(expr.source)));
                }
                for requirement in &contract.requirements {
                    let requirement = requirement
                        .instantiate_parameters(&parameters, &contract_arguments)
                        .map_err(|_| self.failure(ErrorKind::Type, Some(expr.source)))?;
                    self.check_type_contract(&requirement, expr.source)?;
                }
                let selected_subject = self.selected_type_view(subject, Some(expr.source))?;
                let selected_arguments = interface_arguments
                    .iter()
                    .map(|argument| self.selected_type_view(argument, Some(expr.source)))
                    .collect::<Result<Vec<_>, _>>()?;
                let (body, mut arguments) = self
                    .method_bodies
                    .get(&(
                        InterfaceInstance {
                            declaration: *interface,
                            arguments: selected_arguments,
                        },
                        selected_subject,
                        *member,
                    ))
                    .cloned()
                    .ok_or_else(|| {
                        self.failure(ErrorKind::UnmaterializedDispatch, Some(expr.source))
                    })?;
                arguments.extend(method_arguments.iter().cloned());
                if let Some(identity) = self.runtime_function_identities.get(&body).copied() {
                    self.admit_function_inputs(&identity.type_parameters, &arguments, expr.source)?;
                    let signature = identity
                        .signature
                        .instantiate_parameters(
                            &self.input_binders(&identity.type_parameters),
                            &arguments,
                        )
                        .map_err(|_| self.failure(ErrorKind::Type, Some(expr.source)))?;
                    if !selected_types::same(&signature, &expr.ty, &self.associated_values)
                        || (!contract.program.preconditions.is_empty() && !retained_template)
                    {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, Some(expr.source)));
                    }
                    return Ok(Flow::Value(Value::RuntimeTraitMethod(Box::new(
                        TraitMethodIdentity {
                            interface: InterfaceInstance {
                                declaration: *interface,
                                arguments: interface_arguments.clone(),
                            },
                            subject: subject.clone(),
                            member: *member,
                            method_arguments: method_arguments.clone(),
                        },
                    ))));
                }
                let definition = self
                    .functions
                    .get(&body)
                    .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(expr.source)))?;
                let ty = definition
                    .signature
                    .instantiate_parameters(
                        &self.input_binders(&definition.type_parameters),
                        &arguments,
                    )
                    .map_err(|_| self.failure(ErrorKind::Type, Some(expr.source)))?
                    .with_comptime_phase(definition.phase == nir::program::FunctionPhase::Comptime)
                    .with_unsafe_call_requirement(definition.is_unsafe);
                if !selected_types::same(&ty, &expr.ty, &self.associated_values) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                if self.comptime_only && definition.phase == nir::program::FunctionPhase::Runtime {
                    self.admit_function_inputs(
                        &definition.type_parameters,
                        &arguments,
                        expr.source,
                    )?;
                    if !contract.program.preconditions.is_empty() && !retained_template {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, Some(expr.source)));
                    }
                    // Transport the public interface identity even when this
                    // snapshot also contains its implementation's runtime body.
                    return Ok(Flow::Value(Value::RuntimeTraitMethod(Box::new(
                        TraitMethodIdentity {
                            interface: InterfaceInstance {
                                declaration: *interface,
                                arguments: interface_arguments.clone(),
                            },
                            subject: subject.clone(),
                            member: *member,
                            method_arguments: method_arguments.clone(),
                        },
                    ))));
                }
                let resolved = Expr {
                    ty,
                    source: expr.source,
                    kind: if arguments.is_empty() && definition.requirements.is_empty() {
                        K::Function(body)
                    } else {
                        K::FunctionInstance {
                            declaration: body,
                            arguments,
                        }
                    },
                };
                value!(self.eval(&resolved, environment))
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    fn validate_function_inputs(
        &self,
        parameters: &[DeclarationId],
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<(), ErrorKind> {
        *fuel = fuel
            .checked_sub(parameters.len())
            .ok_or(ErrorKind::StepsExhausted)?;
        if parameters.len() != arguments.len() {
            return Err(ErrorKind::Type);
        }
        let binders = self.input_binders(parameters);
        for (binder, argument) in binders.iter().zip(arguments) {
            if argument.sort() != binder.sort {
                return Err(ErrorKind::Type);
            }
            if binder.sort == nir::Sort::Type {
                continue;
            }
            let raw = self
                .declaration_parameter_domains
                .get(&binder.declaration)
                .ok_or(ErrorKind::Type)?;
            let domain = nir::type_computations::instantiate_parameters_bounded(
                raw,
                &binders,
                arguments,
                &[],
                fuel,
            )
            .map_err(|failure| {
                if matches!(failure, nir::type_computations::Error::Budget) {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                }
            })?;
            if let Term::TypeListValue(elements) = argument {
                nir::type_computations::charge_term(argument, fuel).map_err(|failure| {
                    if matches!(failure, nir::type_computations::Error::Budget) {
                        ErrorKind::StepsExhausted
                    } else {
                        ErrorKind::Type
                    }
                })?;
                argument.check_scope(&[]).map_err(|_| ErrorKind::Type)?;
                let Term::NormalForm { head, .. } = &domain else {
                    return Err(ErrorKind::Type);
                };
                // Constructor metadata was validated for uniqueness when this
                // interpreter was created. Never infer the role from spelling.
                if !self
                    .normal_forms
                    .get(head)
                    .is_some_and(|head| head.is_type_list_instance(&domain))
                    || elements
                        .iter()
                        .any(|element| element.sort() != nir::Sort::Type)
                {
                    return Err(ErrorKind::Type);
                }
                continue;
            }
            if domain == Term::Primitive(Primitive::Object) || self.list_element(&domain).is_some()
            {
                nir::type_computations::charge_term(&domain, fuel)
                    .map_err(|_| ErrorKind::StepsExhausted)?;
                // Ordinary function clauses have the same value-domain contract
                // as normal-form inputs; this descriptor is only a checker,
                // never a new declaration or a type identity.
                let contract = nir::normal_forms::Head {
                    declaration: binder.declaration,
                    role: nir::normal_forms::Role::Opaque,
                    requirements: vec![],
                    parameters: vec![nir::normal_forms::Parameter {
                        declaration: binder.declaration,
                        binding: LocalId {
                            owner: binder.declaration.local,
                            index: 0,
                        },
                        kind: nir::normal_forms::Kind::Value,
                        ty: domain.clone(),
                    }],
                };
                contract.check_arguments_in_scope_with_collections_budgeted_in(
                    std::slice::from_ref(argument),
                    |id| self.normal_forms.get(&id).is_some_and(|head| head.role == nir::normal_forms::Role::Tuple),
                    |id| self.normal_forms.get(&id).is_some_and(|head| head.role == nir::normal_forms::Role::TypeList),
                    |ty, variant, fuel| self.normal_value_fields(ty, variant, fuel),
                    |value, domain| matches!(value, Term::Rigid { declaration, sort: nir::Sort::ComptimeValue }
                        if self.abstract_value_parameters.get(declaration) == Some(domain)),
                    &[], fuel, self.target,
                ).map_err(|_| if *fuel == 0 { ErrorKind::StepsExhausted } else { ErrorKind::Type })?;
                continue;
            }
            nir::validate_declaration_arguments_with_evidence_in(
                &[binder.declaration],
                &[domain],
                std::slice::from_ref(argument),
                fuel,
                |ty, variant, fuel| self.admitted_nominal_layout(ty, variant, fuel),
                |value, domain, _| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if self.abstract_value_parameters.get(declaration)==Some(domain)), self.target)
            .map_err(|failure| {
                if matches!(failure, nir::NominalAdmissionError::Budget) {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                }
            })?;
        }
        Ok(())
    }

    fn admit_function_inputs(
        &mut self,
        parameters: &[DeclarationId],
        arguments: &[Term],
        source: SourceRange,
    ) -> Result<(), Error> {
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let checked = self.validate_function_inputs(parameters, arguments, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        checked.map_err(|kind| self.failure(kind, Some(source)))
    }

    #[inline(never)]
    fn eval_function_instance(
        &mut self,
        expr: &Expr,
        _environment: &mut Environment,
    ) -> Result<Flow, Error> {
        if let K::FunctionInstance {
            declaration,
            arguments,
        } = &expr.kind
        {
            if let Some(identity) = self.runtime_function_identities.get(declaration).copied() {
                self.admit_function_inputs(&identity.type_parameters, arguments, expr.source)?;
                let binders = self.input_binders(&identity.type_parameters);
                self.charge_copy_work(Some(expr.source), |fuel| {
                    nir::copy_budget::charge_type_substitution(
                        &identity.signature,
                        &binders,
                        arguments,
                        fuel,
                    )
                })?;
                let signature = identity
                    .signature
                    .instantiate_parameters(
                        &self.input_binders(&identity.type_parameters),
                        arguments,
                    )
                    .map_err(|_| self.failure(ErrorKind::Type, Some(expr.source)))?;
                if !selected_types::same(&signature, &expr.ty, &self.associated_values) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                return Ok(Flow::Value(Value::RuntimeFunction {
                    declaration: *declaration,
                    arguments: arguments.clone(),
                }));
            }
        }
        let result = match &expr.kind {
            K::FunctionInstance {
                declaration,
                arguments,
            } => {
                let definition = *self
                    .functions
                    .get(declaration)
                    .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(expr.source)))?;
                self.admit_function_inputs(&definition.type_parameters, arguments, expr.source)?;
                let key = (*declaration, arguments.clone());
                let binders = self.input_binders(&definition.type_parameters);
                if !self.instances.contains_key(&key) {
                    self.charge_copy_work(Some(expr.source), |fuel| {
                        nir::copy_budget::charge_function_substitution(
                            definition, &binders, arguments, fuel,
                        )
                    })?;
                }
                if self.comptime_only {
                    let definition = *self.functions.get(declaration).ok_or_else(|| {
                        self.failure(ErrorKind::UnknownFunction, Some(expr.source))
                    })?;
                    for requirement in &definition.requirements {
                        // Cache hits reuse the body, but invocation contracts
                        // are substituted again in the current input scope.
                        self.charge_copy_work(Some(expr.source), |fuel| {
                            nir::copy_budget::charge_predicate_substitution(
                                requirement,
                                &binders,
                                arguments,
                                fuel,
                            )
                        })?;
                        let requirement = requirement
                            .instantiate_parameters(
                                &self.input_binders(&definition.type_parameters),
                                arguments,
                            )
                            .map_err(|_| self.failure(ErrorKind::Type, Some(expr.source)))?;
                        if let Some(requirement) =
                            self.bind_invocation_contract(&requirement, None, expr.source, 0)?
                        {
                            self.check_type_contract(&requirement, expr.source)?;
                        }
                    }
                }
                let instance = if let Some(instance) = self.instances.get(&key) {
                    instance.clone()
                } else {
                    let definition = self.functions.get(declaration).ok_or_else(|| {
                        self.failure(ErrorKind::UnknownFunction, Some(expr.source))
                    })?;
                    if !definition.capabilities.iter().all(|capability| {
                        let Ok(bound) = capability.instantiate_parameters(
                            &self.input_binders(&definition.type_parameters),
                            arguments,
                        ) else {
                            return false;
                        };
                        let open = bound.subject.check_scope(&[]).is_err()
                            || bound
                                .interface_arguments
                                .iter()
                                .any(|ty| ty.check_scope(&[]).is_err());
                        if self.comptime_only && open {
                            return true;
                        }
                        self.capabilities_hold_parameters(
                            std::slice::from_ref(capability),
                            &self.input_binders(&definition.type_parameters),
                            arguments,
                        )
                    }) {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, Some(expr.source)));
                    }
                    let signature = definition
                        .signature
                        .instantiate_parameters(
                            &self.input_binders(&definition.type_parameters),
                            arguments,
                        )
                        .map_err(|_| self.failure(ErrorKind::Type, Some(expr.source)))?;
                    let body = definition
                        .body
                        .instantiate_parameters(
                            &self.input_binders(&definition.type_parameters),
                            arguments,
                        )
                        .map_err(|_| self.failure(ErrorKind::Type, Some(expr.source)))?;
                    let instance = Arc::new(Closure {
                        definition_source: None,
                        origin: Some(*declaration),
                        instance_arguments: Some(arguments.clone()),
                        parameters: definition.parameters.clone(),
                        environment: Environment::new(),
                        capture_order: vec![],
                        body: Arc::new(body),
                        ty: signature
                            .with_comptime_phase(
                                definition.phase == nir::program::FunctionPhase::Comptime,
                            )
                            .with_unsafe_call_requirement(definition.is_unsafe),
                    });
                    self.events.push(Event::InstantiateFunction {
                        declaration: *declaration,
                        arguments: arguments.clone(),
                    });
                    self.instances.insert(key, instance.clone());
                    instance
                };
                if !selected_types::same(&instance.ty, &expr.ty, &self.associated_values) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                Value::Closure(instance)
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_closure(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        self.with_prepared_frame(expr.source, |this| {
            let result = match &expr.kind {
                K::Closure {
                    parameters,
                    captures,
                    body,
                } => {
                    let capture_types: &[Term] = match &expr.ty {
                        Term::Closure { environment, .. } => environment,
                        Term::Function { .. }
                        | Term::ComptimeFunction(_)
                        | Term::UnsafeFunction(_) => &[],
                        _ => return Err(this.failure(ErrorKind::Type, Some(expr.source))),
                    };
                    let Term::Function {
                        parameters: types, ..
                    } = expr.ty.callable_signature()
                    else {
                        return Err(this.failure(ErrorKind::Type, Some(expr.source)));
                    };
                    if types.len() != parameters.len() || captures.len() != capture_types.len() {
                        return Err(this.failure(ErrorKind::Type, Some(expr.source)));
                    }
                    let mut captured = Environment::new();
                    for (slot, (capture, ty)) in captures.iter().zip(capture_types).enumerate() {
                        this.tick(Some(expr.source))?;
                        let value = if let Some(value) = &capture.value {
                            value!(this.eval(value, environment))
                        } else {
                            let mut value =
                                this.local_value(environment, capture.binding, Some(expr.source))?;
                            this.cleanup_copy_delivery(
                                expr.source,
                                capture.binding,
                                &[],
                                &mut value,
                            )?;
                            this.cleanup_transfer(expr.source, capture.binding, &[], environment)?;
                            value
                        };
                        if &capture.ty != ty || !this.has_type(&value, ty) {
                            return Err(this.failure(ErrorKind::Type, Some(expr.source)));
                        }
                        this.retain_prepared(
                            slot,
                            capture
                                .value
                                .as_ref()
                                .map_or(expr.source, |value| value.source),
                            &value,
                        )?;
                        // Each construction owns fresh capture cells; repeated
                        // invocations retain updates in either execution phase.
                        // Constant materialization snapshots their values and
                        // reconstructs sharing without exporting active storage.
                        let value = Value::Storage(memory::Storage::new(value));
                        if parameters.contains(&capture.binding)
                            || captured.insert(capture.binding, value).is_some()
                        {
                            return Err(this.failure(ErrorKind::InvalidCallPlan, Some(expr.source)));
                        }
                    }
                    this.events.push(Event::CreateClosure {
                        source: expr.source,
                        captures: captures.len(),
                    });
                    Value::Closure(Arc::new(Closure {
                        definition_source: Some(expr.source),
                        origin: None,
                        instance_arguments: None,
                        parameters: parameters.clone(),
                        environment: captured,
                        capture_order: captures.iter().map(|capture| capture.binding).collect(),
                        body: body.clone(),
                        ty: expr.ty.clone(),
                    }))
                }
                _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
            };
            Ok(Flow::Value(result))
        })
    }
    #[inline(never)]
    fn eval_tuple(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        self.with_prepared_frame(expr.source, |this| {
            let result = match &expr.kind {
                K::Tuple(items) => {
                    let mut values = Vec::new();
                    for (slot, item) in items.iter().enumerate() {
                        let value = value!(this.eval(item, environment));
                        this.retain_prepared(slot, item.source, &value)?;
                        values.push(value);
                    }
                    Value::Tuple(values)
                }
                _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
            };
            Ok(Flow::Value(result))
        })
    }

    #[inline(never)]
    fn match_arm_guard(
        &mut self,
        pattern: &nir::program::Pattern,
        guard: Option<&Expr>,
        value: &Value,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        self.match_arm_guard_with_bindings(pattern, guard, value, environment)
            .map(|(flow, _)| flow)
    }
    fn match_arm_guard_with_bindings(
        &mut self,
        pattern: &nir::program::Pattern,
        guard: Option<&Expr>,
        value: &Value,
        environment: &mut Environment,
    ) -> Result<(Flow, PatternBindings), Error> {
        let (matched, bindings) = self.match_pattern_with_bindings(pattern, value, environment)?;
        if !matches!(matched, Flow::Value(Value::Bool(true))) {
            return Ok((matched, PatternBindings::default()));
        }
        let flow = if let Some(guard) = guard {
            match self.eval(guard, environment)? {
                flow @ Flow::Value(Value::Bool(_)) => flow,
                Flow::Value(Value::AbstractValue { .. }) => {
                    return Err(
                        self.failure(ErrorKind::AbstractValueUnresolved, Some(guard.source))
                    );
                }
                Flow::Value(_) => return Err(self.failure(ErrorKind::Type, Some(guard.source))),
                flow => flow,
            }
        } else {
            Flow::Value(Value::Bool(true))
        };
        let bindings = if matches!(flow, Flow::Value(Value::Bool(true))) {
            bindings
        } else {
            PatternBindings::default()
        };
        Ok((flow, bindings))
    }

    #[inline(never)]
    fn eval_match(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        match &expr.kind {
            K::Match { value, arms } => {
                let value = value!(self.eval(value, environment));
                let existing = environment.snapshot();
                for arm in arms {
                    let flow = (|| {
                        self.tick(Some(expr.source))?;
                        let (matched, bindings) = self.match_arm_guard_with_bindings(
                            &arm.pattern,
                            arm.guard.as_ref(),
                            &value,
                            environment,
                        )?;
                        match matched {
                            Flow::Value(Value::Bool(false)) => return Ok(None),
                            Flow::Value(Value::Bool(true)) => {}
                            flow => return Ok(Some(flow)),
                        }
                        self.retain_pattern_residual(arm.body.source, &value, &bindings)?;
                        let result = self.eval(&arm.body, environment)?;
                        if matches!(result, Flow::Value(_))
                            && !matches!(arm.body.kind, K::Block { .. } | K::CallPreparation { .. })
                        {
                            self.cleanup_site(
                                arm.body.source,
                                iir::cleanup::Boundary::ScopeEnd,
                                environment,
                            )?;
                        }
                        Ok(Some(result))
                    })();
                    environment.retain(|id, _| existing.contains(id));
                    if let Some(flow) = flow? {
                        return Ok(flow);
                    }
                }
                return Err(self.failure(ErrorKind::NonExhaustiveMatch, Some(expr.source)));
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        }
    }

    #[inline(never)]
    fn eval_block(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        let mut frames = Vec::new();
        let mut current = expr;
        let mut flow = 'evaluate: loop {
            // Charge the saved scope as well as each expression. Deep generated
            // wrappers use heap continuations, not unbounded uncharged storage.
            let Some(remaining) = self.budget.steps.checked_sub(environment.len() as u64) else {
                let failure = Err(self.failure(ErrorKind::StepsExhausted, Some(current.source)));
                break if frames.is_empty() {
                    failure
                } else {
                    self.finish_evaluation(current, environment, failure)
                };
            };
            self.budget.steps = remaining;
            frames.push((current, environment.snapshot()));
            let (K::Block { statements, result } | K::CallPreparation { statements, result }) =
                &current.kind
            else {
                unreachable!("expression dispatcher selected an incompatible evaluator")
            };
            for statement in statements {
                match self.eval(statement, environment) {
                    Ok(Flow::Value(_)) => {}
                    other => break 'evaluate other,
                }
            }
            let Some(tail) = result.as_deref() else {
                break Ok(Flow::Value(Value::Unit));
            };
            if matches!(tail.kind, K::Block { .. } | K::CallPreparation { .. }) {
                if let Err(failure) = self.tick(Some(tail.source)) {
                    break Err(failure);
                }
                current = tail;
            } else {
                break self.eval(tail, environment);
            }
        };
        while let Some((frame, existing)) = frames.pop() {
            let cleanup = if matches!(&flow, Ok(Flow::Value(_))) {
                if matches!(frame.kind, K::CallPreparation { .. }) {
                    self.cleanup_site(frame.source, iir::cleanup::Boundary::ScopeEnd, environment)
                } else {
                    self.cleanup_block(frame.source, environment)
                }
            } else {
                Ok(())
            };
            match cleanup {
                Ok(()) => environment.retain(|id, _| existing.contains(id)),
                Err(failure) => flow = Err(failure),
            }
            // The outer eval owns the root's delivery hook. Every tail block
            // still receives the same hook after its lexical cleanup.
            if !frames.is_empty() {
                flow = self.finish_evaluation(frame, environment, flow);
            }
        }
        flow
    }

    #[inline(never)]
    fn eval_let_pattern(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        match &expr.kind {
            K::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => {
                let value_source = value.source;
                let value = value!(self.eval(value, environment));
                let existing = environment.snapshot();
                let succeeds = self.match_arm_guard_with_bindings(
                    pattern,
                    guard.as_deref(),
                    &value,
                    environment,
                );
                if let Ok((Flow::Value(Value::Bool(true)), bindings)) = &succeeds {
                    self.retain_pattern_residual(expr.source, &value, bindings)?;
                    return Ok(Flow::Value(Value::Unit));
                }
                // Roll back candidate bindings, preserving writes to outer places.
                environment.retain(|id, _| existing.contains(id));
                match succeeds?.0 {
                    Flow::Value(Value::Bool(false)) => {
                        if let Some(otherwise) = otherwise {
                            self.retain_residual(value_source, &value)?;
                            return match self.eval(otherwise, environment)? {
                                Flow::Value(_) => {
                                    Err(self
                                        .failure(ErrorKind::InvalidControlFlow, Some(expr.source)))
                                }
                                transfer => Ok(transfer),
                            };
                        }
                        return Err(self.failure(ErrorKind::NonExhaustiveMatch, Some(expr.source)));
                    }
                    Flow::Value(_) => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                    transfer => return Ok(transfer),
                }
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        }
    }

    fn eval_bit_selections(
        &mut self,
        expr: &Expr,
        storage_type: &Term,
        selections: &[nir::program::BitSelection],
        environment: &mut Environment,
    ) -> Result<SelectedBitRange, Error> {
        let mut width = match storage_type {
            Term::Primitive(Primitive::BitVec(width)) => *width,
            Term::Primitive(p) => p
                .unrestricted_scalar_width_in(self.target)
                .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?
                as usize,
            _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
        };
        let position = |value: Value| {
            match value {
                Value::Integer(n) => n.to_u128().ok(),
                Value::UnsignedInteger(n) => Some(n),
                Value::SignedInteger(n) => u128::try_from(n).ok(),
                _ => None,
            }
            .and_then(|n| usize::try_from(n).ok())
        };
        let mut offset = 0usize;
        for selection in selections {
            let (lo, hi, expected_width) = match selection {
                nir::program::BitSelection::Static { start, end } => {
                    (*start, *end, end.saturating_sub(*start))
                }
                nir::program::BitSelection::Index(index) => {
                    let evaluated = match self.eval(index, environment)? {
                        Flow::Value(value) => value,
                        control => return Ok(SelectedBitRange::Control(control)),
                    };
                    if !self.has_type(&evaluated, &index.ty) {
                        return Err(self.failure(ErrorKind::Type, Some(index.source)));
                    }
                    let lo = position(evaluated).ok_or_else(|| {
                        self.failure(ErrorKind::IndexOutOfBounds, Some(index.source))
                    })?;
                    let hi = lo.checked_add(1).ok_or_else(|| {
                        self.failure(ErrorKind::IndexOutOfBounds, Some(index.source))
                    })?;
                    (lo, hi, 1)
                }
                nir::program::BitSelection::Window { start, end, width } => {
                    let lower = match self.eval(start, environment)? {
                        Flow::Value(value) => value,
                        control => return Ok(SelectedBitRange::Control(control)),
                    };
                    let upper = match self.eval(end, environment)? {
                        Flow::Value(value) => value,
                        control => return Ok(SelectedBitRange::Control(control)),
                    };
                    if !self.has_type(&lower, &start.ty) || !self.has_type(&upper, &end.ty) {
                        return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                    }
                    let lo = position(lower).ok_or_else(|| {
                        self.failure(ErrorKind::IndexOutOfBounds, Some(start.source))
                    })?;
                    let hi = position(upper).ok_or_else(|| {
                        self.failure(ErrorKind::IndexOutOfBounds, Some(end.source))
                    })?;
                    (lo, hi, *width)
                }
            };
            if lo > hi || hi > width {
                return Err(self.failure(ErrorKind::IndexOutOfBounds, Some(expr.source)));
            }
            if hi - lo != expected_width {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            }
            offset = offset
                .checked_add(lo)
                .ok_or_else(|| self.failure(ErrorKind::IndexOutOfBounds, Some(expr.source)))?;
            width = expected_width;
        }
        let end = offset
            .checked_add(width)
            .ok_or_else(|| self.failure(ErrorKind::IndexOutOfBounds, Some(expr.source)))?;

        Ok(SelectedBitRange::Range(offset, end))
    }

    fn replaced_bit_value(
        &mut self,
        source: SourceRange,
        storage_type: &Term,
        target: &Value,
        start: usize,
        rhs: &comptime_runtime::bitvec::BitVec,
    ) -> Result<Value, Error> {
        if !self.has_type(target, storage_type) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(match target {
            Value::BitVec(bits) => Value::BitVec(
                self.bit_operation(source, |budget| bits.replaced(start, rhs, budget))?,
            ),
            Value::SignedInteger(_)
            | Value::UnsignedInteger(_)
            | Value::Float32(_)
            | Value::Float64(_) => {
                let Term::Primitive(primitive) = storage_type else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let Some(width) = primitive.unrestricted_scalar_width_in(self.target) else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let raw = match target {
                    Value::SignedInteger(value) => *value as u128,
                    Value::UnsignedInteger(value) => *value,
                    Value::Float32(bits) => u128::from(*bits),
                    Value::Float64(bits) => u128::from(*bits),
                    _ => unreachable!(),
                };
                let raw = if width == 128 {
                    raw
                } else {
                    raw & ((1u128 << width) - 1)
                };
                let raw = self.bit_operation(source, |budget| {
                    let bits = comptime_runtime::Integer::from_u128(raw)
                        .to_bitvec(width as usize, budget)?;
                    let result = bits.replaced(start, rhs, budget)?;
                    budget.consume(result.words().len())?;
                    Ok(result
                        .words()
                        .iter()
                        .rev()
                        .fold(0u128, |acc, word| (acc << 32) | u128::from(*word)))
                })?;
                if *primitive == Primitive::F32 {
                    Value::Float32(raw as u32)
                } else if *primitive == Primitive::F64 {
                    Value::Float64(raw as u64)
                } else if self
                    .integer_layout(*primitive)
                    .is_some_and(|(signed, _)| signed)
                {
                    let shift = 128 - width;
                    Value::SignedInteger(((raw << shift) as i128) >> shift)
                } else {
                    Value::UnsignedInteger(raw)
                }
            }
            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
        })
    }

    #[inline(never)]
    fn eval_bit_write_pointer(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::BitWritePointer {
            pointer,
            selections,
            value,
        } = &expr.kind
        else {
            unreachable!()
        };
        let Term::Pointer(storage_type) = &pointer.ty else {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        };
        self.charge_preparation(selections.len())?;
        if nir::program::bit_update_error_in(
            storage_type,
            selections,
            &value.ty,
            &expr.ty,
            self.target,
        )
        .is_some()
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let target_value = value!(self.eval(pointer, environment));
        if !self.has_type(&target_value, &pointer.ty) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let Value::Pointer(target) = target_value else {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        };
        let (start, end) =
            match self.eval_bit_selections(expr, storage_type, selections, environment)? {
                SelectedBitRange::Range(start, end) => (start, end),
                SelectedBitRange::Control(control) => return Ok(control),
            };
        let Value::BitVec(rhs) = value!(self.eval(value, environment)) else {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        };
        if rhs.width() != end - start || expr.ty != Term::Primitive(Primitive::Void) {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let changed = target
            .inspect(|old| self.replaced_bit_value(expr.source, storage_type, old, start, &rhs))
            .map_err(|kind| self.failure(kind, Some(expr.source)))??;
        target
            .write(changed, |old| self.has_type(old, storage_type))
            .map_err(|kind| self.failure(kind, Some(expr.source)))?;
        Ok(Flow::Value(Value::Unit))
    }

    #[inline(never)]
    fn eval_bit_assign(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let (place, storage_type, start, end, expression) = match &expr.kind {
            K::BitAssign {
                place,
                storage_type,
                start,
                end,
                value,
            } => (place, storage_type, *start, *end, value.as_ref()),
            K::BitUpdate {
                place,
                storage_type,
                selections,
                value,
            } => {
                if !environment.contains_key(&place.root) {
                    return Err(self.failure(ErrorKind::MissingBinding, Some(expr.source)));
                }
                self.charge_preparation(selections.len())?;
                if nir::program::bit_update_error_in(
                    storage_type,
                    selections,
                    &value.ty,
                    &expr.ty,
                    self.target,
                )
                .is_some()
                {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let (start, end) =
                    match self.eval_bit_selections(expr, storage_type, selections, environment)? {
                        SelectedBitRange::Range(start, end) => (start, end),
                        SelectedBitRange::Control(control) => return Ok(control),
                    };
                (place, storage_type, start, end, value.as_ref())
            }
            _ => unreachable!("bit writeback dispatcher"),
        };
        let result = {
            if !environment.contains_key(&place.root) {
                return Err(self.failure(ErrorKind::MissingBinding, Some(expr.source)));
            }
            // Finish the RHS before reading preserved bits. RHS writes to
            // this root or sibling fields must survive the final update.
            let Value::BitVec(rhs) = value!(self.eval(expression, environment)) else {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            };
            if start > end
                || end - start != rhs.width()
                || expr.ty != Term::Primitive(Primitive::Void)
            {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            }
            let storage = match environment.get(&place.root) {
                Some(Value::Storage(storage)) => Some(storage.clone()),
                _ => None,
            };
            let mut locked = storage
                .as_ref()
                .map(|storage| {
                    storage
                        .0
                        .lock()
                        .map_err(|_| self.failure(ErrorKind::InvalidPointer, Some(expr.source)))
                })
                .transpose()?;
            let mut target = match &mut locked {
                Some(value) => &mut **value,
                None => environment.get_mut(&place.root).unwrap(),
            };
            for (expected, index) in &place.fields {
                target = match (expected, target) {
                    (
                        nir::program::FieldOwner::Nominal(expected),
                        Value::Record {
                            declaration,
                            fields,
                            ..
                        },
                    ) => {
                        if declaration == expected {
                            fields.get_mut(*index)
                        } else {
                            None
                        }
                    }
                    (nir::program::FieldOwner::Anonymous, Value::AnonymousRecord(fields)) => {
                        fields.get_mut(*index).map(|(_, value)| value)
                    }
                    _ => None,
                }
                .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
            }
            let changed =
                self.replaced_bit_value(expr.source, storage_type, target, start, &rhs)?;
            // Commit only after all checks and bounded construction succeed.
            *target = changed;
            Value::Unit
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_assign(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::Assign {
                place,
                value: expression,
            } => {
                if !environment.contains_key(&place.root) {
                    return Err(self.failure(ErrorKind::MissingBinding, Some(expr.source)));
                }
                let value = value!(self.eval(expression, environment));
                self.cleanup_site(
                    expr.source,
                    iir::cleanup::Boundary::Replacement,
                    environment,
                )?;
                // Resolve again after RHS evaluation: a write performed by the
                // RHS to a sibling or the root must not be lost to a snapshot.
                let storage = match environment.get(&place.root) {
                    Some(Value::Storage(storage)) => Some(storage.clone()),
                    _ => None,
                };
                if let Some(storage) = storage {
                    let pointer = storage
                        .assignment_pointer(expression.ty.clone(), &place.fields)
                        .map_err(|e| self.failure(e, Some(expr.source)))?;
                    pointer
                        .write(value, |_| true)
                        .map_err(|e| self.failure(e, Some(expr.source)))?;
                    return Ok(Flow::Value(Value::Unit));
                }
                let mut target = environment.get_mut(&place.root).unwrap();
                for (expected, index) in &place.fields {
                    target = match (expected, target) {
                        (
                            nir::program::FieldOwner::Nominal(expected),
                            Value::Record {
                                declaration,
                                fields,
                                ..
                            },
                        ) => {
                            if declaration == expected {
                                fields.get_mut(*index)
                            } else {
                                None
                            }
                        }
                        (nir::program::FieldOwner::Anonymous, Value::AnonymousRecord(fields)) => {
                            fields.get_mut(*index).map(|(_, value)| value)
                        }
                        _ => None,
                    }
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
                }
                *target = value;
                Value::Unit
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_loop(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        match &expr.kind {
            K::Loop { identity, body } => self.with_effect_loop(*identity, expr.source, |this| {
                loop {
                    match this.eval(body, environment)? {
                        Flow::Value(_) => {}
                        Flow::Break(target) if target == *identity => {
                            return Ok(Flow::Value(Value::Unit));
                        }
                        Flow::Continue(target) if target == *identity => {}
                        transfer => return Ok(transfer),
                    }
                }
            }),
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        }
    }

    #[inline(never)]
    fn eval_let(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::Let {
                binding,
                value: expression,
            } => {
                let value = value!(self.eval(expression, environment));
                environment.insert(*binding, value);
                Value::Unit
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_check_predicate(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let (K::RequirePredicate(condition)
        | K::EnsurePredicate(condition)
        | K::CheckPredicate(condition)) = &expr.kind
        else {
            unreachable!()
        };
        if expr.ty != Term::Primitive(Primitive::Void)
            || !nir::program::checkable_predicate(condition)
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let Value::Bool(passed) =
            Self::concrete_value(value!(self.eval(condition, environment)), expr.source)?
        else {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        };
        self.events.push(Event::CheckPredicate {
            source: expr.source,
            passed,
        });
        if !passed {
            self.cleanup_site(
                condition.source,
                iir::cleanup::Boundary::PredicateFailure,
                environment,
            )?;
            return Err(self.failure(ErrorKind::ProgramContractRejected, Some(expr.source)));
        }
        Ok(Flow::Value(Value::Unit))
    }

    #[inline(never)]
    fn eval_discard(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::Discard(expression) => {
                let value = value!(self.eval(expression, environment));
                self.cleanup_discard(expr.source, expression.source, value)?;
                Value::Unit
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_return(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        match &expr.kind {
            K::Return(expression) => {
                let value = if let Some(expression) = expression {
                    value!(self.eval(expression, environment))
                } else {
                    Value::Unit
                };
                self.cleanup_site(
                    expr.source,
                    iir::cleanup::Boundary::TemporaryEnd,
                    environment,
                )?;
                self.check_scheduled_result(&value, expr.source, environment)?;
                self.cleanup_site(expr.source, iir::cleanup::Boundary::Return, environment)?;
                return Ok(Flow::Return(value));
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        }
    }

    #[inline(never)]
    fn eval_if(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let condition = value!(self.eval(condition, environment));
                match condition {
                    Value::Bool(true) => value!(self.eval(then_branch, environment)),
                    Value::Bool(false) => {
                        if let Some(branch) = else_branch {
                            value!(self.eval(branch, environment))
                        } else {
                            Value::Unit
                        }
                    }
                    Value::AbstractValue { .. } => {
                        return Err(
                            self.failure(ErrorKind::AbstractValueUnresolved, Some(expr.source))
                        );
                    }
                    _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                }
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_bit_slice(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::BitWindow { value, start, end } => {
                let Value::BitVec(bits) =
                    Self::concrete_value(value!(self.eval(value, environment)), expr.source)?
                else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let lower =
                    Self::concrete_value(value!(self.eval(start, environment)), expr.source)?;
                let upper = Self::concrete_value(value!(self.eval(end, environment)), expr.source)?;
                if !self.has_type(&lower, &start.ty)
                    || !self.has_type(&upper, &end.ty)
                    || value.ty != Term::Primitive(Primitive::BitVec(bits.width()))
                {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let position = |value: Value| {
                    match value {
                        Value::Integer(n) => n.to_u128().ok(),
                        Value::UnsignedInteger(n) => Some(n),
                        Value::SignedInteger(n) => u128::try_from(n).ok(),
                        _ => None,
                    }
                    .and_then(|n| usize::try_from(n).ok())
                };
                let (Some(start), Some(end)) = (position(lower), position(upper)) else {
                    return Err(self.failure(ErrorKind::IndexOutOfBounds, Some(expr.source)));
                };
                if start > end || end > bits.width() {
                    return Err(self.failure(ErrorKind::IndexOutOfBounds, Some(expr.source)));
                }
                if expr.ty != Term::Primitive(Primitive::BitVec(end - start)) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                Value::BitVec(
                    self.bit_operation(expr.source, |budget| bits.slice(start, end, budget))?,
                )
            }
            K::BitIndex(value, index) => {
                let Value::BitVec(bits) =
                    Self::concrete_value(value!(self.eval(value, environment)), expr.source)?
                else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let position =
                    Self::concrete_value(value!(self.eval(index, environment)), expr.source)?;
                if !self.has_type(&position, &index.ty)
                    || value.ty != Term::Primitive(Primitive::BitVec(bits.width()))
                    || expr.ty != Term::Primitive(Primitive::BitVec(1))
                {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let position = match position {
                    Value::Integer(n) => n.to_u128().ok(),
                    Value::UnsignedInteger(n) => Some(n),
                    Value::SignedInteger(n) => u128::try_from(n).ok(),
                    _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                }
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n < bits.width())
                .ok_or_else(|| self.failure(ErrorKind::IndexOutOfBounds, Some(expr.source)))?;
                Value::BitVec(self.bit_operation(expr.source, |budget| {
                    bits.slice(position, position + 1, budget)
                })?)
            }
            K::BitSlice { value, start, end } => {
                let Value::BitVec(bits) =
                    Self::concrete_value(value!(self.eval(value, environment)), expr.source)?
                else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let result =
                    self.bit_operation(expr.source, |budget| bits.slice(*start, *end, budget))?;
                if expr.ty != Term::Primitive(Primitive::BitVec(result.width())) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                Value::BitVec(result)
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_bit_equal(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::BitEqual(a, b) => {
                let Value::BitVec(a) =
                    Self::concrete_value(value!(self.eval(a, environment)), expr.source)?
                else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let Value::BitVec(b) =
                    Self::concrete_value(value!(self.eval(b, environment)), expr.source)?
                else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                if a.width() != b.width() || expr.ty != Term::Primitive(Primitive::Bool) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                Value::Bool(self.bit_operation(expr.source, |budget| {
                    budget.consume(a.words().len().saturating_add(1))?;
                    Ok(a == b)
                })?)
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_float_arithmetic(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let never = Term::Primitive(Primitive::Never);
        let supported = |ty: &Term| {
            matches!(
                ty,
                Term::Primitive(Primitive::F32 | Primitive::F64 | Primitive::Never)
            )
        };
        match &expr.kind {
            K::FloatNegate(value) => {
                if (!supported(&value.ty) && value.ty != Term::Primitive(Primitive::Float))
                    || expr.ty != value.ty
                {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let result =
                    Self::concrete_value(value!(self.eval(value, environment)), expr.source)?;
                if !self.has_type(&result, &value.ty) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                Ok(Flow::Value(match result {
                    // Negation flips the sign even for zero and NaN, without
                    // arithmetic rounding or payload canonicalization.
                    Value::Float(value) => Value::Float(
                        self.integer_operation(Some(expr.source), |budget| value.negate(budget))?,
                    ),
                    Value::Float32(bits) => Value::Float32(bits ^ (1 << 31)),
                    Value::Float64(bits) => Value::Float64(bits ^ (1 << 63)),
                    _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                }))
            }
            K::FloatBinary { op, left, right } => {
                let expected = if left.ty == never || right.ty == never {
                    &never
                } else {
                    &left.ty
                };
                let exact = !matches!(op, nir::program::FloatOp::Divide);
                let operand_supported = |ty: &Term| {
                    supported(ty) || (exact && *ty == Term::Primitive(Primitive::Float))
                };
                if &expr.ty != expected
                    || !operand_supported(&left.ty)
                    || !operand_supported(&right.ty)
                    || (left.ty != never && right.ty != never && left.ty != right.ty)
                {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let a = Self::concrete_value(value!(self.eval(left, environment)), expr.source)?;
                let b = Self::concrete_value(value!(self.eval(right, environment)), expr.source)?;
                if !self.has_type(&a, &left.ty) || !self.has_type(&b, &right.ty) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                Ok(Flow::Value(match (a, b) {
                    (Value::Float(a), Value::Float(b)) => Value::Float(self.integer_operation(
                        Some(expr.source),
                        |budget| match op {
                            nir::program::FloatOp::Add => a.add(&b, budget),
                            nir::program::FloatOp::Subtract => a.subtract(&b, budget),
                            nir::program::FloatOp::Multiply => a.multiply(&b, budget),
                            nir::program::FloatOp::Divide => {
                                unreachable!("unsupported exact division")
                            }
                        },
                    )?),
                    (Value::Float32(a), Value::Float32(b)) => Value::Float32(op.evaluate_f32(a, b)),
                    (Value::Float64(a), Value::Float64(b)) => Value::Float64(op.evaluate_f64(a, b)),
                    _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                }))
            }
            _ => unreachable!(),
        }
    }

    #[inline(never)]
    fn eval_machine_integer(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
        primitive: Primitive,
    ) -> Result<Flow, Error> {
        use comptime_runtime::machine::{Error as MError, MachineInteger};
        let (signed, width) = self
            .integer_layout(primitive)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
        let (left, right, op) = match &expr.kind {
            K::IntegerNegate(value) => (value.as_ref(), None, None),
            K::IntegerBinary { op, left, right }
                if matches!(
                    op,
                    nir::program::IntegerOp::Add
                        | nir::program::IntegerOp::Subtract
                        | nir::program::IntegerOp::Multiply
                        | nir::program::IntegerOp::DivideTrunc
                        | nir::program::IntegerOp::RemainderTrunc
                ) =>
            {
                (left.as_ref(), Some(right.as_ref()), Some(*op))
            }
            _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
        };
        let target = Term::Primitive(primitive);
        let never = Term::Primitive(Primitive::Never);
        let result_type = if left.ty == never || right.is_some_and(|v| v.ty == never) {
            &never
        } else {
            &target
        };
        if &expr.ty != result_type
            || (left.ty != target && left.ty != never)
            || right.is_some_and(|v| v.ty != target && v.ty != never)
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let convert = |value: Value| match value {
            Value::SignedInteger(value) if signed => {
                MachineInteger::from_signed(value, width as u16)
            }
            Value::UnsignedInteger(value) if !signed => {
                MachineInteger::from_unsigned(value, width as u16)
            }
            _ => Err(MError::RepresentationMismatch),
        };
        let left = convert(Self::concrete_value(
            value!(self.eval(left, environment)),
            expr.source,
        )?)
        .map_err(|_| self.failure(ErrorKind::Type, Some(expr.source)))?;
        let right = if let Some(right) = right {
            Some(
                convert(Self::concrete_value(
                    value!(self.eval(right, environment)),
                    expr.source,
                )?)
                .map_err(|_| self.failure(ErrorKind::Type, Some(expr.source)))?,
            )
        } else {
            None
        };
        let value = self
            .integer_operation(Some(expr.source), |budget| {
                Ok(match op {
                    Some(nir::program::IntegerOp::Add) => left.add(right.unwrap(), budget),
                    Some(nir::program::IntegerOp::Subtract) => {
                        left.subtract(right.unwrap(), budget)
                    }
                    Some(nir::program::IntegerOp::Multiply) => {
                        left.multiply(right.unwrap(), budget)
                    }
                    Some(nir::program::IntegerOp::DivideTrunc) => {
                        left.div_rem_trunc(right.unwrap(), budget).map(|(q, _)| q)
                    }
                    Some(nir::program::IntegerOp::RemainderTrunc) => {
                        left.div_rem_trunc(right.unwrap(), budget).map(|(_, r)| r)
                    }
                    None => left.negate(budget),
                    _ => unreachable!(),
                })
            })?
            .map_err(|error| {
                self.failure(
                    match error {
                        MError::Overflow => ErrorKind::MachineOverflow,
                        MError::DivisionByZero => ErrorKind::DivisionByZero,
                        MError::Resource(comptime_runtime::Error::BudgetExhausted) => {
                            ErrorKind::StepsExhausted
                        }
                        _ => ErrorKind::Type,
                    },
                    Some(expr.source),
                )
            })?;
        Ok(Flow::Value(if signed {
            Value::SignedInteger(value.signed_value().unwrap())
        } else {
            Value::UnsignedInteger(value.unsigned_value().unwrap())
        }))
    }

    #[inline(never)]
    fn eval_integer_negate(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        if let Some(primitive) = nir::program::machine_integer_type_in(expr, self.target) {
            return self.eval_machine_integer(expr, environment, primitive);
        }
        let result = match &expr.kind {
            K::IntegerNegate(value) => {
                if !matches!(
                    value.ty,
                    Term::Primitive(Primitive::Integer | Primitive::Never)
                ) || value.ty != expr.ty
                {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let Value::Integer(value) =
                    Self::concrete_value(value!(self.eval(value, environment)), expr.source)?
                else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                Value::Integer(
                    self.integer_operation(Some(expr.source), |budget| value.negate(budget))?,
                )
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_integer_binary(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        if let Some(primitive) = nir::program::machine_integer_type_in(expr, self.target) {
            return self.eval_machine_integer(expr, environment, primitive);
        }
        let result = match &expr.kind {
            K::IntegerBinary { op, left, right } => {
                let expected = Term::Primitive(
                    if left.ty == Term::Primitive(Primitive::Never)
                        || right.ty == Term::Primitive(Primitive::Never)
                    {
                        Primitive::Never
                    } else if op.is_comparison() {
                        Primitive::Bool
                    } else {
                        Primitive::Integer
                    },
                );
                if expr.ty != expected
                    || !matches!(
                        left.ty,
                        Term::Primitive(Primitive::Integer | Primitive::Never)
                    )
                    || !matches!(
                        right.ty,
                        Term::Primitive(Primitive::Integer | Primitive::Never)
                    )
                {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let Value::Integer(left) =
                    Self::concrete_value(value!(self.eval(left, environment)), expr.source)?
                else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let Value::Integer(right) =
                    Self::concrete_value(value!(self.eval(right, environment)), expr.source)?
                else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let value = self.integer_operation(Some(expr.source), |budget| {
                    op.evaluate(&left, &right, budget)
                })?;
                match value {
                    Term::Integer(value) => Value::Integer(value),
                    Term::Bool(value) => Value::Bool(value),
                    _ => unreachable!("Integer operation returns a canonical scalar"),
                }
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_bit_convert(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::BitConvert(value) => {
                let Value::BitVec(bits) =
                    Self::concrete_value(value!(self.eval(value, environment)), expr.source)?
                else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                match expr.ty {
                    Term::Primitive(Primitive::BitVec(width)) => Value::BitVec(
                        self.bit_operation(expr.source, |budget| bits.widen(width, budget))?,
                    ),
                    Term::Primitive(primitive) => {
                        let Some(width) = primitive.unrestricted_scalar_width_in(self.target)
                        else {
                            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                        };
                        let widening = self
                            .integer_layout(primitive)
                            .is_some_and(|(signed, _)| !signed);
                        if (width as usize) < bits.width()
                            || (!widening && width as usize != bits.width())
                        {
                            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                        }
                        let value = self.bit_operation(expr.source, |budget| {
                            budget.consume(bits.words().len().saturating_add(1))?;
                            Ok(bits
                                .words()
                                .iter()
                                .rev()
                                .fold(0u128, |acc, word| (acc << 32) | u128::from(*word)))
                        })?;
                        match primitive {
                            Primitive::F32 => Value::Float32(value as u32),
                            Primitive::F64 => Value::Float64(value as u64),
                            _ if self
                                .integer_layout(primitive)
                                .is_some_and(|(signed, _)| signed) =>
                            {
                                let shift = 128 - width;
                                Value::SignedInteger(((value << shift) as i128) >> shift)
                            }
                            _ => Value::UnsignedInteger(value),
                        }
                    }
                    _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                }
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_bit_concat(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::BitConcat(a, b) => {
                let Value::BitVec(a) =
                    Self::concrete_value(value!(self.eval(a, environment)), expr.source)?
                else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let Value::BitVec(b) =
                    Self::concrete_value(value!(self.eval(b, environment)), expr.source)?
                else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let result = self.bit_operation(expr.source, |budget| a.concat(&b, budget))?;
                if expr.ty != Term::Primitive(Primitive::BitVec(result.width())) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                Value::BitVec(result)
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_take_bits(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::TakeBits(operand) => {
                let value =
                    Self::concrete_value(value!(self.eval(operand, environment)), expr.source)?;
                let Term::Primitive(Primitive::BitVec(width)) = expr.ty else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                match value {
                    Value::BitVec(bits) if bits.width() == width => Value::BitVec(bits),
                    value @ Value::Variant { .. } => {
                        if !self.has_type(&value, &operand.ty) {
                            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                        }
                        let Value::Variant {
                            declaration,
                            index,
                            fields,
                            arguments,
                        } = value
                        else {
                            unreachable!()
                        };
                        let definition = if let Some(graph) = self.graph {
                            graph
                                .nominals
                                .iter()
                                .find(|nominal| {
                                    matches!(&nominal.ty,
                                Term::Nominal {declaration:actual,arguments:actual_args}
                                    if *actual == declaration && *actual_args == arguments)
                                })
                                .and_then(|nominal| match &nominal.definition {
                                    iir::NominalDefinition::Enum(definition) => {
                                        Some(generated_enums::EnumDefinition::Borrowed(definition))
                                    }
                                    _ => None,
                                })
                        } else {
                            self.enums.get(&declaration).cloned()
                        };
                        let layout = definition
                            .as_ref()
                            .and_then(|definition| definition.encoding.as_ref())
                            .filter(|layout| layout.width() == width)
                            .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
                        let variant = layout
                            .variants()
                            .get(index)
                            .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
                        Value::BitVec(self.bit_operation(expr.source, |budget| {
                            use comptime_runtime::bitvec::{BitVec, Error as BitError};
                            let mut payloads = Vec::new();
                            for (field, value) in variant.fields().iter().zip(&fields) {
                                let bits = if let Value::BitVec(bits) = value {
                                    BitVec::from_words(bits.width(), bits.words(), budget)?
                                } else {
                                    let raw = match value {
                                        Value::UnsignedInteger(value) => *value,
                                        Value::SignedInteger(value) => *value as u128,
                                        Value::Float32(value) => u128::from(*value),
                                        Value::Float64(value) => u128::from(*value),
                                        _ => return Err(BitError::InvalidRepresentation),
                                    };
                                    if field.width > 128 {
                                        return Err(BitError::InvalidRepresentation);
                                    }
                                    let raw = if field.width == 128 {
                                        raw
                                    } else {
                                        raw & ((1u128 << field.width) - 1)
                                    };
                                    comptime_runtime::Integer::from_u128(raw)
                                        .to_bitvec(field.width, budget)?
                                };
                                payloads.push(bits);
                            }
                            variant
                                .construct(&payloads, budget)
                                .map_err(|error| match error {
                                    nir::enum_encoding::Error::Bits(error) => error,
                                    _ => BitError::InvalidRepresentation,
                                })
                        })?)
                    }
                    Value::SignedInteger(_)
                    | Value::UnsignedInteger(_)
                    | Value::Float32(_)
                    | Value::Float64(_)
                    | Value::Char(_) => {
                        let Term::Primitive(primitive) = operand.ty else {
                            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                        };
                        if primitive
                            .readable_scalar_width_in(self.target)
                            .map(|w| w as usize)
                            != Some(width)
                            || !self.has_type(&value, &operand.ty)
                        {
                            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                        }
                        let raw = match value {
                            Value::SignedInteger(v) => v as u128,
                            Value::UnsignedInteger(v) => v,
                            Value::Float32(bits) => u128::from(bits),
                            Value::Float64(bits) => u128::from(bits),
                            Value::Char(value) => u128::from(u32::from(value)),
                            _ => unreachable!(),
                        };
                        let raw = if width == 128 {
                            raw
                        } else {
                            raw & ((1u128 << width) - 1)
                        };
                        Value::BitVec(self.bit_operation(expr.source, |budget| {
                            comptime_runtime::Integer::from_u128(raw).to_bitvec(width, budget)
                        })?)
                    }
                    _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                }
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    #[inline(never)]
    fn eval_not(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        // A linear Not spine needs no native recursion. Keep every node's
        // execution cost and delivery boundary; only the host stack frames are
        // removed. The enclosing eval already charged the outermost node.
        let mut pending = vec![expr];
        let mut current = expr;
        let mut result = loop {
            let K::Not(operand) = &current.kind else {
                unreachable!("expression dispatcher selected an incompatible evaluator")
            };
            if !matches!(operand.kind, K::Not(_)) {
                break self.eval(operand, environment);
            }
            if let Err(error) = self.tick(Some(operand.source)) {
                break Err(error);
            }
            current = operand;
            pending.push(current);
        };
        while let Some(node) = pending.pop() {
            result = result.and_then(|flow| {
                let Flow::Value(value) = flow else {
                    return Ok(flow);
                };
                let value = match Self::concrete_value(value, node.source)? {
                    Value::Bool(value) => Value::Bool(!value),
                    Value::BitVec(value) => {
                        if node.ty != Term::Primitive(Primitive::BitVec(value.width())) {
                            return Err(self.failure(ErrorKind::Type, Some(node.source)));
                        }
                        Value::BitVec(self.bit_operation(node.source, |budget| value.not(budget))?)
                    }
                    _ => return Err(self.failure(ErrorKind::Type, Some(node.source))),
                };
                Ok(Flow::Value(value))
            });
            if !pending.is_empty() {
                result = self.finish_evaluation(node, environment, result);
            }
        }
        // The enclosing eval delivers the outermost result exactly once.
        result
    }

    #[inline(never)]
    fn eval_and(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        let result = match &expr.kind {
            K::And(a, b) | K::Or(a, b) => {
                let left = Self::concrete_value(value!(self.eval(a, environment)), expr.source)?;
                match left {
                    Value::BitVec(left) => {
                        // BitVec operations are eager, unlike boolean short circuiting.
                        let Value::BitVec(right) =
                            Self::concrete_value(value!(self.eval(b, environment)), expr.source)?
                        else {
                            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                        };
                        if expr.ty != Term::Primitive(Primitive::BitVec(left.width())) {
                            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                        }
                        Value::BitVec(self.bit_operation(expr.source, |budget| {
                            if matches!(expr.kind, K::And(..)) {
                                left.and(&right, budget)
                            } else {
                                left.or(&right, budget)
                            }
                        })?)
                    }
                    Value::Bool(left) => {
                        if (matches!(expr.kind, K::And(..)) && !left)
                            || (matches!(expr.kind, K::Or(..)) && left)
                        {
                            Value::Bool(left)
                        } else {
                            let right = value!(self.eval(b, environment));
                            if !self.has_type(&right, &Term::Primitive(Primitive::Bool)) {
                                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                            }
                            right
                        }
                    }
                    _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                }
            }
            _ => unreachable!("expression dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(result))
    }

    fn copy_interface_program(
        &mut self,
        program: &nir::contracts::ProgramContract,
        parameters: &[nir::DeclarationParameter],
        arguments: &[Term],
        source: SourceRange,
    ) -> Result<nir::contracts::ProgramContract, Error> {
        self.charge_copy_work(Some(source), |fuel| {
            *fuel = fuel
                .checked_sub(program.entry_snapshots.len().saturating_add(1))
                .ok_or(nir::copy_budget::CopyBudgetExhausted::Steps)?;
            for condition in program.preconditions.iter().chain(&program.postconditions) {
                nir::copy_budget::charge_substitution(condition, parameters, arguments, fuel)?;
            }
            Ok(())
        })?;
        program
            .instantiate_parameters(parameters, arguments)
            .map_err(|_| self.failure(ErrorKind::Type, Some(source)))
    }

    #[inline(never)]
    fn prepare_call(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<PreparedCall, Error> {
        macro_rules! prepared_value {
            ($value:expr) => {
                match $value? {
                    Flow::Value(value) => value,
                    control => return Ok(PreparedCall::Control(control)),
                }
            };
        }
        let K::Call {
            callee,
            arguments,
            parameter_bindings,
        } = &expr.kind
        else {
            unreachable!("call preparation")
        };
        let prepared_trait;
        let mut trait_default_inputs = None;
        let callee = match self.prepare_dynamic_trait_instance(callee, environment)? {
            dynamic_instances::PreparedTraitInstance::Ready(method) => {
                if arguments.iter().any(|argument| argument.is_default) {
                    trait_default_inputs = Some(self.trait_default_inputs(&method)?);
                }
                prepared_trait = method;
                &prepared_trait
            }
            dynamic_instances::PreparedTraitInstance::Control(flow) => {
                return Ok(PreparedCall::Control(flow));
            }
            dynamic_instances::PreparedTraitInstance::Unrelated => callee.as_ref(),
        };
        let this = self;
        let interface_program = if let K::TraitMethod {
            method_arguments,
            interface,
            member,
            interface_arguments,
            subject,
        } = &callee.kind
        {
            if let Some(method) = this.iir_method(
                *interface,
                interface_arguments,
                subject,
                *member,
                method_arguments,
            )? {
                this.copy_interface_program(&method.program, &[], &[], expr.source)?
            } else {
                let (parameters, contract) = this
                    .method_contracts
                    .get(&(*interface, *member))
                    .ok_or_else(|| this.failure(ErrorKind::InvalidCallPlan, Some(expr.source)))?;
                this.budget.steps = this
                    .budget
                    .steps
                    .checked_sub(parameters.len() as u64)
                    .ok_or_else(|| this.failure(ErrorKind::StepsExhausted, Some(expr.source)))?;
                let contract = *contract;
                let parameters = parameters.clone();
                let arguments: Vec<_> = std::iter::once(subject.clone())
                    .chain(interface_arguments.clone())
                    .chain(method_arguments.clone())
                    .collect();
                this.copy_interface_program(
                    &contract.program,
                    &parameters,
                    &arguments,
                    expr.source,
                )?
            }
        } else {
            Default::default()
        };
        let interface_parameters = if let K::TraitMethod {
            method_arguments,
            interface,
            member,
            interface_arguments,
            subject,
        } = &callee.kind
        {
            if let Some(method) = this.iir_method(
                *interface,
                interface_arguments,
                subject,
                *member,
                method_arguments,
            )? {
                Some(method.parameter_bindings.clone())
            } else {
                Some(
                    this.method_parameters
                        .get(&(*interface, *member))
                        .ok_or_else(|| this.failure(ErrorKind::InvalidCallPlan, Some(expr.source)))?
                        .clone(),
                )
            }
        } else {
            None
        };
        let dynamic_defaults = matches!(
            callee.kind,
            K::ComptimeFunctionInstance { .. } | K::Local(_)
        );
        let callee_source = callee.source;
        // Static function identities carry no owned callee environment. A
        // synthesized trait-method node can share its enclosing call's source
        // with a handler-input temporary; retaining the function under that
        // source would replace the tuple's cleanup lookup with the wrong value.
        let retain_callee = !matches!(
            callee.kind,
            K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. }
        );
        let callee = prepared_value!(this.eval_in_position(callee, environment, true));
        // Spread preparation can retain an already-created instance in a local.
        // Anonymous closure environments have no declaration substitution.
        let dynamic_defaults = dynamic_defaults
            && matches!(&callee,
            Value::Closure(closure) if closure.declaration_instance().is_some());
        if retain_callee {
            this.retain_residual(callee_source, &callee)?;
        }
        let parameters = match &callee {
            Value::RuntimeFunction { .. } | Value::RuntimeTraitMethod(_) => {
                return Err(this.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
            }
            Value::Function(id) => this
                .functions
                .get(id)
                .ok_or_else(|| this.failure(ErrorKind::UnknownFunction, Some(expr.source)))?
                .parameters
                .clone(),
            Value::Closure(closure) => closure.parameters.clone(),
            _ => return Err(this.failure(ErrorKind::Type, Some(expr.source))),
        };
        let preparation_parameters = interface_parameters.as_ref().unwrap_or(&parameters);
        if !parameter_bindings.is_empty()
            && (parameter_bindings != preparation_parameters
                || parameter_bindings.len() != parameters.len())
        {
            return Err(this.failure(ErrorKind::InvalidCallPlan, Some(expr.source)));
        }
        let mut slots = vec![None; parameters.len()];
        let mut default_scope = Environment::new();
        let mut defaults = false;
        let mut last_default = None;
        for argument in arguments {
            if argument.parameter >= slots.len() || slots[argument.parameter].is_some() {
                return Err(this.failure(ErrorKind::InvalidCallPlan, Some(expr.source)));
            }
            if argument.is_default {
                if parameter_bindings.is_empty()
                    || last_default.is_some_and(|last| last >= argument.parameter)
                {
                    return Err(this.failure(ErrorKind::InvalidCallPlan, Some(expr.source)));
                }
                defaults = true;
                last_default = Some(argument.parameter);
            } else if defaults {
                return Err(this.failure(ErrorKind::InvalidCallPlan, Some(expr.source)));
            }
            let value = if argument.is_default {
                if let Some((binders, inputs)) = &trait_default_inputs {
                    prepared_value!(this.evaluate_bound_default(
                        expr.source,
                        argument,
                        binders,
                        inputs,
                        &mut default_scope,
                        environment
                    ))
                } else if dynamic_defaults {
                    prepared_value!(this.evaluate_instance_default(
                        expr.source,
                        argument,
                        &callee,
                        &mut default_scope,
                        environment
                    ))
                } else {
                    prepared_value!(this.evaluate_default(
                        expr.source,
                        argument,
                        &mut default_scope,
                        environment
                    ))
                }
            } else {
                prepared_value!(this.eval(&argument.value, environment))
            };
            let value = this.prepare_argument(argument.parameter, argument.value.source, value)?;
            if !parameter_bindings.is_empty() {
                default_scope.insert(parameter_bindings[argument.parameter], value.clone());
            }
            slots[argument.parameter] = Some(value);
        }
        let arguments = slots
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| this.failure(ErrorKind::Arity, Some(expr.source)))?;
        // The public interface domain is checked even when the chosen
        // implementation accepts a wider domain. Argument routing uses
        // declaration-side locals, independently of body parameter names.
        if !interface_program.is_empty() {
            for (binding, value) in preparation_parameters.iter().zip(&arguments) {
                let observed = match value {
                    Value::Storage(storage) => this.read_owned_value(storage)?,
                    value => this.checkpoint_representation(value, expr.source)?,
                };
                default_scope.insert(*binding, observed);
            }
            for snapshot in &interface_program.entry_snapshots {
                if !preparation_parameters.contains(&snapshot.parameter) {
                    return Err(this.failure(ErrorKind::InvalidCallPlan, Some(expr.source)));
                }
                let value =
                    this.local_value(&default_scope, snapshot.parameter, Some(expr.source))?;
                default_scope.insert(snapshot.binding, value);
            }
            if !interface_program.postconditions.is_empty() {
                this.require_interface_exit_site(expr.source)?;
            }
            for condition in interface_program.preconditions {
                if this.cleanup_schedule.is_some() {
                    if !nir::program::checkable_predicate(&condition) {
                        return Err(this.failure(ErrorKind::Type, Some(condition.source)));
                    }
                    let Flow::Value(Value::Bool(passed)) =
                        this.eval(&condition, &mut default_scope)?
                    else {
                        return Err(
                            this.failure(ErrorKind::InvalidControlFlow, Some(condition.source))
                        );
                    };
                    this.events.push(Event::CheckPredicate {
                        source: condition.source,
                        passed,
                    });
                    if !passed {
                        this.cleanup_site(
                            expr.source,
                            iir::cleanup::Boundary::InterfacePreconditionFailure,
                            environment,
                        )?;
                        return Err(this
                            .failure(ErrorKind::ProgramContractRejected, Some(condition.source)));
                    }
                    continue;
                }
                let check = Expr {
                    source: condition.source,
                    ty: Term::Primitive(Primitive::Void),
                    kind: K::CheckPredicate(Box::new(condition)),
                };
                if !matches!(
                    this.eval(&check, &mut default_scope)?,
                    Flow::Value(Value::Unit)
                ) {
                    return Err(this.failure(ErrorKind::InvalidControlFlow, Some(check.source)));
                }
            }
        }
        let interface_exit = (!interface_program.postconditions.is_empty()).then(|| {
            Box::new(InterfaceExit {
                conditions: interface_program.postconditions,
                result_binding: interface_program.result_binding,
                environment: default_scope,
                result_slot: arguments.len(),
            })
        });
        let invocation_context = this.invocation_cleanup_context(&callee, expr.source)?;
        Ok(PreparedCall::Invoke {
            interface_exit,
            callee,
            arguments,
            context: invocation_context,
        })
    }
    #[inline(never)]
    fn eval_call(&mut self, expr: &Expr, environment: &mut Environment) -> Result<Flow, Error> {
        self.eval_call_with_effects(expr, environment, None)
    }
    fn eval_call_with_effects(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
        effects: Option<Vec<effects::Entry>>,
    ) -> Result<Flow, Error> {
        self.with_prepared_frame(expr.source, |this| {
            let (callee, arguments, context, interface_exit) =
                match this.prepare_call(expr, environment)? {
                    PreparedCall::Invoke {
                        interface_exit,
                        callee,
                        arguments,
                        context,
                    } => (callee, arguments, context, interface_exit),
                    PreparedCall::Control(control) => return Ok(control),
                };
            this.transfer_prepared();
            // Callee selection and all actual/default arguments have completed
            // in the caller's environment. Only invocation receives the row map.
            let previous =
                effects.map(|row| std::mem::replace(&mut this.effect_state.environment, row));
            let invoked = this.invoke_value_scoped(callee, arguments, context);
            if let Some(previous) = previous {
                this.effect_state.environment = previous;
            }
            match invoked {
                Ok(value) => {
                    if let Some(exit) = interface_exit {
                        this.check_interface_exit(*exit, &value, expr.source, environment)?;
                    }
                    Ok(Flow::Value(value))
                }
                Err(error) => {
                    this.cleanup_effect_failure(&error, expr.source, environment)?;
                    Err(error)
                }
            }
        })
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Optional { ty, value } => {
                write!(f, "(optional-value (type {ty}) ")?;
                match value {
                    Some(value) => write!(f, "(present {value}))"),
                    None => f.write_str("(absent))"),
                }
            }
            Self::Object(object) => object.fmt(f),
            Self::ErrorResult { ty, error, value } => {
                write!(f, "(error-result (type {ty}) (branch ")?;
                match error {
                    Some(index) => write!(f, "(error {index})")?,
                    None => f.write_str("success")?,
                };
                write!(f, ") (value {value}))")
            }
            Self::RuntimeTraitMethod(identity) => identity.fmt(f),
            Self::Newtype { ty, value } => {
                write!(f, "(newtype-value (type {ty}) (representation {value}))")
            }
            Self::RuntimeFunction {
                declaration,
                arguments,
            } => {
                write!(f, "(runtime-function {declaration} (arguments")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str("))")
            }
            Self::Pointer(pointer) => pointer.fmt(f),
            Self::Storage(_) => f.write_str("(internal-storage)"),
            Self::Type(ty) => write!(f, "(type-value {ty})"),
            Self::TypeParameter(id) => write!(f, "(abstract-type-input {id})"),
            Self::AbstractValue { declaration, ty } => {
                write!(f, "(abstract-value-input {declaration} (domain {ty}))")
            }
            Self::Integer(value) => value.fmt(f),
            Self::BitVec(value) => value.fmt(f),
            Self::Float(value) => write!(f, "(comptime-float {value})"),
            Self::Float32(bits) => Literal::Float32(*bits).fmt(f),
            Self::Float64(bits) => Literal::Float64(*bits).fmt(f),
            Self::SignedInteger(value) => write!(f, "(signed-integer {value})"),
            Self::UnsignedInteger(value) => write!(f, "(unsigned-integer {value})"),
            Self::Variant {
                declaration,
                arguments,
                index,
                fields,
            } => {
                write!(f, "(variant {declaration} {index} (type-arguments")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str(") (fields")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str("))")
            }
            Self::AnonymousRecord(fields) => {
                f.write_str("(anonymous-record (fields")?;
                for (name, value) in fields {
                    write!(
                        f,
                        " (field {} {value})",
                        nir::program::Literal::String(name.clone())
                    )?;
                }
                f.write_str("))")
            }
            Self::Record {
                declaration,
                arguments,
                fields,
            } => {
                write!(f, "(record (declaration {declaration}) (type-arguments")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str(") (fields")?;
                for field in fields {
                    write!(f, " {field}")?;
                }
                f.write_str("))")
            }
            Self::Unit => f.write_str("(unit)"),
            Self::Bool(v) => write!(f, "(bool {v})"),
            Self::Char(v) => write!(f, "(char {})", *v as u32),
            Self::String(v) => nir::program::Literal::String(v.to_string()).fmt(f),
            Self::Symbol(v) => nir::program::Literal::Symbol(v.to_string()).fmt(f),
            Self::Function(id) => write!(f, "(function {id})"),
            Self::TypeList(types) => {
                f.write_str("(type-list-value (")?;
                for (index, ty) in types.iter().enumerate() {
                    if index != 0 {
                        f.write_str(" ")?;
                    }
                    write!(f, "{ty}")?;
                }
                f.write_str("))")
            }
            Self::List { ty, values } => {
                write!(f, "(comptime-list (type {ty}) (values")?;
                for value in values {
                    write!(f, " {value}")?;
                }
                f.write_str("))")
            }
            Self::Tuple(values) => {
                f.write_str("(tuple")?;
                for value in values {
                    write!(f, " {value}")?;
                }
                f.write_str(")")
            }
            Self::Closure(closure) => {
                write!(f, "(closure (type {})", closure.ty)?;
                if let Some((declaration, arguments)) = closure.declaration_instance() {
                    write!(f, " (instance (declaration {declaration}) (arguments")?;
                    for argument in arguments {
                        write!(f, " {argument}")?;
                    }
                    f.write_str("))")?;
                }
                f.write_str(" (environment")?;
                for id in &closure.capture_order {
                    write!(f, " (binding {id} {})", closure.environment[id])?;
                }
                f.write_str(") (parameters")?;
                for parameter in &closure.parameters {
                    write!(f, " {parameter}")?;
                }
                write!(f, ") (body {}))", closure.body)
            }
        }
    }
}

#[cfg(test)]
mod pattern_budget_tests;
mod pattern_journal;
#[cfg(test)]
mod pattern_remap_tests;
use pattern_journal::{PatternBindings, PatternJournal};

impl Interpreter<'_> {
    #[cfg(test)]
    fn match_pattern(
        &mut self,
        pattern: &nir::program::Pattern,
        value: &Value,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        self.match_pattern_with_bindings(pattern, value, environment)
            .map(|(flow, _)| flow)
    }
    fn match_pattern_with_bindings(
        &mut self,
        pattern: &nir::program::Pattern,
        value: &Value,
        environment: &mut Environment,
    ) -> Result<(Flow, PatternBindings), Error> {
        let mut journal = PatternJournal::new();
        let flow = self.match_pattern_node(pattern, value, environment, &mut journal)?;
        Ok((flow, journal.bindings))
    }
    fn match_pattern_node(
        &mut self,
        pattern: &nir::program::Pattern,
        value: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        self.tick(None)?;
        let mark = undo.len();
        self.depth += 1;
        let result = self.match_pattern_inner(pattern, value, environment, undo);
        self.depth -= 1;
        if !matches!(result, Ok(Flow::Value(Value::Bool(true)))) {
            undo.rollback(mark, environment);
        }
        result
    }
    fn match_pattern_inner(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        use nir::program::Pattern;
        if matches!(input, Value::AbstractValue { .. })
            && !matches!(
                pattern,
                Pattern::Wild
                    | Pattern::Binding { .. }
                    | Pattern::Guard { .. }
                    | Pattern::Then { .. }
                    | Pattern::Remap { .. }
                    | Pattern::Or(..)
                    | Pattern::Observe { .. }
            )
        {
            return Err(self.failure(ErrorKind::AbstractValueUnresolved, None));
        }
        let abstract_input = matches!(input, Value::TypeParameter(_))
            || matches!(input, Value::Type(Term::Rigid { .. }));
        if abstract_input
            && !matches!(
                pattern,
                Pattern::Wild
                    | Pattern::Binding { .. }
                    | Pattern::Guard { .. }
                    | Pattern::Then { .. }
                    | Pattern::Remap { .. }
                    | Pattern::Or(..)
            )
        {
            return Err(self.failure(ErrorKind::TypeContractUnresolved, None));
        }
        let matched = match pattern {
            Pattern::CodeEvidence { .. } => {
                return Err(self.failure(ErrorKind::TypeContractUnresolved, None));
            }
            Pattern::OptionalAbsent => matches!(input, Value::Optional { value: None, .. }),
            Pattern::OptionalPresent(inner) => {
                let Value::Optional {
                    value: Some(value), ..
                } = input
                else {
                    return Ok(Flow::Value(Value::Bool(false)));
                };
                return self.match_pattern_field(inner, value, environment, undo, Some(0));
            }
            Pattern::ErrorResult { label, inner } => {
                let Value::ErrorResult { ty, error, value } = input else {
                    return Ok(Flow::Value(Value::Bool(false)));
                };
                let Term::ErrorResult { errors, .. } = ty else {
                    return Err(self.failure(ErrorKind::Type, None));
                };
                let selected = match error {
                    None => label.is_none(),
                    Some(index) => {
                        let actual = errors
                            .get(*index)
                            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                        label.as_ref() == Some(actual)
                    }
                };
                if !selected {
                    return Ok(Flow::Value(Value::Bool(false)));
                }
                return self.match_pattern_field(inner, value, environment, undo, Some(0));
            }
            Pattern::Wild => true,
            Pattern::Unit => matches!(input, Value::Unit),
            Pattern::Literal(..) => return self.match_literal(pattern, input, environment, undo),
            Pattern::Observe { binding, inner } => {
                if environment.contains_key(binding) {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                }
                // This alias is not part of the rollback/export journal. Only
                // user bindings produced by the inner pattern can escape.
                environment.insert(*binding, input.clone());
                let result = self.match_pattern_node(inner, input, environment, undo);
                environment.remove(binding);
                return result;
            }
            Pattern::Then { .. } => return self.match_then(pattern, input, environment, undo),
            Pattern::Guard { .. } => return self.match_guard(pattern, input, environment, undo),
            Pattern::Binding { .. } => {
                return self.match_binding(pattern, input, environment, undo);
            }
            Pattern::Remap { .. } => return self.match_remap(pattern, input, environment, undo),
            Pattern::Or(..) => return self.match_or(pattern, input, environment, undo),
            Pattern::NominalType { .. } => {
                return self.match_nominal_type(pattern, input, environment, undo);
            }
            Pattern::NormalForm { .. } => {
                return self.match_normal_form(pattern, input, environment, undo);
            }
            Pattern::TypeTuple(..) => {
                return self.match_type_tuple(pattern, input, environment, undo);
            }
            Pattern::AnonymousRecord { names, fields } => {
                let Value::AnonymousRecord(values) = input else {
                    return Ok(Flow::Value(Value::Bool(false)));
                };
                if names.len() != fields.len() || values.len() != fields.len() {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                for (index, ((name, pattern), (actual, value))) in
                    names.iter().zip(fields).zip(values).enumerate()
                {
                    self.integer_operation(None, |budget| {
                        budget.consume(name.len().saturating_add(actual.len()).saturating_add(1))
                    })?;
                    if name != actual {
                        return Err(self.failure(ErrorKind::Type, None));
                    }
                    if !matches!(
                        value!(self.match_pattern_field(
                            pattern,
                            value,
                            environment,
                            undo,
                            Some(index)
                        )),
                        Value::Bool(true)
                    ) {
                        return Ok(Flow::Value(Value::Bool(false)));
                    }
                }
                true
            }
            Pattern::Tuple(..) | Pattern::Record { .. } | Pattern::Variant { .. } => {
                return self.match_tuple(pattern, input, environment, undo);
            }
        };
        Ok(Flow::Value(Value::Bool(matched)))
    }
    #[inline(never)]
    fn match_literal(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        _environment: &mut Environment,
        _undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        use nir::program::Pattern;
        let matched = match pattern {
            Pattern::Literal(Literal::Integer(spelling)) => {
                if let Value::Integer(actual) = input {
                    let expected = self.integer_literal(spelling, None)?;
                    self.integer_operation(None, |budget| {
                        budget.consume(
                            actual
                                .bit_len()
                                .max(expected.bit_len())
                                .div_ceil(32)
                                .saturating_add(1),
                        )?;
                        Ok(actual == &expected)
                    })?
                } else {
                    false
                }
            }
            Pattern::Literal(Literal::BitVec { width, words }) => {
                if let Value::BitVec(actual) = input {
                    self.integer_operation(None, |budget| {
                        budget.consume(actual.words().len().min(words.len()).saturating_add(1))
                    })?;
                    actual.width() == *width && actual.words() == words
                } else {
                    false
                }
            }
            Pattern::Literal(Literal::Float(spelling)) => {
                if let Value::Float(actual) = input {
                    self.integer_operation(None, |budget| {
                        let expected = comptime_runtime::decimal::DecimalValue::parse_literal(
                            spelling, budget,
                        )?;
                        Ok(actual.compare(&expected, budget)?.is_eq())
                    })?
                } else {
                    false
                }
            }
            Pattern::Literal(Literal::Float32(expected)) => {
                matches!(input, Value::Float32(actual) if f32::from_bits(*actual) == f32::from_bits(*expected))
            }
            Pattern::Literal(Literal::Float64(expected)) => {
                matches!(input, Value::Float64(actual) if f64::from_bits(*actual) == f64::from_bits(*expected))
            }
            Pattern::Literal(Literal::SignedInteger(expected)) => {
                matches!(input, Value::SignedInteger(actual) if actual == expected)
            }
            Pattern::Literal(Literal::UnsignedInteger(expected)) => {
                matches!(input, Value::UnsignedInteger(actual) if actual == expected)
            }
            Pattern::Literal(Literal::Bool(expected)) => {
                matches!(input, Value::Bool(actual) if actual == expected)
            }
            Pattern::Literal(Literal::Char(expected)) => {
                matches!(input, Value::Char(actual) if actual == expected)
            }
            Pattern::Literal(Literal::Symbol(expected))
            | Pattern::Literal(Literal::String(expected)) => {
                let actual = match (pattern, input) {
                    (Pattern::Literal(Literal::Symbol(_)), Value::Symbol(actual))
                    | (Pattern::Literal(Literal::String(_)), Value::String(actual)) => Some(actual),
                    _ => None,
                };
                if let Some(actual) = actual {
                    self.integer_operation(None, |budget| {
                        budget.consume(actual.len().min(expected.len()).saturating_add(1))
                    })?;
                    actual.as_ref() == expected
                } else {
                    false
                }
            }
            _ => unreachable!("pattern dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(Value::Bool(matched)))
    }

    #[inline(never)]
    fn match_then(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        use nir::program::Pattern;
        let matched = match pattern {
            Pattern::Then { first, value, next } => {
                if !matches!(
                    value!(self.match_pattern_node(first, input, environment, undo)),
                    Value::Bool(true)
                ) {
                    false
                } else {
                    let value = value!(self.eval(value, environment));
                    return self.match_pattern_field(next, &value, environment, undo, None);
                }
            }
            _ => unreachable!("pattern dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(Value::Bool(matched)))
    }

    #[inline(never)]
    fn match_guard(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        use nir::program::Pattern;
        let matched = match pattern {
            Pattern::Guard { pattern, condition } => {
                if !matches!(
                    value!(self.match_pattern_node(pattern, input, environment, undo)),
                    Value::Bool(true)
                ) {
                    false
                } else {
                    match Self::concrete_value(
                        value!(self.eval(condition, environment)),
                        condition.source,
                    )? {
                        Value::Bool(value) => value,
                        _ => return Err(self.failure(ErrorKind::Type, Some(condition.source))),
                    }
                }
            }
            _ => unreachable!("pattern dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(Value::Bool(matched)))
    }

    #[inline(never)]
    fn match_binding(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        use nir::program::Pattern;
        let matched = match pattern {
            Pattern::Binding { binding, inner } => {
                if !matches!(
                    value!(self.match_pattern_node(inner, input, environment, undo)),
                    Value::Bool(true)
                ) {
                    false
                } else {
                    undo.insert(*binding, input.clone(), undo.path.clone(), environment);
                    true
                }
            }
            _ => unreachable!("pattern dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(Value::Bool(matched)))
    }

    #[inline(never)]
    fn match_remap(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        use nir::program::Pattern;
        let matched = match pattern {
            Pattern::Remap { pattern, bindings } => {
                let mut sources = BTreeSet::new();
                let mut targets = BTreeSet::new();
                for (from, to) in bindings {
                    self.tick(None)?;
                    if !sources.insert(*from) || !targets.insert(*to) {
                        return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                    }
                }
                let candidate_start = undo.len();
                if !matches!(
                    value!(self.match_pattern_node(pattern, input, environment, undo)),
                    Value::Bool(true)
                ) {
                    false
                } else {
                    // Only bindings produced by this successful candidate may
                    // be renamed. Ambient locals and bindings rolled back by
                    // an earlier alternative are not candidate exports.
                    let mut exports = BTreeSet::new();
                    for (id, _, _, _) in &undo.changes[candidate_start..] {
                        self.tick(None)?;
                        if environment.contains_key(id) && undo.bindings.0.contains_key(id) {
                            exports.insert(*id);
                        }
                    }
                    for source in &sources {
                        self.tick(None)?;
                        if !exports.contains(source) {
                            return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                        }
                    }
                    for target in &targets {
                        self.tick(None)?;
                        if exports.contains(target) && !sources.contains(target) {
                            return Err(self.failure(ErrorKind::InvalidCallPlan, None));
                        }
                    }
                    let values = bindings
                        .iter()
                        .map(|(from, to)| {
                            let rank =
                                undo.bindings.1.get(from).copied().ok_or_else(|| {
                                    self.failure(ErrorKind::InvalidCallPlan, None)
                                })?;
                            environment
                                .get(from)
                                .cloned()
                                .map(|v| {
                                    (*to, v, undo.bindings.0.get(from).cloned().flatten(), rank)
                                })
                                .ok_or_else(|| self.failure(ErrorKind::MissingBinding, None))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    for (id, value, origin, rank) in values {
                        undo.insert_ranked(id, value, origin, rank, environment);
                    }
                    for (from, _) in bindings {
                        if !targets.contains(from) {
                            undo.remove(*from, environment);
                        }
                    }
                    true
                }
            }
            _ => unreachable!("pattern dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(Value::Bool(matched)))
    }

    #[inline(never)]
    fn match_or(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        use nir::program::Pattern;
        let matched = match pattern {
            Pattern::Or(patterns) => {
                let mut found = false;
                for pattern in patterns {
                    if matches!(
                        value!(self.match_pattern_node(pattern, input, environment, undo)),
                        Value::Bool(true)
                    ) {
                        found = true;
                        break;
                    }
                }
                found
            }
            _ => unreachable!("pattern dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(Value::Bool(matched)))
    }

    fn match_nominal_type(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        let nir::program::Pattern::NominalType {
            declaration,
            fields,
        } = pattern
        else {
            unreachable!()
        };
        let Value::Type(Term::Nominal {
            declaration: actual,
            arguments,
        }) = input
        else {
            return Ok(Flow::Value(Value::Bool(false)));
        };
        if actual != declaration {
            return Ok(Flow::Value(Value::Bool(false)));
        }
        let struct_definition = self.structs.get(declaration).cloned();
        let enum_definition = self.enums.get(declaration).cloned();
        let (binders, domains) = if let Some(def) = struct_definition.as_ref() {
            (def.parameter_binders(), &def.parameter_types)
        } else if let Some(def) = enum_definition.as_ref() {
            (def.parameter_binders(), &def.parameter_types)
        } else if let Some(def) = self.newtypes.get(declaration).copied() {
            (def.parameter_binders(), &def.parameter_types)
        } else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        let binders = binders.map_err(|_| self.failure(ErrorKind::Type, None))?;
        if fields.len() != arguments.len() || binders.len() != arguments.len() {
            return Err(self.failure(ErrorKind::Type, None));
        }
        // Domain substitution is based on the completed telescope, including captures.
        for (index, (pattern, argument)) in fields.iter().zip(arguments).enumerate() {
            self.tick(None)?;
            let domain = domains
                .get(index)
                .unwrap_or(&Term::Primitive(Primitive::Type));
            let before = self.budget.steps as usize;
            let mut remaining = before;
            let domain = nir::type_computations::instantiate_parameters_bounded(
                domain,
                &binders,
                arguments,
                &[],
                &mut remaining,
            );
            self.budget.steps -= (before - remaining) as u64;
            let domain = domain.map_err(|e| {
                self.failure(
                    if matches!(e, nir::type_computations::Error::Budget) {
                        ErrorKind::StepsExhausted
                    } else {
                        ErrorKind::Type
                    },
                    None,
                )
            })?;
            let value = self.normal_pattern_value(argument, &domain)?;
            if !matches!(
                value!(self.match_pattern_field(pattern, &value, environment, undo, None)),
                Value::Bool(true)
            ) {
                return Ok(Flow::Value(Value::Bool(false)));
            }
        }
        Ok(Flow::Value(Value::Bool(true)))
    }

    #[inline(never)]
    fn match_normal_form(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        let nir::program::Pattern::NormalForm {
            head,
            arguments: patterns,
        } = pattern
        else {
            unreachable!()
        };
        let metadata = self
            .normal_forms
            .get(head)
            .copied()
            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
        if patterns.len() != metadata.parameters.len() {
            return Err(self.failure(ErrorKind::Type, None));
        }
        let Value::Type(input) = input else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        let Term::NormalForm {
            head: actual,
            arguments,
            ..
        } = input
        else {
            return Ok(Flow::Value(Value::Bool(false)));
        };
        // This compares retained declaration provenance, not result-type equality.
        if actual != head {
            return Ok(Flow::Value(Value::Bool(false)));
        }
        if arguments.len() != patterns.len() {
            return Err(self.failure(ErrorKind::Type, None));
        }
        let work = self
            .type_description_work(input)
            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
        self.integer_operation(None, |budget| budget.consume(work))?;
        let mut type_parameters = Vec::new();
        let mut type_arguments = Vec::new();
        for ((pattern, argument), parameter) in
            patterns.iter().zip(arguments).zip(&metadata.parameters)
        {
            let ty = parameter
                .ty
                .instantiate_parameters(&type_parameters, &type_arguments)
                .map_err(|_| self.failure(ErrorKind::Type, None))?;
            type_parameters.push(nir::DeclarationParameter {
                declaration: parameter.declaration,
                sort: if parameter.kind == nir::normal_forms::Kind::Type {
                    nir::Sort::Type
                } else {
                    nir::Sort::ComptimeValue
                },
            });
            type_arguments.push(argument.clone());
            if matches!(pattern, nir::program::Pattern::Wild) {
                continue;
            }
            let value = self.normal_pattern_value(argument, &ty)?;
            if !matches!(
                value!(self.match_pattern_field(pattern, &value, environment, undo, None)),
                Value::Bool(true)
            ) {
                return Ok(Flow::Value(Value::Bool(false)));
            }
        }
        Ok(Flow::Value(Value::Bool(true)))
    }

    /// Invoke with an already evaluated value graph. Check every advertised
    /// domain before executing code, without reconstructing closure storage.
    pub fn invoke_with_typed_values(
        &mut self,
        function: DeclarationId,
        arguments: Vec<(Term, Value)>,
    ) -> Result<Value, Error> {
        self.invoke_with_prepared_arguments(
            function,
            arguments
                .into_iter()
                .map(|(ty, value)| (ty, InvocationValue::Live(value)))
                .collect(),
        )
    }

    /// Mix completed graph inputs with frozen inputs. Validate every live
    /// domain before running a normal-input factory or the target body.
    pub fn invoke_with_prepared_arguments(
        &mut self,
        function: DeclarationId,
        arguments: Vec<(Term, InvocationValue)>,
    ) -> Result<Value, Error> {
        for (ty, value) in &arguments {
            if let InvocationValue::Live(value) = value {
                let mut fuel = usize::try_from(self.remaining_steps()).unwrap_or(usize::MAX);
                let before = fuel;
                let bounded = nir::type_computations::charge_term(ty, &mut fuel);
                self.charge_preparation(before - fuel)?;
                bounded.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
                ty.check_scope(&[])
                    .map_err(|_| self.failure(ErrorKind::Type, None))?;
                if ty.sort() != nir::Sort::Type {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                if let Value::Storage(storage) = value {
                    // Prepared calls may retain owned roots between invocations.
                    // Validate the payload without copying the cell or turning a
                    // borrowed pointer into an owner. Check responsibility while
                    // borrowing, before any argument factory or body executes.
                    let mut fuel = self.remaining_steps();
                    let checked = (|| {
                        let contents = storage.contents_for_copy(&mut fuel, 128)?;
                        if self.has_type(contents.value(), ty) {
                            Ok(())
                        } else {
                            Err(ErrorKind::Type)
                        }
                    })();
                    self.budget.steps = fuel;
                    checked.map_err(|kind| self.failure(kind, None))?;
                } else if !self.has_type(value, ty) {
                    return Err(self.failure(ErrorKind::Type, None));
                }
            }
        }
        let values = arguments
            .into_iter()
            .map(|(ty, value)| match value {
                InvocationValue::Live(value) => Ok(value),
                InvocationValue::Normal(term) => self.normal_pattern_value(&term, &ty),
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.invoke(function, values)
    }

    /// Invoke with typed, canonical normal-form inputs. Reuse the checked
    /// conversion used by normal-form patterns. Abstract Type descriptions need
    /// explicit scope authorization; they never become concrete runtime inputs.
    pub fn invoke_with_normal_arguments(
        &mut self,
        function: DeclarationId,
        arguments: Vec<(Term, Term)>,
    ) -> Result<Value, Error> {
        let values = arguments
            .iter()
            .map(|(ty, term)| self.normal_pattern_value(term, ty))
            .collect::<Result<Vec<_>, _>>()?;
        self.invoke(function, values)
    }

    fn is_type_list_input(&self, ty: &Term) -> bool {
        nir::normal_forms::type_list_input_type(ty)
            && self.normal_forms.values().any(|head| {
                head.is_type_list_instance(ty)
                    || head.parameters.iter().any(|parameter| {
                        parameter.kind == nir::normal_forms::Kind::TypeList && parameter.ty == *ty
                    })
            })
    }

    fn list_element<'a>(&self, ty: &'a Term) -> Option<&'a Term> {
        let Term::NormalForm { head, .. } = ty else {
            return None;
        };
        self.normal_forms.get(head)?.list_element(ty)
    }

    fn tuple_input_elements<'a>(&self, ty: &'a Term) -> Option<&'a [Term]> {
        match ty {
            Term::Tuple(elements) => Some(elements),
            Term::NormalForm { head, .. } => self
                .normal_forms
                .get(head)
                .and_then(|metadata| metadata.tuple_elements(ty)),
            _ => None,
        }
    }

    fn normal_pattern_value(&mut self, term: &Term, ty: &Term) -> Result<Value, Error> {
        self.normal_pattern_value_at(term, ty, 0, true)
    }
    fn normal_pattern_value_at(
        &mut self,
        term: &Term,
        ty: &Term,
        depth: usize,
        reconstruct_callables: bool,
    ) -> Result<Value, Error> {
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, None));
        }
        self.tick(None)?;
        if !reconstruct_callables
            && matches!(
                term,
                Term::ClosureSnapshot { .. } | Term::FunctionValue { .. }
            )
        {
            return Err(self.failure(ErrorKind::Type, None));
        }

        if term.has_pending_error_branch() {
            return Err(self.failure(ErrorKind::AbstractValueUnresolved, None));
        }
        if let Term::Rigid {
            declaration,
            sort: nir::Sort::ComptimeValue,
        } = term
        {
            if self.comptime_only && self.abstract_value_parameters.get(declaration) == Some(ty) {
                return Ok(Value::AbstractValue {
                    declaration: *declaration,
                    ty: ty.clone(),
                });
            }
            return Err(self.failure(ErrorKind::Type, None));
        }
        let work = if *ty == Term::Primitive(Primitive::Type) {
            self.type_description_work(term)
                .ok_or_else(|| self.failure(ErrorKind::Type, None))?
        } else {
            match term {
                Term::Integer(value) => value.bit_len().div_ceil(32),
                Term::Float(value) => value
                    .digits()
                    .len()
                    .saturating_add(value.exponent().bit_len().div_ceil(32)),
                Term::String(value) | Term::Symbol(value) => value.len(),
                Term::BitVec(value) => value.words().len(),
                Term::TupleValue(values) | Term::TypeListValue(values) => values.len(),
                Term::OptionalValue { ty, value } => self
                    .type_description_work(ty)
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?
                    .saturating_add(usize::from(value.is_some())),
                Term::ErrorValue { ty, label, .. } => {
                    let mut work = self
                        .type_description_work(ty)
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                    if let Some(label) = label {
                        work = work.saturating_add(
                            self.type_description_work(label)
                                .ok_or_else(|| self.failure(ErrorKind::Type, None))?,
                        );
                    }
                    work.saturating_add(1)
                }
                Term::NominalValue { ty, fields, .. } => fields.len().saturating_add(
                    self.type_description_work(ty)
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?,
                ),
                _ => 0,
            }
        };
        self.integer_operation(None, |budget| budget.consume(work))?;
        let value = if *ty == Term::Primitive(Primitive::Type) {
            Value::Type(term.clone())
        } else {
            match (term, ty) {
                (Term::TypeListValue(elements), ty) if self.is_type_list_input(ty) => {
                    for element in elements {
                        let work = self
                            .type_description_work(element)
                            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                        self.integer_operation(None, |budget| budget.consume(work))?;
                    }
                    Value::TypeList(elements.clone())
                }
                (Term::Bool(value), _) => Value::Bool(*value),
                (Term::Char(value), _) => Value::Char(*value),
                (Term::String(value), _) => Value::String(value.clone()),
                (Term::Symbol(value), _) => Value::Symbol(value.clone()),
                (Term::ClosureSnapshot { factory, arguments }, ty) if ty.is_comptime_callable() => {
                    let function = self
                        .functions
                        .get(factory)
                        .copied()
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                    if function.phase != nir::program::FunctionPhase::Comptime {
                        return Err(self.failure(ErrorKind::Type, None));
                    }
                    for argument in arguments {
                        let work = self
                            .type_description_work(argument)
                            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                        self.integer_operation(None, |budget| budget.consume(work))?;
                    }
                    let signature = function
                        .signature
                        .instantiate_parameters(
                            &self.input_binders(&function.type_parameters),
                            arguments,
                        )
                        .map_err(|_| self.failure(ErrorKind::Type, None))?;
                    if !matches!(signature.callable_signature(), Term::Function { parameters, result }
                        if parameters.is_empty() && result.as_ref() == ty)
                    {
                        return Err(self.failure(ErrorKind::Type, None));
                    }
                    let expression = Expr {
                        source: function.body.source,
                        ty: signature.with_comptime_phase(true),
                        kind: K::FunctionInstance {
                            declaration: *factory,
                            arguments: arguments.clone(),
                        },
                    };
                    let Flow::Value(callable) = self.eval(&expression, &mut Environment::new())?
                    else {
                        return Err(
                            self.failure(ErrorKind::InvalidControlFlow, Some(expression.source))
                        );
                    };
                    self.invoke_value(callable, vec![])?
                }
                (
                    Term::FunctionValue {
                        declaration,
                        arguments,
                    },
                    ty,
                ) if matches!(
                    ty,
                    Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_)
                ) =>
                {
                    let source = if let Some(identity) =
                        self.runtime_function_identities.get(declaration)
                    {
                        if ty.is_comptime_callable() || identity.signature.is_comptime_callable() {
                            return Err(self.failure(ErrorKind::Type, None));
                        }
                        SourceRange { start: 0, end: 0 }
                    } else {
                        let function = self
                            .functions
                            .get(declaration)
                            .copied()
                            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                        if function.phase == nir::program::FunctionPhase::NormalForm
                            || (function.phase == nir::program::FunctionPhase::Comptime)
                                != ty.is_comptime_callable()
                            || function.is_unsafe != ty.is_unsafe_callable()
                        {
                            return Err(self.failure(ErrorKind::Type, None));
                        }
                        function.body.source
                    };
                    for argument in arguments {
                        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                        let before = fuel;
                        let checked = nir::type_computations::charge_term(argument, &mut fuel);
                        self.budget.steps -= (before - fuel) as u64;
                        checked.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
                        argument
                            .check_scope(&[])
                            .map_err(|_| self.failure(ErrorKind::Type, None))?;
                        if argument.sort() == nir::Sort::Type {
                            let work = self
                                .type_description_work(argument)
                                .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                            self.integer_operation(None, |budget| budget.consume(work))?;
                        }
                    }
                    let expression = Expr {
                        source,
                        ty: ty.clone(),
                        kind: K::FunctionInstance {
                            declaration: *declaration,
                            arguments: arguments.clone(),
                        },
                    };
                    match self.eval(&expression, &mut Environment::new())? {
                        Flow::Value(value) => value,
                        _ => {
                            return Err(self
                                .failure(ErrorKind::InvalidControlFlow, Some(expression.source)));
                        }
                    }
                }
                (Term::Integer(value), Term::Primitive(Primitive::Integer)) => {
                    Value::Integer(value.clone())
                }
                (
                    Term::Integer(value),
                    Term::Primitive(
                        Primitive::U8
                        | Primitive::U16
                        | Primitive::U32
                        | Primitive::U64
                        | Primitive::U128
                        | Primitive::Usize,
                    ),
                ) => Value::UnsignedInteger(
                    value
                        .to_u128()
                        .map_err(|_| self.failure(ErrorKind::Type, None))?,
                ),
                (Term::Integer(value), _) => Value::SignedInteger(
                    value
                        .to_i128()
                        .map_err(|_| self.failure(ErrorKind::Type, None))?,
                ),
                (Term::BitVec(value), _) => Value::BitVec(value.clone()),
                (Term::Float(value), Term::Primitive(Primitive::Float)) => {
                    Value::Float(value.clone())
                }
                (Term::Float32(bits), Term::Primitive(Primitive::F32)) => Value::Float32(*bits),
                (Term::Float64(bits), Term::Primitive(Primitive::F64)) => Value::Float64(*bits),
                (Term::Unit, _) => Value::Unit,
                (Term::OptionalValue { ty: actual, value }, Term::Optional(payload))
                    if actual.as_ref() == ty =>
                {
                    Value::Optional {
                        ty: ty.clone(),
                        value: value
                            .as_ref()
                            .map(|value| {
                                self.normal_pattern_value_at(
                                    value,
                                    payload,
                                    depth + 1,
                                    reconstruct_callables,
                                )
                                .map(Box::new)
                            })
                            .transpose()?,
                    }
                }
                (
                    Term::ErrorValue {
                        ty: actual,
                        label,
                        value,
                    },
                    Term::ErrorResult { errors, result },
                ) if actual.as_ref() == ty => {
                    let (error, payload) = match label {
                        None => (None, result.as_ref()),
                        Some(label) => (
                            Some(
                                errors
                                    .iter()
                                    .position(|ty| ty == label.as_ref())
                                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?,
                            ),
                            label.as_ref(),
                        ),
                    };
                    Value::ErrorResult {
                        ty: ty.clone(),
                        error,
                        value: Box::new(self.normal_pattern_value_at(
                            value,
                            payload,
                            depth + 1,
                            reconstruct_callables,
                        )?),
                    }
                }
                (
                    Term::NominalValue {
                        ty: actual,
                        variant: None,
                        fields,
                    },
                    Term::Primitive(Primitive::Object),
                ) if actual.as_ref() == ty => {
                    if !self.comptime_only {
                        return Err(self.failure(ErrorKind::PhaseMismatch, None));
                    }
                    self.integer_operation(None, |budget| budget.consume(fields.len()))?;
                    let mut entries = Vec::with_capacity(fields.len());
                    for entry in fields {
                        let (key, domain, value) = nir::objects::entry_parts(entry)
                            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                        let work = self
                            .type_description_work(domain)
                            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                        self.integer_operation(None, |budget| {
                            budget.consume(work.saturating_add(key.map_or(0, str::len)))
                        })?;
                        let value = ObjectValue {
                            ty: domain.clone(),
                            value: self.normal_pattern_value_at(
                                value,
                                domain,
                                depth + 1,
                                reconstruct_callables,
                            )?,
                        };
                        entries.push(match key {
                            Some(key) => ObjectEntry::Property {
                                key: key.to_owned(),
                                value,
                            },
                            None => ObjectEntry::Value(value),
                        });
                    }
                    Value::Object(Object::from_entries(entries))
                }
                (
                    Term::NominalValue {
                        ty: actual,
                        variant: None,
                        fields,
                    },
                    Term::NormalForm { .. },
                ) if actual.as_ref() == ty => {
                    let element = self
                        .list_element(ty)
                        .cloned()
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                    if element == Term::Primitive(Primitive::Type) {
                        return Err(self.failure(ErrorKind::Type, None));
                    }
                    self.integer_operation(None, |budget| budget.consume(fields.len()))?;
                    let values = fields
                        .iter()
                        .map(|value| {
                            self.normal_pattern_value_at(
                                value,
                                &element,
                                depth + 1,
                                reconstruct_callables,
                            )
                        })
                        .collect::<Result<_, _>>()?;
                    let value = Value::List {
                        ty: ty.clone(),
                        values,
                    };
                    self.validate_immutable_input(&value, ty)?;
                    value
                }
                (
                    Term::NominalValue {
                        ty: actual,
                        variant,
                        fields,
                    },
                    Term::Nominal {
                        declaration,
                        arguments,
                    },
                ) if actual.as_ref() == ty => {
                    self.integer_operation(None, |budget| budget.consume(fields.len()))?;
                    let mut fuel = usize::try_from(self.budget.steps)
                        .unwrap_or(usize::MAX)
                        .min(100_000);
                    let before = fuel;
                    let types = self.normal_value_fields(ty, *variant, &mut fuel);
                    self.budget.steps -= (before - fuel) as u64;
                    let types = types.ok_or_else(|| {
                        self.failure(
                            if fuel == 0 {
                                ErrorKind::StepsExhausted
                            } else {
                                ErrorKind::Type
                            },
                            None,
                        )
                    })?;
                    if fields.len() != types.len() {
                        return Err(self.failure(ErrorKind::Type, None));
                    }
                    let values = fields
                        .iter()
                        .zip(&types)
                        .map(|(value, ty)| {
                            self.normal_pattern_value_at(
                                value,
                                ty,
                                depth + 1,
                                reconstruct_callables,
                            )
                        })
                        .collect::<Result<_, _>>()?;
                    match variant {
                        Some(index) => Value::Variant {
                            declaration: *declaration,
                            arguments: arguments.clone(),
                            index: *index as usize,
                            fields: values,
                        },
                        None if self.graph.map_or_else(
                            || self.newtypes.contains_key(declaration),
                            |graph| {
                                graph.nominals.iter().any(|n| {
                                    n.ty == *ty
                                        && matches!(
                                            n.definition,
                                            iir::NominalDefinition::Newtype(_)
                                        )
                                })
                            },
                        ) =>
                        {
                            let [value]: [Value; 1] = values
                                .try_into()
                                .map_err(|_| self.failure(ErrorKind::Type, None))?;
                            Value::Newtype {
                                ty: ty.clone(),
                                value: Box::new(value),
                            }
                        }
                        None => Value::Record {
                            declaration: *declaration,
                            arguments: arguments.clone(),
                            fields: values,
                        },
                    }
                }
                (Term::RecordValue(values), Term::Record(types)) if values.len() == types.len() => {
                    let mut fields = Vec::new();
                    for ((name, value), (expected_name, ty)) in values.iter().zip(types) {
                        self.integer_operation(None, |budget| budget.consume(name.len()))?;
                        if name != expected_name {
                            return Err(self.failure(ErrorKind::Type, None));
                        }
                        fields.push((
                            name.clone(),
                            self.normal_pattern_value_at(
                                value,
                                ty,
                                depth + 1,
                                reconstruct_callables,
                            )?,
                        ));
                    }
                    Value::AnonymousRecord(fields)
                }
                (Term::TupleValue(values), ty) => {
                    let Some(types) = self.tuple_input_elements(ty) else {
                        return Err(self.failure(ErrorKind::Type, None));
                    };
                    if values.len() != types.len() {
                        return Err(self.failure(ErrorKind::Type, None));
                    }
                    Value::Tuple(
                        values
                            .iter()
                            .zip(types)
                            .map(|(value, ty)| {
                                self.normal_pattern_value_at(
                                    value,
                                    ty,
                                    depth + 1,
                                    reconstruct_callables,
                                )
                            })
                            .collect::<Result<_, _>>()?,
                    )
                }
                _ => return Err(self.failure(ErrorKind::Type, None)),
            }
        };
        if !self.has_type(&value, ty) {
            return Err(self.failure(ErrorKind::Type, None));
        }
        Ok(value)
    }

    #[inline(never)]
    fn match_type_tuple(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        let nir::program::Pattern::TypeTuple(fields) = pattern else {
            unreachable!()
        };
        let Value::Type(ty) = input else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        let types = match ty {
            Term::Tuple(types) => types,
            Term::NormalForm { head, .. } => {
                let Some(types) = self
                    .normal_forms
                    .get(head)
                    .and_then(|metadata| metadata.tuple_elements(ty))
                else {
                    return Err(self.failure(ErrorKind::TypeContractUnresolved, None));
                };
                types
            }
            Term::Primitive(Primitive::Unit) => {
                return Err(self.failure(ErrorKind::TypeContractUnresolved, None));
            }
            Term::Primitive(_)
            | Term::Pointer(_)
            | Term::Function { .. }
            | Term::ComptimeFunction(_) => return Ok(Flow::Value(Value::Bool(false))),
            // Missing shape information is not negative classification evidence.
            // In particular a normal-form body is never inspected here.
            _ => return Err(self.failure(ErrorKind::TypeContractUnresolved, None)),
        };
        if fields.len() != types.len() {
            return Ok(Flow::Value(Value::Bool(false)));
        }
        for (field, ty) in fields.iter().zip(types) {
            let work = self
                .type_description_work(ty)
                .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
            self.integer_operation(None, |budget| budget.consume(work))?;
            if !matches!(
                value!(self.match_pattern_field(
                    field,
                    &Value::Type(ty.clone()),
                    environment,
                    undo,
                    None
                )),
                Value::Bool(true)
            ) {
                return Ok(Flow::Value(Value::Bool(false)));
            }
        }
        Ok(Flow::Value(Value::Bool(true)))
    }

    #[inline(never)]
    fn match_tuple(
        &mut self,
        pattern: &nir::program::Pattern,
        input: &Value,
        environment: &mut Environment,
        undo: &mut PatternJournal,
    ) -> Result<Flow, Error> {
        use nir::program::Pattern;
        let matched = match pattern {
            Pattern::Tuple(fields)
            | Pattern::Record { fields, .. }
            | Pattern::Variant { fields, .. } => {
                let values = match (pattern, input) {
                    (Pattern::Tuple(_), Value::Tuple(values)) => Some(values),
                    (
                        Pattern::Record { declaration, .. },
                        Value::Record {
                            declaration: actual,
                            fields,
                            ..
                        },
                    ) if declaration == actual => Some(fields),
                    (
                        Pattern::Variant {
                            declaration, index, ..
                        },
                        Value::Variant {
                            declaration: actual,
                            index: found,
                            fields,
                            ..
                        },
                    ) if declaration == actual && index == found => Some(fields),
                    _ => None,
                };
                if let Some(values) = values.filter(|values| values.len() == fields.len()) {
                    let mut found = true;
                    for (index, (pattern, value)) in fields.iter().zip(values).enumerate() {
                        if !matches!(
                            value!(self.match_pattern_field(
                                pattern,
                                value,
                                environment,
                                undo,
                                Some(index)
                            )),
                            Value::Bool(true)
                        ) {
                            found = false;
                            break;
                        }
                    }
                    found
                } else {
                    false
                }
            }
            _ => unreachable!("pattern dispatcher selected an incompatible evaluator"),
        };
        Ok(Flow::Value(Value::Bool(matched)))
    }
}

/// Bound shape inspection for a concrete type value. Scoped binders inside a
/// function type are legitimate; free runtime environments/value-store handles
/// and active rigid inputs require elaboration support outside this interpreter.
fn type_value_work(
    ty: &Term,
    heads: &BTreeMap<DeclarationId, &nir::normal_forms::Head>,
) -> Option<usize> {
    scoped_type_value_work(ty, heads, &BTreeSet::new())
}

// Rigid Type inputs are descriptions, never permission to inspect their layout.
fn scoped_type_value_work(
    ty: &Term,
    heads: &BTreeMap<DeclarationId, &nir::normal_forms::Head>,
    parameters: &BTreeSet<DeclarationId>,
) -> Option<usize> {
    scoped_type_value_work_with_layouts(ty, heads, parameters, |_, _, _| None)
}
fn scoped_type_value_work_with_layouts(
    ty: &Term,
    heads: &BTreeMap<DeclarationId, &nir::normal_forms::Head>,
    parameters: &BTreeSet<DeclarationId>,
    nominal_fields: impl Fn(&Term, Option<u32>, &mut usize) -> Option<Vec<Term>>,
) -> Option<usize> {
    scoped_type_value_work_with_evidence(ty, heads, parameters, &BTreeMap::new(), nominal_fields)
}

fn scoped_type_value_work_with_evidence(
    ty: &Term,
    heads: &BTreeMap<DeclarationId, &nir::normal_forms::Head>,
    parameters: &BTreeSet<DeclarationId>,
    value_parameters: &BTreeMap<DeclarationId, Term>,
    nominal_fields: impl Fn(&Term, Option<u32>, &mut usize) -> Option<Vec<Term>>,
) -> Option<usize> {
    scoped_type_value_work_with_evidence_in(
        ty,
        heads,
        parameters,
        value_parameters,
        nominal_fields,
        None,
    )
}

fn scoped_type_value_work_with_evidence_in(
    ty: &Term,
    heads: &BTreeMap<DeclarationId, &nir::normal_forms::Head>,
    parameters: &BTreeSet<DeclarationId>,
    value_parameters: &BTreeMap<DeclarationId, Term>,
    nominal_fields: impl Fn(&Term, Option<u32>, &mut usize) -> Option<Vec<Term>>,
    target: Option<nir::target::TargetData>,
) -> Option<usize> {
    if ty.sort() != nir::Sort::Type {
        return None;
    }
    scoped_term_value_work_with_evidence_in(
        ty,
        heads,
        parameters,
        value_parameters,
        nominal_fields,
        target,
    )
}

// Instance identities contain completed value arguments as well as types.
// Use the same bounded structural validation without imposing a Type root.
fn instance_argument_work(
    argument: &Term,
    heads: &BTreeMap<DeclarationId, &nir::normal_forms::Head>,
) -> Option<usize> {
    scoped_term_value_work_with_layouts(argument, heads, &BTreeSet::new(), |_, _, _| None)
}

fn scoped_term_value_work_with_layouts(
    ty: &Term,
    heads: &BTreeMap<DeclarationId, &nir::normal_forms::Head>,
    parameters: &BTreeSet<DeclarationId>,
    nominal_fields: impl Fn(&Term, Option<u32>, &mut usize) -> Option<Vec<Term>>,
) -> Option<usize> {
    scoped_term_value_work_with_evidence(ty, heads, parameters, &BTreeMap::new(), nominal_fields)
}

fn scoped_term_value_work_with_evidence(
    ty: &Term,
    heads: &BTreeMap<DeclarationId, &nir::normal_forms::Head>,
    parameters: &BTreeSet<DeclarationId>,
    value_parameters: &BTreeMap<DeclarationId, Term>,
    nominal_fields: impl Fn(&Term, Option<u32>, &mut usize) -> Option<Vec<Term>>,
) -> Option<usize> {
    scoped_term_value_work_with_evidence_in(
        ty,
        heads,
        parameters,
        value_parameters,
        nominal_fields,
        None,
    )
}

fn scoped_term_value_work_with_evidence_in(
    ty: &Term,
    heads: &BTreeMap<DeclarationId, &nir::normal_forms::Head>,
    parameters: &BTreeSet<DeclarationId>,
    value_parameters: &BTreeMap<DeclarationId, Term>,
    nominal_fields: impl Fn(&Term, Option<u32>, &mut usize) -> Option<Vec<Term>>,
    target: Option<nir::target::TargetData>,
) -> Option<usize> {
    // A leaf needs neither a heap traversal stack nor an empty shared frame.
    // Allocate pending nodes only for children, and shared binder frames only
    // when entering a function telescope. Logical work charges stay unchanged.
    let mut pending = Vec::new();
    let mut first = Some((ty, 0usize, None::<std::rc::Rc<Vec<&[nir::Parameter]>>>));
    let mut work = 0usize;
    while let Some((ty, depth, frames)) = first.take().or_else(|| pending.pop()) {
        let frame_slice = frames.as_deref().map_or(&[][..], Vec::as_slice);
        work += 1;
        if depth > 128 || work > 100_000 {
            return None;
        }
        // A concatenation is a pending type-list operation, not a completed
        // value identity. Even literal-only parts must first pass through the
        // canonical constructor/substitution that produces TypeListValue.
        if matches!(ty, Term::TypeListConcat(_)) {
            return None;
        }
        let scalar_work = match ty {
            Term::Integer(integer) => integer.bit_len().div_ceil(32),
            Term::Float(value) => value
                .digits()
                .len()
                .saturating_add(value.exponent().bit_len().div_ceil(32)),
            Term::String(value) | Term::Symbol(value) => value.len(),
            Term::BitVec(value) => value.words().len(),
            Term::Record(fields) | Term::RecordValue(fields) => fields
                .iter()
                .try_fold(0usize, |sum, (name, _)| sum.checked_add(name.len()))?,
            _ => 0,
        };
        work = work.checked_add(scalar_work)?;
        if work > 100_000 {
            return None;
        }
        let children = match ty {
            Term::OptionalValue { value, .. } => 1 + usize::from(value.is_some()),
            Term::ErrorValue { .. } => 3,
            Term::NominalValue { fields, .. } => fields.len().saturating_add(1),
            Term::Record(fields) | Term::RecordValue(fields) => fields.len(),
            Term::Tuple(items)
            | Term::ClosureContextValue(items)
            | Term::TypeListValue(items)
            | Term::TypeListConcat(items)
            | Term::TupleValue(items)
            | Term::ClosureSnapshot {
                arguments: items, ..
            }
            | Term::FunctionValue {
                arguments: items, ..
            }
            | Term::Nominal {
                arguments: items, ..
            }
            | Term::NormalForm {
                arguments: items, ..
            } => items.len(),
            Term::ErrorResult { errors, .. } => errors.len().saturating_add(1),
            Term::Computation { effects, .. } => {
                work = work.checked_add(effects.len())?;
                effects
                    .iter()
                    .try_fold(1usize, |sum, op| sum.checked_add(op.arguments.len()))?
            }
            Term::Closure { environment, .. } => environment.len().saturating_add(1),
            Term::Function { parameters, .. } => parameters.len().saturating_add(1),
            Term::Optional(_) | Term::Pointer(_) | Term::ComptimeFunction(_) => 1,
            _ => 0,
        };
        if work.saturating_add(pending.len()).saturating_add(children) > 100_000 {
            return None;
        }
        if let Term::NormalForm {
            head,
            sort,
            arguments,
        } = ty
        {
            if let Some(metadata) = heads.get(head) {
                if *sort != nir::Sort::Type {
                    return None;
                }
                let mut fuel = 100_000usize.checked_sub(work)?;
                let before = fuel;
                let mut bound_domains = BTreeMap::new();
                for (frame_depth, frame) in frame_slice.iter().rev().enumerate() {
                    fuel = fuel.checked_sub(frame.len().saturating_add(1))?;
                    for (index, parameter) in frame.iter().enumerate() {
                        if parameter.sort != nir::Sort::ComptimeValue {
                            continue;
                        }
                        nir::type_computations::charge_term(&parameter.ty, &mut fuel).ok()?;
                        let mut domain = parameter.ty.clone();
                        for _ in 0..frame_depth {
                            nir::type_computations::charge_term(&domain, &mut fuel).ok()?;
                            domain = domain.abstract_parameters(&[]).ok()?;
                        }
                        bound_domains.insert((frame_depth as u32, index), domain);
                    }
                }
                metadata
                    .check_arguments_in_scope_with_collections_budgeted_in(
                        arguments,
                        |head| {
                            heads
                                .get(&head)
                                .is_some_and(|head| head.role == nir::normal_forms::Role::Tuple)
                        },
                        |head| {
                            heads
                                .get(&head)
                                .is_some_and(|head| head.role == nir::normal_forms::Role::TypeList)
                        },
                        &nominal_fields,
                        |value, domain| match value {
                            Term::Rigid {
                                declaration,
                                sort: nir::Sort::ComptimeValue,
                            } => value_parameters.get(declaration) == Some(domain),
                            Term::Bound {
                                depth,
                                index,
                                sort: nir::Sort::ComptimeValue,
                            } => bound_domains.get(&(*depth, *index)) == Some(domain),
                            _ => false,
                        },
                        &frame_slice
                            .iter()
                            .map(|frame| frame.iter().map(|p| p.sort).collect())
                            .collect::<Vec<_>>(),
                        &mut fuel,
                        target,
                    )
                    .ok()?;
                work = work.checked_add(before - fuel)?;
            }
        }
        match ty {
            Term::OptionalValue { ty, value } => {
                pending.push((ty, depth + 1, frames.clone()));
                if let Some(value) = value {
                    pending.push((value, depth + 1, frames.clone()));
                }
            }
            Term::ErrorValue { ty, label, value } => {
                pending.push((ty, depth + 1, frames.clone()));
                if let Some(label) = label {
                    pending.push((label, depth + 1, frames.clone()));
                }
                pending.push((value, depth + 1, frames.clone()));
            }
            Term::NominalValue { ty, fields, .. } => {
                pending.push((ty, depth + 1, frames.clone()));
                pending.extend(
                    fields
                        .iter()
                        .map(|field| (field, depth + 1, frames.clone())),
                );
            }
            Term::Record(fields) | Term::RecordValue(fields) => {
                pending.extend(fields.iter().map(|(_, ty)| (ty, depth + 1, frames.clone())));
            }
            Term::Rigid {
                declaration,
                sort: nir::Sort::Type,
            } if parameters.contains(declaration) => {}
            Term::Rigid {
                declaration,
                sort: nir::Sort::ComptimeValue,
            } if value_parameters.contains_key(declaration) => {}
            Term::AssociatedType { .. }
            | Term::Rigid { .. }
            | Term::Runtime(_)
            | Term::Value { .. }
            | Term::Hole { .. } => {
                return None;
            }
            Term::Tuple(items)
            | Term::ClosureContextValue(items)
            | Term::TypeListValue(items)
            | Term::TypeListConcat(items)
            | Term::TupleValue(items)
            | Term::ClosureSnapshot {
                arguments: items, ..
            }
            | Term::FunctionValue {
                arguments: items, ..
            }
            | Term::Nominal {
                arguments: items, ..
            }
            | Term::NormalForm {
                arguments: items, ..
            } => {
                pending.extend(items.iter().map(|t| (t, depth + 1, frames.clone())));
            }
            Term::ErrorResult { errors, result } => {
                pending.push((result, depth + 1, frames.clone()));
                pending.extend(errors.iter().map(|ty| (ty, depth + 1, frames.clone())));
            }
            Term::Computation { effects, result } => {
                pending.push((result, depth + 1, frames.clone()));
                for operation in effects {
                    pending.extend(
                        operation
                            .arguments
                            .iter()
                            .map(|t| (t, depth + 1, frames.clone())),
                    );
                }
            }
            Term::Closure {
                environment,
                signature,
            } => {
                pending.extend(environment.iter().map(|t| (t, depth + 1, frames.clone())));
                pending.push((signature, depth + 1, frames.clone()));
            }
            Term::Optional(inner)
            | Term::Pointer(inner)
            | Term::ComptimeFunction(inner)
            | Term::UnsafeFunction(inner) => pending.push((inner, depth + 1, frames.clone())),
            Term::Function { parameters, result } => {
                work = work.checked_add(frame_slice.len().saturating_add(1))?;
                if work > 100_000 {
                    return None;
                }
                let mut nested = frame_slice.to_vec();
                nested.push(parameters);
                let frames = Some(std::rc::Rc::new(nested));
                pending.extend(
                    parameters
                        .iter()
                        .map(|p| (&p.ty, depth + 1, frames.clone())),
                );
                pending.push((result, depth + 1, frames.clone()));
            }
            _ => {}
        }
    }
    ty.check_scope(&[]).ok()?;
    Some(work)
}

#[cfg(test)]
mod computation_type_tests {
    use super::*;

    #[test]
    fn callable_bound_value_domains_are_checked_without_admitting_foreign_or_later_inputs() {
        use nir::{Parameter, Sort};
        let id = DeclarationId {
            package: 9,
            local: 1,
        };
        let metadata = nir::normal_forms::Head {
            role: Default::default(),
            declaration: id,
            requirements: vec![],
            parameters: vec![nir::normal_forms::Parameter {
                declaration: DeclarationId {
                    package: 9,
                    local: 2,
                },
                binding: LocalId { owner: 1, index: 0 },
                kind: nir::normal_forms::Kind::Value,
                ty: Term::Primitive(Primitive::Bool),
            }],
        };
        let heads = BTreeMap::from([(id, &metadata)]);
        let head = |depth, index| Term::NormalForm {
            head: id,
            sort: Sort::Type,
            arguments: vec![Term::Bound {
                depth,
                index,
                sort: Sort::ComptimeValue,
            }],
        };
        let parameter = Parameter {
            sort: Sort::ComptimeValue,
            ty: Term::Primitive(Primitive::Bool),
        };
        let valid = Term::Function {
            parameters: vec![parameter.clone()],
            result: Box::new(head(0, 0)),
        };
        assert!(type_value_work(&valid, &heads).is_some());
        let nested = Term::Function {
            parameters: vec![parameter.clone()],
            result: Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(head(1, 0)),
            }),
        };
        assert!(type_value_work(&nested, &heads).is_some());
        for invalid in [
            Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ..parameter.clone()
                }],
                result: Box::new(head(0, 0)),
            },
            Term::Function {
                parameters: vec![Parameter {
                    ty: Term::Primitive(Primitive::U8),
                    ..parameter.clone()
                }],
                result: Box::new(head(0, 0)),
            },
            Term::Function {
                parameters: vec![parameter.clone()],
                result: Box::new(head(0, 1)),
            },
            Term::Function {
                parameters: vec![parameter.clone()],
                result: Box::new(head(1, 0)),
            },
            Term::Function {
                parameters: vec![
                    Parameter {
                        sort: Sort::Runtime,
                        ty: head(0, 1),
                    },
                    parameter,
                ],
                result: Box::new(Term::Primitive(Primitive::Bool)),
            },
        ] {
            assert!(type_value_work(&invalid, &heads).is_none(), "{invalid}");
        }
        assert!(type_value_work(&valid, &heads).is_some());
    }

    #[test]
    fn instance_argument_budget_accepts_completed_values_but_rejects_open_terms() {
        let heads = BTreeMap::new();
        let value = Term::Bool(true);
        assert!(type_value_work(&value, &heads).is_none());
        assert_eq!(instance_argument_work(&value, &heads), Some(1));
        assert!(instance_argument_work(&Term::Primitive(Primitive::Bool), &heads).is_some());
        let open = Term::Rigid {
            declaration: DeclarationId {
                package: 0,
                local: 1,
            },
            sort: nir::Sort::ComptimeValue,
        };
        assert!(instance_argument_work(&open, &heads).is_none());
        assert!(instance_argument_work(&Term::TupleValue(vec![value, open]), &heads).is_none());
        let oversized = Term::TupleValue(vec![Term::Bool(false); 100_000]);
        assert!(instance_argument_work(&oversized, &heads).is_none());
    }

    #[test]
    fn enum_name_rejects_forged_domains_before_evaluation() {
        let package = Package::default();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let source = SourceRange { start: 3, end: 9 };
        for ty in [
            Term::Primitive(Primitive::Symbol),
            Term::Nominal {
                declaration: DeclarationId {
                    package: 0,
                    local: 17,
                },
                arguments: vec![],
            },
        ] {
            let expr = Expr {
                ty: Term::Primitive(Primitive::Str),
                source,
                kind: K::EnumName(Box::new(Expr {
                    ty,
                    source,
                    kind: K::Local(LocalId {
                        owner: 17,
                        index: 1,
                    }),
                })),
            };
            assert_eq!(
                vm.eval(&expr, &mut Environment::new()).unwrap_err().kind,
                ErrorKind::Type
            );
        }
    }

    #[test]
    fn symbol_name_rejects_forged_types_before_reading_the_operand() {
        let package = Package::default();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let source = SourceRange { start: 3, end: 9 };
        let missing = LocalId {
            owner: 17,
            index: 1,
        };
        for (input, output) in [
            (Primitive::Symbol, Primitive::Bool),
            (Primitive::Str, Primitive::Str),
            (Primitive::Bool, Primitive::Str),
        ] {
            let expr = Expr {
                ty: Term::Primitive(output),
                source,
                kind: K::SymbolName(Box::new(Expr {
                    ty: Term::Primitive(input),
                    source,
                    kind: K::Local(missing),
                })),
            };
            assert_eq!(
                vm.eval(&expr, &mut Environment::new()).unwrap_err().kind,
                ErrorKind::Type
            );
        }
        let valid = Expr {
            ty: Term::Primitive(Primitive::Str),
            source,
            kind: K::SymbolName(Box::new(Expr {
                ty: Term::Primitive(Primitive::Symbol),
                source,
                kind: K::Literal(Literal::Symbol("ready".into())),
            })),
        };
        assert!(
            matches!(vm.eval(&valid,&mut Environment::new()).unwrap(),Flow::Value(Value::String(name)) if name.as_ref()=="ready")
        );
    }

    #[test]
    fn scoped_type_descriptions_do_not_admit_foreign_inputs_or_pending_operations() {
        let id = DeclarationId {
            package: 7,
            local: 1,
        };
        let parameters = BTreeSet::from([id]);
        let heads = BTreeMap::new();
        let rigid = Term::Rigid {
            declaration: id,
            sort: nir::Sort::Type,
        };
        let pointer = Term::Pointer(Box::new(rigid.clone()));
        assert!(scoped_type_value_work(&pointer, &heads, &parameters).is_some());
        assert!(type_value_work(&pointer, &heads).is_none());
        for invalid in [
            Term::Rigid {
                declaration: DeclarationId {
                    package: 7,
                    local: 2,
                },
                sort: nir::Sort::Type,
            },
            Term::Rigid {
                declaration: id,
                sort: nir::Sort::Runtime,
            },
            Term::Rigid {
                declaration: id,
                sort: nir::Sort::ComptimeValue,
            },
            Term::TypeListConcat(vec![Term::TypeListValue(vec![rigid.clone()])]),
        ] {
            let nested = Term::NormalForm {
                head: id,
                sort: nir::Sort::Type,
                arguments: vec![invalid],
            };
            assert!(scoped_type_value_work(&nested, &heads, &parameters).is_none());
        }
        let value = Value::TypeList(vec![pointer]);
        assert!(value.scoped_copy_work(&parameters).is_some());
        assert!(value.immutable_copy_work().is_none());
        assert!(
            value
                .clone()
                .into_normal_term(&mut comptime_runtime::Budget {
                    steps: 100,
                    max_limbs: 128
                })
                .is_err()
        );
        assert!(
            value
                .clone()
                .into_scoped_normal_term(
                    &mut comptime_runtime::Budget {
                        steps: 100,
                        max_limbs: 128
                    },
                    &parameters
                )
                .is_ok()
        );
        assert_eq!(
            value.into_scoped_normal_term(
                &mut comptime_runtime::Budget {
                    steps: 0,
                    max_limbs: 128
                },
                &parameters
            ),
            Err(ErrorKind::StepsExhausted)
        );
    }

    #[test]
    fn concrete_type_inspection_checks_effect_arguments_and_width() {
        let operation = |arguments| nir::effects::Operation {
            declaration: DeclarationId {
                package: 1,
                local: 1,
            },
            arguments,
        };
        let boolean = Term::Primitive(Primitive::Bool);
        let closed = Term::computation(vec![operation(vec![boolean.clone()])], boolean.clone());
        assert!(type_value_work(&closed, &BTreeMap::new()).is_some());
        let open = Term::computation(
            vec![operation(vec![Term::Rigid {
                declaration: DeclarationId {
                    package: 1,
                    local: 2,
                },
                sort: nir::Sort::Type,
            }])],
            boolean.clone(),
        );
        assert!(type_value_work(&open, &BTreeMap::new()).is_none());
        let wide = Term::computation(vec![operation(vec![]); 100_001], boolean);
        assert!(type_value_work(&wide, &BTreeMap::new()).is_none());
    }
}

#[cfg(test)]
mod normal_input_data_tests {
    use super::*;

    #[test]
    fn freezing_named_callables_does_not_erase_captures_or_specialization() {
        let declaration = DeclarationId {
            package: 2,
            local: 3,
        };
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(Primitive::Type)),
        }
        .with_comptime_phase(true);
        let closure = Closure {
            definition_source: None,
            origin: Some(declaration),
            instance_arguments: Some(vec![]),
            parameters: vec![],
            environment: Environment::new(),
            capture_order: vec![],
            body: Arc::new(Expr {
                source: SourceRange { start: 0, end: 1 },
                ty: Term::Primitive(Primitive::Type),
                kind: K::Unit,
            }),
            ty: signature,
        };
        let freeze = |value: Value| {
            value.into_scoped_invocation_term(
                &mut comptime_runtime::Budget::default(),
                &BTreeSet::new(),
            )
        };
        assert_eq!(
            freeze(Value::Function(declaration)).unwrap(),
            Term::FunctionValue {
                declaration,
                arguments: vec![]
            }
        );
        assert_eq!(
            freeze(Value::Closure(Arc::new(closure.clone()))).unwrap(),
            Term::FunctionValue {
                declaration,
                arguments: vec![]
            }
        );
        let mut captured = closure.clone();
        let binding = LocalId { owner: 1, index: 0 };
        captured.environment.insert(binding, Value::Bool(true));
        captured.capture_order.push(binding);
        let mut specialized = closure.clone();
        specialized.instance_arguments = Some(vec![Term::Primitive(Primitive::Bool)]);
        for argument in [
            Term::Bool(true),
            Term::TupleValue(vec![Term::Bool(false), Term::Primitive(Primitive::Bool)]),
            Term::TypeListValue(vec![Term::Primitive(Primitive::Bool)]),
        ] {
            let mut instance = closure.clone();
            instance.instance_arguments = Some(vec![argument.clone()]);
            assert_eq!(
                freeze(Value::Closure(Arc::new(instance))).unwrap(),
                Term::FunctionValue {
                    declaration,
                    arguments: vec![argument]
                }
            );
        }
        for argument in [
            Term::Rigid {
                declaration,
                sort: nir::Sort::ComptimeValue,
            },
            Term::TupleValue(vec![Term::Rigid {
                declaration,
                sort: nir::Sort::ComptimeValue,
            }]),
            Term::TypeListConcat(vec![Term::TypeListValue(vec![])]),
        ] {
            let mut instance = closure.clone();
            instance.instance_arguments = Some(vec![argument]);
            assert_eq!(
                Value::Closure(Arc::new(instance)).into_scoped_invocation_term(
                    &mut comptime_runtime::Budget::default(),
                    &BTreeSet::from([declaration]),
                ),
                Err(ErrorKind::Type)
            );
        }
        let mut scoped = closure.clone();
        let parameter = Term::Rigid {
            declaration,
            sort: nir::Sort::Type,
        };
        scoped.instance_arguments = Some(vec![parameter.clone()]);
        assert_eq!(
            freeze(Value::Closure(Arc::new(scoped.clone()))),
            Err(ErrorKind::Type)
        );
        assert_eq!(
            Value::Closure(Arc::new(scoped))
                .into_scoped_invocation_term(
                    &mut comptime_runtime::Budget::default(),
                    &BTreeSet::from([declaration]),
                )
                .unwrap(),
            Term::FunctionValue {
                declaration,
                arguments: vec![parameter]
            }
        );
        let mut anonymous = closure.clone();
        anonymous.origin = None;
        let mut runtime = closure;
        runtime.ty = runtime.ty.callable_signature().clone();
        assert_eq!(
            freeze(Value::Closure(Arc::new(specialized))).unwrap(),
            Term::FunctionValue {
                declaration,
                arguments: vec![Term::Primitive(Primitive::Bool)]
            }
        );
        assert_eq!(
            freeze(Value::Closure(Arc::new(runtime))).unwrap(),
            Term::FunctionValue {
                declaration,
                arguments: vec![]
            }
        );
        for unsupported in [captured, anonymous] {
            assert_eq!(
                freeze(Value::Tuple(vec![Value::Closure(Arc::new(
                    unsupported.clone()
                ))]))
                .unwrap_err(),
                ErrorKind::Unsupported
            );
            assert_eq!(
                freeze(Value::Closure(Arc::new(unsupported))).unwrap_err(),
                ErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn invocation_aggregates_retain_named_callables_without_exporting_them_as_data() {
        let declaration = DeclarationId {
            package: 2,
            local: 3,
        };
        let value = Value::Tuple(vec![Value::AnonymousRecord(vec![(
            "callback".into(),
            Value::Function(declaration),
        )])]);
        let expected = Term::TupleValue(vec![Term::RecordValue(vec![(
            "callback".into(),
            Term::FunctionValue {
                declaration,
                arguments: vec![],
            },
        )])]);
        assert_eq!(
            value
                .clone()
                .into_normal_term(&mut comptime_runtime::Budget::default()),
            Err(ErrorKind::Unsupported)
        );
        assert_eq!(
            value.clone().into_scoped_invocation_term(
                &mut comptime_runtime::Budget {
                    steps: 1,
                    max_limbs: 4096
                },
                &BTreeSet::new()
            ),
            Err(ErrorKind::StepsExhausted)
        );
        assert_eq!(
            value
                .into_scoped_invocation_term(
                    &mut comptime_runtime::Budget::default(),
                    &BTreeSet::new()
                )
                .unwrap(),
            expected
        );
        let mut deep = Value::Function(declaration);
        for _ in 0..130 {
            deep = Value::Tuple(vec![deep]);
        }
        assert_eq!(
            deep.into_scoped_invocation_term(
                &mut comptime_runtime::Budget::default(),
                &BTreeSet::new()
            ),
            Err(ErrorKind::DepthExhausted)
        );
    }

    #[test]
    fn aggregate_snapshot_classification_is_bounded_and_excludes_executable_members() {
        let package = Package::default();
        let source = SourceRange { start: 1, end: 2 };
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let data = Value::AnonymousRecord(vec![(
            "nested".into(),
            Value::Tuple(vec![Value::Bool(true)]),
        )]);
        assert!(vm.normal_input_is_data(&data, source).unwrap());
        let executable = Value::Tuple(vec![
            data.clone(),
            Value::Function(DeclarationId {
                package: 1,
                local: 1,
            }),
        ]);
        assert!(!vm.normal_input_is_data(&executable, source).unwrap());
        assert!(vm.normal_input_is_data(&data, source).unwrap());
        let mut deep = Value::Bool(true);
        for _ in 0..130 {
            deep = Value::Tuple(vec![deep]);
        }
        assert_eq!(
            vm.normal_input_is_data(&deep, source).unwrap_err().kind,
            ErrorKind::DepthExhausted
        );
        assert!(vm.normal_input_is_data(&data, source).unwrap());
        let mut small = Interpreter::new(
            &package,
            Budget {
                steps: 4,
                ..Budget::default()
            },
        )
        .unwrap();
        assert_eq!(
            small
                .normal_input_is_data(&Value::Tuple(vec![Value::Unit; 10]), source)
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
    }
}

#[cfg(test)]
mod block_continuation_tests {
    use super::*;

    #[test]
    fn deep_block_scopes_preserve_transfers_and_errors() {
        let source = SourceRange { start: 3, end: 7 };
        let binding = LocalId { owner: 1, index: 0 };
        let target = LocalId { owner: 2, index: 0 };
        for mode in 0..4 {
            let mut body = Expr {
                ty: Term::Primitive(Primitive::Never),
                source,
                kind: match mode {
                    0 => K::Return(None),
                    1 => K::Break(target),
                    2 => K::Continue(target),
                    _ => K::SymbolName(Box::new(Expr {
                        ty: Term::Primitive(Primitive::Bool),
                        source,
                        kind: K::Literal(Literal::Bool(true)),
                    })),
                },
            };
            for index in 1..=200 {
                let statement = Expr {
                    ty: Term::Primitive(Primitive::Void),
                    source,
                    kind: K::Let {
                        binding: LocalId { owner: 1, index },
                        value: Box::new(Expr {
                            ty: Term::Primitive(Primitive::Bool),
                            source,
                            kind: K::Literal(Literal::Bool(true)),
                        }),
                    },
                };
                body = Expr {
                    ty: Term::Primitive(Primitive::Never),
                    source,
                    kind: if index % 2 == 0 {
                        K::Block {
                            statements: vec![statement],
                            result: Some(Box::new(body)),
                        }
                    } else {
                        K::CallPreparation {
                            statements: vec![statement],
                            result: Some(Box::new(body)),
                        }
                    },
                };
            }
            let package = Package::default();
            let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
            let mut environment = Environment::new();
            environment.insert(binding, Value::Bool(false));
            let result = vm.eval(&body, &mut environment);
            match mode {
                0 => assert!(matches!(result.unwrap(), Flow::Return(Value::Unit))),
                1 => assert!(matches!(result.unwrap(), Flow::Break(id) if id == target)),
                2 => assert!(matches!(result.unwrap(), Flow::Continue(id) if id == target)),
                _ => assert_eq!(result.unwrap_err().kind, ErrorKind::Type),
            }
            assert_eq!(environment.len(), 1);
            assert_eq!(environment.get(&binding), Some(&Value::Bool(false)));
            assert_eq!(vm.depth, 0);
        }
    }

    #[test]
    fn deep_block_scopes_unwind_on_success_and_fuel_failure() {
        let boolean = Term::Primitive(Primitive::Bool);
        let source = SourceRange { start: 1, end: 2 };
        let literal = || Expr {
            ty: boolean.clone(),
            source,
            kind: K::Literal(Literal::Bool(true)),
        };
        let mut body = literal();
        for index in 1..=200 {
            let statement = Expr {
                ty: Term::Primitive(Primitive::Void),
                source,
                kind: K::Let {
                    binding: LocalId { owner: 1, index },
                    value: Box::new(literal()),
                },
            };
            body = Expr {
                ty: boolean.clone(),
                source,
                kind: if index % 2 == 0 {
                    K::Block {
                        statements: vec![statement],
                        result: Some(Box::new(body)),
                    }
                } else {
                    K::CallPreparation {
                        statements: vec![statement],
                        result: Some(Box::new(body)),
                    }
                },
            };
        }
        let package = Package::default();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let binding = LocalId { owner: 1, index: 0 };
        let mut environment = Environment::new();
        environment.insert(binding, Value::Bool(false));
        assert!(matches!(
            vm.eval(&body, &mut environment).unwrap(),
            Flow::Value(Value::Bool(true))
        ));
        assert_eq!(environment.len(), 1);
        assert_eq!(environment.get(&binding), Some(&Value::Bool(false)));
        assert_eq!(vm.depth, 0);
        vm.budget.steps = 64;
        assert_eq!(
            vm.eval(&body, &mut environment).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        assert_eq!(environment.len(), 1);
        assert_eq!(vm.depth, 0);
        vm.budget.steps = 100_000;
        assert!(matches!(
            vm.eval(&body, &mut environment).unwrap(),
            Flow::Value(Value::Bool(true))
        ));
        assert_eq!(environment.len(), 1);
    }
}

#[cfg(test)]
mod pointer_copy_budget_tests {
    use super::*;
    #[test]
    fn pointer_write_proof_uses_live_fuel_before_mutation_and_can_retry() {
        let ty = Term::Tuple(vec![Term::Primitive(Primitive::Bool)]);
        let storage = Storage::new(Value::Tuple(vec![Value::Bool(false)]));
        let binding = LocalId { owner: 1, index: 0 };
        let source = SourceRange { start: 1, end: 2 };
        let expr = Expr {
            ty: Term::Primitive(Primitive::Void),
            source,
            kind: K::WritePointer {
                pointer: Box::new(Expr {
                    ty: Term::Pointer(Box::new(ty.clone())),
                    source,
                    kind: K::Local(binding),
                }),
                value: Box::new(Expr {
                    ty: ty.clone(),
                    source,
                    kind: K::Tuple(vec![Expr {
                        ty: Term::Primitive(Primitive::Bool),
                        source,
                        kind: K::Literal(Literal::Bool(true)),
                    }]),
                }),
            },
        };
        let mut environment = Environment::new();
        environment.insert(binding, Value::Pointer(storage.pointer(ty, vec![])));
        let observed = std::cell::Cell::new(0);
        let permission = |_: &Term, remaining: &mut u64| {
            observed.set(*remaining);
            let allowed = *remaining >= 20;
            *remaining = remaining.saturating_sub(20);
            allowed
        };
        let package = Package::default();
        let mut vm = Interpreter::new(
            &package,
            Budget {
                steps: 10,
                depth: 128,
            },
        )
        .unwrap()
        .with_budgeted_pointer_copy_permission(&permission);
        assert_eq!(
            vm.eval(&expr, &mut environment).unwrap_err().kind,
            ErrorKind::Unsupported
        );
        assert!(
            observed.get() < 10,
            "operand evaluation must precede proof acquisition"
        );
        assert_eq!(vm.remaining_steps(), 0);
        assert_eq!(
            storage.read().unwrap(),
            Value::Tuple(vec![Value::Bool(false)])
        );
        assert_eq!(vm.depth, 0);
        vm.budget.steps = 40;
        assert!(matches!(
            vm.eval(&expr, &mut environment).unwrap(),
            Flow::Value(Value::Unit)
        ));
        assert_eq!(vm.remaining_steps(), observed.get() - 20);
        assert_eq!(
            storage.read().unwrap(),
            Value::Tuple(vec![Value::Bool(true)])
        );
    }

    #[test]
    fn pointer_reads_charge_copy_work_and_recover_without_mutation() {
        let ty = Term::Tuple(vec![Term::Primitive(Primitive::Bool); 64]);
        let original = Value::Tuple(vec![Value::Bool(true); 64]);
        let storage = Storage::new(original.clone());
        let binding = LocalId { owner: 1, index: 0 };
        let source = SourceRange { start: 1, end: 2 };
        let expr = Expr {
            ty: ty.clone(),
            source,
            kind: K::ReadPointer(Box::new(Expr {
                ty: Term::Pointer(Box::new(ty.clone())),
                source,
                kind: K::Local(binding),
            })),
        };
        let mut environment = Environment::new();
        environment.insert(binding, Value::Pointer(storage.pointer(ty, vec![])));
        let package = Package::default();
        // Resource permission is explicit; it does not waive copying limits.
        let mut vm = Interpreter::new(
            &package,
            Budget {
                steps: 8,
                depth: 128,
            },
        )
        .unwrap()
        .with_pointer_copy_permission(&|_| true);
        assert_eq!(
            vm.eval(&expr, &mut environment).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        assert_eq!(storage.read().unwrap(), original);
        assert_eq!(vm.depth, 0);
        vm.budget.steps = 1000;
        assert!(
            matches!(vm.eval(&expr,&mut environment).unwrap(),Flow::Value(value) if value==original)
        );
        assert_eq!(storage.read().unwrap(), original);
        assert!(vm.remaining_steps() < 1000 - 64);
    }
}

mod error_results;
mod newtypes;
mod optional;

#[cfg(test)]
mod live_graph_tests {
    use super::*;

    #[test]
    fn owned_graph_visits_shared_storage_and_cycles_once_without_cloning() {
        let storage = Storage::new(Value::Unit);
        let nested = Storage::new(Value::Tuple(vec![
            Value::Storage(storage.clone()),
            Value::Function(DeclarationId {
                package: 3,
                local: 7,
            }),
        ]));
        *storage.0.lock().unwrap() = Value::Tuple(vec![
            Value::Storage(nested.clone()),
            Value::Storage(nested.clone()),
        ]);
        let root = Value::Storage(storage.clone());
        let mut cells = 0;
        let mut functions = Vec::new();
        let result = root.visit_owned_graph(&mut 100, &mut |value, _| {
            match value {
                Value::Storage(_) => cells += 1,
                Value::Function(id) => functions.push(*id),
                _ => {}
            }
            Ok(())
        });
        // Break the test-created ownership cycle even if an assertion fails.
        *storage.0.lock().unwrap() = Value::Unit;
        result.unwrap();
        assert_eq!(cells, 2);
        assert_eq!(
            functions,
            vec![DeclarationId {
                package: 3,
                local: 7
            }]
        );
    }

    #[test]
    fn owned_graph_budget_and_locked_storage_fail_without_waiting() {
        let storage = Storage::new(Value::Bool(true));
        let root = Value::Storage(storage.clone());
        let mut seen = 0;
        assert_eq!(
            root.visit_owned_graph(&mut 0, &mut |_, _| {
                seen += 1;
                Ok(())
            }),
            Err(ErrorKind::StepsExhausted)
        );
        assert_eq!(seen, 0);
        let lock = storage.0.lock().unwrap();
        assert_eq!(
            root.visit_owned_graph(&mut 10, &mut |_, _| Ok(())),
            Err(ErrorKind::InvalidPointer)
        );
        drop(lock);
        root.visit_owned_graph(&mut 10, &mut |value, _| {
            if matches!(value, Value::Bool(true)) {
                seen += 1;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, 1);
        let mut deep = Value::Unit;
        for _ in 0..130 {
            deep = Value::Tuple(vec![deep]);
        }
        assert_eq!(
            deep.visit_owned_graph(&mut 1000, &mut |_, _| Ok(())),
            Err(ErrorKind::DepthExhausted)
        );
    }
}

#[cfg(test)]
mod invocation_environment_tests {
    use super::*;
    #[test]
    fn failed_capture_preparation_restores_frame_and_can_resume_with_fresh_budget() {
        let package = Package::default();
        let source = SourceRange { start: 1, end: 2 };
        let binding = LocalId { owner: 1, index: 0 };
        let closure = Value::Closure(Arc::new(Closure {
            definition_source: None,
            origin: None,
            instance_arguments: None,
            parameters: vec![],
            capture_order: vec![binding],
            environment: Environment::from([(binding, Value::Tuple(vec![Value::Bool(false); 64]))]),
            body: Arc::new(Expr {
                ty: Term::Primitive(Primitive::Bool),
                source,
                kind: K::Literal(Literal::Bool(true)),
            }),
            ty: Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(Primitive::Bool)),
            },
        }));
        let mut vm = Interpreter::new(
            &package,
            Budget {
                steps: 8,
                depth: 128,
            },
        )
        .unwrap();
        vm.effect_state.invocation = 7;
        vm.effect_state.resume = Some(9);
        assert_eq!(
            vm.invoke_value(closure.clone(), vec![]).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        assert_eq!(vm.depth, 0);
        assert_eq!(vm.effect_state.invocation, 7);
        assert_eq!(vm.effect_state.resume, Some(9));
        assert!(vm.cleanup_frame.is_none());
        assert!(vm.prepared_frames.is_empty());
        vm.budget.steps = 10_000;
        assert_eq!(vm.invoke_value(closure, vec![]).unwrap(), Value::Bool(true));
        assert_eq!(vm.depth, 0);
        assert_eq!(vm.effect_state.invocation, 7);
        assert_eq!(vm.effect_state.resume, Some(9));
    }
}

#[cfg(test)]
mod nominal_environment_tests {
    use super::*;
    use nir::program::{NormalFormArgument as A, NormalFormPreparation, Pattern};

    #[test]
    fn nominal_input_and_packet_bindings_are_removed_on_every_exit() {
        let id = |local| DeclarationId { package: 1, local };
        let local = |index| LocalId { owner: 0, index };
        let ty = |primitive| Term::Primitive(primitive);
        let source = SourceRange { start: 1, end: 2 };
        let type_value = || Expr {
            source,
            ty: ty(Primitive::Type),
            kind: K::TypeValue(ty(Primitive::Bool)),
        };
        let package = Package {
            structs: vec![nir::program::Struct {
                declaration: id(20),
                name: "Pair".into(),
                type_parameters: vec![id(21), id(22)],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
                fields: vec![],
            }],
            ..Default::default()
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut environment = Environment::from([(local(99), Value::Bool(true))]);
        for mode in 0..3 {
            let second = match mode {
                0 => A::Value(type_value()),
                1 => A::Integer {
                    source,
                    spelling: "7".into(),
                    negative: false,
                },
                _ => A::Value(Expr {
                    source,
                    ty: ty(Primitive::Never),
                    kind: K::Return(Some(Box::new(type_value()))),
                }),
            };
            let expression = Expr {
                source,
                ty: ty(if second.is_diverging() {
                    Primitive::Never
                } else {
                    Primitive::Type
                }),
                kind: K::NominalType {
                    declaration: id(20),
                    order: None,
                    bindings: Some(vec![local(1), local(2)]),
                    arguments: vec![
                        A::Value(Expr {
                            source,
                            ty: ty(Primitive::Type),
                            kind: K::Local(local(3)),
                        }),
                        second,
                    ],
                    preparations: vec![NormalFormPreparation {
                        before: 0,
                        expression: Expr {
                            source,
                            ty: ty(Primitive::Void),
                            kind: K::LetPattern {
                                pattern: Pattern::Tuple(vec![Pattern::Binding {
                                    binding: local(3),
                                    inner: Box::new(Pattern::Wild),
                                }]),
                                value: Box::new(Expr {
                                    source,
                                    ty: Term::Tuple(vec![ty(Primitive::Type)]),
                                    kind: K::Tuple(vec![type_value()]),
                                }),
                                guard: None,
                                otherwise: None,
                            },
                        },
                    }],
                },
            };
            for _ in 0..2 {
                let result = vm.eval(&expression, &mut environment);
                match mode {
                    0 => assert!(matches!(
                        result,
                        Ok(Flow::Value(Value::Type(Term::Nominal { .. })))
                    )),
                    1 => assert_eq!(result.unwrap_err().kind, ErrorKind::Type),
                    _ => assert!(matches!(
                        result,
                        Ok(Flow::Return(Value::Type(Term::Primitive(Primitive::Bool))))
                    )),
                }
                assert_eq!(environment.len(), 1);
                assert_eq!(environment.get(&local(99)), Some(&Value::Bool(true)));
            }
        }
    }
}

#[cfg(test)]
mod metered_type_solver_tests {
    use super::*;

    #[test]
    fn host_type_proof_spends_invocation_budget_and_cannot_replenish_it() {
        let package = Package::default();
        let predicate = nir::program::TypePredicate::Equal(
            Term::Primitive(Primitive::Bool),
            Term::Primitive(Primitive::U8),
        );
        let calls = std::cell::Cell::new(0);
        let solver = |_: &nir::program::TypePredicate, remaining: &mut u64| {
            calls.set(calls.get() + 1);
            *remaining = remaining.saturating_sub(7);
            ContractOutcome::Unknown
        };
        let mut vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 10,
                depth: 128,
            },
        )
        .unwrap()
        .with_budgeted_type_solver(&solver);
        let available = vm.remaining_steps();
        assert_eq!(
            vm.solve_type_predicate(&predicate),
            ContractOutcome::Unknown
        );
        assert_eq!(vm.remaining_steps(), available - 7);
        vm.solve_type_predicate(&predicate);
        assert_eq!(vm.remaining_steps(), 0);
        vm.solve_type_predicate(&predicate);
        assert_eq!(calls.get(), 2);
        let replenish = |_: &nir::program::TypePredicate, remaining: &mut u64| {
            *remaining = u64::MAX;
            ContractOutcome::Satisfied
        };
        let mut fresh = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 10,
                depth: 128,
            },
        )
        .unwrap()
        .with_budgeted_type_solver(&replenish);
        let available = fresh.remaining_steps();
        assert_eq!(
            fresh.solve_type_predicate(&predicate),
            ContractOutcome::Satisfied
        );
        assert_eq!(fresh.remaining_steps(), available);
    }
}

#[cfg(test)]
mod not_spine_tests;
