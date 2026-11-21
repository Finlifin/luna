//! Trait program-contract replacement; no obligation here is a type proposition.
#[cfg(test)]
mod tests;

use super::{Limits, flow};
use nir::{Primitive, Term, contracts::remap_predicate_metered, program::*};
use std::collections::{BTreeMap, BTreeSet};

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

fn copy_declaration(
    value: &FunctionDeclaration,
    source: SourceRange,
    limits: &mut Limits,
) -> Result<FunctionDeclaration, flow::Error> {
    nir::copy_budget::clone_function_declaration(value, &mut limits.arithmetic.steps)
        .map_err(|_| budget(source))
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
/// validation; errors or unwinding restore every replaced declaration.
/// Executable bodies stay in place.
struct Overlay<'a> {
    package: &'a mut Package,
    originals: Vec<(usize, FunctionDeclaration)>,
    committed: bool,
}

impl Drop for Overlay<'_> {
    fn drop(&mut self) {
        if !self.committed {
            for (index, original) in self.originals.drain(..) {
                self.package.functions[index].interface = original;
            }
        }
    }
}

/// Prepare all effective contracts before checking explicit weakening.
/// Successful inheritance is applied once to a fresh elaboration snapshot.
pub fn inherit(package: &mut Package, next: &mut u32) -> Result<(), flow::Error> {
    inherit_with_limits(package, next, &mut Limits::default())
}

/// Incremental version of [`inherit`] that skips already-verified upstream
/// implementations while keeping upstream traits and methods available for
/// downstream implementations.
pub fn inherit_incremental(
    package: &mut Package,
    next: &mut u32,
    verified: &flow::VerifiedUpstream,
) -> Result<(), flow::Error> {
    inherit_incremental_with_limits(package, next, verified, &mut Limits::default())
}

/// Failure preserves both the caller's package and its binding cursor. Copy
/// preparation and proof checks consume the same caller-owned work budget.
pub fn inherit_with_limits(
    package: &mut Package,
    next: &mut u32,
    limits: &mut Limits,
) -> Result<(), flow::Error> {
    inherit_inner(package, next, None, limits)
}

/// Budgeted incremental version of [`inherit_with_limits`].
pub fn inherit_incremental_with_limits(
    package: &mut Package,
    next: &mut u32,
    verified: &flow::VerifiedUpstream,
    limits: &mut Limits,
) -> Result<(), flow::Error> {
    let filter =
        (!verified.implementations.is_empty()).then_some(IncrementalFilter::SkipAll(verified));
    inherit_inner(package, next, filter, limits)
}

/// Apply trait-contract inheritance to all implementations while skipping
/// weakening proof obligations for already-verified upstream implementations.
/// Use when upstream declarations were freshly lowered rather than deserialized
/// post-inheritance.
pub fn inherit_incremental_skip_proofs(
    package: &mut Package,
    next: &mut u32,
    verified: &flow::VerifiedUpstream,
) -> Result<(), flow::Error> {
    inherit_incremental_skip_proofs_with_limits(package, next, verified, &mut Limits::default())
}

/// Budgeted version of [`inherit_incremental_skip_proofs`].
pub fn inherit_incremental_skip_proofs_with_limits(
    package: &mut Package,
    next: &mut u32,
    verified: &flow::VerifiedUpstream,
    limits: &mut Limits,
) -> Result<(), flow::Error> {
    let filter =
        (!verified.implementations.is_empty()).then_some(IncrementalFilter::SkipProofs(verified));
    inherit_inner(package, next, filter, limits)
}

#[derive(Clone, Copy)]
enum IncrementalFilter<'a> {
    SkipAll(&'a flow::VerifiedUpstream),
    SkipProofs(&'a flow::VerifiedUpstream),
}

fn inherit_inner(
    package: &mut Package,
    next: &mut u32,
    verified: Option<IncrementalFilter<'_>>,
    limits: &mut Limits,
) -> Result<(), flow::Error> {
    let mut prepared_next = *next;
    let (drafts, obligations) = prepare(package, &mut prepared_next, verified, limits)?;
    let mut overlay = Overlay {
        package,
        originals: Vec::with_capacity(drafts.len()),
        committed: false,
    };
    for (index, draft) in drafts {
        let original = std::mem::replace(&mut overlay.package.functions[index].interface, draft);
        overlay.originals.push((index, original));
    }
    for obligation in obligations {
        let source = obligation.source;
        flow::prove_implication(&obligation, Some(overlay.package), limits).map_err(|error| {
            match error.reason {
                flow::Reason::Unknown(_) | flow::Reason::DepthLimit => error,
                _ => failure(source, "implementation strengthens trait program preconditions or their definedness is not proven"),
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
    verified: Option<IncrementalFilter<'_>>,
    limits: &mut Limits,
) -> Result<
    (
        BTreeMap<usize, FunctionDeclaration>,
        Vec<FunctionDeclaration>,
    ),
    flow::Error,
> {
    let metadata = super::declarations::Metadata::new(Some(package));
    let source = SourceRange { start: 0, end: 0 };
    let domains = metadata
        .domains(&mut limits.arithmetic.steps)
        .map_err(|error| preparation(source, error))?;
    // These describe abstract definition inputs. Completed domain substitution
    // below, not this initial classification, admits concrete Type values.
    let rigid = |id: &nir::DeclarationId| Term::Rigid {
        declaration: *id,
        sort: domains
            .get(id)
            .map_or(nir::Sort::Type, nir::Sort::of_comptime_domain),
    };
    limits
        .step(
            package
                .traits
                .len()
                .saturating_add(package.runtime_function_identities.len())
                .saturating_add(package.functions.len())
                .saturating_add(package.method_links.len())
                .saturating_add(1),
        )
        .map_err(|_| budget(source))?;
    let mut traits_by_id = BTreeMap::new();
    let mut trait_methods = BTreeMap::new();
    for definition in &package.traits {
        if let std::collections::btree_map::Entry::Vacant(slot) =
            traits_by_id.entry(definition.declaration)
        {
            slot.insert(definition);
            limits
                .step(definition.methods.len().saturating_add(1))
                .map_err(|_| budget(source))?;
            for method in &definition.methods {
                trait_methods
                    .entry((definition.declaration, method.name.as_str()))
                    .or_insert(method);
            }
        }
    }
    let runtime_identities: BTreeSet<_> = package
        .runtime_function_identities
        .iter()
        .map(|identity| identity.declaration)
        .collect();
    let mut function_indices = BTreeMap::new();
    for (index, function) in package.functions.iter().enumerate() {
        function_indices
            .entry(function.declaration)
            .or_insert(index);
    }
    let method_links: BTreeSet<_> = package
        .method_links
        .iter()
        .map(|link| link.declaration)
        .collect();
    let mut drafts = BTreeMap::new();
    let mut obligations = Vec::new();
    for implementation in &package.implementations {
        limits.step(1).map_err(|_| budget(source))?;
        let skip_all = matches!(
            verified,
            Some(IncrementalFilter::SkipAll(v)) if v.implementations.contains(&implementation.declaration)
        );
        if skip_all {
            continue;
        }
        let skip_proof = matches!(
            verified,
            Some(IncrementalFilter::SkipProofs(v)) if v.implementations.contains(&implementation.declaration)
        );
        let Some(interface) = implementation.interface else {
            continue;
        };
        let definition = traits_by_id.get(&interface).copied().ok_or_else(|| {
            failure(
                SourceRange { start: 0, end: 0 },
                "trait contract is missing",
            )
        })?;
        for (name, body) in &implementation.methods {
            limits.step(1).map_err(|_| budget(source))?;
            let method = trait_methods
                .get(&(interface, name.as_str()))
                .copied()
                .ok_or_else(|| {
                    failure(
                        SourceRange { start: 0, end: 0 },
                        "trait member contract is missing",
                    )
                })?;
            if method.default == Some(*body) {
                continue;
            }
            // A required-invocation snapshot may carry only the identity of a
            // runtime method. It cannot invoke the body; the complete package
            // still performs inheritance and body verification later.
            if runtime_identities.contains(body)
                && !method.signature.is_comptime_callable()
                && (method.program.preconditions.is_empty()
                    || matches!(method.signature.callable_signature(), Ok(Term::Function { parameters, .. })
                        if parameters.iter().any(|p| p.sort != nir::Sort::Runtime)))
            {
                continue;
            }
            let Some(index) = function_indices.get(body).copied() else {
                // In a required snapshot this slot can name a declaration whose
                // body belongs to a different receiver. Full source checks it.
                if method_links.contains(body) {
                    continue;
                }
                return Err(failure(
                    SourceRange { start: 0, end: 0 },
                    "implementation contract body is missing",
                ));
            };
            let source = package.functions[index].source;
            let mut actual = copy_declaration(
                drafts
                    .get(&index)
                    .unwrap_or(&package.functions[index].interface),
                source,
                limits,
            )?;
            // Omitting the spelling on an override does not weaken the
            // interface behavior promise. The complete body is checked later.
            actual.pure |= method.pure;
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
            } else if !skip_proof {
                let mut obligation = copy_declaration(&actual, source, limits)?;
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
