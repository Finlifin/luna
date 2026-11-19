//! Operand checking in the caller's proof context. Pending input subjects
//! belong to a single query scope and never enter the packet sent to the VM.
use super::*;
use declaration_inputs::CallableBindings;

pub(super) struct Declaration {
    pub(super) owner: OwnerId,
    pub(super) parameters: Vec<DeclarationId>,
    pub(super) domains: Vec<Term>,
}

impl std::fmt::Display for Declaration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(clause-declaration (owner {}) (parameters", self.owner)?;
        for (id, domain) in self.parameters.iter().zip(&self.domains) {
            write!(f, " (input {id} {domain})")?;
        }
        f.write_str("))")
    }
}

impl Declaration {
    pub(super) fn from_telescope(
        owner: OwnerId,
        telescope: declaration_inputs::PreparedTelescope,
    ) -> (Self, Vec<Term>) {
        let (mut parameters, mut domains, prefix) = telescope.captured.into_parts();
        for (id, _, domain) in telescope.local {
            parameters.push(id);
            domains.push(domain);
        }
        (
            Self {
                owner,
                parameters,
                domains,
            },
            prefix,
        )
    }

    /// Validate one completed packet in the restored caller scope, including
    /// dependent binder sorts. Every result slot uses this same substitution.
    pub(super) fn check_completed(
        &self,
        checker: &mut Checker<'_, '_>,
        actual: &[Term],
        values: &[Term],
        span: Span,
    ) -> Result<(), Error> {
        if actual.len() != self.domains.len() {
            return Err(error(span, "clause input packet domain arity mismatch"));
        }
        let frames = checker.call_input_frames();
        let substitution = nominal_fields::metered(&mut checker.required_steps, |fuel| {
            nir::DeclarationSubstitution::bounded(
                &self.parameters,
                &self.domains,
                values,
                &frames,
                fuel,
            )
        })
        .map_err(|failure| match failure {
            nir::type_computations::Error::Scope(reason @ nir::TypeError::WrongSort { .. }) => {
                error(span, format!("clause input type mismatch: {reason}"))
            }
            reason => error(span, reason.to_string()),
        })?;
        for (actual, template) in actual.iter().zip(&self.domains) {
            let expected = nominal_fields::metered(&mut checker.required_steps, |fuel| {
                substitution.instantiate_bounded(template, fuel)
            })
            .map_err(|failure| error(span, failure.to_string()))?;
            checker.require(actual, &expected, span)?;
        }
        Ok(())
    }

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
        // This synchronous proof neither publishes definitions nor calls the VM.
        // Its temporary rigids expire before the source allocator runs again.
        let package = declaration(checker.package, &checker.package_identities, self.owner).package;
        let mut next = checker.definition_identities.next();
        let mut subjects = Vec::new();
        for ((id, domain), value) in self
            .parameters
            .iter()
            .zip(&self.domains)
            .zip(&formation.terms)
        {
            let subject = if formation.deferred.contains(id) {
                let domain = bindings.instantiate(domain, steps, span)?;
                let sort = Sort::of_comptime_domain(&domain);
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
