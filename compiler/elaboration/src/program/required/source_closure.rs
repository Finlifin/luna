//! Source declarations retained by a required computation. Input data and
//! checked expressions share a worklist; only the VM selects an implementation.
use super::*;

pub(super) struct SourceClosure {
    pub(super) functions: Vec<n::Function>,
    pub(super) runtime_function_identities: Vec<n::RuntimeFunctionIdentity>,
    pub(super) retained: BTreeSet<Dependency>,
}

impl Checker<'_, '_> {
    pub(super) fn required_source_closure(
        &mut self,
        roots: BTreeSet<Dependency>,
        span: Span,
    ) -> Result<SourceClosure, Error> {
        let mut functions = Vec::new();
        let mut runtime_function_identities = Vec::new();
        let mut work = dependencies::Worklist::default();
        work.extend(roots, &mut self.required_steps, span)?;
        while let Some(need) = work.next() {
            let id = match need {
                // The complete associated contract is already available to the
                // type theory. Only implementation edges demand its value.
                Dependency::InterfaceMember(Member::Associated(_)) => continue,
                Dependency::ImplementationMember {
                    implementation,
                    member: Member::Associated(member),
                } => {
                    if self
                        .associated_initializers
                        .contains_key(&(implementation, member))
                    {
                        self.ensure_associated_value(implementation, member, span)?;
                    }
                    if let Some(&slot) = self.implementation_positions.get(&implementation) {
                        let row = &self.checked_implementations[slot];
                        if let Some(source) = interfaces::associated_source(
                            row,
                            member,
                            &mut self.required_steps,
                            span,
                        )? {
                            let found = Discovery::Required(
                                &self.implementation_index,
                                &self.member_sources,
                            )
                            .references(source.expression(), &mut self.required_steps)
                            .map_err(|failure| error(span, failure.to_string()))?;
                            work.extend(found, &mut self.required_steps, span)?;
                        }
                    }
                    continue;
                }
                Dependency::InterfaceMember(Member::Method(member)) => {
                    if !self.interfaces_ready {
                        return Err(error(
                            span,
                            "required comptime evaluation is waiting for the trait implementation world",
                        ));
                    }
                    let (_, contract) = self.checked_traits.method(member).ok_or_else(|| {
                        error(span, "required comptime method contract is missing")
                    })?;
                    if !contract.signature.is_comptime_callable()
                        && !contract.program.preconditions.is_empty()
                        && !matches!(contract.signature.callable_signature(), Ok(Term::Function { parameters, .. })
                            if parameters.iter().any(|p| p.sort != Sort::Runtime))
                    {
                        return Err(error(
                            span,
                            "trait method value would erase program preconditions; callable program contracts are not implemented",
                        ));
                    }
                    for expression in contract
                        .program
                        .preconditions
                        .iter()
                        .chain(&contract.program.postconditions)
                    {
                        let found =
                            Discovery::Required(&self.implementation_index, &self.member_sources)
                                .references(expression, &mut self.required_steps)
                                .map_err(|failure| error(span, failure.to_string()))?;
                        work.extend(found, &mut self.required_steps, span)?;
                    }
                    continue;
                }
                Dependency::ImplementationMember {
                    implementation,
                    member: Member::Method(member),
                } => {
                    let (interface, contract) =
                        self.checked_traits.method(member).ok_or_else(|| {
                            error(span, "required comptime method contract is missing")
                        })?;
                    let selected = self
                        .implementation_positions
                        .get(&implementation)
                        .map(|&slot| &self.checked_implementations[slot])
                        .filter(|i| i.interface == Some(interface.declaration));
                    if let Some((_, body)) = selected
                        .and_then(|i| i.methods.iter().find(|(name, _)| *name == contract.name))
                    {
                        work.extend(
                            [Dependency::Definition(*body)],
                            &mut self.required_steps,
                            span,
                        )?;
                    }
                    continue;
                }
                Dependency::Callable(id) => {
                    // A method value references the contract. Implementations
                    // may inherit a default whose code has that same ID.
                    if let Some((interface, _)) = self.checked_traits.method(id) {
                        let member = Member::Method(id);
                        work.extend(
                            std::iter::once(Dependency::InterfaceMember(member)).chain(
                                self.checked_implementations
                                    .iter()
                                    .filter(|i| i.interface == Some(interface.declaration))
                                    .map(|i| Dependency::ImplementationMember {
                                        implementation: i.declaration,
                                        member,
                                    }),
                            ),
                            &mut self.required_steps,
                            span,
                        )?;
                    } else {
                        work.extend([Dependency::Definition(id)], &mut self.required_steps, span)?;
                    }
                    continue;
                }
                Dependency::Definition(id) => id,
            };
            if let Some(interface) = self.generated_functions.declaration(id) {
                if interface.phase == n::FunctionPhase::Runtime {
                    if !interface.preconditions.is_empty()
                        && !has_staged_inputs(&interface.signature)
                    {
                        return Err(error(span, ERASED_PRECONDITIONS));
                    }
                    runtime_function_identities.push(function_bodies::retain_runtime_identity(
                        interface,
                        &mut self.required_steps,
                        span,
                    )?);
                } else {
                    let function = self.generated_functions.body(id).ok_or_else(|| {
                        error(span, "required comptime code is still being checked")
                    })?;
                    let found =
                        Discovery::Required(&self.implementation_index, &self.member_sources)
                            .function_references(function, &mut self.required_steps)
                            .map_err(|failure| error(span, failure.to_string()))?;
                    work.extend(found, &mut self.required_steps, span)?;
                    functions.push(
                        nir::copy_budget::clone_function(function, &mut self.required_steps)
                            .map_err(|_| {
                                error(span, "required function snapshot budget exhausted")
                            })?,
                    );
                }
                continue;
            }
            let owner = self
                .source_declarations
                .owner(id)
                .ok_or_else(|| error(span, "required comptime function declaration is missing"))?;
            let modifiers = self.package.item(owner).and_then(|item| match &item.kind {
                ItemKind::Fn(signature, _) => Some(&signature.modifiers),
                _ => None,
            });
            if modifiers.is_some_and(|flags| !flags.is_comptime && !flags.is_normal_form_head) {
                self.ensure_signature(owner, span)?;
                self.ensure_contract(owner, span)?;
                let signature = self
                    .declared_signature(owner)
                    .map(|sig| sig.into_owned())
                    .ok_or_else(|| {
                        error(span, "runtime function identity requires a known signature")
                    })?;
                let preconditions = self
                    .program_requirements
                    .get(&owner)
                    .is_some_and(|requirements| !requirements.is_empty());
                if preconditions && !has_staged_inputs(&signature) {
                    return Err(error(span, ERASED_PRECONDITIONS));
                }
                runtime_function_identities.push(self.runtime_function_identity(owner, signature));
                continue;
            }
            if modifiers.is_some_and(|flags| flags.is_normal_form_head)
                || self.builtin_normal_form_constant(owner, span)?.is_some()
            {
                self.ensure_signature(owner, span)?;
                self.ensure_contract(owner, span)?;
                continue;
            }
            let function = self.required_function(owner, span)?;
            let found = Discovery::Required(&self.implementation_index, &self.member_sources)
                .function_references(&function, &mut self.required_steps)
                .map_err(|failure| error(span, failure.to_string()))?;
            work.extend(found, &mut self.required_steps, span)?;
            functions.push(function);
        }
        Ok(SourceClosure {
            functions,
            runtime_function_identities,
            retained: work.into_discovered(),
        })
    }
}

const ERASED_PRECONDITIONS: &str = "function value would erase program preconditions; callable program contracts are not implemented";

fn has_staged_inputs(signature: &Term) -> bool {
    matches!(signature, Term::Function { parameters, .. } if parameters.iter().any(|p| p.sort != Sort::Runtime))
}
