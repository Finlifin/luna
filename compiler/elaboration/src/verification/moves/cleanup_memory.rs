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
    GuardedIndirect {
        base: Place,
        fields: Vec<(nir::program::FieldOwner, usize)>,
        guards: Vec<(Vec<usize>, usize)>,
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
            Self::GuardedIndirect {
                base,
                fields,
                guards,
                pointee,
            } => {
                write!(f, "(guarded-indirect-origin (base {base}) (fields")?;
                for (owner, index) in fields {
                    write!(f, " (field {owner} {index})")?;
                }
                f.write_str(") (guards")?;
                for (path, tag) in guards {
                    f.write_str(" (guard (fields")?;
                    for field in path {
                        write!(f, " {field}")?;
                    }
                    write!(f, ") (variant {tag}))")?;
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
    project_origin_guarded(report, package, place, &[], fuel, depth)
}
fn project_origin_guarded(
    report: &Report,
    package: &Package,
    place: &Place,
    guards: &[(Vec<usize>, usize)],
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
                    conditions: Default::default(),
                    fields: vec![],
                },
                pointee.clone(),
            )
        }
        BorrowedRoot::GuardedIndirect {
            base,
            fields,
            guards,
            pointee,
        } => {
            let (mut target, ty) = project_origin(report, package, base, fuel, depth + 1)?
                .ok_or("conditional cleanup origin lacks a parameter root")?;
            let mut traversal = memory::StorageTraversal::new(Some(package), &ty, &[], fuel)?;
            traversal.then_guarded(Some(package), fields, guards, fuel)?;
            if !matches!(&traversal.target.ty, Term::Pointer(actual) if actual.as_ref() == pointee)
            {
                return Err("conditional cleanup origin pointee disagrees with slot".into());
            }
            let prefix: Vec<_> = target.fields.iter().map(|(_, index)| *index).collect();
            let selected = target.conditions.entry(target.through.len()).or_default();
            for (path, tag) in guards {
                charge(fuel, prefix.len() + path.len() + 1)?;
                let mut container = prefix.clone();
                container.extend(path);
                selected.push((container, *tag));
            }
            target.fields.extend(fields);
            target.through.push(std::mem::take(&mut target.fields));
            (target, pointee.clone())
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
    let mut symbolic = vec![];
    let mut consumed = 0;
    for (depth, index) in place.fields.iter().enumerate() {
        charge(fuel, guards.len().saturating_add(1))?;
        for (path, tag) in guards {
            charge(fuel, path.len())?;
            if path.as_slice() == &place.fields[..depth] {
                symbolic.extend([VARIANT_PATH, *tag]);
                consumed += 1;
            }
        }
        symbolic.push(*index);
    }
    if consumed != guards.len() {
        return Err("cleanup terminal guard lies outside its target path".into());
    }
    let projection = memory::project_owned_storage(Some(package), &ty, &symbolic, fuel)?;
    let prefix: Vec<_> = target.fields.iter().map(|(_, index)| *index).collect();
    if !projection.guards.is_empty() {
        let conditions = target.conditions.entry(target.through.len()).or_default();
        for (path, tag) in projection.guards {
            charge(
                fuel,
                prefix.len().saturating_add(path.len()).saturating_add(1),
            )?;
            let mut absolute = prefix.clone();
            absolute.extend(path);
            conditions.push((absolute, tag));
        }
    }
    target.fields.extend(projection.fields);
    ty = projection.ty;
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
                let guards = effect
                    .target
                    .conditions
                    .get(&effect.target.through.len())
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let mut absolute = Vec::new();
                if !guards.is_empty() {
                    let suffix: Vec<_> = effect
                        .target
                        .fields
                        .iter()
                        .map(|(_, index)| *index)
                        .collect();
                    let prefix = place
                        .fields
                        .strip_suffix(suffix.as_slice())
                        .ok_or("cleanup guarded target disagrees with terminal field path")?;
                    for (path, tag) in guards {
                        charge(
                            fuel,
                            prefix.len().saturating_add(path.len()).saturating_add(1),
                        )?;
                        let mut path_in_root = prefix.to_vec();
                        path_in_root.extend(path);
                        absolute.push((path_in_root, *tag));
                    }
                }
                let Some((target, ty)) =
                    project_origin_guarded(self, package, place, &absolute, fuel, 0)?
                else {
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

/// Speculative work within the same invocation needs the origin registry even
/// when its execution records are isolated. This grants no storage validity.
pub(super) fn copy_roots(
    roots: &BTreeMap<PlaceId, BorrowedRoot>,
    fuel: &mut usize,
) -> Result<BTreeMap<PlaceId, BorrowedRoot>, String> {
    for origin in roots.values() {
        charge(fuel, 1)?;
        match origin {
            BorrowedRoot::Parameter {
                input_fields,
                pointee,
                ..
            } => {
                charge(fuel, input_fields.len())?;
                memory::bound_type(pointee, fuel)?;
            }
            BorrowedRoot::Indirect { slot, pointee } => {
                charge(fuel, slot.fields.len())?;
                memory::bound_type(pointee, fuel)?;
            }
            BorrowedRoot::GuardedIndirect {
                base,
                fields,
                guards,
                pointee,
            } => {
                charge(fuel, base.fields.len().saturating_add(fields.len()))?;
                for (path, _) in guards {
                    charge(fuel, path.len().saturating_add(1))?;
                }
                memory::bound_type(pointee, fuel)?;
            }
        }
    }
    Ok(roots.clone())
}

impl<'a, F: FnMut(&Term) -> Result<Delivery, String>, G: FnMut(&Term) -> Result<Shape, String>>
    Analyzer<'a, F, G>
{
    pub(super) fn prepare_cleanup_origins(
        &mut self,
        flow: &mut Flow,
        root: PlaceId,
        ty: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        if self.report.borrowed_roots.is_empty()
            || matches!(
                ty,
                Term::Primitive(_)
                    | Term::Pointer(_)
                    | Term::Function { .. }
                    | Term::UnsafeFunction(_)
            )
        {
            return Ok(());
        }
        let Some(policy) = self.cleanup.as_deref_mut() else {
            return Ok(());
        };
        let Some(reads) = policy
            .cleanup_reads(ty, &mut self.fuel)
            .map_err(|e| error(source, e))?
        else {
            return Ok(());
        };
        let receiver = Place {
            root,
            fields: vec![],
        };
        let arguments =
            BTreeMap::from([(0, BTreeMap::from([(vec![], BTreeSet::from([receiver]))]))]);
        for read in reads {
            let Some(first) = read.target.through.first() else {
                continue;
            };
            // A moved component no longer contributes its own destructor. The
            // acquired plan still rejects an enclosing Drop reading that slot.
            if read.target.conditions.get(&0).is_none_or(Vec::is_empty) {
                let slot = Place {
                    root,
                    fields: first.iter().map(|(_, index)| *index).collect(),
                };
                if flow.state.require_present(&slot).is_err() {
                    continue;
                }
            }
            self.map_memory_target(&read.target, &arguments, flow, source)?;
        }
        Ok(())
    }
}
