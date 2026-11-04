//! Invocation-local completed input bindings shared by input plans.
use super::*;

/// Substitutions follow completion order; final arguments follow declaration order.
/// Slots store indices rather than cloning potentially large comptime terms.
pub(super) struct Bindings {
    slots: Vec<Option<usize>>,
    pub parameters: Vec<nir::DeclarationParameter>,
    pub arguments: Vec<Term>,
    domains: Vec<Term>,
}
impl Bindings {
    pub fn new(count: usize) -> Self {
        Self {
            slots: vec![None; count],
            parameters: vec![],
            arguments: vec![],
            domains: vec![],
        }
    }

    /// Publish a completion only after both the domain copy and value transport
    /// succeed. A missing slot never contains a fabricated declaration value.
    pub fn bind(
        &mut self,
        vm: &mut Interpreter<'_>,
        slot: usize,
        parameter: nir::DeclarationParameter,
        domain: &Term,
        value: Value,
        source: SourceRange,
    ) -> Result<(), Error> {
        if slot >= self.slots.len() || self.contains(slot) {
            return Err(vm.failure(ErrorKind::Type, Some(source)));
        }
        let before = usize::try_from(vm.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let domain = nir::copy_budget::clone_term(domain, &mut fuel);
        vm.budget.steps -= (before - fuel) as u64;
        let domain = domain.map_err(|failure| {
            vm.failure(
                match failure {
                    nir::copy_budget::CopyBudgetExhausted::Steps => ErrorKind::StepsExhausted,
                    nir::copy_budget::CopyBudgetExhausted::Depth => ErrorKind::DepthExhausted,
                },
                Some(source),
            )
        })?;
        let term = vm.residual_input_term(value, source)?;
        self.slots[slot] = Some(self.arguments.len());
        self.parameters.push(parameter);
        self.domains.push(domain);
        self.arguments.push(term);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.arguments.len()
    }

    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    pub fn completed(&self) -> impl Iterator<Item = (&nir::DeclarationParameter, &Term, &Term)> {
        // Domain admission follows declaration order even when execution and
        // completion follow a different source order. Callers charge the scan.
        self.slots.iter().filter_map(|index| {
            index.map(|index| {
                (
                    &self.parameters[index],
                    &self.domains[index],
                    &self.arguments[index],
                )
            })
        })
    }
    // NF substitution policy. Nominal consumers retain their evidence-aware
    // admission path; sharing completed storage does not confer that evidence.
    pub fn instantiate(
        &self,
        vm: &mut Interpreter<'_>,
        ty: &Term,
        source: SourceRange,
    ) -> Result<Term, Error> {
        vm.integer_operation(Some(source), |budget| budget.consume(self.parameters.len()))?;
        if self
            .parameters
            .iter()
            .zip(&self.arguments)
            .all(|(p, a)| p.sort == a.sort())
        {
            return vm.instantiate_contextual_domain(ty, &self.parameters, &self.arguments, source);
        }
        let before = usize::try_from(vm.budget.steps).unwrap_or(usize::MAX);
        let mut remaining = before;
        let ids = self
            .parameters
            .iter()
            .map(|p| p.declaration)
            .collect::<Vec<_>>();
        let result = nir::DeclarationSubstitution::bounded(
            &ids,
            &self.domains,
            &self.arguments,
            &[],
            &mut remaining,
        )
        .and_then(|substitution| substitution.instantiate_bounded(ty, &mut remaining));
        vm.budget.steps -= (before - remaining) as u64;
        result.map_err(|error| {
            vm.failure(
                if matches!(error, nir::type_computations::Error::Budget) {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                },
                Some(source),
            )
        })
    }
    pub fn specialize(
        &self,
        vm: &mut Interpreter<'_>,
        input: &mut nir::program::NormalFormArgument,
    ) -> Result<(), Error> {
        let source = input.source();
        let mut rewrite = |ty: &Term| self.instantiate(vm, ty, source);
        for pattern in input.patterns_mut() {
            pattern.rewrite_types(&mut rewrite)?;
        }
        for expression in input.expressions_mut() {
            expression.rewrite_types(&mut rewrite)?;
        }
        Ok(())
    }
    pub fn contains(&self, slot: usize) -> bool {
        self.slots.get(slot).is_some_and(Option::is_some)
    }
    pub fn finish(self) -> Option<Vec<Term>> {
        if self
            .slots
            .iter()
            .enumerate()
            .all(|(slot, index)| *index == Some(slot))
        {
            return Some(self.arguments);
        }
        let order = self.slots.into_iter().collect::<Option<Vec<_>>>()?;
        let mut terms: Vec<_> = self.arguments.into_iter().map(Some).collect();
        order
            .into_iter()
            .map(|index| terms.get_mut(index)?.take())
            .collect()
    }
}
impl fmt::Display for Bindings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(input-bindings")?;
        for (slot, index) in self.slots.iter().enumerate() {
            write!(f, " (slot {slot}")?;
            if let Some(index) = index {
                write!(
                    f,
                    " (declaration {}) (value {})",
                    self.parameters[*index].declaration, self.arguments[*index]
                )?;
            } else {
                write!(f, " pending")?;
            }
            write!(f, ")")?;
        }
        write!(f, ")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: SourceRange = SourceRange { start: 3, end: 9 };
    fn parameter() -> nir::DeclarationParameter {
        nir::DeclarationParameter {
            declaration: DeclarationId {
                package: 92,
                local: 1,
            },
            sort: nir::Sort::Type,
        }
    }

    #[test]
    fn exhausted_completion_never_publishes_and_can_retry() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let domain = Term::Primitive(Primitive::Type);
        let value = Term::Optional(Box::new(Term::Primitive(Primitive::U8)));
        let mut failures = 0;
        let mut successes = 0;
        for steps in 0..32 {
            let mut bindings = Bindings::new(1);
            vm.budget.steps = steps;
            match bindings.bind(
                &mut vm,
                0,
                parameter(),
                &domain,
                Value::Type(value.clone()),
                SOURCE,
            ) {
                Ok(()) => successes += 1,
                Err(error) => {
                    assert_eq!(error.kind, ErrorKind::StepsExhausted);
                    failures += 1;
                    assert_eq!(bindings.len(), 0);
                    assert!(!bindings.contains(0));
                    assert!(bindings.parameters.is_empty());
                    assert!(bindings.domains.is_empty());
                    assert_eq!(bindings.to_string(), "(input-bindings (slot 0 pending))");
                    vm.budget = Budget::default();
                    bindings
                        .bind(
                            &mut vm,
                            0,
                            parameter(),
                            &domain,
                            Value::Type(value.clone()),
                            SOURCE,
                        )
                        .unwrap();
                }
            }
            assert_eq!(bindings.finish(), Some(vec![value.clone()]));
        }
        assert!(failures > 1 && successes > 0);
    }

    #[test]
    fn unauthorized_transport_preserves_missing_slots_and_prior_completions() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let domain = Term::Primitive(Primitive::Type);
        let mut bindings = Bindings::new(2);
        bindings
            .bind(
                &mut vm,
                1,
                parameter(),
                &domain,
                Value::Type(domain.clone()),
                SOURCE,
            )
            .unwrap();
        let before = bindings.to_string();
        assert!(
            bindings
                .bind(
                    &mut vm,
                    0,
                    parameter(),
                    &domain,
                    Value::TypeParameter(parameter().declaration),
                    SOURCE
                )
                .is_err()
        );
        assert_eq!(bindings.to_string(), before);
        assert_eq!(bindings.len(), 1);
        assert!(bindings.finish().is_none());
    }
}
