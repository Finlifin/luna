//! Observe a named function instance in its clause preparation scope.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn observe_factory_instance(
        &mut self,
        declaration: DeclarationId,
        inputs: &n::ClauseOperands,
        scope: &mut ObservationScope,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<Completion, Error> {
        if self.checked_traits.method(declaration).is_some() {
            // A member carries its interface identity until dispatch.
            return Ok(Completion::Unavailable);
        }
        let mut inner = self.schema_fuel(span, |_, fuel| scope.fork(fuel))?;
        for statement in &inputs.statements {
            let step = self.observe_factory_expr(statement, &mut inner, returns, span, depth)?;
            if !matches!(step, Completion::Next(_)) {
                return Ok(step);
            }
        }
        let mut arguments = Vec::new();
        for operand in &inputs.arguments {
            let step = self.observe_factory_expr(operand, &mut inner, returns, span, depth)?;
            let Completion::Next(value) = step else {
                return Ok(step);
            };
            if let Some(term) = value.known_term() {
                arguments.push(self.schema_fuel(span, |_, fuel| {
                    nir::copy_budget::clone_term(term, fuel)
                        .map_err(|_| "structural function instance budget exhausted".to_string())
                })?);
            }
        }
        self.schema_fuel(span, |_, fuel| {
            inner.retain(|id| scope.values.contains_key(id), fuel)
        })?;
        *scope = inner;
        let value = if arguments.len() == inputs.arguments.len() {
            PartialStagedValue::known(Term::FunctionValue {
                declaration,
                arguments,
            })
        } else {
            PartialStagedValue::unknown()
        };
        Ok(Completion::Next(value))
    }

    // The call carries the public interface's default expression. Only its
    // declaration inputs come from the function instance selected at runtime.
    pub(super) fn instance_default_substitution(
        &self,
        id: DeclarationId,
        indices: &[Term],
        fuel: &mut usize,
    ) -> Result<Option<nir::DeclarationSubstitution<'static>>, String> {
        let (signature, parameters) = if let Some(body) = self.generated_functions.body(id) {
            (&body.signature, &body.type_parameters)
        } else {
            let Some(owner) = self.source_declarations.owner(id) else {
                return Ok(None);
            };
            let (Some(signature), Some(parameters)) = (
                self.signatures.get(&owner),
                self.type_parameters.get(&owner),
            ) else {
                return Ok(None);
            };
            (signature, parameters)
        };
        let domains = self.factory_input_domains(parameters, fuel)?;
        let frames = [nir::function_parameters::ordinary_frame(signature, fuel)
            .map_err(|error| error.to_string())?];
        nir::DeclarationSubstitution::bounded_domains(
            parameters,
            &domains,
            indices,
            nir::DeclarationInputScope::ClosedInBody(&frames),
            fuel,
        )
        .and_then(|substitution| substitution.into_owned(fuel))
        .map(Some)
        .map_err(|error| error.to_string())
    }

    pub(super) fn factory_input_domains(
        &self,
        parameters: &[DeclarationId],
        fuel: &mut usize,
    ) -> Result<Vec<Term>, String> {
        let mut domains = Vec::new();
        for parameter in parameters {
            let domain = self
                .observe_input_domain(*parameter, || {
                    *fuel = fuel
                        .checked_sub(1)
                        .ok_or(nir::type_computations::Error::Budget)?;
                    Ok(())
                })
                .map_err(|e| e.to_string())?;
            domains.push(
                nir::copy_budget::clone_term(domain.unwrap_or(&Term::Primitive(P::Type)), fuel)
                    .map_err(|_| "structural declaration domain budget exhausted".to_string())?,
            );
        }
        Ok(domains)
    }
}
