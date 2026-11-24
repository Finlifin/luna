//! Shared immutable keys for executable and body-free callable identities.
use super::*;
use nir::value_admission::Error as AdmissionError;
use std::borrow::Cow;

pub(super) fn instance_positions<'a>(
    entries: impl Iterator<Item = (usize, &'a iir::InstanceKey)>,
    fuel: &mut usize,
) -> Result<BTreeMap<DeclarationId, DeterministicMap<Arc<[Term]>, usize>>, ()> {
    let mut positions: BTreeMap<_, DeterministicMap<_, _>> = BTreeMap::new();
    for (index, key) in entries {
        *fuel = fuel.checked_sub(1).ok_or(())?;
        for argument in key.arguments.iter() {
            nir::type_computations::charge_term(argument, fuel).map_err(|_| ())?;
        }
        // Graph validation rejects duplicate identities before this index is
        // installed. Share graph-owned arguments rather than cloning key trees.
        positions
            .entry(key.declaration)
            .or_default()
            .insert(key.arguments.clone(), index);
    }
    Ok(positions)
}

impl<'a> Interpreter<'a> {
    /// Read the declaration that authorizes transport in this concrete world.
    /// An executable instance can supply its interface without lending its body.
    pub(crate) fn iir_callable_signature(
        &self,
        graph: &'a iir::Package,
        declaration: DeclarationId,
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<Option<Cow<'a, Term>>, AdmissionError> {
        use nir::type_computations::charge_term;
        for argument in arguments {
            charge_term(argument, fuel)?;
        }
        let indexes = self
            .graph_indexes
            .as_ref()
            .expect("validated graph indexes");
        let signature = if let Some(position) = indexes
            .callables
            .get(&declaration)
            .and_then(|keys| keys.get(arguments))
        {
            graph.callable_identities[*position].closed_signature()?
        } else if let Some(signature) = self.iir_template_at_key(declaration, arguments) {
            signature
        } else if let Some(position) = self.iir_function_position(declaration, arguments) {
            let function = &graph.functions[position].function.interface;
            charge_term(&function.signature, fuel)?;
            return Ok(Some(function.callable_type_view()));
        } else {
            return Ok(None);
        };
        charge_term(signature, fuel)?;
        Ok(Some(Cow::Borrowed(signature)))
    }

    pub(crate) fn iir_template_type(
        &self,
        declaration: DeclarationId,
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<Option<&'a Term>, AdmissionError> {
        for argument in arguments {
            nir::type_computations::charge_term(argument, fuel)?;
        }
        Ok(self.iir_template_at_key(declaration, arguments))
    }

    fn iir_template_at_key(
        &self,
        declaration: DeclarationId,
        arguments: &[Term],
    ) -> Option<&'a Term> {
        let index = self
            .graph_indexes
            .as_ref()?
            .templates
            .get(&declaration)?
            .get(arguments)?;
        Some(self.graph?.templates[*index].logical_type())
    }

    pub(in crate::iir_execution) fn iir_function_position(
        &self,
        declaration: DeclarationId,
        arguments: &[Term],
    ) -> Option<usize> {
        self.graph_indexes
            .as_ref()?
            .functions
            .get(&declaration)?
            .get(arguments)
            .copied()
    }

    pub(crate) fn iir_template_method_type(
        &self,
        identity: &TraitMethodIdentity,
        fuel: &mut usize,
    ) -> Result<Option<&'a Term>, AdmissionError> {
        for term in identity.terms() {
            nir::type_computations::charge_term(term, fuel)?;
        }
        let Some(graph) = self.graph else {
            return Ok(None);
        };
        let indexes = self
            .graph_indexes
            .as_ref()
            .expect("validated graph indexes");
        let Some((outer, inner)) = indexes.dispatch.get(
            identity.interface.declaration,
            &identity.interface.arguments,
            &identity.subject,
            identity.member,
            &identity.method_arguments,
        ) else {
            return Ok(None);
        };
        let method = &graph.dispatch[outer].methods[inner];
        self.iir_template_type(method.body, &method.arguments, fuel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(local: u32, value: bool) -> iir::InstanceKey {
        iir::InstanceKey {
            declaration: DeclarationId { package: 94, local },
            arguments: vec![Term::Bool(value)].into(),
        }
    }

    #[test]
    fn target_keys_share_owners_and_preserve_value_specializations() {
        let keys = [key(1, true), key(1, false), key(2, true)];
        let mut fuel = 1000;
        let positions = instance_positions(keys.iter().enumerate(), &mut fuel).unwrap();
        for (expected, key) in keys.iter().enumerate() {
            let (owner, index) = positions[&key.declaration]
                .get_key_value(key.arguments.as_ref())
                .unwrap();
            assert_eq!(*index, expected);
            assert!(Arc::ptr_eq(owner, &key.arguments));
        }
        assert!(!positions[&keys[2].declaration].contains_key(key(2, false).arguments.as_ref()));
        let cost = 1000 - fuel;
        for mut fuel in 0..cost {
            assert!(instance_positions(keys.iter().enumerate(), &mut fuel).is_err());
        }
        assert_eq!(
            instance_positions(keys.iter().enumerate(), &mut 1000).unwrap(),
            positions
        );
    }

    #[test]
    fn template_membership_uses_current_graph_and_full_instance_identity() {
        fn key(local: u32, value: bool) -> iir::InstanceKey {
            iir::InstanceKey {
                declaration: DeclarationId { package: 94, local },
                arguments: vec![Term::Primitive(if value {
                    Primitive::Bool
                } else {
                    Primitive::Char
                })]
                .into(),
            }
        }
        let signature = Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        };
        let mut graph = iir::Package::default();
        for local in 0..128 {
            let source = nir::program::Package {
                runtime_function_identities: vec![nir::program::RuntimeFunctionIdentity {
                    source: nir::program::SourceRange { start: 0, end: 0 },
                    requirements: vec![],
                    capabilities: vec![],
                    declaration: key(local, true).declaration,
                    type_parameters: vec![DeclarationId { package: 95, local }],
                    signature: signature.clone(),
                }],
                ..Default::default()
            };
            graph.templates.push(
                iir::template_storage::TemplateStorage::from_package(
                    signature.clone(),
                    vec![key(local, true), key(local, false)],
                    &source,
                    &mut 100_000,
                )
                .unwrap(),
            );
        }
        let mut vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
        for local in 0..128 {
            for value in [true, false] {
                let key = key(local, value);
                let handle = Value::FunctionInstance {
                    declaration: key.declaration,
                    arguments: key.arguments.to_vec(),
                };
                assert!(vm.check_retained_value(&handle, &signature, None).is_ok());
                assert!(
                    vm.check_retained_value(&handle, &Term::Primitive(Primitive::Bool), None)
                        .is_err()
                );
                assert!(
                    vm.iir_function_position(key.declaration, &key.arguments)
                        .is_none()
                );
            }
        }
        assert!(
            vm.iir_template_type(key(128, true).declaration, &[Term::Bool(true)], &mut 100)
                .unwrap()
                .is_none()
        );
        assert!(
            vm.iir_template_type(key(0, true).declaration, &[], &mut 100)
                .unwrap()
                .is_none()
        );
        let mut runtime = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        let instance = key(0, true);
        let value = Value::FunctionInstance {
            declaration: instance.declaration,
            arguments: instance.arguments.to_vec(),
        };
        let source = Some(SourceRange { start: 3, end: 8 });
        let before = runtime.remaining_steps();
        runtime
            .check_value_type(&value, &signature, source)
            .unwrap();
        let cost = before - runtime.remaining_steps();
        assert!(
            cost > 1,
            "template lookup and signature comparison must both be charged"
        );
        for steps in 0..cost {
            runtime.budget.steps = steps;
            assert_eq!(
                runtime
                    .check_value_type(&value, &signature, source)
                    .unwrap_err(),
                Error {
                    kind: ErrorKind::StepsExhausted,
                    source
                }
            );
        }
        runtime.budget.steps = cost;
        runtime
            .check_value_type(&value, &signature, source)
            .unwrap();
        assert_eq!(runtime.remaining_steps(), 0);
        assert!(runtime.events.is_empty());
        let empty = iir::Package::default();
        let mut other = Interpreter::from_iir_for_comptime(&empty, Budget::default()).unwrap();
        assert!(
            other
                .check_retained_value(
                    &Value::FunctionInstance {
                        declaration: key(0, true).declaration,
                        arguments: key(0, true).arguments.to_vec()
                    },
                    &signature,
                    None
                )
                .is_err()
        );
    }
}
