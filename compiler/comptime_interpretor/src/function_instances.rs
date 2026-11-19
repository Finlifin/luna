//! Checked generic function formation shared by reference and bytecode execution.
use super::*;

// Separate declaration lookup allows borrowed slice queries without constructing
// an owned tuple key. A source VM retains its specialized expression bodies;
// only immutable code may also enter the persistent cache below.
pub(super) struct Instances<T> {
    entries: HashMap<DeclarationId, HashMap<Arc<[Term]>, Arc<T>>>,
}
impl<T> Default for Instances<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}
impl<T> Instances<T> {
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.values().map(HashMap::len).sum()
    }
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn lookup(&self, declaration: DeclarationId, arguments: &[Term]) -> Option<&Arc<T>> {
        self.entries.get(&declaration)?.get(arguments)
    }
    pub(super) fn insert(&mut self, key: (DeclarationId, Arc<[Term]>), code: Arc<T>) {
        self.entries.entry(key.0).or_default().insert(key.1, code);
    }
}

#[derive(Default)]
pub(super) struct Cache {
    entries: HashMap<DeclarationId, HashMap<Arc<[Term]>, Retained>>,
    count: usize,
    weight: u64,
}
struct Retained {
    code: Arc<Expr>,
    weight: u64,
}
impl Cache {
    // Callers preflight the query's term depth and compute its structural work.
    // Charge hashing/comparison before looking up the borrowed input slice.
    pub(super) fn lookup(
        &self,
        declaration: DeclarationId,
        arguments: &[Term],
        key_work: u64,
        fuel: &mut u64,
    ) -> Result<Option<(Arc<[Term]>, Arc<Expr>)>, ()> {
        *fuel = fuel.checked_sub(1).ok_or(())?;
        let Some(entries) = self.entries.get(&declaration) else {
            return Ok(None);
        };
        *fuel = fuel.checked_sub(key_work).ok_or(())?;
        Ok(entries
            .get_key_value(arguments)
            .map(|(inputs, entry)| (inputs.clone(), entry.code.clone())))
    }

    pub(super) fn insert(
        &mut self,
        key: (DeclarationId, Arc<[Term]>),
        code: Arc<Expr>,
        key_work: u64,
        work: u64,
    ) {
        let weight = work.saturating_add(key_work).saturating_add(1);
        if weight > 100_000 {
            return;
        }
        if let Some(previous) = self
            .entries
            .get_mut(&key.0)
            .and_then(|entries| entries.remove(key.1.as_ref()))
        {
            self.count -= 1;
            self.weight -= previous.weight;
        }
        if self.count >= 64 || self.weight.saturating_add(weight) > 100_000 {
            self.entries.clear();
            self.count = 0;
            self.weight = 0;
        }
        self.weight += weight;
        self.count += 1;
        self.entries
            .entry(key.0)
            .or_default()
            .insert(key.1, Retained { code, weight });
    }
}

impl Interpreter<'_> {
    pub(super) fn function_instance_value(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
        output: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if self.graph.is_some() {
            return self.iir_function_value(declaration, arguments, output, source);
        }
        let observed_arguments = arguments
            .iter()
            .map(|argument| self.selected_type_view(argument, Some(source)))
            .collect::<Result<Vec<_>, _>>()?;
        let arguments = observed_arguments.as_slice();
        let formation = self
            .functions
            .formation(&declaration)
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(source)))?;
        let signature = self.formed_function_signature(formation, arguments, source)?;
        if !self.observe_same_type(&signature, output, source)? {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        // A named value carries identity and checked inputs, never executable
        // code. Its receiving world rechecks formation when it is invoked.
        Ok(Value::FunctionInstance {
            declaration,
            arguments: observed_arguments,
        })
    }

    pub(super) fn formed_function_signature(
        &mut self,
        formation: nir::callable_source::Formation<'_>,
        arguments: &[Term],
        source: SourceRange,
    ) -> Result<Term, Error> {
        let frames = [self.ordinary_input_frame(formation.signature, source)?];
        let inputs = self.admit_function_inputs(
            formation.parameters,
            arguments,
            nir::DeclarationInputScope::ClosedInBody(&frames),
            source,
        )?;
        let signature = self.signature_from_admission(formation, &inputs, source)?;
        self.check_callable_requirements(formation, &inputs, source)?;
        Ok(signature)
    }

    pub(super) fn specialize_function_signature(
        &mut self,
        declaration: &nir::program::FunctionDeclaration,
        arguments: &[Term],
        source: SourceRange,
    ) -> Result<Term, Error> {
        let inputs = self.admit_function_inputs(
            &declaration.type_parameters,
            arguments,
            nir::DeclarationInputScope::Caller(&[]),
            source,
        )?;
        self.signature_from_admission(declaration.into(), &inputs, source)
    }

    pub(super) fn signature_from_admission(
        &mut self,
        formation: nir::callable_source::Formation<'_>,
        inputs: &nir::AdmittedArguments<'_>,
        source: SourceRange,
    ) -> Result<Term, Error> {
        let signature = self.query_admission(Some(source), |_, fuel| {
            Ok(inputs.instantiate_in_input_scope(formation.signature, fuel)?)
        })?;
        Ok(formation.qualify(signature))
    }

    /// Acquire immutable declaration-specialized code only after this
    /// activation's admission and entry predicates succeeded. Cache entries
    /// contain neither ordinary input values nor invocation results.
    pub(super) fn prepare_function_instance_body(
        &mut self,
        definition: &nir::program::Function,
        substitution: &nir::DeclarationSubstitution<'_>,
        source: SourceRange,
    ) -> Result<Arc<Expr>, Error> {
        let declaration = definition.declaration;
        let arguments = substitution.arguments();
        let before_key = self.budget.steps;
        self.charge_copy_work(Some(source), |fuel| {
            for argument in arguments {
                nir::copy_budget::charge_type_substitution(argument, &[], &[], fuel)?;
            }
            Ok(())
        })?;
        let key_work = before_key - self.budget.steps;
        if let Some(instance) = self.instances.lookup(declaration, arguments) {
            return Ok(instance.clone());
        }
        let persistent = self.abstract_type_parameters.is_empty()
            && self.abstract_value_parameters.is_empty()
            && arguments
                .iter()
                .all(|argument| argument.check_scope(&[]).is_ok());
        if persistent
            && let Some((inputs, body)) = self
                .prepared_instance_cache
                .lookup(declaration, arguments, key_work, &mut self.budget.steps)
                .map_err(|()| self.failure(ErrorKind::StepsExhausted, Some(source)))?
        {
            self.instances.insert((declaration, inputs), body.clone());
            return Ok(body);
        }
        let before_preparation = self.budget.steps;
        let body = Arc::new(self.instantiate_activation_code(&definition.body, substitution)?);
        let key = (declaration, Arc::from(arguments));
        if persistent && self.budget.steps >= key_work {
            self.budget.steps -= key_work;
            self.prepared_instance_cache.insert(
                key.clone(),
                body.clone(),
                key_work,
                before_preparation - self.budget.steps,
            );
        }
        self.instances.insert(key, body.clone());
        self.events.push(Event::InstantiateFunction {
            declaration,
            arguments: arguments.to_vec(),
        });
        Ok(body)
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;

    #[test]
    fn capability_queries_precede_instance_publication_and_cached_reuse() {
        use std::cell::Cell;
        let declaration = DeclarationId {
            package: 98,
            local: 1,
        };
        let source = SourceRange { start: 3, end: 8 };
        let requirement = nir::program::TraitRequirement {
            interface: DeclarationId {
                package: 98,
                local: 2,
            },
            interface_arguments: vec![Term::Bool(true)],
            subject: Term::Primitive(Primitive::Bool),
        };
        let function = nir::program::Function {
            interface: nir::program::FunctionDeclaration {
                source: nir::program::SourceRange { start: 0, end: 0 },
                declaration,
                name: "guarded".into(),
                phase: nir::program::FunctionPhase::Comptime,
                inline: false,
                pure: false,
                is_unsafe: false,
                type_parameters: vec![],
                parameters: vec![],
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(Term::Primitive(Primitive::Bool)),
                },
                defaults: vec![],
                preconditions: vec![],
                postconditions: vec![],
                requirements: vec![],
                capabilities: vec![requirement.clone()],
                result_binding: None,
                entry_snapshots: vec![],
            },
            body: Expr {
                source,
                ty: Term::Primitive(Primitive::Bool),
                kind: K::Literal(Literal::Bool(true)),
            },
        };
        let output = function.callable_type();
        let package = Package {
            functions: vec![function],
            ..Default::default()
        };
        let mode = Cell::new(0);
        let calls = Cell::new(0);
        let solver = |predicate: &nir::program::TypePredicate, fuel: &mut u64| {
            assert_eq!(
                predicate,
                &nir::program::TypePredicate::Trait(requirement.clone())
            );
            calls.set(calls.get() + 1);
            *fuel = fuel.saturating_sub(11);
            Ok(match mode.get() {
                0 => ContractOutcome::Unknown,
                1 => ContractOutcome::Rejected,
                _ => ContractOutcome::Satisfied,
            })
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_budgeted_type_solver(&solver);
        for (state, expected) in [
            (0, ErrorKind::TypeContractUnresolved),
            (1, ErrorKind::TypeContractRejected),
        ] {
            mode.set(state);
            let before = vm.remaining_steps();
            assert_eq!(
                vm.function_instance_value(declaration, &[], &output, source)
                    .unwrap_err()
                    .kind,
                expected
            );
            assert!(vm.remaining_steps() < before - 10);
            assert!(vm.instances.is_empty());
            assert_eq!(vm.prepared_instance_cache.count, 0);
            assert!(
                !vm.events
                    .iter()
                    .any(|event| matches!(event, Event::InstantiateFunction { .. }))
            );
        }
        mode.set(2);
        let value = vm
            .function_instance_value(declaration, &[], &output, source)
            .unwrap();
        assert!(vm.instances.is_empty());
        assert_eq!(vm.invoke_value(value, vec![]).unwrap(), Value::Bool(true));
        assert_eq!(vm.instances.len(), 1);
        assert!(vm.method_bodies.is_empty());
        assert!(vm.resolved_method_bodies.is_empty());
        let count = calls.get();
        mode.set(0);
        assert_eq!(
            vm.function_instance_value(declaration, &[], &output, source)
                .unwrap_err()
                .kind,
            ErrorKind::TypeContractUnresolved
        );
        assert_eq!(calls.get(), count + 1);
        assert_eq!(vm.instances.len(), 1);
        mode.set(2);
        vm.function_instance_value(declaration, &[], &output, source)
            .unwrap();
        assert_eq!(calls.get(), count + 2);
    }

    #[test]
    fn expression_specialization_preserves_limit_reasons_and_recovers() {
        let package = Package::default();
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        let source = SourceRange { start: 2, end: 9 };
        let small = Expr {
            source,
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Literal(Literal::Bool(true)),
        };
        let mut deep = small.clone();
        for _ in 0..nir::copy_budget::MAX_COPY_DEPTH + 1 {
            deep = Expr {
                source,
                ty: Term::Primitive(Primitive::Void),
                kind: K::Discard(Box::new(deep)),
            };
        }
        let before = vm.budget.steps;
        assert_eq!(
            vm.specialize_function_expression(&deep, &[], &[], &[])
                .unwrap_err()
                .kind,
            ErrorKind::DepthExhausted
        );
        assert!(vm.budget.steps > 0 && vm.budget.steps < before);
        let wide = Expr {
            source,
            ty: Term::Primitive(Primitive::Str),
            kind: K::Literal(Literal::String("x".repeat(4096))),
        };
        vm.budget.steps = 500;
        assert_eq!(
            vm.specialize_function_expression(&wide, &[], &[], &[])
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        assert!(vm.budget.steps > 0 && vm.budget.steps < 500);
        assert_eq!(
            vm.specialize_function_expression(&small, &[], &[], &[])
                .unwrap(),
            small
        );
        let invalid = Expr {
            source,
            ty: Term::Bound {
                depth: 0,
                index: 0,
                sort: nir::Sort::Type,
            },
            kind: K::Unit,
        };
        assert_eq!(
            vm.specialize_function_expression(&invalid, &[], &[], &[])
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
        assert_eq!(
            vm.specialize_function_expression(&small, &[], &[], &[])
                .unwrap(),
            small
        );
        assert!(vm.instances.is_empty());
    }

    #[test]
    fn prepared_code_copies_selected_payloads_and_reuses_immutable_instances() {
        let declaration = DeclarationId {
            package: 97,
            local: 1,
        };
        let parameter = DeclarationId {
            package: 97,
            local: 2,
        };
        let source = SourceRange { start: 2, end: 9 };
        let function = nir::program::Function {
            interface: nir::program::FunctionDeclaration {
                source: nir::program::SourceRange { start: 0, end: 0 },
                phase: nir::program::FunctionPhase::Runtime,
                inline: false,
                pure: false,
                is_unsafe: false,
                declaration,
                name: "not-retained-by-closure".repeat(10_000),
                type_parameters: vec![parameter],
                parameters: vec![],
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(Term::Primitive(Primitive::Bool)),
                },
                defaults: vec![],
                preconditions: vec![],
                postconditions: vec![],
                requirements: vec![],
                capabilities: vec![],
                result_binding: None,
                entry_snapshots: vec![],
            },
            body: Expr {
                source,
                ty: Term::Primitive(Primitive::Bool),
                kind: K::Literal(Literal::Bool(true)),
            },
        };
        let output = function.callable_type();
        let package = Package {
            functions: vec![function],
            ..Default::default()
        };
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        vm.budget.steps = 2000;
        let arguments = [Term::Primitive(Primitive::Bool)];
        let missing = Term::AssociatedType {
            origin: Box::new(nir::AssociatedTypeOrigin {
                interface: DeclarationId {
                    package: 97,
                    local: 3,
                },
                member: DeclarationId {
                    package: 97,
                    local: 4,
                },
            }),
            subject: Box::new(Term::Primitive(Primitive::Bool)),
            arguments: vec![],
        };
        for (result, expected) in [
            (Term::Primitive(Primitive::Char), ErrorKind::Type),
            (missing, ErrorKind::UnmaterializedDispatch),
            (
                Term::Record(vec![(
                    "wide".repeat(4096),
                    Term::Primitive(Primitive::Bool),
                )]),
                ErrorKind::StepsExhausted,
            ),
        ] {
            let wrong = Term::Function {
                parameters: vec![],
                result: Box::new(result),
            };
            let before = vm.remaining_steps();
            assert_eq!(
                vm.function_instance_value(declaration, &arguments, &wrong, source)
                    .unwrap_err()
                    .kind,
                expected
            );
            assert!(vm.remaining_steps() < before);
            assert!(vm.instances.is_empty());
            assert_eq!(vm.prepared_instance_cache.count, 0);
            assert!(
                !vm.events
                    .iter()
                    .any(|event| matches!(event, Event::InstantiateFunction { .. }))
            );
        }
        let first = vm
            .function_instance_value(declaration, &arguments, &output, source)
            .unwrap();
        let second = vm
            .function_instance_value(declaration, &arguments, &output, source)
            .unwrap();
        assert_eq!(first, second);
        assert!(matches!(first, Value::FunctionInstance { .. }));
        assert!(vm.instances.is_empty());
        assert_eq!(vm.invoke_value(first, vec![]).unwrap(), Value::Bool(true));
        let prepared = vm
            .instances
            .lookup(declaration, &arguments)
            .unwrap()
            .clone();
        assert_eq!(vm.invoke_value(second, vec![]).unwrap(), Value::Bool(true));
        let wrong = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(Primitive::Char)),
        };
        assert_eq!(
            vm.function_instance_value(declaration, &arguments, &wrong, source)
                .unwrap_err()
                .kind,
            ErrorKind::Type
        );
        assert_eq!(vm.prepared_instance_cache.count, 1);
        assert!(Arc::ptr_eq(
            vm.instances.lookup(declaration, &arguments).unwrap(),
            &prepared
        ));
        assert_eq!(vm.instances.len(), 1);
        assert_eq!(
            vm.events
                .iter()
                .filter(|event| matches!(event, Event::InstantiateFunction { .. }))
                .count(),
            1
        );
        let failure = vm
            .function_instance_value(declaration, &[Term::Bool(true)], &output, source)
            .unwrap_err();
        assert!(matches!(
            failure.kind,
            ErrorKind::FunctionInputRejected(_) | ErrorKind::Type
        ));
        assert_eq!(vm.instances.len(), 1);
        assert!(matches!(
            vm.function_instance_value(declaration, &arguments, &output, source)
                .unwrap(),
            Value::FunctionInstance { .. }
        ));
    }

    #[test]
    fn named_formation_and_rejected_entry_do_not_prepare_generic_bodies() {
        let declaration = DeclarationId {
            package: 96,
            local: 1,
        };
        let parameter = DeclarationId {
            package: 96,
            local: 2,
        };
        let flag = LocalId {
            owner: 96,
            index: 0,
        };
        let source = SourceRange { start: 10, end: 20 };
        let mut formation_cost = None;
        for width in [0, 4096] {
            let output = Expr {
                source,
                ty: Term::Primitive(Primitive::Type),
                kind: K::TypeValue(Term::Rigid {
                    declaration: parameter,
                    sort: nir::Sort::Type,
                }),
            };
            let function = nir::program::Function {
                interface: nir::program::FunctionDeclaration {
                    source: nir::program::SourceRange { start: 0, end: 0 },
                    declaration,
                    name: "guarded-type".into(),
                    phase: nir::program::FunctionPhase::Comptime,
                    inline: false,
                    pure: false,
                    is_unsafe: false,
                    type_parameters: vec![parameter],
                    parameters: vec![flag],
                    signature: Term::Function {
                        parameters: vec![nir::Parameter {
                            sort: nir::Sort::ComptimeValue,
                            ty: Term::Primitive(Primitive::Bool),
                        }],
                        result: Box::new(output.ty.clone()),
                    },
                    preconditions: vec![Expr {
                        source,
                        ty: Term::Primitive(Primitive::Bool),
                        kind: K::Local(flag),
                    }],
                    postconditions: vec![],
                    result_binding: None,
                    entry_snapshots: vec![],
                    defaults: vec![],
                    requirements: vec![],
                    capabilities: vec![],
                },
                body: Expr {
                    source,
                    ty: output.ty.clone(),
                    kind: K::Block {
                        statements: vec![
                            Expr {
                                source,
                                ty: Term::Primitive(Primitive::Void),
                                kind: K::Unit
                            };
                            width
                        ],
                        result: Some(Box::new(output)),
                    },
                },
            };
            let signature = function.callable_type();
            let package = Package {
                functions: vec![function],
                ..Default::default()
            };
            for tree in [false, true] {
                let mut vm = if tree {
                    Interpreter::for_comptime_tree(&package, Budget::default())
                } else {
                    Interpreter::for_comptime(&package, Budget::default())
                }
                .unwrap();
                vm.budget.steps = 1000;
                let callable = vm
                    .function_instance_value(
                        declaration,
                        &[Term::Primitive(Primitive::Bool)],
                        &signature,
                        source,
                    )
                    .unwrap();
                let cost = 1000 - vm.budget.steps;
                if let Some(previous) = formation_cost {
                    assert_eq!(cost, previous);
                } else {
                    formation_cost = Some(cost);
                }
                assert!(vm.instances.is_empty());
                assert_eq!(vm.prepared_instance_cache.count, 0);
                vm.effect_state.invocation = 41;
                let failure = vm
                    .invoke_value(callable.clone(), vec![Value::Bool(false)])
                    .unwrap_err();
                assert_eq!(failure.kind, ErrorKind::ProgramContractRejected);
                assert_eq!(failure.source, Some(source));
                assert!(vm.instances.is_empty());
                assert_eq!(vm.prepared_instance_cache.count, 0);
                assert_eq!(vm.invocation_depth, 0);
                assert_eq!(vm.effect_state.invocation, 41);
                assert!(!vm.events.iter().any(|event| matches!(
                    event,
                    Event::InstantiateFunction { .. } | Event::EnterFunction(_)
                )));

                vm.budget.steps = 1_000_000;
                assert_eq!(
                    vm.invoke_value(callable.clone(), vec![Value::Bool(true)])
                        .unwrap(),
                    Value::Type(Term::Primitive(Primitive::Bool))
                );
                assert_eq!(vm.instances.len(), 1);
                let cache = vm
                    .instances
                    .lookup(declaration, &[Term::Primitive(Primitive::Bool)])
                    .unwrap()
                    .clone();
                vm.budget.steps = 1000;
                assert_eq!(
                    vm.invoke_value(callable.clone(), vec![Value::Bool(false)])
                        .unwrap_err()
                        .kind,
                    ErrorKind::ProgramContractRejected
                );
                assert!(Arc::ptr_eq(
                    &cache,
                    vm.instances
                        .lookup(declaration, &[Term::Primitive(Primitive::Bool)])
                        .unwrap()
                ));
                assert_eq!(
                    vm.events
                        .iter()
                        .filter(|event| matches!(event, Event::EnterFunction(_)))
                        .count(),
                    1
                );
                vm.budget.steps = 1_000_000;
                assert_eq!(
                    vm.invoke_value(callable, vec![Value::Bool(true)]).unwrap(),
                    Value::Type(Term::Primitive(Primitive::Bool))
                );
                assert_eq!(
                    vm.events
                        .iter()
                        .filter(|event| matches!(event, Event::InstantiateFunction { .. }))
                        .count(),
                    1
                );
            }
        }
    }

    #[test]
    fn borrowed_lookup_preserves_shared_frames_and_distinguishes_inputs() {
        let declaration = DeclarationId {
            package: 1,
            local: 1,
        };
        let frame: Arc<[Term]> = vec![Term::Bool(true)].into();
        let code = Arc::new(Expr {
            source: SourceRange { start: 0, end: 1 },
            ty: Term::Primitive(Primitive::Void),
            kind: K::Unit,
        });
        let mut instances = Instances::default();
        instances.insert((declaration, frame.clone()), code.clone());
        let query = vec![Term::Bool(true)];
        let found = instances.lookup(declaration, &query).unwrap();
        assert!(Arc::ptr_eq(found, &code));
        let (retained, _) = instances.entries[&declaration]
            .get_key_value(query.as_slice())
            .unwrap();
        assert!(Arc::ptr_eq(retained, &frame));
        assert!(
            instances
                .lookup(declaration, &[Term::Bool(false)])
                .is_none()
        );
        assert!(
            instances
                .lookup(
                    DeclarationId {
                        local: 2,
                        ..declaration
                    },
                    &query
                )
                .is_none()
        );
        let mut cache = Cache::default();
        cache.insert((declaration, frame.clone()), code.clone(), 1, 1);
        assert!(Arc::ptr_eq(
            cache.entries[&declaration].keys().next().unwrap(),
            &frame
        ));
        assert!(cache.lookup(declaration, &query, 1, &mut 0).is_err());
        let (restored_inputs, restored) = cache
            .lookup(declaration, &query, 1, &mut 100)
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&restored, &code));
        assert!(Arc::ptr_eq(&restored_inputs, &frame));
        for local in 2..=64 {
            cache.insert(
                (
                    DeclarationId {
                        local,
                        ..declaration
                    },
                    frame.clone(),
                ),
                code.clone(),
                1,
                1,
            );
        }
        assert_eq!(cache.count, 64);
        // Unrelated declarations no longer consume a scan of their input frames.
        assert!(
            cache
                .lookup(declaration, &query, 1, &mut 2)
                .unwrap()
                .is_some()
        );
        cache.insert((declaration, frame.clone()), code.clone(), 1, 5);
        assert_eq!(cache.count, 64);
        assert_eq!(cache.weight, 63 * 3 + 7);
        cache.insert((declaration, frame.clone()), code.clone(), 1, 100_000);
        assert_eq!(cache.count, 64);
        assert_eq!(cache.weight, 63 * 3 + 7);
        cache.insert(
            (
                DeclarationId {
                    local: 65,
                    ..declaration
                },
                frame,
            ),
            code.clone(),
            1,
            1,
        );
        assert_eq!(cache.count, 1);
        assert_eq!(cache.weight, 3);
        assert!(
            cache
                .lookup(declaration, &query, 1, &mut 2)
                .unwrap()
                .is_none()
        );

        let mut specializations = Cache::default();
        for value in 0..64 {
            let inputs: Arc<[Term]> = (0..6)
                .map(|bit| Term::Bool(value & (1 << bit) != 0))
                .collect();
            specializations.insert((declaration, inputs), code.clone(), 6, 1);
        }
        let last = vec![Term::Bool(true); 6];
        assert!(
            specializations
                .lookup(declaration, &last, 6, &mut 6)
                .is_err()
        );
        let mut remaining = 7;
        assert!(
            specializations
                .lookup(declaration, &last, 6, &mut remaining)
                .unwrap()
                .is_some()
        );
        assert_eq!(remaining, 0);
        assert_eq!(specializations.count, 64);
        assert!(
            specializations
                .lookup(declaration, &[Term::Bool(true)], 1, &mut 2)
                .unwrap()
                .is_none()
        );
    }
}
