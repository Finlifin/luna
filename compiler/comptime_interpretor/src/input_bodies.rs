//! Cache only input-independent prepared bodies of immutable package declarations.
use super::*;

#[derive(Clone)]
enum Entry {
    Dependent,
    Prepared(Arc<Expr>),
}
#[derive(Default)]
pub(super) struct Cache {
    entries: BTreeMap<DeclarationId, Entry>,
    weight: u64,
}
impl Cache {
    fn insert(&mut self, id: DeclarationId, entry: Entry, weight: u64) {
        const LIMIT: u64 = 100_000;
        if weight > LIMIT {
            return;
        }
        if self.weight.saturating_add(weight) > LIMIT {
            self.entries.clear();
            self.weight = 0;
        }
        self.weight += weight;
        self.entries.insert(id, entry);
    }
}
impl Interpreter<'_> {
    pub(super) fn prepare_input_body(
        &mut self,
        body: &Expr,
        scope: &nir::FunctionInputBinding,
        declaration: Option<DeclarationId>,
    ) -> Result<(Arc<Expr>, bool), Error> {
        let cached = declaration.and_then(|id| self.input_body_cache.entries.get(&id).cloned());
        if let Some(Entry::Prepared(body)) = cached {
            return Ok((body, true));
        }
        let dependent = if matches!(cached, Some(Entry::Dependent)) {
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
        if let Some(id) = declaration {
            if dependent {
                if cached.is_none() {
                    self.input_body_cache.insert(id, Entry::Dependent, 1);
                }
            } else {
                let before = self.budget.steps;
                nir::copy_budget::charge_type_rewrite(&prepared, &mut self.budget.steps)
                    .map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(body.source)))?;
                self.input_body_cache.insert(
                    id,
                    Entry::Prepared(prepared.clone()),
                    (before - self.budget.steps).saturating_add(1),
                );
            }
        }
        Ok((prepared, !dependent))
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
    fn independent_bodies_share_code_but_dependent_values_stay_call_local() {
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
            Some(Entry::Dependent)
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
        vm.input_body_cache.insert(id(2), Entry::Dependent, 100_000);
        assert!(!vm.input_body_cache.entries.contains_key(&id(1)));
        assert_eq!(held.to_string(), valid.to_string());
        let (again, _) = vm
            .prepare_input_body(&valid, &scope(Primitive::Bool), Some(id(1)))
            .unwrap();
        assert!(!Arc::ptr_eq(&held, &again));
        assert_eq!(held.to_string(), again.to_string());
        assert!(vm.input_body_cache.weight <= 100_000);
    }
}
