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
                            let found = references(value, &mut self.required_steps)
                                .map_err(|failure| error(span, failure.to_string()))?;
                            work.extend(found, &mut self.required_steps, span)?;
                        }
                        for (_, initializer) in implementation
                            .associated_initializers
                            .iter()
                            .filter(|(id, _)| *id == member)
                        {
                            let found =
                                references(initializer.expression(), &mut self.required_steps)
                                    .map_err(|failure| error(span, failure.to_string()))?;
                            work.extend(found, &mut self.required_steps, span)?;
                        }
                    }
                    continue;
                }
                Dependency::Method(member) => {
                    if !self.interfaces_ready {
                        return Err(error(
                            span,
                            "required comptime evaluation is waiting for the trait implementation world",
                        ));
                    }
                    let (interface, contract) =
                        self.checked_traits.method(member).ok_or_else(|| {
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
                    // Checked implementation tables already contain the default
                    // body when it is inherited. An overridden default is not an
                    // execution dependency and may depend on this very query.
                    for implementation in &self.checked_implementations {
                        if implementation.interface == Some(interface.declaration) {
                            work.extend(
                                implementation
                                    .methods
                                    .iter()
                                    .filter(|(name, _)| *name == contract.name)
                                    .map(|(_, body)| Dependency::Definition(*body)),
                                &mut self.required_steps,
                                span,
                            )?;
                        }
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
            if let Some(function) = self.generated_functions.body(id) {
                if function.phase == n::FunctionPhase::Runtime {
                    if !function.preconditions.is_empty() && !has_staged_inputs(&function.signature)
                    {
                        return Err(error(span, ERASED_PRECONDITIONS));
                    }
                    runtime_function_identities.push(n::RuntimeFunctionIdentity {
                        source: function.source,
                        requirements: function.requirements.clone(),
                        capabilities: function.capabilities.clone(),
                        declaration: id,
                        type_parameters: function.interface.type_parameters.clone(),
                        signature: function
                            .interface
                            .signature
                            .clone()
                            .with_unsafe_call_requirement(function.interface.is_unsafe),
                    });
                } else {
                    let found = function_references(function, &mut self.required_steps)
                        .map_err(|failure| error(span, failure.to_string()))?;
                    work.extend(found, &mut self.required_steps, span)?;
                    functions.push(function.clone());
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
                let signature = self.known_callable_signature(owner).ok_or_else(|| {
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
            let found = function_references(&function, &mut self.required_steps)
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
