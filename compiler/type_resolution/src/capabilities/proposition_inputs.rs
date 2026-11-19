//! Declaration observations that may change a PTC answer. Keep whole operands:
//! an equality about a type inside a callable binder cannot leave that binder.
use super::*;
use crate::ptc::TraitQuery;

#[derive(Default)]
pub(super) struct PropositionInputs<'a> {
    pub traits: Vec<&'a TraitQuery>,
    pub projections: Vec<&'a Term>,
}

impl<'a> PropositionInputs<'a> {
    pub fn collect(
        context: &'a Context,
        goal: &'a Formula,
        depth: usize,
        fuel: &mut usize,
    ) -> Result<Self, QueryError> {
        let mut inputs = Self::default();
        let mut traits = HashSet::new();
        let mut projections = HashSet::new();
        let mut pending = Vec::new();
        for root in context.assumptions().iter().chain(std::iter::once(goal)) {
            pending.push((root, depth));
            while let Some((formula, depth)) = pending.pop() {
                *fuel = fuel.checked_sub(1).ok_or(QueryError::Budget)?;
                let child_depth = depth.checked_sub(1).ok_or(QueryError::Budget)?;
                let mut observe = |operand: &'a Term| -> Result<(), QueryError> {
                    if nir::type_computations::contains_associated_projection(
                        operand,
                        fuel,
                        child_depth,
                    )? && projections.insert(operand)
                    {
                        inputs.projections.push(operand);
                    }
                    Ok(())
                };
                match formula {
                    Formula::Trait(query) => {
                        // The selected trait fact carries its own projection
                        // evidence. Given facts need no implementation view.
                        for operand in query.inputs() {
                            crate::associated::charge_projection_copy(operand, fuel)?;
                        }
                        if traits.insert(query) {
                            inputs.traits.push(query);
                        }
                    }
                    Formula::Equal(crate::ptc::Equality(left, right)) => {
                        observe(left)?;
                        observe(right)?;
                    }
                    Formula::Kind(query) => observe(&query.ty)?,
                    Formula::Not(inner) => pending.push((inner, child_depth)),
                    Formula::And(items) | Formula::Or(items) => {
                        if items.len() > fuel.saturating_sub(pending.len()) {
                            return Err(QueryError::Budget);
                        }
                        pending.extend(items.iter().map(|item| (item, child_depth)));
                    }
                    Formula::True | Formula::False | Formula::IntLe(_) => {}
                }
            }
        }
        Ok(inputs)
    }

    pub fn is_empty(&self) -> bool {
        self.traits.is_empty() && self.projections.is_empty()
    }
}

/// A whole-operand equality derived from the selected associated Type definitions.
/// Both sides live in the caller's scope, including any nested callable binders.
#[derive(Debug)]
pub(super) struct ProjectionEquation {
    pub source: Term,
    pub value: Term,
}

impl ProjectionEquation {
    pub fn retain(
        source: &Term,
        value: &Term,
        equations: &mut Vec<Self>,
        fuel: &mut usize,
    ) -> Result<(), QueryError> {
        if source != value {
            crate::associated::charge_projection_copy(source, fuel)?;
            crate::associated::charge_projection_copy(value, fuel)?;
            equations.push(Self {
                source: source.clone(),
                value: value.clone(),
            });
        }
        Ok(())
    }

    pub fn replay(
        &self,
        world: &World<'_>,
        budget: &mut Budget,
        completed: &mut std::collections::HashMap<Term, Term>,
    ) -> Result<(), QueryError> {
        let view = world.projection_operand_view_cached(&self.source, budget, completed)?;
        if view.value != self.value {
            return Err(
                "associated projection evidence disagrees with its declaration world".into(),
            );
        }
        Ok(())
    }
}
