//! Relocate a verified interface frame's owner plans into a prepared call.
use super::*;
use elaboration::verification::moves::{CleanupAction, CleanupReason};
use iir::cleanup::{Action, Boundary, Site, Target};
use nir::program::{Expr, ExprKind as K};

/// Only the interface-owned part of a caller schedule. Caller locals, enclosing
/// preparations, argument provenance and invocation contexts must be supplied by
/// the caller analysis before publication as an executable Schedule.
#[derive(Debug)]
pub struct MethodCallCleanup {
    pub callee: Expr,
    pub sites: Vec<Site>,
    pub roots: Vec<iir::InstanceKey>,
    pub receivers: Vec<(iir::InstanceKey, nir::Term)>,
}
impl fmt::Display for MethodCallCleanup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(method-call-cleanup (callee {}) (sites", self.callee)?;
        for site in &self.sites {
            write!(f, " {site}")?;
        }
        f.write_str(") (roots")?;
        for root in &self.roots {
            write!(f, " {root}")?;
        }
        f.write_str(") (receivers")?;
        for (key, ty) in &self.receivers {
            write!(f, " (receiver {key} {ty})")?;
        }
        f.write_str("))")
    }
}

fn invokes(steps: &[CleanupStep], fuel: &mut usize) -> Result<bool, String> {
    *fuel = fuel
        .checked_sub(steps.len())
        .ok_or("interface cleanup inspection budget exhausted")?;
    let mut pending: Vec<_> = steps.iter().collect();
    let mut found = false;
    while let Some(step) = pending.pop() {
        match step {
            CleanupStep::Invoke { .. } => found = true,
            CleanupStep::Release { .. } => (),
            CleanupStep::IfConstructed { steps, .. } => {
                *fuel = fuel
                    .checked_sub(steps.len())
                    .ok_or("interface cleanup inspection budget exhausted")?;
                pending.extend(steps);
            }
            CleanupStep::SelectVariantPlan { plans } => {
                for plan in plans {
                    *fuel = fuel
                        .checked_sub(plan.steps.len().saturating_add(1))
                        .ok_or("interface cleanup inspection budget exhausted")?;
                    pending.extend(&plan.steps);
                }
            }
            CleanupStep::SwitchVariant { branches, .. } => {
                for branch in branches {
                    *fuel = fuel
                        .checked_sub(branch.steps.len().saturating_add(1))
                        .ok_or("interface cleanup inspection budget exhausted")?;
                    pending.extend(&branch.steps);
                }
            }
        }
    }
    Ok(found)
}

pub fn lower_method_call_cleanup(
    evidence: &crate::protocols::MethodCleanup,
    call: &Expr,
    fuel: &mut usize,
) -> Result<MethodCallCleanup, String> {
    let K::Call {
        callee,
        arguments,
        parameter_bindings,
    } = &call.kind
    else {
        return Err("interface cleanup requires a call expression".into());
    };
    nir::copy_budget::charge_substitution(callee, &[], &[], fuel)
        .map_err(|_| "interface cleanup identity budget exhausted")?;
    if callee.kind != evidence.invocation.callee.kind || callee.ty != evidence.invocation.callee.ty
    {
        return Err("interface cleanup belongs to a different method identity".into());
    }
    if !evidence.report.deferred_calls.is_empty()
        || !evidence.report.named_calls.is_empty()
        || !evidence.report.named_candidate_calls.is_empty()
        || !evidence.report.callable_parameters.is_empty()
        || !evidence.report.enum_parameters.is_empty()
        || !evidence.report.effect_parameters.is_empty()
        || !evidence.report.borrowed_parameters.is_empty()
        || !evidence.report.borrowed_fields.is_empty()
        || !evidence.report.borrowed_code.is_empty()
        || !evidence.report.handlers.is_empty()
        || !evidence.report.defaults.is_empty()
        || evidence
            .report
            .closures
            .iter()
            .any(|closure| closure.invoked)
    {
        return Err("interface cleanup requires completed invocation contexts".into());
    }
    let function = &evidence.invocation.function;
    let nir::Term::Function { parameters, result } = function.signature.callable_signature() else {
        return Err("interface cleanup lacks a function signature".into());
    };
    if arguments.len() != parameters.len() || *result.as_ref() != call.ty {
        return Err("interface cleanup call layout mismatch".into());
    }
    *fuel = fuel
        .checked_sub(parameters.len().saturating_add(1))
        .ok_or("interface cleanup frame budget exhausted")?;
    // Both execution engines use this binding frame for snapshots and defaults.
    // Matching only argument slots is insufficient: aliased or missing bindings
    // would attach an otherwise valid owner plan to a different runtime frame.
    if parameter_bindings.len() != parameters.len()
        || parameter_bindings
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != parameters.len()
    {
        return Err("interface cleanup requires a complete unique parameter binding frame".into());
    }
    let mut sources = vec![None; parameters.len()];
    let mut last_default = None;
    for argument in arguments {
        if argument.is_default {
            if last_default.is_some_and(|last| last >= argument.parameter) {
                return Err("interface cleanup defaults are not in declaration order".into());
            }
            last_default = Some(argument.parameter);
        } else if last_default.is_some() {
            return Err("interface cleanup explicit argument follows a default".into());
        }
        let slot = sources
            .get_mut(argument.parameter)
            .ok_or("interface cleanup argument out of bounds")?;
        if slot.replace(argument.value.source).is_some()
            || argument.value.ty != parameters[argument.parameter].ty
        {
            return Err("interface cleanup argument layout mismatch".into());
        }
    }
    let mut output = MethodCallCleanup {
        callee: callee.as_ref().clone(),
        sites: vec![],
        roots: vec![],
        receivers: vec![],
    };
    // One runtime boundary serves all interface checks. Merge only identical
    // owner programs; concatenation would run the same destructor twice.
    for site in &evidence.report.cleanup_sites {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("interface cleanup site budget exhausted")?;
        if site
            .result_origin
            .is_some_and(|origin| origin != function.body.source)
        {
            return Err("interface cleanup result belongs to another invocation".into());
        }
        let exit = site.result_origin.is_some();
        let entry = site.reason == CleanupReason::PredicateFailure
            && !exit
            && function
                .preconditions
                .iter()
                .any(|condition| condition.source == site.source);
        let boundary = if exit {
            Some(Boundary::InterfacePostconditionFailure)
        } else if entry {
            Some(Boundary::InterfacePreconditionFailure)
        } else {
            None
        };
        let mut actions = vec![];
        for action in &site.actions {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("interface cleanup action budget exhausted")?;
            let CleanupAction::Planned { candidate, plan } = action else {
                return Err("interface cleanup contains an unplanned responsibility".into());
            };
            let meaningful = invokes(&plan.steps, fuel)?;
            if !meaningful {
                continue;
            }
            boundary.ok_or("interface observation cleanup needs a separate runtime context")?;
            let (ty, slot, value_source) = match candidate.as_ref() {
                CleanupAction::Owned { ty, remaining } if remaining.place.fields.is_empty() => {
                    let binding = nir::program::LocalId {
                        owner: u32::try_from(remaining.place.root.owner)
                            .map_err(|_| "interface cleanup owner is not a local")?,
                        index: remaining.place.root.local,
                    };
                    if exit && Some(binding) == function.result_binding {
                        (ty, parameters.len(), call.source)
                    } else if !exit {
                        let parameter = function
                            .parameters
                            .iter()
                            .position(|p| *p == binding)
                            .ok_or("interface entry cleanup has an unmapped observation owner")?;
                        (
                            ty,
                            parameter,
                            sources[parameter].ok_or("interface cleanup argument is missing")?,
                        )
                    } else {
                        return Err(
                            "interface exit cleanup still owns an input or observation value"
                                .into(),
                        );
                    }
                }
                CleanupAction::Prepared {
                    construction,
                    value,
                } if exit
                    && function.result_binding.is_none()
                    && *construction == function.body.source
                    && value.slot == 0
                    && value.source == function.body.source =>
                {
                    (&value.ty, parameters.len(), call.source)
                }
                _ => {
                    return Err(
                        "interface cleanup target requires a richer call-site mapping".into(),
                    );
                }
            };
            let expected = if slot == parameters.len() {
                result.as_ref()
            } else {
                &parameters[slot].ty
            };
            if ty != expected {
                return Err("interface cleanup target type mismatch".into());
            }
            let steps = lower(
                &plan.steps,
                &plan.receiver,
                &mut output.roots,
                &mut output.receivers,
                fuel,
                0,
            )?;
            actions.push(Action {
                target: Target::Prepared {
                    construction: call.source,
                    slot,
                    value_source,
                },
                program: iir::cleanup::Program {
                    ty: ty.clone(),
                    guards: plan
                        .variants
                        .iter()
                        .map(|v| (v.fields.clone(), v.index))
                        .collect(),
                    steps,
                },
            });
        }
        if let Some(boundary) = boundary {
            if let Some(previous) = output
                .sites
                .iter()
                .find(|previous| previous.boundary == boundary)
            {
                if previous.actions != actions {
                    return Err("interface checks need distinct failure cleanup programs".into());
                }
            } else {
                output.sites.push(Site {
                    source: call.source,
                    boundary,
                    actions,
                });
            }
        }
    }
    if !function.postconditions.is_empty()
        && !output
            .sites
            .iter()
            .any(|site| site.boundary == Boundary::InterfacePostconditionFailure)
    {
        // A statically true postcondition still has an explicit empty boundary;
        // both engines require publication evidence, not absence-as-success.
        output.sites.push(Site {
            source: call.source,
            boundary: Boundary::InterfacePostconditionFailure,
            actions: vec![],
        });
    }
    output.roots.sort_by_cached_key(ToString::to_string);
    output.roots.dedup();
    Ok(output)
}
