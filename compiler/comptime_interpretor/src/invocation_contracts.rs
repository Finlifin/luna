//! Ordinary input binding for type contracts, separate from program predicates.
use super::*;
use nir::program::TypePredicate as P;

impl Interpreter<'_> {
    /// Observe an already instantiated requirement using this VM's evidence.
    /// A passive lookup cannot run a source query or initialize a Type member.
    pub(super) fn retains_capability(
        &self,
        requirement: &nir::program::TraitRequirement,
        fuel: &mut usize,
    ) -> Result<bool, ErrorKind> {
        for term in std::iter::once(&requirement.subject).chain(&requirement.interface_arguments) {
            nir::type_computations::charge_term(term, fuel)
                .map_err(|_| ErrorKind::StepsExhausted)?;
        }
        if self.assumed_interfaces.contains(&(
            requirement.interface_instance(),
            requirement.subject.clone(),
        )) {
            return Ok(true);
        }
        let subject = selected_types::view(&requirement.subject, &self.associated_values, fuel)?;
        let arguments = requirement
            .interface_arguments
            .iter()
            .map(|value| selected_types::view(value, &self.associated_values, fuel))
            .collect::<Result<Vec<_>, _>>()?;
        let key = (
            InterfaceInstance {
                declaration: requirement.interface,
                arguments,
            },
            subject,
        );
        Ok(
            self.implemented_interfaces.contains_key(&key)
                || self.assumed_interfaces.contains(&key),
        )
    }

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
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let retained = self.retains_capability(requirement, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        match retained {
            Ok(true) => return Ok(()),
            // A missing passive selection still permits the ordinary bounded
            // proof query. Resource failure must not start a second attempt.
            Ok(false) | Err(ErrorKind::UnmaterializedDispatch) => {}
            Err(kind) => return Err(self.failure(kind, Some(source))),
        }
        self.check_type_contract(&P::Trait(requirement.clone()), source)
    }

    #[inline(never)]
    pub(super) fn check_invocation_contracts(
        &mut self,
        definition: &nir::program::FunctionDeclaration,
        inputs: &nir::DeclarationSubstitution<'_>,
        scope: Option<&nir::FunctionInputBinding>,
        source: SourceRange,
    ) -> Result<(), Error> {
        for requirement in &definition.requirements {
            let predicate = self.query_admission(Some(source), |_, fuel| {
                Ok(inputs.instantiate_predicate_bounded(requirement, fuel)?)
            })?;
            let bound = self
                .bind_invocation_contract(&predicate, scope, source, 0)?
                .ok_or_else(|| self.failure(ErrorKind::TypeContractUnresolved, Some(source)))?;
            self.check_type_contract(&bound, source)?;
        }
        for capability in &definition.capabilities {
            let predicate = P::Trait(self.query_admission(Some(source), |_, fuel| {
                Ok(inputs.instantiate_requirement_bounded(capability, fuel)?)
            })?);
            match self.bind_invocation_contract(&predicate, None, source, 0)? {
                Some(P::Trait(bound)) if self.comptime_only && self.graph.is_none() => {
                    // Direct source entry has no prior formation receipt, and
                    // shared code grants no evidence in this invocation scope.
                    self.check_capability_contract(&bound, source)?;
                }
                Some(_) => {
                    // Concrete IIR carries producer-side formation evidence;
                    // runtime source admission checks its closed formation.
                }
                None => {
                    let bound = self
                        .bind_invocation_contract(&predicate, scope, source, 0)?
                        .ok_or_else(|| {
                            self.failure(ErrorKind::TypeContractUnresolved, Some(source))
                        })?;
                    self.check_type_contract(&bound, source)?;
                }
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
