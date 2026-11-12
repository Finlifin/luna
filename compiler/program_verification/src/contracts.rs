//! Trait program-contract replacement; no obligation here is a type proposition.
#[cfg(test)]
mod tests;
use super::{Limits, flow};
use nir::{Primitive, Term, contracts::remap_predicate_metered, program::*};
use std::collections::BTreeMap;

fn failure(source: SourceRange, message: &'static str) -> flow::Error {
    flow::Error {
        source,
        reason: flow::Reason::Phase(message),
    }
}
fn fresh(next: &mut u32, source: SourceRange) -> Result<LocalId, flow::Error> {
    let index = *next;
    *next = index
        .checked_add(1)
        .ok_or_else(|| failure(source, "contract binding budget exhausted"))?;
    Ok(LocalId {
        owner: u32::MAX,
        index,
    })
}
fn budget(source: SourceRange) -> flow::Error {
    flow::Error {
        source,
        reason: flow::Reason::Unknown(comptime_runtime::Error::BudgetExhausted.into()),
    }
}
fn preparation(source: SourceRange, error: nir::type_computations::Error) -> flow::Error {
    match error {
        nir::type_computations::Error::Budget => budget(source),
        _ => failure(source, "invalid contract declaration substitution"),
    }
}
fn copy_function(value: &Function, limits: &mut Limits) -> Result<Function, flow::Error> {
    nir::copy_budget::charge_function_substitution(value, &[], &[], &mut limits.arithmetic.steps)
        .map_err(|_| budget(value.body.source))?;
    Ok(value.clone())
}
fn copy_expression(value: &Expr, limits: &mut Limits) -> Result<Expr, flow::Error> {
    nir::copy_budget::charge_substitution(value, &[], &[], &mut limits.arithmetic.steps)
        .map_err(|_| budget(value.source))?;
    Ok(value.clone())
}
fn copy_expressions(values: &[Expr], limits: &mut Limits) -> Result<Vec<Expr>, flow::Error> {
    values
        .iter()
        .map(|value| copy_expression(value, limits))
        .collect()
}
fn remap(
    values: &mut [Expr],
    bindings: &BTreeMap<LocalId, LocalId>,
    limits: &mut Limits,
) -> Result<(), flow::Error> {
    for value in values {
        remap_predicate_metered(value, bindings, &mut limits.arithmetic.steps).map_err(
            |message| {
                if message == "program contract substitution budget exhausted" {
                    budget(value.source)
                } else {
                    failure(value.source, message)
                }
            },
        )?;
    }
    Ok(())
}

/// Private overlay under an exclusive package borrow. No snapshot can escape
/// validation; errors or unwinding restore every replaced function.
struct Overlay<'a> {
    package: &'a mut Package,
    originals: Vec<(usize, Function)>,
    committed: bool,
}
impl Drop for Overlay<'_> {
    fn drop(&mut self) {
        if !self.committed {
            for (index, original) in self.originals.drain(..) {
                self.package.functions[index] = original;
            }
        }
    }
}

/// Prepare all effective contracts before checking explicit weakening.
/// Successful inheritance is applied once to a fresh elaboration snapshot.
pub fn inherit(package: &mut Package, next: &mut u32) -> Result<(), flow::Error> {
    inherit_with_limits(package, next, &mut Limits::default())
}

/// Failure preserves both the caller's package and its binding cursor. Copy
/// preparation and proof checks consume the same caller-owned work budget.
pub fn inherit_with_limits(
    package: &mut Package,
    next: &mut u32,
    limits: &mut Limits,
) -> Result<(), flow::Error> {
    let mut prepared_next = *next;
    let (drafts, obligations) = prepare(package, &mut prepared_next, limits)?;
    let mut overlay = Overlay {
        package,
        originals: Vec::with_capacity(drafts.len()),
        committed: false,
    };
    for (index, draft) in drafts {
        let original = std::mem::replace(&mut overlay.package.functions[index], draft);
        overlay.originals.push((index, original));
    }
    for obligation in obligations {
        flow::analyze_in_package(overlay.package, &obligation, limits).map_err(|error| {
            match error.reason {
                flow::Reason::Unknown(_) | flow::Reason::DepthLimit => error,
                _ => failure(obligation.body.source, "implementation strengthens trait program preconditions or their definedness is not proven"),
            }
        })?;
    }
    overlay.committed = true;
    *next = prepared_next;
    Ok(())
}

fn prepare(
    package: &Package,
    next: &mut u32,
    limits: &mut Limits,
) -> Result<(BTreeMap<usize, Function>, Vec<Function>), flow::Error> {
    let metadata = super::declarations::Metadata::new(Some(package));
    let source = SourceRange { start: 0, end: 0 };
    let domains = metadata
        .domains(&mut limits.arithmetic.steps)
        .map_err(|error| preparation(source, error))?;
    // These describe abstract definition inputs. Completed domain substitution
    // below, not this initial classification, admits concrete Type values.
    let rigid = |id: &nir::DeclarationId| Term::Rigid {
        declaration: *id,
        sort: if domains
            .get(id)
            .is_some_and(|domain| *domain != Term::Primitive(Primitive::Type))
        {
            nir::Sort::ComptimeValue
        } else {
            nir::Sort::Type
        },
    };
    let mut drafts = BTreeMap::new();
    let mut obligations = Vec::new();
    for implementation in &package.implementations {
        limits.step(1).map_err(|_| budget(source))?;
        let Some(interface) = implementation.interface else {
            continue;
        };
        limits
            .step(package.traits.len())
            .map_err(|_| budget(source))?;
        let definition = package
            .traits
            .iter()
            .find(|t| t.declaration == interface)
            .ok_or_else(|| {
                failure(
                    SourceRange { start: 0, end: 0 },
                    "trait contract is missing",
                )
            })?;
        for (name, body) in &implementation.methods {
            limits
                .step(definition.methods.len().saturating_add(1))
                .map_err(|_| budget(source))?;
            let method = definition
                .methods
                .iter()
                .find(|m| m.name == *name)
                .ok_or_else(|| {
                    failure(
                        SourceRange { start: 0, end: 0 },
                        "trait member contract is missing",
                    )
                })?;
            if method.default == Some(*body) {
                continue;
            }
            limits
                .step(
                    package
                        .runtime_function_identities
                        .len()
                        .saturating_add(package.functions.len()),
                )
                .map_err(|_| budget(source))?;
            // A required-invocation snapshot may carry only the identity of a
            // runtime method. It cannot invoke the body; the complete package
            // still performs inheritance and body verification later.
            if package
                .runtime_function_identities
                .iter()
                .any(|identity| identity.declaration == *body)
                && !method.signature.is_comptime_callable()
                && (method.program.preconditions.is_empty()
                    || matches!(method.signature.callable_signature(), Ok(Term::Function { parameters, .. })
                        if parameters.iter().any(|p| p.sort != nir::Sort::Runtime)))
            {
                continue;
            }
            let Some(index) = package
                .functions
                .iter()
                .position(|f| f.declaration == *body)
            else {
                return Err(failure(
                    SourceRange { start: 0, end: 0 },
                    "implementation contract body is missing",
                ));
            };
            let mut actual = copy_function(
                drafts.get(&index).unwrap_or(&package.functions[index]),
                limits,
            )?;
            // Omitting the spelling on an override does not weaken the
            // interface behavior promise. The complete body is checked later.
            actual.pure |= method.pure;
            let source = actual.body.source;
            limits
                .step(
                    definition
                        .type_parameters
                        .len()
                        .saturating_add(method.type_parameters.len())
                        .saturating_add(1),
                )
                .map_err(|_| budget(source))?;
            let parameter_ids: Vec<_> = definition
                .parameters_with_self()
                .into_iter()
                .chain(method.type_parameters.iter().copied())
                .collect();
            let prefix = actual
                .type_parameters
                .get(..implementation.type_parameters.len())
                .ok_or_else(|| failure(source, "implementation method input prefix is missing"))?;
            limits
                .step(actual.type_parameters.len())
                .map_err(|_| budget(source))?;
            let prefix_inputs = prefix.iter().map(&rigid).collect::<Vec<_>>();
            let owner_domains = metadata
                .parameter_domains(
                    &implementation.type_parameters,
                    &mut limits.arithmetic.steps,
                )
                .map_err(|error| preparation(source, error))?;
            let owner = nir::DeclarationSubstitution::bounded(
                &implementation.type_parameters,
                &owner_domains,
                &prefix_inputs,
                &[],
                &mut limits.arithmetic.steps,
            )
            .map_err(|error| preparation(source, error))?;
            let mut method_arguments = std::iter::once(&implementation.subject)
                .chain(&implementation.interface_arguments)
                .map(|argument| {
                    owner
                        .instantiate_bounded(argument, &mut limits.arithmetic.steps)
                        .map_err(|error| preparation(source, error))
                })
                .collect::<Result<Vec<_>, _>>()?;
            method_arguments.extend(
                actual
                    .type_parameters
                    .iter()
                    .skip(implementation.type_parameters.len())
                    .map(&rigid),
            );
            let parameter_domains = metadata
                .parameter_domains(&parameter_ids, &mut limits.arithmetic.steps)
                .map_err(|error| preparation(source, error))?;
            let frames = match actual.signature.callable_signature() {
                Term::Function { parameters, .. } if !parameters.is_empty() => {
                    vec![parameters.iter().map(|parameter| parameter.sort).collect()]
                }
                _ => Vec::new(),
            };
            let mut contract = method
                .program
                .instantiate_declaration_parameters(
                    &parameter_ids,
                    &parameter_domains,
                    &method_arguments,
                    &frames,
                    &mut limits.arithmetic.steps,
                )
                .map_err(|error| preparation(source, error))?;
            if method.parameter_bindings.len() != actual.parameters.len() {
                return Err(failure(source, "trait program contract arity mismatch"));
            }
            let mut bindings: BTreeMap<_, _> = method
                .parameter_bindings
                .iter()
                .copied()
                .zip(actual.parameters.iter().copied())
                .collect();
            remap(&mut contract.preconditions, &bindings, limits)?;
            let inherited_domain = actual.preconditions.is_empty();
            if inherited_domain {
                // An omitted program entry clause uses the interface domain.
                actual.preconditions = copy_expressions(&contract.preconditions, limits)?;
            } else {
                let mut obligation = copy_function(&actual, limits)?;
                let Term::Function { result, .. } = &mut obligation.signature else {
                    return Err(failure(source, "invalid implementation signature"));
                };
                *result = Box::new(Term::Primitive(Primitive::Void));
                obligation.body = Expr {
                    source,
                    ty: Term::Primitive(Primitive::Void),
                    kind: ExprKind::Unit,
                };
                obligation.preconditions = copy_expressions(&contract.preconditions, limits)?;
                obligation.postconditions = copy_expressions(&actual.preconditions, limits)?;
                obligation.result_binding = None;
                obligation.entry_snapshots.clear();
                obligation.defaults.clear();
                obligations.push(obligation);
            }
            if let Some(binding) = contract.result_binding {
                let destination = match actual.result_binding {
                    Some(binding) => binding,
                    None => {
                        let binding = fresh(next, source)?;
                        actual.result_binding = Some(binding);
                        binding
                    }
                };
                bindings.insert(binding, destination);
            }
            for snapshot in contract.entry_snapshots {
                limits
                    .step(actual.entry_snapshots.len().saturating_add(1))
                    .map_err(|_| budget(source))?;
                let parameter = *bindings.get(&snapshot.parameter).ok_or_else(|| {
                    failure(source, "trait snapshot does not refer to a parameter")
                })?;
                let binding = if let Some(snapshot) = actual
                    .entry_snapshots
                    .iter()
                    .find(|s| s.parameter == parameter)
                {
                    snapshot.binding
                } else {
                    let binding = fresh(next, source)?;
                    actual
                        .entry_snapshots
                        .push(EntrySnapshot { parameter, binding });
                    binding
                };
                bindings.insert(snapshot.binding, binding);
            }
            remap(&mut contract.postconditions, &bindings, limits)?;
            // A weaker implementation domain must not make trait ensures apply
            // to inputs outside the trait's domain. Guard with frozen ENTRY
            // predicates, never with the possibly modified exit parameters.
            if inherited_domain {
                contract.preconditions.clear();
            }
            let mut entry = BTreeMap::new();
            if !contract.preconditions.is_empty() && !contract.postconditions.is_empty() {
                for parameter in &actual.parameters {
                    limits
                        .step(actual.entry_snapshots.len().saturating_add(1))
                        .map_err(|_| budget(source))?;
                    let binding = if let Some(snapshot) = actual
                        .entry_snapshots
                        .iter()
                        .find(|s| s.parameter == *parameter)
                    {
                        snapshot.binding
                    } else {
                        let binding = fresh(next, source)?;
                        actual.entry_snapshots.push(EntrySnapshot {
                            parameter: *parameter,
                            binding,
                        });
                        binding
                    };
                    entry.insert(*parameter, binding);
                }
            }
            remap(&mut contract.preconditions, &entry, limits)?;
            let guard = contract.preconditions.into_iter().reduce(|a, b| Expr {
                source: a.source,
                ty: Term::Primitive(Primitive::Bool),
                kind: ExprKind::And(Box::new(a), Box::new(b)),
            });
            for postcondition in contract.postconditions {
                let postcondition = if let Some(guard) = &guard {
                    Expr {
                        source: postcondition.source,
                        ty: Term::Primitive(Primitive::Bool),
                        kind: ExprKind::Or(
                            Box::new(Expr {
                                source: guard.source,
                                ty: guard.ty.clone(),
                                kind: ExprKind::Not(Box::new(copy_expression(guard, limits)?)),
                            }),
                            Box::new(postcondition),
                        ),
                    }
                } else {
                    postcondition
                };
                actual.postconditions.push(postcondition);
            }
            drafts.insert(index, actual);
        }
    }
    Ok((drafts, obligations))
}
