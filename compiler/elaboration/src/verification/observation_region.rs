//! Structural envelope shared by observation blocks and generated return regions.
//! Ownership, Copy evidence,
//! call rejection and memory provenance remain obligations of their analyzers.
use super::pattern_facts::{bounded_outer_guards, cannot_complete, constant_bool, known_pattern};
use nir::{
    Term,
    program::{Expr, ExprKind as K, LocalId, Package, Pattern},
};
use std::collections::BTreeSet;

fn reachable(
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
    visitor: &mut dyn FnMut(&Expr, &mut usize) -> Result<bool, String>,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("observation region nesting budget exhausted".into());
    }
    nir::visit::expression_pruned(expr, fuel, |node, fuel| {
        if !visitor(node, fuel)? {
            return Ok(false);
        }
        match &node.kind {
            K::Match { value, arms } => {
                reachable(value, fuel, depth + 1, visitor)?;
                for arm in arms {
                    *fuel = fuel
                        .checked_sub(1)
                        .ok_or("observation arm budget exhausted")?;
                    let (pattern, mut guards) = bounded_outer_guards(&arm.pattern, fuel)?;
                    guards.extend(arm.guard.as_ref());
                    let observations = nir::visit::pattern_observations(pattern, fuel)?;
                    let known = known_pattern(pattern, value, 0);
                    if known == Some(false) {
                        continue;
                    }
                    for observation in observations {
                        reachable(observation, fuel, depth + 1, visitor)?;
                    }
                    let mut accepted = Some(true);
                    for guard in guards {
                        reachable(guard, fuel, depth + 1, visitor)?;
                        match constant_bool(guard, 0) {
                            Some(false) => {
                                accepted = Some(false);
                                break;
                            }
                            None => accepted = None,
                            _ => {}
                        }
                    }
                    if accepted != Some(false) {
                        reachable(&arm.body, fuel, depth + 1, visitor)?;
                    }
                    if known == Some(true) && accepted == Some(true) {
                        break;
                    }
                }
                Ok(false)
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } if constant_bool(condition, 0).is_some() => {
                reachable(condition, fuel, depth + 1, visitor)?;
                if constant_bool(condition, 0) == Some(true) {
                    reachable(then_branch, fuel, depth + 1, visitor)?;
                } else if let Some(branch) = else_branch {
                    reachable(branch, fuel, depth + 1, visitor)?;
                }
                Ok(false)
            }
            K::And(left, _) | K::Or(left, _)
                if node.ty == nir::Term::Primitive(nir::Primitive::Bool)
                    && constant_bool(left, 0) == Some(matches!(node.kind, K::Or(..))) =>
            {
                reachable(left, fuel, depth + 1, visitor)?;
                Ok(false)
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                for statement in statements {
                    reachable(statement, fuel, depth + 1, visitor)?;
                    if cannot_complete(statement, fuel, 0)? {
                        return Ok(false);
                    }
                }
                if let Some(result) = result {
                    reachable(result, fuel, depth + 1, visitor)?;
                }
                Ok(false)
            }
            _ => Ok(true),
        }
    })
}

fn packet(ty: &Term, package: &Package) -> bool {
    matches!(ty,Term::Nominal {declaration,..} if package.internal_return_packets.contains(declaration))
}
fn value(ty: &Term, package: &Package, fuel: &mut usize) -> Result<bool, String> {
    if matches!(ty, Term::Primitive(_)) && nir::program::intrinsic_pointer_value(ty)
        || matches!(
            ty,
            Term::Primitive(nir::Primitive::Void | nir::Primitive::Never)
        )
        || matches!(
            ty,
            Term::Function { .. } | Term::UnsafeFunction(_) | Term::Pointer(_)
        )
        || packet(ty, package)
    {
        return Ok(true);
    }
    if matches!(
        ty,
        Term::Nominal { .. }
            | Term::Record(_)
            | Term::Tuple(_)
            | Term::NormalForm { .. }
            | Term::ErrorResult { .. }
    ) {
        // Shape admission is not delivery or cleanup permission. The resource
        // pass retains pointer origins and proves observation-local cleanup.
        return super::memory::copy_readable_representation(Some(package), ty, fuel);
    }
    Ok(false)
}

fn pattern_locals(
    pattern: &Pattern,
    locals: &mut BTreeSet<LocalId>,
    fuel: &mut usize,
) -> Result<(), String> {
    let mut pending = vec![pattern];
    while let Some(pattern) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("observation pattern binding budget exhausted")?;
        match pattern {
            Pattern::CodeEvidence { .. } => return Err("unresolved pattern-code evidence".into()),
            Pattern::Observe { binding, inner } | Pattern::Binding { binding, inner } => {
                locals.insert(*binding);
                pending.push(inner);
            }
            Pattern::Remap { pattern, bindings } => {
                *fuel = fuel
                    .checked_sub(bindings.len())
                    .ok_or("observation remap budget exhausted")?;
                locals.extend(bindings.iter().map(|(_, target)| *target));
                pending.push(pattern);
            }
            Pattern::Then { first, next, .. } => {
                pending.push(next);
                pending.push(first);
            }
            Pattern::OptionalPresent(pattern)
            | Pattern::ErrorResult { inner: pattern, .. }
            | Pattern::Guard { pattern, .. } => pending.push(pattern),
            Pattern::Tuple(fields)
            | Pattern::TypeTuple(fields)
            | Pattern::Or(fields)
            | Pattern::NominalType { fields, .. }
            | Pattern::Record { fields, .. }
            | Pattern::AnonymousRecord { fields, .. }
            | Pattern::Variant { fields, .. }
            | Pattern::NormalForm {
                arguments: fields, ..
            } => {
                *fuel = fuel
                    .checked_sub(fields.len())
                    .ok_or("observation pattern field budget exhausted")?;
                pending.extend(fields);
            }
            Pattern::OptionalAbsent | Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => {}
        }
    }
    Ok(())
}
pub(super) fn check(
    expr: &Expr,
    package: &Package,
    fuel: &mut usize,
) -> Result<BTreeSet<LocalId>, String> {
    let mut locals = BTreeSet::new();
    let mut loops = BTreeSet::new();
    reachable(expr, fuel, 0, &mut |node, fuel| {
        match &node.kind {
            K::Let {
                binding,
                value: input,
            } => {
                if !value(&input.ty, package, fuel)? || !locals.insert(*binding) {
                    return Err(
                        "observation region requires fresh observation-local bindings".into(),
                    );
                }
            }
            K::LetPattern {
                pattern,
                value: input,
                ..
            } => {
                if !value(&input.ty, package, fuel)? {
                    return Err(
                        "observation region pattern requires a readable observation input".into(),
                    );
                }
                pattern_locals(pattern, &mut locals, fuel)?;
            }
            K::Match { arms, .. } => {
                for arm in arms {
                    pattern_locals(&arm.pattern, &mut locals, fuel)?;
                }
            }
            K::Loop { identity, .. } => {
                if !loops.insert(*identity) {
                    return Err("duplicate observation loop identity".into());
                }
            }
            K::Closure { captures, .. } if captures.is_empty() => return Ok(false),
            _ => {}
        }
        Ok(true)
    })?;
    reachable(expr, fuel, 0, &mut |node, fuel| {
        match &node.kind {
            K::Intrinsic { .. } => {
                nir::intrinsics::checked(node).map_err(str::to_owned)?;
            }

            K::Let { .. }
            | K::LetPattern { .. }
            | K::Block { .. }
            | K::CallPreparation { .. }
            | K::UnsafeScope(_)
            | K::Discard(_)
            | K::If { .. }
            | K::Match { .. }
            | K::Loop { .. }
            | K::ComptimeValue(_)
            | K::Literal(_)
            | K::Unit
            | K::Local(_)
            | K::Function(_)
            | K::FunctionInstance { .. }
            | K::Field { .. }
            | K::AnonymousField { .. }
            | K::AddressOf(_)
            | K::ReadPointer(_)
            | K::PointerField { .. }
            | K::PointerAnonymousField { .. }
            // Provenance checks in the resource walk must establish that every
            // possible destination belongs to this observation's private locals.
            | K::WritePointer { .. }
            | K::BitWritePointer { .. }
            | K::Not(_)
            | K::IntegerNegate(_)
            | K::FloatNegate(_)
            | K::BitConvert(_)
            | K::NewtypeConvert(_)
            | K::Snapshot(_)
            | K::TakeBits(_)
            | K::TextQuery { subject: _, .. }
            | K::IntegerRepresentation(_)
            | K::ComptimeIntegerConvert(_)
            | K::SymbolName(_)
            | K::EnumName(_)
            | K::BitSlice { .. }
            | K::BitWindow { .. }
            | K::And(..)
            | K::Or(..)
            | K::TemplateEqual(..)
            | K::ScalarEqual(..)
            | K::ScalarLess(..)
            | K::ScalarLessEqual(..)
            | K::ScalarGreater(..)
            | K::ScalarGreaterEqual(..)
            | K::BitEqual(..)
            | K::BitConcat(..)
            | K::BitIndex(..)
            | K::IntegerBinary { .. }
            | K::FloatBinary { .. }
            | K::Call { .. } => {}
            K::Tuple(_)
            | K::Record { .. }
            | K::AnonymousRecord { .. }
            | K::Variant { .. }
            | K::ErrorConvert { .. }
                if value(&node.ty, package, fuel)? => {}
            K::Assign {
                place,
                value: input,
            } if locals.contains(&place.root) && value(&input.ty, package, fuel)? => {
                // Direct fields belong to this private root. Ordinary resource
                // analysis validates their layout, state and replacement cleanup;
                // pointer-target writes separately check their actual provenance.
            }
            K::BitAssign { place, .. } | K::BitUpdate { place, .. }
                if locals.contains(&place.root) =>
            {
                // Private bit storage follows the same ownership boundary as
                // direct assignment. Bounds and Copy remain ordinary checks.
            }
            K::Break(identity) | K::Continue(identity) if loops.contains(identity) => {}
            K::RequirePredicate(condition)
            | K::EnsurePredicate(condition)
            | K::CheckPredicate(condition)
                if super::pattern_facts::constant_bool(condition, 0) == Some(true) => {}
            K::Closure { captures, .. } if captures.is_empty() => return Ok(false),
            _ => return Err(
                "observation region changes external state or needs failure-path resource transfer"
                    .into(),
            ),
        }
        Ok(true)
    })?;
    Ok(locals)
}
