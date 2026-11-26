//! Conservative static stack-frame upper-bound analysis up to delimited
//! effect boundaries (Silver P0-3 / `meta.runtime.bounded_stack_bytes`).
//!
//! Computes local stack slot bytes + target alignment and traverses only direct,
//! monomorphic local calls up to delimited effect boundaries. Returns `None`
//! whenever recursion, indirect/dynamic calls, unresolved layouts, or missing
//! function bodies prevent a static bound. Post-isel LLVM machine register
//! spills and stack-protector frames are outside NIR scope.
use nir::program::{Expr, ExprKind as K, Function, Package};
use nir::{DeclarationId, PackageLayoutWorld, Term};
use std::collections::{BTreeMap, BTreeSet};

const STACK_ALIGN_BYTES: u64 = 16;

fn align_up(value: u64, align: u64) -> Option<u64> {
    if align <= 1 {
        return Some(value);
    }
    let rem = value % align;
    if rem == 0 {
        Some(value)
    } else {
        value.checked_add(align - rem)
    }
}

/// Compute a conservative static stack byte upper bound for `entry` (and optional
/// type/clause arguments) before any delimited effect boundary.
pub(crate) fn compute_bounded_stack_bytes(
    package: &Package,
    entry: DeclarationId,
    arguments: &[Term],
    fuel: &mut usize,
) -> Option<u64> {
    let target = package.target?;
    if !arguments.is_empty() {
        // Unmonomorphized / polymorphic instantiation arguments are analyzed
        // only when concrete lowered instance bodies exist; check runtime_instances
        // or return None conservatively if only an uninstantiated generic exists.
        if let Some(instance) = package
            .runtime_instances
            .iter()
            .find(|inst| inst.declaration == entry && inst.arguments == arguments)
        {
            let mut visiting = BTreeSet::new();
            let mut memo = BTreeMap::new();
            return analyze_body(
                package,
                target.pointer_bits(),
                entry,
                &instance.function.body,
                &[],
                &mut visiting,
                &mut memo,
                fuel,
            );
        }
    }
    let mut visiting = BTreeSet::new();
    let mut memo = BTreeMap::new();
    analyze_function(
        package,
        target.pointer_bits(),
        entry,
        &mut visiting,
        &mut memo,
        fuel,
    )
}

fn find_function(package: &Package, declaration: DeclarationId) -> Option<&Function> {
    package
        .functions
        .iter()
        .find(|f| f.declaration == declaration)
}

fn analyze_function(
    package: &Package,
    pointer_bits: u32,
    declaration: DeclarationId,
    visiting: &mut BTreeSet<DeclarationId>,
    memo: &mut BTreeMap<DeclarationId, Option<u64>>,
    fuel: &mut usize,
) -> Option<u64> {
    if let Some(&cached) = memo.get(&declaration) {
        return cached;
    }
    if !visiting.insert(declaration) {
        // Direct or mutual recursion cycle cannot be statically bounded here.
        return None;
    }
    *fuel = fuel.checked_sub(1)?;
    let result = if let Some(func) = find_function(package, declaration) {
        if !func.type_parameters.is_empty() {
            None
        } else {
            let param_types: Vec<Term> = match &func.signature.callable_signature() {
                Term::Function { parameters, .. } => {
                    parameters.iter().map(|p| p.ty.clone()).collect()
                }
                _ => Vec::new(),
            };
            analyze_body(
                package,
                pointer_bits,
                declaration,
                &func.body,
                &param_types,
                visiting,
                memo,
                fuel,
            )
        }
    } else {
        None
    };
    visiting.remove(&declaration);
    memo.insert(declaration, result);
    result
}

#[allow(clippy::too_many_arguments)]
fn analyze_body(
    package: &Package,
    pointer_bits: u32,
    _declaration: DeclarationId,
    body: &Expr,
    param_types: &[Term],
    visiting: &mut BTreeSet<DeclarationId>,
    memo: &mut BTreeMap<DeclarationId, Option<u64>>,
    fuel: &mut usize,
) -> Option<u64> {
    let world = PackageLayoutWorld(package);
    let mut local_bytes: u64 = 0;

    for param_ty in param_types {
        let layout = nir::layout::compute_layout(&world, param_ty, fuel).ok()?;
        local_bytes = align_up(local_bytes, layout.align)?;
        local_bytes = local_bytes.checked_add(layout.size)?;
    }

    let mut max_callee_bytes: u64 = 0;
    let mut has_call = false;
    let mut bounded = true;

    let walk = nir::visit::expression_pruned(body, fuel, |expr, fuel| {
        if !bounded {
            return Err(String::new());
        }
        match &expr.kind {
            // Stop at delimited effect request/resume boundaries without tracing outer handlers.
            K::EffectRequest { arguments, .. } => {
                for arg in arguments {
                    let Some(layout) = nir::layout::compute_layout(&world, &arg.ty, fuel).ok()
                    else {
                        bounded = false;
                        return Err(String::new());
                    };
                    let Some(aligned) = align_up(local_bytes, layout.align) else {
                        bounded = false;
                        return Err(String::new());
                    };
                    let Some(next) = aligned.checked_add(layout.size) else {
                        bounded = false;
                        return Err(String::new());
                    };
                    local_bytes = next;
                }
                Ok(false)
            }
            K::Closure { .. } => Ok(false),
            K::Let { value, .. } | K::LetPattern { value, .. } => {
                let Some(layout) = nir::layout::compute_layout(&world, &value.ty, fuel).ok() else {
                    bounded = false;
                    return Err(String::new());
                };
                let Some(aligned) = align_up(local_bytes, layout.align) else {
                    bounded = false;
                    return Err(String::new());
                };
                let Some(next) = aligned.checked_add(layout.size) else {
                    bounded = false;
                    return Err(String::new());
                };
                local_bytes = next;
                Ok(true)
            }
            K::Call { callee, .. } => {
                has_call = true;
                match &callee.kind {
                    K::Function(callee_id) => {
                        let Some(callee_bound) = analyze_function(
                            package,
                            pointer_bits,
                            *callee_id,
                            visiting,
                            memo,
                            fuel,
                        ) else {
                            bounded = false;
                            return Err(String::new());
                        };
                        max_callee_bytes = max_callee_bytes.max(callee_bound);
                        Ok(true)
                    }
                    K::FunctionInstance {
                        declaration: callee_id,
                        arguments: type_args,
                    } => {
                        let callee_bound = if type_args.is_empty() {
                            analyze_function(
                                package,
                                pointer_bits,
                                *callee_id,
                                visiting,
                                memo,
                                fuel,
                            )
                        } else if let Some(inst) = package.runtime_instances.iter().find(|inst| {
                            inst.declaration == *callee_id && inst.arguments == *type_args
                        }) {
                            if !visiting.insert(*callee_id) {
                                bounded = false;
                                return Err(String::new());
                            }
                            let b = analyze_body(
                                package,
                                pointer_bits,
                                *callee_id,
                                &inst.function.body,
                                &[],
                                visiting,
                                memo,
                                fuel,
                            );
                            visiting.remove(callee_id);
                            b
                        } else {
                            None
                        };
                        let Some(callee_bound) = callee_bound else {
                            bounded = false;
                            return Err(String::new());
                        };
                        max_callee_bytes = max_callee_bytes.max(callee_bound);
                        Ok(true)
                    }
                    _ => {
                        // Indirect or dynamic callee target cannot be statically bounded.
                        bounded = false;
                        Err(String::new())
                    }
                }
            }
            K::DynCall(_) => {
                bounded = false;
                Err(String::new())
            }
            _ => Ok(true),
        }
    });
    if !bounded || walk.is_err() {
        return None;
    }

    let own_frame = align_up(local_bytes, STACK_ALIGN_BYTES)?;
    let call_overhead = if has_call {
        u64::from(pointer_bits / 8).checked_mul(2)?
    } else {
        0
    };
    let total = own_frame
        .checked_add(call_overhead)?
        .checked_add(max_callee_bytes)?;
    align_up(total, STACK_ALIGN_BYTES)
}
