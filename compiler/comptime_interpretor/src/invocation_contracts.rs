//! Ordinary input binding for type contracts, separate from program predicates.
use super::*;
use nir::program::TypePredicate as P;

impl Interpreter<'_> {
    #[inline(never)]
    pub(super) fn check_deferred_capabilities(
        &mut self,
        definition: &nir::program::Function,
        arguments: &[Term],
        scope: Option<&nir::FunctionInputBinding>,
        source: SourceRange,
    ) -> Result<(), Error> {
        for capability in &definition.capabilities {
            // Enclosing nominal parameters can be values as well as Types.
            // Keep their declared sorts when closing invocation obligations.
            let capability = capability
                .instantiate_parameters(&self.input_binders(&definition.type_parameters), arguments)
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            let predicate = P::Trait(capability);
            // Closed capabilities already use the executable/assumed dispatch
            // world. Only obligations deferred at formation need input binding.
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
