//! Explicit decision continuations. Logical depth remains metered, while
//! branch restoration and backjumping no longer depend on the Rust call stack.
use super::{Conflict, Search, SearchResult, UnknownReason, boolean, learning};
use std::fmt;

enum Frame {
    Boolean(Vec<(usize, bool)>),
    Learned {
        index: usize,
        value: bool,
        reason: Conflict,
    },
    Negative {
        index: usize,
        child_depth: usize,
    },
    Positive {
        index: usize,
        negative: SearchResult,
    },
}
enum Step {
    Return(SearchResult),
    Descend(Frame, usize),
}
impl fmt::Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boolean(trail) => {
                write!(f, "(boolean-frame (trail")?;
                for (index, value) in trail {
                    write!(f, " (literal {index} {value})")?;
                }
                write!(f, "))")
            }
            Self::Learned {
                index,
                value,
                reason,
            } => write!(
                f,
                "(learned-frame (literal {index} {value}) (reason {reason}))"
            ),
            Self::Negative { index, child_depth } => write!(
                f,
                "(negative-frame (atom {index}) (child-depth {child_depth}))"
            ),
            Self::Positive { index, negative } => {
                let result = match negative {
                    SearchResult::Unsat(_) => "unsat",
                    SearchResult::Unknown(_) => "unknown",
                    SearchResult::Model(_) => "model",
                };
                write!(
                    f,
                    "(positive-frame (atom {index}) (negative-result {result}))"
                )
            }
        }
    }
}
impl Search<'_> {
    pub(super) fn solve(
        &mut self,
        assignment: &mut [Option<bool>],
        mut depth: usize,
    ) -> SearchResult {
        let mut frames = Vec::new();
        'search: loop {
            let mut result = match self.enter(assignment, depth) {
                Step::Descend(frame, child_depth) => {
                    frames.push(frame);
                    depth = child_depth;
                    continue;
                }
                Step::Return(result) => result,
            };
            loop {
                let Some(frame) = frames.pop() else {
                    return result;
                };
                match frame {
                    Frame::Boolean(trail) => {
                        for (index, value) in trail.into_iter().rev() {
                            assignment[index] = None;
                            if let SearchResult::Unsat(proof) = result {
                                if !proof.support.contains(&index) {
                                    result = SearchResult::Unsat(proof);
                                    continue;
                                }
                                assignment[index] = Some(!value);
                                let reason = self.boolean_reason(assignment);
                                assignment[index] = None;
                                let (negative, positive) = if value {
                                    (reason, proof)
                                } else {
                                    (proof, reason)
                                };
                                result = SearchResult::Unsat(Conflict::resolve(
                                    index,
                                    &self.atoms[index],
                                    negative,
                                    positive,
                                ));
                            }
                        }
                    }
                    Frame::Learned {
                        index,
                        value,
                        reason,
                    } => {
                        assignment[index] = None;
                        if let SearchResult::Unsat(proof) = result {
                            let (negative, positive) = if value {
                                (reason, proof)
                            } else {
                                (proof, reason)
                            };
                            result = SearchResult::Unsat(Conflict::resolve(
                                index,
                                &self.atoms[index],
                                negative,
                                positive,
                            ));
                        }
                    }
                    Frame::Negative { index, child_depth } => {
                        assignment[index] = None;
                        if matches!(result, SearchResult::Model(_))
                            || matches!(&result, SearchResult::Unsat(conflict) if !conflict.support.contains(&index))
                        {
                            continue;
                        }
                        assignment[index] = Some(true);
                        frames.push(Frame::Positive {
                            index,
                            negative: result,
                        });
                        depth = child_depth;
                        continue 'search;
                    }
                    Frame::Positive { index, negative } => {
                        assignment[index] = None;
                        if matches!(&result, SearchResult::Unsat(conflict) if !conflict.support.contains(&index))
                        {
                            continue;
                        }
                        result = match (negative, result) {
                            (SearchResult::Unsat(negative), SearchResult::Unsat(positive)) => {
                                let conflict = Conflict::resolve(
                                    index,
                                    &self.atoms[index],
                                    negative,
                                    positive,
                                );
                                SearchResult::Unsat(self.learn_resolved(conflict, assignment))
                            }
                            (_, model @ SearchResult::Model(_)) => model,
                            (SearchResult::Unknown(reason), _)
                            | (_, SearchResult::Unknown(reason)) => SearchResult::Unknown(reason),
                            _ => unreachable!(),
                        };
                    }
                }
            }
        }
    }

    fn enter(&mut self, assignment: &mut [Option<bool>], depth: usize) -> Step {
        if depth >= self.max_depth || self.tick().is_err() {
            return Step::Return(SearchResult::Unknown(UnknownReason::BudgetExhausted));
        }
        if self.boolean.is_none() {
            match boolean::Circuit::compile(self.formula, self.atoms, &mut self.steps) {
                Ok(circuit) => self.boolean = Some(circuit),
                Err(()) => {
                    return Step::Return(SearchResult::Unknown(UnknownReason::BudgetExhausted));
                }
            }
        }
        let propagation = match self
            .boolean
            .as_ref()
            .unwrap()
            .propagate(assignment, &mut self.steps)
        {
            Ok(result) => result,
            Err(()) => return Step::Return(SearchResult::Unknown(UnknownReason::BudgetExhausted)),
        };
        if propagation.truth == Some(false) {
            let mut conflict = Conflict::boolean(assignment);
            if let Some(support) = propagation.conflict_support {
                conflict.support = support;
            }
            return Step::Return(SearchResult::Unsat(conflict));
        }
        let forced = propagation.literals;
        match self.propagate_learned(assignment) {
            Err(reason) => return Step::Return(SearchResult::Unknown(reason)),
            Ok(learning::Propagation::Conflict(proof)) => {
                return Step::Return(SearchResult::Unsat(proof));
            }
            Ok(learning::Propagation::Unit {
                index,
                value,
                reason,
            }) => {
                assignment[index] = Some(value);
                return Step::Descend(
                    Frame::Learned {
                        index,
                        value,
                        reason,
                    },
                    depth + 1,
                );
            }
            Ok(learning::Propagation::None) => {}
        }
        let classes = match self.theory(assignment) {
            Ok(Some(classes)) => classes,
            Ok(None) => return Step::Return(SearchResult::Unsat(self.learn_conflict(assignment))),
            Err(reason) => return Step::Return(SearchResult::Unknown(reason)),
        };
        if !forced.is_empty() {
            let mut trail = Vec::new();
            for (index, value) in forced {
                if let Some(previous) = assignment[index] {
                    if previous != value {
                        break;
                    }
                    continue;
                }
                assignment[index] = Some(value);
                trail.push((index, value));
            }
            let child_depth = depth + trail.len();
            return Step::Descend(Frame::Boolean(trail), child_depth);
        }
        if let Some(index) = assignment.iter().position(Option::is_none) {
            assignment[index] = Some(false);
            Step::Descend(
                Frame::Negative {
                    index,
                    child_depth: depth + 1,
                },
                depth + 1,
            )
        } else {
            Step::Return(
                self.witness(&classes)
                    .map(SearchResult::Model)
                    .unwrap_or(SearchResult::Unknown(UnknownReason::UnvalidatedModel)),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Context, Formula, Terms, collect};
    use nir::{DeclarationId, Primitive, Sort, Term};

    #[test]
    fn every_search_exit_restores_the_entry_assignment() {
        let atom = |local| {
            Formula::equal(
                Term::Rigid {
                    declaration: DeclarationId { package: 81, local },
                    sort: Sort::Type,
                },
                Term::Primitive(Primitive::Bool),
            )
        };
        let a = atom(0);
        let b = atom(1);
        let c = atom(2);
        let formulas = [
            Formula::And(vec![
                a.clone(),
                Formula::Or(vec![a.clone().negate(), b.clone()]),
                Formula::Or(vec![b.clone().negate(), c.clone()]),
            ]),
            Formula::And(vec![
                Formula::Or(vec![a.clone(), b.clone()]),
                Formula::Or(vec![a.clone().negate(), b.clone()]),
                Formula::Or(vec![a.clone(), b.clone().negate()]),
                Formula::Or(vec![a.clone().negate(), b.clone().negate()]),
            ]),
            Formula::Or(vec![a, b, c]),
        ];
        let context = Context::default();
        let mut saw_unknown = false;
        let mut saw_model = false;
        let mut saw_unsat = false;
        for formula in formulas {
            let mut atoms = Vec::new();
            let mut terms = Terms::default();
            collect(&formula, &mut atoms, &mut terms, 0, 128, &[]).unwrap();
            for initial in [None, Some(false), Some(true)] {
                let mut original = vec![None; atoms.len()];
                original[0] = initial;
                for steps in (0..128).chain([256, 512, 4096, 100_000]) {
                    let mut search = Search {
                        boolean: None,
                        learned: learning::Clauses::default(),
                        context: &context,
                        formula: &formula,
                        atoms: &atoms,
                        terms: &terms,
                        steps,
                        max_depth: 128,
                    };
                    let mut assignment = original.clone();
                    match search.solve(&mut assignment, 0) {
                        SearchResult::Unknown(_) => saw_unknown = true,
                        SearchResult::Model(_) => saw_model = true,
                        SearchResult::Unsat(conflict) => {
                            saw_unsat = true;
                            search.steps = 100_000;
                            search
                                .check_tree(&conflict.tree, &mut assignment, 0)
                                .unwrap();
                        }
                    }
                    assert_eq!(
                        assignment, original,
                        "leaked branch state with budget {steps}"
                    );
                }
            }
        }
        assert!(saw_unknown && saw_model && saw_unsat);
    }

    #[test]
    fn continuations_dump_as_stable_sexpressions() {
        assert_eq!(
            Frame::Boolean(vec![(2, false), (4, true)]).to_string(),
            "(boolean-frame (trail (literal 2 false) (literal 4 true)))"
        );
        assert_eq!(
            Frame::Negative {
                index: 3,
                child_depth: 7
            }
            .to_string(),
            "(negative-frame (atom 3) (child-depth 7))"
        );
        assert_eq!(
            Frame::Positive {
                index: 3,
                negative: SearchResult::Unknown(UnknownReason::BudgetExhausted)
            }
            .to_string(),
            "(positive-frame (atom 3) (negative-result unknown))"
        );
    }
}
