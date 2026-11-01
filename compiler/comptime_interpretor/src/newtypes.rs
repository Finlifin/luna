//! Newtype conversions transfer a whole payload, never expose a record field.
use super::*;
impl Interpreter<'_> {
    #[inline(never)]
    pub(super) fn eval_newtype_convert(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::NewtypeConvert(operand) = &expr.kind else {
            unreachable!()
        };
        let plan = self.prepare_newtype_conversion(&operand.ty, &expr.ty, expr.source)?;
        let value = value!(self.eval(operand, environment));
        self.complete_newtype_conversion(&plan, value, expr.source)
            .map(Flow::Value)
    }

    pub(super) fn prepare_newtype_conversion(
        &mut self,
        input: &Term,
        output: &Term,
        source: SourceRange,
    ) -> Result<nir::newtypes::Conversion, Error> {
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let plan = if self.graph.is_some() {
            // Only the two endpoint instances can establish a direct edge.
            // Their IIR descriptors already substituted the source telescope.
            (|| {
                for endpoint in [input, output] {
                    nir::type_computations::charge_term(endpoint, &mut fuel)
                        .map_err(|_| nir::newtypes::ConversionError::Budget)?;
                    if let Some(ConcreteNominal::Newtype(definition)) =
                        self.concrete_nominal(endpoint)
                    {
                        if let Some(plan) = nir::newtypes::instance_conversion(
                            &definition,
                            endpoint,
                            input,
                            output,
                            &mut fuel,
                        )? {
                            return Ok(Some(plan));
                        }
                    }
                }
                Ok(None)
            })()
        } else {
            nir::newtypes::conversion_with(input, output, &[], &mut fuel, |id, _| {
                Ok(self.newtypes.get(&id))
            })
        };
        self.budget.steps -= (before - fuel) as u64;
        let plan = plan
            .map_err(|e| {
                self.failure(
                    match e {
                        nir::newtypes::ConversionError::Budget => ErrorKind::StepsExhausted,
                        _ => ErrorKind::Type,
                    },
                    Some(source),
                )
            })?
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        self.check_comptime_domain(input, source)?;
        self.check_comptime_domain(output, source)?;
        self.admit_type(input, 0)?;
        self.admit_type(output, 0)?;
        Ok(plan)
    }

    pub(super) fn complete_newtype_conversion(
        &self,
        plan: &nir::newtypes::Conversion,
        value: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if !self.has_type(&value, plan.source()) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        let value = match plan.direction() {
            nir::newtypes::Direction::Wrap => Value::Newtype {
                ty: plan.target().clone(),
                value: Box::new(value),
            },
            nir::newtypes::Direction::Unwrap => {
                let Value::Newtype { ty, value } = value else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                if &ty != plan.source() {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                *value
            }
        };
        if !self.has_type(&value, plan.target()) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::Place;
    const SOURCE: SourceRange = SourceRange { start: 3, end: 9 };
    const INPUT: LocalId = LocalId { owner: 1, index: 0 };
    const TRACE: LocalId = LocalId { owner: 1, index: 1 };
    fn expr(ty: Term, kind: K) -> Expr {
        Expr {
            source: SOURCE,
            ty,
            kind,
        }
    }
    fn nominal(local: u32) -> Term {
        Term::Nominal {
            declaration: DeclarationId { package: 0, local },
            arguments: vec![],
        }
    }
    fn query(input: Term, output: Term) -> Expr {
        let boolean = Term::Primitive(Primitive::Bool);
        expr(
            output,
            K::NewtypeConvert(Box::new(expr(
                input.clone(),
                K::Block {
                    statements: vec![expr(
                        Term::Primitive(Primitive::Void),
                        K::Assign {
                            place: Place {
                                root: TRACE,
                                fields: vec![],
                            },
                            value: Box::new(expr(
                                boolean.clone(),
                                K::Not(Box::new(expr(boolean, K::Local(TRACE)))),
                            )),
                        },
                    )],
                    result: Some(Box::new(expr(input, K::Local(INPUT)))),
                },
            ))),
        )
    }
    fn execute(
        vm: &mut Interpreter<'_>,
        body: &Expr,
        code: &Arc<comptime_compiler::Program>,
        env: &mut Environment,
        compiled: bool,
    ) -> Result<Flow, Error> {
        if compiled {
            vm.execute_code(code, env)
        } else {
            vm.eval(body, env)
        }
    }
    #[test]
    fn newtype_vm_rechecks_live_edges_and_preserves_failure_effects() {
        let boolean = Term::Primitive(Primitive::Bool);
        let mut package = Package::default();
        for local in [1, 2] {
            package.newtypes.push(nir::newtypes::Newtype {
                declaration: DeclarationId { package: 0, local },
                name: format!("Wrapper{local}"),
                type_parameters: vec![],
                parameter_types: vec![],
                capture_count: 0,
                requirements: vec![],
                capabilities: vec![],
                representation: boolean.clone(),
            });
        }
        let wrap = query(boolean.clone(), nominal(1));
        let wrap_code = Arc::new(comptime_compiler::compile(&wrap, &mut 10000).unwrap());
        let dump = wrap_code.to_string();
        assert!(dump.contains("(prepare-newtype-conversion "), "{dump}");
        assert!(dump.contains("(finish-newtype-conversion "), "{dump}");
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut env =
                Environment::from([(INPUT, Value::Bool(true)), (TRACE, Value::Bool(false))]);
            assert!(matches!(
                execute(&mut vm, &wrap, &wrap_code, &mut env, compiled).unwrap(),
                Flow::Value(Value::Newtype { .. })
            ));
            assert_eq!(
                vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                Value::Bool(true)
            );
            let position = Arc::make_mut(&mut vm.newtypes.source.positions)
                .remove(&package.newtypes[0].declaration)
                .unwrap();
            env.insert(TRACE, Value::Bool(false));
            assert_eq!(
                execute(&mut vm, &wrap, &wrap_code, &mut env, compiled)
                    .unwrap_err()
                    .kind,
                ErrorKind::Type
            );
            assert_eq!(
                vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                Value::Bool(false)
            );
            Arc::make_mut(&mut vm.newtypes.source.positions)
                .insert(package.newtypes[0].declaration, position);
            for (input, output, value, observed) in [
                (
                    nominal(1),
                    nominal(2),
                    Value::Newtype {
                        ty: nominal(1),
                        value: Box::new(Value::Bool(true)),
                    },
                    false,
                ),
                (
                    nominal(1),
                    boolean.clone(),
                    Value::Newtype {
                        ty: nominal(2),
                        value: Box::new(Value::Bool(true)),
                    },
                    true,
                ),
                (
                    nominal(1),
                    boolean.clone(),
                    Value::Newtype {
                        ty: nominal(1),
                        value: Box::new(Value::UnsignedInteger(1)),
                    },
                    true,
                ),
            ] {
                let body = query(input, output);
                let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                env.insert(INPUT, value);
                env.insert(TRACE, Value::Bool(false));
                assert_eq!(
                    execute(&mut vm, &body, &code, &mut env, compiled)
                        .unwrap_err()
                        .kind,
                    ErrorKind::Type
                );
                assert_eq!(
                    vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                    Value::Bool(observed)
                );
            }
            env.insert(INPUT, Value::Bool(false));
            env.insert(TRACE, Value::Bool(false));
            assert!(matches!(
                execute(&mut vm, &wrap, &wrap_code, &mut env, compiled).unwrap(),
                Flow::Value(Value::Newtype { .. })
            ));
        }
    }
    #[test]
    fn conversion_checks_formation_and_inactive_storage_before_operand_effects() {
        let boolean = Term::Primitive(Primitive::Bool);
        let optional_pointer = Term::Optional(Box::new(Term::Pointer(Box::new(boolean.clone()))));
        let mut package = Package::default();
        for (local, representation, requirements) in [
            (
                1,
                boolean.clone(),
                vec![nir::program::TypePredicate::Bool(false)],
            ),
            (2, boolean.clone(), vec![]),
            (3, optional_pointer.clone(), vec![]),
        ] {
            package.newtypes.push(nir::newtypes::Newtype {
                declaration: DeclarationId { package: 0, local },
                name: format!("Wrapper{local}"),
                type_parameters: vec![],
                parameter_types: vec![],
                capture_count: 0,
                capabilities: vec![],
                requirements,
                representation,
            });
        }
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            for (input, output, value, expected) in [
                (
                    boolean.clone(),
                    nominal(1),
                    Value::Bool(true),
                    ErrorKind::TypeContractRejected,
                ),
                (
                    nominal(1),
                    boolean.clone(),
                    Value::Newtype {
                        ty: nominal(1),
                        value: Box::new(Value::Bool(true)),
                    },
                    ErrorKind::TypeContractRejected,
                ),
                (
                    optional_pointer.clone(),
                    nominal(3),
                    Value::Optional {
                        ty: optional_pointer.clone(),
                        value: None,
                    },
                    ErrorKind::PhaseMismatch,
                ),
            ] {
                let body = query(input, output);
                let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                let mut env = Environment::from([(INPUT, value), (TRACE, Value::Bool(false))]);
                assert_eq!(
                    execute(&mut vm, &body, &code, &mut env, compiled)
                        .unwrap_err()
                        .kind,
                    expected
                );
                assert_eq!(
                    vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                    Value::Bool(false)
                );
            }
            let body = query(boolean.clone(), nominal(2));
            let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
            let mut env =
                Environment::from([(INPUT, Value::Bool(true)), (TRACE, Value::Bool(false))]);
            assert!(matches!(
                execute(&mut vm, &body, &code, &mut env, compiled),
                Ok(Flow::Value(Value::Newtype { .. }))
            ));
        }
        let mut runtime = Interpreter::new(&package, Budget::default()).unwrap();
        let body = query(optional_pointer.clone(), nominal(3));
        let mut env = Environment::from([
            (
                INPUT,
                Value::Optional {
                    ty: optional_pointer,
                    value: None,
                },
            ),
            (TRACE, Value::Bool(false)),
        ]);
        assert!(matches!(
            runtime.eval(&body, &mut env),
            Ok(Flow::Value(Value::Newtype { .. }))
        ));
        assert_eq!(
            runtime.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
            Value::Bool(true)
        );
    }
    #[test]
    fn concrete_iir_conversion_uses_full_instance_storage_domains() {
        let declaration = DeclarationId {
            package: 0,
            local: 1,
        };
        let boolean = Term::Primitive(Primitive::Bool);
        let optional_pointer = Term::Optional(Box::new(Term::Pointer(Box::new(boolean.clone()))));
        let safe = Term::Nominal {
            declaration,
            arguments: vec![boolean.clone()],
        };
        let unsafe_storage = Term::Nominal {
            declaration,
            arguments: vec![Term::Primitive(Primitive::Char)],
        };
        let graph = iir::Package {
            nominals: [
                (safe.clone(), boolean.clone()),
                (unsafe_storage.clone(), optional_pointer.clone()),
            ]
            .into_iter()
            .map(|(ty, representation)| iir::NominalInstance {
                ty,
                definition: iir::NominalDefinition::Newtype(nir::newtypes::Newtype {
                    declaration,
                    name: "Applied".into(),
                    type_parameters: vec![],
                    parameter_types: vec![],
                    capture_count: 0,
                    requirements: vec![],
                    capabilities: vec![],
                    representation,
                }),
            })
            .collect(),
            ..Default::default()
        };
        for compiled in [false, true] {
            let mut vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
            for (input, output, value, accepted) in [
                (boolean.clone(), safe.clone(), Value::Bool(true), true),
                (
                    optional_pointer.clone(),
                    unsafe_storage.clone(),
                    Value::Optional {
                        ty: optional_pointer.clone(),
                        value: None,
                    },
                    false,
                ),
                (boolean.clone(), safe.clone(), Value::Bool(false), true),
            ] {
                let body = query(input, output);
                let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                let mut env = Environment::from([(INPUT, value), (TRACE, Value::Bool(false))]);
                let result = execute(&mut vm, &body, &code, &mut env, compiled);
                if accepted {
                    assert!(matches!(result, Ok(Flow::Value(Value::Newtype { .. }))));
                } else {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::PhaseMismatch);
                }
                assert_eq!(
                    vm.local_value(&env, TRACE, Some(SOURCE)).unwrap(),
                    Value::Bool(accepted)
                );
            }
            let before = vm.remaining_steps();
            vm.check_comptime_domain(&safe, SOURCE).unwrap();
            let cost = before - vm.remaining_steps();
            for steps in 0..cost {
                vm.budget.steps = steps;
                assert_eq!(
                    vm.check_comptime_domain(&safe, SOURCE).unwrap_err().kind,
                    ErrorKind::StepsExhausted
                );
            }
            vm.budget.steps = 100_000;
            vm.check_comptime_domain(&safe, SOURCE).unwrap();
        }
    }

    #[test]
    fn source_newtype_conversion_cost_ignores_unrelated_declarations() {
        let boolean = Term::Primitive(Primitive::Bool);
        let definition = |local, representation: Term| nir::newtypes::Newtype {
            declaration: DeclarationId { package: 0, local },
            name: format!("Wrapper{local}"),
            type_parameters: vec![],
            parameter_types: vec![],
            capture_count: 0,
            requirements: vec![],
            capabilities: vec![],
            representation,
        };
        let mut costs = vec![];
        for count in [0, 128] {
            let package = Package {
                newtypes: (100..100 + count)
                    .map(|local| definition(local, boolean.clone()))
                    .chain([definition(1, boolean.clone())])
                    .collect(),
                ..Default::default()
            };
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            vm.install_owned_generated_newtypes_with(
                vec![definition(2, nominal(1))].into(),
                || Ok::<_, ()>(()),
            )
            .unwrap();
            let before = vm.remaining_steps();
            for (input, output) in [
                (boolean.clone(), nominal(1)),
                (nominal(1), boolean.clone()),
                (nominal(1), nominal(2)),
                (nominal(2), nominal(1)),
            ] {
                let plan = vm
                    .prepare_newtype_conversion(&input, &output, SOURCE)
                    .unwrap();
                assert_eq!(plan.source(), &input);
                assert_eq!(plan.target(), &output);
            }
            costs.push(before - vm.remaining_steps());
        }
        assert_eq!(
            costs[0], costs[1],
            "unrelated declarations enter each conversion"
        );
    }

    #[test]
    fn repeated_generic_declaration_at_both_endpoints_remains_one_identity() {
        let boolean = Term::Primitive(Primitive::Bool);
        let parameter = DeclarationId {
            package: 0,
            local: 2,
        };
        let package = Package {
            newtypes: vec![nir::newtypes::Newtype {
                declaration: DeclarationId {
                    package: 0,
                    local: 1,
                },
                name: "Wrapper".into(),
                type_parameters: vec![parameter],
                parameter_types: vec![],
                capture_count: 0,
                requirements: vec![],
                capabilities: vec![],
                representation: Term::Rigid {
                    declaration: parameter,
                    sort: nir::Sort::Type,
                },
            }],
            ..Default::default()
        };
        let applied = |argument| Term::Nominal {
            declaration: DeclarationId {
                package: 0,
                local: 1,
            },
            arguments: vec![argument],
        };
        let inner = applied(boolean.clone());
        let outer = applied(inner.clone());
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for (input, output) in [
            (inner.clone(), outer.clone()),
            (outer.clone(), inner.clone()),
        ] {
            let before = vm.remaining_steps();
            let expected = vm
                .prepare_newtype_conversion(&input, &output, SOURCE)
                .unwrap();
            let cost = before - vm.remaining_steps();
            for steps in 0..cost {
                vm.budget.steps = steps;
                assert_eq!(
                    vm.prepare_newtype_conversion(&input, &output, SOURCE)
                        .unwrap_err()
                        .kind,
                    ErrorKind::StepsExhausted,
                    "budget {steps}"
                );
                vm.budget.steps = 100_000;
                assert_eq!(
                    vm.prepare_newtype_conversion(&input, &output, SOURCE)
                        .unwrap(),
                    expected
                );
            }
        }
        assert!(
            vm.prepare_newtype_conversion(&inner, &inner, SOURCE)
                .is_err()
        );
        assert!(
            vm.prepare_newtype_conversion(&outer, &boolean, SOURCE)
                .is_err()
        );
        assert!(
            vm.prepare_newtype_conversion(
                &inner,
                &applied(Term::Primitive(Primitive::Char)),
                SOURCE
            )
            .is_err()
        );
    }
}
