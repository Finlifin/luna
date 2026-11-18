//! Invocation-local completed input bindings shared by input plans.
use super::*;

/// A changed sort needs either a structural substitution or declaration-domain
/// admission. This policy belongs to the consumer, not to a completed value.
#[derive(Clone, Copy)]
enum SortChange {
    Substitute,
    Admit,
}
impl fmt::Display for SortChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Substitute => "substitute",
            Self::Admit => "admit",
        })
    }
}

/// Substitutions follow completion order; final arguments follow declaration order.
/// Slots store indices rather than cloning potentially large comptime terms.
pub(super) struct Bindings {
    positions: BTreeMap<DeclarationId, usize>,
    slots: Vec<Option<usize>>,
    changed_sort: bool,
    parameters: Vec<nir::DeclarationParameter>,
    arguments: Vec<Term>,
    sort_change: SortChange,
    domains: Vec<Term>,
    descriptions: BTreeMap<usize, (DeclarationId, nir::described_applications::DataArgument)>,
}
impl Bindings {
    /// NF heads admit the complete application separately; substitution does
    /// not confer declaration-domain evidence on the stored values.
    pub fn structural(ids: impl IntoIterator<Item = DeclarationId>) -> Self {
        Self::new(ids, SortChange::Substitute)
    }

    /// Nominal/alias inputs require current-world evidence when an input changes
    /// a declaration binder's sort. Neither transport nor shared storage grants it.
    pub fn admitted(ids: impl IntoIterator<Item = DeclarationId>) -> Self {
        Self::new(ids, SortChange::Admit)
    }

    fn new(ids: impl IntoIterator<Item = DeclarationId>, sort_change: SortChange) -> Self {
        // Head and nominal contract validation establish unique binder identities.
        let positions: BTreeMap<_, _> = ids
            .into_iter()
            .enumerate()
            .map(|(slot, id)| (id, slot))
            .collect();
        Self {
            sort_change,
            slots: vec![None; positions.len()],
            positions,
            changed_sort: false,
            parameters: vec![],
            arguments: vec![],
            domains: vec![],
            descriptions: BTreeMap::new(),
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
        if self.positions.get(&parameter.declaration) != Some(&slot) || self.contains(slot) {
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
        let term = if matches!(self.sort_change, SortChange::Substitute) {
            let data = vm.freeze_normal_argument(&value, source)?;
            if !data.is_concrete() {
                let expected = self.instantiate(vm, &domain, source)?;
                vm.check_value_type(&value, &expected, Some(source))?;
                self.descriptions
                    .insert(slot, (parameter.declaration, data));
                return Ok(());
            }
            data.into_concrete().expect("concrete argument")
        } else {
            vm.residual_input_term(value, source)?
        };
        self.changed_sort |= parameter.sort != term.sort();
        self.slots[slot] = Some(self.arguments.len());
        self.parameters.push(parameter);
        self.domains.push(domain);
        self.arguments.push(term);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.arguments.len() + self.descriptions.len()
    }

    fn completed(&self) -> impl Iterator<Item = (&nir::DeclarationParameter, &Term, &Term)> {
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
    pub fn instantiate(
        &self,
        vm: &mut Interpreter<'_>,
        ty: &Term,
        source: SourceRange,
    ) -> Result<Term, Error> {
        if !self.changed_sort || !self.descriptions.is_empty() {
            let inputs = vm.observe_input_dependencies(ty, source)?;
            vm.integer_operation(Some(source), |budget| {
                budget.consume(inputs.declarations.len())
            })?;
            let demanded = inputs
                .declarations
                .keys()
                .filter_map(|id| self.positions.get(id));
            if !self.descriptions.is_empty()
                && demanded
                    .clone()
                    .any(|slot| self.descriptions.contains_key(slot))
            {
                return Err(vm.failure(ErrorKind::AbstractValueUnresolved, Some(source)));
            }
            if !self.changed_sort {
                let selected = demanded
                    .filter_map(|&slot| self.slots[slot])
                    .map(|index| (&self.parameters[index], &self.arguments[index]));
                return vm.instantiate_selected_domain(ty, selected, source);
            }
        }
        // A changed sort needs the declaration domains as evidence, including
        // completed inputs that are absent from the term being rewritten.
        vm.integer_operation(Some(source), |budget| budget.consume(self.parameters.len()))?;
        let before = usize::try_from(vm.budget.steps).unwrap_or(usize::MAX);
        let mut remaining = before;
        let result = match self.sort_change {
            SortChange::Substitute => {
                let ids = self
                    .parameters
                    .iter()
                    .map(|p| p.declaration)
                    .collect::<Vec<_>>();
                nir::DeclarationSubstitution::bounded(
                    &ids,
                    &self.domains,
                    &self.arguments,
                    &[],
                    &mut remaining,
                )
                .and_then(|substitution| substitution.instantiate_bounded(ty, &mut remaining))
                .map_err(|error| {
                    if matches!(error, nir::type_computations::Error::Budget) {
                        ErrorKind::StepsExhausted
                    } else {
                        ErrorKind::Type
                    }
                })
            }
            SortChange::Admit => self.instantiate_admitted(vm, ty, &mut remaining),
        };
        vm.budget.steps -= (before - remaining) as u64;
        result.map_err(|kind| vm.failure(kind, Some(source)))
    }

    fn instantiate_admitted(
        &self,
        vm: &Interpreter<'_>,
        ty: &Term,
        remaining: &mut usize,
    ) -> Result<Term, ErrorKind> {
        let result = (|| -> Result<Term, nir::NominalAdmissionError> {
            *remaining = remaining
                .checked_sub(self.slots.len())
                .ok_or(nir::NominalAdmissionError::Budget)?;
            let mut ids = Vec::new();
            let mut domains = Vec::new();
            let mut arguments = Vec::new();
            for (parameter, domain, argument) in self.completed() {
                nir::type_computations::charge_term(domain, remaining)?;
                nir::type_computations::charge_term(argument, remaining)?;
                ids.push(parameter.declaration);
                domains.push(domain.clone());
                arguments.push(argument.clone());
            }
            let admitted = nir::admit_declaration_arguments_with_evidence_in(
                &ids,
                &domains,
                &arguments,
                remaining,
                |ty, variant, fuel| vm.query_normal_value_fields(ty, variant, fuel),
                |value, domain, _| {
                    matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue}
                    if vm.abstract_value_parameters.get(declaration) == Some(domain))
                },
                vm.target,
            )?;
            admitted.instantiate(ty, remaining).map_err(Into::into)
        })();
        result.map_err(|error| match error {
            nir::NominalAdmissionError::Budget => ErrorKind::StepsExhausted,
            nir::NominalAdmissionError::Unresolved(_) => ErrorKind::TypeContractUnresolved,
            nir::NominalAdmissionError::Invalid(_) => ErrorKind::Type,
        })
    }

    /// Waiting slots belong to the declaration telescope. Substitution may
    /// introduce equally numbered caller binders into the expected domain.
    pub fn pending_inputs(
        &self,
        vm: &mut Interpreter<'_>,
        domain: &Term,
        source: SourceRange,
    ) -> Result<BTreeSet<usize>, Error> {
        let dependencies = vm.observe_input_dependencies(domain, source)?;
        vm.integer_operation(Some(source), |budget| {
            budget.consume(dependencies.declarations.len())
        })?;
        Ok(dependencies
            .declarations
            .keys()
            .filter_map(|id| self.positions.get(id).copied())
            .filter(|&slot| !self.contains(slot))
            .collect())
    }

    pub fn specialized(
        &self,
        vm: &mut Interpreter<'_>,
        template: &nir::program::NormalFormArgument,
    ) -> Result<nir::program::NormalFormArgument, Error> {
        let source = template.source();
        nir::copy_budget::charge_argument_type_rewrite(template, &mut vm.budget.steps).map_err(
            |limit| {
                vm.failure(
                    match limit {
                        nir::copy_budget::CopyBudgetExhausted::Steps => ErrorKind::StepsExhausted,
                        nir::copy_budget::CopyBudgetExhausted::Depth => ErrorKind::DepthExhausted,
                    },
                    Some(source),
                )
            },
        )?;
        let mut input = template.clone();
        let mut rewrite = |ty: &Term| self.instantiate(vm, ty, source);
        for pattern in input.patterns_mut() {
            pattern.rewrite_types(&mut rewrite)?;
        }
        for expression in input.expressions_mut() {
            expression.rewrite_types(&mut rewrite)?;
        }
        Ok(input)
    }
    pub fn contains(&self, slot: usize) -> bool {
        self.slots.get(slot).is_some_and(Option::is_some) || self.descriptions.contains_key(&slot)
    }
    pub fn finish_normal(mut self) -> Option<Vec<nir::described_applications::DataArgument>> {
        use nir::described_applications::DataArgument;
        if self.descriptions.is_empty() {
            return self
                .finish()
                .map(|args| args.into_iter().map(DataArgument::concrete).collect());
        }
        let mut terms = self.arguments.into_iter().map(Some).collect::<Vec<_>>();
        self.slots
            .into_iter()
            .enumerate()
            .map(|(slot, index)| {
                if let Some((_, data)) = self.descriptions.remove(&slot) {
                    Some(data)
                } else {
                    terms.get_mut(index?)?.take().map(DataArgument::concrete)
                }
            })
            .collect()
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
        write!(f, "(input-bindings (sort-change {})", self.sort_change)?;
        for (slot, index) in self.slots.iter().enumerate() {
            write!(f, " (slot {slot}")?;
            if let Some((id, data)) = self.descriptions.get(&slot) {
                write!(f, " (declaration {id}) {data}")?;
            } else if let Some(index) = index {
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

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 92, local }
    }
    fn rigid(local: u32, sort: nir::Sort) -> Term {
        Term::Rigid {
            declaration: id(local),
            sort,
        }
    }

    #[test]
    fn domain_observation_preserves_pending_slots_and_recovers_from_exhaustion() {
        let package = Package::default();
        for make in [Bindings::structural, Bindings::admitted] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut bindings = make([id(1), id(2)]);
            let domain = rigid(2, nir::Sort::ComptimeValue);
            let observe = |bindings: &Bindings, vm: &mut Interpreter<'_>| {
                let awaiting = bindings.pending_inputs(vm, &domain, SOURCE)?;
                let expected = bindings.instantiate(vm, &domain, SOURCE)?;
                Ok::<_, Error>((expected, awaiting))
            };
            let pending = observe(&bindings, &mut vm).unwrap();
            assert_eq!(pending.0, domain);
            assert_eq!(pending.1, BTreeSet::from([1]));
            // The value parameter becomes Type-sorted only through its actual
            // domain input. Recording that input alone does not finish slot 1.
            bindings
                .bind(
                    &mut vm,
                    0,
                    parameter(),
                    &Term::Primitive(Primitive::Type),
                    Value::Type(Term::Primitive(Primitive::Type)),
                    SOURCE,
                )
                .unwrap();
            let pending = observe(&bindings, &mut vm).unwrap();
            assert_eq!(pending.0, domain);
            assert_eq!(pending.1, BTreeSet::from([1]));
            bindings
                .bind(
                    &mut vm,
                    1,
                    nir::DeclarationParameter {
                        declaration: id(2),
                        sort: nir::Sort::ComptimeValue,
                    },
                    &rigid(1, nir::Sort::Type),
                    Value::Type(Term::Primitive(Primitive::U8)),
                    SOURCE,
                )
                .unwrap();
            let snapshot = bindings.to_string();
            let mut exhausted = 0;
            let mut completed = 0;
            for steps in 0..256 {
                vm.budget.steps = steps;
                match observe(&bindings, &mut vm) {
                    Ok(domain) => {
                        assert_eq!(domain.0, Term::Primitive(Primitive::U8));
                        assert!(domain.1.is_empty());
                        completed += 1;
                    }
                    Err(failure) => {
                        assert_eq!(failure.kind, ErrorKind::StepsExhausted);
                        assert_eq!(failure.source, Some(SOURCE));
                        exhausted += 1;
                        // Recovery reuses the completed values, with no input
                        // re-execution and no sticky failed domain observation.
                        vm.budget = Budget::default();
                        let domain = observe(&bindings, &mut vm).unwrap();
                        assert_eq!(domain.0, Term::Primitive(Primitive::U8));
                        assert!(domain.1.is_empty());
                    }
                }
                assert_eq!(bindings.to_string(), snapshot);
            }
            assert!(exhausted > 0 && completed > 0);
            assert_eq!(
                bindings.finish(),
                Some(vec![
                    Term::Primitive(Primitive::Type),
                    Term::Primitive(Primitive::U8),
                ])
            );
        }
    }

    #[test]
    fn sparse_domain_queries_do_not_scan_unrelated_slots_or_completions() {
        let package = Package::default();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let ty = Term::Primitive(Primitive::Type);
        let value = Term::Primitive(Primitive::Char);
        let domain = Term::Tuple(vec![rigid(0, nir::Sort::Type), rigid(1, nir::Sort::Type)]);
        let run = |vm: &mut Interpreter<'_>, width: u32| {
            let mut bindings = Bindings::structural((0..width).map(id));
            for slot in (0..width).rev().filter(|slot| *slot != 1) {
                bindings
                    .bind(
                        vm,
                        slot as usize,
                        nir::DeclarationParameter {
                            declaration: id(slot),
                            sort: nir::Sort::Type,
                        },
                        &ty,
                        Value::Type(value.clone()),
                        SOURCE,
                    )
                    .unwrap();
            }
            // Charge the queries separately from building the invocation.
            let before = vm.budget.steps;
            assert_eq!(
                bindings.pending_inputs(vm, &domain, SOURCE).unwrap(),
                BTreeSet::from([1])
            );
            assert_eq!(
                bindings.instantiate(vm, &domain, SOURCE).unwrap(),
                Term::Tuple(vec![value.clone(), rigid(1, nir::Sort::Type)])
            );
            before - vm.budget.steps
        };
        let narrow = run(&mut vm, 2);
        assert_eq!(run(&mut vm, 512), narrow);
    }

    #[test]
    fn sort_change_admission_uses_the_receiving_world_without_retaining_authority() {
        let package = Package::default();
        let ty = Term::Primitive(Primitive::Type);
        let boolean = Term::Primitive(Primitive::Bool);
        let mut sender = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        sender
            .authorize_abstract_value_parameters(&[(id(4), boolean.clone())])
            .unwrap();
        let mut bindings = Bindings::admitted([id(1), id(2), id(3)]);
        // Completion order differs from declaration order. The abstract value
        // was authorized for transport by the sender, not by future receivers.
        bindings
            .bind(
                &mut sender,
                2,
                nir::DeclarationParameter {
                    declaration: id(3),
                    sort: nir::Sort::ComptimeValue,
                },
                &boolean,
                Value::AbstractValue {
                    declaration: id(4),
                    ty: boolean.clone(),
                },
                SOURCE,
            )
            .unwrap();
        bindings
            .bind(
                &mut sender,
                0,
                parameter(),
                &ty,
                Value::Type(ty.clone()),
                SOURCE,
            )
            .unwrap();
        bindings
            .bind(
                &mut sender,
                1,
                nir::DeclarationParameter {
                    declaration: id(2),
                    sort: nir::Sort::ComptimeValue,
                },
                &rigid(1, nir::Sort::Type),
                Value::Type(Term::Primitive(Primitive::U8)),
                SOURCE,
            )
            .unwrap();
        let snapshot = bindings.to_string();
        let domain = rigid(2, nir::Sort::ComptimeValue);
        for authorized in [false, true, false, true] {
            let mut receiver = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            if authorized {
                receiver
                    .authorize_abstract_value_parameters(&[(id(4), boolean.clone())])
                    .unwrap();
            }
            let observed = bindings.instantiate(&mut receiver, &domain, SOURCE);
            if authorized {
                let observed = observed.unwrap();
                assert_eq!(observed, Term::Primitive(Primitive::U8));
            } else {
                let failure = observed
                    .err()
                    .expect("transport is not admission authority");
                assert_eq!(failure.source, Some(SOURCE));
                assert_eq!(failure.kind, ErrorKind::Type);
            }
            assert_eq!(bindings.to_string(), snapshot);
        }
        assert_eq!(
            bindings.finish(),
            Some(vec![
                ty,
                Term::Primitive(Primitive::U8),
                rigid(4, nir::Sort::ComptimeValue),
            ])
        );
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
            let mut bindings = Bindings::structural([id(1)]);
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
                    assert_eq!(
                        bindings.to_string(),
                        "(input-bindings (sort-change substitute) (slot 0 pending))"
                    );
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
        let mut bindings = Bindings::structural([id(2), id(1)]);
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
                    nir::DeclarationParameter {
                        declaration: id(2),
                        ..parameter()
                    },
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
