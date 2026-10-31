//! Concrete high-level instances. Structured NIR operations are retained; this
//! stage neither invents SSA names nor erases program contracts or opaque heads.
pub mod effect_values;
pub mod effects;
mod index;
pub mod staging;
mod storage;
pub mod template_storage;
pub mod visit;
use nir::program::{DispatchInstance, Function};
use nir::{DeclarationId, RuntimeId, Term};
use std::{collections::BTreeSet, fmt};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InstanceKey {
    pub declaration: DeclarationId,
    pub arguments: Vec<Term>,
}
impl fmt::Display for InstanceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(instance-key (declaration {}) (arguments",
            self.declaration
        )?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        f.write_str("))")
    }
}
/// Declaration evidence for a callable retained only inside immutable types.
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
        nir::copy_budget::charge_type_substitution(&signature, &[], &[], fuel)
            .map_err(|error| format!("IIR callable signature budget: {error:?}"))?;
        completed_type(&signature, &BTreeSet::new(), fuel)?;
        if !matches!(signature.callable_signature(), Term::Function { .. }) {
            return Err("IIR callable identity has a non-callable signature".into());
        }
        let core = interner
            .intern(vec![], signature)
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
}
impl fmt::Display for FunctionInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(function-instance {} {})", self.key, self.function)
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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Package {
    pub target: Option<nir::target::TargetData>,
    pub expansion_sources: Vec<nir::program::ExpansionSource>,
    pub roots: Vec<InstanceKey>,
    pub nominals: Vec<NominalInstance>,
    pub interfaces: Vec<InterfaceContract>,
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
                table
                    .targets()
                    .iter()
                    .any(|key| key.declaration == method.body && key.arguments == method.arguments)
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
    /// Validate completed annotations and closure of every named callable
    /// reference, including references embedded in defaults and patterns.
    pub fn validate(&self, fuel: &mut usize) -> Result<(), String> {
        self.validate_index()?;
        for identity in &self.callable_identities {
            let signature = identity.closed_signature()?;
            if !matches!(signature.callable_signature(), Term::Function { .. }) {
                return Err("IIR callable identity has a non-callable signature".into());
            }
            self.validate_type(signature, fuel)?;
            for argument in &identity.key.arguments {
                self.validate_type(argument, fuel)?;
            }
        }
        for table in &self.templates {
            self.validate_type(table.logical_type(), fuel)?;
            for target in table.targets() {
                for argument in &target.arguments {
                    self.validate_type(argument, fuel)?;
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
            self.validate_type(&nominal.ty, fuel)?;
            match &nominal.definition {
                NominalDefinition::Newtype(n) => self.validate_type(&n.representation, fuel)?,
                NominalDefinition::Struct(s) => {
                    for (_, ty) in &s.fields {
                        self.validate_type(ty, fuel)?;
                    }
                }
                NominalDefinition::Enum(e) => {
                    for variant in &e.variants {
                        for ty in &variant.fields {
                            self.validate_type(ty, fuel)?;
                        }
                    }
                }
            }
        }
        for interface in &self.interfaces {
            for argument in &interface.interface.arguments {
                self.validate_type(argument, fuel)?;
            }
            self.validate_type(&interface.subject, fuel)?;
            for method in &interface.methods {
                self.validate_type(
                    method.signature.ordinary().map_err(|e| e.to_string())?,
                    fuel,
                )?;
                for argument in &method.type_arguments {
                    self.validate_type(argument, fuel)?;
                }
                let summary = method
                    .program_summary(&[], &[])
                    .map_err(|e| e.to_string())?;
                visit::expressions(&summary, fuel, |expr, fuel| {
                    self.validate_expression_types(expr, fuel)
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
        let layouts: std::collections::HashMap<_, _> = self
            .nominals
            .iter()
            .map(|instance| (&instance.ty, &instance.definition))
            .collect();
        let resolve_storage = |ty: &Term, fuel: &mut usize| -> Option<Vec<Term>> {
            let mut copy_field = |field: &Term| {
                nir::type_computations::charge_term(field, fuel).ok()?;
                Some(field.clone())
            };
            match layouts.get(ty)? {
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
                    Some(vec![copy_field(&definition.representation)?])
                }
            }
        };
        use nir::program::ExprKind as K;
        for instance in &self.functions {
            self.validate_type(&instance.function.signature, fuel)?;
            nir::call_safety::validate(&instance.function, fuel)?;
            if instance.function.is_unsafe && self.roots.contains(&instance.key) {
                return Err("unsafe entry requires an explicit safe wrapper".into());
            }
            if instance.function.phase == nir::program::FunctionPhase::Runtime {
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
            }

            storage::validate(
                &instance.function,
                &storage_metadata,
                &resolve_storage,
                fuel,
            )?;
            for argument in &instance.key.arguments {
                self.validate_type(argument, fuel)?;
            }
            visit::expressions(&instance.function, fuel, |expr, fuel| {
                if matches!(
                    expr.kind,
                    K::Literal(
                        nir::program::Literal::PendingFloat { .. }
                            | nir::program::Literal::PendingInteger { .. }
                    )
                ) {
                    return Err("IIR contains an unformed float or integer literal".into());
                }

                self.validate_expression_types(expr, fuel)?;
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
                    if instance.function.phase != nir::program::FunctionPhase::Runtime
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
                if instance.function.phase == nir::program::FunctionPhase::Runtime {
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
                if let Some(message) = nir::program::pointer_operation_error_in(expr, self.target) {
                    return Err(message.into());
                }
                let target = match &expr.kind {
                    K::Function(id) => Some(InstanceKey {
                        declaration: *id,
                        arguments: vec![],
                    }),
                    K::FunctionInstance {
                        declaration,
                        arguments,
                    } => Some(InstanceKey {
                        declaration: *declaration,
                        arguments: arguments.clone(),
                    }),
                    K::TraitMethod {
                        method_arguments,
                        interface,
                        interface_arguments,
                        member,
                        subject,
                    } => {
                        if self.interfaces.iter().any(|contract| {
                            contract.interface.declaration == *interface
                                && contract.interface.arguments == *interface_arguments
                                && contract.subject == *subject
                                && contract
                                    .methods
                                    .iter()
                                    .any(|method| method.declaration == *member && method.is_quote)
                        }) {
                            return Err(
                                "IIR quote method requires completed deterministic expansion"
                                    .into(),
                            );
                        }
                        let selected = self
                            .dispatch
                            .iter()
                            .find(|d| {
                                d.interface == *interface
                                    && d.interface_arguments == *interface_arguments
                                    && d.subject == *subject
                            })
                            .ok_or("IIR trait reference has no selected implementation")?;
                        let method = selected
                            .methods
                            .iter()
                            .find(|m| {
                                m.member == *member && m.method_arguments == *method_arguments
                            })
                            .ok_or("IIR trait reference has no selected method")?;
                        Some(InstanceKey {
                            declaration: method.body,
                            arguments: method.arguments.clone(),
                        })
                    }

                    K::ComptimeValue(_) | K::BuildMetadata { .. } => {
                        return Err("IIR requires captured comptime value materialization".into());
                    }
                    K::AssociatedValue { .. } => {
                        return Err("IIR requires associated value selection".into());
                    }
                    _ => None,
                };
                if let Some(target) = target {
                    if let Some(ty) = self.template_type(&target) {
                        if ty != &expr.ty {
                            return Err("IIR template reference changed logical type".into());
                        }
                        return Ok(());
                    }
                    let target = self
                        .function(&target)
                        .ok_or("IIR callable reference is dangling")?;
                    let comptime = target.function.phase == nir::program::FunctionPhase::Comptime;
                    if comptime != expr.ty.is_comptime_callable() {
                        return Err("IIR callable reference changed execution phase".into());
                    }
                    if target.function.is_unsafe != expr.ty.is_unsafe_callable() {
                        return Err("IIR callable reference changed unsafe responsibility".into());
                    }
                }
                Ok(())
            })?;
            effects::validate(self, &instance.function, fuel)?;
        }
        effects::validate_contracts(self, fuel)?;
        Ok(())
    }
    /// Effect control exits need acquired cleanup schedules before this graph
    /// may participate in resource-checked execution or destructor invocation.
    pub fn validate_cleanup_control(&self, fuel: &mut usize) -> Result<(), String> {
        for instance in &self.functions {
            visit::expressions(&instance.function, fuel, |expr, fuel| {
                use nir::program::ExprKind as K;
                match &expr.kind {
                    K::EffectHandle { handlers, .. } => {
                        for handler in handlers {
                            if handler.abi != nir::effects::Abi::Direct
                                && !(handler.abi == nir::effects::Abi::DirectPassing
                                    && effects::data_passing_signature(
                                        self,
                                        &handler.signature,
                                        fuel,
                                    )?)
                            {
                                return Err("IIR effect installation requires slot/capture cleanup protocol lowering".into());
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
                        if *abi != nir::effects::Abi::Direct
                            && !(*abi == nir::effects::Abi::DirectPassing
                                && effects::data_passing_signature(self, signature, fuel)?) =>
                    {
                        return Err(
                            "IIR effect request requires slot/capture cleanup protocol lowering"
                                .into(),
                        );
                    }
                    _ => {}
                }
                Ok(())
            })?;
        }
        Ok(())
    }
    // Contracts and executable bodies must inspect the same embedded types.
    // Validating only Expr::ty misses constants and capture/storage annotations.
    fn validate_expression_types(
        &self,
        expr: &nir::program::Expr,
        fuel: &mut usize,
    ) -> Result<(), String> {
        use nir::program::ExprKind as K;
        self.validate_type(&expr.ty, fuel)?;
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
                        let mut found = false;
                        for nominal in &self.nominals {
                            *fuel = fuel
                                .checked_sub(1)
                                .ok_or("IIR tag_name layout budget exhausted")?;
                            if nominal.ty == operand.ty {
                                found = matches!(nominal.definition, NominalDefinition::Enum(_));
                                break;
                            }
                        }
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
            let mut family = None;
            for nominal in &self.nominals {
                *fuel = fuel
                    .checked_sub(1)
                    .ok_or("IIR intrinsic tag family lookup budget exhausted")?;
                if matches!(&nominal.ty, Term::Nominal { declaration, arguments }
                    if *declaration == tag.family && arguments.is_empty())
                {
                    if let NominalDefinition::Enum(definition) = &nominal.definition {
                        family = Some(definition);
                    }
                    break;
                }
            }
            let family = family.ok_or("IIR intrinsic tag family instance is missing")?;
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
                self.validate_type(ty, fuel)
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
                    &|ty| self.normal_forms.iter().find_map(|m| m.tuple_elements(ty)),
                )
                .map_err(|e| format!("IIR error result conversion: {e}"))?;
            }
            K::NewtypeConvert(value) => {
                if self
                    .newtype_conversion(&value.ty, &expr.ty, fuel)
                    .map_err(|e| e.to_string())?
                    .is_none()
                {
                    return Err("IIR newtype conversion has no direct representation edge".into());
                }
            }
            K::TypeList(fields) => {
                let diverging = {
                    for field in fields {
                        *fuel = fuel
                            .checked_sub(1)
                            .ok_or("IIR type-list budget exhausted")?;
                        if !matches!(
                            field.ty,
                            Term::Primitive(nir::Primitive::Type | nir::Primitive::Never)
                        ) {
                            return Err("IIR type-list operand is not Type".into());
                        }
                    }
                    fields
                        .iter()
                        .any(|field| field.ty == Term::Primitive(nir::Primitive::Never))
                };
                if diverging {
                    if expr.ty != Term::Primitive(nir::Primitive::Never) {
                        return Err("IIR diverging type-list construction must be Never".into());
                    }
                    return Ok(());
                }
                let mut admitted = false;
                for head in &self.normal_forms {
                    *fuel = fuel
                        .checked_sub(1)
                        .ok_or("IIR type-list metadata budget exhausted")?;
                    admitted |= head.is_type_list_instance(&expr.ty);
                }
                if !admitted || !nir::normal_forms::type_list_input_type(&expr.ty) {
                    return Err(
                        "IIR type-list constant requires trusted meta.List<Type> metadata".into(),
                    );
                }
            }
            K::TypeValue(ty) => {
                if expr.ty != Term::Primitive(nir::Primitive::Type) || ty.sort() != nir::Sort::Type
                {
                    return Err("IIR Type value requires a Type domain and type identity".into());
                }
                self.validate_type(ty, fuel)?;
            }
            K::BitAssign {
                storage_type: ty, ..
            }
            | K::BitUpdate {
                storage_type: ty, ..
            } => self.validate_type(ty, fuel)?,
            K::Closure { captures, .. } => {
                for capture in captures {
                    self.validate_type(&capture.ty, fuel)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn validate_type(&self, ty: &Term, fuel: &mut usize) -> Result<(), String> {
        completed_type(ty, &self.runtime_bindings, fuel)?;
        self.type_links(ty, fuel)
    }
    fn type_links(&self, ty: &Term, fuel: &mut usize) -> Result<(), String> {
        let mut pending = vec![ty];
        while let Some(ty) = pending.pop() {
            if *fuel == 0 {
                return Err("IIR type link budget exhausted".into());
            }
            *fuel -= 1;
            match ty {
                Term::FunctionValue {
                    declaration,
                    arguments,
                } => {
                    *fuel = fuel
                        .checked_sub(self.callable_identities.len())
                        .ok_or("IIR callable identity lookup budget exhausted")?;
                    if !self.callable_identities.iter().any(|identity| {
                        identity.key.declaration == *declaration
                            && identity.key.arguments == *arguments
                    }) {
                        return Err("IIR frozen callable has no signature evidence".into());
                    }
                }
                Term::Nominal { .. } => {
                    if !self.nominals.iter().any(|n| &n.ty == ty) {
                        return Err("IIR nominal reference is dangling".into());
                    }
                }
                Term::NormalForm { head, .. } => {
                    if !self.normal_forms.iter().any(|h| h.declaration == *head) {
                        return Err("IIR normal-form reference is dangling".into());
                    }
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
