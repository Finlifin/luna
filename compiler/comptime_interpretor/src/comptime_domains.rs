//! Recheck dependent storage domains after invocation inputs are substituted.
use super::*;

fn charge_layout_work(fuel: &mut usize, count: usize) -> Option<()> {
    match fuel.checked_sub(count) {
        Some(left) => {
            *fuel = left;
            Some(())
        }
        None => {
            *fuel = 0;
            None
        }
    }
}

impl Interpreter<'_> {
    pub(super) fn check_comptime_domain(
        &mut self,
        ty: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        if !self.comptime_only {
            return Ok(());
        }
        if self.graph.is_some() {
            let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let checked = nir::phases::validate_comptime_storage_with_layouts(
                ty,
                &[],
                &|ty, fuel| {
                    let nominal;
                    let fields: Vec<&Term> = match ty {
                        Term::Nominal { .. } => {
                            charge_layout_work(fuel, 1)?;
                            nir::type_computations::charge_term(ty, fuel)
                                .map_err(|_| *fuel = 0)
                                .ok()?;
                            nominal = self.concrete_nominal(ty)?;
                            let definition = &nominal;
                            let count = match definition {
                                ConcreteNominal::Struct(def) => def.fields.len(),
                                ConcreteNominal::Enum(def) => {
                                    charge_layout_work(fuel, def.variants.len())?;
                                    def.variants
                                        .iter()
                                        .fold(0usize, |n, v| n.saturating_add(v.fields.len()))
                                }
                                ConcreteNominal::Newtype(_) => 1,
                            };
                            charge_layout_work(fuel, count)?;
                            match definition {
                                ConcreteNominal::Struct(def) => {
                                    def.fields.iter().map(|(_, ty)| ty).collect()
                                }
                                ConcreteNominal::Enum(def) => {
                                    def.variants.iter().flat_map(|v| &v.fields).collect()
                                }
                                ConcreteNominal::Newtype(def) => vec![&def.representation],
                            }
                        }
                        Term::NormalForm { head, .. } => {
                            *fuel = fuel.checked_sub(1)?;
                            let metadata = self.normal_forms.get(head)?;
                            if let Some(fields) = metadata.tuple_elements(ty) {
                                charge_layout_work(fuel, fields.len())?;
                                fields.iter().collect()
                            } else if let Some(element) = metadata.list_element(ty) {
                                charge_layout_work(fuel, 1)?;
                                vec![element]
                            } else {
                                vec![]
                            }
                        }
                        _ => return None,
                    };
                    for field in &fields {
                        nir::type_computations::charge_term(field, fuel)
                            .map_err(|_| *fuel = 0)
                            .ok()?;
                    }
                    Some(fields.into_iter().cloned().collect())
                },
                &mut fuel,
            );
            self.budget.steps -= (before - fuel) as u64;
            return checked.map_err(|_| {
                self.failure(
                    if fuel == 0 {
                        ErrorKind::StepsExhausted
                    } else {
                        ErrorKind::PhaseMismatch
                    },
                    Some(source),
                )
            });
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
        let mut package = package();
        let declaration = DeclarationId {
            package: 81,
            local: 2,
        };
        package.enums.push(nir::program::Enum {
            declaration,
            name: "Packet".into(),
            is_intrinsic: false,
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            groups: vec![],
            encoding: None,
            symbol_encoding: None,
            variants: vec![
                nir::program::Variant {
                    name: "empty".into(),
                    shape: nir::program::VariantShape::Unit,
                    fields: vec![],
                },
                nir::program::Variant {
                    name: "pointer".into(),
                    shape: nir::program::VariantShape::Tuple,
                    fields: vec![Term::Pointer(Box::new(Term::Primitive(Primitive::Bool)))],
                },
            ],
        });
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
            *cell.0.lock().unwrap() = Value::Variant {
                declaration,
                arguments: vec![],
                index: 0,
                fields: vec![],
            };
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
    #[test]
    fn concrete_domain_lookup_does_not_rescan_unrelated_instances() {
        let ty = |local| Term::Nominal {
            declaration: DeclarationId { package: 91, local },
            arguments: vec![],
        };
        let make = |count| iir::Package {
            nominals: (1..=count)
                .map(|local| iir::NominalInstance {
                    ty: ty(local),
                    definition: iir::NominalDefinition::Newtype(nir::newtypes::Newtype {
                        declaration: DeclarationId { package: 91, local },
                        name: "Flag".into(),
                        type_parameters: vec![],
                        parameter_types: vec![],
                        capture_count: 0,
                        requirements: vec![],
                        capabilities: vec![],
                        representation: Term::Primitive(Primitive::Bool),
                    }),
                })
                .collect(),
            ..Default::default()
        };
        let mut costs = vec![];
        for count in [1, 128] {
            let graph = make(count);
            let mut vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
            let before = vm.remaining_steps();
            vm.check_comptime_domain(&ty(count), SourceRange { start: 1, end: 2 })
                .unwrap();
            costs.push(before - vm.remaining_steps());
        }
        assert_eq!(
            costs[0], costs[1],
            "unrelated declarations affect every domain lookup"
        );
    }

    #[test]
    fn concrete_domain_checks_charge_empty_enum_alternatives() {
        let declaration = DeclarationId {
            package: 91,
            local: 1,
        };
        let ty = Term::Nominal {
            declaration,
            arguments: vec![],
        };
        let make = |count| iir::Package {
            nominals: vec![iir::NominalInstance {
                ty: ty.clone(),
                definition: iir::NominalDefinition::Enum(nir::program::Enum {
                    declaration,
                    name: "Choices".into(),
                    is_intrinsic: false,
                    type_parameters: vec![],
                    parameter_types: vec![],
                    requirements: vec![],
                    capabilities: vec![],
                    groups: vec![],
                    encoding: None,
                    symbol_encoding: None,
                    variants: (0..count)
                        .map(|i| nir::program::Variant {
                            name: format!("case{i}"),
                            shape: nir::program::VariantShape::Unit,
                            fields: vec![],
                        })
                        .collect(),
                }),
            }],
            ..Default::default()
        };
        let mut costs = vec![];
        for count in [1, 128] {
            let graph = make(count);
            let mut vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
            let before = vm.remaining_steps();
            vm.check_comptime_domain(&ty, SourceRange { start: 1, end: 2 })
                .unwrap();
            costs.push(before - vm.remaining_steps());
        }
        assert!(
            costs[1] >= costs[0] + 127,
            "empty variants were scanned for free: {costs:?}"
        );
    }
}
