//! Ordinary input binding for type contracts, separate from program predicates.
use super::*;
use nir::program::TypePredicate as P;

impl Interpreter<'_> {
    /// A dispatch table is retained positive evidence, not a complete account
    /// of the source world's capabilities. Demand missing evidence through the
    /// existing bounded type-query service without installing method bodies.
    pub(super) fn check_capability_contract(
        &mut self,
        requirement: &nir::program::TraitRequirement,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.tick(Some(source))?;
        self.charge_copy_work(Some(source), |fuel| {
            for term in
                std::iter::once(&requirement.subject).chain(&requirement.interface_arguments)
            {
                nir::copy_budget::charge_type_substitution(term, &[], &[], fuel)?;
            }
            Ok(())
        })?;
        if self.capabilities_hold_parameters(std::slice::from_ref(requirement), &[], &[]) {
            return Ok(());
        }
        self.check_type_contract(&P::Trait(requirement.clone()), source)
    }

    pub(super) fn check_capability_parameters(
        &mut self,
        requirements: &[nir::program::TraitRequirement],
        parameters: &[nir::DeclarationParameter],
        arguments: &[Term],
        source: SourceRange,
    ) -> Result<(), Error> {
        for requirement in requirements {
            self.charge_copy_work(Some(source), |fuel| {
                for term in
                    std::iter::once(&requirement.subject).chain(&requirement.interface_arguments)
                {
                    nir::copy_budget::charge_type_substitution(term, parameters, arguments, fuel)?;
                }
                Ok(())
            })?;
            let bound = requirement
                .instantiate_parameters(parameters, arguments)
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            self.check_capability_contract(&bound, source)?;
        }
        Ok(())
    }

    #[inline(never)]
    pub(super) fn check_deferred_capabilities(
        &mut self,
        definition: &nir::program::Function,
        arguments: &[Term],
        scope: Option<&nir::FunctionInputBinding>,
        source: SourceRange,
    ) -> Result<(), Error> {
        for capability in &definition.capabilities {
            let predicate = self.specialize_function_predicate(
                definition,
                arguments,
                &P::Trait(capability.clone()),
                source,
            )?;
            // Closed capabilities were checked against retained evidence or
            // the host world at formation. Deferred obligations need binding.
            if self
                .bind_invocation_contract(&predicate, None, source, 0)?
                .is_none()
            {
                let bound = self
                    .bind_invocation_contract(&predicate, scope, source, 0)?
                    .ok_or_else(|| self.failure(ErrorKind::TypeContractUnresolved, Some(source)))?;
                self.check_type_contract(&bound, source)?;
            }
        }
        Ok(())
    }

    // None means this predicate still reads an ordinary input telescope. Such
    // a predicate waits at function formation and must be closed at invocation.
    pub(super) fn bind_invocation_contract(
        &mut self,
        predicate: &P,
        scope: Option<&nir::FunctionInputBinding>,
        source: SourceRange,
        depth: usize,
    ) -> Result<Option<P>, Error> {
        self.tick(Some(source))?;
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, Some(source)));
        }
        let term = |value: &Term| -> Result<Option<Term>, Error> {
            let value = match scope {
                Some(scope) => scope.rebind(value),
                None => Ok(value.clone()),
            }
            .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            Ok(value.check_scope(&[]).is_ok().then_some(value))
        };
        Ok(Some(match predicate {
            P::Bool(value) => P::Bool(*value),
            P::Equal(a, b) => {
                let (Some(a), Some(b)) = (term(a)?, term(b)?) else {
                    return Ok(None);
                };
                P::Equal(a, b)
            }
            P::Trait(requirement) => {
                let Some(subject) = term(&requirement.subject)? else {
                    return Ok(None);
                };
                let mut arguments = Vec::new();
                for argument in &requirement.interface_arguments {
                    let Some(argument) = term(argument)? else {
                        return Ok(None);
                    };
                    arguments.push(argument);
                }
                P::Trait(nir::program::TraitRequirement {
                    subject,
                    interface: requirement.interface,
                    interface_arguments: arguments,
                })
            }
            P::Not(inner) => {
                let Some(inner) = self.bind_invocation_contract(inner, scope, source, depth + 1)?
                else {
                    return Ok(None);
                };
                P::Not(Box::new(inner))
            }
            P::And(parts) | P::Or(parts) => {
                let mut bound = Vec::new();
                for part in parts {
                    let Some(part) =
                        self.bind_invocation_contract(part, scope, source, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    bound.push(part);
                }
                if matches!(predicate, P::And(_)) {
                    P::And(bound)
                } else {
                    P::Or(bound)
                }
            }
        }))
    }
}
