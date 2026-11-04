//! Invocation-local borrowed indexes for completed type references.
use crate::{NominalDefinition, Package, Term};
use nir::{DeclarationId, value_admission::Error};
mod applications;
type MethodKey<'a> = (
    DeclarationId,
    &'a [Term],
    &'a Term,
    DeclarationId,
    &'a [Term],
);
use std::collections::{HashMap, HashSet};

pub(super) struct TypeLinks<'a> {
    methods: HashSet<(
        DeclarationId,
        &'a [Term],
        &'a Term,
        DeclarationId,
        &'a [Term],
    )>,
    callables: HashMap<(DeclarationId, &'a [Term]), &'a Term>,
    signatures: HashMap<MethodKey<'a>, &'a Term>,
    target: Option<nir::target::TargetData>,
    nominals: HashMap<&'a Term, &'a NominalDefinition>,
    heads: HashMap<DeclarationId, &'a nir::normal_forms::Head>,
}

fn tick(fuel: &mut usize) -> Result<(), Error> {
    *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
    Ok(())
}
fn charge(ty: &Term, fuel: &mut usize) -> Result<(), Error> {
    nir::type_computations::charge_term(ty, fuel).map_err(Into::into)
}
impl<'a> TypeLinks<'a> {
    pub(super) fn new(package: &'a Package, fuel: &mut usize) -> Result<Self, String> {
        let mut result = Self {
            methods: HashSet::new(),
            callables: HashMap::new(),
            signatures: HashMap::new(),
            target: package.target,
            nominals: HashMap::new(),
            heads: HashMap::new(),
        };
        for identity in &package.callable_identities {
            tick(fuel)?;
            for argument in identity.key.arguments.iter() {
                charge(argument, fuel)?;
            }
            result.callables.insert(
                (identity.key.declaration, &identity.key.arguments),
                identity.closed_signature()?,
            );
        }
        for dispatch in &package.dispatch {
            tick(fuel)?;
            charge(&dispatch.subject, fuel)?;
            for argument in &dispatch.interface_arguments {
                charge(argument, fuel)?;
            }
            for method in &dispatch.methods {
                tick(fuel)?;
                for argument in &method.method_arguments {
                    charge(argument, fuel)?;
                }
                result.methods.insert((
                    dispatch.interface,
                    &dispatch.interface_arguments,
                    &dispatch.subject,
                    method.member,
                    &method.method_arguments,
                ));
            }
        }
        for contract in &package.interfaces {
            tick(fuel)?;
            charge(&contract.subject, fuel)?;
            for argument in &contract.interface.arguments {
                charge(argument, fuel)?;
            }
            for method in &contract.methods {
                tick(fuel)?;
                for argument in &method.type_arguments {
                    charge(argument, fuel)?;
                }
                if let Ok(signature) = method.signature.ordinary() {
                    result.signatures.insert(
                        (
                            contract.interface.declaration,
                            &contract.interface.arguments,
                            &contract.subject,
                            method.declaration,
                            &method.type_arguments,
                        ),
                        signature,
                    );
                }
            }
        }
        for nominal in &package.nominals {
            tick(fuel)?;
            charge(&nominal.ty, fuel)?;
            result.nominals.insert(&nominal.ty, &nominal.definition);
        }
        for head in &package.normal_forms {
            tick(fuel)?;
            result.heads.insert(head.declaration, head);
        }
        Ok(result)
    }

    pub(super) fn nominal(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<&'a NominalDefinition>, Error> {
        tick(fuel)?;
        charge(ty, fuel)?;
        Ok(self.nominals.get(ty).copied())
    }

    pub(super) fn head(&self, ty: &Term) -> Option<&'a nir::normal_forms::Head> {
        let Term::NormalForm { head, .. } = ty else {
            return None;
        };
        self.heads.get(head).copied()
    }

    pub(super) fn list_element<'t>(
        &self,
        ty: &'t Term,
        fuel: &mut usize,
    ) -> Result<Option<&'t Term>, String> {
        tick(fuel)?;
        Ok(self.head(ty).and_then(|head| head.list_element(ty)))
    }

    pub(super) fn newtype_conversion(
        &self,
        source: &Term,
        target: &Term,
        fuel: &mut usize,
    ) -> Result<Option<nir::newtypes::Conversion>, nir::newtypes::ConversionError> {
        for endpoint in [source, target] {
            if let Some(NominalDefinition::Newtype(definition)) = self
                .nominal(endpoint, fuel)
                .map_err(|_| nir::newtypes::ConversionError::Budget)?
            {
                if let Some(plan) =
                    nir::newtypes::instance_conversion(definition, endpoint, source, target, fuel)?
                {
                    return Ok(Some(plan));
                }
            }
        }
        Ok(None)
    }

    pub(super) fn validate(&self, ty: &Term, fuel: &mut usize) -> Result<(), String> {
        let mut pending = vec![ty];
        while let Some(ty) = pending.pop() {
            tick(fuel)?;
            match ty {
                Term::MethodValue(method) => {
                    for argument in method.terms() {
                        charge(argument, fuel)?;
                    }
                    if !self.methods.contains(&(
                        method.interface.declaration,
                        method.interface.arguments.as_slice(),
                        &method.subject,
                        method.member,
                        method.method_arguments.as_slice(),
                    )) {
                        return Err("IIR frozen method has no dispatch evidence".into());
                    }
                }
                Term::FunctionValue {
                    declaration,
                    arguments,
                } => {
                    for argument in arguments {
                        charge(argument, fuel)?;
                    }
                    if !self
                        .callables
                        .contains_key(&(*declaration, arguments.as_slice()))
                    {
                        return Err("IIR frozen callable has no signature evidence".into());
                    }
                }
                Term::Nominal { .. } => {
                    charge(ty, fuel)?;
                    if !self.nominals.contains_key(ty) {
                        return Err("IIR nominal reference is dangling".into());
                    }
                }
                Term::NormalForm {
                    head, arguments, ..
                } => {
                    let definition = self
                        .heads
                        .get(head)
                        .ok_or("IIR normal-form reference is dangling")?;
                    self.validate_application(definition, arguments, fuel)?;
                    if arguments.len() != definition.parameters.len() {
                        return Err(format!(
                            "IIR normal-form head expects {} inputs, found {}",
                            definition.parameters.len(),
                            arguments.len(),
                        ));
                    }
                    for (index, (parameter, argument)) in
                        definition.parameters.iter().zip(arguments).enumerate()
                    {
                        tick(fuel)?;
                        // A declared Type domain has an unconditional sort
                        // contract, including an ordinary value parameter whose
                        // domain is Type. Dependent domains still require the
                        // producer's semantic formation evidence.
                        if (parameter.kind == nir::normal_forms::Kind::Type
                            || parameter.ty == Term::Primitive(nir::Primitive::Type))
                            && argument.sort() != nir::Sort::Type
                        {
                            return Err(format!("IIR normal-form input {index} requires a Type"));
                        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use nir::Primitive;
    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 95, local }
    }
    fn nominal(local: u32, argument: Term) -> Term {
        Term::Nominal {
            declaration: id(local),
            arguments: vec![argument],
        }
    }
    fn instance(local: u32, scalar: Primitive) -> crate::NominalInstance {
        let representation = Term::Primitive(scalar);
        crate::NominalInstance {
            ty: nominal(local, representation.clone()),
            definition: NominalDefinition::Newtype(nir::newtypes::Newtype {
                declaration: id(local),
                name: "Wrapper".into(),
                type_parameters: vec![],
                parameter_types: vec![],
                capture_count: 0,
                requirements: vec![],
                capabilities: vec![],
                representation,
            }),
        }
    }
    fn value_head(domain: Term) -> nir::normal_forms::Head {
        nir::normal_forms::Head {
            declaration: id(90),
            collector: None,
            role: nir::normal_forms::Role::Opaque,
            requirements: vec![],
            parameters: vec![nir::normal_forms::Parameter {
                declaration: id(91),
                binding: nir::program::LocalId {
                    owner: 90,
                    index: 0,
                },
                kind: nir::normal_forms::Kind::Value,
                ty: domain,
            }],
        }
    }

    fn check_query_budget_boundaries(links: &TypeLinks<'_>, domain: Term, value: Term) {
        let head = value_head(domain);
        let mut completed = false;
        for allowance in 0..20_000 {
            let mut fuel = allowance;
            match links.validate_application(&head, std::slice::from_ref(&value), &mut fuel) {
                Ok(()) => {
                    completed = true;
                    break;
                }
                Err(error) => assert!(
                    error.contains("budget"),
                    "allowance {allowance}, remaining {fuel}: {error}"
                ),
            }
        }
        assert!(
            completed,
            "valid retained evidence must eventually be admitted"
        );
        links
            .validate_application(&head, &[value], &mut 20_000)
            .unwrap();
    }

    #[test]
    fn nominal_application_queries_preserve_budget_failures() {
        let mut retained = instance(1, Primitive::Bool);
        retained.ty = nominal(1, Term::String("index".repeat(40).into()));
        let ty = retained.ty.clone();
        let graph = Package {
            nominals: vec![retained],
            ..Default::default()
        };
        let links = TypeLinks::new(&graph, &mut 20_000).unwrap();
        let value = Term::NominalValue {
            ty: Box::new(ty.clone()),
            variant: None,
            fields: vec![Term::Bool(true)],
        };
        check_query_budget_boundaries(&links, ty.clone(), value.clone());
        let head = value_head(ty.clone());
        for (variant, fields) in [
            (Some(0), vec![Term::Bool(true)]),
            (None, vec![Term::Char('x')]),
        ] {
            let invalid = Term::NominalValue {
                ty: Box::new(ty.clone()),
                variant,
                fields,
            };
            let failure = links
                .validate_application(&head, &[invalid], &mut 20_000)
                .unwrap_err();
            assert!(!failure.contains("budget"), "{failure}");
        }
        let empty = Package::default();
        let absent = TypeLinks::new(&empty, &mut 20_000).unwrap();
        let failure = absent
            .validate_application(&head, &[value], &mut 20_000)
            .unwrap_err();
        assert!(failure.contains("incompatible value"), "{failure}");
        assert!(!failure.contains("budget"), "{failure}");
    }

    #[test]
    fn callable_application_queries_preserve_budget_failures() {
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        };
        let inputs = vec![Term::String("input".repeat(40).into())];
        let mut pool = nir::TypeInterner::default();
        let identity = crate::CallableIdentity::intern(
            crate::InstanceKey {
                declaration: id(7),
                arguments: inputs.clone().into(),
            },
            signature.clone(),
            &mut pool,
            &mut 20_000,
        )
        .unwrap();
        let graph = Package {
            callable_identities: vec![identity],
            ..Default::default()
        };
        let links = TypeLinks::new(&graph, &mut 20_000).unwrap();
        let value = Term::FunctionValue {
            declaration: id(7),
            arguments: inputs,
        };
        check_query_budget_boundaries(&links, signature.clone(), value.clone());
        let head = value_head(signature);
        let empty = Package::default();
        let absent = TypeLinks::new(&empty, &mut 20_000).unwrap();
        let failure = absent
            .validate_application(&head, &[value], &mut 20_000)
            .unwrap_err();
        assert!(failure.contains("incompatible value"), "{failure}");
        assert!(!failure.contains("budget"), "{failure}");
    }

    #[test]
    fn method_application_queries_keep_instance_keys_and_budget_failures() {
        let signature = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        };
        let method = nir::TraitMethodIdentity {
            interface: nir::program::InterfaceInstance {
                declaration: id(5),
                arguments: vec![Term::String("trait".repeat(40).into())],
            },
            subject: Term::Primitive(Primitive::Bool),
            member: id(6),
            method_arguments: vec![Term::String("method".repeat(40).into())],
        };
        let graph = Package {
            interfaces: vec![crate::InterfaceContract {
                interface: method.interface.clone(),
                subject: method.subject.clone(),
                methods: vec![nir::program::MethodContract {
                    phase: Default::default(),
                    is_quote: false,
                    pure: false,
                    is_unsafe: false,
                    declaration: method.member,
                    name: "predicate".into(),
                    signature: signature.clone().into(),
                    parameter_bindings: vec![],
                    type_arguments: method.method_arguments.clone(),
                    type_parameters: vec![],
                    capabilities: vec![],
                    requirements: vec![],
                    by_value_receiver: false,
                    by_reference_receiver: false,
                    default: None,
                    program: Default::default(),
                }],
            }],
            ..Default::default()
        };
        let links = TypeLinks::new(&graph, &mut 20_000).unwrap();
        let value = Term::MethodValue(Box::new(method.clone()));
        check_query_budget_boundaries(&links, signature.clone(), value.clone());
        let head = value_head(signature);
        for changed_part in 0..4 {
            let mut absent = method.clone();
            match changed_part {
                0 => absent.interface.arguments[0] = Term::String("other".repeat(40).into()),
                1 => absent.subject = Term::Primitive(Primitive::Char),
                2 => absent.member = id(99),
                _ => absent.method_arguments[0] = Term::String("absent".repeat(40).into()),
            }
            let failure = links
                .validate_application(&head, &[Term::MethodValue(Box::new(absent))], &mut 20_000)
                .unwrap_err();
            assert!(!failure.contains("budget"), "{failure}");
        }
        links
            .validate_application(&head, &[value], &mut 20_000)
            .unwrap();
    }

    #[test]
    fn concrete_lookup_and_conversion_keep_complete_keys_and_constant_query_cost() {
        let query_cost = |padding: u32| {
            let mut graph = Package::default();
            graph
                .nominals
                .extend((100..100 + padding).map(|local| instance(local, Primitive::Bool)));
            graph
                .nominals
                .extend([instance(1, Primitive::Bool), instance(1, Primitive::Char)]);
            graph.validate(&mut 100_000).unwrap();
            let links = TypeLinks::new(&graph, &mut 100_000).unwrap();
            let mut fuel = 100_000;
            for scalar in [Primitive::Bool, Primitive::Char] {
                let repr = Term::Primitive(scalar);
                let ty = nominal(1, repr.clone());
                let Some(NominalDefinition::Newtype(found)) =
                    links.nominal(&ty, &mut fuel).unwrap()
                else {
                    panic!()
                };
                assert_eq!(found.representation, repr);
                assert!(
                    links
                        .newtype_conversion(&ty, &repr, &mut fuel)
                        .unwrap()
                        .is_some()
                );
                assert!(
                    links
                        .newtype_conversion(&repr, &ty, &mut fuel)
                        .unwrap()
                        .is_some()
                );
            }
            let boolean = Term::Primitive(Primitive::Bool);
            let character = Term::Primitive(Primitive::Char);
            assert!(
                links
                    .newtype_conversion(&nominal(1, boolean.clone()), &character, &mut fuel)
                    .unwrap()
                    .is_none()
            );
            assert!(
                links
                    .nominal(
                        &Term::Nominal {
                            declaration: id(1),
                            arguments: vec![]
                        },
                        &mut fuel
                    )
                    .unwrap()
                    .is_none()
            );
            assert!(links.nominal(&nominal(1, boolean.clone()), &mut 0).is_err());
            assert!(
                links
                    .newtype_conversion(&nominal(1, boolean.clone()), &boolean, &mut 0)
                    .is_err()
            );
            assert!(
                links
                    .newtype_conversion(&nominal(1, boolean.clone()), &boolean, &mut 100_000)
                    .unwrap()
                    .is_some()
            );
            100_000 - fuel
        };
        assert_eq!(query_cost(0), query_cost(128));
    }
}
