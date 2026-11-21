//! A match test may write enclosing locals on either exit. Binding evidence
//! is killed by a write, so the join retains facts common to all incoming paths.
use super::*;

type Bindings<'a> = BTreeMap<n::LocalId, &'a n::Expr>;

enum Observation<'a> {
    Expression(&'a n::Expr),
    UnknownWrites,
}

impl Observation<'_> {
    fn invalidate(self, bindings: &mut Bindings<'_>, fuel: &mut usize) -> Result<(), String> {
        match self {
            Self::Expression(expr) => {
                Checker::invalidate_mutated_type_bindings(expr, bindings, fuel)
                    .map_err(String::from)
            }
            Self::UnknownWrites => {
                bindings.clear();
                Ok(())
            }
        }
    }
}

pub(super) fn arm_bindings<'a, 'e>(
    arm: &'e n::MatchArm,
    fallthrough: &mut Bindings<'a>,
    fuel: &mut usize,
) -> Result<Bindings<'a>, String> {
    let mut accepted_only = Vec::new();
    if !fallthrough.is_empty() {
        // Walk backwards from the two destinations. Killing a fact is
        // idempotent and independent of the order of other kills.
        let mut observe =
            |observation: Observation<'e>, may_fall_through: bool, fuel: &mut usize| {
                if may_fall_through {
                    observation.invalidate(fallthrough, fuel)
                } else {
                    accepted_only.push(observation);
                    Ok(())
                }
            };
        let mut accept = false;
        if let Some(guard) = &arm.guard {
            accept = reaches_after_guard(guard, false, true, fuel)?;
            observe(Observation::Expression(guard), accept, fuel)?;
        }
        pattern(&arm.pattern, accept, true, fuel, 0, &mut observe)?;
    }
    let mut bindings = fallthrough.clone();
    for observation in accepted_only {
        observation.invalidate(&mut bindings, fuel)?;
    }
    nir::free_bindings::remove_pattern_bindings(&arm.pattern, &mut bindings, fuel)?;
    Ok(bindings)
}

// Only a normally completed guard can select an arm or try the next one.
// Reading a block's tail here does not evaluate or discard its statements.
fn reaches_after_guard(
    mut expr: &n::Expr,
    yes: bool,
    no: bool,
    fuel: &mut usize,
) -> Result<bool, String> {
    loop {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("match guard evidence budget exhausted")?;
        if is_never(&expr.ty) {
            return Ok(false);
        }
        match &expr.kind {
            K::Literal(n::Literal::Bool(test)) => return Ok(if *test { yes } else { no }),
            K::Block {
                result: Some(tail), ..
            }
            | K::CallPreparation {
                result: Some(tail), ..
            }
            | K::UnsafeScope(tail) => expr = tail,
            _ => return Ok(yes || no),
        }
    }
}

// `yes` and `no` indicate whether success or failure of this subpattern can
// eventually try a later arm. Report each executable observation once.
fn pattern<'e>(
    pat: &'e n::Pattern,
    yes: bool,
    no: bool,
    fuel: &mut usize,
    depth: usize,
    observe: &mut impl FnMut(Observation<'e>, bool, &mut usize) -> Result<(), String>,
) -> Result<bool, String> {
    use n::Pattern as P;
    *fuel = fuel
        .checked_sub(1)
        .ok_or("match pattern evidence budget exhausted")?;
    if depth >= 128 {
        // Bound the analysis stack. An opaque subtree may write any local;
        // types returned independently of those locals still admit a proof.
        observe(Observation::UnknownWrites, yes || no, fuel)?;
        return Ok(yes || no);
    }
    match pat {
        P::Wild => Ok(yes),
        P::Binding { inner, .. } | P::Observe { inner, .. } | P::Remap { pattern: inner, .. } => {
            pattern(inner, yes, no, fuel, depth + 1, observe)
        }
        P::Guard {
            pattern: inner,
            condition,
        } => {
            let after = reaches_after_guard(condition, yes, no, fuel)?;
            observe(Observation::Expression(condition), after, fuel)?;
            pattern(inner, after, no, fuel, depth + 1, observe)
        }
        P::Then { first, value, next } => {
            let after = pattern(next, yes, no, fuel, depth + 1, observe)? && !is_never(&value.ty);
            observe(Observation::Expression(value), after, fuel)?;
            pattern(first, after, no, fuel, depth + 1, observe)
        }
        P::Or(alternatives) => {
            let mut reject = no;
            for alternative in alternatives.iter().rev() {
                reject = pattern(alternative, yes, reject, fuel, depth + 1, observe)?;
            }
            Ok(reject)
        }
        P::Tuple(fields)
        | P::TypeTuple(fields)
        | P::Variant { fields, .. }
        | P::Record { fields, .. }
        | P::AnonymousRecord { fields, .. }
        | P::NominalType { fields, .. }
        | P::NormalForm {
            arguments: fields, ..
        } => {
            let mut accept = yes;
            for field in fields.iter().rev() {
                accept = pattern(field, accept, no, fuel, depth + 1, observe)?;
            }
            // The shape/tag check precedes every field.
            Ok(accept || no)
        }
        P::OptionalPresent(inner) | P::ErrorResult { inner, .. } => {
            Ok(pattern(inner, yes, no, fuel, depth + 1, observe)? || no)
        }
        P::Literal(_) | P::Unit | P::OptionalAbsent => Ok(yes || no),
        P::CodeEvidence { .. } => {
            // Open code has no executable body to inspect at definition time.
            observe(Observation::UnknownWrites, yes || no, fuel)?;
            Ok(yes || no)
        }
    }
}
