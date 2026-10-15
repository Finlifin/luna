//! Trait program-contract replacement; no obligation here is a type proposition.
use super::{Limits, flow};
use nir::{Primitive, Term, contracts::remap_predicate, program::*};
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
fn remap(values: &mut [Expr], bindings: &BTreeMap<LocalId, LocalId>) -> Result<(), flow::Error> {
    for value in values {
        remap_predicate(value, bindings).map_err(|message| failure(value.source, message))?;
    }
    Ok(())
}
/// Prepare effective body contracts, then verify every explicit weakening.
/// Each input package is a fresh elaboration snapshot; do not apply twice.
pub fn inherit(package: &mut Package, next: &mut u32) -> Result<(), flow::Error> {
    for implementation in package.implementations.clone() {
        let Some(interface) = implementation.interface else {
            continue;
        };
        let definition = package
            .traits
            .iter()
            .find(|t| t.declaration == interface)
            .cloned()
            .ok_or_else(|| {
                failure(
                    SourceRange { start: 0, end: 0 },
                    "trait contract is missing",
                )
            })?;
        let arguments: Vec<_> = std::iter::once(implementation.subject.clone())
            .chain(implementation.interface_arguments.clone())
            .collect();
        for (name, body) in &implementation.methods {
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
            // A required-invocation snapshot may carry only the identity of a
            // runtime method. It cannot invoke the body; the complete package
            // still performs inheritance and body verification later.
            if package
                .runtime_function_identities
                .iter()
                .any(|identity| identity.declaration == *body)
                && !method.signature.is_comptime_callable()
                && method.program.preconditions.is_empty()
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
            let mut actual = package.functions[index].clone();
            // Omitting the spelling on an override does not weaken the
            // interface behavior promise. The complete body is checked later.
            actual.pure |= method.pure;
            let source = actual.body.source;
            let parameters: Vec<_> = definition
                .binders_with_self(&mut 100_000)
                .map_err(|_| failure(source, "invalid trait parameter domains"))?
                .into_iter()
                .chain(
                    method
                        .type_parameters
                        .iter()
                        .map(|id| nir::DeclarationParameter {
                            declaration: *id,
                            sort: nir::Sort::Type,
                        }),
                )
                .collect();
            let method_arguments: Vec<_> = arguments
                .iter()
                .cloned()
                .chain(
                    actual
                        .type_parameters
                        .iter()
                        .skip(implementation.type_parameters.len())
                        .map(|id| nir::Term::Rigid {
                            declaration: *id,
                            sort: nir::Sort::Type,
                        }),
                )
                .collect();
            let mut contract = method
                .program
                .instantiate_parameters(&parameters, &method_arguments)
                .map_err(|_| failure(source, "trait program contract substitution failed"))?;
            if method.parameter_bindings.len() != actual.parameters.len() {
                return Err(failure(source, "trait program contract arity mismatch"));
            }
            let mut bindings: BTreeMap<_, _> = method
                .parameter_bindings
                .iter()
                .copied()
                .zip(actual.parameters.iter().copied())
                .collect();
            remap(&mut contract.preconditions, &bindings)?;
            let inherited_domain = actual.preconditions.is_empty();
            if inherited_domain {
                // An omitted program entry clause uses the interface domain.
                actual.preconditions = contract.preconditions.clone();
            } else {
                let mut obligation = actual.clone();
                let Term::Function { result, .. } = &mut obligation.signature else {
                    return Err(failure(source, "invalid implementation signature"));
                };
                *result = Box::new(Term::Primitive(Primitive::Void));
                obligation.body = Expr {
                    source,
                    ty: Term::Primitive(Primitive::Void),
                    kind: ExprKind::Unit,
                };
                obligation.preconditions = contract.preconditions.clone();
                obligation.postconditions = actual.preconditions.clone();
                obligation.result_binding = None;
                obligation.entry_snapshots.clear();
                obligation.defaults.clear();
                flow::analyze_in_package(package, &obligation, &mut Limits::default())
                    .map_err(|_| failure(source, "implementation strengthens trait program preconditions or their definedness is not proven"))?;
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
            remap(&mut contract.postconditions, &bindings)?;
            // A weaker implementation domain must not make trait ensures apply
            // to inputs outside the trait's domain. Guard with frozen ENTRY
            // predicates, never with the possibly modified exit parameters.
            if inherited_domain {
                contract.preconditions.clear();
            }
            let mut entry = BTreeMap::new();
            if !contract.preconditions.is_empty() && !contract.postconditions.is_empty() {
                for parameter in &actual.parameters {
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
            remap(&mut contract.preconditions, &entry)?;
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
                                kind: ExprKind::Not(Box::new(guard.clone())),
                            }),
                            Box::new(postcondition),
                        ),
                    }
                } else {
                    postcondition
                };
                actual.postconditions.push(postcondition);
            }
            package.functions[index] = actual;
        }
    }
    Ok(())
}
