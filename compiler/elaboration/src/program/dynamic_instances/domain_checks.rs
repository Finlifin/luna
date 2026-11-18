//! Operand checking in the caller's proof context. Pending input subjects
//! belong to a single query scope and never enter the packet sent to the VM.
use super::*;
use declaration_inputs::CallableBindings;

impl CallableCompletion {
    pub(super) fn check_domains(
        &self,
        checker: &Checker<'_, '_>,
        bindings: &CallableBindings,
        formation: &FormationInputs,
        operands: &[n::Expr],
        steps: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        let mut proof = None;
        for (operand, template) in operands.iter().zip(&self.domains) {
            if !bindings.awaits_input(template, steps, span)? {
                let domain = bindings.instantiate(template, steps, span)?;
                checker.require(&operand.ty, &domain, span)?;
                continue;
            }
            let proof = match &proof {
                Some(proof) => proof,
                None => proof
                    .insert(self.proof_substitution(checker, bindings, formation, steps, span)?),
            };
            let expected =
                nominal_fields::metered(steps, |fuel| proof.instantiate_bounded(template, fuel))
                    .map_err(|reason| error(span, reason.to_string()))?;
            if disproves_equality(checker, &operand.ty, &expected, steps, span)? {
                let domain = bindings.instantiate(template, steps, span)?;
                return Err(error(
                    span,
                    format!("type mismatch: expected {domain}, found {}", operand.ty),
                ));
            }
        }
        Ok(())
    }

    fn proof_substitution(
        &self,
        checker: &Checker<'_, '_>,
        bindings: &CallableBindings,
        formation: &FormationInputs,
        steps: &mut u64,
        span: Span,
    ) -> Result<nir::DeclarationSubstitution<'static>, Error> {
        // The shared checker borrow excludes definition allocation throughout
        // this proof. Its first unused identity starts a private range; querying
        // an operand cannot advance the cursor or invalidate prepared code.
        let package = declaration(checker.package, &checker.package_identities, self.owner).package;
        let mut next = checker.next_definition_identity;
        let mut subjects = Vec::new();
        for ((id, domain), value) in self
            .parameters
            .iter()
            .zip(&self.domains)
            .zip(&formation.terms)
        {
            let subject = if formation.deferred.contains(id) {
                let domain = bindings.instantiate(domain, steps, span)?;
                let sort = if domain == p(P::Type) {
                    Sort::Type
                } else {
                    Sort::ComptimeValue
                };
                let declaration = DeclarationId {
                    package,
                    local: next,
                };
                next = next
                    .checked_add(1)
                    .ok_or_else(|| error(span, "proof binder identity space exhausted"))?;
                Term::Rigid { declaration, sort }
            } else {
                declaration_inputs::copy_input_domain(value, steps, span)?
            };
            subjects.push(subject);
        }
        // Simultaneous substitution preserves caller terms even if a caller
        // rigid has the same declaration identity as a pending callee slot.
        let frames = checker.call_input_frames();
        nominal_fields::metered(steps, |fuel| {
            nir::DeclarationSubstitution::bounded_owned(
                &self.parameters,
                &self.domains,
                subjects,
                frames,
                fuel,
            )
        })
        .map_err(|reason| error(span, reason.to_string()))
    }
}

/// A negative proof can reject a contradiction in an unused definition.
/// All other admissible cases still need the completed VM domain check.
fn disproves_equality(
    checker: &Checker<'_, '_>,
    actual: &Term,
    expected: &Term,
    steps: &mut u64,
    span: Span,
) -> Result<bool, Error> {
    use crate::ptc::{Budget, Formula, Outcome, UnknownReason};
    if is_never(actual) {
        return Ok(false);
    }
    let (actual, expected) = checker.scoped_equality_operands(actual, expected, span)?;
    let mut budget = Budget::default();
    let before = budget
        .steps
        .min(usize::try_from(*steps).unwrap_or(usize::MAX));
    budget.steps = before;
    let outcome = checker
        .context
        .prove_metered(Formula::equal(actual, expected).negate(), &mut budget);
    *steps -= (before - budget.steps) as u64;
    match outcome {
        Outcome::Entailed(_) => Ok(true),
        Outcome::NotEntailed(_)
        | Outcome::Unknown {
            reason: UnknownReason::UnvalidatedModel,
            ..
        } => Ok(false),
        Outcome::Unknown { reason, .. } => Err(error(
            span,
            format!("pending input domain observation failed ({reason:?})"),
        )),
        Outcome::IllFormed(reason) => Err(error(
            span,
            format!("ill-formed pending input domain: {reason}"),
        )),
    }
}
