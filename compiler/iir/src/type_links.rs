//! Invocation-local borrowed indexes for completed type references.
use crate::{NominalDefinition, Package, Term};
use nir::DeclarationId;
use std::collections::{HashMap, HashSet};

pub(super) struct TypeLinks<'a> {
    methods: HashSet<(
        DeclarationId,
        &'a [Term],
        &'a Term,
        DeclarationId,
        &'a [Term],
    )>,
    callables: HashSet<(DeclarationId, &'a [Term])>,
    nominals: HashMap<&'a Term, &'a NominalDefinition>,
    heads: HashMap<DeclarationId, &'a nir::normal_forms::Head>,
}

fn tick(fuel: &mut usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(1)
        .ok_or("IIR type link budget exhausted")?;
    Ok(())
}
fn charge(ty: &Term, fuel: &mut usize) -> Result<(), String> {
    nir::type_computations::charge_term(ty, fuel)
        .map_err(|e| format!("IIR type link key budget: {e}"))
}
impl<'a> TypeLinks<'a> {
    pub(super) fn new(package: &'a Package, fuel: &mut usize) -> Result<Self, String> {
        let mut result = Self {
            methods: HashSet::new(),
            callables: HashSet::new(),
            nominals: HashMap::new(),
            heads: HashMap::new(),
        };
        for identity in &package.callable_identities {
            tick(fuel)?;
            for argument in identity.key.arguments.iter() {
                charge(argument, fuel)?;
            }
            result
                .callables
                .insert((identity.key.declaration, &identity.key.arguments));
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
    ) -> Result<Option<&'a NominalDefinition>, String> {
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
                        .contains(&(*declaration, arguments.as_slice()))
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
