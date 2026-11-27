//! Concrete high-level instances. Structured NIR operations are retained; this
//! stage neither invents SSA names nor erases program contracts or opaque heads.
pub mod behavior;
pub mod effect_values;
pub mod effects;
mod index;
mod lists;
mod type_links;

pub use behavior::{BehaviorAttributes, CallsiteBehavior, MemoryEffect, PointerParamAttrs};

use type_links::TypeLinks;

pub mod staging;
mod storage;
pub mod template_storage;
pub mod visit;

use nir::program::{DispatchInstance, Function};
use nir::{DeclarationId, OrderedTerm, RuntimeId, Term};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct TermRef<'a>(pub(crate) &'a Term);

impl Ord for TermRef<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        nir::cmp_term(self.0, other.0)
    }
}

impl PartialOrd for TermRef<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct TermsRef<'a>(pub(crate) &'a [Term]);

impl Ord for TermsRef<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        nir::cmp_term_slice(self.0, other.0)
    }
}

impl PartialOrd for TermsRef<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InstanceKey {
    pub declaration: DeclarationId,
    /// Frozen input frame, shared by identity and graph clones. Equality and
    /// hashing remain structural; pointer identity is not semantic evidence.
    pub arguments: std::sync::Arc<[Term]>,
}

impl InstanceKey {
    fn parts(&self) -> (DeclarationId, &[Term]) {
        (self.declaration, &self.arguments)
    }
}

/// Dependency of a concrete clause operation, conditional on its operands
/// returning. The executable packet still owns all lexical computations.
pub fn observed_clause_arguments(
    declaration: DeclarationId,
    inputs: &nir::program::ClauseOperands,
    fuel: &mut usize,
) -> Result<Vec<Term>, String> {
    inputs
        .copy_observed_arguments(fuel)
        .map_err(|e| format!("IIR clause dependency budget: {e:?}"))?
        .ok_or_else(|| {
            format!("IIR clause dependency for {declaration} still requires invocation-time input")
        })
}

impl fmt::Display for InstanceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(instance-key (declaration {}) (arguments",
            self.declaration
        )?;
        for argument in self.arguments.iter() {
            write!(f, " {argument}")?;
        }
        f.write_str("))")
    }
}

/// Declaration evidence for a callable carried as comptime data.
/// This records its completed signature, not an executable instance or a tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallableIdentity {
    pub key: InstanceKey,
    pub signature: nir::TypeRef,
}

impl CallableIdentity {
    /// Retain completed, environment-free signature evidence in a shared core.
    /// Declaration identity remains in `key`; sharing a core proves no callable
    /// identity equality and does not bypass formation or invocation contracts.
    pub fn intern(
        key: InstanceKey,
        signature: Term,
        interner: &mut nir::TypeInterner,
        fuel: &mut usize,
    ) -> Result<Self, String> {
        completed_type(&signature, &BTreeSet::new(), fuel)?;
        if !matches!(signature.callable_signature(), Term::Function { .. }) {
            return Err("IIR callable identity has a non-callable signature".into());
        }
        let core = interner
            .intern_with_budget(vec![], signature, fuel)
            .map_err(|e| e.to_string())?;
        let signature = nir::TypeRef::bind(core, vec![]).map_err(|e| e.to_string())?;
        Ok(Self { key, signature })
    }

    pub fn closed_signature(&self) -> Result<&Term, String> {
        if !self.signature.core().holes().is_empty() || !self.signature.arguments().is_empty() {
            return Err("IIR frozen callable signature requires an environment".into());
        }
        Ok(self.signature.core().body())
    }
}

impl fmt::Display for CallableIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(callable-identity {} (signature {}))",
            self.key, self.signature
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FunctionInstance {
    pub key: InstanceKey,
    /// All declaration parameters have been substituted, including contracts,
    /// capture types and default expressions. Local IDs retain source identity.
    pub function: Function,
    /// Conservatively certified purity and memory behavior for LLVM optimization.
    /// `None` denotes uncertified / conservative P0 semantics.
    pub behavior: Option<BehaviorAttributes>,
    /// Certified callsite behavior for indirect calls and direct effect handlers.
    pub callsite_behaviors: Vec<CallsiteBehavior>,
}

impl FunctionInstance {
    /// Construct an uncertified `FunctionInstance` with conservative behavior defaults.
    pub fn new(key: InstanceKey, function: Function) -> Self {
        Self {
            key,
            function,
            behavior: None,
            callsite_behaviors: Vec::new(),
        }
    }
}

impl fmt::Display for FunctionInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(function-instance {} {}", self.key, self.function)?;
        if let Some(behavior) = &self.behavior {
            write!(f, " {behavior}")?;
        }
        if !self.callsite_behaviors.is_empty() {
            f.write_str(" (callsite-behaviors")?;
            for entry in &self.callsite_behaviors {
                write!(f, " {entry}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NominalDefinition {
    Struct(nir::program::Struct),
    Enum(nir::program::Enum),
    Newtype(nir::newtypes::Newtype),
}

impl fmt::Display for NominalDefinition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Struct(s) => s.fmt(f),
            Self::Enum(e) => e.fmt(f),
            Self::Newtype(n) => n.fmt(f),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NominalInstance {
    pub ty: Term,
    pub definition: NominalDefinition,
}

impl fmt::Display for NominalInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(nominal-instance (type {}) {})",
            self.ty, self.definition
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InterfaceContract {
    pub interface: nir::program::InterfaceInstance,
    pub subject: Term,
    pub methods: Vec<nir::program::MethodContract>,
}

impl fmt::Display for InterfaceContract {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(interface-contract (interface {}) (subject {}) (methods",
            self.interface, self.subject
        )?;
        for method in &self.methods {
            write!(f, " {method}")?;
        }
        f.write_str("))")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DynVtableMethod {
    pub member: nir::DeclarationId,
    pub returns_self: bool,
    pub by_reference_receiver: bool,
    pub body: nir::DeclarationId,
    pub arguments: Vec<Term>,
}

impl fmt::Display for DynVtableMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(dyn-method (member {}) (returns-self {}) (by-ref {}) (body {}) (arguments",
            self.member, self.returns_self, self.by_reference_receiver, self.body
        )?;
        for arg in &self.arguments {
            write!(f, " {arg}")?;
        }
        f.write_str("))")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DynVtable {
    pub interface: nir::program::InterfaceInstance,
    pub subject: Term,
    pub by_pointer: bool,
    pub env_identities: Vec<Term>,
    pub methods: Vec<DynVtableMethod>,
}

impl fmt::Display for DynVtable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(dyn-vtable (interface {}) (subject {}) (by-pointer {}) (envs",
            self.interface, self.subject, self.by_pointer
        )?;
        for env in &self.env_identities {
            write!(f, " {env}")?;
        }
        f.write_str(") (methods")?;
        for method in &self.methods {
            write!(f, " {method}")?;
        }
        f.write_str("))")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DynInterfaceMethod {
    pub member: nir::DeclarationId,
    pub returns_self: bool,
    pub by_reference_receiver: bool,
    pub parameters: Vec<nir::Parameter>,
    pub result: Term,
}

impl fmt::Display for DynInterfaceMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(dyn-interface-method (member {}) (returns-self {}) (by-ref {}) (params",
            self.member, self.returns_self, self.by_reference_receiver
        )?;
        for param in &self.parameters {
            write!(f, " {}", param.ty)?;
        }
        write!(f, ") (result {}))", self.result)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DynInterface {
    pub interface: nir::program::InterfaceInstance,
    pub methods: Vec<DynInterfaceMethod>,
}

impl fmt::Display for DynInterface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(dyn-interface (interface {}) (methods", self.interface)?;
        for method in &self.methods {
            write!(f, " {method}")?;
        }
        f.write_str("))")
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Package {
    pub target: Option<nir::target::TargetData>,
    pub expansion_sources: Vec<nir::program::ExpansionSource>,
    pub roots: Vec<InstanceKey>,
    pub nominals: Vec<NominalInstance>,
    pub interfaces: Vec<InterfaceContract>,
    pub dyn_interfaces: Vec<DynInterface>,
    pub dyn_vtables: Vec<DynVtable>,
    pub normal_forms: Vec<nir::normal_forms::Head>,
    pub callable_identities: Vec<CallableIdentity>,
    pub functions: Vec<FunctionInstance>,
    /// Declaration-derived contracts demanded by concrete effect rows, even
    /// when the operation is never requested or handled in this graph.
    pub effect_contracts: Vec<effects::Contract>,
    /// Validated identities stored as values, with no executable template body.
    pub templates: Vec<template_storage::TemplateStorage>,
    /// Fully selected implementation evidence; no runtime implementation search.
    pub dispatch: Vec<DispatchInstance>,
    /// Explicitly supplied external runtime identities. Availability is not an
    /// ownership/lifetime proof; responsible resource passes retain that duty.
    pub runtime_bindings: BTreeSet<RuntimeId>,
    /// Concrete lift domains for runtime identities referenced by this graph.
    pub runtime_domains: std::collections::BTreeMap<RuntimeId, Term>,
    /// Package-level canonical runtime environment deliveries (e.g. global const
    /// addresses keyed by their `RuntimeId`).
    pub global_runtime_deliveries: std::collections::BTreeMap<RuntimeId, nir::program::Expr>,
    /// Checked `__flurry_volatile` declarations and struct fields.
    pub volatile_storage: nir::program::VolatileStorage,
    /// Checked `__flurry_align` overrides on nominal declarations and struct fields.
    pub layout_alignments: nir::program::LayoutAlignments,
    /// Compiled pattern-matching decision DAGs and list-view metadata keyed by `(function, source)`.
    pub match_plans: nir::match_plan::MatchPlans,
    pub statics: Vec<nir::program::Static>,
}

struct IirLayoutWorld<'a>(&'a Package);

impl nir::LayoutWorld for IirLayoutWorld<'_> {
    fn target(&self) -> Option<nir::target::TargetData> {
        self.0.target
    }

    fn is_tuple_head(&self, head: DeclarationId) -> bool {
        self.0
            .normal_forms
            .iter()
            .any(|nf| nf.declaration == head && nf.role == nir::normal_forms::Role::Tuple)
    }

    fn is_list_head(&self, head: DeclarationId) -> bool {
        self.0
            .normal_forms
            .iter()
            .any(|nf| nf.declaration == head && nf.role == nir::normal_forms::Role::TypeList)
    }

    fn is_array_head(&self, head: DeclarationId) -> bool {
        self.0
            .normal_forms
            .iter()
            .any(|nf| nf.declaration == head && nf.role == nir::normal_forms::Role::Array)
    }

    fn is_maybe_uninit_storage_head(&self, head: DeclarationId) -> bool {
        self.0.normal_forms.iter().any(|nf| {
            nf.declaration == head && nf.role == nir::normal_forms::Role::MaybeUninitStorage
        })
    }

    fn is_manually_drop_storage_head(&self, head: DeclarationId) -> bool {
        self.0.normal_forms.iter().any(|nf| {
            nf.declaration == head && nf.role == nir::normal_forms::Role::ManuallyDropStorage
        })
    }

    fn declaration_alignment(&self, declaration: DeclarationId) -> Option<u64> {
        self.0.layout_alignments.declaration_align(declaration)
    }

    fn field_alignment(&self, declaration: DeclarationId, index: usize) -> Option<u64> {
        self.0.layout_alignments.field_align(declaration, index)
    }

    fn nominal_kind(&self, ty: &Term) -> Option<nir::NominalKind> {
        let instance = self.0.nominals.iter().find(|entry| entry.ty == *ty)?;
        Some(match &instance.definition {
            NominalDefinition::Struct(_) => nir::NominalKind::Struct,
            NominalDefinition::Enum(_) => nir::NominalKind::Enum,
            NominalDefinition::Newtype(_) => nir::NominalKind::Newtype,
        })
    }

    fn is_template_identity(&self, ty: &Term) -> bool {
        self.0
            .templates
            .iter()
            .any(|table| table.logical_type() == ty)
    }

    fn nominal_layout(
        &self,
        ty: &Term,
        _fuel: &mut usize,
    ) -> Result<Option<nir::NominalLayout>, nir::LayoutError> {
        let Some(instance) = self.0.nominals.iter().find(|instance| instance.ty == *ty) else {
            return Ok(None);
        };
        Ok(Some(match &instance.definition {
            NominalDefinition::Struct(def) => {
                let fields = def.fields.iter().map(|(_, t)| t.clone()).collect();
                match def.packed {
                    nir::program::PackedMode::None => nir::NominalLayout::Struct(fields),
                    nir::program::PackedMode::Byte => nir::NominalLayout::PackedStruct(fields),
                    nir::program::PackedMode::Bit => nir::NominalLayout::BitPackedStruct(fields),
                }
            }
            NominalDefinition::Newtype(def) => {
                nir::NominalLayout::Newtype(def.representation.clone())
            }
            NominalDefinition::Enum(def) => {
                if def.symbol_encoding.is_some() {
                    nir::NominalLayout::EnumSymbol
                } else if let Some(encoding) = &def.encoding {
                    let bits = u32::try_from(encoding.width())
                        .map_err(|_| nir::LayoutError::Unsupported(ty.clone()))?;
                    nir::NominalLayout::EnumFixedBits {
                        width: bits,
                        niche: encoding.layout.contiguous_niche(),
                        exhaustive: encoding.layout.is_exhaustive(),
                    }
                } else if def.variants.iter().all(|v| v.fields.is_empty()) {
                    nir::NominalLayout::EnumUnit(def.variants.len())
                } else {
                    nir::NominalLayout::EnumAggregate(
                        def.variants.iter().map(|v| v.fields.clone()).collect(),
                    )
                }
            }
        }))
    }

    fn nominal_field_index(
        &self,
        ty: &Term,
        field: &str,
        _fuel: &mut usize,
    ) -> Result<nir::FieldLookup, nir::LayoutError> {
        let Some(instance) = self.0.nominals.iter().find(|instance| instance.ty == *ty) else {
            return Ok(nir::FieldLookup::NotFound);
        };
        Ok(match &instance.definition {
            NominalDefinition::Struct(def) => def
                .fields
                .iter()
                .position(|(name, _)| name == field)
                .map_or(nir::FieldLookup::NotFound, nir::FieldLookup::Index),
            _ => nir::FieldLookup::NotFound,
        })
    }
}

impl Package {
    pub fn template_method_type(
        &self,
        interface: &nir::program::InterfaceInstance,
        subject: &Term,
        member: DeclarationId,
        arguments: &[Term],
    ) -> Option<&Term> {
        let method = self
            .dispatch
            .iter()
            .find(|selected| {
                selected.interface == interface.declaration
                    && selected.interface_arguments == interface.arguments
                    && &selected.subject == subject
            })?
            .methods
            .iter()
            .find(|method| method.member == member && method.method_arguments == arguments)?;
        self.templates
            .iter()
            .find(|table| {
                table.targets().iter().any(|key| {
                    key.declaration == method.body
                        && key.arguments.as_ref() == method.arguments.as_slice()
                })
            })
            .map(|table| table.logical_type())
    }

    pub fn template_type(&self, key: &InstanceKey) -> Option<&Term> {
        self.templates
            .iter()
            .find(|table| table.targets().contains(key))
            .map(|table| table.logical_type())
    }

    pub fn function(&self, key: &InstanceKey) -> Option<&FunctionInstance> {
        self.functions.iter().find(|function| &function.key == key)
    }

    /// Canonical operational runtime environment parameters required by a
    /// closure or indirect callable of type `callable_ty`. Derived strictly
    /// from non-global `RuntimeId`s in `callable_ty` so semantic captures and
    /// operational delivery stay separated and ABI-uniform across all call sites.
    pub fn closure_runtime_env_parameters(
        &self,
        callable_ty: &Term,
    ) -> Result<Vec<nir::program::RuntimeEnvParameter>, String> {
        nir::program::implicit_runtime_env_parameters(
            &self.runtime_domains,
            &self.global_runtime_deliveries,
            std::slice::from_ref(callable_ty),
            &[],
        )
        .map_err(|reason| reason.to_string())
    }

    pub fn type_is_volatile(&self, ty: &Term, fuel: &mut usize) -> bool {
        self.volatile_storage.type_is_volatile_with(
            |cur| {
                let nominal = self.nominals.iter().find(|n| &n.ty == cur)?;
                match &nominal.definition {
                    NominalDefinition::Struct(def) => {
                        Some(nir::program::VolatileNominal::Struct(&def.fields))
                    }
                    NominalDefinition::Newtype(def) => {
                        Some(nir::program::VolatileNominal::Newtype(&def.representation))
                    }
                    NominalDefinition::Enum(_) => None,
                }
            },
            ty,
            fuel,
        )
    }

    /// Validate completed annotations and closure of every named callable
    /// reference, including references embedded in defaults and patterns.
    pub fn validate(&self, fuel: &mut usize) -> Result<(), String> {
        let callables = self.checked_callables(fuel)?;
        let links = TypeLinks::new(self, fuel)?;
        *fuel = fuel
            .checked_sub(self.statics.len())
            .ok_or("IIR static validation budget exhausted")?;
        let mut declared_statics = BTreeMap::new();
        for static_def in &self.statics {
            *fuel = fuel
                .checked_sub(static_def.name.len().saturating_add(1))
                .ok_or("IIR static validation budget exhausted")?;
            if declared_statics
                .insert(static_def.declaration, static_def)
                .is_some()
            {
                return Err(format!(
                    "IIR duplicate static declaration {}",
                    static_def.declaration
                ));
            }
        }
        let mut validated_types = BTreeSet::<OrderedTerm>::new();
        let mut validated_closed_storage = BTreeSet::<OrderedTerm>::new();
        for identity in &self.callable_identities {
            let signature = identity.closed_signature()?;
            if !matches!(signature.callable_signature(), Term::Function { .. }) {
                return Err("IIR callable identity has a non-callable signature".into());
            }
            self.validate_type_cached(&links, &mut validated_types, signature, fuel)?;
            for argument in identity.key.arguments.iter() {
                self.validate_type_cached(&links, &mut validated_types, argument, fuel)?;
            }
        }
        for table in &self.templates {
            self.validate_type_cached(&links, &mut validated_types, table.logical_type(), fuel)?;
            for target in table.targets() {
                for argument in target.arguments.iter() {
                    self.validate_type_cached(&links, &mut validated_types, argument, fuel)?;
                }
            }
        }
        if self
            .dispatch
            .iter()
            .any(|instance| !instance.associated_initializers.is_empty())
        {
            return Err("IIR requires associated initializers to be invoked".into());
        }
        for nominal in &self.nominals {
            self.validate_type_cached(&links, &mut validated_types, &nominal.ty, fuel)?;
            match &nominal.definition {
                NominalDefinition::Newtype(n) => self.validate_type_cached(
                    &links,
                    &mut validated_types,
                    &n.representation,
                    fuel,
                )?,
                NominalDefinition::Struct(s) => {
                    for (_, ty) in &s.fields {
                        self.validate_type_cached(&links, &mut validated_types, ty, fuel)?;
                    }
                }
                NominalDefinition::Enum(e) => {
                    for variant in &e.variants {
                        for ty in &variant.fields {
                            self.validate_type_cached(&links, &mut validated_types, ty, fuel)?;
                        }
                    }
                }
            }
        }
        for interface in &self.interfaces {
            for argument in &interface.interface.arguments {
                self.validate_type_cached(&links, &mut validated_types, argument, fuel)?;
            }
            self.validate_type_cached(&links, &mut validated_types, &interface.subject, fuel)?;
            for method in &interface.methods {
                self.validate_type_cached(
                    &links,
                    &mut validated_types,
                    method.signature.ordinary().map_err(|e| e.to_string())?,
                    fuel,
                )?;
                for argument in &method.type_arguments {
                    self.validate_type_cached(&links, &mut validated_types, argument, fuel)?;
                }
                let program = nir::contracts::CallableProgram::method(method)
                    .map_err(|error| error.to_string())?;
                let mut direct_callees = BTreeSet::new();
                visit::program_expressions(program, fuel, |expr, fuel| {
                    if let nir::program::ExprKind::Call { callee, .. } = &expr.kind {
                        direct_callees.insert(std::ptr::from_ref(callee.as_ref()) as usize);
                    }
                    let is_direct_callee =
                        direct_callees.remove(&(std::ptr::from_ref(expr) as usize));
                    callables.validate_reference(
                        expr,
                        method.signature.is_comptime_callable(),
                        is_direct_callee,
                        fuel,
                    )?;
                    self.validate_expression_types_in(&links, &mut validated_types, expr, fuel)
                })?;
            }
        }
        // Storage checks consume concrete layout evidence from this artifact;
        // they must not rely on the producer's generic source declarations.
        let storage_metadata = nir::program::Package {
            target: self.target,
            normal_forms: self.normal_forms.clone(),
            ..Default::default()
        };
        let resolve_storage = |ty: &Term,
                               fuel: &mut usize|
         -> Result<Vec<Term>, nir::phases::StorageError> {
            let definition = links
                .nominal(ty, fuel)?
                .ok_or(nir::phases::StorageError::Invalid(
                    "IIR nominal storage layout unavailable",
                ))?;
            let mut copy_field = |field: &Term| {
                nir::type_computations::charge_term(field, fuel)?;
                Ok(field.clone())
            };
            match definition {
                NominalDefinition::Struct(definition) => definition
                    .fields
                    .iter()
                    .map(|(_, ty)| copy_field(ty))
                    .collect(),
                NominalDefinition::Enum(definition) => definition
                    .variants
                    .iter()
                    .flat_map(|v| &v.fields)
                    .map(copy_field)
                    .collect(),
                NominalDefinition::Newtype(definition) => {
                    Ok(vec![copy_field(&definition.representation)?])
                }
            }
        };
        use nir::program::ExprKind as K;
        let validate_expr_node = |expr: &nir::program::Expr,
                                  phase: nir::program::FunctionPhase,
                                  direct_callees: &mut BTreeSet<usize>,
                                  validated_types: &mut BTreeSet<OrderedTerm>,
                                  fuel: &mut usize|
         -> Result<(), String> {
            if matches!(
                expr.kind,
                K::Literal(
                    nir::program::Literal::PendingFloat { .. }
                        | nir::program::Literal::PendingInteger { .. }
                )
            ) {
                return Err("IIR contains an unformed float or integer literal".into());
            }

            self.validate_expression_types_in(&links, validated_types, expr, fuel)?;
            if let K::TemplateEqual(left, right) = &expr.kind {
                let identity_operand = |value: &nir::program::Expr| {
                    matches!(
                        value.kind,
                        K::Local(_)
                            | K::Function(_)
                            | K::FunctionInstance { .. }
                            | K::TraitMethod { .. }
                    )
                };
                if phase != nir::program::FunctionPhase::Runtime
                    || expr.ty != Term::Primitive(nir::Primitive::Bool)
                    || left.ty != right.ty
                    || !identity_operand(left)
                    || !identity_operand(right)
                    || !self
                        .templates
                        .iter()
                        .any(|table| table.logical_type() == &left.ty)
                {
                    return Err(
                        "IIR template comparison requires stored callable identities".into(),
                    );
                }
            }
            if phase == nir::program::FunctionPhase::Runtime {
                if matches!(expr.kind, K::ComptimeIntegerConvert(_)) {
                    return Err("IIR requires comptime integer conversion completion".into());
                }
                if let K::Call { callee, .. } = &expr.kind {
                    if !callee.ty.is_comptime_callable()
                        && matches!(callee.ty.callable_signature(), Term::Function { parameters, .. }
                            if parameters.iter().any(|p| p.sort != nir::Sort::Runtime))
                    {
                        return Err("IIR runtime call retains staged inputs".into());
                    }
                }
            }
            if let Some(message) =
                nir::program::pointer_operation_error_with_world(&IirLayoutWorld(self), expr, fuel)
            {
                return Err(message.into());
            }
            if let K::Call { callee, .. } = &expr.kind {
                direct_callees.insert(std::ptr::from_ref(callee.as_ref()) as usize);
            }
            let is_direct_callee = direct_callees.remove(&(std::ptr::from_ref(expr) as usize));
            callables.validate_reference(
                expr,
                phase == nir::program::FunctionPhase::Comptime,
                is_direct_callee,
                fuel,
            )?;
            match &expr.kind {
                K::StagedValue(_) => {
                    return Err("IIR retains an unbound staged value".into());
                }
                K::ComptimeValue(_) => {
                    return Err("IIR requires captured comptime value materialization".into());
                }
                K::BuildMetadata { .. } if phase != nir::program::FunctionPhase::Comptime => {
                    return Err("IIR runtime body retains package metadata query".into());
                }
                K::AssociatedValue { .. } => {
                    return Err("IIR requires associated value selection".into());
                }
                K::ConstAddress { value, .. } => {
                    if !nir::free_bindings::collect(value, fuel)?.is_empty() {
                        return Err("IIR constant address retains free local bindings".into());
                    }
                }
                K::StaticAddress(declaration) => {
                    let Some(static_def) = declared_statics.get(declaration) else {
                        return Err(format!(
                            "IIR static address references missing static {declaration}"
                        ));
                    };
                    if expr.ty != Term::Pointer(Box::new(static_def.ty.clone())) {
                        return Err(format!(
                            "IIR static address type disagrees with static declaration {declaration}"
                        ));
                    }
                }
                K::DynCast(cast) => {
                    if phase == nir::program::FunctionPhase::Comptime {
                        return Err("IIR comptime body cannot perform runtime dyn cast".into());
                    }
                    if !cast.env_arguments.is_empty() && matches!(expr.ty, Term::TraitObject { .. })
                    {
                        return Err(
                            "IIR dyn cast cannot silently drop non-empty runtime environment"
                                .into(),
                        );
                    }
                }
                _ => {}
            }
            Ok(())
        };
        for (id, domain) in &self.runtime_domains {
            if !self.runtime_bindings.contains(id) {
                return Err(format!(
                    "IIR runtime domain references unknown runtime binding {id}"
                ));
            }
            self.validate_type_cached(&links, &mut validated_types, domain, fuel)?;
        }
        for (id, expr) in &self.global_runtime_deliveries {
            if !self.runtime_bindings.contains(id) {
                return Err(format!(
                    "IIR global runtime delivery references unknown runtime binding {id}"
                ));
            }
            if !nir::free_bindings::collect(expr, fuel)?.is_empty() {
                return Err("IIR global runtime delivery retains free local bindings".into());
            }
            self.validate_type_cached(&links, &mut validated_types, &expr.ty, fuel)?;
            let mut direct_callees = BTreeSet::new();
            visit::expression(expr, fuel, |node, fuel| {
                validate_expr_node(
                    node,
                    nir::program::FunctionPhase::Runtime,
                    &mut direct_callees,
                    &mut validated_types,
                    fuel,
                )
            })?;
        }
        for static_def in &self.statics {
            self.validate_type_cached(&links, &mut validated_types, &static_def.ty, fuel)?;
            let ordered_ty = OrderedTerm(static_def.ty.clone());
            if validated_closed_storage.contains(&ordered_ty) {
                *fuel = fuel
                    .checked_sub(1)
                    .ok_or("storage inspection budget exhausted")?;
            } else {
                nir::phases::validate_runtime_storage_with_nominals(
                    &static_def.ty,
                    &storage_metadata,
                    &[],
                    &resolve_storage,
                    fuel,
                )
                .map_err(|message| {
                    format!(
                        "IIR runtime storage for static {}: {message}",
                        static_def.declaration
                    )
                })?;
                validated_closed_storage.insert(ordered_ty);
            }
            if let Some(init) = &static_def.initializer {
                if init.ty != static_def.ty {
                    return Err(format!(
                        "IIR static {} initializer type mismatch",
                        static_def.declaration
                    ));
                }
                if !nir::free_bindings::collect(init, fuel)?.is_empty() {
                    return Err(format!(
                        "IIR static {} initializer retains free local bindings",
                        static_def.declaration
                    ));
                }
                storage::validate_expressions(
                    vec![init],
                    &[],
                    &storage_metadata,
                    &resolve_storage,
                    &mut validated_closed_storage,
                    &static_def.name,
                    fuel,
                )?;
                let mut direct_callees = BTreeSet::new();
                visit::expression(init, fuel, |node, fuel| {
                    validate_expr_node(
                        node,
                        nir::program::FunctionPhase::Runtime,
                        &mut direct_callees,
                        &mut validated_types,
                        fuel,
                    )
                })?;
            }
        }
        for instance in &self.functions {
            self.validate_type_cached(
                &links,
                &mut validated_types,
                &instance.function.signature,
                fuel,
            )?;
            for env in &instance.function.env_parameters {
                self.validate_type_cached(&links, &mut validated_types, &env.ty, fuel)?;
            }
            nir::call_safety::validate(&instance.function, fuel)?;
            if instance.function.is_unsafe && self.roots.contains(&instance.key) {
                return Err("unsafe entry requires an explicit safe wrapper".into());
            }
            if instance.function.phase == nir::program::FunctionPhase::Runtime {
                let ordered_signature = OrderedTerm(instance.function.signature.clone());
                if validated_closed_storage.contains(&ordered_signature) {
                    *fuel = fuel
                        .checked_sub(1)
                        .ok_or("storage inspection budget exhausted")?;
                } else {
                    nir::phases::validate_runtime_storage_with_nominals(
                        &instance.function.signature,
                        &storage_metadata,
                        &[],
                        &resolve_storage,
                        fuel,
                    )
                    .map_err(|message| {
                        format!("IIR runtime signature for {}: {message}", instance.key)
                    })?;
                    validated_closed_storage.insert(ordered_signature);
                }
            }

            storage::validate(
                &instance.function,
                &storage_metadata,
                &resolve_storage,
                &mut validated_closed_storage,
                fuel,
            )?;
            for argument in instance.key.arguments.iter() {
                self.validate_type_cached(&links, &mut validated_types, argument, fuel)?;
            }
            for (id, expr) in &instance.function.runtime_deliveries {
                if !self.runtime_bindings.contains(id) {
                    return Err(format!(
                        "IIR function {} delivery references unknown runtime binding {id}",
                        instance.key
                    ));
                }
                self.validate_type_cached(&links, &mut validated_types, &expr.ty, fuel)?;
                let mut direct_callees = BTreeSet::new();
                visit::expression(expr, fuel, |node, fuel| {
                    validate_expr_node(
                        node,
                        nir::program::FunctionPhase::Runtime,
                        &mut direct_callees,
                        &mut validated_types,
                        fuel,
                    )
                })?;
            }
            let mut direct_callees = BTreeSet::new();
            visit::expressions(&instance.function, fuel, |expr, fuel| {
                validate_expr_node(
                    expr,
                    instance.function.phase,
                    &mut direct_callees,
                    &mut validated_types,
                    fuel,
                )
            })?;
            effects::validate(self, &links, &instance.function, fuel)?;
        }
        effects::validate_contracts(self, &links, fuel)?;
        Ok(())
    }

    /// Effect control exits need acquired cleanup schedules before this graph
    /// may participate in resource-checked execution or destructor invocation.
    pub fn validate_cleanup_control(&self, fuel: &mut usize) -> Result<(), String> {
        for instance in &self.functions {
            self.validate_cleanup_control_instance(instance, fuel)?;
        }
        Ok(())
    }

    pub fn validate_cleanup_control_instance(
        &self,
        instance: &FunctionInstance,
        fuel: &mut usize,
    ) -> Result<(), String> {
        visit::expressions(&instance.function, fuel, |expr, fuel| {
            self.validate_cleanup_control_expr(expr, fuel)
        })
    }

    pub fn validate_cleanup_control_expr(
        &self,
        expr: &nir::program::Expr,
        fuel: &mut usize,
    ) -> Result<(), String> {
        use nir::program::ExprKind as K;
        match &expr.kind {
            K::EffectHandle { handlers, .. } => {
                for handler in handlers {
                    if !matches!(
                        handler.abi,
                        nir::effects::Abi::Direct | nir::effects::Abi::Capturing
                    ) && !(handler.abi == nir::effects::Abi::DirectPassing
                        && effects::data_passing_signature(self, &handler.signature, fuel)?)
                    {
                        return Err(
                            "IIR effect installation requires slot/capture cleanup protocol lowering"
                                .into(),
                        );
                    }
                    // Capture ownership/initialization is discharged by
                    // resource schedule publication, not type-only IIR
                    // validation. Traverse here to retain bounded lexical
                    // well-formedness checks before execution.
                    for arm in &handler.arms {
                        nir::free_bindings::collect_arm(arm, fuel)?;
                    }
                }
            }
            K::EffectRequest { abi, signature, .. }
                if !matches!(
                    abi,
                    nir::effects::Abi::Direct | nir::effects::Abi::Capturing
                ) && !(*abi == nir::effects::Abi::DirectPassing
                    && effects::data_passing_signature(self, signature, fuel)?) =>
            {
                return Err(
                    "IIR effect request requires slot/capture cleanup protocol lowering".into(),
                );
            }
            _ => {}
        }
        Ok(())
    }

    // Contracts and executable bodies must inspect the same embedded types.
    // Validating only Expr::ty misses constants and capture/storage annotations.
    fn validate_expression_types_in(
        &self,
        links: &TypeLinks<'_>,
        validated: &mut BTreeSet<OrderedTerm>,
        expr: &nir::program::Expr,
        fuel: &mut usize,
    ) -> Result<(), String> {
        use nir::program::ExprKind as K;
        self.validate_type_cached(&links, validated, &expr.ty, fuel)?;
        lists::validate(links, expr, fuel)?;
        if let K::BuildMetadata {
            selector, entries, ..
        } = &expr.kind
        {
            if selector.ty != Term::Primitive(nir::Primitive::Str) {
                return Err("IIR package metadata requires a str selector".into());
            }
            let mut aliases = std::collections::BTreeSet::new();
            for (alias, _, value) in entries {
                *fuel = fuel
                    .checked_sub(alias.len().saturating_add(1))
                    .ok_or("IIR metadata alias budget exhausted")?;
                if alias.is_empty() || !aliases.insert(alias) {
                    return Err("IIR package metadata has empty or duplicate aliases".into());
                }
                if let Some(value) = value {
                    // Only the selected payload is admitted against the output
                    // domain at invocation; all retained references must link.
                    self.validate_type_cached(links, validated, value, fuel)?;
                }
            }
        }
        if let K::SymbolName(operand) | K::EnumName(operand) = &expr.kind {
            let never = Term::Primitive(nir::Primitive::Never);
            let expected = if operand.ty == never {
                never
            } else {
                Term::Primitive(nir::Primitive::Str)
            };
            if expr.ty != expected {
                return Err("IIR tag_name result type disagrees with its operand".into());
            }
            if operand.ty != Term::Primitive(nir::Primitive::Never) {
                match &expr.kind {
                    K::SymbolName(_) if operand.ty != Term::Primitive(nir::Primitive::Symbol) => {
                        return Err("IIR Symbol tag_name requires a Symbol operand".into());
                    }
                    K::EnumName(_) => {
                        let found = matches!(
                            links.nominal(&operand.ty, fuel)?,
                            Some(NominalDefinition::Enum(_))
                        );
                        if !found {
                            return Err("IIR enum tag_name requires an enum instance".into());
                        }
                    }
                    _ => {}
                }
            }
        }
        if let K::Intrinsic { tag, .. } = &expr.kind {
            nir::intrinsics::checked(expr).map_err(str::to_owned)?;
            // Intrinsic families currently have no type/value parameters.
            // A matching declaration alone must not authorize another instance.
            let key = Term::Nominal {
                declaration: tag.family,
                arguments: vec![],
            };
            let Some(NominalDefinition::Enum(family)) = links.nominal(&key, fuel)? else {
                return Err("IIR intrinsic tag family instance is missing".into());
            };
            nir::intrinsics::validate_family(family, *tag).map_err(str::to_owned)?;
        }
        if matches!(&expr.kind, K::UnsafeScope(body) if body.ty != expr.ty) {
            return Err("unsafe scope changes its body type".into());
        }
        let mut check_pattern = |pattern: &nir::program::Pattern| {
            nir::visit::pattern_types(pattern, fuel, |ty, fuel| {
                if ty.sort() != nir::Sort::Type {
                    return Err("IIR pattern label is not a type".into());
                }
                self.validate_type_cached(&links, validated, ty, fuel)
            })
        };
        match &expr.kind {
            K::Match { arms, .. } => {
                for arm in arms {
                    check_pattern(&arm.pattern)?;
                }
            }
            K::LetPattern { pattern, .. } => check_pattern(pattern)?,
            K::EffectHandle { handlers, .. } => {
                for handler in handlers {
                    for arm in &handler.arms {
                        check_pattern(&arm.pattern)?;
                    }
                }
            }
            _ => (),
        }
        match &expr.kind {
            K::OptionalAbsent | K::OptionalPresent(_) => {
                let Term::Optional(payload) = &expr.ty else {
                    return Err("IIR optional construction requires an optional type".into());
                };
                if let K::OptionalPresent(value) = &expr.kind {
                    if value.ty != **payload {
                        return Err("IIR optional payload type mismatch".into());
                    }
                }
            }
            K::ErrorConvert { conversion, value } => {
                nir::error_results::validate_conversion_with_layouts(
                    *conversion,
                    &value.ty,
                    &expr.ty,
                    fuel,
                    &mut |a, b| a == b,
                    &|ty| links.head(ty).and_then(|head| head.tuple_elements(ty)),
                )
                .map_err(|e| format!("IIR error result conversion: {e}"))?;
            }
            K::NewtypeConvert(value) => {
                if links
                    .newtype_conversion(&value.ty, &expr.ty, fuel)
                    .map_err(|e| e.to_string())?
                    .is_none()
                {
                    return Err("IIR newtype conversion has no direct representation edge".into());
                }
            }
            K::TypeValue(ty) => {
                if expr.ty != Term::Primitive(nir::Primitive::Type) || ty.sort() != nir::Sort::Type
                {
                    return Err("IIR Type value requires a Type domain and type identity".into());
                }
                self.validate_type_cached(&links, validated, ty, fuel)?;
            }
            K::BitAssign {
                storage_type: ty, ..
            }
            | K::BitUpdate {
                storage_type: ty, ..
            } => self.validate_type_cached(&links, validated, ty, fuel)?,
            K::Closure { captures, .. } => {
                for capture in captures {
                    self.validate_type_cached(&links, validated, &capture.ty, fuel)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn validate_type_cached(
        &self,
        links: &TypeLinks<'_>,
        validated: &mut BTreeSet<OrderedTerm>,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<(), String> {
        let key = OrderedTerm(ty.clone());
        if validated.contains(&key) {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("IIR type validation budget exhausted")?;
            return Ok(());
        }
        completed_type(ty, &self.runtime_bindings, fuel)?;
        links.validate(ty, fuel)?;
        validated.insert(key);
        Ok(())
    }

    fn validate_type(
        &self,
        links: &TypeLinks<'_>,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<(), String> {
        completed_type(ty, &self.runtime_bindings, fuel)?;
        links.validate(ty, fuel)
    }

    #[cfg(test)]
    fn type_links(&self, ty: &Term, fuel: &mut usize) -> Result<(), String> {
        TypeLinks::new(self, fuel)?.validate(ty, fuel)
    }

    #[cfg(test)]
    fn validate_expression_types(
        &self,
        expr: &nir::program::Expr,
        fuel: &mut usize,
    ) -> Result<(), String> {
        let links = TypeLinks::new(self, fuel)?;
        self.validate_expression_types_in(&links, &mut BTreeSet::new(), expr, fuel)
    }
}

impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(iir-package (roots")?;
        for root in &self.roots {
            write!(f, " {root}")?;
        }
        f.write_str(") (runtime-bindings")?;
        for binding in &self.runtime_bindings {
            write!(f, " (runtime {} {})", binding.instance, binding.local)?;
        }
        f.write_str(")")?;
        if !self.runtime_domains.is_empty() {
            f.write_str(" (runtime-domains")?;
            for (id, domain) in &self.runtime_domains {
                write!(f, " (domain {id} {domain})")?;
            }
            f.write_str(")")?;
        }
        if !self.global_runtime_deliveries.is_empty() {
            f.write_str(" (global-runtime-deliveries")?;
            for (id, expr) in &self.global_runtime_deliveries {
                write!(f, " (delivery {id} {expr})")?;
            }
            f.write_str(")")?;
        }
        if !self.volatile_storage.is_empty() {
            write!(f, " {}", self.volatile_storage)?;
        }
        if !self.layout_alignments.is_empty() {
            write!(f, " {}", self.layout_alignments)?;
        }
        if !self.match_plans.is_empty() {
            write!(f, " {}", self.match_plans)?;
        }
        if let Some(target) = self.target {
            write!(f, " {target}")?;
        }
        if !self.expansion_sources.is_empty() {
            f.write_str(" (expansion-sources")?;
            for (index, origin) in self.expansion_sources.iter().enumerate() {
                write!(f, " (expansion-source {index} {origin})")?;
            }
            f.write_str(")")?;
        }
        if !self.effect_contracts.is_empty() {
            f.write_str(" (effect-contracts")?;
            for contract in &self.effect_contracts {
                write!(f, " {contract}")?;
            }
            f.write_str(")")?;
        }
        if !self.callable_identities.is_empty() {
            f.write_str(" (callable-identities")?;
            for identity in &self.callable_identities {
                write!(f, " {identity}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(" (normal-form-heads")?;
        for head in &self.normal_forms {
            write!(f, " {head}")?;
        }
        f.write_str(") (nominals")?;
        for nominal in &self.nominals {
            write!(f, "\n {nominal}")?;
        }
        f.write_str(") (interfaces")?;
        for interface in &self.interfaces {
            write!(f, "\n {interface}")?;
        }
        if !self.templates.is_empty() {
            f.write_str(") (templates")?;
            for template in &self.templates {
                write!(f, " {template}")?;
            }
        }
        f.write_str(") (functions")?;
        for function in &self.functions {
            write!(f, "\n {function}")?;
        }
        f.write_str(") (dispatch")?;
        for instance in &self.dispatch {
            write!(f, "\n {instance}")?;
        }
        if !self.dyn_interfaces.is_empty() {
            f.write_str(") (dyn-interfaces")?;
            for iface in &self.dyn_interfaces {
                write!(f, "\n {iface}")?;
            }
        }
        if !self.dyn_vtables.is_empty() {
            f.write_str(") (dyn-vtables")?;
            for vtable in &self.dyn_vtables {
                write!(f, "\n {vtable}")?;
            }
        }
        if !self.statics.is_empty() {
            f.write_str(") (statics")?;
            for entry in &self.statics {
                write!(f, "\n {entry}")?;
            }
        }
        f.write_str("))\n")
    }
}

/// Check a completed type/semantic argument. Runtime identities are preserved;
/// missing environments and unsubstituted definition rigids are rejected.
pub fn completed_type(
    ty: &Term,
    available: &BTreeSet<RuntimeId>,
    fuel: &mut usize,
) -> Result<(), String> {
    // Scope validation is recursive; guard depth and payload work first, even
    // for externally assembled IIR that has not passed a producer's preflight.
    nir::type_computations::charge_term(ty, fuel)
        .map_err(|e| format!("IIR type validation budget: {e}"))?;
    ty.check_scope(&[])
        .map_err(|e| format!("IIR type scope: {e}"))?;
    completed_dependencies(ty, available, fuel)
}

/// Check dependencies within a term whose outer binder scope was already
/// validated. Nested function parameters/results retain their binder context.
pub fn completed_dependencies(
    ty: &Term,
    available: &BTreeSet<RuntimeId>,
    fuel: &mut usize,
) -> Result<(), String> {
    let mut pending = vec![ty];
    while let Some(ty) = pending.pop() {
        if *fuel == 0 {
            return Err("IIR type validation budget exhausted".into());
        }
        *fuel -= 1;
        match ty {
            Term::AssociatedType { .. } => {
                return Err("IIR type has an unresolved associated type projection".into());
            }
            Term::Value { .. } => {
                return Err("IIR value identity lacks retained value-store evidence".into());
            }
            Term::ListConcat { .. } => {
                return Err("IIR type retains an unresolved list concatenation".into());
            }
            Term::Rigid { .. } | Term::Hole { .. } => {
                return Err(
                    "IIR type has an unresolved declaration parameter or template hole".into(),
                );
            }
            Term::Runtime(id) if !available.contains(id) => {
                return Err(format!(
                    "IIR type has an unavailable runtime binding {}:{}",
                    id.instance, id.local
                ));
            }
            _ => {}
        }
        ty.visit_children(|child| {
            pending.push(child);
            Ok::<_, String>(())
        })?;
    }
    Ok(())
}

pub mod cleanup;

impl Package {
    pub fn newtype_conversion(
        &self,
        source: &Term,
        target: &Term,
        fuel: &mut usize,
    ) -> Result<Option<nir::newtypes::Conversion>, nir::newtypes::ConversionError> {
        for nominal in &self.nominals {
            nir::type_computations::charge_term(&nominal.ty, fuel)
                .map_err(|_| nir::newtypes::ConversionError::Budget)?;
            if nominal.ty != *source && nominal.ty != *target {
                continue;
            }
            if let NominalDefinition::Newtype(definition) = &nominal.definition {
                if let Some(plan) = nir::newtypes::instance_conversion(
                    definition,
                    &nominal.ty,
                    source,
                    target,
                    fuel,
                )? {
                    return Ok(Some(plan));
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod optional_construction_tests {
    use super::*;
    use nir::program::{Expr, ExprKind as K, Literal, SourceRange};

    #[test]
    fn optional_types_and_frozen_payloads_cannot_hide_dangling_declarations() {
        let graph = Package::default();
        let missing = Term::Nominal {
            declaration: DeclarationId {
                package: 99,
                local: 1,
            },
            arguments: vec![],
        };
        let optional = |ty| Term::Optional(Box::new(ty));
        let source = SourceRange { start: 1, end: 2 };
        let expression = Expr {
            source,
            ty: Term::Primitive(nir::Primitive::Type),
            kind: K::TypeValue(optional(missing.clone())),
        };
        assert!(
            graph
                .validate_expression_types(&expression, &mut 100)
                .unwrap_err()
                .contains("dangling")
        );
        let frozen = Term::OptionalValue {
            ty: Box::new(optional(Term::Primitive(nir::Primitive::Type))),
            value: Some(Box::new(missing)),
        };
        assert!(
            graph
                .type_links(&frozen, &mut 100)
                .unwrap_err()
                .contains("dangling")
        );
        let valid = Expr {
            kind: K::TypeValue(optional(Term::Primitive(nir::Primitive::Bool))),
            ..expression
        };
        graph.validate_expression_types(&valid, &mut 100).unwrap();
    }

    #[test]
    fn optional_construction_requires_exact_payload_domain() {
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let optional = Term::Optional(Box::new(boolean.clone()));
        let source = SourceRange { start: 1, end: 2 };
        let absent = Expr {
            source,
            ty: optional.clone(),
            kind: K::OptionalAbsent,
        };
        let graph = Package::default();
        graph.validate_expression_types(&absent, &mut 100).unwrap();
        let present = Expr {
            source,
            ty: optional.clone(),
            kind: K::OptionalPresent(Box::new(Expr {
                source,
                ty: boolean.clone(),
                kind: K::Literal(Literal::Bool(true)),
            })),
        };
        graph.validate_expression_types(&present, &mut 100).unwrap();
        for malformed in [
            Expr {
                ty: boolean,
                ..absent
            },
            Expr {
                ty: Term::Optional(Box::new(optional)),
                ..present
            },
        ] {
            assert!(
                graph
                    .validate_expression_types(&malformed, &mut 100)
                    .is_err()
            );
        }
    }
}

#[cfg(test)]
mod validate_memo_tests {
    use super::*;
    use nir::program::{
        Expr, ExprKind as K, Function, FunctionDeclaration, FunctionPhase, Literal, SourceRange,
    };

    fn make_runtime_fn(decl_local: u32, body: Expr) -> FunctionInstance {
        let sig = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(nir::Primitive::Void)),
        };
        let key = InstanceKey {
            declaration: DeclarationId {
                package: 1,
                local: decl_local,
            },
            arguments: vec![].into(),
        };
        FunctionInstance {
            key: key.clone(),
            function: Function {
                interface: FunctionDeclaration {
                    source: SourceRange { start: 1, end: 2 },
                    phase: FunctionPhase::Runtime,
                    inline: false,
                    pure: false,
                    is_unsafe: false,
                    preconditions: vec![],
                    postconditions: vec![],
                    result_binding: None,
                    result_type_binder: None,
                    result_guarantees: vec![],
                    entry_snapshots: vec![],
                    capabilities: vec![],
                    requirements: vec![],
                    type_parameters: vec![],
                    declaration: key.declaration,
                    name: format!("fn_{decl_local}"),
                    signature: sig,
                    parameters: vec![],
                    env_parameters: vec![],
                    defaults: vec![],
                },
                body,
                runtime_deliveries: Default::default(),
            },
            behavior: None,
            callsite_behaviors: Vec::new(),
        }
    }

    #[test]
    fn repeated_closed_nominals_memoize_across_expressions_and_functions_while_rejecting_invalid_storage()
     {
        let source = SourceRange { start: 1, end: 2 };
        let struct_decl = DeclarationId {
            package: 1,
            local: 100,
        };
        let nominal_ty = Term::Nominal {
            declaration: struct_decl,
            arguments: vec![],
        };
        let fields: Vec<_> = (0..16)
            .map(|i| (format!("f{i}"), Term::Primitive(nir::Primitive::U64)))
            .collect();
        let repeated_ty = Term::Optional(Box::new(nominal_ty.clone()));
        let make_block = |count: usize| {
            let statements = (0..count)
                .map(|_| Expr {
                    source,
                    ty: Term::Primitive(nir::Primitive::Void),
                    kind: K::Discard(Box::new(Expr {
                        source,
                        ty: repeated_ty.clone(),
                        kind: K::OptionalAbsent,
                    })),
                })
                .collect();
            Expr {
                source,
                ty: Term::Primitive(nir::Primitive::Void),
                kind: K::Block {
                    statements,
                    result: None,
                },
            }
        };

        let make_pkg = |fn_count: u32, stmts_per_fn: usize| Package {
            nominals: vec![NominalInstance {
                ty: nominal_ty.clone(),
                definition: NominalDefinition::Struct(nir::program::Struct {
                    parameter_types: vec![],
                    capabilities: vec![],
                    requirements: vec![],
                    type_parameters: vec![],
                    declaration: struct_decl,
                    name: "WidePod".into(),
                    fields: fields.clone(),
                    packed: nir::program::PackedMode::None,
                }),
            }],
            functions: (1..=fn_count)
                .map(|id| make_runtime_fn(id, make_block(stmts_per_fn)))
                .collect(),
            ..Package::default()
        };

        let base_pkg = make_pkg(1, 1);
        let mut base_fuel = 100_000;
        base_pkg.validate(&mut base_fuel).unwrap();
        let base_cost = 100_000 - base_fuel;

        // 10 functions x 10 copies = 100 copies of a 16-field Optional<Nominal>.
        // Without memoization, each copy re-traverses WidePod's 16 fields for both
        // type links and runtime storage (> 60 extra steps/copy, > 6,000 steps).
        // With memoization, incremental per-copy validation stays well below 20 steps.
        let mut package = make_pkg(10, 10);
        let max_expected_fuel = base_cost + 2_500;
        let mut tight_fuel = max_expected_fuel;
        package.validate(&mut tight_fuel).unwrap();
        let memo_cost = max_expected_fuel - tight_fuel;
        assert!(
            memo_cost - base_cost < 2_500,
            "expected memoized incremental cost < 2500, got {} (base={})",
            memo_cost - base_cost,
            base_cost
        );

        // Memoization must still reject comptime-only Type values in runtime storage.
        let K::Block { statements, .. } = &mut package.functions[9].function.body.kind else {
            unreachable!()
        };
        statements[9] = Expr {
            source,
            ty: Term::Primitive(nir::Primitive::Void),
            kind: K::Discard(Box::new(Expr {
                source,
                ty: Term::Primitive(nir::Primitive::Type),
                kind: K::TypeValue(Term::Primitive(nir::Primitive::U8)),
            })),
        };
        let mut reject_fuel = 100_000;
        let error = package.validate(&mut reject_fuel).unwrap_err();
        assert!(
            error.contains("IIR runtime storage"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn statics_validate_types_storage_initializers_and_emit_sexpr() {
        use nir::program::Literal;
        let source = SourceRange { start: 1, end: 2 };
        let struct_decl = DeclarationId {
            package: 1,
            local: 10,
        };
        let static_decl = DeclarationId {
            package: 1,
            local: 20,
        };
        let other_static_decl = DeclarationId {
            package: 1,
            local: 21,
        };
        let nominal_ty = Term::Nominal {
            declaration: struct_decl,
            arguments: vec![],
        };
        let valid_pkg = Package {
            nominals: vec![NominalInstance {
                ty: nominal_ty.clone(),
                definition: NominalDefinition::Struct(nir::program::Struct {
                    parameter_types: vec![],
                    capabilities: vec![],
                    requirements: vec![],
                    type_parameters: vec![],
                    declaration: struct_decl,
                    name: "Cfg".into(),
                    fields: vec![("flag".into(), Term::Primitive(nir::Primitive::Bool))],
                    packed: nir::program::PackedMode::None,
                }),
            }],
            statics: vec![
                nir::program::Static {
                    declaration: static_decl,
                    name: "cfg".into(),
                    ty: nominal_ty.clone(),
                    initializer: Some(Expr {
                        source,
                        ty: nominal_ty.clone(),
                        kind: K::Record {
                            declaration: struct_decl,
                            fields: vec![Expr {
                                source,
                                ty: Term::Primitive(nir::Primitive::Bool),
                                kind: K::Literal(Literal::Bool(true)),
                            }],
                        },
                    }),
                },
                nir::program::Static {
                    declaration: other_static_decl,
                    name: "cfg_ptr".into(),
                    ty: Term::Pointer(Box::new(nominal_ty.clone())),
                    initializer: Some(Expr {
                        source,
                        ty: Term::Pointer(Box::new(nominal_ty.clone())),
                        kind: K::StaticAddress(static_decl),
                    }),
                },
            ],
            ..Package::default()
        };
        valid_pkg.validate(&mut 100_000).unwrap();

        let dump = valid_pkg.to_string();
        assert!(
            dump.contains("(statics\n (static (id (declaration (package 1) (local 20)))"),
            "{dump}"
        );
        assert!(
            dump.contains("(static (id (declaration (package 1) (local 21)))"),
            "{dump}"
        );
        let open_parens = dump.chars().filter(|&c| c == '(').count();
        let close_parens = dump.chars().filter(|&c| c == ')').count();
        assert_eq!(open_parens, close_parens, "unbalanced S-expr:\n{dump}");

        // Duplicate static declaration must be rejected.
        let mut dup = valid_pkg.clone();
        dup.statics.push(dup.statics[0].clone());
        assert!(
            dup.validate(&mut 100_000)
                .unwrap_err()
                .contains("duplicate static")
        );

        // Dangling nominal type in static must be rejected.
        let mut dangling_ty = valid_pkg.clone();
        dangling_ty.nominals.clear();
        assert!(
            dangling_ty
                .validate(&mut 100_000)
                .unwrap_err()
                .contains("dangling")
        );

        // Comptime-only static type must be rejected.
        let comptime_ty = Package {
            statics: vec![nir::program::Static {
                declaration: static_decl,
                name: "bad".into(),
                ty: Term::Primitive(nir::Primitive::Type),
                initializer: None,
            }],
            ..Package::default()
        };
        assert!(
            comptime_ty
                .validate(&mut 100_000)
                .unwrap_err()
                .contains("runtime storage")
        );

        // Initializer type mismatch must be rejected.
        let mut type_mismatch = valid_pkg.clone();
        type_mismatch.statics[0].initializer = Some(Expr {
            source,
            ty: Term::Primitive(nir::Primitive::Bool),
            kind: K::Literal(Literal::Bool(false)),
        });
        assert!(
            type_mismatch
                .validate(&mut 100_000)
                .unwrap_err()
                .contains("type mismatch")
        );

        // Free local binding in initializer must be rejected.
        let mut free_local = valid_pkg.clone();
        free_local.statics[1].initializer = Some(Expr {
            source,
            ty: Term::Pointer(Box::new(nominal_ty.clone())),
            kind: K::Local(nir::program::LocalId { owner: 1, index: 0 }),
        });
        assert!(
            free_local
                .validate(&mut 100_000)
                .unwrap_err()
                .contains("free local")
        );

        // Unstaged pending literal in initializer must be rejected.
        let pending_lit = Package {
            statics: vec![nir::program::Static {
                declaration: static_decl,
                name: "n".into(),
                ty: Term::Primitive(nir::Primitive::U64),
                initializer: Some(Expr {
                    source,
                    ty: Term::Primitive(nir::Primitive::U64),
                    kind: K::Literal(Literal::PendingInteger {
                        negative: false,
                        spelling: "1".into(),
                    }),
                }),
            }],
            ..Package::default()
        };
        assert!(
            pending_lit
                .validate(&mut 100_000)
                .unwrap_err()
                .contains("unformed")
        );

        // Dangling StaticAddress in initializer must be rejected.
        let mut dangling_ref = valid_pkg.clone();
        dangling_ref.statics[1].initializer = Some(Expr {
            source,
            ty: Term::Pointer(Box::new(nominal_ty.clone())),
            kind: K::StaticAddress(DeclarationId {
                package: 1,
                local: 999,
            }),
        });
        assert!(
            dangling_ref
                .validate(&mut 100_000)
                .unwrap_err()
                .contains("missing static")
        );

        // Dangling StaticAddress in a runtime function must also be rejected.
        let mut fn_dangling = valid_pkg.clone();
        fn_dangling.statics.clear();
        fn_dangling.functions.push(make_runtime_fn(
            1,
            Expr {
                source,
                ty: Term::Primitive(nir::Primitive::Void),
                kind: K::Discard(Box::new(Expr {
                    source,
                    ty: Term::Pointer(Box::new(nominal_ty)),
                    kind: K::StaticAddress(static_decl),
                })),
            },
        ));
        assert!(
            fn_dangling
                .validate(&mut 100_000)
                .unwrap_err()
                .contains("missing static")
        );
    }

    #[test]
    fn capturing_effect_handlers_and_requests_validate_and_check_branch_results() {
        let source = SourceRange { start: 1, end: 2 };
        let op = nir::effects::Operation {
            declaration: DeclarationId {
                package: 1,
                local: 50,
            },
            arguments: vec![],
        };
        let op_sig = Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::Runtime,
                ty: Term::Primitive(nir::Primitive::U32),
            }],
            result: Box::new(Term::Primitive(nir::Primitive::U32)),
        };
        let u32_expr = |val: u32| Expr {
            source,
            ty: Term::Primitive(nir::Primitive::U32),
            kind: K::Literal(Literal::Integer(val.to_string())),
        };
        let bool_expr = |val: bool| Expr {
            source,
            ty: Term::Primitive(nir::Primitive::Bool),
            kind: K::Literal(Literal::Bool(val)),
        };
        let request_expr = Expr {
            source,
            ty: Term::Primitive(nir::Primitive::U32),
            kind: K::EffectRequest {
                operation: op.clone(),
                abi: nir::effects::Abi::Capturing,
                allowed_index: 0,
                signature: Box::new(op_sig.clone()),
                arguments: vec![u32_expr(7)],
            },
        };
        assert!(effects::suspension_boundary(&request_expr));
        let comp_expr = Expr {
            source,
            ty: Term::Computation {
                effects: vec![op.clone()],
                result: Box::new(Term::Primitive(nir::Primitive::Bool)),
            },
            kind: K::EffectComputation {
                effects: vec![op.clone()],
                body: Box::new(Expr {
                    source,
                    ty: Term::Primitive(nir::Primitive::Bool),
                    kind: K::Block {
                        statements: vec![Expr {
                            source,
                            ty: Term::Primitive(nir::Primitive::Void),
                            kind: K::Discard(Box::new(request_expr)),
                        }],
                        result: Some(Box::new(bool_expr(true))),
                    },
                }),
            },
        };
        let make_arm = |arm_body: Expr| nir::program::MatchArm {
            pattern: nir::program::Pattern::Tuple(vec![
                nir::program::Pattern::Wild,
                nir::program::Pattern::Wild,
            ]),
            guard: None,
            body: arm_body,
        };
        let handle_expr = Expr {
            source,
            ty: Term::Primitive(nir::Primitive::Bool),
            kind: K::EffectHandle {
                computation: Box::new(comp_expr),
                handlers: vec![nir::program::EffectHandler {
                    operation: op.clone(),
                    abi: nir::effects::Abi::Capturing,
                    signature: op_sig.clone(),
                    arms: vec![
                        make_arm(bool_expr(false)),
                        make_arm(Expr {
                            source,
                            ty: Term::Primitive(nir::Primitive::Never),
                            kind: K::EffectResume(Box::new(u32_expr(9))),
                        }),
                    ],
                    fallback: None,
                }],
                outer: vec![],
            },
        };
        assert!(!effects::suspension_boundary(&handle_expr));
        let valid_pkg = Package {
            functions: vec![make_runtime_fn(
                1,
                Expr {
                    source,
                    ty: Term::Primitive(nir::Primitive::Void),
                    kind: K::Discard(Box::new(handle_expr)),
                },
            )],
            ..Package::default()
        };
        valid_pkg.validate(&mut 100_000).unwrap();
        valid_pkg.validate_cleanup_control(&mut 100_000).unwrap();

        // Capturing arm returning operation result Y (u32) when handle produces B (bool) must fail.
        let mut wrong_arm_pkg = valid_pkg.clone();
        let K::Discard(inner) = &mut wrong_arm_pkg.functions[0].function.body.kind else {
            unreachable!()
        };
        let K::EffectHandle { handlers, .. } = &mut inner.kind else {
            unreachable!()
        };
        handlers[0].arms[0].body = u32_expr(1);
        assert!(
            wrong_arm_pkg
                .validate(&mut 100_000)
                .unwrap_err()
                .contains("IIR handler branch result mismatch")
        );

        // EffectResume inside capturing arm must still match operation result Y (u32), not bool.
        let mut wrong_resume_pkg = valid_pkg.clone();
        let K::Discard(inner) = &mut wrong_resume_pkg.functions[0].function.body.kind else {
            unreachable!()
        };
        let K::EffectHandle { handlers, .. } = &mut inner.kind else {
            unreachable!()
        };
        handlers[0].arms[1].body = Expr {
            source,
            ty: Term::Primitive(nir::Primitive::Never),
            kind: K::EffectResume(Box::new(bool_expr(false))),
        };
        assert!(
            wrong_resume_pkg
                .validate(&mut 100_000)
                .unwrap_err()
                .contains("IIR resume has no matching lexical branch result")
        );

        // Abi::Async remains rejected by both effect validation and cleanup control checks.
        let mut async_pkg = valid_pkg;
        let K::Discard(inner) = &mut async_pkg.functions[0].function.body.kind else {
            unreachable!()
        };
        let K::EffectHandle {
            computation,
            handlers,
            ..
        } = &mut inner.kind
        else {
            unreachable!()
        };
        handlers[0].abi = nir::effects::Abi::Async;
        let K::EffectComputation { body, .. } = &mut computation.kind else {
            unreachable!()
        };
        let K::Block { statements, .. } = &mut body.kind else {
            unreachable!()
        };
        let K::Discard(req) = &mut statements[0].kind else {
            unreachable!()
        };
        let K::EffectRequest { abi, .. } = &mut req.kind else {
            unreachable!()
        };
        *abi = nir::effects::Abi::Async;
        assert!(
            async_pkg
                .validate(&mut 100_000)
                .unwrap_err()
                .contains("IIR non-direct effects require control ABI lowering")
        );
        assert!(
            async_pkg
                .validate_cleanup_control(&mut 100_000)
                .unwrap_err()
                .contains("slot/capture cleanup protocol lowering")
        );
    }
}
