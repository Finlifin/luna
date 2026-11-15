//! Optional, transactional removal of proven program checks. This preserves
//! predicate evaluation and only removes the redundant rejection operation.
use super::{Limits, PredicateProof, flow};
use nir::{
    DeclarationId, Primitive, Term,
    program::{Expr, ExprKind as K, Package, SourceRange},
};
use std::{collections::BTreeMap, fmt};

#[derive(Default)]
pub struct ElisionReport {
    pub eliminated: Vec<ElidedCheck>,
    pub fallback_functions: usize,
}
pub struct ElidedCheck {
    pub function: DeclarationId,
    pub source: SourceRange,
    pub proof: PredicateProof,
}
impl fmt::Display for ElisionReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(check-elision (fallback-functions {}) (eliminated",
            self.fallback_functions
        )?;
        for check in &self.eliminated {
            write!(f, " {check}")?;
        }
        f.write_str("))")
    }
}
impl fmt::Display for ElidedCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(elided-check (function {}) (source {} {}) {})",
            self.function, self.source.start, self.source.end, self.proof
        )
    }
}

/// The baseline must already have passed ordinary frontend validation. An
/// unsupported analysis, exhausted budget or failed post-validation retains
/// that function exactly. No partial rewrite is published on failure.
pub fn elide_proven_checks(package: &mut Package, limits: &mut Limits) -> ElisionReport {
    let mut report = ElisionReport::default();
    for index in 0..package.functions.len() {
        if limits.arithmetic.steps == 0 {
            report.fallback_functions += package.functions.len() - index;
            break;
        }
        match elide_function_checks(package, &package.functions[index], limits) {
            Ok(Some((candidate, eliminated))) => {
                package.functions[index] = candidate;
                report.eliminated.extend(eliminated);
            }
            Ok(_) => {}
            Err(()) => report.fallback_functions += 1,
        }
    }
    report
}

/// Analyze a private copy without publishing changes into its source world.
/// Copying and closure detachment share the proof budget. None means no rewrite;
/// errors require the caller to retain its baseline as well.
pub(crate) fn elide_function_checks(
    package: &Package,
    source: &nir::program::Function,
    limits: &mut Limits,
) -> Result<Option<(nir::program::Function, Vec<ElidedCheck>)>, ()> {
    let mut checks = 0;
    nir::visit::expression(&source.body, &mut limits.arithmetic.steps, |expr, _| {
        checks += usize::from(matches!(expr.kind, K::CheckPredicate(_)));
        Ok(())
    })
    .map_err(|_| ())?;
    if checks == 0 {
        return Ok(None);
    }
    // Account for all owned function payloads and closure copy-on-write before
    // cloning. The purity and package drivers share this preparation boundary.
    nir::copy_budget::charge_function_substitution(source, &[], &[], &mut limits.arithmetic.steps)
        .map_err(|_| ())?;
    let mut candidate = source.clone();
    // Detach shared closure bodies before recording ephemeral proof sites.
    // Different capture frames must never rewrite a shared body using one
    // frame's facts. The shared traversal keeps expression allocations stable.
    nir::visit::expression_mut(&mut candidate.body, &mut limits.arithmetic.steps, |_, _| {
        Ok(())
    })
    .map_err(|_| ())?;
    let Ok(analysis) = flow::analyze_in_package(package, &candidate, limits) else {
        return Err(());
    };
    let mut proven = BTreeMap::<usize, Option<PredicateProof>>::new();
    for input in analysis
        .points
        .iter()
        .filter_map(|p| p.check_input.as_ref())
    {
        let proof = input
            .prove(limits)
            .ok()
            .filter(|p| p.replay(&input.facts, limits).is_ok());
        // If a node is visited in more than one analysis context, every
        // visit must justify removal; one successful visit is insufficient.
        proven
            .entry(input.site)
            .and_modify(|previous| {
                if proof.is_none() {
                    *previous = None;
                }
            })
            .or_insert(proof);
    }
    if !proven.values().any(Option::is_some) {
        return Ok(None);
    }
    let mut eliminated = Vec::new();
    let result = nir::visit::expression_mut(
        &mut candidate.body,
        &mut limits.arithmetic.steps,
        |expr, steps| {
            let site = expr as *const Expr as usize;
            if expr.ty != Term::Primitive(Primitive::Void) {
                return Ok(());
            }
            let K::CheckPredicate(condition) = &expr.kind else {
                return Ok(());
            };
            if !nir::program::checkable_predicate(condition, steps)
                .map_err(|error| error.to_string())?
            {
                return Ok(());
            }
            let Some(Some(proof)) = proven.remove(&site) else {
                return Ok(());
            };
            let K::CheckPredicate(condition) = std::mem::replace(&mut expr.kind, K::Unit) else {
                unreachable!()
            };
            expr.kind = K::Discard(condition);
            eliminated.push(ElidedCheck {
                function: candidate.interface.declaration,
                source: expr.source,
                proof,
            });
            Ok(())
        },
    );
    if result.is_err() || flow::analyze_in_package(package, &candidate, limits).is_err() {
        return Err(());
    }
    Ok(Some((candidate, eliminated)))
}
