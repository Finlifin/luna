//! Construct and replay concrete Type models. Candidate choices are private
//! search state, never equality evidence or a complete enumeration of Type.
use super::*;
mod combinations;
mod fresh;
pub(super) use fresh::candidate as fresh_candidate;

#[cfg(test)]
thread_local! {
    static GENERATED_ROUNDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Search<'_> {
    pub(super) fn witness(&mut self, theory: &TheoryModel) -> Option<Model> {
        // A bare theory model cannot interpret implementation-world predicates.
        // Detect that boundary before allocating type witnesses that would be
        // discarded regardless of their values.
        for atom in self.atoms {
            self.steps = self.steps.checked_sub(1)?;
            if matches!(atom, Atom::Trait(_)) {
                return None;
            }
        }
        let classes = &theory.classes;
        // Reify actual closed types, not arbitrary EUF domain elements. Missing
        // value-store, runtime or opaque-call interpretations keep Unknown.
        let mut representatives = HashMap::new();
        for (id, term) in self.terms.nodes.iter().enumerate() {
            if !matches!(
                term,
                Term::Rigid {
                    sort: Sort::Type,
                    ..
                } | Term::Bound {
                    sort: Sort::Type,
                    ..
                }
            ) && !model_applications::admitted(term, self.context)
            {
                representatives.entry(classes.root(id)).or_insert(id);
            }
        }
        // Classification selects a domain, not a single canonical model value.
        // Try combinations transactionally: disequalities and opaque application
        // collisions can invalidate a choice that satisfies every local kind.
        let mut domains = Vec::new();
        let mut expandable = Vec::new();
        let mut seen = BTreeSet::new();
        for &(root, _, _) in &theory.kinds {
            self.steps = self.steps.checked_sub(1)?;
            if representatives.contains_key(&root) || !seen.insert(root) {
                continue;
            }
            let mut candidates = Vec::new();
            let seeds = classification::witnesses();
            // A structural category has arbitrarily many Type inhabitants.
            // One representative cannot satisfy even two distinct pointer
            // inputs. Generate a bounded supply of fresh shapes; their count
            // is search coverage, never a claim that the category is exhausted.
            let structural = [TypeKind::Tuple, TypeKind::Pointer, TypeKind::Function];
            let mut needs_shapes = false;
            for category in structural {
                let mut permitted = true;
                for &(other, kind, truth) in &theory.kinds {
                    self.steps = self.steps.checked_sub(1)?;
                    if other == root && category.consequence(true, kind) != Some(truth) {
                        permitted = false;
                        break;
                    }
                }
                needs_shapes |= permitted;
            }
            // Try source and primitive candidates before allocating any fresh
            // structures. Even a large problem may already have a simple model.
            for candidate in seeds.iter().chain(self.terms.nodes.iter()) {
                nir::type_computations::charge_term(candidate, &mut self.steps).ok()?;
                if !closed_type(candidate, &self.context.nominal_constructors) {
                    continue;
                }
                let mut accepts = true;
                for &(other, kind, truth) in &theory.kinds {
                    self.steps = self.steps.checked_sub(1)?;
                    if other == root && kind.evaluate(candidate) != Some(truth) {
                        accepts = false;
                        break;
                    }
                }
                if accepts {
                    let mut duplicate = false;
                    for previous in &candidates {
                        nir::type_computations::charge_term(previous, &mut self.steps).ok()?;
                        duplicate |= previous == candidate;
                    }
                    if !duplicate {
                        candidates.push(candidate.clone());
                    }
                }
            }
            if candidates.is_empty() {
                return None;
            }
            domains.push((root, candidates));
            expandable.push(needs_shapes);
        }
        let mut sizes: Vec<_> = domains
            .iter()
            .map(|(_, candidates)| candidates.len())
            .collect();
        let mut previous_sizes = vec![0; domains.len()];
        let mut positions = combinations::first(&sizes, &previous_sizes)?;
        let mut next_width = 0;
        loop {
            // Meter the index traversal as well as actual model construction.
            self.steps = self
                .steps
                .checked_sub(domains.len().saturating_mul(8).saturating_add(1))?;
            let mut free = HashMap::new();
            for ((root, candidates), &position) in domains.iter().zip(&positions) {
                let candidate = &candidates[position];
                nir::type_computations::charge_term(candidate, &mut self.steps).ok()?;
                free.insert(*root, candidate.clone());
            }
            if let Some(model) = self.witness_with_choices(theory, &representatives, free) {
                return Some(model);
            }
            if combinations::advance(&mut positions, &sizes, &previous_sizes) {
                continue;
            }
            if !expandable.iter().any(|expand| *expand) {
                return None;
            }
            previous_sizes.clone_from(&sizes);
            loop {
                // This bound is search coverage, never a finite-domain axiom.
                if next_width > self.terms.nodes.len() {
                    return None;
                }
                #[cfg(test)]
                GENERATED_ROUNDS.with(|count| count.set(count.get() + 1));
                let generated = classification::structural_witnesses(next_width, &mut self.steps)?;
                next_width += 1;
                let mut added = false;
                for ((root, candidates), expand) in domains.iter_mut().zip(&expandable) {
                    self.steps = self.steps.checked_sub(1)?;
                    if !expand {
                        continue;
                    }
                    for candidate in &generated {
                        let mut accepts = true;
                        for &(other, kind, truth) in &theory.kinds {
                            self.steps = self.steps.checked_sub(1)?;
                            if other == *root && kind.evaluate(candidate) != Some(truth) {
                                accepts = false;
                                break;
                            }
                        }
                        if !accepts {
                            continue;
                        }
                        let mut duplicate = false;
                        for previous in candidates.iter() {
                            nir::type_computations::charge_term(previous, &mut self.steps).ok()?;
                            duplicate |= previous == candidate;
                        }
                        if !duplicate {
                            nir::type_computations::charge_term(candidate, &mut self.steps).ok()?;
                            candidates.push(candidate.clone());
                            added = true;
                        }
                    }
                }
                if added {
                    break;
                }
            }
            sizes = domains
                .iter()
                .map(|(_, candidates)| candidates.len())
                .collect();
            positions = combinations::first(&sizes, &previous_sizes)?;
        }
    }

    fn witness_with_choices(
        &mut self,
        theory: &TheoryModel,
        representatives: &HashMap<usize, usize>,
        mut free: HashMap<usize, Term>,
    ) -> Option<Model> {
        let classes = &theory.classes;
        let mut types = BTreeMap::new();
        let mut bound_types = BTreeMap::new();
        for term in &self.terms.nodes {
            if matches!(
                term,
                Term::Rigid {
                    sort: Sort::Type,
                    ..
                } | Term::Bound {
                    sort: Sort::Type,
                    ..
                }
            ) {
                let id = self.terms.ids[term];
                let value = reify(
                    id,
                    self.terms,
                    classes,
                    &representatives,
                    &mut free,
                    &mut BTreeSet::new(),
                    &self.context.nominal_constructors,
                    &mut self.steps,
                )?;
                match term {
                    Term::Rigid { declaration, .. } => {
                        types.insert(*declaration, value);
                    }
                    Term::Bound { depth, index, .. } => {
                        bound_types.insert(
                            TypeBinder {
                                depth: *depth,
                                index: *index,
                            },
                            value,
                        );
                    }
                    _ => unreachable!(),
                }
            }
        }
        let mut applications = Vec::new();
        let mut keys = HashMap::new();
        for (id, term) in self.terms.nodes.iter().enumerate() {
            self.steps = self.steps.checked_sub(1)?;
            if !model_applications::admitted(term, self.context) {
                continue;
            }
            let Term::NormalForm {
                head, arguments, ..
            } = term
            else {
                unreachable!()
            };
            let mut concrete = Vec::new();
            for argument in arguments {
                if model_applications::fixed_argument(argument) {
                    nir::type_computations::charge_term(argument, &mut self.steps).ok()?;
                    concrete.push(argument.clone());
                    continue;
                }
                concrete.push(reify(
                    self.terms.ids[argument],
                    self.terms,
                    classes,
                    &representatives,
                    &mut free,
                    &mut BTreeSet::new(),
                    &self.context.nominal_constructors,
                    &mut self.steps,
                )?);
            }
            let result = reify(
                id,
                self.terms,
                classes,
                &representatives,
                &mut free,
                &mut BTreeSet::new(),
                &self.context.nominal_constructors,
                &mut self.steps,
            )?;
            for value in concrete.iter().chain(std::iter::once(&result)) {
                nir::type_computations::charge_term(value, &mut self.steps).ok()?;
            }
            let key = (*head, concrete.clone());
            if let Some(previous) = keys.get(&key) {
                if previous != &result {
                    return None;
                }
            } else {
                keys.insert(key, result.clone());
                applications.push(TypeApplication {
                    head: *head,
                    arguments: concrete,
                    result,
                });
            }
        }
        let application_index = model_applications::index(
            &applications,
            self.context,
            &mut self.steps,
            self.max_depth.min(128),
        )
        .ok()?;
        // A Boolean short circuit must not conceal an uninterpreted atom in
        // a purported concrete model (opaque calls still require evidence).
        let mut atom_values = Vec::new();
        let evaluation_fuel = std::cell::Cell::new(self.steps);
        for atom in self.atoms {
            let value = model_atom(
                atom,
                &types,
                &bound_types,
                &application_index,
                &evaluation_fuel,
                &theory.integers,
                &self.context.nominal_constructors,
                &self.context.tuple_heads,
            );
            self.steps = evaluation_fuel.get();
            atom_values.push(value?);
        }
        let cnf = self
            .boolean
            .as_mut()?
            .clauses(self.atoms.len(), &mut self.steps)
            .ok()?;
        cnf.accepts(&atom_values, &mut self.steps)
            .ok()?
            .then_some(Model {
                applications,
                types,
                bound_types,
                integers: theory.integers.clone(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_type_inputs_do_not_require_deep_model_types() {
        let input = |local| Term::Rigid {
            declaration: DeclarationId {
                package: 986,
                local,
            },
            sort: Sort::Type,
        };
        let mut context = Context::default();
        context.assume(
            Formula::equal(
                Term::Tuple((0..24).map(input).collect()),
                Term::Tuple(vec![Term::Primitive(Primitive::Bool); 24]),
            )
            .negate(),
        );
        let budget = Budget {
            steps: 1_000_000,
            max_depth: 8,
        };
        let result = context.prove(Formula::False, budget);
        let Outcome::NotEntailed(witness) = result else {
            panic!("shallow independent inputs have a shallow model: {result:?}");
        };
        context.verify_witness(&witness, budget).unwrap();
    }

    #[test]
    fn fresh_model_types_avoid_explicit_shallow_shapes() {
        let variable = DeclarationId {
            package: 986,
            local: 0,
        };
        let input = Term::Rigid {
            declaration: variable,
            sort: Sort::Type,
        };
        let excluded: Vec<_> = [
            vec![],
            vec![Primitive::U8],
            vec![Primitive::Bool, Primitive::U8],
        ]
        .into_iter()
        .map(|fields| {
            Term::Pointer(Box::new(Term::Tuple(
                fields.into_iter().map(Term::Primitive).collect(),
            )))
        })
        .collect();
        let mut context = Context::default();
        for ty in &excluded {
            context.assume(Formula::equal(input.clone(), ty.clone()).negate());
        }
        let Outcome::NotEntailed(witness) = context.prove(Formula::False, Budget::default()) else {
            panic!("explicit shapes must be skipped when choosing a fresh model type");
        };
        assert!(!excluded.contains(&witness.types[&variable]));
        context.verify_witness(&witness, Budget::default()).unwrap();
    }

    #[test]
    fn existing_model_candidates_do_not_allocate_fresh_structures() {
        for kind in [TypeKind::Pointer, TypeKind::Tuple, TypeKind::Function] {
            let input = Term::Rigid {
                declaration: DeclarationId {
                    package: 985,
                    local: 1,
                },
                sort: Sort::Type,
            };
            let mut context = Context::default();
            context.assume(Formula::kind(input, kind));
            GENERATED_ROUNDS.with(|count| count.set(0));
            let Outcome::NotEntailed(witness) = context.prove(Formula::False, Budget::default())
            else {
                panic!("a seed already satisfies this classification")
            };
            assert_eq!(GENERATED_ROUNDS.with(|count| count.get()), 0);
            context.verify_witness(&witness, Budget::default()).unwrap();
        }
    }

    #[test]
    fn failed_seed_assignments_expand_then_validate_a_new_model() {
        let input = |local| Term::Rigid {
            declaration: DeclarationId {
                package: 985,
                local,
            },
            sort: Sort::Type,
        };
        let mut context = Context::default();
        context.assume(Formula::kind(input(0), TypeKind::Pointer));
        context.assume(Formula::kind(input(1), TypeKind::Pointer));
        context.assume(Formula::equal(input(0), input(1)).negate());
        GENERATED_ROUNDS.with(|count| count.set(0));
        let Outcome::NotEntailed(witness) = context.prove(Formula::False, Budget::default()) else {
            panic!("distinct pointer inputs require more than one seed")
        };
        assert!(GENERATED_ROUNDS.with(|count| count.get()) > 0);
        context.verify_witness(&witness, Budget::default()).unwrap();
        assert!(matches!(
            context.prove(
                Formula::False,
                Budget {
                    steps: 0,
                    ..Budget::default()
                }
            ),
            Outcome::Unknown {
                reason: UnknownReason::BudgetExhausted,
                ..
            }
        ));
        assert!(matches!(
            context.prove(Formula::False, Budget::default()),
            Outcome::NotEntailed(_)
        ));
    }
}
