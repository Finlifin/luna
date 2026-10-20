//! Preserve conditional caller origins when exporting acquired cleanup accesses.
use super::*;
use crate::verification::{drop::CleanupStep, memory};

#[derive(Clone, Debug, PartialEq)]
pub enum BorrowedRoot {
    Parameter {
        parameter: usize,
        input_fields: Vec<usize>,
        pointee: Term,
    },
    Indirect {
        slot: Place,
        pointee: Term,
    },
}
impl fmt::Display for BorrowedRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parameter {
                parameter,
                input_fields,
                pointee,
            } => {
                write!(f, "(parameter-origin (parameter {parameter}) (input-fields")?;
                for field in input_fields {
                    write!(f, " {field}")?;
                }
                write!(f, ") (pointee {pointee}))")
            }
            Self::Indirect { slot, pointee } => {
                write!(f, "(indirect-origin (slot {slot}) (pointee {pointee}))")
            }
        }
    }
}
fn charge(fuel: &mut usize, cost: usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(cost)
        .ok_or("cleanup memory export budget exhausted")?;
    Ok(())
}
pub(super) fn project_origin(
    report: &Report,
    package: &Package,
    place: &Place,
    fuel: &mut usize,
    depth: usize,
) -> Result<Option<(memory::ParameterPlace, Term)>, String> {
    charge(fuel, place.fields.len().saturating_add(1))?;
    if depth > 128 {
        return Err("cleanup memory origin depth exhausted".into());
    }
    let Some(origin) = report.borrowed_roots.get(&place.root) else {
        if matches!(place.root.owner, x if x == u64::MAX - 1 || x == u64::MAX - 2 || x == u64::MAX - 4)
        {
            return Err("cleanup memory origin requires its enclosing invocation context".into());
        }
        return Ok(None);
    };
    let (mut target, mut ty) = match origin {
        BorrowedRoot::Parameter {
            parameter,
            input_fields,
            pointee,
        } => {
            charge(fuel, input_fields.len())?;
            memory::bound_type(pointee, fuel)?;
            (
                memory::ParameterPlace {
                    parameter: *parameter,
                    input_fields: input_fields.clone(),
                    through: vec![],
                    fields: vec![],
                },
                pointee.clone(),
            )
        }
        BorrowedRoot::Indirect { slot, pointee } => {
            let (mut target, ty) = project_origin(report, package, slot, fuel, depth + 1)?
                .ok_or("indirect cleanup origin lacks a parameter root")?;
            memory::bound_type(pointee, fuel)?;
            if !matches!(&ty, Term::Pointer(actual) if actual.as_ref() == pointee) {
                return Err("indirect cleanup origin pointee disagrees with slot".into());
            }
            memory::bound_type(pointee, fuel)?;
            target.through.push(std::mem::take(&mut target.fields));
            (target, pointee.clone())
        }
    };
    for index in &place.fields {
        charge(fuel, 1)?;
        let owner = match &ty {
            Term::Nominal { declaration, .. } => nir::program::FieldOwner::Nominal(*declaration),
            Term::Record(_) => nir::program::FieldOwner::Anonymous,
            _ if nir::normal_forms::tuple_fields(Some(package), &ty).is_some() => {
                nir::program::FieldOwner::Anonymous
            }
            _ => {
                return Err("cleanup memory projection requires a known owned layout".into());
            }
        };
        ty = memory::owned_path_type(Some(package), &ty, &[*index], fuel)?;
        target.fields.push((owner, *index));
    }
    Ok(Some((target, ty)))
}
impl Report {
    /// Export only the acquired cleanup accesses in this report's own frame.
    /// Nested reports need their own context; this is not a complete body summary.
    pub fn cleanup_memory_accesses(
        &self,
        package: &Package,
        fuel: &mut usize,
    ) -> Result<Vec<memory::Access>, String> {
        let mut output = vec![];
        let mut resolved = Vec::new();
        for (effect, targets) in &self.cleanup_call_accesses {
            charge(fuel, 1)?;
            resolved.push((effect, targets));
        }
        let mut pending = Vec::new();
        for site in &self.cleanup_sites {
            charge(fuel, 1)?;
            for action in &site.actions {
                charge(fuel, 1)?;
                let CleanupAction::Planned { plan, .. } = action else {
                    return Err("cleanup memory export requires acquired cleanup plans".into());
                };
                charge(fuel, plan.steps.len())?;
                pending.extend(plan.steps.iter().rev());
            }
        }
        while let Some(step) = pending.pop() {
            charge(fuel, 1)?;
            match step {
                CleanupStep::Invoke { access, .. } => {
                    if access.targets.len() != access.footprint.accesses.len() {
                        return Err("cleanup access evidence length mismatch".into());
                    }
                    for pair in access.footprint.accesses.iter().zip(&access.targets) {
                        charge(fuel, 1)?;
                        resolved.push(pair);
                    }
                }

                CleanupStep::SelectVariantPlan { plans } => {
                    charge(fuel, plans.len())?;
                    for plan in plans {
                        charge(fuel, plan.steps.len())?;
                        pending.extend(plan.steps.iter().rev());
                    }
                }
                CleanupStep::IfConstructed { steps, .. } => {
                    charge(fuel, steps.len())?;
                    pending.extend(steps.iter().rev());
                }
                CleanupStep::SwitchVariant { branches, .. } => {
                    charge(fuel, branches.len())?;
                    for branch in branches {
                        charge(fuel, branch.steps.len())?;
                        pending.extend(branch.steps.iter().rev());
                    }
                }
                CleanupStep::Release { .. } => {}
            }
        }
        for (effect, targets) in resolved {
            charge(fuel, 1)?;
            for place in targets {
                let Some((target, ty)) = project_origin(self, package, place, fuel, 0)? else {
                    continue;
                };
                if ty != effect.ty {
                    return Err("cleanup access target type mismatch".into());
                }
                let effect = memory::Access {
                    target,
                    ty,
                    write: effect.write,
                    read_value: effect.read_value,
                };
                charge(fuel, output.len())?;
                if !output.contains(&effect) {
                    output.push(effect);
                }
            }
        }
        Ok(output)
    }
}
