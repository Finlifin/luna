//! Recheck dependent storage domains after invocation inputs are substituted.
use super::*;

impl Interpreter<'_> {
    pub(super) fn check_comptime_domain(
        &mut self,
        ty: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        if !self.comptime_only {
            return Ok(());
        }
        let package = self
            .declaration_world
            .ok_or_else(|| self.failure(ErrorKind::PhaseMismatch, Some(source)))?;
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let checked = nir::phases::validate_comptime_storage_with_nominals(
            ty,
            package,
            &[],
            &|ty, fuel| {
                let Term::Nominal {
                    declaration,
                    arguments,
                } = ty
                else {
                    return None;
                };
                *fuel = fuel.checked_sub(1)?;
                if let Some(definition) = self.structs.get(declaration) {
                    Some(
                        definition
                            .instantiate_fields_in_scope(arguments, &[])
                            .ok()?
                            .into_iter()
                            .map(|(_, field)| field)
                            .collect(),
                    )
                } else if let Some(definition) = self.enums.get(declaration) {
                    Some(
                        definition
                            .instantiate_variants_in_scope(arguments, &[])
                            .ok()?
                            .into_iter()
                            .flat_map(|variant| variant.fields)
                            .collect(),
                    )
                } else if let Some(definition) = self.newtypes.get(declaration) {
                    Some(vec![
                        definition
                            .representation_budgeted(arguments, &[], fuel)
                            .ok()?,
                    ])
                } else {
                    None
                }
            },
            &mut fuel,
        );
        self.budget.steps -= (before - fuel) as u64;
        checked.map_err(|_| {
            self.failure(
                if fuel == 0 {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::PhaseMismatch
                },
                Some(source),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn package() -> Package {
        let binding = LocalId { owner: 1, index: 0 };
        Package {
            functions: vec![nir::program::Function {
                declaration: DeclarationId {
                    package: 81,
                    local: 1,
                },
                name: "accept".into(),
                phase: nir::program::FunctionPhase::Comptime,
                inline: false,
                pure: false,
                is_unsafe: false,
                type_parameters: vec![],
                requirements: vec![],
                capabilities: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                entry_snapshots: vec![],
                parameters: vec![binding],
                defaults: vec![],
                signature: Term::Function {
                    parameters: vec![nir::Parameter {
                        sort: nir::Sort::Runtime,
                        ty: Term::Primitive(Primitive::Object),
                    }],
                    result: Box::new(Term::Primitive(Primitive::Bool)),
                },
                body: Expr {
                    source: SourceRange { start: 1, end: 2 },
                    ty: Term::Primitive(Primitive::Bool),
                    kind: K::Literal(nir::program::Literal::Bool(true)),
                },
            }],
            ..Default::default()
        }
    }
    #[test]
    fn object_payloads_reject_host_pointers_and_recover() {
        let package = package();
        let storage = Storage::new(Value::Bool(true));
        let object = Value::Object(Object::from_entries(vec![ObjectEntry::Value(
            ObjectValue {
                ty: Term::Pointer(Box::new(Term::Primitive(Primitive::Bool))),
                value: Value::Pointer(storage.pointer(Term::Primitive(Primitive::Bool), vec![])),
            },
        )]));
        for tree in [false, true] {
            let mut vm = if tree {
                Interpreter::for_comptime_tree(&package, Budget::default())
            } else {
                Interpreter::for_comptime(&package, Budget::default())
            }
            .unwrap();
            let error = vm
                .invoke(package.functions[0].declaration, vec![object.clone()])
                .unwrap_err();
            assert_eq!(error.kind, ErrorKind::PhaseMismatch);
            assert_eq!(
                vm.invoke(
                    package.functions[0].declaration,
                    vec![Value::Object(Object::from_entries(vec![]))]
                )
                .unwrap(),
                Value::Bool(true)
            );
        }
    }
    #[test]
    fn shared_closure_cells_are_rechecked_after_mutation() {
        let package = package();
        let cell = Storage::new(Value::Bool(true));
        let binding = LocalId { owner: 2, index: 0 };
        let ty = Term::Closure {
            environment: vec![Term::Primitive(Primitive::Bool)],
            signature: Box::new(Term::ComptimeFunction(Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(Primitive::Bool)),
            }))),
        };
        let closure = Value::Closure(Arc::new(Closure {
            definition_source: None,
            origin: None,
            instance_arguments: None,
            parameters: vec![],
            environment: std::iter::once((binding, Value::Storage(cell.clone()))).collect(),
            capture_order: vec![binding],
            body: Arc::new(package.functions[0].body.clone()),
            ty: ty.clone(),
        }));
        let object = Value::Object(Object::from_entries(vec![ObjectEntry::Value(
            ObjectValue { ty, value: closure },
        )]));
        let target = Storage::new(Value::Bool(true));
        for tree in [false, true] {
            *cell.0.lock().unwrap() = Value::Bool(true);
            let mut vm = if tree {
                Interpreter::for_comptime_tree(&package, Budget::default())
            } else {
                Interpreter::for_comptime(&package, Budget::default())
            }
            .unwrap();
            assert_eq!(
                vm.invoke(package.functions[0].declaration, vec![object.clone()])
                    .unwrap(),
                Value::Bool(true)
            );
            *cell.0.lock().unwrap() =
                Value::Pointer(target.pointer(Term::Primitive(Primitive::Bool), vec![]));
            assert_eq!(
                vm.invoke(package.functions[0].declaration, vec![object.clone()])
                    .unwrap_err()
                    .kind,
                ErrorKind::PhaseMismatch
            );
            *cell.0.lock().unwrap() = Value::Bool(false);
            assert_eq!(
                vm.invoke(package.functions[0].declaration, vec![object.clone()])
                    .unwrap(),
                Value::Bool(true)
            );
        }
    }

    #[test]
    fn capture_graph_cycles_do_not_skip_later_payloads_or_block() {
        let package = package();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let cell = Storage::new(Value::Unit);
        let root = Value::Storage(cell.clone());
        *cell.0.lock().unwrap() = Value::Tuple(vec![root.clone(), Value::Bool(true)]);
        let source = SourceRange { start: 1, end: 2 };
        vm.check_comptime_graph(&root, source, 0).unwrap();
        let target = Storage::new(Value::Bool(true));
        *cell.0.lock().unwrap() = Value::Tuple(vec![
            root.clone(),
            Value::Pointer(target.pointer(Term::Primitive(Primitive::Bool), vec![])),
        ]);
        let rejected = vm.check_comptime_graph(&root, source, 0);
        // Break the deliberate strong cycle before asserting the result.
        *cell.0.lock().unwrap() = Value::Unit;
        assert_eq!(rejected.unwrap_err().kind, ErrorKind::PhaseMismatch);
        vm.check_comptime_graph(&root, source, 0).unwrap();
    }
}
