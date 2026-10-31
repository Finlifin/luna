//! Mentioned types are search candidates, never facts. Scope-preserving
//! traversal does not descend through binders with local references.
use super::*;

impl Context {
    /// Enumerate distinct Type terms mentioned by regional propositions,
    /// including nested structure and explicit opaque-head arguments. Every
    /// use still needs an equality proof; occurrences under Not/Or carry no
    /// authority. Local binder terms are not reinterpreted in the outer scope.
    pub fn mentioned_types_metered(
        &self,
        budget: &mut Budget,
    ) -> Result<Vec<&Term>, UnknownReason> {
        let limit = budget.max_depth.min(128);
        for formula in self.assumptions.iter() {
            if !within_limits(formula, limit, &mut budget.steps) {
                return Err(UnknownReason::BudgetExhausted);
            }
        }
        let mut pending: Vec<_> = self.assumptions.iter().rev().map(Input::Formula).collect();
        let mut candidates = Vec::new();
        let mut seen = std::collections::HashSet::new();
        while let Some(input) = pending.pop() {
            budget.steps = budget
                .steps
                .checked_sub(1)
                .ok_or(UnknownReason::BudgetExhausted)?;
            match input {
                Input::Formula(Formula::Equal(Equality(a, b))) => {
                    pending.push(Input::Term(b));
                    pending.push(Input::Term(a));
                }
                Input::Formula(Formula::Kind(query)) => pending.push(Input::Term(&query.ty)),
                Input::Formula(Formula::Trait(query)) => {
                    pending.extend(query.inputs().map(Input::Term))
                }
                Input::Formula(Formula::Not(inner)) => pending.push(Input::Formula(inner)),
                Input::Formula(Formula::And(parts) | Formula::Or(parts)) => {
                    pending.extend(parts.iter().rev().map(Input::Formula))
                }
                Input::Term(term) => {
                    // Hashing, scope checking and child discovery may all inspect
                    // nested structure; account for that before doing the work.
                    if !within_input_limits(Input::Term(term), limit, &mut budget.steps) {
                        return Err(UnknownReason::BudgetExhausted);
                    }
                    if term.sort() == Sort::Type
                        && term.check_scope(&self.scope).is_ok()
                        && seen.insert(term)
                    {
                        candidates.push(term);
                    }
                    pending.extend(subterms(term).into_iter().rev().map(Input::Term));
                }
                _ => {}
            }
        }
        Ok(candidates)
    }
}
