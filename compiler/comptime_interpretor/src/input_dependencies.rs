//! Bounded read-only input observation for invocation domain preparation.
//! Observed identities are dependencies, not authority to transport or use values.
use super::*;

impl Interpreter<'_> {
    pub(super) fn observe_input_dependencies(
        &mut self,
        term: &Term,
        source: SourceRange,
    ) -> Result<nir::copy_budget::TermInputs, Error> {
        nir::copy_budget::term_inputs(term, &mut self.budget.steps).map_err(|failure| {
            use nir::copy_budget::{
                CopyBudgetExhausted as Limit, InputObservationError as Failure,
            };
            self.failure(
                match failure {
                    Failure::Limit(Limit::Steps) => ErrorKind::StepsExhausted,
                    Failure::Limit(Limit::Depth) => ErrorKind::DepthExhausted,
                    Failure::Scope(_) => ErrorKind::Type,
                },
                Some(source),
            )
        })
    }

    pub(super) fn instantiate_selected_domain<'a>(
        &mut self,
        ty: &Term,
        selected: impl Iterator<Item = (&'a nir::DeclarationParameter, &'a Term)>,
        source: SourceRange,
    ) -> Result<Term, Error> {
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut remaining = before;
        let result = nir::type_computations::instantiate_bindings_bounded(
            ty,
            selected.map(|(parameter, value)| (*parameter, value)),
            &[],
            &mut remaining,
        );
        self.budget.steps -= (before - remaining) as u64;
        result.map_err(|error| {
            self.failure(
                if matches!(error, nir::type_computations::Error::Budget) {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                },
                Some(source),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::Sort;

    const SOURCE: SourceRange = SourceRange { start: 13, end: 21 };
    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 91, local }
    }
    fn rigid(local: u32, sort: Sort) -> Term {
        Term::Rigid {
            declaration: id(local),
            sort,
        }
    }
    fn bound(depth: u32, index: usize) -> Term {
        Term::Bound {
            depth,
            index,
            sort: Sort::Type,
        }
    }
    fn domain() -> Term {
        Term::NormalForm {
            head: id(90),
            sort: Sort::Type,
            arguments: vec![
                Term::ClosureSnapshot {
                    factory: id(91),
                    arguments: vec![rigid(1, Sort::Type)],
                },
                Term::ComptimeFunction(Box::new(Term::Function {
                    parameters: vec![nir::Parameter {
                        sort: Sort::Type,
                        ty: Term::Primitive(Primitive::Type),
                    }],
                    result: Box::new(Term::Tuple(vec![bound(0, 0), rigid(1, Sort::Type)])),
                })),
            ],
        }
    }

    #[test]
    fn domain_observation_preserves_opaque_identities_and_nested_scopes() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let term = Term::Tuple(vec![domain(), bound(0, 3)]);
        let before = term.to_string();
        let inputs = vm.observe_input_dependencies(&term, SOURCE).unwrap();
        assert_eq!(inputs.declarations, BTreeMap::from([(id(1), Sort::Type)]));
        assert_eq!(inputs.parameters, BTreeMap::from([(3, Sort::Type)]));
        // Heads and factories carry identity, not declaration-input demand.
        assert!(!inputs.declarations.contains_key(&id(90)));
        assert!(!inputs.declarations.contains_key(&id(91)));
        assert_eq!(term.to_string(), before);
    }

    #[test]
    fn observation_failures_keep_resource_categories_source_and_recovery() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut deep = rigid(1, Sort::Type);
        for _ in 0..nir::copy_budget::MAX_COPY_DEPTH {
            deep = Term::Optional(Box::new(deep));
        }
        let conflicting = Term::NormalForm {
            head: id(90),
            sort: Sort::Type,
            arguments: vec![rigid(1, Sort::Type), rigid(1, Sort::ComptimeValue)],
        };
        for (term, steps, kind) in [
            (domain(), 0, ErrorKind::StepsExhausted),
            (deep, 100_000, ErrorKind::DepthExhausted),
            (conflicting, 100_000, ErrorKind::Type),
        ] {
            vm.budget.steps = steps;
            let error = vm.observe_input_dependencies(&term, SOURCE).unwrap_err();
            assert_eq!(error.kind, kind);
            assert_eq!(error.source, Some(SOURCE));
            if steps > 0 {
                assert!(vm.budget.steps < steps);
            }
            vm.budget = Budget::default();
            assert_eq!(
                vm.observe_input_dependencies(&domain(), SOURCE)
                    .unwrap()
                    .declarations,
                BTreeMap::from([(id(1), Sort::Type)])
            );
        }
    }

    #[test]
    fn domain_substitution_selects_only_demanded_values_and_recovers_from_limits() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let ty = domain();
        let boolean = Term::Primitive(Primitive::Bool);
        let expected = Term::NormalForm {
            head: id(90),
            sort: Sort::Type,
            arguments: vec![
                Term::ClosureSnapshot {
                    factory: id(91),
                    arguments: vec![boolean.clone()],
                },
                Term::ComptimeFunction(Box::new(Term::Function {
                    parameters: vec![nir::Parameter {
                        sort: Sort::Type,
                        ty: Term::Primitive(Primitive::Type),
                    }],
                    result: Box::new(Term::Tuple(vec![bound(0, 0), boolean.clone()])),
                })),
            ],
        };
        let mut bindings = input_bindings::Bindings::structural([id(1), id(2)]);
        let mut unselected = boolean.clone();
        for _ in 0..24 {
            unselected = Term::Optional(Box::new(unselected));
        }
        for (local, value) in [(2, unselected), (1, boolean)] {
            bindings
                .bind(
                    &mut vm,
                    (local - 1) as usize,
                    nir::DeclarationParameter {
                        declaration: id(local),
                        sort: Sort::Type,
                    },
                    &Term::Primitive(Primitive::Type),
                    Value::Type(value),
                    SOURCE,
                )
                .unwrap();
        }
        let mut failed = false;
        let mut succeeded = false;
        for steps in [0, 4, 16, 64, 256, 10_000] {
            vm.budget.steps = steps;
            match bindings.instantiate(&mut vm, &ty, SOURCE) {
                Ok(result) => {
                    assert_eq!(result, expected);
                    succeeded = true;
                }
                Err(error) => {
                    assert_eq!(error.kind, ErrorKind::StepsExhausted);
                    assert_eq!(error.source, Some(SOURCE));
                    failed = true;
                    vm.budget = Budget::default();
                    assert_eq!(
                        bindings.instantiate(&mut vm, &ty, SOURCE).unwrap(),
                        expected
                    );
                }
            }
        }
        assert!(failed && succeeded);
        assert_eq!(ty, domain());
    }
}
