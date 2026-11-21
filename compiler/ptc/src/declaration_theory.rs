//! Package-level type contracts that can be shared independently of lexical proofs.
use super::{Context, DeclarationId, ProofError};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

/// A head can carry identity authority before its declaration category is known.
pub type NominalConstructors = BTreeMap<DeclarationId, Option<nir::NominalKind>>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeclarationTheory {
    pub(super) constructors: ConstructorContracts,
    pub(super) formed_nominals: nir::program::FormedNullaryNominals,
    /// Trusted List domain and candidate metadata; never a solver injectivity rule.
    pub(super) list_inference_heads: Arc<BTreeSet<DeclarationId>>,
}

/// Constructor rules used by proof search and replay. List inference hints
/// guide candidate generation and supply no rule for replaying a certificate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConstructorContracts {
    pub congruence_heads: Arc<BTreeSet<DeclarationId>>,
    pub nominal_constructors: Arc<NominalConstructors>,
    /// Trusted Tuple classification/layout contracts; never body equations.
    pub tuple_heads: Arc<BTreeSet<DeclarationId>>,
}

impl ConstructorContracts {
    pub(super) fn verify(&self, recorded: &Self, steps: &mut usize) -> Result<(), ProofError> {
        for (current, recorded) in [
            (&self.congruence_heads, &recorded.congruence_heads),
            (&self.tuple_heads, &recorded.tuple_heads),
        ] {
            if Arc::ptr_eq(current, recorded) {
                continue;
            }
            verify_entries(current.iter(), recorded.iter(), steps)?;
        }
        if !Arc::ptr_eq(&self.nominal_constructors, &recorded.nominal_constructors) {
            verify_entries(
                self.nominal_constructors.iter(),
                recorded.nominal_constructors.iter(),
                steps,
            )?;
        }
        Ok(())
    }
}

fn verify_entries<T: PartialEq>(
    current: impl ExactSizeIterator<Item = T>,
    recorded: impl ExactSizeIterator<Item = T>,
    steps: &mut usize,
) -> Result<(), ProofError> {
    if current.len() != recorded.len() {
        return Err(ProofError::Invalid);
    }
    for (left, right) in current.zip(recorded) {
        *steps = steps.checked_sub(1).ok_or(ProofError::BudgetExhausted)?;
        if left != right {
            return Err(ProofError::Invalid);
        }
    }
    Ok(())
}

impl DeclarationTheory {
    /// Validate the declarations before exposing their theory to any query.
    pub fn from_package_metered(
        package: &nir::program::Package,
        fuel: &mut usize,
    ) -> Result<Self, nir::normal_forms::ValidationError> {
        nir::normal_forms::validate_in(package, fuel)?;
        let mut theory = Self::default();
        use nir::normal_forms::NominalValueDeclaration as D;
        let mut formed = 0;
        for definition in package
            .structs
            .iter()
            .map(D::Struct)
            .chain(package.enums.iter().map(D::Enum))
            .chain(package.newtypes.iter().map(D::Newtype))
        {
            *fuel = fuel
                .checked_sub(1)
                .ok_or(nir::normal_forms::ValidationError::Budget)?;
            let (kind, declaration) = definition.identity();
            if package.formed_nullary_nominals.contains(kind, declaration)
                && definition.is_unconditional_nullary()
            {
                formed += 1;
            }
            if Arc::make_mut(&mut theory.constructors.nominal_constructors)
                .insert(declaration, Some(kind))
                .is_some()
            {
                return Err(nir::normal_forms::ValidationError::Invalid(
                    "duplicate nominal declaration identity",
                ));
            }
        }
        if formed != package.formed_nullary_nominals.len() {
            return Err(nir::normal_forms::ValidationError::Invalid(
                "nullary nominal formation evidence lacks a matching unconditional declaration",
            ));
        }
        theory.formed_nominals = package.formed_nullary_nominals.clone();
        for head in &package.normal_forms {
            *fuel = fuel
                .checked_sub(1)
                .ok_or(nir::normal_forms::ValidationError::Budget)?;
            theory.admit_normal_form_head(head.declaration, head.role);
        }
        Ok(theory)
    }

    fn admit_normal_form_head(&mut self, head: DeclarationId, role: nir::normal_forms::Role) {
        use nir::normal_forms::Role;
        Arc::make_mut(&mut self.constructors.congruence_heads).insert(head);
        match role {
            Role::Opaque => {}
            Role::TypeList => {
                Arc::make_mut(&mut self.list_inference_heads).insert(head);
            }
            Role::Tuple => {
                Arc::make_mut(&mut self.constructors.tuple_heads).insert(head);
            }
        }
    }

    pub fn into_context(self) -> Context {
        Context {
            theory: self,
            scope: Arc::default(),
            assumptions: Arc::default(),
        }
    }
}

impl Context {
    /// Called by the declaration owner only after checking all formation obligations.
    pub fn admit_checked_nominal(
        &mut self,
        definition: nir::normal_forms::NominalValueDeclaration<'_>,
    ) {
        let (kind, id) = definition.identity();
        self.admit_nominal_kind(id, kind);
        self.theory.formed_nominals.admit_checked(definition);
    }

    pub fn formed_nullary_nominals(&self) -> &nir::program::FormedNullaryNominals {
        &self.theory.formed_nominals
    }

    /// Publish the owner's checked head contract. Congruence is forward only;
    /// tuple classification and list inference require their own layout roles.
    pub fn admit_normal_form_head(&mut self, head: DeclarationId, role: nir::normal_forms::Role) {
        self.theory.admit_normal_form_head(head, role);
    }
}

impl fmt::Display for DeclarationTheory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(declaration-theory {}", self.constructors)?;
        write!(f, " {}", self.formed_nominals)?;
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
        f.write_str(" (nominal-constructors")?;
        for (id, kind) in self.nominal_constructors.iter() {
            if let Some(kind) = kind {
                write!(f, " ({kind} {id})")?;
            } else {
                write!(f, " {id}")?;
            }
        }
        f.write_str(")")?;
        write_group(f, "tuple-heads", &self.tuple_heads)?;
        f.write_str(")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Budget, Formula, Outcome};

    #[test]
    fn nominal_kind_evidence_requires_the_same_descriptor_on_replay() {
        use nir::{NominalKind, Term, TypeKind};
        let id = DeclarationId {
            package: 2,
            local: 3,
        };
        let ty = Term::Nominal {
            declaration: id,
            arguments: vec![],
        };
        let goal = Formula::kind(ty.clone(), TypeKind::Nominal(NominalKind::Enum));
        let mut bare = Context::default();
        bare.admit_nominal_constructor(id);
        assert_eq!(
            bare.evaluate_kind(TypeKind::Nominal(NominalKind::Enum), &ty),
            None
        );
        let mut enums = bare.fork();
        enums.admit_nominal_kind(id, NominalKind::Enum);
        let mut records = bare.fork();
        records.admit_nominal_kind(id, NominalKind::Struct);
        let Outcome::Entailed(proof) = enums.prove(goal.clone(), Budget::default()) else {
            panic!("enum descriptor")
        };
        let Outcome::NotEntailed(witness) = records.prove(goal, Budget::default()) else {
            panic!("struct witness")
        };
        enums.verify_proof(&proof, Budget::default()).unwrap();
        records.verify_witness(&witness, Budget::default()).unwrap();
        assert_eq!(
            records.verify_proof(&proof, Budget::default()),
            Err(ProofError::Invalid)
        );
        assert_eq!(
            enums.verify_witness(&witness, Budget::default()),
            Err(ProofError::Invalid)
        );
    }

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
