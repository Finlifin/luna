//! Source declarations retained by a required computation. Input data and
//! checked expressions share a worklist; only the VM selects an implementation.
use super::*;

pub(super) struct SourceClosure {
    pub(super) functions: Vec<n::Function>,
    pub(super) runtime_function_identities: Vec<n::RuntimeFunctionIdentity>,
    pub(super) retained: BTreeSet<Dependency>,
}

impl Checker<'_, '_> {
    pub(super) fn required_method_link(
        &mut self,
        id: DeclarationId,
        span: Span,
    ) -> Result<n::MethodLink, Error> {
        let (source, phase, is_quote, parameters) =
            if let Some(function) = self.generated_functions.declaration(id) {
                (
                    function.source,
                    function.phase,
                    false,
                    &function.type_parameters,
                )
            } else {
                let owner = self
                    .source_declarations
                    .owner(id)
                    .ok_or_else(|| error(span, "implementation method declaration is missing"))?;
                let item = self
                    .package
                    .item(owner)
                    .expect("indexed source declaration");
                let ItemKind::Fn(signature, _) = &item.kind else {
                    return Err(error(span, "implementation method is not a function"));
                };
                (
                    source(item.span),
                    function_bodies::phase(&signature.modifiers),
                    signature.modifiers.is_quote,
                    &self.type_parameters[&owner],
                )
            };
        self.required_steps = self
            .required_steps
            .checked_sub(1 + parameters.len() as u64)
            .ok_or_else(|| error(span, "implementation method link budget exhausted"))?;
        Ok(n::MethodLink {
            source,
            declaration: id,
            phase,
            is_quote,
            type_parameters: parameters.clone(),
        })
    }

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
                Dependency::Associated(member) => {
                    // A generic comptime body may discover a member dependency
                    // that the enclosing initializer has not demanded yet.
                    let implementations: Vec<_> = self
                        .associated_initializers
                        .keys()
                        .filter(|(_, id)| *id == member)
                        .map(|(implementation, _)| *implementation)
                        .collect();
                    for implementation in implementations {
                        self.selected_associated_value(implementation, member, span)?;
                    }
                    for implementation in &self.checked_implementations {
                        for (_, value) in implementation
                            .associated
                            .iter()
                            .filter(|(id, _)| *id == member)
                        {
                            let found = Discovery::Required(&self.implementation_index)
                                .references(value, &mut self.required_steps)
                                .map_err(|failure| error(span, failure.to_string()))?;
                            work.extend(found, &mut self.required_steps, span)?;
                        }
                        for (_, initializer) in implementation
                            .associated_initializers
                            .iter()
                            .filter(|(id, _)| *id == member)
                        {
                            let found = Discovery::Required(&self.implementation_index)
                                .references(initializer.expression(), &mut self.required_steps)
                                .map_err(|failure| error(span, failure.to_string()))?;
                            work.extend(found, &mut self.required_steps, span)?;
                        }
                    }
                    continue;
                }
                Dependency::Method(member) => {
                    // A bare callable declaration carries no receiver evidence.
                    let (interface, _) = self.checked_traits.method(member).ok_or_else(|| {
                        error(span, "required comptime method contract is missing")
                    })?;
                    work.extend(
                        [Dependency::MethodContract(member)],
                        &mut self.required_steps,
                        span,
                    )?;
                    work.extend(
                        self.checked_implementations
                            .iter()
                            .filter(|i| i.interface == Some(interface.declaration))
                            .map(|i| Dependency::MethodBody {
                                implementation: i.declaration,
                                member,
                            }),
                        &mut self.required_steps,
                        span,
                    )?;
                    continue;
                }
                Dependency::MethodContract(member) => {
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
                        let found = Discovery::Required(&self.implementation_index)
                            .references(expression, &mut self.required_steps)
                            .map_err(|failure| error(span, failure.to_string()))?;
                        work.extend(found, &mut self.required_steps, span)?;
                    }
                    continue;
                }
                Dependency::MethodBody {
                    implementation,
                    member,
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
                    let edge = if self.checked_traits.method(id).is_some() {
                        Dependency::Method(id)
                    } else {
                        Dependency::Definition(id)
                    };
                    work.extend([edge], &mut self.required_steps, span)?;
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
                    let found = Discovery::Required(&self.implementation_index)
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
            let found = Discovery::Required(&self.implementation_index)
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
