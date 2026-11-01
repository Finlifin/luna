//! Source-literal certificates for integer difference bounds. Verification
//! checks an ordered path, never a cached model or a retained solver graph.
use super::{IntLe, tick};
use crate::{Atom, ProofError, UnknownReason};
use std::{fmt, sync::Arc};

#[derive(Clone, Debug)]
pub struct IntegerBoundProof {
    pub path: Arc<[(IntLe, bool)]>,
    pub conclusion: Arc<IntLe>,
    pub value: bool,
}
impl IntegerBoundProof {
    pub(super) fn check_bound(&self, steps: &mut usize) -> Result<bool, UnknownReason> {
        tick(steps)?;
        let (from, to, bound) = self.conclusion.directed(self.value)?;
        let mut current = from.variable;
        let mut total = 0i128;
        for (literal, truth) in self.path.iter() {
            tick(steps)?;
            let (edge_from, edge_to, weight) = literal.directed(*truth)?;
            if current != edge_from.variable {
                return Ok(false);
            }
            total = total
                .checked_add(weight)
                .ok_or(UnknownReason::ArithmeticRange)?;
            current = edge_to.variable;
        }
        Ok(current == to.variable && total <= bound)
    }
    pub(crate) fn verify(
        &self,
        atoms: &[Atom],
        assignment: &[Option<bool>],
        steps: &mut usize,
    ) -> Result<(), ProofError> {
        // The conclusion is present with the opposite polarity. Every path
        // edge must be a current source premise with its exact signed meaning.
        for (literal, truth) in std::iter::once((self.conclusion.as_ref(), !self.value))
            .chain(self.path.iter().map(|(literal, truth)| (literal, *truth)))
        {
            let mut found = false;
            for (atom, value) in atoms.iter().zip(assignment) {
                *steps = steps.checked_sub(1).ok_or(ProofError::BudgetExhausted)?;
                if matches!(atom, Atom::IntLe(candidate) if candidate == literal) {
                    if *value != Some(truth) {
                        return Err(ProofError::Invalid);
                    }
                    found = true;
                    break;
                }
            }
            if !found {
                return Err(ProofError::Invalid);
            }
        }
        match self.check_bound(steps) {
            Ok(true) => Ok(()),
            Ok(false) => Err(ProofError::Invalid),
            Err(UnknownReason::ArithmeticRange) => Err(ProofError::ArithmeticRange),
            Err(_) => Err(ProofError::BudgetExhausted),
        }
    }
}
impl fmt::Display for IntegerBoundProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(integer-bound (conclusion {} {}) (path",
            self.value, self.conclusion
        )?;
        for (literal, value) in self.path.iter() {
            write!(f, " (literal {value} {literal})")?;
        }
        write!(f, "))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integer::{IntegerTerm, propagate_explained};
    use nir::DeclarationId;
    fn variable(local: u32, offset: i128) -> IntegerTerm {
        IntegerTerm::variable(
            DeclarationId {
                package: 106,
                local,
            },
            offset,
        )
    }
    #[test]
    fn signed_path_certificates_require_connected_source_premises() {
        let a = IntLe {
            left: variable(0, 1),
            right: variable(1, 0),
        };
        let b = IntLe {
            left: variable(1, 0),
            right: variable(2, 0),
        };
        for truth in [true, false] {
            let c = if truth {
                IntLe {
                    left: variable(0, 1),
                    right: variable(2, 0),
                }
            } else {
                IntLe {
                    left: variable(2, 0),
                    right: variable(0, 0),
                }
            };
            let stronger = if truth {
                IntLe {
                    left: variable(0, 2),
                    right: variable(2, 0),
                }
            } else {
                IntLe {
                    left: variable(2, 0),
                    right: variable(0, 1),
                }
            };
            let unrelated = IntLe {
                left: variable(4, 0),
                right: variable(5, 0),
            };
            let (index, proof, support) = propagate_explained(
                &[
                    (0, &a, Some(true)),
                    (1, &b, Some(true)),
                    (2, &c, None),
                    (3, &unrelated, Some(true)),
                ],
                &mut 10000,
            )
            .unwrap()
            .unwrap();
            assert_eq!((index, proof.value), (2, truth));
            assert_eq!(support, [0, 1, 2].into_iter().collect());
            let atoms: Vec<_> = [a.clone(), b.clone(), c.clone(), stronger.clone()]
                .into_iter()
                .map(Atom::IntLe)
                .collect();
            let assignment = [Some(true), Some(true), Some(!truth), Some(!truth)];
            let mut fuel = 10000;
            proof.verify(&atoms, &assignment, &mut fuel).unwrap();
            for budget in 0..10000 - fuel {
                assert_eq!(
                    proof.verify(&atoms, &assignment, &mut { budget }),
                    Err(ProofError::BudgetExhausted)
                );
            }
            let mut missing = assignment;
            missing[0] = None;
            assert_eq!(
                proof.verify(&atoms, &missing, &mut 1000),
                Err(ProofError::Invalid)
            );
            let mut wrong_polarity = assignment;
            wrong_polarity[2] = Some(truth);
            assert_eq!(
                proof.verify(&atoms, &wrong_polarity, &mut 1000),
                Err(ProofError::Invalid)
            );
            let mut forged = proof.clone();
            Arc::make_mut(&mut forged.path).reverse();
            assert_eq!(
                forged.verify(&atoms, &assignment, &mut 1000),
                Err(ProofError::Invalid)
            );
            forged = proof.clone();
            forged.conclusion = Arc::new(stronger);
            assert_eq!(
                forged.verify(&atoms, &assignment, &mut 1000),
                Err(ProofError::Invalid)
            );
            forged = proof.clone();
            forged.path = Arc::from([]);
            assert_eq!(
                forged.verify(&atoms, &assignment, &mut 1000),
                Err(ProofError::Invalid)
            );
            proof.verify(&atoms, &assignment, &mut 1000).unwrap();
            assert!(proof.to_string().starts_with("(integer-bound (conclusion"));
        }
    }
    #[test]
    fn direct_path_avoids_negative_cycle_overflow_without_widening_arithmetic() {
        let premise = IntLe {
            left: variable(0, 0),
            right: variable(1, i128::MIN),
        };
        let implied = IntLe {
            left: variable(0, -1),
            right: variable(1, i128::MIN),
        };
        let (_, proof, _) =
            propagate_explained(&[(0, &premise, Some(true)), (1, &implied, None)], &mut 1000)
                .unwrap()
                .unwrap();
        proof
            .verify(
                &[Atom::IntLe(premise.clone()), Atom::IntLe(implied)],
                &[Some(true), Some(false)],
                &mut 1000,
            )
            .unwrap();
        let disconnected = IntLe {
            left: variable(2, 0),
            right: variable(3, 0),
        };
        let forged = IntegerBoundProof {
            path: vec![(premise, true)].into(),
            conclusion: Arc::new(disconnected.clone()),
            value: true,
        };
        assert_eq!(forged.check_bound(&mut 1000), Ok(false));
        let empty = IntegerBoundProof {
            path: Arc::from([]),
            conclusion: Arc::new(disconnected),
            value: true,
        };
        assert_eq!(empty.check_bound(&mut 1000), Ok(false));
        let reflexive = IntLe {
            left: variable(0, 0),
            right: variable(0, 0),
        };
        let empty = IntegerBoundProof {
            path: Arc::from([]),
            conclusion: Arc::new(reflexive.clone()),
            value: true,
        };
        empty
            .verify(&[Atom::IntLe(reflexive)], &[Some(false)], &mut 1000)
            .unwrap();
        let overflow = IntegerBoundProof {
            path: vec![
                (
                    IntLe {
                        left: variable(1, 0),
                        right: variable(0, i128::MAX),
                    },
                    true,
                ),
                (
                    IntLe {
                        left: variable(2, 0),
                        right: variable(1, 1),
                    },
                    true,
                ),
            ]
            .into(),
            conclusion: Arc::new(IntLe {
                left: variable(2, 0),
                right: variable(0, i128::MAX),
            }),
            value: true,
        };
        assert_eq!(
            overflow.check_bound(&mut 1000),
            Err(UnknownReason::ArithmeticRange)
        );
    }
}
