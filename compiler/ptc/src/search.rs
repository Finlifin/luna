//! Explicit decision continuations. Logical depth remains metered, while
//! branch restoration and backjumping no longer depend on the Rust call stack.
use super::{Conflict, Search, SearchResult, UnknownReason, boolean, learning};
use std::fmt;

#[cfg(test)]
thread_local! { static BACKJUMPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

fn backjump(
    frames: &mut Vec<Frame>,
    assignment: &mut [Option<bool>],
    levels: &mut [usize],
    decision_level: &mut usize,
    target: usize,
) {
    debug_assert!(target < *decision_level);
    for (value, level) in assignment.iter_mut().zip(levels) {
        if *level > target {
            *value = None;
            *level = 0;
        }
    }
    while *decision_level > target {
        match frames
            .pop()
            .expect("decision frame missing during backjump")
        {
            Frame::Negative { .. } | Frame::Positive { .. } => *decision_level -= 1,
            Frame::Boolean(_) | Frame::Learned { .. } => {}
        }
    }
    #[cfg(test)]
    BACKJUMPS.with(|n| n.set(n.get() + 1));
}

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
        let entry_depth = depth;
        let entry_assignments = assignment.iter().filter(|v| v.is_some()).count();
        let mut frames = Vec::new();
        let mut levels = vec![0usize; assignment.len()];
        let mut decision_level = 0usize;
        'search: loop {
            let mut result = match self.enter(assignment, depth) {
                Step::Descend(frame, child_depth) => {
                    match &frame {
                        Frame::Boolean(trail) => {
                            for (index, _) in trail {
                                levels[*index] = decision_level;
                            }
                        }
                        Frame::Learned { index, .. } => levels[*index] = decision_level,
                        Frame::Negative { index, .. } => {
                            decision_level += 1;
                            levels[*index] = decision_level;
                        }
                        Frame::Positive { .. } => unreachable!(),
                    }
                    frames.push(frame);
                    depth = child_depth;
                    continue;
                }
                Step::Return(result) => result,
            };
            let mut learned_at = None;
            loop {
                let (next, jump) = self.learn_first_uip(
                    result,
                    assignment,
                    &levels,
                    decision_level,
                    &mut learned_at,
                );
                result = next;
                if let Some(target) = jump {
                    backjump(
                        &mut frames,
                        assignment,
                        &mut levels,
                        &mut decision_level,
                        target,
                    );
                    depth = entry_depth + assignment.iter().filter(|v| v.is_some()).count()
                        - entry_assignments;
                    continue 'search;
                }
                let Some(frame) = frames.pop() else {
                    // A completed subquery may still depend on entry facts.
                    // Retain that conditional result even when a restart
                    // eliminated the final decision-resolution frame.
                    return match result {
                        SearchResult::Unsat(conflict) => {
                            SearchResult::Unsat(self.learn_resolved(conflict, assignment))
                        }
                        other => other,
                    };
                };
                match frame {
                    Frame::Boolean(trail) => {
                        for (index, value) in trail.into_iter().rev() {
                            let (next, jump) = self.learn_first_uip(
                                result,
                                assignment,
                                &levels,
                                decision_level,
                                &mut learned_at,
                            );
                            result = next;
                            if let Some(target) = jump {
                                backjump(
                                    &mut frames,
                                    assignment,
                                    &mut levels,
                                    &mut decision_level,
                                    target,
                                );
                                depth = entry_depth
                                    + assignment.iter().filter(|v| v.is_some()).count()
                                    - entry_assignments;
                                continue 'search;
                            }
                            assignment[index] = None;
                            levels[index] = 0;
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
                        levels[index] = 0;
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
                            levels[index] = 0;
                            decision_level -= 1;
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
                        levels[index] = 0;
                        decision_level -= 1;
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

    fn learn_first_uip(
        &mut self,
        result: SearchResult,
        assignment: &[Option<bool>],
        levels: &[usize],
        decision_level: usize,
        learned_at: &mut Option<usize>,
    ) -> (SearchResult, Option<usize>) {
        if decision_level == 0 || *learned_at == Some(decision_level) {
            return (result, None);
        }
        let SearchResult::Unsat(conflict) = result else {
            return (result, None);
        };
        let mut current = 0;
        for index in &conflict.support {
            // This optional analysis shares the query budget. Its failure does
            // not change the proof or permit any unsupported assignment.
            if self.steps == 0 {
                return (SearchResult::Unsat(conflict), None);
            }
            self.steps -= 1;
            if levels[*index] == decision_level {
                current += 1;
                if current > 1 {
                    break;
                }
            }
        }
        if current == 1 {
            *learned_at = Some(decision_level);
            let target = conflict
                .support
                .iter()
                .map(|index| levels[*index])
                .filter(|level| *level < decision_level)
                .max()
                .unwrap_or(0);
            let conflict = self.learn_resolved(conflict, assignment);
            let mut jump = None;
            if self.retains_conflict(&conflict, assignment) {
                let work = assignment.len().saturating_mul(3);
                if self.steps >= work {
                    self.steps -= work;
                    jump = Some(target);
                }
            }
            (SearchResult::Unsat(conflict), jump)
        } else {
            (SearchResult::Unsat(conflict), None)
        }
    }

    fn clause_conflict(&mut self, trace: Vec<usize>, assignment: &[Option<bool>]) -> Conflict {
        let mut conflict = Conflict::boolean(assignment);
        // Explanation is optional. Retain the original certificate and its
        // conservative support if the bounded dependency walk is interrupted.
        let allowance = (self.steps / 8).min(4096);
        let mut work = allowance;
        let explained = self
            .boolean
            .as_mut()
            .unwrap()
            .clauses(self.atoms.len(), &mut work)
            .and_then(|cnf| cnf.explain(&trace, assignment, &mut work));
        self.steps -= allowance - work;
        let trace = if let Ok((trace, support)) = explained {
            conflict.support = support;
            trace
        } else {
            trace
        };
        conflict.tree = super::ProofTree::ClauseConflict(trace);
        conflict
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
        if forced.is_empty() && propagation.truth.is_none() {
            let result = self
                .boolean
                .as_mut()
                .unwrap()
                .clauses(self.atoms.len(), &mut self.steps)
                .and_then(|cnf| cnf.propagate(assignment, &mut self.steps));
            use boolean::cnf::Propagation;
            match result {
                Err(()) => {
                    return Step::Return(SearchResult::Unknown(UnknownReason::BudgetExhausted));
                }
                Ok(Propagation::Conflict(trace)) => {
                    let conflict = self.clause_conflict(trace, assignment);
                    return Step::Return(SearchResult::Unsat(conflict));
                }
                Ok(Propagation::Unit {
                    index,
                    value,
                    trace,
                }) => {
                    assignment[index] = Some(!value);
                    let reason = self.clause_conflict(trace, assignment);
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
                Ok(Propagation::None) => {}
            }
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
                    .unwrap_or_else(|| {
                        SearchResult::Unknown(if self.steps == 0 {
                            UnknownReason::BudgetExhausted
                        } else {
                            UnknownReason::UnvalidatedModel
                        })
                    }),
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
    fn first_uip_retains_the_propagated_dominator_before_the_decision() {
        let atom = |local| {
            Formula::equal(
                Term::Rigid {
                    declaration: DeclarationId { package: 82, local },
                    sort: Sort::Type,
                },
                Term::Primitive(Primitive::Bool),
            )
        };
        let d = atom(0);
        let x = atom(1);
        let y = atom(2);
        // d=false forces x=true. x forces both y polarities, so x is the
        // first UIP; learning only d=true loses this stronger explanation.
        let formula = Formula::And(vec![
            Formula::Or(vec![d, x.clone()]),
            Formula::Or(vec![x.clone().negate(), y.clone()]),
            Formula::Or(vec![x.negate(), y.negate()]),
        ]);
        let context = Context::default();
        let mut atoms = Vec::new();
        let mut terms = Terms::default();
        collect(&formula, &mut atoms, &mut terms, 0, 128, &[]).unwrap();
        let mut search = Search {
            boolean: None,
            learned: learning::Clauses::default(),
            context: &context,
            formula: &formula,
            atoms: &atoms,
            terms: &terms,
            steps: 100_000,
            max_depth: 128,
        };
        let mut assignment = vec![None; 3];
        BACKJUMPS.with(|n| n.set(0));
        assert!(matches!(
            search.solve(&mut assignment, 0),
            SearchResult::Model(_)
        ));
        assert_eq!(assignment, vec![None; 3]);
        assert!(
            BACKJUMPS.with(|n| n.get()) > 0,
            "UIP learning did not restart propagation"
        );
        // The decision's opposite branch must still benefit from x=false.
        assignment[0] = Some(true);
        let learning::Propagation::Unit {
            index,
            value,
            reason,
        } = search.propagate_learned(&assignment).unwrap()
        else {
            panic!("first-UIP clause was not retained");
        };
        assert_eq!((index, value), (1, false));
        assert_eq!(reason.support.iter().copied().collect::<Vec<_>>(), vec![1]);
        assignment[1] = Some(true);
        search.check_tree(&reason.tree, &mut assignment, 0).unwrap();
        assignment[1] = Some(false);
        assert_eq!(
            search.check_tree(&reason.tree, &mut assignment, 0),
            Err(crate::ProofError::Invalid)
        );
    }

    #[test]
    fn asserting_backjump_preserves_the_lower_level_prefix() {
        let mut frames = vec![
            Frame::Boolean(vec![(0, true)]),
            Frame::Negative {
                index: 1,
                child_depth: 2,
            },
            Frame::Boolean(vec![(2, false)]),
            Frame::Negative {
                index: 3,
                child_depth: 4,
            },
            Frame::Boolean(vec![(4, true)]),
            Frame::Negative {
                index: 5,
                child_depth: 6,
            },
            Frame::Boolean(vec![(6, false)]),
        ];
        let mut assignment = vec![
            Some(true),
            Some(false),
            Some(false),
            Some(false),
            Some(true),
            Some(false),
            Some(false),
        ];
        let mut levels = vec![0, 1, 1, 2, 2, 3, 3];
        let mut level = 3;
        backjump(&mut frames, &mut assignment, &mut levels, &mut level, 1);
        assert_eq!(level, 1);
        assert_eq!(frames.len(), 3);
        assert_eq!(
            assignment,
            vec![Some(true), Some(false), Some(false), None, None, None, None]
        );
        assert_eq!(levels, vec![0, 1, 1, 0, 0, 0, 0]);
        backjump(&mut frames, &mut assignment, &mut levels, &mut level, 0);
        assert_eq!(frames.len(), 1);
        assert_eq!(
            assignment,
            vec![Some(true), None, None, None, None, None, None]
        );
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
    #[test]
    fn root_unit_reason_excludes_unrelated_entry_assignments() {
        let atom = |local| {
            Formula::equal(
                Term::Rigid {
                    declaration: DeclarationId { package: 89, local },
                    sort: Sort::Type,
                },
                Term::Primitive(Primitive::Bool),
            )
        };
        let a = atom(0);
        let b = atom(1);
        let formula = Formula::And(vec![
            Formula::Or(vec![a.clone(), a]),
            Formula::Or(vec![b.clone(), b.negate()]),
        ]);
        let context = Context::default();
        let mut atoms = Vec::new();
        let mut terms = Terms::default();
        collect(&formula, &mut atoms, &mut terms, 0, 128, &[]).unwrap();
        let mut search = Search {
            boolean: None,
            learned: learning::Clauses::default(),
            context: &context,
            formula: &formula,
            atoms: &atoms,
            terms: &terms,
            steps: 100000,
            max_depth: 128,
        };
        let mut assignment = [None, Some(true)];
        let Step::Descend(
            Frame::Learned {
                index,
                value,
                reason,
            },
            _,
        ) = search.enter(&mut assignment, 0)
        else {
            panic!("root clauses must supply a unit");
        };
        assert_eq!((index, value), (0, true));
        assert_eq!(reason.support, std::collections::BTreeSet::from([0]));
        assert_eq!(assignment, [Some(true), Some(true)]);
        search
            .check_tree(&reason.tree, &mut [Some(false), None], 0)
            .unwrap();
        assert!(
            search
                .check_tree(&reason.tree, &mut [None, Some(true)], 0)
                .is_err()
        );
    }
}
