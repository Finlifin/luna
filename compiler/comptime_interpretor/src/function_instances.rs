//! Checked generic function formation shared by reference and bytecode execution.
use super::*;

impl Interpreter<'_> {
    pub(super) fn function_instance_value(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
        output: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
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
        let key = (declaration, arguments.to_vec());
        let binders = self.input_binders(&definition.type_parameters);
        if !self.instances.contains_key(&key) {
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
        let instance = if let Some(instance) = self.instances.get(&key) {
            instance.clone()
        } else {
            let definition = *self
                .functions
                .get(&declaration)
                .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(source)))?;
            for capability in &definition.capabilities {
                let nir::program::TypePredicate::Trait(bound) = self
                    .specialize_function_predicate(
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
            self.instances.insert(key, instance.clone());
            instance
        };
        if !selected_types::same(&instance.ty, output, &self.associated_values) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(Value::Closure(instance))
    }
}
