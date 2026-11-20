//! Read-only identity observation for declaration input plans, shared by
//! opaque heads and nominal constructors. Executable producers remain intact.
use super::*;

use nir::program::InputSchedule as Plan;

type LocalValues = BTreeMap<n::LocalId, std::sync::Arc<Term>>;

fn observable_schedule<'p>(
    plan: Plan<'p>,
    count: usize,
    outer: &LocalValues,
    budget: &mut comptime_runtime::Budget,
) -> Result<Option<n::CheckedInputSchedule<'p>>, Failure> {
    Ok(plan
        .check(count, |n| budget.consume(n))?
        .filter(|schedule| schedule.locals.iter().all(|id| !outer.contains_key(id))))
}

/// A contextual operand may need a later argument. Follow declaration
/// templates only: rigids inside supplied values belong to the caller.
fn missing_domain_input(
    domain: &Term,
    parameters: &BTreeMap<DeclarationId, (usize, &Term)>,
    values: &[Option<Term>],
    budget: &mut comptime_runtime::Budget,
) -> Result<Option<usize>, Failure> {
    let mut dependencies = BTreeSet::new();
    nir::type_computations::rigid_declarations_bounded(
        domain,
        &mut budget.steps,
        &mut dependencies,
    )?;
    let mut seen = BTreeSet::new();
    while let Some(id) = dependencies.pop_first() {
        let Some(&(slot, template)) = parameters.get(&id) else {
            continue;
        };
        if values[slot].is_none() {
            return Ok(Some(slot));
        }
        if seen.insert(id) {
            nir::type_computations::rigid_declarations_bounded(
                template,
                &mut budget.steps,
                &mut dependencies,
            )?;
        }
    }
    Ok(None)
}

impl Checker<'_, '_> {
    fn observed_declaration_domains<'a>(
        &'a self,
        head: DeclarationId,
        budget: &mut comptime_runtime::Budget,
    ) -> Result<Option<Vec<(DeclarationId, &'a Term)>>, Failure> {
        budget.consume(1)?;
        if let Some(parameters) = self
            .source_declarations
            .owner(head)
            .and_then(|owner| self.normal_parameters.get(&owner))
        {
            budget.consume(parameters.len())?;
            return Ok(Some(
                parameters
                    .iter()
                    .map(|(id, _, domain)| (*id, domain))
                    .collect(),
            ));
        }
        let signature = self
            .structs
            .get(&head)
            .map(|def| (&def.type_parameters, &def.parameter_types))
            .or_else(|| {
                self.enums
                    .get(&head)
                    .map(|def| (&def.type_parameters, &def.parameter_types))
            })
            .or_else(|| {
                self.newtypes
                    .get(&head)
                    .map(|def| (&def.type_parameters, &def.parameter_types))
            });
        let Some((parameters, domains)) = signature else {
            return Ok(None);
        };
        budget.consume(parameters.len())?;
        if !domains.is_empty() && domains.len() != parameters.len() {
            return Err(Failure::Invalid(
                "declaration input domain arity mismatch".into(),
            ));
        }
        Ok(Some(
            parameters
                .iter()
                .enumerate()
                .map(|(slot, id)| (*id, domains.get(slot).unwrap_or(&Term::Primitive(P::Type))))
                .collect(),
        ))
    }
    pub(super) fn observe_declaration_arguments(
        &self,
        head: DeclarationId,
        arguments: &[n::NormalFormArgument],
        plan: Plan<'_>,
        depth: usize,
        budget: &mut comptime_runtime::Budget,
        bindings: &LocalValues,
    ) -> Result<Vec<Term>, Failure> {
        budget.consume(arguments.len().saturating_add(bindings.len()))?;
        let schedule = observable_schedule(plan, arguments.len(), bindings, budget)?
            .ok_or(Failure::Unavailable)?;
        let (signature, frames) = if arguments
            .iter()
            .any(|operand| operand.expression().is_none())
        {
            let signature = self
                .observed_declaration_domains(head, budget)?
                .ok_or(Failure::Unavailable)?;
            if signature.len() != arguments.len() {
                return Err(Failure::Unavailable);
            }
            (signature, self.call_input_frames())
        } else {
            (Vec::new(), Vec::new())
        };
        let parameters = signature
            .iter()
            .enumerate()
            .map(|(slot, &(id, domain))| (id, (slot, domain)))
            .collect();

        let mut scoped = bindings.clone();
        let mut values = vec![None; arguments.len()];
        let mut steps = schedule.steps();
        let mut waiting: BTreeMap<usize, Vec<(usize, LocalValues)>> = BTreeMap::new();
        let mut ready = BTreeMap::new();
        loop {
            // Wake before advancing to the next preparation or default. A
            // delayed observation sees exactly the bindings at its source.
            let (slot, snapshot) = if let Some((slot, saved)) = ready.pop_first() {
                (slot, Some(saved))
            } else {
                match steps.next() {
                    Some(n::InputStep::Prepare(expr)) => {
                        self.observe_normal_binding(expr, depth + 1, budget, &mut scoped)?;
                        continue;
                    }
                    Some(n::InputStep::Supply(slot)) => (slot, None),
                    None => break,
                }
            };
            budget.consume(1)?;
            let operand = &arguments[slot];
            let domain = if operand.expression().is_none() {
                let template = signature[slot].1;
                if let Some(dependency) =
                    missing_domain_input(template, &parameters, &values, budget)?
                {
                    let saved = match snapshot {
                        Some(saved) => saved,
                        None => {
                            budget.consume(scoped.len())?;
                            scoped.clone()
                        }
                    };
                    waiting.entry(dependency).or_default().push((slot, saved));
                    continue;
                }
                Some(nir::instantiate_observed_domain(
                    template,
                    |id| {
                        let &(index, domain) = parameters.get(&id)?;
                        Some((domain, values[index].as_ref()?))
                    },
                    &frames,
                    &mut budget.steps,
                )?)
            } else {
                None
            };
            let value = collect_known_normal_argument(
                operand,
                domain.as_ref(),
                self.protocols.meta_list,
                budget,
                |expr, budget| {
                    self.observe_normal_input_in(
                        expr,
                        depth + 1,
                        budget,
                        snapshot.as_ref().unwrap_or(&scoped),
                    )
                },
            )?;
            if let Some(binding) = schedule.binding(slot) {
                scoped.insert(
                    binding,
                    std::sync::Arc::new(copy_observed_term(&value, budget)?),
                );
            }
            values[slot] = Some(value);
            if let Some(dependents) = waiting.remove(&slot) {
                budget.consume(dependents.len())?;
                ready.extend(dependents);
            }
        }
        values
            .into_iter()
            .map(|value| value.ok_or(Failure::Unavailable))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dependent_domain_waits_for_its_template_inputs_only() {
        let id = |local| DeclarationId { package: 7, local };
        let rigid = |local| Term::Rigid {
            declaration: id(local),
            sort: Sort::Type,
        };
        let domain = rigid(1);
        let element_domain = rigid(0);
        let ty = p(P::Type);
        let parameters = BTreeMap::from([
            (id(0), (0, &ty)),
            (id(1), (1, &element_domain)),
            (id(2), (2, &ty)),
        ]);
        let mut budget = comptime_runtime::Budget::default();
        let mut values = [None, None, None];
        assert_eq!(
            missing_domain_input(&domain, &parameters, &values, &mut budget).unwrap(),
            Some(1)
        );

        // The caller's value happens to mention the declaration of slot 2.
        // Its identity is a replacement; only slot 1's template depends on 0.
        values[1] = Some(rigid(2));
        assert_eq!(
            missing_domain_input(&domain, &parameters, &values, &mut budget).unwrap(),
            Some(0)
        );
        values[0] = Some(ty.clone());
        assert_eq!(
            missing_domain_input(&domain, &parameters, &values, &mut budget).unwrap(),
            None
        );

        budget.steps = 0;
        assert!(matches!(
            missing_domain_input(&domain, &parameters, &values, &mut budget),
            Err(Failure::Structure)
        ));
    }

    #[test]
    fn input_plans_require_a_permutation_and_fresh_unique_bindings() {
        let a = n::LocalId { owner: 1, index: 0 };
        let b = n::LocalId { owner: 1, index: 1 };
        let outer = BTreeMap::new();
        let mut budget = comptime_runtime::Budget::default();
        let bindings = [a, b];
        let plan = Plan {
            bindings: Some(&bindings),
            order: Some(&[1, 0]),
            ..Plan::default()
        };
        assert!(
            observable_schedule(plan, 2, &outer, &mut budget)
                .unwrap()
                .is_some()
        );
        for order in [&[0, 0][..], &[0, 2], &[0]] {
            assert!(
                observable_schedule(
                    Plan {
                        order: Some(order),
                        ..plan
                    },
                    2,
                    &outer,
                    &mut budget
                )
                .unwrap()
                .is_none()
            );
        }
        for bindings in [&[a, a][..], &[a]] {
            assert!(
                observable_schedule(
                    Plan {
                        bindings: Some(bindings),
                        ..plan
                    },
                    2,
                    &outer,
                    &mut budget
                )
                .unwrap()
                .is_none()
            );
        }
        let outer = BTreeMap::from([(a, std::sync::Arc::new(Term::Primitive(P::Bool)))]);
        assert!(
            observable_schedule(plan, 2, &outer, &mut budget)
                .unwrap()
                .is_none()
        );
        let mut short = comptime_runtime::Budget {
            steps: 3,
            ..Default::default()
        };
        assert!(matches!(
            observable_schedule(plan, 2, &BTreeMap::new(), &mut short),
            Err(Failure::Steps)
        ));
        assert!(
            observable_schedule(plan, 2, &BTreeMap::new(), &mut budget)
                .unwrap()
                .is_some()
        );
        let dump = format!("{plan}");
        assert!(
            dump.starts_with("(input-plan (order 1 0) (bindings "),
            "{dump}"
        );
        assert!(dump.ends_with(") (preparations))"), "{dump}");
    }
}
