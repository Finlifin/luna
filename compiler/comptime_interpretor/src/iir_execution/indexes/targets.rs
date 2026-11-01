//! Shared immutable keys for executable and body-free callable identities.
use super::*;

pub(super) fn instance_positions<'a>(
    entries: impl Iterator<Item = (usize, &'a iir::InstanceKey)>,
    fuel: &mut usize,
) -> Result<HashMap<DeclarationId, HashMap<Arc<[Term]>, usize>>, ()> {
    let mut positions: HashMap<_, HashMap<_, _>> = HashMap::new();
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
    pub(crate) fn iir_template_type(
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

    pub(crate) fn iir_function_definition(
        &self,
        declaration: DeclarationId,
        arguments: &[Term],
    ) -> Option<&'a nir::program::Function> {
        let index = self
            .graph_indexes
            .as_ref()?
            .functions
            .get(&declaration)?
            .get(arguments)?;
        Some(&self.graph?.functions[*index].function)
    }

    pub(crate) fn iir_template_method_type(
        &self,
        identity: &TraitMethodIdentity,
    ) -> Option<&'a Term> {
        let (outer, inner) = self.graph_indexes.as_ref()?.dispatch.get(
            identity.interface.declaration,
            &identity.interface.arguments,
            &identity.subject,
            identity.member,
            &identity.method_arguments,
        )?;
        let method = &self.graph?.dispatch[outer].methods[inner];
        self.iir_template_type(method.body, &method.arguments)
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
        let vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
        for local in 0..128 {
            for value in [true, false] {
                let key = key(local, value);
                let handle = Value::FunctionInstance {
                    declaration: key.declaration,
                    arguments: key.arguments.to_vec(),
                };
                assert!(vm.has_type(&handle, &signature));
                assert!(!vm.has_type(&handle, &Term::Primitive(Primitive::Bool)));
                assert!(
                    vm.iir_function_definition(key.declaration, &key.arguments)
                        .is_none()
                );
            }
        }
        assert!(
            vm.iir_template_type(key(128, true).declaration, &[Term::Bool(true)])
                .is_none()
        );
        assert!(
            vm.iir_template_type(key(0, true).declaration, &[])
                .is_none()
        );
        let empty = iir::Package::default();
        let other = Interpreter::from_iir_for_comptime(&empty, Budget::default()).unwrap();
        assert!(!other.has_type(
            &Value::FunctionInstance {
                declaration: key(0, true).declaration,
                arguments: key(0, true).arguments.to_vec()
            },
            &signature
        ));
    }
}
