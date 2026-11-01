//! Explained theory propagation from an already checked, reversible closure.
//! Model representatives never become premises or inferred equalities here.
use super::{Classes, Search};
use crate::{Atom, Conflict, ProofTree, TypeKind};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct Unit {
    pub index: usize,
    pub value: bool,
    pub reason: Conflict,
}
fn charge(fuel: &mut usize, work: usize) -> Option<()> {
    *fuel = fuel.checked_sub(work)?;
    Some(())
}
impl Classes {
    fn metered_root(&self, mut index: usize, fuel: &mut usize) -> Option<usize> {
        loop {
            charge(fuel, 1)?;
            if self.parent[index] == index {
                return Some(index);
            }
            index = self.parent[index];
        }
    }
}
impl Search<'_> {
    // Called only after successful theory synchronization for this assignment.
    // Propagation is optional; interruption publishes neither a literal nor
    // an incomplete explanation and does not modify retained closure state.
    pub(crate) fn theory_unit(&mut self, assignment: &[Option<bool>]) -> Option<Unit> {
        let allowance = (self.steps / 8).min(1024);
        let mut work = allowance;
        let result = self.find_theory_unit(assignment, &mut work);
        self.steps -= allowance - work;
        result
    }
    fn find_theory_unit(&self, assignment: &[Option<bool>], fuel: &mut usize) -> Option<Unit> {
        let state = self.theory_state.as_ref()?;
        if state.layers.is_empty() || !state.pending.is_empty() {
            return None;
        }
        enum Query {
            Equality((usize, usize)),
            Trait((nir::DeclarationId, Vec<usize>)),
            Kind(usize, TypeKind),
        }
        let mut unequal = BTreeSet::new();
        let mut traits = BTreeMap::new();
        let mut kinds = Vec::new();
        let mut integers = Vec::new();
        let mut pending_integer = false;
        let mut pending = Vec::new();
        for (index, (atom, value)) in self.atoms.iter().zip(assignment).enumerate() {
            charge(fuel, 1)?;
            let query = match atom {
                Atom::TypeEqual(equality) => {
                    if *value == Some(true) {
                        continue;
                    }
                    let a = state
                        .classes
                        .metered_root(self.terms.ids[&equality.0], fuel)?;
                    let b = state
                        .classes
                        .metered_root(self.terms.ids[&equality.1], fuel)?;
                    let pair = (a.min(b), a.max(b));
                    charge(fuel, 1)?;
                    if *value == Some(false) {
                        unequal.insert(pair);
                        continue;
                    }
                    Query::Equality(pair)
                }
                Atom::Trait(query) => {
                    let mut inputs = Vec::new();
                    for input in query.inputs() {
                        charge(fuel, 1)?;
                        inputs.push(state.classes.metered_root(self.terms.ids[input], fuel)?);
                    }
                    let key = (query.interface, inputs);
                    if let Some(value) = value {
                        traits.insert(key, *value);
                        continue;
                    }
                    Query::Trait(key)
                }
                Atom::Kind(query) => {
                    let root = state
                        .classes
                        .metered_root(self.terms.ids[&query.ty], fuel)?;
                    if let Some(value) = value {
                        kinds.push((root, query.kind, *value));
                        continue;
                    }
                    Query::Kind(root, query.kind)
                }
                Atom::IntLe(atom) => {
                    integers.push((index, atom, *value));
                    pending_integer |= value.is_none();
                    continue;
                }
            };
            pending.push((index, query));
        }
        for (index, query) in pending {
            charge(fuel, 1)?;
            let value = match query {
                Query::Equality(pair) => {
                    if pair.0 == pair.1 {
                        Some(true)
                    } else if unequal.contains(&pair) {
                        Some(false)
                    } else {
                        None
                    }
                }
                Query::Trait(key) => traits.get(&key).copied(),
                Query::Kind(root, query) => {
                    let mut inferred = None;
                    for &(known_root, known, truth) in &kinds {
                        charge(fuel, 1)?;
                        if root == known_root {
                            inferred = known.consequence(truth, query);
                            if inferred.is_some() {
                                break;
                            }
                        }
                    }
                    if inferred.is_none() {
                        for (id, term) in self.terms.nodes.iter().enumerate() {
                            charge(fuel, 1)?;
                            if state.classes.metered_root(id, fuel)? == root {
                                inferred = self.context.evaluate_kind(query, term);
                                if inferred.is_some() {
                                    break;
                                }
                            }
                        }
                    }
                    inferred
                }
            };
            if let Some(value) = value {
                return self.explain_unit(assignment, index, value, fuel);
            }
        }
        if pending_integer {
            let (index, proof, support) =
                crate::integer::propagate_explained(&integers, fuel).ok()??;
            return Some(Unit {
                index,
                value: proof.value,
                reason: Conflict {
                    tree: ProofTree::IntegerBound(proof),
                    support,
                },
            });
        }
        None
    }
    fn explain_unit(
        &self,
        assignment: &[Option<bool>],
        index: usize,
        value: bool,
        fuel: &mut usize,
    ) -> Option<Unit> {
        // Retain all admitted source premises. Negating this target contradicts
        // checked theory; no selected model or implementation absence is used.
        charge(fuel, assignment.len().saturating_mul(2).saturating_add(1))?;
        let literals = assignment.iter().enumerate().filter_map(|(i, assigned)| {
            let value = if i == index { Some(!value) } else { *assigned };
            value.map(|value| (self.atoms[i].clone(), value))
        });
        let mut support: BTreeSet<_> = assignment
            .iter()
            .enumerate()
            .filter_map(|(i, value)| value.is_some().then_some(i))
            .collect();
        support.insert(index);
        Some(Unit {
            index,
            value,
            reason: Conflict {
                tree: ProofTree::TheoryConflict(literals.collect()),
                support,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Context, Formula, Terms, collect, learning};
    use nir::{DeclarationId, Sort, Term};

    #[test]
    fn predicate_units_preserve_classification_and_trait_input_boundaries() {
        let rigid = |local| Term::Rigid {
            declaration: DeclarationId {
                package: 102,
                local,
            },
            sort: Sort::Type,
        };
        let kind = |local, kind| {
            Formula::Kind(crate::KindQuery {
                ty: rigid(local),
                kind,
            })
        };
        let tr = |interface, subject, argument| {
            Formula::Trait(crate::TraitQuery {
                interface: DeclarationId {
                    package: 102,
                    local: interface,
                },
                subject: rigid(subject),
                arguments: vec![rigid(argument)],
            })
        };
        let indexed = |subject, index| {
            Formula::Trait(crate::TraitQuery {
                interface: DeclarationId {
                    package: 102,
                    local: 12,
                },
                subject: rigid(subject),
                arguments: vec![Term::Rigid {
                    declaration: DeclarationId {
                        package: 102,
                        local: index,
                    },
                    sort: Sort::ComptimeValue,
                }],
            })
        };
        let formulas = [
            Formula::And(vec![
                Formula::equal(rigid(0), Term::Primitive(nir::Primitive::U8)),
                kind(0, TypeKind::UnsignedInteger),
                kind(0, TypeKind::SignedInteger),
                kind(0, TypeKind::Integer),
            ]),
            Formula::And(vec![
                Formula::equal(rigid(0), rigid(1)),
                indexed(0, 4),
                indexed(1, 4),
                indexed(1, 5),
            ]),
            Formula::And(vec![
                Formula::equal(rigid(0), rigid(1)),
                kind(0, TypeKind::SignedInteger),
                kind(1, TypeKind::Integer),
                kind(1, TypeKind::Boolean),
                kind(0, TypeKind::FloatingPoint),
            ]),
            Formula::And(vec![
                Formula::equal(rigid(0), rigid(1)),
                tr(10, 0, 2),
                tr(10, 1, 2),
                tr(10, 1, 3),
                tr(11, 1, 2),
                Formula::equal(rigid(2), rigid(3)),
            ]),
        ];
        let mut kinds_seen = [false; 2];
        let mut traits_seen = [false; 2];
        for formula in formulas {
            let context = Context::default();
            let mut atoms = Vec::new();
            let mut terms = Terms::default();
            collect(&formula, &mut atoms, &mut terms, 0, 128, &[]).unwrap();
            let mut search = Search {
                boolean: None,
                theory_state: None,
                learned: learning::Clauses::default(),
                context: &context,
                formula: &formula,
                atoms: &atoms,
                terms: &terms,
                steps: 100000,
                max_depth: 128,
            };
            let count = 3usize.pow(atoms.len() as u32);
            for code in (0..count).chain((0..count).rev()) {
                let mut code = code;
                let assignment: Vec<_> = (0..atoms.len())
                    .map(|_| {
                        let value = [None, Some(false), Some(true)][code % 3];
                        code /= 3;
                        value
                    })
                    .collect();
                search.steps = 100000;
                if search.theory_cached(&assignment).unwrap().is_none() {
                    continue;
                }
                if let Some(unit) = search.theory_unit(&assignment) {
                    match &atoms[unit.index] {
                        Atom::Kind(_) => kinds_seen[unit.value as usize] = true,
                        Atom::Trait(_) => traits_seen[unit.value as usize] = true,
                        _ => {}
                    }
                    let mut opposite = assignment.clone();
                    opposite[unit.index] = Some(!unit.value);
                    search.steps = 100000;
                    search
                        .check_tree(&unit.reason.tree, &mut opposite, 0)
                        .unwrap();
                    opposite[unit.index] = Some(unit.value);
                    search.steps = 100000;
                    assert!(search.theory(&opposite).unwrap().is_some());
                }
            }
            let empty = vec![None; atoms.len()];
            search.steps = 100000;
            assert!(search.theory_cached(&empty).unwrap().is_some());
            assert!(search.theory_unit(&empty).is_none());
        }
        assert_eq!(kinds_seen, [true, true]);
        assert_eq!(traits_seen, [true, true]);
    }

    #[test]
    fn theory_units_replay_across_branches_and_interrupted_budgets() {
        let rigid = |local| Term::Rigid {
            declaration: DeclarationId {
                package: 100,
                local,
            },
            sort: Sort::Type,
        };
        let formula = Formula::And(vec![
            Formula::equal(rigid(0), rigid(1)),
            Formula::equal(rigid(1), rigid(2)),
            Formula::equal(rigid(0), rigid(2)),
            Formula::equal(
                Term::Optional(Box::new(rigid(0))),
                Term::Optional(Box::new(rigid(2))),
            ),
        ]);
        let context = Context::default();
        let mut atoms = Vec::new();
        let mut terms = Terms::default();
        collect(&formula, &mut atoms, &mut terms, 0, 128, &[]).unwrap();
        let mut search = Search {
            boolean: None,
            theory_state: None,
            learned: learning::Clauses::default(),
            context: &context,
            formula: &formula,
            atoms: &atoms,
            terms: &terms,
            steps: 100000,
            max_depth: 128,
        };
        let mut positive = 0;
        let mut negative = 0;
        for code in (0..81).chain((0..81).rev()) {
            let mut code = code;
            let assignment: Vec<_> = (0..4)
                .map(|_| {
                    let value = [None, Some(false), Some(true)][code % 3];
                    code /= 3;
                    value
                })
                .collect();
            search.steps = 100000;
            if search.theory_cached(&assignment).unwrap().is_none() {
                continue;
            }
            let before = search.theory_state.as_ref().unwrap().to_string();
            if let Some(unit) = search.theory_unit(&assignment) {
                assert!(assignment[unit.index].is_none());
                if unit.value {
                    positive += 1;
                } else {
                    negative += 1;
                }
                let mut opposite = assignment.clone();
                opposite[unit.index] = Some(!unit.value);
                search.steps = 100000;
                search
                    .check_tree(&unit.reason.tree, &mut opposite, 0)
                    .unwrap();
                assert!(search.theory(&opposite).unwrap().is_none());
                opposite[unit.index] = Some(unit.value);
                search.steps = 100000;
                assert!(
                    search
                        .check_tree(&unit.reason.tree, &mut opposite, 0)
                        .is_err()
                );
                search.steps = 100000;
                assert!(search.theory(&opposite).unwrap().is_some());
            }
            assert_eq!(search.theory_state.as_ref().unwrap().to_string(), before);
        }
        assert!(positive > 0 && negative > 0);
        let assignment = [Some(true), Some(true), None, None];
        search.steps = 100000;
        assert!(search.theory_cached(&assignment).unwrap().is_some());
        let before = search.theory_state.as_ref().unwrap().to_string();
        let mut interrupted = false;
        let mut propagated = false;
        for budget in 0..512 {
            search.steps = budget;
            if let Some(unit) = search.theory_unit(&assignment) {
                assert_eq!((unit.index, unit.value), (2, true));
                let mut opposite = assignment;
                opposite[2] = Some(false);
                search.steps = 100000;
                search
                    .check_tree(&unit.reason.tree, &mut opposite, 0)
                    .unwrap();
                propagated = true;
            } else {
                interrupted = true;
            }
            assert_eq!(search.theory_state.as_ref().unwrap().to_string(), before);
        }
        assert!(interrupted && propagated);
        // An unconstrained model may pick equal representatives, but this is
        // not a logical equality and must not become a propagated literal.
        search.steps = 100000;
        assert!(search.theory_cached(&[None; 4]).unwrap().is_some());
        assert!(search.theory_unit(&[None; 4]).is_none());
    }
}
