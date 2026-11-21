//! Reuse prepared bodies in one immutable world, without retaining call state.
use super::*;

#[derive(Clone)]
enum Entry {
    Dependent(Arc<BoundBody>),
    Prepared(Arc<Expr>),
}
struct BoundBody {
    slots: Arc<BTreeSet<usize>>,
    binding: nir::FunctionInputBinding,
    body: Arc<Expr>,
}
#[derive(Default)]
pub(super) struct Cache {
    entries: BTreeMap<CodeOrigin, (Entry, u64)>,
    owned: BTreeMap<usize, (std::sync::Weak<Expr>, Entry, u64)>,
    weight: u64,
}
impl Cache {
    fn insert(&mut self, id: CodeOrigin, entry: Entry, weight: u64) {
        const LIMIT: u64 = 100_000;
        if weight > LIMIT {
            return;
        }
        if let Some((_, previous)) = self.entries.remove(&id) {
            self.weight -= previous;
        }
        if self.weight.saturating_add(weight) > LIMIT {
            self.entries.clear();
            self.owned.clear();
            self.weight = 0;
        }
        self.weight += weight;
        self.entries.insert(id, (entry, weight));
    }
    fn owned(&mut self, body: &Arc<Expr>) -> Option<Entry> {
        let key = Arc::as_ptr(body) as usize;
        let (source, entry, _) = self.owned.get(&key)?;
        if source
            .upgrade()
            .is_some_and(|source| Arc::ptr_eq(&source, body))
        {
            return Some(entry.clone());
        }
        let (_, _, weight) = self.owned.remove(&key).unwrap();
        self.weight -= weight;
        None
    }

    fn insert_owned(&mut self, body: &Arc<Expr>, entry: Entry, weight: u64) {
        if weight > 100_000 {
            return;
        }
        let key = Arc::as_ptr(body) as usize;
        if let Some((_, _, previous)) = self.owned.remove(&key) {
            self.weight -= previous;
        }
        if self.weight.saturating_add(weight) > 100_000 {
            self.entries.clear();
            self.owned.clear();
            self.weight = 0;
        }
        self.weight += weight;
        self.owned
            .insert(key, (Arc::downgrade(body), entry, weight));
    }
}
pub(super) enum SourceKey<'a> {
    Named(CodeOrigin),
    Owned(&'a Arc<Expr>),
}

impl Interpreter<'_> {
    #[cfg(test)]
    pub(super) fn prepare_input_body(
        &mut self,
        body: &Expr,
        scope: &nir::FunctionInputBinding,
        origin: CodeOrigin,
    ) -> Result<(Arc<Expr>, bool), Error> {
        self.prepare_input_body_key(
            body,
            scope,
            SourceKey::Named(origin),
            &[],
            &Environment::new(),
        )
    }

    #[cfg(test)]
    pub(super) fn prepare_owned_input_body(
        &mut self,
        body: &Arc<Expr>,
        scope: &nir::FunctionInputBinding,
    ) -> Result<(Arc<Expr>, bool), Error> {
        self.prepare_input_body_key(
            body,
            scope,
            SourceKey::Owned(body),
            &[],
            &Environment::new(),
        )
    }

    pub(super) fn prepare_input_body_key(
        &mut self,
        body: &Expr,
        scope: &nir::FunctionInputBinding,
        key: SourceKey<'_>,
        deferred: &[(usize, LocalId)],
        environment: &Environment,
    ) -> Result<(Arc<Expr>, bool), Error> {
        let cached = match key {
            SourceKey::Named(origin) => self
                .input_body_cache
                .entries
                .get(&origin)
                .map(|(entry, _)| entry.clone()),
            SourceKey::Owned(body) => self.input_body_cache.owned(body),
        };
        if let Some(Entry::Prepared(body)) = cached {
            return Ok((body, true));
        }
        let slots = match &cached {
            Some(Entry::Dependent(previous)) => Some(previous.slots.clone()),
            _ => nir::copy_budget::expression_contextual_slots(body, &mut self.budget.steps)
                .map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(body.source)))?
                .map(Arc::new),
        };
        let completed = match &slots {
            Some(slots) => {
                self.complete_body_inputs(scope, slots, deferred, environment, body.source)?
            }
            None => None,
        };
        let scope = completed.as_ref().unwrap_or(scope);
        // Rewriting depends only on this checked binding and the immutable
        // source body. Equality covers all replacements and slot refinements,
        // not just the resulting signature or the positions declared as Type.
        let persistent =
            self.abstract_type_parameters.is_empty() && self.abstract_value_parameters.is_empty();
        if let Some(Entry::Dependent(previous)) = &cached {
            if persistent {
                self.charge_input_binding(scope, body.source)?;
                self.charge_input_binding(&previous.binding, body.source)?;
                if previous.binding == *scope {
                    return Ok((previous.body.clone(), false));
                }
            }
        }
        if cached.is_some() {
            // A cold dependency observation already preflights the source copy.
            nir::copy_budget::charge_type_rewrite(body, &mut self.budget.steps)
                .map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(body.source)))?;
        }
        let dependent = slots.is_some();
        let mut prepared = body.clone();
        // Retain all original rewrite/scope validation, including pattern-domain
        // completion, before publishing an independent body for later calls.
        self.specialize_input_types(&mut prepared, scope)?;
        let prepared = Arc::new(prepared);
        if !dependent || persistent {
            let before = self.budget.steps;
            nir::copy_budget::charge_type_rewrite(&prepared, &mut self.budget.steps)
                .map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(body.source)))?;
            if dependent {
                self.charge_input_binding(scope, body.source)?;
            }
            let weight = (before - self.budget.steps)
                .saturating_add(1)
                .saturating_add(slots.as_ref().map_or(0, |s| s.len() as u64));
            let entry = if let Some(slots) = slots {
                Entry::Dependent(Arc::new(BoundBody {
                    slots,
                    binding: scope.clone(),
                    body: prepared.clone(),
                }))
            } else {
                Entry::Prepared(prepared.clone())
            };
            match key {
                SourceKey::Named(origin) => self.input_body_cache.insert(origin, entry, weight),
                SourceKey::Owned(body) => self.input_body_cache.insert_owned(body, entry, weight),
            }
        }
        Ok((prepared, !dependent))
    }

    fn complete_body_inputs(
        &mut self,
        scope: &nir::FunctionInputBinding,
        slots: &BTreeSet<usize>,
        deferred: &[(usize, LocalId)],
        environment: &Environment,
        source: SourceRange,
    ) -> Result<Option<nir::FunctionInputBinding>, Error> {
        let mut supplied = BTreeMap::new();
        for (slot, binding) in deferred {
            self.tick(Some(source))?;
            if slots.contains(slot) {
                let value = environment
                    .get(binding)
                    .ok_or_else(|| self.failure(ErrorKind::MissingBinding, Some(source)))?;
                let term = match value {
                    Value::Type(_) | Value::TypeParameter(_) | Value::Description(_) => {
                        self.ordinary_type_identity(value, source)?
                    }
                    value if self.normal_input_is_data(value, source)? => {
                        Some(self.borrowed_residual_input_term(value, source)?)
                    }
                    _ => None,
                };
                if let Some(term) = term {
                    supplied.insert(*slot, term);
                }
            }
        }
        if supplied.is_empty() {
            return Ok(None);
        }
        self.query_admission(Some(source), |_, fuel| {
            Ok(Some(scope.bind_retained_bounded(supplied, fuel)?))
        })
    }

    fn charge_input_binding(
        &mut self,
        binding: &nir::FunctionInputBinding,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.charge_copy_work(Some(source), |fuel| {
            for term in binding.terms() {
                nir::copy_budget::charge_type_substitution(term, &[], &[], fuel)?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 1, local }
    }
    fn body(term: Term) -> Expr {
        Expr {
            source: SourceRange { start: 1, end: 2 },
            ty: Term::Primitive(Primitive::Type),
            kind: K::TypeValue(term),
        }
    }
    fn named(local: u32) -> CodeOrigin {
        CodeOrigin::Declaration(id(local))
    }

    fn scope(input: Primitive) -> nir::FunctionInputBinding {
        Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            result: Box::new(Term::Primitive(Primitive::Type)),
        }
        .prepare_function_inputs(&[Some(Term::Primitive(input))])
        .unwrap()
    }

    fn guarded_type_body(width: usize) -> Package {
        let parameter = |index| LocalId { owner: 1, index };
        let signature = Term::Function {
            parameters: vec![
                nir::Parameter {
                    sort: nir::Sort::Type,
                    ty: Term::Primitive(Primitive::Type),
                },
                nir::Parameter {
                    sort: nir::Sort::Runtime,
                    ty: Term::Primitive(Primitive::Bool),
                },
            ],
            result: Box::new(Term::Primitive(Primitive::Type)),
        };
        let result = body(Term::Bound {
            depth: 0,
            index: 0,
            sort: nir::Sort::Type,
        });
        Package {
            functions: vec![nir::program::Function {
                interface: nir::program::FunctionDeclaration {
                    source: nir::program::SourceRange { start: 0, end: 0 },
                    declaration: id(1),
                    name: "Guarded".into(),
                    phase: nir::program::FunctionPhase::Comptime,
                    signature,
                    parameters: vec![parameter(0), parameter(1)],
                    preconditions: vec![Expr {
                        source: SourceRange { start: 3, end: 4 },
                        ty: Term::Primitive(Primitive::Bool),
                        kind: K::Local(parameter(1)),
                    }],
                    inline: false,
                    pure: false,
                    is_unsafe: false,
                    type_parameters: vec![],
                    env_parameters: vec![],
                    defaults: vec![],
                    postconditions: vec![],
                    result_binding: None,
                    result_type_binder: None,
                    result_guarantees: vec![],
                    entry_snapshots: vec![],
                    requirements: vec![],
                    capabilities: vec![],
                },
                runtime_deliveries: Default::default(),
                body: Expr {
                    source: result.source,
                    ty: result.ty.clone(),
                    kind: K::Block {
                        statements: vec![
                            Expr {
                                source: result.source,
                                ty: Term::Primitive(Primitive::Void),
                                kind: K::Unit,
                            };
                            width
                        ],
                        result: Some(Box::new(result)),
                    },
                },
            }],
            ..Default::default()
        }
    }

    #[test]
    fn rejected_entry_does_not_spend_or_publish_dependent_body_preparation() {
        for width in [0, 4096] {
            let package = guarded_type_body(width);
            for (tree, instance) in [(false, false), (true, false), (false, true), (true, true)] {
                let callable = if instance {
                    Value::FunctionInstance {
                        declaration: id(1),
                        arguments: vec![],
                    }
                } else {
                    Value::Function(id(1))
                };
                let mut vm = if tree {
                    Interpreter::for_comptime_tree(&package, Budget::default())
                } else {
                    Interpreter::for_comptime(&package, Budget::default())
                }
                .unwrap();
                vm.budget.steps = 500;
                vm.effect_state.invocation = 41;
                let failure = vm
                    .invoke_value(
                        callable.clone(),
                        vec![
                            Value::Type(Term::Primitive(Primitive::U8)),
                            Value::Bool(false),
                        ],
                    )
                    .unwrap_err();
                assert_eq!(failure.kind, ErrorKind::ProgramContractRejected);
                assert_eq!(failure.source, Some(SourceRange { start: 3, end: 4 }));
                assert!(vm.input_body_cache.entries.is_empty());
                assert!(vm.input_body_cache.owned.is_empty());
                assert!(!vm.events().contains(&Event::EnterFunction(id(1))));
                assert_eq!(vm.invocation_depth, 0);
                assert_eq!(vm.effect_state.invocation, 41);
                for input in [Primitive::U8, Primitive::Bool, Primitive::U8] {
                    vm.budget = Budget::default();
                    assert_eq!(
                        vm.invoke_value(
                            callable.clone(),
                            vec![Value::Type(Term::Primitive(input)), Value::Bool(true)],
                        )
                        .unwrap(),
                        Value::Type(Term::Primitive(input)),
                    );
                }
            }
        }
    }

    #[test]
    fn instance_contracts_share_declaration_and_ordinary_type_bindings() {
        use nir::program::{Literal, MatchArm, Pattern};
        let mut package = guarded_type_body(0);
        let interface = &mut package.functions[0].interface;
        interface.type_parameters.push(id(2));
        let marker = Term::Rigid {
            declaration: id(2),
            sort: nir::Sort::Type,
        };
        let ordinary = Term::Bound {
            depth: 0,
            index: 0,
            sort: nir::Sort::Type,
        };
        let Term::Function { parameters, .. } = &mut interface.signature else {
            unreachable!()
        };
        parameters.truncate(1);
        interface.parameters.truncate(1);
        let mut predicates = Vec::new();
        for (index, ty) in [(1, marker), (2, ordinary)] {
            let ty = Term::Optional(Box::new(ty));
            let local = LocalId { owner: 1, index };
            parameters.push(nir::Parameter {
                ty: ty.clone(),
                sort: nir::Sort::Runtime,
            });
            interface.parameters.push(local);
            let source = SourceRange {
                start: index,
                end: index + 1,
            };
            let arm = |pattern, value| MatchArm {
                pattern,
                guard: None,
                body: Expr {
                    source,
                    ty: Term::Primitive(Primitive::Bool),
                    kind: K::Literal(Literal::Bool(value)),
                },
            };
            predicates.push(Expr {
                source,
                ty: Term::Primitive(Primitive::Bool),
                kind: K::Match {
                    value: Box::new(Expr {
                        source,
                        ty,
                        kind: K::Local(local),
                    }),
                    arms: vec![
                        arm(Pattern::OptionalAbsent, true),
                        arm(Pattern::Wild, false),
                    ],
                },
            });
        }
        // The ordinary input occurs before and after execution. The declaration
        // input independently controls the first exit predicate.
        interface.preconditions = vec![predicates[1].clone()];
        interface.postconditions = predicates;
        for tree in [false, true] {
            let mut vm = if tree {
                Interpreter::for_comptime_tree(&package, Budget::default())
            } else {
                Interpreter::for_comptime(&package, Budget::default())
            }
            .unwrap();
            let optional = |ty, value: Option<Value>| Value::Optional {
                ty: Term::Optional(Box::new(Term::Primitive(ty))),
                value: value.map(Box::new),
            };
            for (ty, marker, ordinary, rejection) in [
                (Primitive::U8, None, None, None),
                (
                    Primitive::U8,
                    None,
                    Some(Value::UnsignedInteger(3)),
                    Some(2),
                ),
                (Primitive::Bool, Some(Value::Char('x')), None, Some(1)),
                (Primitive::Bool, None, None, None),
            ] {
                let term = Term::Primitive(ty);
                let result = vm.invoke_instantiated(
                    id(1),
                    vec![Term::Primitive(Primitive::Char)],
                    vec![
                        Value::Type(term.clone()),
                        optional(Primitive::Char, marker),
                        optional(ty, ordinary),
                    ],
                );
                if let Some(start) = rejection {
                    let error = result.unwrap_err();
                    assert_eq!(error.kind, ErrorKind::ProgramContractRejected);
                    assert_eq!(
                        error.source,
                        Some(SourceRange {
                            start,
                            end: start + 1
                        })
                    );
                } else {
                    assert_eq!(result.unwrap(), Value::Type(term));
                }
            }
        }
    }

    #[test]
    fn unpolled_call_without_preconditions_does_not_prepare_its_body() {
        let mut package = guarded_type_body(4096);
        package.functions[0].interface.preconditions.clear();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.budget.steps = 500;
        let active = vm
            .start_invocation(
                Value::Function(id(1)),
                vec![
                    Value::Type(Term::Primitive(Primitive::Bool)),
                    Value::Bool(true),
                ],
            )
            .unwrap();
        assert!(active.to_string().contains("(need-body)"));
        drop(active);
        assert!(vm.input_body_cache.entries.is_empty());
        assert!(!vm.events().contains(&Event::EnterFunction(id(1))));
        assert_eq!(vm.invocation_depth, 0);
    }

    #[test]
    fn cancelled_entry_and_failed_body_preparation_restore_the_same_caller() {
        let package = guarded_type_body(4096);
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let arguments = || {
            vec![
                Value::Type(Term::Primitive(Primitive::Bool)),
                Value::Bool(true),
            ]
        };
        vm.budget.steps = 500;
        vm.effect_state.invocation = 41;
        let session = vm
            .start_invocation(Value::Function(id(1)), arguments())
            .unwrap();
        assert!(session.to_string().contains("(entry (next 0))"));
        drop(session);
        assert!(vm.input_body_cache.entries.is_empty());
        assert_eq!(vm.invocation_depth, 0);
        assert_eq!(vm.effect_state.invocation, 41);
        vm.budget.steps = 500;
        assert_eq!(
            vm.invoke(id(1), arguments()).unwrap_err().kind,
            ErrorKind::StepsExhausted,
        );
        assert!(vm.input_body_cache.entries.is_empty());
        assert_eq!(vm.invocation_depth, 0);
        assert_eq!(vm.effect_state.invocation, 41);
        vm.budget = Budget::default();
        assert_eq!(
            vm.invoke(id(1), arguments()).unwrap(),
            Value::Type(Term::Primitive(Primitive::Bool)),
        );
        assert_eq!(
            vm.events()
                .iter()
                .filter(|event| **event == Event::EnterFunction(id(1)))
                .count(),
            1,
        );
    }

    #[test]
    fn independent_code_is_shared_and_dependent_code_tracks_the_binding() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let constant = body(Term::Primitive(Primitive::Bool));
        let (first, independent) = vm
            .prepare_input_body(&constant, &scope(Primitive::Bool), named(1))
            .unwrap();
        assert!(independent);
        let before = vm.budget.steps;
        let (next, independent) = vm
            .prepare_input_body(&constant, &scope(Primitive::Char), named(1))
            .unwrap();
        assert!(independent && Arc::ptr_eq(&first, &next));
        assert_eq!(before, vm.budget.steps);
        let dependent = body(Term::Bound {
            depth: 0,
            index: 0,
            sort: nir::Sort::Type,
        });
        for input in [Primitive::Bool, Primitive::Char, Primitive::Bool] {
            let (prepared, independent) = vm
                .prepare_input_body(&dependent, &scope(input), named(2))
                .unwrap();
            assert!(!independent);
            assert!(matches!(&prepared.kind, K::TypeValue(ty) if *ty == Term::Primitive(input)));
        }
        assert!(matches!(
            vm.input_body_cache.entries.get(&named(2)),
            Some((Entry::Dependent(_), _))
        ));
    }
    #[test]
    fn locally_bound_callable_code_is_shared_across_invocation_inputs() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let callable = |depth| Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            result: Box::new(Term::Bound {
                depth,
                index: 0,
                sort: nir::Sort::Type,
            }),
        };
        let local = Arc::new(body(callable(0)));
        let outer = Arc::new(body(callable(1)));
        let mut original = None;
        for input in [Primitive::Bool, Primitive::Char] {
            let binding = scope(input);
            let (prepared, shared) = vm.prepare_owned_input_body(&local, &binding).unwrap();
            assert!(shared);
            assert_eq!(prepared.kind, K::TypeValue(callable(0)));
            if let Some(previous) = original.replace(prepared.clone()) {
                assert!(Arc::ptr_eq(&previous, &prepared));
            }

            let (prepared, shared) = vm.prepare_owned_input_body(&outer, &binding).unwrap();
            assert!(!shared);
            let K::TypeValue(Term::Function { result, .. }) = &prepared.kind else {
                panic!()
            };
            assert_eq!(**result, Term::Primitive(input));
        }
    }

    #[test]
    fn failed_preparation_publishes_nothing_and_eviction_keeps_owned_bodies_alive() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let invalid = body(Term::Bound {
            depth: 0,
            index: 2,
            sort: nir::Sort::Type,
        });
        assert!(
            vm.prepare_input_body(&invalid, &scope(Primitive::Bool), named(1))
                .is_err()
        );
        assert!(vm.input_body_cache.entries.is_empty());
        let valid = body(Term::Primitive(Primitive::Bool));
        vm.budget.steps = 0;
        assert_eq!(
            vm.prepare_input_body(&valid, &scope(Primitive::Bool), named(1))
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        assert!(vm.input_body_cache.entries.is_empty());
        vm.budget = Budget::default();
        let (held, _) = vm
            .prepare_input_body(&valid, &scope(Primitive::Bool), named(1))
            .unwrap();
        vm.input_body_cache
            .insert(named(2), Entry::Prepared(held.clone()), 100_000);
        assert!(!vm.input_body_cache.entries.contains_key(&named(1)));
        assert_eq!(held.to_string(), valid.to_string());
        let (again, _) = vm
            .prepare_input_body(&valid, &scope(Primitive::Bool), named(1))
            .unwrap();
        assert!(!Arc::ptr_eq(&held, &again));
        assert_eq!(held.to_string(), again.to_string());
        assert!(vm.input_body_cache.weight <= 100_000);
    }
    #[test]
    fn owned_body_identity_separates_preparation_and_does_not_pin_source() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let source = Arc::new(body(Term::Primitive(Primitive::Bool)));
        let (first, _) = vm
            .prepare_owned_input_body(&source, &scope(Primitive::Bool))
            .unwrap();
        let (second, _) = vm
            .prepare_owned_input_body(&source, &scope(Primitive::Char))
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let changed = Arc::new(body(Term::Primitive(Primitive::Char)));
        let (other, _) = vm
            .prepare_owned_input_body(&changed, &scope(Primitive::Bool))
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &other));
        assert!(matches!(
            &other.kind,
            K::TypeValue(Term::Primitive(Primitive::Char))
        ));
        let dependent = Arc::new(body(Term::Bound {
            depth: 0,
            index: 0,
            sort: nir::Sort::Type,
        }));
        for input in [Primitive::Bool, Primitive::Char, Primitive::Bool] {
            let (prepared, independent) = vm
                .prepare_owned_input_body(&dependent, &scope(input))
                .unwrap();
            assert!(!independent);
            assert!(matches!(&prepared.kind, K::TypeValue(ty) if *ty == Term::Primitive(input)));
        }
        let weak = Arc::downgrade(&source);
        drop(source);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn dependent_body_reuse_is_metered_and_failed_replacement_preserves_code() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let source = body(Term::Bound {
            depth: 0,
            index: 0,
            sort: nir::Sort::Type,
        });
        let first_scope = scope(Primitive::Bool);
        let other_scope = scope(Primitive::Char);
        assert_eq!(first_scope.signature(), other_scope.signature());
        let (first, _) = vm
            .prepare_input_body(&source, &first_scope, named(1))
            .unwrap();
        let before = vm.remaining_steps();
        let (warm, independent) = vm
            .prepare_input_body(&source, &first_scope, named(1))
            .unwrap();
        assert!(!independent && Arc::ptr_eq(&first, &warm));
        let work = before - vm.remaining_steps();
        assert!(work > 0);
        vm.budget.steps = work - 1;
        let error = vm
            .prepare_input_body(&source, &other_scope, named(1))
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::StepsExhausted);
        assert_eq!(error.source, Some(source.source));
        vm.budget = Budget::default();
        let (recovered, _) = vm
            .prepare_input_body(&source, &first_scope, named(1))
            .unwrap();
        assert!(Arc::ptr_eq(&first, &recovered));
        let previous_weight = vm.input_body_cache.weight;
        let (changed, _) = vm
            .prepare_input_body(&source, &other_scope, named(1))
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &changed));
        assert!(matches!(
            changed.kind,
            K::TypeValue(Term::Primitive(Primitive::Char))
        ));
        assert_eq!(vm.input_body_cache.weight, previous_weight);
        assert!(matches!(
            first.kind,
            K::TypeValue(Term::Primitive(Primitive::Bool))
        ));
    }

    #[test]
    fn only_body_type_uses_join_the_binding_on_cold_and_warm_calls() {
        let source = SourceRange { start: 1, end: 2 };
        let binding = |index| LocalId { owner: 1, index };
        let term = |index| Term::Bound {
            depth: 0,
            index,
            sort: nir::Sort::ComptimeValue,
        };
        let boolean = Term::Primitive(Primitive::Bool);
        let signature = Term::Function {
            parameters: vec![
                nir::Parameter {
                    sort: nir::Sort::ComptimeValue,
                    ty: boolean.clone()
                };
                2
            ],
            result: Box::new(boolean.clone()),
        };
        let mut package = guarded_type_body(0);
        let function = &mut package.functions[0];
        function.interface.signature = signature;
        function.interface.preconditions.clear();
        // One ordinary read and one use in an embedded term. Only the latter
        // changes the specialized code; both remain ordinary invocation values.
        function.body = Expr {
            source,
            ty: boolean.clone(),
            kind: K::Block {
                statements: vec![Expr {
                    source,
                    ty: boolean.clone(),
                    kind: K::Local(binding(0)),
                }],
                result: Some(Box::new(Expr {
                    source,
                    ty: boolean,
                    kind: K::StagedValue(term(1)),
                })),
            },
        };
        for tree in [false, true] {
            let mut vm = if tree {
                Interpreter::for_comptime_tree(&package, Budget::default())
            } else {
                Interpreter::for_comptime(&package, Budget::default())
            }
            .unwrap();
            let mut previous: Option<Arc<Expr>> = None;
            for (first, second) in [(true, false), (false, false), (true, true)] {
                assert_eq!(
                    vm.invoke(id(1), vec![Value::Bool(first), Value::Bool(second)])
                        .unwrap(),
                    Value::Bool(second)
                );
                let (Entry::Dependent(bound), _) = &vm.input_body_cache.entries[&named(1)] else {
                    panic!("missing dependent body")
                };
                assert_eq!(bound.binding.rebind(&term(0)).unwrap(), term(0));
                assert_eq!(bound.binding.rebind(&term(1)).unwrap(), Term::Bool(second));
                if let Some(previous) = &previous {
                    assert_eq!(Arc::ptr_eq(previous, &bound.body), !second);
                }
                previous = Some(bound.body.clone());
            }
            // A caller can change a value-reference receiver while a source
            // call is suspended. Its type input keeps the admission-time value.
            let cell = Storage::new(Value::Bool(false));
            let receiver =
                Value::ComptimeReference(value_reference::ValueReference::root(cell.clone()));
            let mut fuel = 100_000;
            let call = vm
                .admit_source_call(id(1), vec![], vec![Value::Bool(true), receiver], &mut fuel)
                .unwrap();
            cell.pointer(Term::Primitive(Primitive::Bool), vec![])
                .write(Value::Bool(true), |_| Ok(()))
                .unwrap();
            assert_eq!(call.resume(&mut fuel).unwrap(), Value::Bool(false));
            assert!(
                vm.invoke(id(1), vec![Value::Unit, Value::Bool(true)])
                    .is_err()
            );
        }
    }

    #[test]
    fn type_data_reads_use_current_values_with_shared_specialized_code() {
        let source = SourceRange { start: 1, end: 2 };
        let mut package = guarded_type_body(0);
        let function = &mut package.functions[0];
        let type_domain = Term::Primitive(Primitive::Type);
        let result = Term::Tuple(vec![type_domain.clone(); 2]);
        function.interface.signature = Term::Function {
            parameters: vec![
                nir::Parameter {
                    sort: nir::Sort::Type,
                    ty: type_domain.clone()
                };
                2
            ],
            result: Box::new(result.clone()),
        };
        function.interface.preconditions.clear();
        let observed = Term::Bound {
            depth: 0,
            index: 1,
            sort: nir::Sort::Type,
        };
        function.body = Expr {
            source,
            ty: result,
            kind: K::Tuple(vec![
                Expr {
                    source,
                    ty: type_domain,
                    kind: K::Local(function.parameters[0]),
                },
                body(observed.clone()),
            ]),
        };
        for tree in [false, true] {
            let mut vm = if tree {
                Interpreter::for_comptime_tree(&package, Budget::default()).unwrap()
            } else {
                Interpreter::for_comptime(&package, Budget::default()).unwrap()
            };
            let mut previous = None;
            for (data, element, shared) in [
                (
                    Value::Type(Term::Primitive(Primitive::Bool)),
                    Primitive::Char,
                    false,
                ),
                (
                    Value::Type(Term::Primitive(Primitive::Char)),
                    Primitive::Char,
                    true,
                ),
                (
                    Value::Description(nir::program::DescriptionLiteral::Any),
                    Primitive::Char,
                    true,
                ),
                (
                    Value::Type(Term::Primitive(Primitive::Bool)),
                    Primitive::U8,
                    false,
                ),
            ] {
                let element = Term::Primitive(element);
                let inputs = vec![data, Value::Type(element.clone())];
                assert_eq!(
                    vm.invoke(id(1), inputs.clone()).unwrap(),
                    Value::Tuple(inputs)
                );
                let (Entry::Dependent(entry), _) = &vm.input_body_cache.entries[&named(1)] else {
                    panic!("expected specialization by the second Type");
                };
                assert_eq!(entry.binding.rebind(&observed).unwrap(), element);
                if let Some(previous) = &previous {
                    assert_eq!(Arc::ptr_eq(previous, &entry.body), shared);
                }
                previous = Some(entry.body.clone());
            }
            let data = Value::Type(Term::Primitive(Primitive::Bool));
            let original = Value::Type(Term::Primitive(Primitive::Char));
            let storage = Storage::new(original.clone());
            let receiver =
                Value::ComptimeReference(value_reference::ValueReference::root(storage.clone()));
            let mut fuel = 100_000;
            let call = vm
                .admit_source_call(id(1), vec![], vec![data.clone(), receiver], &mut fuel)
                .unwrap();
            storage
                .pointer(Term::Primitive(Primitive::Type), vec![])
                .write(Value::Type(Term::Primitive(Primitive::U8)), |_| Ok(()))
                .unwrap();
            assert_eq!(
                call.resume(&mut fuel).unwrap(),
                Value::Tuple(vec![data, original])
            );
        }
    }

    #[test]
    fn value_index_bindings_use_the_same_preparation_cache() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let signature = Term::Function {
            parameters: vec![nir::Parameter {
                sort: nir::Sort::ComptimeValue,
                ty: Term::Primitive(Primitive::Bool),
            }],
            result: Box::new(Term::Primitive(Primitive::Type)),
        };
        let nominal = |argument| Term::Nominal {
            declaration: id(2),
            arguments: vec![argument],
        };
        let source = Arc::new(body(nominal(Term::Bound {
            depth: 0,
            index: 0,
            sort: nir::Sort::ComptimeValue,
        })));
        for flag in [true, false, true] {
            let binding = signature
                .prepare_function_inputs(&[Some(Term::Bool(flag))])
                .unwrap();
            let (first, _) = vm.prepare_owned_input_body(&source, &binding).unwrap();
            let (warm, independent) = vm.prepare_owned_input_body(&source, &binding).unwrap();
            assert!(!independent && Arc::ptr_eq(&first, &warm));
            assert!(matches!(&warm.kind, K::TypeValue(term) if *term == nominal(Term::Bool(flag))));
        }
    }

    #[test]
    fn abstract_invocations_do_not_publish_dependent_preparation() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let source = body(Term::Bound {
            depth: 0,
            index: 0,
            sort: nir::Sort::Type,
        });
        vm.abstract_type_parameters.insert(id(10));
        let (first, _) = vm
            .prepare_input_body(&source, &scope(Primitive::Bool), named(1))
            .unwrap();
        let (next, _) = vm
            .prepare_input_body(&source, &scope(Primitive::Bool), named(1))
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &next));
        assert!(vm.input_body_cache.entries.is_empty());
        vm.abstract_type_parameters.clear();
        let (concrete, _) = vm
            .prepare_input_body(&source, &scope(Primitive::Bool), named(1))
            .unwrap();
        let (warm, _) = vm
            .prepare_input_body(&source, &scope(Primitive::Bool), named(1))
            .unwrap();
        assert!(Arc::ptr_eq(&concrete, &warm));
    }
}
