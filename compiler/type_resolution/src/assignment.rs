//! Producer-owned output slots. Reservations survive candidate snapshots and
//! prevent ordinary unification from replacing an invocation's future result.
use super::*;

/// Non-cloneable publication authority for one committed inference output.
/// Dropping the permit abandons publication; it does not reopen the variable
/// for ordinary inference or make the producer safe to execute again.
#[derive(Debug)]
pub struct AssignmentPermit {
    context: u64,
    output: MetaId,
}

impl fmt::Display for AssignmentPermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(assignment-permit (context {}) (output {}))",
            self.context, self.output
        )
    }
}

impl InferenceContext {
    /// Reserve an unassigned output before executing its producer. Other
    /// variables remain writable, including through candidate transactions.
    pub fn reserve_assignment(
        &mut self,
        output: MetaId,
    ) -> Result<AssignmentPermit, InferenceError> {
        if self.is_trial() {
            return Err(InferenceError::SpeculativeAssignment);
        }
        self.assignment_sort(output, None)?;
        Arc::make_mut(&mut self.variables)
            .get_mut(&output)
            .unwrap()
            .reserved = true;
        self.changed();
        Ok(AssignmentPermit {
            context: self.identity,
            output,
        })
    }

    /// Publish through the original output authority after checking its owner,
    /// occurrence and sort. A failed check leaves the reservation unchanged.
    pub fn assign_reserved(
        &mut self,
        permit: &AssignmentPermit,
        value: InferenceTerm,
    ) -> Result<(), InferenceError> {
        self.assign_authorized(permit.output, value, Some(permit))
    }

    pub(super) fn assignment_sort(
        &self,
        id: MetaId,
        permit: Option<&AssignmentPermit>,
    ) -> Result<Sort, InferenceError> {
        if let Some(permit) = permit {
            if self.is_trial() {
                return Err(InferenceError::SpeculativeAssignment);
            }
            if permit.context != self.identity || permit.output != id {
                return Err(InferenceError::ForeignAssignment);
            }
        }
        let variable = self
            .variables
            .get(&id)
            .ok_or(InferenceError::ForeignVariable(id))?;
        if variable.value.is_some() {
            return Err(InferenceError::AlreadyAssigned(id));
        }
        if variable.reserved != permit.is_some() {
            return Err(InferenceError::ReservedVariable(id));
        }
        Ok(variable.sort)
    }
}
