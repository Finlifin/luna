//! Structural integrity of retained definitions and selected dispatch.
use super::*;
use std::collections::{HashMap, HashSet};

impl Package {
    pub fn validate_index(&self) -> Result<(), String> {
        self.validate_index_with_budget(&mut 100_000)
    }

    /// Guard structural keys before recursive hashing, comparison or cloning.
    pub fn validate_index_with_budget(&self, fuel: &mut usize) -> Result<(), String> {
        spend(fuel, 1)?;
        let mut callable_keys = HashSet::new();
        for identity in &self.callable_identities {
            key(&identity.key, fuel)?;
            if !callable_keys.insert(&identity.key) {
                return Err("duplicate IIR callable signature evidence".into());
            }
        }
        let mut template_targets = HashMap::new();
        for table in &self.templates {
            term(table.logical_type(), fuel)?;
            for target in table.targets() {
                key(target, fuel)?;
                if template_targets
                    .insert(target, table.logical_type())
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
            if template_targets.contains_key(&instance.key) {
                return Err("IIR identity is both a template and executable instance".into());
            }
            if functions
                .insert(&instance.key, &instance.function)
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
        }
        for root in &self.roots {
            key(root, fuel)?;
            if !functions.contains_key(root) {
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
            if contracts
                .insert((&contract.interface, &contract.subject), contract)
                .is_some()
            {
                return Err("duplicate IIR interface contract".into());
            }
            let mut members = HashSet::new();
            for method in &contract.methods {
                spend(fuel, 1)?;
                terms(&method.type_arguments, fuel)?;
                term(
                    method.signature.ordinary().map_err(|e| e.to_string())?,
                    fuel,
                )?;
                spend(fuel, method.parameter_bindings.len())?;
                if !members.insert((method.declaration, &method.type_arguments)) {
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
        }
        let mut worlds = HashSet::new();
        for dispatch in &self.dispatch {
            spend(fuel, 1)?;
            terms(&dispatch.interface_arguments, fuel)?;
            term(&dispatch.subject, fuel)?;
            let interface = dispatch.interface_instance();
            if !worlds.insert((interface.clone(), &dispatch.subject)) {
                return Err("duplicate IIR dispatch selection".into());
            }
            let contract = contracts
                .get(&(&interface, &dispatch.subject))
                .ok_or("IIR dispatch has no interface contract")?;
            let mut members = HashSet::new();
            for method in &dispatch.methods {
                spend(fuel, 1)?;
                terms(&method.method_arguments, fuel)?;
                terms(&method.arguments, fuel)?;
                if !members.insert((method.member, &method.method_arguments)) {
                    return Err("duplicate IIR dispatch member".into());
                }
                // Bound the complete candidate scan, including argument equality.
                for candidate in &contract.methods {
                    spend(fuel, 1)?;
                    terms(&candidate.type_arguments, fuel)?;
                    terms(&method.method_arguments, fuel)?;
                }
                let declared = contract
                    .methods
                    .iter()
                    .find(|m| {
                        m.declaration == method.member
                            && m.type_arguments == method.method_arguments
                    })
                    .ok_or("IIR dispatch member is absent from its interface")?;
                let key = InstanceKey {
                    declaration: method.body,
                    arguments: method.arguments.clone(),
                };
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
        for contract in &self.interfaces {
            spend(fuel, 1)?;
            terms(&contract.interface.arguments, fuel)?;
            term(&contract.subject, fuel)?;
            if !worlds.contains(&(contract.interface.clone(), &contract.subject)) {
                return Err("IIR interface has no dispatch selection".into());
            }
        }
        Ok(())
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
fn terms(values: &[Term], fuel: &mut usize) -> Result<(), String> {
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
