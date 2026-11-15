//! Observe selected control paths without retaining a value-constructor frame
//! for every branch and lexical block. This is bounded static evidence, not VM
//! execution: calls, assignments and transfers remain unavailable observations.
use super::*;
use std::borrow::Cow;

impl Checker<'_, '_> {
    pub(in crate::program) fn observe_normal_input_in(
        &self,
        mut expr: &n::Expr,
        mut depth: usize,
        budget: &mut comptime_runtime::Budget,
        bindings: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    ) -> Result<Term, Failure> {
        let mut bindings = Cow::Borrowed(bindings);
        loop {
            if depth >= 128 {
                return Err(Failure::Depth);
            }
            budget.consume(1)?;
            match &expr.kind {
                K::If {
                    condition,
                    then_branch,
                    else_branch,
                } => {
                    // Only a known selector permits choosing an arm. The
                    // checked expression itself remains in the executable plan.
                    let Term::Bool(selected) =
                        self.observe_normal_input_in(condition, depth + 1, budget, &bindings)?
                    else {
                        return Err(Failure::Unavailable);
                    };
                    expr = if selected {
                        then_branch
                    } else if let Some(other) = else_branch {
                        other
                    } else {
                        return Ok(Term::Unit);
                    };
                }
                K::Match { value, arms } => {
                    let value =
                        self.observe_normal_input_in(value, depth + 1, budget, &bindings)?;
                    let mut selected = None;
                    for arm in arms {
                        budget.consume(bindings.len().saturating_add(1))?;
                        let mut scoped = bindings.as_ref().clone();
                        if !self.observe_normal_pattern(
                            &arm.pattern,
                            &value,
                            depth + 1,
                            budget,
                            &mut scoped,
                        )? {
                            continue;
                        }
                        if let Some(guard) = &arm.guard {
                            let Term::Bool(accepted) =
                                self.observe_normal_input_in(guard, depth + 1, budget, &scoped)?
                            else {
                                return Err(Failure::Unavailable);
                            };
                            if !accepted {
                                continue;
                            }
                        }
                        selected = Some((&arm.body, scoped));
                        break;
                    }
                    let Some((body, scoped)) = selected else {
                        return Err(Failure::Unavailable);
                    };
                    expr = body;
                    bindings = Cow::Owned(scoped);
                }
                K::Block { statements, result } | K::CallPreparation { statements, result } => {
                    if statements.len() > budget.steps {
                        return Err(Failure::Steps);
                    }
                    budget.consume(bindings.len())?;
                    let mut scoped = bindings.as_ref().clone();
                    for statement in statements {
                        budget.consume(1)?;
                        self.observe_normal_binding(statement, depth, budget, &mut scoped)?;
                    }
                    let Some(result) = result else {
                        return Ok(Term::Unit);
                    };
                    expr = result;
                    bindings = Cow::Owned(scoped);
                }
                _ => return self.observe_normal_value_in(expr, depth, budget, &bindings),
            }
            depth += 1;
        }
    }
}
