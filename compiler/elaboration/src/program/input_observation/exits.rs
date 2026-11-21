//! Find the identity delivered on normal completion. The entire producer,
//! including selectors, transfers, guards and writes, stays in its VM frame.
use super::*;
use std::sync::Arc;

#[derive(Clone)]
struct Facts {
    values: BTreeMap<n::LocalId, Arc<Term>>,
    snapshots: BTreeSet<n::LocalId>,
}

impl Facts {
    fn fork(&self, budget: &mut comptime_runtime::Budget) -> Result<Self, Failure> {
        budget.consume(
            self.values
                .len()
                .saturating_add(self.snapshots.len())
                .saturating_add(1),
        )?;
        Ok(self.clone())
    }

    fn after_opaque(
        &mut self,
        expr: &n::Expr,
        budget: &mut comptime_runtime::Budget,
    ) -> Result<(), Failure> {
        budget.consume(self.values.len())?;
        self.values.retain(|id, _| self.snapshots.contains(id));
        if !self.values.is_empty() {
            Checker::invalidate_mutated_type_bindings(expr, &mut self.values, &mut budget.steps)?;
        }
        Ok(())
    }
}

/// A trap in an unselected alternative is not a compile-time diagnostic.
/// Budget and structural failures apply even when the path is speculative.
fn path_observation<T>(
    observed: Result<T, Failure>,
    speculative: bool,
) -> Result<Option<T>, Failure> {
    if speculative {
        observation::hint(observed)
    } else {
        observation::optional(observed)
    }
}

pub(super) fn merge_identity(
    result: &mut Option<Term>,
    value: Term,
    budget: &mut comptime_runtime::Budget,
) -> Result<(), Failure> {
    if let Some(previous) = result {
        nir::copy_budget::charge_term_copy(previous, &mut budget.steps, 128)?;
        nir::copy_budget::charge_term_copy(&value, &mut budget.steps, 128)?;
        if *previous != value {
            return Err(Failure::Unavailable);
        }
    } else {
        *result = Some(value);
    }
    Ok(())
}

impl Checker<'_, '_> {
    /// A partial call frame can read only its supplied slots. In particular,
    /// identifiers missing from a branch cannot fall back to the caller scope.
    pub(in crate::program) fn observe_input_tail_in(
        &self,
        producer: &n::Expr,
        values: BTreeMap<n::LocalId, Arc<Term>>,
        snapshots: BTreeSet<n::LocalId>,
        budget: &mut comptime_runtime::Budget,
    ) -> Result<Term, Failure> {
        let mut paths = vec![(producer, Facts { values, snapshots }, false)];
        let mut result = None;
        while let Some((mut expr, mut facts, mut speculative)) = paths.pop() {
            loop {
                budget.consume(1)?;
                // A checked dead tail can still supply its written domain to
                // the unconditional source path. A transferring alternative
                // contributes no value to a join with its siblings.
                if speculative && is_never(&expr.ty) {
                    break;
                }
                match &expr.kind {
                    K::Block {
                        statements,
                        result: tail,
                    }
                    | K::CallPreparation {
                        statements,
                        result: tail,
                    } => {
                        let prefix = self.observe_input_prefix_in(
                            statements,
                            &mut facts.values,
                            &mut facts.snapshots,
                            budget,
                        );
                        path_observation(prefix, speculative)?.ok_or(Failure::Unavailable)?;
                        if let Some(tail) = tail {
                            expr = tail;
                            continue;
                        }
                        if is_never(&expr.ty) {
                            return Err(Failure::Unavailable);
                        }
                        merge_identity(&mut result, Term::Unit, budget)?;
                    }
                    K::If {
                        condition,
                        then_branch,
                        else_branch,
                    } => {
                        let selector = if input_facts_cover(condition, &facts.values, budget)? {
                            path_observation(
                                self.observe_normal_input_in(condition, 0, budget, &facts.values),
                                speculative,
                            )?
                        } else {
                            None
                        };
                        match selector {
                            Some(Term::Bool(true)) => expr = then_branch,
                            Some(Term::Bool(false)) => {
                                if let Some(other) = else_branch {
                                    expr = other;
                                } else {
                                    merge_identity(&mut result, Term::Unit, budget)?;
                                    break;
                                }
                            }
                            _ => {
                                facts.after_opaque(condition, budget)?;
                                if let Some(other) = else_branch {
                                    paths.push((other, facts.fork(budget)?, true));
                                } else {
                                    merge_identity(&mut result, Term::Unit, budget)?;
                                }
                                expr = then_branch;
                                speculative = true;
                            }
                        }
                        continue;
                    }
                    K::Match { arms, .. } => {
                        // Preserve any selected pattern evidence in a complete frame.
                        if input_facts_cover(expr, &facts.values, budget)?
                            && let Some(known) = path_observation(
                                self.observe_normal_input_in(expr, 0, budget, &facts.values),
                                speculative,
                            )?
                        {
                            merge_identity(&mut result, known, budget)?;
                            break;
                        }
                        // Failed guards can modify the surrounding frame before
                        // a later arm. Retain only independent snapshots.
                        facts.after_opaque(expr, budget)?;
                        for arm in arms {
                            let mut branch = facts.fork(budget)?;
                            nir::free_bindings::remove_pattern_bindings(
                                &arm.pattern,
                                &mut branch.values,
                                &mut budget.steps,
                            )?;
                            budget.consume(branch.snapshots.len())?;
                            branch.snapshots.retain(|id| branch.values.contains_key(id));
                            paths.push((&arm.body, branch, true));
                        }
                    }
                    _ => {
                        if !input_facts_cover(expr, &facts.values, budget)? {
                            return Err(Failure::Unavailable);
                        }
                        let observed = self.observe_normal_input_in(expr, 0, budget, &facts.values);
                        let value =
                            path_observation(observed, speculative)?.ok_or(Failure::Unavailable)?;
                        merge_identity(&mut result, value, budget)?;
                    }
                }
                break;
            }
        }
        result.ok_or(Failure::Unavailable)
    }
}
