//! Checked generic function formation shared by reference and bytecode execution.
use super::*;

#[derive(Default)]
pub(super) struct Cache {
    entries: Vec<(DeclarationId, Vec<Term>, Arc<Closure>, u64)>,
    weight: u64,
}
impl Cache {
    fn lookup(
        &self,
        declaration: DeclarationId,
        arguments: &[Term],
        fuel: &mut u64,
    ) -> Result<Option<Arc<Closure>>, ()> {
        for (id, inputs, code, key_work) in &self.entries {
            *fuel = fuel.checked_sub(1).ok_or(())?;
            if *id == declaration {
                *fuel = fuel.checked_sub(*key_work).ok_or(())?;
                if inputs == arguments {
                    return Ok(Some(code.clone()));
                }
            }
        }
        Ok(None)
    }

    fn insert(
        &mut self,
        key: (DeclarationId, Vec<Term>),
        code: Arc<Closure>,
        key_work: u64,
        work: u64,
    ) {
        let weight = work.saturating_add(key_work).saturating_add(1);
        if weight > 100_000 {
            return;
        }
        if self.entries.len() >= 64 || self.weight.saturating_add(weight) > 100_000 {
            self.entries.clear();
            self.weight = 0;
        }
        self.weight += weight;
        self.entries.push((key.0, key.1, code, key_work));
    }
}

impl Interpreter<'_> {
    pub(super) fn function_instance_value(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
        output: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if self.graph.is_some() {
            return self.iir_function_value(declaration, arguments, output, source);
        }
        if let Some(identity) = self.runtime_function_identities.get(&declaration).copied() {
            self.admit_function_inputs(&identity.type_parameters, arguments, source)?;
            let binders = self.input_binders(&identity.type_parameters);
            self.charge_copy_work(Some(source), |fuel| {
                nir::copy_budget::charge_type_substitution(
                    &identity.signature,
                    &binders,
                    arguments,
                    fuel,
                )
            })?;
            let substitution =
                self.function_substitution(&identity.type_parameters, arguments, source)?;
            let signature = substitution
                .instantiate(&identity.signature)
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            if !selected_types::same(&signature, output, &self.associated_values) {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            }
            return Ok(Value::FunctionInstance {
                declaration,
                arguments: arguments.to_vec(),
            });
        }
        let definition = *self
            .functions
            .get(&declaration)
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(source)))?;
        self.admit_function_inputs(&definition.type_parameters, arguments, source)?;
        let substitution =
            self.function_substitution(&definition.type_parameters, arguments, source)?;
        let before_key = self.budget.steps;
        self.charge_copy_work(Some(source), |fuel| {
            for argument in arguments {
                nir::copy_budget::charge_type_substitution(argument, &[], &[], fuel)?;
            }
            Ok(())
        })?;
        let key_work = before_key - self.budget.steps;
        let key = (declaration, arguments.to_vec());
        let persistent = self.abstract_type_parameters.is_empty()
            && self.abstract_value_parameters.is_empty()
            && arguments
                .iter()
                .all(|argument| argument.check_scope(&[]).is_ok());
        let shared = if persistent && !self.instances.contains_key(&key) {
            self.prepared_instance_cache
                .lookup(declaration, arguments, &mut self.budget.steps)
                .map_err(|()| self.failure(ErrorKind::StepsExhausted, Some(source)))?
        } else {
            None
        };
        let before_preparation = self.budget.steps;
        let binders = self.input_binders(&definition.type_parameters);
        if !self.instances.contains_key(&key) && shared.is_none() {
            self.charge_copy_work(Some(source), |fuel| {
                nir::copy_budget::charge_function_substitution(
                    definition, &binders, arguments, fuel,
                )
            })?;
        }
        if self.comptime_only {
            let definition = *self
                .functions
                .get(&declaration)
                .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(source)))?;
            for requirement in &definition.requirements {
                // Cached bodies do not suppress current invocation obligations.
                let requirement =
                    self.specialize_function_predicate(definition, arguments, requirement, source)?;
                if let Some(requirement) =
                    self.bind_invocation_contract(&requirement, None, source, 0)?
                {
                    self.check_type_contract(&requirement, source)?;
                }
            }
        }
        for capability in &definition.capabilities {
            let nir::program::TypePredicate::Trait(bound) = self.specialize_function_predicate(
                definition,
                arguments,
                &nir::program::TypePredicate::Trait(capability.clone()),
                source,
            )?
            else {
                unreachable!("trait substitution preserves predicate kind");
            };
            let open = bound.subject.check_scope(&[]).is_err()
                || bound
                    .interface_arguments
                    .iter()
                    .any(|ty| ty.check_scope(&[]).is_err());
            if self.comptime_only && open {
                continue;
            }
            if !self.capabilities_hold_parameters(std::slice::from_ref(&bound), &[], &[]) {
                return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
            }
        }
        let instance = if let Some(instance) = self.instances.get(&key).cloned().or(shared) {
            instance
        } else {
            let definition = *self
                .functions
                .get(&declaration)
                .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(source)))?;
            let signature = substitution
                .instantiate(&definition.signature)
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            let mut body = definition.body.clone();
            // Body types live under the function's parameter telescope;
            // a declaration substitution must retain that frame.
            let Term::Function { parameters, .. } = signature.callable_signature() else {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            };
            let frames = vec![parameters.iter().map(|p| p.sort).collect()];
            let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let rewritten = self
                .function_substitution_bounded(
                    &definition.type_parameters,
                    arguments,
                    &frames,
                    &mut fuel,
                )
                .and_then(|substitution| {
                    body.rewrite_types(&mut |ty| substitution.instantiate(ty))
                        .map_err(nir::type_computations::Error::Scope)
                });
            self.budget.steps -= (before - fuel) as u64;
            rewritten.map_err(|failure| {
                self.failure(
                    if matches!(failure, nir::type_computations::Error::Budget) {
                        ErrorKind::StepsExhausted
                    } else {
                        ErrorKind::Type
                    },
                    Some(source),
                )
            })?;
            let instance = Arc::new(Closure {
                definition_source: None,
                origin: Some(declaration),
                instance_arguments: Some(arguments.to_vec()),
                parameters: definition.parameters.clone(),
                environment: Environment::new(),
                capture_order: vec![],
                body: Arc::new(body),
                ty: signature
                    .with_comptime_phase(definition.phase == nir::program::FunctionPhase::Comptime)
                    .with_unsafe_call_requirement(definition.is_unsafe),
            });
            self.events.push(Event::InstantiateFunction {
                declaration,
                arguments: arguments.to_vec(),
            });
            if persistent && self.budget.steps >= key_work {
                self.budget.steps -= key_work;
                self.prepared_instance_cache.insert(
                    key.clone(),
                    instance.clone(),
                    key_work,
                    before_preparation - self.budget.steps,
                );
            }
            self.instances.insert(key, instance.clone());
            instance
        };
        if !selected_types::same(&instance.ty, output, &self.associated_values) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(Value::Closure(instance))
    }
}
