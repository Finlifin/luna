//! Structural integrity of retained definitions and selected dispatch.
use super::*;
use std::collections::{HashMap, HashSet};

impl Package {
    pub fn validate_index(&self) -> Result<(), String> {
        let mut functions = HashMap::new();
        for instance in &self.functions {
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
            if !functions.contains_key(root) {
                return Err(format!("missing IIR root {root}"));
            }
        }
        let mut nominals = HashSet::new();
        for nominal in &self.nominals {
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
                    e.validate_encoding().map_err(str::to_owned)?;
                    (e.declaration, &e.type_parameters)
                }
            };
            if *declaration != actual || !parameters.is_empty() {
                return Err("IIR nominal definition disagrees with its instance key".into());
            }
        }
        let mut heads = HashSet::new();
        for head in &self.normal_forms {
            if !heads.insert(head.declaration) {
                return Err("duplicate IIR normal-form head".into());
            }
        }
        let mut contracts = HashMap::new();
        for contract in &self.interfaces {
            if contracts
                .insert((&contract.interface, &contract.subject), contract)
                .is_some()
            {
                return Err("duplicate IIR interface contract".into());
            }
            let mut members = HashSet::new();
            for method in &contract.methods {
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
            let interface = dispatch.interface_instance();
            if !worlds.insert((interface.clone(), &dispatch.subject)) {
                return Err("duplicate IIR dispatch selection".into());
            }
            let contract = contracts
                .get(&(&interface, &dispatch.subject))
                .ok_or("IIR dispatch has no interface contract")?;
            let mut members = HashSet::new();
            for method in &dispatch.methods {
                if !members.insert((method.member, &method.method_arguments)) {
                    return Err("duplicate IIR dispatch member".into());
                }
                let declared = contract
                    .methods
                    .iter()
                    .find(|m| {
                        m.declaration == method.member
                            && m.type_arguments == method.method_arguments
                    })
                    .ok_or("IIR dispatch member is absent from its interface")?;
                let function = functions
                    .get(&InstanceKey {
                        declaration: method.body,
                        arguments: method.arguments.clone(),
                    })
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
            if !worlds.contains(&(contract.interface.clone(), &contract.subject)) {
                return Err("IIR interface has no dispatch selection".into());
            }
        }
        Ok(())
    }
}
