//! Completed associated Type identities delivered across a single package freeze.
use super::{DeclarationId, SourceRange, Term};
use crate::associated_initializers::ProgramIdentity;
use crate::type_computations::{Error, charge_term};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

/// The projection key includes its origin, receiver and interface operands.
/// The selected implementation and its substitution distinguish the evidence
/// under which the source observation was made.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObservedAssociatedTypes(HashMap<Term, Vec<Delivery>>);

#[derive(Clone, Debug, PartialEq)]
struct Delivery {
    fact: Arc<ObservedAssociatedType>,
    conflict: Option<SourceRange>,
}

#[derive(Debug, PartialEq)]
pub struct ObservedAssociatedType {
    pub implementation: DeclarationId,
    pub arguments: Vec<Term>,
    pub program: ProgramIdentity,
    pub result: Term,
    pub source: SourceRange,
}

impl ObservedAssociatedTypes {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.values().map(Vec::len).sum()
    }

    /// Stage only the affected buckets. No delivery becomes visible if
    /// retaining the last result in a traversal runs out of fuel.
    pub fn publish(
        &mut self,
        incoming: Vec<(Term, ObservedAssociatedType)>,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        use std::collections::hash_map::Entry;
        let mut changed = HashMap::new();
        for (projection, fact) in incoming {
            charge_term(&projection, fuel)?;
            let entries = match changed.entry(projection) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => {
                    let prior = self
                        .0
                        .get(entry.key())
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    *fuel = fuel.checked_sub(prior.len()).ok_or(Error::Budget)?;
                    entry.insert(prior.to_vec())
                }
            };
            let mut matched = false;
            for prior in entries.iter_mut() {
                if prior
                    .fact
                    .matches(fact.implementation, &fact.arguments, &fact.program, fuel)?
                {
                    charge_term(&fact.result, fuel)?;
                    if fact.result != prior.fact.result {
                        prior.conflict.get_or_insert(fact.source);
                    }
                    matched = true;
                    break;
                }
            }
            if !matched {
                entries.push(Delivery {
                    fact: Arc::new(fact),
                    conflict: None,
                });
            }
        }
        self.0.extend(changed);
        Ok(())
    }

    /// Query only after selection and formation in the consuming world.
    pub fn lookup(
        &self,
        projection: &Term,
        implementation: DeclarationId,
        arguments: &[Term],
        program: &ProgramIdentity,
        fuel: &mut usize,
    ) -> Result<Option<&Term>, Error> {
        if self.is_empty() {
            return Ok(None);
        }
        charge_term(projection, fuel)?;
        if let Some(entries) = self.0.get(projection) {
            for entry in entries {
                if entry
                    .fact
                    .matches(implementation, arguments, program, fuel)?
                {
                    if let Some(conflict) = entry.conflict {
                        return Err(Error::Structure(format!(
                            "associated Type observations at {} and {} disagree for {projection}",
                            entry.fact.source, conflict
                        )));
                    }
                    return Ok(Some(&entry.fact.result));
                }
            }
        }
        Ok(None)
    }
}

impl ObservedAssociatedType {
    fn matches(
        &self,
        implementation: DeclarationId,
        arguments: &[Term],
        program: &ProgramIdentity,
        fuel: &mut usize,
    ) -> Result<bool, Error> {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        if self.implementation != implementation
            || &self.program != program
            || self.arguments.len() != arguments.len()
        {
            return Ok(false);
        }
        for (left, right) in self.arguments.iter().zip(arguments) {
            charge_term(right, fuel)?;
            if left != right {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl fmt::Display for ObservedAssociatedTypes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut records = vec![];
        for (projection, entries) in &self.0 {
            for delivery in entries {
                let entry = &delivery.fact;
                use std::fmt::Write;
                let mut text = format!(
                    "(observation {projection} (implementation {}) (arguments",
                    entry.implementation
                );
                for arg in &entry.arguments {
                    write!(text, " {arg}")?;
                }
                write!(text, ") (result {}) {}", entry.result, entry.source)?;
                if let Some(other) = delivery.conflict {
                    write!(text, " (conflict {other})")?;
                }
                text.push(')');
                records.push(text);
            }
        }
        records.sort_unstable();
        write!(f, "(observed-associated-types")?;
        for record in records {
            write!(f, " {record}")?;
        }
        f.write_str(")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::associated_initializers::AssociatedInitializer;
    use crate::program::{Expr, ExprKind};
    use crate::{AssociatedTypeOrigin, Primitive};

    #[test]
    fn delivery_requires_the_same_selection_and_publishes_a_whole_batch() {
        let id = |local| DeclarationId { package: 1, local };
        let boolean = Term::Primitive(Primitive::Bool);
        let char_ty = Term::Primitive(Primitive::Char);
        let origin = AssociatedTypeOrigin {
            interface: id(2),
            member: id(3),
        };
        let projection = |input| Term::AssociatedType {
            origin: Box::new(origin.clone()),
            subject: Box::new(boolean.clone()),
            arguments: vec![input],
        };
        let yes = projection(Term::Bool(true));
        let no = projection(Term::Bool(false));
        let program = AssociatedInitializer::new(
            Expr {
                source: SourceRange { start: 0, end: 1 },
                ty: Term::Primitive(Primitive::Type),
                kind: ExprKind::TypeValue(boolean.clone()),
            },
            &mut 1_000,
        )
        .unwrap();
        let token = program.identity();
        let fact = |flag, result, start| ObservedAssociatedType {
            implementation: id(4),
            arguments: vec![Term::Bool(flag)],
            program: token.clone(),
            result,
            source: SourceRange {
                start,
                end: start + 1,
            },
        };
        let mut table = ObservedAssociatedTypes::default();
        table
            .publish(
                vec![(yes.clone(), fact(true, boolean.clone(), 1))],
                &mut 1_000,
            )
            .unwrap();
        assert!(
            table
                .lookup(&yes, id(9), &[Term::Bool(true)], &token, &mut 1_000)
                .unwrap()
                .is_none()
        );
        assert!(
            table
                .lookup(&yes, id(4), &[Term::Bool(false)], &token, &mut 1_000)
                .unwrap()
                .is_none()
        );
        let expensive = projection(Term::String("x".repeat(4096).into()));
        let before = table.clone();
        assert!(
            table
                .publish(
                    vec![
                        (no.clone(), fact(false, char_ty.clone(), 2)),
                        (expensive, fact(true, boolean.clone(), 3))
                    ],
                    &mut 100
                )
                .is_err()
        );
        assert_eq!(before, table);
        table
            .publish(
                vec![
                    (yes.clone(), fact(true, char_ty.clone(), 4)),
                    (no.clone(), fact(false, char_ty.clone(), 5)),
                ],
                &mut 1_000,
            )
            .unwrap();
        let issue = table
            .lookup(&yes, id(4), &[Term::Bool(true)], &token, &mut 1_000)
            .unwrap_err()
            .to_string();
        assert!(
            issue.contains("disagree")
                && issue.contains("(source 1 2)")
                && issue.contains("(source 4 5)"),
            "{issue}"
        );
        assert_eq!(
            table
                .lookup(&no, id(4), &[Term::Bool(false)], &token, &mut 1_000)
                .unwrap(),
            Some(&char_ty)
        );
    }
}
