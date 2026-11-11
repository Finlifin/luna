//! Bounded preflight shared by single-output and dependency-driving resumes.
//! Plans own observations, never values, inference authority or executable state.
use super::*;

pub(super) struct Computation {
    pub output: MetaId,
    pub inputs: BTreeSet<MetaId>,
}
pub(super) struct Plan {
    pub order: Vec<Computation>,
    pub external: BTreeSet<MetaId>,
    pub failed: bool,
}
impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(type-demand-plan (failed {}) (order", self.failed)?;
        for computation in &self.order {
            write!(f, " (computation {} (inputs", computation.output)?;
            for input in &computation.inputs {
                write!(f, " {input}")?;
            }
            f.write_str("))")?;
        }
        f.write_str(") (external")?;
        for input in &self.external {
            write!(f, " {input}")?;
        }
        f.write_str("))")
    }
}

enum Work {
    Enter(MetaId),
    Leave(Computation),
}
impl Session<'_> {
    pub(super) fn plan_demand(
        &self,
        context: &InferenceContext,
        output: MetaId,
        fuel: &mut usize,
    ) -> Result<Plan, Error> {
        let mut work = vec![Work::Enter(output)];
        let mut active = BTreeSet::new();
        let mut done = BTreeSet::new();
        let mut plan = Plan {
            order: vec![],
            external: BTreeSet::new(),
            failed: false,
        };
        while let Some(work_item) = work.pop() {
            *fuel = fuel.checked_sub(1).ok_or(Error::SchedulingBudget)?;
            let id = match work_item {
                Work::Enter(id) => id,
                Work::Leave(computation) => {
                    active.remove(&computation.output);
                    done.insert(computation.output);
                    plan.order.push(computation);
                    continue;
                }
            };
            if done.contains(&id) {
                continue;
            }
            let Some(pending) = self.pending.get(&id) else {
                plan.external.insert(id);
                continue;
            };
            match pending.state {
                State::Failed(_) => {
                    plan.failed = true;
                    continue;
                }
                State::Complete(_) => continue,
                State::Waiting(_) => {}
            }
            if !active.insert(id) {
                return Err(Error::DependencyCycle(id));
            }
            if context.is_assigned(id)? {
                return Err(Error::Inference(InferenceError::AlreadyAssigned(id)));
            }
            let inputs = pending
                .inputs
                .dependencies_metered(context, fuel)
                .map_err(|error| {
                    if error == InferenceError::Budget {
                        Error::SchedulingBudget
                    } else {
                        Error::Inference(error)
                    }
                })?;
            // Reserve graph edges, retained observations and the postorder
            // completion/failure filter before any invocation can execute.
            *fuel = fuel
                .checked_sub(inputs.len().saturating_mul(3))
                .ok_or(Error::SchedulingBudget)?;
            work.push(Work::Leave(Computation {
                output: id,
                inputs: inputs.clone(),
            }));
            work.extend(inputs.into_iter().rev().map(Work::Enter));
        }
        Ok(plan)
    }
}
