//! Package-level type contracts that can be shared independently of lexical proofs.
use super::{Context, DeclarationId, ProofError};
use std::{collections::BTreeSet, fmt, sync::Arc};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeclarationTheory {
    pub(super) constructors: ConstructorContracts,
    /// Trusted List domain and candidate metadata; never a solver injectivity rule.
    pub(super) list_inference_heads: Arc<BTreeSet<DeclarationId>>,
}

/// Constructor rules used by proof search and replay. List inference hints
/// guide candidate generation and supply no rule for replaying a certificate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConstructorContracts {
    pub congruence_heads: Arc<BTreeSet<DeclarationId>>,
    pub nominal_constructors: Arc<BTreeSet<DeclarationId>>,
    /// Trusted Tuple classification/layout contracts; never body equations.
    pub tuple_heads: Arc<BTreeSet<DeclarationId>>,
}

impl ConstructorContracts {
    pub(super) fn verify(&self, recorded: &Self, steps: &mut usize) -> Result<(), ProofError> {
        for (current, recorded) in [
            (&self.congruence_heads, &recorded.congruence_heads),
            (&self.nominal_constructors, &recorded.nominal_constructors),
            (&self.tuple_heads, &recorded.tuple_heads),
        ] {
            if Arc::ptr_eq(current, recorded) {
                continue;
            }
            if current.len() != recorded.len() {
                return Err(ProofError::Invalid);
            }
            for (left, right) in current.iter().zip(recorded.iter()) {
                *steps = steps.checked_sub(1).ok_or(ProofError::BudgetExhausted)?;
                if left != right {
                    return Err(ProofError::Invalid);
                }
            }
        }
        Ok(())
    }
}

impl DeclarationTheory {
    /// Validate the declarations before exposing their theory to any query.
    pub fn from_package_metered(
        package: &nir::program::Package,
        fuel: &mut usize,
    ) -> Result<Self, nir::normal_forms::ValidationError> {
        nir::normal_forms::validate_in(package, fuel)?;
        let mut theory = Self::default();
        for declaration in package
            .structs
            .iter()
            .map(|s| s.declaration)
            .chain(package.enums.iter().map(|e| e.declaration))
            .chain(package.newtypes.iter().map(|n| n.declaration))
        {
            *fuel = fuel
                .checked_sub(1)
                .ok_or(nir::normal_forms::ValidationError::Budget)?;
            Arc::make_mut(&mut theory.constructors.nominal_constructors).insert(declaration);
        }
        for head in &package.normal_forms {
            *fuel = fuel
                .checked_sub(1)
                .ok_or(nir::normal_forms::ValidationError::Budget)?;
            Arc::make_mut(&mut theory.constructors.congruence_heads).insert(head.declaration);
            if head.role == nir::normal_forms::Role::TypeList {
                Arc::make_mut(&mut theory.list_inference_heads).insert(head.declaration);
            }
            if head.role == nir::normal_forms::Role::Tuple {
                Arc::make_mut(&mut theory.constructors.tuple_heads).insert(head.declaration);
            }
        }
        Ok(theory)
    }

    pub fn into_context(self) -> Context {
        Context {
            theory: self,
            scope: Arc::default(),
            assumptions: Arc::default(),
        }
    }
}

impl fmt::Display for DeclarationTheory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(declaration-theory {}", self.constructors)?;
        write_group(f, "list-inference-heads", &self.list_inference_heads)?;
        f.write_str(")")
    }
}

fn write_group(
    f: &mut fmt::Formatter<'_>,
    name: &str,
    ids: &BTreeSet<DeclarationId>,
) -> fmt::Result {
    write!(f, " ({name}")?;
    for id in ids {
        write!(f, " {id}")?;
    }
    f.write_str(")")
}
impl fmt::Display for ConstructorContracts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(constructor-contracts")?;
        write_group(f, "congruence-heads", &self.congruence_heads)?;
        write_group(f, "nominal-constructors", &self.nominal_constructors)?;
        write_group(f, "tuple-heads", &self.tuple_heads)?;
        f.write_str(")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Budget, Formula, Outcome};

    #[test]
    fn replay_shares_exact_sets_and_meters_equal_independent_support() {
        let mut original = Context::default();
        let mut independent = Context::default();
        for local in 0..512 {
            let id = DeclarationId { package: 1, local };
            original.admit_nominal_constructor(id);
            independent.admit_nominal_constructor(id);
        }
        independent.admit_list_inference_head(DeclarationId {
            package: 2,
            local: 0,
        });
        let Outcome::Entailed(proof) = original.prove(Formula::True, Budget::default()) else {
            panic!()
        };
        let Outcome::NotEntailed(witness) = original.prove(Formula::False, Budget::default())
        else {
            panic!()
        };
        assert!(Arc::ptr_eq(
            &proof.constructors.nominal_constructors,
            &witness.constructors.nominal_constructors
        ));
        let replay = |context: &Context, fuel: &mut Budget, counterexample| {
            if counterexample {
                context.verify_witness_metered(&witness, fuel)
            } else {
                context.verify_proof_metered(&proof, fuel)
            }
        };
        for counterexample in [false, true] {
            let mut shared = Budget {
                steps: 128,
                ..Default::default()
            };
            replay(&original, &mut shared, counterexample).unwrap();
            let mut limited = Budget {
                steps: 128,
                ..Default::default()
            };
            assert_eq!(
                replay(&independent, &mut limited, counterexample),
                Err(ProofError::BudgetExhausted)
            );
            assert_eq!(limited.steps, 0);
            let mut equivalent = Budget {
                steps: 128 + 512,
                ..Default::default()
            };
            replay(&independent, &mut equivalent, counterexample).unwrap();
            assert_eq!(equivalent.steps, shared.steps);
        }
    }
}
