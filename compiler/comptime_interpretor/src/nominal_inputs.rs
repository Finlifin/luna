//! Nominal/alias input policies over the shared invocation scheduler.
use super::*;
use nir::program::NormalFormArgument as A;

pub(super) struct Source<'e> {
    expression: input_collection::Expression<'e>,
    specialize: bool,
    parameters: Vec<nir::DeclarationParameter>,
    domains: Vec<Term>,
    alias: bool,
    locals: Arc<[LocalId]>,
    completed: input_bindings::Bindings,
}

pub(super) type Frame<'e> = input_collection::Frame<Source<'e>>;

impl fmt::Display for Source<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(nominal-input-source (alias {}) {})",
            self.alias, self.completed
        )
    }
}

impl<'e> Frame<'e> {
    pub fn new(
        expression: input_collection::Expression<'e>,
        parameters: Vec<nir::DeclarationParameter>,
        domains: Vec<Term>,
        specialize: bool,
        locals: Arc<[LocalId]>,
    ) -> Self {
        Self::from_source(Source::new(
            expression, parameters, domains, specialize, locals,
        ))
    }
    pub fn finish(self) -> Option<Vec<Term>> {
        self.source.completed.finish()
    }
}

impl<'e> Source<'e> {
    pub fn new(
        expression: input_collection::Expression<'e>,
        parameters: Vec<nir::DeclarationParameter>,
        domains: Vec<Term>,
        specialize: bool,
        locals: Arc<[LocalId]>,
    ) -> Self {
        Self {
            locals,
            specialize,
            alias: matches!(expression.kind, K::CheckTypeAlias { .. }),
            completed: input_bindings::Bindings::admitted(parameters.iter().map(|p| p.declaration)),
            expression,
            parameters,
            domains,
        }
    }
    fn domain(&self, slot: usize) -> &Term {
        self.domains
            .get(slot)
            .unwrap_or(&Term::Primitive(Primitive::Type))
    }
}

impl input_collection::Source for Source<'_> {
    fn expression(&self) -> &Expr {
        &self.expression
    }
    fn substitutions(&self) -> Option<&input_bindings::Bindings> {
        self.specialize.then_some(&self.completed)
    }
    fn pending_inputs(
        &self,
        vm: &mut Interpreter<'_>,
        slot: usize,
    ) -> Result<BTreeSet<usize>, Error> {
        self.completed
            .pending_inputs(vm, self.domain(slot), self.input(slot).source())
    }
    fn expected(&self, vm: &mut Interpreter<'_>, slot: usize) -> Result<Term, Error> {
        self.completed
            .instantiate(vm, self.domain(slot), self.input(slot).source())
    }
    fn check_input(
        &self,
        vm: &mut Interpreter<'_>,
        slot: usize,
        operand: &A,
        bindings: Option<&input_bindings::Bindings>,
        expected: Option<&Term>,
    ) -> Result<(), Error> {
        let type_slot = self.parameters[slot].sort == nir::Sort::Type;
        if let A::Value(value) = operand
            && (type_slot || self.alias && expected.is_some())
        {
            let ty = input_domains::annotation(vm, bindings, &value.ty, value.source)?;
            if type_slot && !matches!(*ty, Term::Primitive(Primitive::Type | Primitive::Never)) {
                return Err(vm.failure(ErrorKind::Type, Some(value.source)));
            }
            if self.alias
                && let Some(expected) = expected
                && *ty != *expected
                && *ty != Term::Primitive(Primitive::Never)
            {
                return Err(vm.failure(ErrorKind::Type, Some(value.source)));
            }
        }
        Ok(())
    }
    fn bind(&mut self, vm: &mut Interpreter<'_>, slot: usize, value: Value) -> Result<(), Error> {
        let source = self.input(slot).source();
        let domain = self
            .domains
            .get(slot)
            .unwrap_or(&Term::Primitive(Primitive::Type));
        self.completed.bind(
            vm,
            slot,
            self.parameters[slot].clone(),
            domain,
            value,
            source,
        )
    }
    fn complete(&mut self, vm: &mut Interpreter<'_>) -> Result<input_collection::Progress, Error> {
        if self.completed.len() != self.inputs().0.len() {
            return Err(vm.failure(ErrorKind::Type, Some(self.expression.source)));
        }
        Ok(input_collection::Progress::InputsComplete)
    }
    fn cleanup(&self, environment: &mut Environment) {
        for binding in self.locals.iter() {
            environment.remove(binding);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use input_collection::InputPlan;

    #[test]
    fn transitive_pending_domains_drain_in_both_engines_and_fail_terminally() {
        let source = SourceRange { start: 1, end: 2 };
        let id = |local| DeclarationId { package: 93, local };
        let local = |index| LocalId { owner: 93, index };
        let rigid = |index, sort| Term::Rigid {
            declaration: id(index),
            sort,
        };
        let ty = Term::Primitive(Primitive::Type);
        let parameters = vec![
            nir::DeclarationParameter {
                declaration: id(0),
                sort: nir::Sort::ComptimeValue,
            },
            nir::DeclarationParameter {
                declaration: id(1),
                sort: nir::Sort::ComptimeValue,
            },
            nir::DeclarationParameter {
                declaration: id(2),
                sort: nir::Sort::Type,
            },
        ];
        let domains = vec![
            rigid(1, nir::Sort::ComptimeValue),
            rigid(2, nir::Sort::Type),
            ty.clone(),
        ];
        let package = Package::default();
        for compiled in [false, true] {
            let mut vm = if compiled {
                Interpreter::for_comptime(&package, Budget::default()).unwrap()
            } else {
                Interpreter::for_comptime_tree(&package, Budget::default()).unwrap()
            };
            for spelling in ["256", "17"] {
                let expression = Arc::new(Expr {
                    source,
                    ty: ty.clone(),
                    kind: K::NominalType {
                        declaration: id(3),
                        preparations: vec![],
                        bindings: Some(vec![local(0), local(1), local(2)]),
                        order: None,
                        arguments: vec![
                            A::Integer {
                                source,
                                spelling: spelling.into(),
                                negative: false,
                            },
                            A::Value(Expr {
                                source,
                                ty: ty.clone(),
                                kind: K::TypeValue(Term::Primitive(Primitive::U8)),
                            }),
                            A::Value(Expr {
                                source,
                                ty: ty.clone(),
                                kind: K::TypeValue(ty.clone()),
                            }),
                        ],
                    },
                });
                let mut environment = Environment::from([(local(99), Value::Bool(true))]);
                let locals =
                    input_collection::admit_scope(&mut vm, &expression, &environment).unwrap();
                let mut frame = Frame::new(
                    input_collection::Expression::Owned(expression),
                    parameters.clone(),
                    domains.clone(),
                    true,
                    locals.into(),
                );
                let result = vm.run_nominal_inputs(&mut frame, &mut environment);
                if spelling == "256" {
                    assert!(
                        result.is_err(),
                        "out-of-range input admitted in engine {compiled}"
                    );
                } else {
                    assert!(result.unwrap().is_none());
                    assert_eq!(
                        environment.get(&local(0)),
                        Some(&Value::UnsignedInteger(17))
                    );
                }
                assert_eq!(
                    frame
                        .next_input(&mut vm, &mut environment)
                        .err()
                        .unwrap()
                        .kind,
                    ErrorKind::InvalidControlFlow,
                );
                frame.cleanup(&mut environment);
                assert_eq!(
                    environment,
                    Environment::from([(local(99), Value::Bool(true))])
                );
                if spelling == "17" {
                    assert_eq!(
                        frame.finish().unwrap(),
                        vec![
                            Term::Integer(comptime_runtime::Integer::from_u128(17)),
                            Term::Primitive(Primitive::U8),
                            ty.clone(),
                        ]
                    );
                }
            }
        }
    }

    #[test]
    fn captured_type_arriving_last_completes_earlier_staged_input() {
        let source = SourceRange { start: 1, end: 2 };
        let id = |local| DeclarationId { package: 92, local };
        let local = |index| LocalId { owner: 92, index };
        let parameters = vec![
            nir::DeclarationParameter {
                declaration: id(1),
                sort: nir::Sort::ComptimeValue,
            },
            nir::DeclarationParameter {
                declaration: id(2),
                sort: nir::Sort::Type,
            },
        ];
        let domains = vec![
            Term::Rigid {
                declaration: id(2),
                sort: nir::Sort::Type,
            },
            Term::Primitive(Primitive::Type),
        ];
        let expression = Expr {
            source,
            ty: Term::Primitive(Primitive::Type),
            kind: K::NominalType {
                declaration: id(0),
                preparations: vec![],
                order: None,
                bindings: Some(vec![local(0), local(1)]),
                arguments: vec![
                    A::Integer {
                        spelling: "17".into(),
                        negative: false,
                        source,
                    },
                    A::Value(Expr {
                        source,
                        ty: Term::Primitive(Primitive::Type),
                        kind: K::TypeValue(Term::Primitive(Primitive::U8)),
                    }),
                ],
            },
        };
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut environment = Environment::new();
        let locals = input_collection::admit_scope(&mut vm, &expression, &environment).unwrap();
        let mut frame = Frame::new(
            input_collection::Expression::Owned(Arc::new(expression)),
            parameters,
            domains,
            true,
            locals.into(),
        );
        let input_collection::Progress::Input(bytecode::InputFrame { mut code, finish }) =
            frame.next_input(&mut vm, &mut environment).unwrap()
        else {
            panic!("expected staged input");
        };
        let flow = vm.run_fragment(&mut code);
        let result = finish.complete(&mut vm, code, flow);
        assert!(
            frame
                .accept_input(&mut vm, &mut environment, result)
                .unwrap()
                .is_none()
        );
        assert!(environment.is_empty());
        assert!(
            frame
                .to_string()
                .contains("(pending (input 0 (awaiting 1)))")
        );
        let mut frame = Box::new(frame);
        assert!(
            vm.run_nominal_inputs(&mut frame, &mut environment)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            environment.get(&local(1)),
            Some(&Value::Type(Term::Primitive(Primitive::U8)))
        );
        assert_eq!(vm.bytecode_staged_inputs(), 1);
        assert_eq!(vm.bytecode_contextual_inputs(), 1);
        assert_eq!(
            frame
                .next_input(&mut vm, &mut environment)
                .err()
                .unwrap()
                .kind,
            ErrorKind::InvalidControlFlow
        );
        assert_eq!(
            frame.finish().unwrap(),
            vec![
                Term::Integer(comptime_runtime::Integer::from_u128(17)),
                Term::Primitive(Primitive::U8)
            ]
        );
    }

    #[test]
    fn completed_inputs_retain_dynamic_sort_admission_and_abstract_evidence() {
        let source = SourceRange { start: 1, end: 2 };
        let id = |local| DeclarationId { package: 92, local };
        let rigid = |local, sort| Term::Rigid {
            declaration: id(local),
            sort,
        };
        let ty = Term::Primitive(Primitive::Type);
        let boolean = Term::Primitive(Primitive::Bool);
        let parameters = vec![
            nir::DeclarationParameter {
                declaration: id(1),
                sort: nir::Sort::Type,
            },
            nir::DeclarationParameter {
                declaration: id(2),
                sort: nir::Sort::ComptimeValue,
            },
            nir::DeclarationParameter {
                declaration: id(3),
                sort: nir::Sort::ComptimeValue,
            },
        ];
        let domains = vec![ty.clone(), rigid(1, nir::Sort::Type), boolean.clone()];
        let expression = Arc::new(Expr {
            source,
            ty: ty.clone(),
            kind: K::NominalType {
                declaration: id(0),
                preparations: vec![],
                order: None,
                bindings: None,
                arguments: (0..3)
                    .map(|_| {
                        A::Value(Expr {
                            source,
                            ty: ty.clone(),
                            kind: K::TypeValue(ty.clone()),
                        })
                    })
                    .collect(),
            },
        });
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.authorize_abstract_value_parameters(&[(id(4), boolean.clone())])
            .unwrap();
        for valid in [false, true] {
            let environment = Environment::new();
            let locals = input_collection::admit_scope(&mut vm, &expression, &environment).unwrap();
            let mut frame = Frame::new(
                input_collection::Expression::Owned(expression.clone()),
                parameters.clone(),
                domains.clone(),
                false,
                locals.into(),
            );
            // Source completion order differs from the declaration telescope.
            let value = if valid {
                Value::AbstractValue {
                    declaration: id(4),
                    ty: boolean.clone(),
                }
            } else {
                Value::UnsignedInteger(7)
            };
            frame.source.bind(&mut vm, 2, value).unwrap();
            use input_collection::Source as _;
            assert_eq!(
                frame.source.expected(&mut vm, 1).unwrap(),
                rigid(1, nir::Sort::Type)
            );
            assert_eq!(
                frame.source.pending_inputs(&mut vm, 1).unwrap(),
                BTreeSet::from([0])
            );
            frame
                .source
                .bind(&mut vm, 0, Value::Type(ty.clone()))
                .unwrap();
            frame
                .source
                .bind(&mut vm, 1, Value::Type(Term::Primitive(Primitive::U8)))
                .unwrap();
            let result = frame.source.completed.instantiate(
                &mut vm,
                &rigid(2, nir::Sort::ComptimeValue),
                source,
            );
            if valid {
                assert_eq!(result.unwrap(), Term::Primitive(Primitive::U8));
                assert_eq!(
                    frame.finish().unwrap(),
                    vec![
                        ty.clone(),
                        Term::Primitive(Primitive::U8),
                        rigid(4, nir::Sort::ComptimeValue)
                    ]
                );
            } else {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
            }
        }
    }
}
