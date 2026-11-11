//! Read-only identity observation for declaration input plans, shared by
//! opaque heads and nominal constructors. Executable producers remain intact.
use super::*;

#[derive(Clone, Copy, Default)]
pub(super) struct Plan<'a> {
    pub preparations: &'a [n::NormalFormPreparation],
    pub bindings: Option<&'a [n::LocalId]>,
    pub order: Option<&'a [usize]>,
}
impl<'a> Plan<'a> {
    pub(super) fn normal(plan: Option<&'a n::NormalFormInputPlan>) -> Self {
        plan.map_or_else(Self::default, |plan| Self {
            preparations: &plan.preparations,
            bindings: Some(&plan.bindings),
            order: Some(&plan.order),
        })
    }
    fn valid(
        self,
        count: usize,
        outer: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        budget: &mut comptime_runtime::Budget,
    ) -> Result<bool, Failure> {
        budget.consume(
            self.order
                .map_or(0, <[_]>::len)
                .saturating_add(self.bindings.map_or(0, <[_]>::len)),
        )?;
        if let Some(order) = self.order {
            let mut seen = BTreeSet::new();
            if order.len() != count
                || order
                    .iter()
                    .any(|slot| *slot >= count || !seen.insert(*slot))
            {
                return Ok(false);
            }
        }
        if let Some(bindings) = self.bindings {
            let mut seen = BTreeSet::new();
            if bindings.len() != count
                || bindings
                    .iter()
                    .any(|id| outer.contains_key(id) || !seen.insert(*id))
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
impl std::fmt::Display for Plan<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(observed-input-plan (order")?;
        if let Some(order) = self.order {
            for slot in order {
                write!(f, " {slot}")?;
            }
        } else {
            f.write_str(" declaration")?;
        }
        f.write_str(") (bindings")?;
        if let Some(bindings) = self.bindings {
            for binding in bindings {
                write!(f, " {binding}")?;
            }
        }
        f.write_str(") (preparations")?;
        for preparation in self.preparations {
            write!(
                f,
                " (before {} {})",
                preparation.before, preparation.expression
            )?;
        }
        f.write_str("))")
    }
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
        if !plan.preparations.is_empty() || !plan.valid(arguments.len(), bindings, budget)? {
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
                if arguments[slot].expression().is_none() {
                    if let (Some(parameters), Some(domain)) = (parameters.as_ref(), domain.as_ref())
                    {
                        budget.consume(
                            parameters.len().saturating_mul(ids.len().saturating_add(1)),
                        )?;
                        let mut waiting = false;
                        for (id, _) in parameters {
                            if !ids.contains(id) {
                                nir::type_computations::charge_term(domain, &mut budget.steps)?;
                                if domain.mentions_rigid(*id) {
                                    waiting = true;
                                    break;
                                }
                            }
                        }
                        if waiting {
                            deferred.push(slot);
                            continue;
                        }
                    }
                }
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
        assert!(plan.valid(2, &outer, &mut budget).unwrap());
        for order in [&[0, 0][..], &[0, 2], &[0]] {
            assert!(
                !Plan {
                    order: Some(order),
                    ..plan
                }
                .valid(2, &outer, &mut budget)
                .unwrap()
            );
        }
        for bindings in [&[a, a][..], &[a]] {
            assert!(
                !Plan {
                    bindings: Some(bindings),
                    ..plan
                }
                .valid(2, &outer, &mut budget)
                .unwrap()
            );
        }
        let outer = BTreeMap::from([(a, std::sync::Arc::new(Term::Primitive(P::Bool)))]);
        assert!(!plan.valid(2, &outer, &mut budget).unwrap());
        let mut short = comptime_runtime::Budget {
            steps: 3,
            ..Default::default()
        };
        assert_eq!(
            plan.valid(2, &BTreeMap::new(), &mut short),
            Err(Failure::Steps)
        );
        assert!(plan.valid(2, &BTreeMap::new(), &mut budget).unwrap());
        let dump = format!("{plan}");
        assert!(
            dump.starts_with("(observed-input-plan (order 1 0) (bindings "),
            "{dump}"
        );
        assert!(dump.ends_with(") (preparations))"), "{dump}");
    }
}
