//! Reuse prepared bodies in one immutable world, without retaining call state.
use super::*;

#[derive(Clone)]
enum Entry {
    Dependent(Arc<BoundBody>),
    Prepared(Arc<Expr>),
}
struct BoundBody {
    binding: nir::FunctionInputBinding,
    body: Arc<Expr>,
}
#[derive(Default)]
pub(super) struct Cache {
    entries: BTreeMap<DeclarationId, (Entry, u64)>,
    owned: BTreeMap<usize, (std::sync::Weak<Expr>, Entry, u64)>,
    weight: u64,
}
impl Cache {
    fn insert(&mut self, id: DeclarationId, entry: Entry, weight: u64) {
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
impl Interpreter<'_> {
    pub(super) fn prepare_input_body(
        &mut self,
        body: &Expr,
        scope: &nir::FunctionInputBinding,
        declaration: Option<DeclarationId>,
    ) -> Result<(Arc<Expr>, bool), Error> {
        self.prepare_input_body_key(body, scope, declaration, None)
    }

    pub(super) fn prepare_owned_input_body(
        &mut self,
        body: &Arc<Expr>,
        scope: &nir::FunctionInputBinding,
    ) -> Result<(Arc<Expr>, bool), Error> {
        self.prepare_input_body_key(body, scope, None, Some(body))
    }

    fn prepare_input_body_key(
        &mut self,
        body: &Expr,
        scope: &nir::FunctionInputBinding,
        declaration: Option<DeclarationId>,
        owned: Option<&Arc<Expr>>,
    ) -> Result<(Arc<Expr>, bool), Error> {
        let cached = declaration
            .and_then(|id| {
                self.input_body_cache
                    .entries
                    .get(&id)
                    .map(|(entry, _)| entry.clone())
            })
            .or_else(|| owned.and_then(|body| self.input_body_cache.owned(body)));
        if let Some(Entry::Prepared(body)) = cached {
            return Ok((body, true));
        }
        // Rewriting depends only on this checked binding and the immutable
        // source body. Equality covers all replacements and slot refinements,
        // not just the resulting signature or the positions declared as Type.
        let persistent =
            self.abstract_type_parameters.is_empty() && self.abstract_value_parameters.is_empty();
        let before_binding = self.budget.steps;
        if persistent {
            self.charge_input_binding(scope, body.source)?;
        }
        let binding_work = before_binding - self.budget.steps;
        if let Some(Entry::Dependent(previous)) = &cached {
            if persistent {
                self.charge_input_binding(&previous.binding, body.source)?;
                if previous.binding == *scope {
                    return Ok((previous.body.clone(), false));
                }
            }
        }
        let dependent = if matches!(cached, Some(Entry::Dependent(_))) {
            nir::copy_budget::charge_type_rewrite(body, &mut self.budget.steps)
                .map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(body.source)))?;
            true
        } else {
            nir::copy_budget::expression_has_contextual_types(body, &mut self.budget.steps)
                .map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(body.source)))?
        };
        let mut prepared = body.clone();
        // Retain all original rewrite/scope validation, including pattern-domain
        // completion, before publishing an independent body for later calls.
        self.specialize_input_types(&mut prepared, scope)?;
        let prepared = Arc::new(prepared);
        if declaration.is_some() || owned.is_some() {
            let entry = if !dependent || persistent {
                let before = self.budget.steps;
                nir::copy_budget::charge_type_rewrite(&prepared, &mut self.budget.steps)
                    .map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(body.source)))?;
                let weight = (before - self.budget.steps)
                    .saturating_add(binding_work)
                    .saturating_add(1);
                let entry = if dependent {
                    Entry::Dependent(Arc::new(BoundBody {
                        binding: scope.clone(),
                        body: prepared.clone(),
                    }))
                } else {
                    Entry::Prepared(prepared.clone())
                };
                Some((entry, weight))
            } else {
                None
            };
            if let Some((entry, weight)) = entry {
                if let Some(id) = declaration {
                    self.input_body_cache.insert(id, entry, weight);
                } else if let Some(body) = owned {
                    self.input_body_cache.insert_owned(body, entry, weight);
                }
            }
        }
        Ok((prepared, !dependent))
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
    #[test]
    fn independent_code_is_shared_and_dependent_code_tracks_the_binding() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let constant = body(Term::Primitive(Primitive::Bool));
        let (first, independent) = vm
            .prepare_input_body(&constant, &scope(Primitive::Bool), Some(id(1)))
            .unwrap();
        assert!(independent);
        let before = vm.budget.steps;
        let (next, independent) = vm
            .prepare_input_body(&constant, &scope(Primitive::Char), Some(id(1)))
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
                .prepare_input_body(&dependent, &scope(input), Some(id(2)))
                .unwrap();
            assert!(!independent);
            assert!(matches!(&prepared.kind, K::TypeValue(ty) if *ty == Term::Primitive(input)));
        }
        assert!(matches!(
            vm.input_body_cache.entries.get(&id(2)),
            Some((Entry::Dependent(_), _))
        ));
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
            vm.prepare_input_body(&invalid, &scope(Primitive::Bool), Some(id(1)))
                .is_err()
        );
        assert!(vm.input_body_cache.entries.is_empty());
        let valid = body(Term::Primitive(Primitive::Bool));
        vm.budget.steps = 0;
        assert_eq!(
            vm.prepare_input_body(&valid, &scope(Primitive::Bool), Some(id(1)))
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
        assert!(vm.input_body_cache.entries.is_empty());
        vm.budget = Budget::default();
        let (held, _) = vm
            .prepare_input_body(&valid, &scope(Primitive::Bool), Some(id(1)))
            .unwrap();
        vm.input_body_cache
            .insert(id(2), Entry::Prepared(held.clone()), 100_000);
        assert!(!vm.input_body_cache.entries.contains_key(&id(1)));
        assert_eq!(held.to_string(), valid.to_string());
        let (again, _) = vm
            .prepare_input_body(&valid, &scope(Primitive::Bool), Some(id(1)))
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
            .prepare_input_body(&source, &first_scope, Some(id(1)))
            .unwrap();
        let before = vm.remaining_steps();
        let (warm, independent) = vm
            .prepare_input_body(&source, &first_scope, Some(id(1)))
            .unwrap();
        assert!(!independent && Arc::ptr_eq(&first, &warm));
        let work = before - vm.remaining_steps();
        assert!(work > 0);
        vm.budget.steps = work - 1;
        let error = vm
            .prepare_input_body(&source, &other_scope, Some(id(1)))
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::StepsExhausted);
        assert_eq!(error.source, Some(source.source));
        vm.budget = Budget::default();
        let (recovered, _) = vm
            .prepare_input_body(&source, &first_scope, Some(id(1)))
            .unwrap();
        assert!(Arc::ptr_eq(&first, &recovered));
        let previous_weight = vm.input_body_cache.weight;
        let (changed, _) = vm
            .prepare_input_body(&source, &other_scope, Some(id(1)))
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
            .prepare_input_body(&source, &scope(Primitive::Bool), Some(id(1)))
            .unwrap();
        let (next, _) = vm
            .prepare_input_body(&source, &scope(Primitive::Bool), Some(id(1)))
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &next));
        assert!(vm.input_body_cache.entries.is_empty());
        vm.abstract_type_parameters.clear();
        let (concrete, _) = vm
            .prepare_input_body(&source, &scope(Primitive::Bool), Some(id(1)))
            .unwrap();
        let (warm, _) = vm
            .prepare_input_body(&source, &scope(Primitive::Bool), Some(id(1)))
            .unwrap();
        assert!(Arc::ptr_eq(&concrete, &warm));
    }
}
