//! Read-only identity observation for declaration input plans, shared by
//! opaque heads and nominal constructors. Executable producers remain intact.
use super::*;

use nir::program::InputSchedule as Plan;

fn observable(
    plan: Plan<'_>,
    count: usize,
    outer: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    budget: &mut comptime_runtime::Budget,
) -> Result<bool, Failure> {
    Ok(plan
        .check(count, |n| budget.consume(n))?
        .is_some_and(|schedule| schedule.locals.iter().all(|id| !outer.contains_key(id))))
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
        bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    ) -> Result<Vec<Term>, Failure> {
        budget.consume(arguments.len().saturating_add(bindings.len()))?;
        if !plan.preparations.is_empty() || !observable(plan, arguments.len(), bindings, budget)? {
            return Err(Failure::Unavailable);
        }
        // Contextual operands obtain their domain from the declaration. This
        // observation neither runs its formation checks nor replaces the plan.
        let parameters = if arguments
            .iter()
            .any(|argument| argument.expression().is_none())
        {
            self.observed_declaration_domains(head, budget)?
        } else {
            None
        };
        if parameters
            .as_ref()
            .is_some_and(|inputs| inputs.len() != arguments.len())
        {
            return Err(Failure::Unavailable);
        }
        let order: Vec<_> = plan
            .order
            .map_or_else(|| (0..arguments.len()).collect(), |order| order.to_vec());
        let mut scoped = bindings.clone();
        let mut values = vec![None; arguments.len()];
        let mut ids = Vec::new();
        let mut domains = Vec::new();
        let mut completed = Vec::new();
        let mut pending = order;
        while !pending.is_empty() {
            let before = pending.len();
            let mut deferred = Vec::new();
            for slot in pending {
                if arguments[slot].expression().is_none()
                    && let Some(parameters) = parameters.as_ref()
                {
                    // A caller rigid introduced by substitution cannot add a
                    // dependency on this declaration's still-pending slots.
                    let mut required = BTreeSet::new();
                    nir::type_computations::rigid_declarations_bounded(
                        parameters[slot].1,
                        &mut budget.steps,
                        &mut required,
                    )?;
                    budget.consume(parameters.len())?;
                    if parameters
                        .iter()
                        .zip(&values)
                        .any(|((id, _), value)| value.is_none() && required.contains(id))
                    {
                        deferred.push(slot);
                        continue;
                    }
                }
                let domain = if let Some(parameters) = parameters.as_ref() {
                    Some(
                        nir::DeclarationSubstitution::bounded(
                            &ids,
                            &domains,
                            &completed,
                            &[],
                            &mut budget.steps,
                        )
                        .and_then(|substitution| {
                            substitution.instantiate_bounded(parameters[slot].1, &mut budget.steps)
                        })?,
                    )
                } else {
                    None
                };
                let value = collect_known_normal_argument(
                    &arguments[slot],
                    domain.as_ref(),
                    self.protocols.meta_list,
                    budget,
                    |value, budget| self.observe_normal_input_in(value, depth + 1, budget, &scoped),
                )?;
                if let Some(bindings) = plan.bindings {
                    scoped.insert(
                        bindings[slot],
                        std::sync::Arc::new(copy_observed_term(&value, budget)?),
                    );
                }
                if let (Some(parameters), Some(domain)) = (parameters.as_ref(), domain) {
                    ids.push(parameters[slot].0);
                    domains.push(domain);
                    completed.push(copy_observed_term(&value, budget)?);
                }
                values[slot] = Some(value);
            }
            if deferred.len() == before {
                return Err(Failure::Unavailable);
            }
            pending = deferred;
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
        assert!(observable(plan, 2, &outer, &mut budget).unwrap());
        for order in [&[0, 0][..], &[0, 2], &[0]] {
            assert!(
                !observable(
                    Plan {
                        order: Some(order),
                        ..plan
                    },
                    2,
                    &outer,
                    &mut budget
                )
                .unwrap()
            );
        }
        for bindings in [&[a, a][..], &[a]] {
            assert!(
                !observable(
                    Plan {
                        bindings: Some(bindings),
                        ..plan
                    },
                    2,
                    &outer,
                    &mut budget
                )
                .unwrap()
            );
        }
        let outer = BTreeMap::from([(a, std::sync::Arc::new(Term::Primitive(P::Bool)))]);
        assert!(!observable(plan, 2, &outer, &mut budget).unwrap());
        let mut short = comptime_runtime::Budget {
            steps: 3,
            ..Default::default()
        };
        assert_eq!(
            observable(plan, 2, &BTreeMap::new(), &mut short),
            Err(Failure::Steps)
        );
        assert!(observable(plan, 2, &BTreeMap::new(), &mut budget).unwrap());
        let dump = format!("{plan}");
        assert!(
            dump.starts_with("(input-plan (order 1 0) (bindings "),
            "{dump}"
        );
        assert!(dump.ends_with(") (preparations))"), "{dump}");
    }
}
