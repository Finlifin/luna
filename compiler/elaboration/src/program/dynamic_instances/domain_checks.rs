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
        self.check_operand_domains(
            checker,
            bindings,
            formation,
            operands.iter().zip(&self.domains),
            steps,
            span,
        )
    }

    pub(super) fn check_contextual_domains(
        &self,
        checker: &Checker<'_, '_>,
        bindings: &CallableBindings,
        formation: &FormationInputs,
        operands: &[n::NormalFormArgument],
        steps: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        use n::NormalFormArgument as A;
        if operands.len() != self.domains.len() {
            return Err(error(span, "clause input packet domain arity mismatch"));
        }
        let mut charge = |count: usize| {
            *steps = steps
                .checked_sub(count as u64)
                .ok_or_else(|| error(span, "contextual input domain budget exhausted"))?;
            Ok::<(), Error>(())
        };
        charge(operands.len())?;
        let mut work: Vec<_> = operands.iter().zip(&self.domains).collect();
        let mut typed = Vec::new();
        while let Some((operand, domain)) = work.pop() {
            match operand {
                A::Value(expr) | A::ListSpread(expr) => typed.push((expr, domain)),
                A::Block { result, .. } => {
                    if let Some(result) = result.as_deref() {
                        charge(1)?;
                        work.push((result, domain));
                    }
                }
                A::If {
                    then_input,
                    else_input,
                    ..
                } => {
                    charge(2)?;
                    work.push((then_input, domain));
                    work.push((else_input, domain));
                }
                A::Match { arms, .. } => {
                    charge(arms.len())?;
                    work.extend(arms.iter().map(|arm| (&arm.body, domain)));
                }
                A::List { items, .. } => {
                    if let Some(element) = checker.list_element_type(domain) {
                        charge(items.len())?;
                        work.extend(items.iter().map(|item| {
                            (
                                item,
                                if matches!(item, A::ListSpread(_)) {
                                    domain
                                } else {
                                    element
                                },
                            )
                        }));
                    }
                }
                A::Tuple { items, .. } => {
                    if let Some(fields) =
                        checker
                            .bound_tuple_elements(domain)
                            .or_else(|| match domain {
                                Term::Tuple(fields) => Some(fields.as_slice()),
                                _ => None,
                            })
                    {
                        if fields.len() != items.len() {
                            return Err(error(span, "contextual tuple arity mismatch"));
                        }
                        charge(items.len())?;
                        work.extend(items.iter().zip(fields));
                    }
                }
                A::Unit { .. } | A::Null { .. } | A::Integer { .. } => {}
            }
        }
        self.check_operand_domains(checker, bindings, formation, typed, steps, span)
    }

    fn check_operand_domains<'a>(
        &self,
        checker: &Checker<'_, '_>,
        bindings: &CallableBindings,
        formation: &FormationInputs,
        operands: impl IntoIterator<Item = (&'a n::Expr, &'a Term)>,
        steps: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        let mut pending = Vec::new();
        for (operand, template) in operands {
            if !bindings.awaits_input(template, steps, span)? {
                let domain = bindings.instantiate(template, steps, span)?;
                checker.require(&operand.ty, &domain, span)?;
            } else if !is_never(&operand.ty) {
                pending.push((operand, template));
            }
        }
        if pending.is_empty() {
            return Ok(());
        }

        // A single execution supplies one substitution for the whole packet.
        // Two individually possible domains can still require incompatible
        // identities for the same input.
        let proof = self.proof_substitution(checker, bindings, formation, steps, span)?;
        let mut equalities = Vec::with_capacity(pending.len());
        for (operand, template) in &pending {
            let expected =
                nominal_fields::metered(steps, |fuel| proof.instantiate_bounded(template, fuel))
                    .map_err(|reason| error(span, reason.to_string()))?;
            let (actual, expected) =
                checker.scoped_equality_operands(&operand.ty, &expected, span)?;
            equalities.push(crate::ptc::Formula::equal(actual, expected));
        }
        if disproves_domains(checker, equalities, steps, span)? {
            let mut failure = error(
                span,
                "type mismatch: no common substitution satisfies the clause input domains",
            );
            for (operand, template) in pending {
                failure.notes.push(DiagnosticNote {
                    source: operand.source,
                    message: format!(
                        "operand of type {} supplied for domain {template}",
                        operand.ty
                    ),
                });
            }
            return Err(failure);
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
fn disproves_domains(
    checker: &Checker<'_, '_>,
    equalities: Vec<crate::ptc::Formula>,
    steps: &mut u64,
    span: Span,
) -> Result<bool, Error> {
    use crate::ptc::{Budget, Formula, Outcome, UnknownReason};
    let before = usize::try_from(*steps).unwrap_or(usize::MAX);
    let mut budget = Budget {
        steps: before,
        ..Budget::default()
    };
    let outcome = checker
        .context
        .prove_metered(Formula::And(equalities).negate(), &mut budget);
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
