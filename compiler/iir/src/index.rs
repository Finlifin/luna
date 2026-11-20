//! Structural integrity of retained definitions and selected dispatch.
use super::*;
use std::collections::{HashMap, HashSet};

/// The validation walk borrows the same executable and template declarations
/// that established uniqueness and dispatch closure.
pub(super) struct CallableLinks<'a> {
    functions: HashMap<(DeclarationId, &'a [Term]), &'a Function>,
    templates: HashMap<(DeclarationId, &'a [Term]), &'a Term>,
    identities: HashMap<(DeclarationId, &'a [Term]), &'a Term>,
    methods: HashMap<
        crate::type_links::MethodKey<'a>,
        (
            &'a nir::program::MethodContract,
            &'a nir::program::MethodInstance,
        ),
    >,
}

impl CallableLinks<'_> {
    fn resolve_direct_target(&self, expr: &nir::program::Expr) -> Option<&Function> {
        use nir::program::ExprKind as K;
        let key = if let Some(key) = expr.named_function() {
            key
        } else if let K::TraitMethod {
            interface,
            interface_arguments,
            subject,
            member,
            method_arguments,
        } = &expr.kind
        {
            let (_, target) = self.methods.get(&(
                *interface,
                interface_arguments.as_slice(),
                subject,
                *member,
                method_arguments.as_slice(),
            ))?;
            (target.body, target.arguments.as_slice())
        } else {
            return None;
        };
        self.functions.get(&key).copied()
    }

    pub(super) fn validate_reference(
        &self,
        expr: &nir::program::Expr,
        comptime: bool,
        direct_callee: bool,
        fuel: &mut usize,
    ) -> Result<(), String> {
        use nir::program::ExprKind as K;
        if let K::Call {
            callee,
            env_arguments,
            ..
        } = &expr.kind
        {
            if comptime != callee.ty.is_comptime_callable() {
                return Err("IIR call target belongs to a different execution phase".into());
            }
            if let Some(target) = self.resolve_direct_target(callee) {
                if env_arguments.len() != target.env_parameters.len() {
                    return Err("IIR call runtime environment argument count mismatch".into());
                }
                for (argument, parameter) in env_arguments.iter().zip(&target.env_parameters) {
                    if argument.ty != parameter.ty {
                        return Err("IIR call runtime environment argument type mismatch".into());
                    }
                }
            } else if !env_arguments.is_empty() {
                return Err("IIR indirect call cannot pass runtime environment arguments".into());
            }
        }
        let transport = comptime && !expr.ty.is_comptime_callable();
        let key = if let Some(key) = expr.named_function() {
            key
        } else if let K::TraitMethod {
            interface,
            interface_arguments,
            subject,
            member,
            method_arguments,
        } = &expr.kind
        {
            terms(interface_arguments, fuel)?;
            term(subject, fuel)?;
            terms(method_arguments, fuel)?;
            let (contract, target) = self
                .methods
                .get(&(
                    *interface,
                    interface_arguments.as_slice(),
                    subject,
                    *member,
                    method_arguments.as_slice(),
                ))
                .ok_or("IIR trait reference has no selected method")?;
            if contract.is_quote {
                return Err("IIR quote method requires completed deterministic expansion".into());
            }
            if transport {
                term(&expr.ty, fuel)?;
                if contract.signature.ordinary().map_err(|e| e.to_string())? != &expr.ty {
                    return Err("IIR transported method changed interface signature".into());
                }
            }
            (target.body, target.arguments.as_slice())
        } else {
            return self.validate_clause(expr, comptime, fuel);
        };
        terms(key.1, fuel)?;
        if transport && let Some(&signature) = self.identities.get(&key) {
            term(&expr.ty, fuel)?;
            if signature != &expr.ty {
                return Err("IIR transported function changed signature".into());
            }
            return Ok(());
        }
        if let Some(&ty) = self.templates.get(&key) {
            if ty != &expr.ty {
                return Err("IIR template reference changed logical type".into());
            }
            return Ok(());
        }
        let function = self
            .functions
            .get(&key)
            .ok_or("IIR callable reference is dangling")?;
        if (function.phase == nir::program::FunctionPhase::Comptime)
            != expr.ty.is_comptime_callable()
        {
            return Err("IIR callable reference changed execution phase".into());
        }
        if function.is_unsafe != expr.ty.is_unsafe_callable() {
            return Err("IIR callable reference changed unsafe responsibility".into());
        }
        if !direct_callee && !function.env_parameters.is_empty() {
            return Err(
                "function with runtime environment parameters cannot form a bare function value"
                    .into(),
            );
        }
        if transport {
            term(&function.signature, fuel)?;
            if function.callable_type_view().as_ref() != &expr.ty {
                return Err("IIR transported function changed signature".into());
            }
        }
        Ok(())
    }

    fn validate_clause(
        &self,
        expr: &nir::program::Expr,
        comptime: bool,
        fuel: &mut usize,
    ) -> Result<(), String> {
        if matches!(
            expr.kind,
            nir::program::ExprKind::PlannedFunctionInstance { .. }
        ) {
            return Err("IIR contains a source callable with unresolved clause inputs".into());
        }
        let nir::program::ExprKind::ComptimeFunctionInstance {
            declaration,
            inputs,
        } = &expr.kind
        else {
            return Ok(());
        };
        if !comptime || !expr.ty.is_comptime_callable() {
            return Err("IIR clause computation requires comptime invocation".into());
        }
        let arguments = observed_clause_arguments(*declaration, inputs, fuel)?;
        let key = (*declaration, arguments.as_slice());
        let target = self
            .functions
            .get(&key)
            .ok_or("IIR clause dependency has no executable instance")?;
        if target.phase != nir::program::FunctionPhase::Comptime
            || target.is_unsafe != expr.ty.is_unsafe_callable()
            || target.signature.callable_signature() != expr.ty.callable_signature()
        {
            return Err("IIR clause dependency changed callable signature or phase".into());
        }
        Ok(())
    }
}

impl Package {
    pub fn validate_index(&self) -> Result<(), String> {
        self.validate_index_with_budget(&mut 100_000)
    }

    /// Guard structural keys before recursive hashing, comparison or cloning.
    pub fn validate_index_with_budget(&self, fuel: &mut usize) -> Result<(), String> {
        self.checked_callables(fuel).map(|_| ())
    }

    pub(super) fn checked_callables(&self, fuel: &mut usize) -> Result<CallableLinks<'_>, String> {
        spend(fuel, 1)?;
        let mut identities = HashMap::new();
        for identity in &self.callable_identities {
            key(&identity.key, fuel)?;
            if identities
                .insert(identity.key.parts(), identity.closed_signature()?)
                .is_some()
            {
                return Err("duplicate IIR callable signature evidence".into());
            }
        }
        let mut template_targets = HashMap::new();
        for table in &self.templates {
            term(table.logical_type(), fuel)?;
            for target in table.targets() {
                key(target, fuel)?;
                if identities
                    .get(&target.parts())
                    .is_some_and(|ty| *ty != table.logical_type())
                {
                    return Err("IIR template and retained identity disagree on signature".into());
                }
                if template_targets
                    .insert(target.parts(), table.logical_type())
                    .is_some()
                {
                    return Err("duplicate IIR template identity".into());
                }
            }
        }
        let mut functions = HashMap::new();
        for instance in &self.functions {
            key(&instance.key, fuel)?;
            term(&instance.function.signature, fuel)?;
            spend(fuel, instance.function.parameters.len())?;
            if template_targets.contains_key(&instance.key.parts()) {
                return Err("IIR identity is both a template and executable instance".into());
            }
            if let Some(signature) = identities.get(&instance.key.parts())
                && instance.function.callable_type_view().as_ref() != *signature
            {
                return Err("IIR executable and retained identity disagree on signature".into());
            }
            if functions
                .insert(instance.key.parts(), &instance.function)
                .is_some()
            {
                return Err("duplicate IIR function instance".into());
            }
            let function = &instance.function;
            if function.phase == nir::program::FunctionPhase::NormalForm {
                return Err("IIR cannot execute a normal-form head".into());
            }
            if function.declaration != instance.key.declaration
                || !function.type_parameters.is_empty()
            {
                return Err("IIR function retains unresolved declaration parameters".into());
            }
            let Term::Function { parameters, .. } = function.signature.callable_signature() else {
                return Err("IIR function signature is not callable".into());
            };
            if function.phase == nir::program::FunctionPhase::Runtime
                && parameters.iter().any(|p| p.sort != nir::Sort::Runtime)
            {
                return Err("IIR runtime function retains staged inputs".into());
            }
            if parameters.len() != function.parameters.len()
                || function.parameters.iter().collect::<BTreeSet<_>>().len()
                    != function.parameters.len()
            {
                return Err("IIR function parameter bindings do not match its signature".into());
            }
            spend(fuel, function.env_parameters.len())?;
            let mut bindings: BTreeSet<_> = function.parameters.iter().copied().collect();
            for env in &function.env_parameters {
                if !bindings.insert(env.binding) {
                    return Err(
                        "IIR function runtime environment binding collides with parameter".into(),
                    );
                }
            }
        }
        for root in &self.roots {
            key(root, fuel)?;
            if !functions.contains_key(&root.parts()) {
                return Err(format!("missing IIR root {root}"));
            }
        }
        let mut nominals = HashSet::new();
        for nominal in &self.nominals {
            term(&nominal.ty, fuel)?;
            if !nominals.insert(&nominal.ty) {
                return Err("duplicate IIR nominal instance".into());
            }
            let Term::Nominal { declaration, .. } = &nominal.ty else {
                return Err("IIR nominal instance has a non-nominal key".into());
            };
            let (actual, parameters) = match &nominal.definition {
                NominalDefinition::Struct(s) => (s.declaration, &s.type_parameters),
                NominalDefinition::Newtype(n) => (n.declaration, &n.type_parameters),
                NominalDefinition::Enum(e) => {
                    enum_metadata(e, fuel)?;
                    e.validate_encoding_in(self.target).map_err(str::to_owned)?;
                    (e.declaration, &e.type_parameters)
                }
            };
            if *declaration != actual || !parameters.is_empty() {
                return Err("IIR nominal definition disagrees with its instance key".into());
            }
        }
        let mut heads = HashSet::new();
        for head in &self.normal_forms {
            spend(fuel, 1)?;
            if !heads.insert(head.declaration) {
                return Err("duplicate IIR normal-form head".into());
            }
        }
        let mut contracts = HashMap::new();
        for contract in &self.interfaces {
            spend(fuel, 1)?;
            terms(&contract.interface.arguments, fuel)?;
            term(&contract.subject, fuel)?;
            let world = (
                contract.interface.declaration,
                contract.interface.arguments.as_slice(),
                &contract.subject,
            );
            let mut members = HashMap::new();
            for method in &contract.methods {
                spend(fuel, 1)?;
                terms(&method.type_arguments, fuel)?;
                term(
                    method.signature.ordinary().map_err(|e| e.to_string())?,
                    fuel,
                )?;
                spend(fuel, method.parameter_bindings.len())?;
                if members
                    .insert(
                        (method.declaration, method.type_arguments.as_slice()),
                        method,
                    )
                    .is_some()
                {
                    return Err("duplicate IIR interface member".into());
                }
                let Ok(Term::Function { parameters, .. }) = method.signature.callable_signature()
                else {
                    return Err("IIR interface member is not callable".into());
                };
                if parameters.len() != method.parameter_bindings.len()
                    || method
                        .parameter_bindings
                        .iter()
                        .collect::<BTreeSet<_>>()
                        .len()
                        != parameters.len()
                {
                    return Err(
                        "IIR interface parameter bindings do not match its signature".into(),
                    );
                }
                if !method.type_parameters.is_empty() {
                    return Err("IIR interface retains unresolved method parameters".into());
                }
                if method.default.is_some() {
                    return Err("IIR interface retains an unselected default body".into());
                }
            }
            if contracts.insert(world, (members, false)).is_some() {
                return Err("duplicate IIR interface contract".into());
            }
        }
        let mut methods = HashMap::new();
        for dispatch in &self.dispatch {
            spend(fuel, 1)?;
            terms(&dispatch.interface_arguments, fuel)?;
            term(&dispatch.subject, fuel)?;
            let world = (
                dispatch.interface,
                dispatch.interface_arguments.as_slice(),
                &dispatch.subject,
            );
            let (members, selected) = contracts
                .get_mut(&world)
                .ok_or("IIR dispatch has no interface contract")?;
            if std::mem::replace(selected, true) {
                return Err("duplicate IIR dispatch selection".into());
            }
            for method in &dispatch.methods {
                spend(fuel, 1)?;
                terms(&method.method_arguments, fuel)?;
                terms(&method.arguments, fuel)?;
                let declared = *members
                    .get(&(method.member, method.method_arguments.as_slice()))
                    .ok_or("IIR dispatch member is absent from its interface")?;
                terms(&dispatch.interface_arguments, fuel)?;
                term(&dispatch.subject, fuel)?;
                if methods
                    .insert(
                        (
                            dispatch.interface,
                            dispatch.interface_arguments.as_slice(),
                            &dispatch.subject,
                            method.member,
                            method.method_arguments.as_slice(),
                        ),
                        (declared, method),
                    )
                    .is_some()
                {
                    return Err("duplicate IIR dispatch member".into());
                }
                let key = (method.body, method.arguments.as_slice());
                if let Some(&ty) = template_targets.get(&key) {
                    if declared.signature.ordinary().map_err(|e| e.to_string())? != ty
                        || declared.signature.is_comptime_callable()
                    {
                        return Err(
                            "IIR template method disagrees with its interface signature".into()
                        );
                    }
                    continue;
                }
                if let Some(&signature) = identities.get(&key) {
                    if declared.signature.ordinary().map_err(|e| e.to_string())? != signature {
                        return Err(
                            "IIR transported method disagrees with its interface signature".into(),
                        );
                    }
                    if !functions.contains_key(&key) && !signature.is_comptime_callable() {
                        continue;
                    }
                }
                let function = functions
                    .get(&key)
                    .ok_or("IIR dispatch refers to a missing function instance")?;
                if (function.phase == nir::program::FunctionPhase::Comptime)
                    != declared.signature.is_comptime_callable()
                    || function.parameters.len() != declared.parameter_bindings.len()
                {
                    return Err(
                        "IIR dispatch body disagrees with the interface calling convention".into(),
                    );
                }
            }
        }
        spend(fuel, contracts.len())?;
        if contracts.values().any(|(_, selected)| !selected) {
            return Err("IIR interface has no dispatch selection".into());
        }
        Ok(CallableLinks {
            functions,
            templates: template_targets,
            identities,
            methods,
        })
    }
}

fn spend(fuel: &mut usize, work: usize) -> Result<(), String> {
    *fuel = fuel.checked_sub(work).ok_or("IIR index budget exhausted")?;
    Ok(())
}

fn term(value: &Term, fuel: &mut usize) -> Result<(), String> {
    nir::type_computations::charge_term(value, fuel)
        .map_err(|e| format!("IIR index key budget: {e}"))
}

pub(super) fn terms(values: &[Term], fuel: &mut usize) -> Result<(), String> {
    for value in values {
        term(value, fuel)?;
    }
    Ok(())
}

fn key(value: &InstanceKey, fuel: &mut usize) -> Result<(), String> {
    spend(fuel, 1)?;
    terms(&value.arguments, fuel)
}

// Encoding constructors already validate bit masks. Index validation observes
// logical names, field counts/types and the selected representation metadata.
fn enum_metadata(value: &nir::program::Enum, fuel: &mut usize) -> Result<(), String> {
    for group in &value.groups {
        spend(fuel, group.len().saturating_add(1))?;
    }
    for variant in &value.variants {
        spend(fuel, variant.name.len().saturating_add(1))?;
        spend(fuel, variant.fields.len())?;
        if let nir::program::VariantShape::Named(names) = &variant.shape {
            for name in names {
                spend(fuel, name.len().saturating_add(1))?;
            }
        }
    }
    if let Some(encoding) = &value.encoding {
        spend(fuel, 1)?;
        for variant in encoding.variants() {
            spend(fuel, variant.name().len().saturating_add(1))?;
            for field in variant.fields() {
                spend(fuel, field.name.len().saturating_add(1))?;
            }
        }
    }
    if let Some(encoding) = &value.symbol_encoding {
        for variant in encoding.variants() {
            spend(fuel, variant.name.len().saturating_add(1))?;
        }
    }
    Ok(())
}
