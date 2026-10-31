//! Equality and evidence-based refutation for error-result types. Error labels
//! form a set, unlike ordered repeated effect occurrences. Failed matching is
//! never evidence that a label is absent or distinct.
use super::*;

impl Search<'_> {
    pub(super) fn error_result_distinct(
        &mut self,
        left: usize,
        right: usize,
        classes: &Classes,
        assignment: &[Option<bool>],
    ) -> Result<bool, UnknownReason> {
        if !matches!(self.terms.nodes[left], Term::ErrorResult { .. })
            && !matches!(self.terms.nodes[right], Term::ErrorResult { .. })
        {
            return Ok(false);
        }
        let Some((left, lr)) = self.error_result_view(left, classes)? else {
            return Ok(false);
        };
        let Some((right, rr)) = self.error_result_view(right, classes)? else {
            return Ok(false);
        };
        // An opaque success can add labels, but cannot remove an explicit one.
        // Absence needs a closed destination, not a closed source residual.
        let mut closed = [false; 2];
        for (index, term) in self.terms.nodes.iter().enumerate() {
            self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
            if constructor(term).is_some()
                || matches!(term, Term::Record(_))
                || self.context.is_tuple_type(term)
                || self.context.is_list_type(term)
            {
                let root = classes.root(index);
                closed[0] |= root == lr;
                closed[1] |= root == rr;
            }
        }
        if closed.into_iter().all(|known| known)
            && self.distinct_type_classes(lr, rr, classes, assignment, &mut BTreeSet::new(), 0)?
        {
            return Ok(true);
        }
        for (source, destination, destination_closed) in
            [(&left, &right, closed[1]), (&right, &left, closed[0])]
        {
            if !destination_closed {
                continue;
            }
            for label in source {
                let mut absent = true;
                for candidate in destination {
                    self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
                    if label == candidate
                        || !self.distinct_type_classes(
                            *label,
                            *candidate,
                            classes,
                            assignment,
                            &mut BTreeSet::new(),
                            0,
                        )?
                    {
                        absent = false;
                        break;
                    }
                }
                if absent {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    pub(super) fn error_result_congruent(
        &mut self,
        left: usize,
        right: usize,
        classes: &Classes,
    ) -> Result<bool, UnknownReason> {
        if !matches!(self.terms.nodes[left], Term::ErrorResult { .. })
            && !matches!(self.terms.nodes[right], Term::ErrorResult { .. })
        {
            return Ok(false);
        }
        let Some(left) = self.error_result_view(left, classes)? else {
            return Ok(false);
        };
        let Some(right) = self.error_result_view(right, classes)? else {
            return Ok(false);
        };
        Ok(left == right)
    }
    fn error_result_view(
        &mut self,
        start: usize,
        classes: &Classes,
    ) -> Result<Option<(BTreeSet<usize>, usize)>, UnknownReason> {
        if has_bound(&self.terms.nodes[start]) {
            return Ok(None);
        }
        let terms = self.terms;
        let mut labels = BTreeSet::new();
        let mut current = start;
        for _ in 0..self.max_depth.min(128) {
            self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
            let Term::ErrorResult { errors, result } = &terms.nodes[current] else {
                return Ok(Some((labels, classes.root(current))));
            };
            for error in errors {
                self.tick().map_err(|()| UnknownReason::BudgetExhausted)?;
                labels.insert(classes.root(terms.ids[error]));
            }
            current = terms.ids[result.as_ref()];
        }
        Err(UnknownReason::BudgetExhausted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frozen_types(fields: Vec<nir::Term>) -> nir::Term {
        nir::Term::NominalValue {
            ty: Box::new(nir::Term::NormalForm {
                head: nir::DeclarationId {
                    package: 87,
                    local: 3,
                },
                sort: nir::Sort::Type,
                arguments: vec![nir::Term::Primitive(nir::Primitive::Type)],
            }),
            variant: None,
            fields,
        }
    }

    #[test]
    fn tuple_error_absence_requires_registration_without_unpacking_the_head() {
        let head = DeclarationId {
            package: 87,
            local: 1,
        };
        let tuple = |field| Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![frozen_types(vec![field])],
        };
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let character = Term::Primitive(nir::Primitive::Char);
        let open = Term::error_result(
            vec![boolean.clone()],
            Term::Rigid {
                declaration: DeclarationId {
                    package: 87,
                    local: 2,
                },
                sort: Sort::Type,
            },
        );
        let nested = tuple(Term::error_result(vec![boolean.clone()], character.clone()));
        let mut context = Context::default();
        context.admit_congruence(head);
        context.admit_tuple_head(head);
        for goal in [
            Formula::equal(open.clone(), nested.clone()).negate(),
            Formula::equal(nested, open).negate(),
        ] {
            let Outcome::Entailed(proof) = context.prove(goal.clone(), Budget::default()) else {
                panic!("Tuple itself is ordinary even when one element is error-qualified");
            };
            context.verify_proof(&proof, Budget::default()).unwrap();
            let mut opaque = context.fork();
            std::sync::Arc::make_mut(&mut opaque.tuple_heads).clear();
            assert!(opaque.verify_proof(&proof, Budget::default()).is_err());
            assert!(!matches!(
                opaque.prove(goal, Budget::default()),
                Outcome::Entailed(_)
            ));
        }
        assert!(
            !matches!(
                context.prove(
                    Formula::equal(tuple(boolean), tuple(character)).negate(),
                    Budget::default()
                ),
                Outcome::Entailed(_)
            ),
            "classification must not introduce Tuple injectivity"
        );
    }
}
