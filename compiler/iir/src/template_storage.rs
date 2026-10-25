//! Finite storage for staged callable identities. The logical callable type is
//! retained separately from the runtime tag representation; this is not a type
//! equality between a callable and an integer.
use crate::InstanceKey;
use nir::{Sort, Term};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateStorage {
    logical_type: Term,
    targets: Vec<InstanceKey>,
}

impl TemplateStorage {
    /// Resolve concrete target identities against actual runtime declarations.
    /// This validates declaration inputs and signatures, not invocation-time
    /// preconditions or implementation selection obligations.
    pub fn from_package(
        logical_type: Term,
        targets: Vec<InstanceKey>,
        package: &nir::program::Package,
        fuel: &mut usize,
    ) -> Result<Self, String> {
        let domains = package.declaration_parameter_domains(fuel)?;
        let mut selected = Vec::new();
        for key in targets {
            *fuel = fuel
                .checked_sub(package.functions.len().saturating_add(1))
                .ok_or("template declaration lookup budget exhausted")?;
            let mut declarations = package
                .functions
                .iter()
                .filter(|function| function.declaration == key.declaration);
            let (type_parameters, declared_signature) =
                if let Some(declaration) = declarations.next() {
                    if declarations.next().is_some() {
                        return Err("template storage target declaration is ambiguous".into());
                    }
                    if declaration.phase != nir::program::FunctionPhase::Runtime {
                        return Err("template storage target is not a runtime declaration".into());
                    }
                    (&declaration.type_parameters, &declaration.signature)
                } else {
                    // Open runtime declarations expose identity before any staged
                    // invocation has produced a body. Tags store that identity;
                    // execution still requires a separately prepared instance.
                    *fuel = fuel
                        .checked_sub(package.runtime_function_identities.len())
                        .ok_or("template identity lookup budget exhausted")?;
                    let mut identities = package
                        .runtime_function_identities
                        .iter()
                        .filter(|identity| identity.declaration == key.declaration);
                    let identity = identities
                        .next()
                        .ok_or("template storage target declaration is missing")?;
                    if identities.next().is_some() {
                        return Err("template storage target identity is ambiguous".into());
                    }
                    (&identity.type_parameters, &identity.signature)
                };
            let mut input_domains = Vec::new();
            for id in type_parameters {
                let default_domain = Term::Primitive(nir::Primitive::Type);
                let domain = domains.get(id).unwrap_or(&default_domain);
                nir::type_computations::charge_term(domain, fuel).map_err(|e| e.to_string())?;
                input_domains.push(domain.clone());
            }
            nir::function_parameters::validate_arguments(
                package,
                type_parameters,
                &input_domains,
                &key.arguments,
                fuel,
            )?;
            let binders = nir::declaration_parameters(type_parameters, &input_domains)
                .map_err(|e| e.to_string())?;
            let before = *fuel;
            nir::type_computations::charge_term(declared_signature, fuel)
                .map_err(|e| e.to_string())?;
            let signature_work = before - *fuel;
            let before = *fuel;
            charge_key(&key, fuel)?;
            let input_work = before - *fuel;
            *fuel = fuel
                .checked_sub(signature_work.saturating_mul(input_work))
                .ok_or("template signature substitution budget exhausted")?;
            let signature = declared_signature
                .instantiate_parameters(&binders, &key.arguments)
                .map_err(|e| e.to_string())?;
            selected.push((key, signature));
        }
        Self::new(logical_type, selected, fuel)
    }

    /// Targets must already have selected declaration inputs and matching
    /// signatures. The table records identities, never executes their bodies.
    /// Input order fixes tag numbering and is retained by Display and lookup.
    /// This checks representation consistency; the materializer must establish
    /// each target's declaration/signature evidence from its source package.
    fn new(
        logical_type: Term,
        targets: Vec<(InstanceKey, Term)>,
        fuel: &mut usize,
    ) -> Result<Self, String> {
        let Term::Function { parameters, .. } = logical_type.callable_signature() else {
            return Err("template storage requires a callable type".into());
        };
        if logical_type.is_comptime_callable()
            || matches!(
                logical_type,
                Term::Closure { .. } | Term::ContextualClosure { .. }
            )
            || !parameters
                .iter()
                .any(|parameter| parameter.sort != Sort::Runtime)
        {
            return Err(
                "template storage requires an environment-free runtime callable with staged inputs"
                    .into(),
            );
        }
        if targets.is_empty() {
            return Err("template storage requires at least one selected target".into());
        }
        crate::completed_type(&logical_type, &Default::default(), fuel)?;
        let mut selected = Vec::new();
        for (key, signature) in targets {
            nir::type_computations::charge_term(&signature, fuel).map_err(|e| e.to_string())?;
            if signature != logical_type {
                return Err(
                    "template storage target signature differs from its logical type".into(),
                );
            }
            *fuel = fuel
                .checked_sub(key.arguments.len().saturating_add(1))
                .ok_or("template storage budget exhausted")?;
            for argument in &key.arguments {
                crate::completed_type(argument, &Default::default(), fuel)?;
            }
            for previous in &selected {
                charge_key(previous, fuel)?;
                charge_key(&key, fuel)?;
                if previous == &key {
                    return Err("template storage contains a duplicate target".into());
                }
            }
            selected.push(key);
        }
        Ok(Self {
            logical_type,
            targets: selected,
        })
    }

    pub fn logical_type(&self) -> &Term {
        &self.logical_type
    }
    pub fn targets(&self) -> &[InstanceKey] {
        &self.targets
    }

    /// This encoding is valid only for this table; a tag from another table
    /// must not be copied without translating its selected target identity.
    pub fn encode(&self, target: &InstanceKey, fuel: &mut usize) -> Result<u64, String> {
        charge_key(target, fuel)?;
        for (index, candidate) in self.targets.iter().enumerate() {
            charge_key(candidate, fuel)?;
            if candidate == target {
                return u64::try_from(index).map_err(|_| "template storage tag overflow".into());
            }
        }
        Err("template target is not in this storage domain".into())
    }

    pub fn decode(&self, tag: u64) -> Result<&InstanceKey, String> {
        usize::try_from(tag)
            .ok()
            .and_then(|index| self.targets.get(index))
            .ok_or_else(|| "template storage tag is outside its domain".into())
    }
}

fn charge_key(key: &InstanceKey, fuel: &mut usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(key.arguments.len().saturating_add(1))
        .ok_or("template storage budget exhausted")?;
    for argument in &key.arguments {
        nir::type_computations::charge_term(argument, fuel).map_err(|e| e.to_string())?;
    }
    Ok(())
}

impl fmt::Display for TemplateStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(template-storage (logical-type {}) (representation u64) (targets",
            self.logical_type
        )?;
        for (tag, target) in self.targets.iter().enumerate() {
            write!(f, " (target (tag {tag}) {target})")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::{DeclarationId, Parameter, Primitive};

    fn signature() -> Term {
        Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            result: Box::new(Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::Type,
            }),
        }
    }
    fn key(local: u32) -> InstanceKey {
        InstanceKey {
            declaration: DeclarationId { package: 3, local },
            arguments: vec![Term::Primitive(Primitive::Bool)],
        }
    }
    #[test]
    fn tags_preserve_target_identity_and_require_translation_between_domains() {
        let ty = signature();
        let table = TemplateStorage::new(
            ty.clone(),
            vec![(key(1), ty.clone()), (key(2), ty.clone())],
            &mut 10_000,
        )
        .unwrap();
        let other = TemplateStorage::new(
            ty.clone(),
            vec![(key(2), ty.clone()), (key(1), ty.clone())],
            &mut 10_000,
        )
        .unwrap();
        let saved = table.encode(&key(1), &mut 100).unwrap();
        let current = table.encode(&key(2), &mut 100).unwrap();
        assert_eq!(table.decode(saved).unwrap(), &key(1));
        assert_eq!(table.decode(current).unwrap(), &key(2));
        assert_eq!(
            other
                .encode(table.decode(saved).unwrap(), &mut 100)
                .unwrap(),
            1
        );
        assert!(table.decode(2).is_err());
        assert!(table.decode(u64::MAX).is_err());
        assert!(table.encode(&key(3), &mut 100).is_err());
        assert_eq!(table.logical_type(), &ty);
        assert_eq!(table.targets(), &[key(1), key(2)]);
        let dump = table.to_string();
        assert!(
            dump.starts_with("(template-storage (logical-type (function "),
            "{dump}"
        );
        assert!(dump.contains("(representation u64)"));
        assert_eq!(dump.matches("(target (tag ").count(), 2);
        assert_eq!(
            dump.bytes().filter(|b| *b == b'(').count(),
            dump.bytes().filter(|b| *b == b')').count()
        );
    }
    #[test]
    fn malformed_domains_and_exhausted_budgets_do_not_publish_tables() {
        let ty = signature();
        assert!(TemplateStorage::new(ty.clone(), vec![], &mut 100).is_err());
        assert!(
            TemplateStorage::new(
                ty.clone(),
                vec![(key(1), ty.clone()), (key(1), ty.clone())],
                &mut 10_000
            )
            .is_err()
        );
        assert!(
            TemplateStorage::new(
                ty.clone(),
                vec![(key(1), Term::Primitive(Primitive::Bool))],
                &mut 10_000
            )
            .is_err()
        );
        for invalid in [
            Term::Primitive(Primitive::Bool),
            ty.clone().with_comptime_phase(true),
            Term::Closure {
                environment: vec![Term::Primitive(Primitive::Bool)],
                signature: Box::new(ty.clone()),
            },
            Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(Primitive::Bool)),
            },
        ] {
            assert!(
                TemplateStorage::new(invalid.clone(), vec![(key(1), invalid)], &mut 10_000)
                    .is_err()
            );
        }
        assert!(TemplateStorage::new(ty.clone(), vec![(key(1), ty.clone())], &mut 0).is_err());
        let table = TemplateStorage::new(ty.clone(), vec![(key(1), ty)], &mut 10_000).unwrap();
        assert!(table.encode(&key(1), &mut 0).is_err());
        assert_eq!(table.encode(&key(1), &mut 100).unwrap(), 0);
    }
}
