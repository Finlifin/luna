//! Immutable query ownership for captured destructor choices and exclusions.
//! This retains type evidence, not runtime environments or program proofs.
use super::*;
use std::sync::Arc;

#[derive(Debug)]
struct Query {
    protocol: DeclarationId,
    subject: Term,
    resolution: crate::capabilities::Resolution,
}

#[derive(Clone, Debug)]
pub struct SelectionEvidence(Arc<Query>);

impl SelectionEvidence {
    pub(super) fn capture(
        protocol: DeclarationId,
        subject: &Term,
        resolution: crate::capabilities::Resolution,
        fuel: &mut usize,
    ) -> Result<Self, String> {
        crate::memory::bound_type(subject, fuel)?;
        Ok(Self(Arc::new(Query {
            protocol,
            subject: subject.clone(),
            resolution,
        })))
    }

    pub fn type_selection(&self) -> &TypeResolution {
        &self.0.resolution.types
    }

    pub(super) fn qualification(&self) -> &[Witness] {
        let Selection::Selected(id) = self.0.resolution.types.selection else {
            unreachable!("selected Drop retains a selected query")
        };
        let Some(Qualification::Proved(witnesses)) = self.0.resolution.qualifications.get(&id)
        else {
            unreachable!("selected Drop retains complete qualification")
        };
        witnesses
    }

    pub(super) fn check_world(&self, world: &World<'_>, fuel: &mut usize) -> Result<(), String> {
        let mut budget = Budget {
            steps: *fuel,
            ..Budget::default()
        };
        let result = world.verify_condition_support_metered(&self.0.resolution, &mut budget);
        *fuel = budget.steps;
        result
            .map_err(|e| format!("captured Drop query scope or condition support is invalid: {e}"))
    }

    pub(super) fn check_instance(
        &self,
        protocol: DeclarationId,
        subject: &Term,
        implementation: DeclarationId,
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<(), String> {
        crate::memory::bound_type(subject, fuel)?;
        crate::memory::bound_type(&self.0.subject, fuel)?;
        if protocol != self.0.protocol
            || subject != &self.0.subject
            || self.type_selection().selection != Selection::Selected(implementation)
        {
            return Err("captured Drop instance disagrees with its type selection".into());
        }
        let Some(TypeAssessment::Applicable(evidence)) =
            self.type_selection().assessments.get(&implementation)
        else {
            return Err("captured Drop lacks applicable type evidence".into());
        };
        for argument in arguments.iter().chain(&evidence.type_arguments) {
            crate::memory::bound_type(argument, fuel)?;
        }
        if evidence.implementation != implementation || evidence.type_arguments != arguments {
            return Err("captured Drop arguments disagree with its type selection".into());
        }
        Ok(())
    }
}

impl fmt::Display for SelectionEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(drop-selection-evidence (protocol {}) (subject {}) {})",
            self.0.protocol, self.0.subject, self.0.resolution
        )
    }
}
