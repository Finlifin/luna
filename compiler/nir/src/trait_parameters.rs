//! Declaration domains for trait indices. Identity and domain formation do not
//! establish an implementation, a runtime environment, or a program contract.
use crate::{DeclarationParameter, Primitive, Sort, Term, program::Trait};
use std::collections::BTreeSet;

impl Trait {
    pub fn parameter_binders(&self, fuel: &mut usize) -> Result<Vec<DeclarationParameter>, String> {
        *fuel = fuel
            .checked_sub(self.type_parameters.len().saturating_add(1))
            .ok_or("trait parameter budget exhausted")?;
        if !self.parameter_types.is_empty()
            && self.parameter_types.len() != self.type_parameters.len()
        {
            return Err("trait parameter domain count mismatch".into());
        }
        let mut seen = BTreeSet::from([self.self_parameter]);
        let mut result = Vec::new();
        for (index, declaration) in self.type_parameters.iter().enumerate() {
            if !seen.insert(*declaration) {
                return Err("duplicate trait parameter identity".into());
            }
            let domain = self
                .parameter_types
                .get(index)
                .unwrap_or(&Term::Primitive(Primitive::Type));
            crate::type_computations::charge_term(domain, fuel).map_err(|e| e.to_string())?;
            if domain.sort() != Sort::Type {
                return Err("trait parameter domain must be a Type".into());
            }
            result.push(DeclarationParameter {
                declaration: *declaration,
                sort: if *domain == Term::Primitive(Primitive::Type) {
                    Sort::Type
                } else {
                    Sort::ComptimeValue
                },
            });
        }
        Ok(result)
    }

    pub fn binders_with_self(&self, fuel: &mut usize) -> Result<Vec<DeclarationParameter>, String> {
        let parameters = self.parameter_binders(fuel)?;
        Ok(std::iter::once(DeclarationParameter {
            declaration: self.self_parameter,
            sort: Sort::Type,
        })
        .chain(parameters)
        .collect())
    }

    /// Validate completed input domains in order. An untyped store handle or
    /// symbolic value is not guessed to inhabit the requested source type.
    pub fn validate_arguments(
        &self,
        arguments: &[Term],
        scope: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<(), String> {
        self.validate_arguments_in_declarations(arguments, scope, &[], fuel)
    }

    /// Open trait definitions may supply rigid value indices. Their domains must
    /// come from a unique declaration, never from the demanded interface slot.
    pub fn validate_arguments_in_declarations(
        &self,
        arguments: &[Term],
        scope: &[Vec<Sort>],
        declarations: &[Trait],
        fuel: &mut usize,
    ) -> Result<(), String> {
        let binders = self.parameter_binders(fuel)?;
        if arguments.len() != binders.len() {
            return Err("trait interface argument count mismatch".into());
        }
        for (index, argument) in arguments.iter().enumerate() {
            crate::type_computations::charge_term(argument, fuel).map_err(|e| e.to_string())?;
            argument.check_scope(scope).map_err(|e| e.to_string())?;
            if argument.sort() != binders[index].sort {
                return Err("trait interface argument sort mismatch".into());
            }
            if binders[index].sort == Sort::Type {
                continue;
            }
            let domain = self.parameter_types[index]
                .instantiate_parameters_in_scope(&binders[..index], &arguments[..index], scope)
                .map_err(|e| e.to_string())?;
            let actual = match argument {
                Term::Symbol(_) => Term::Primitive(Primitive::Symbol),
                Term::String(_) => Term::Primitive(Primitive::Str),
                Term::Char(_) => Term::Primitive(Primitive::Char),
                Term::Bool(_) => Term::Primitive(Primitive::Bool),
                Term::Integer(_) => Term::Primitive(Primitive::Integer),
                Term::Float(_) => Term::Primitive(Primitive::Float),
                Term::Float32(_) => Term::Primitive(Primitive::F32),
                Term::Float64(_) => Term::Primitive(Primitive::F64),
                Term::BitVec(value) => Term::Primitive(Primitive::BitVec(value.width())),
                Term::Unit => Term::Primitive(Primitive::Void),
                Term::Rigid {
                    declaration,
                    sort: Sort::ComptimeValue,
                } => {
                    let mut found = None;
                    for owner in declarations {
                        let binders = owner.parameter_binders(fuel)?;
                        for (slot, binder) in binders.iter().enumerate() {
                            if binder.declaration == *declaration {
                                if found.is_some() || binder.sort != Sort::ComptimeValue {
                                    return Err(
                                        "ambiguous or mismatched trait value declaration".into()
                                    );
                                }
                                crate::type_computations::charge_term(
                                    &owner.parameter_types[slot],
                                    fuel,
                                )
                                .map_err(|e| e.to_string())?;
                                found = Some(owner.parameter_types[slot].clone());
                            }
                        }
                    }
                    found.ok_or("trait value argument requires declared domain evidence")?
                }
                _ => return Err("trait value argument requires declared domain evidence".into()),
            };
            if domain != actual {
                return Err("trait value argument domain mismatch".into());
            }
        }
        Ok(())
    }
}

impl crate::program::Package {
    /// Sort evidence inherited from trait declarations, keyed by stable binder
    /// identity. Never infer a binder's sort from a supplied argument.
    pub fn trait_parameter_sorts(
        &self,
        fuel: &mut usize,
    ) -> Result<std::collections::BTreeMap<crate::DeclarationId, Sort>, String> {
        let mut sorts = std::collections::BTreeMap::new();
        for declaration in &self.traits {
            for binder in declaration.binders_with_self(fuel)? {
                if sorts.insert(binder.declaration, binder.sort).is_some() {
                    return Err("duplicate trait parameter identity across declarations".into());
                }
            }
        }
        Ok(sorts)
    }
}
