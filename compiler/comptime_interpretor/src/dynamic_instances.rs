//! Clause computations use the ordinary function-instance cache and contracts.
use super::*;

pub(super) enum PreparedTraitInstance {
    Unrelated,
    Ready(Expr),
    Control(Flow),
}

impl Interpreter<'_> {
    /// Resolve representation evidence without invoking a callable. Formation
    /// predicates remain part of admission; signature lookup cannot prove them.
    pub(super) fn normal_callable_signature(
        &self,
        declaration: DeclarationId,
        arguments: &[Term],
        frames: &[Vec<nir::Sort>],
        fuel: &mut usize,
    ) -> Option<Term> {
        self.normal_callable_signature_view(declaration, arguments, frames, fuel)
            .map(std::borrow::Cow::into_owned)
    }

    pub(super) fn normal_callable_signature_view(
        &self,
        declaration: DeclarationId,
        arguments: &[Term],
        frames: &[Vec<nir::Sort>],
        fuel: &mut usize,
    ) -> Option<std::borrow::Cow<'_, Term>> {
        *fuel = fuel.checked_sub(1)?;
        if let Some(graph) = self.graph {
            // Hash only bounded, complete instance inputs. The immutable graph
            // owns the borrowed keys and signatures for this VM's lifetime.
            for argument in arguments {
                nir::type_computations::charge_term(argument, fuel).ok()?;
            }
            let index = self
                .graph_indexes
                .as_ref()?
                .callables
                .get(&declaration)?
                .get(arguments)?;
            let signature = graph.callable_identities[*index].closed_signature().ok()?;
            nir::type_computations::charge_term(signature, fuel).ok()?;
            return Some(std::borrow::Cow::Borrowed(signature));
        }
        if let Some(head) = self.normal_forms.get(&declaration) {
            if !arguments.is_empty() {
                return None;
            }
            *fuel = fuel.checked_sub(head.parameters.len())?;
            for parameter in &head.parameters {
                nir::type_computations::charge_term(&parameter.ty, fuel).ok()?;
            }
            return head.callable_type().ok().map(std::borrow::Cow::Owned);
        }
        let (parameters, signature, qualifiers) =
            if let Some(identity) = self.runtime_function_identities.get(&declaration) {
                (&identity.type_parameters, &identity.signature, None)
            } else {
                let function = self.functions.get(&declaration)?;
                (
                    &function.type_parameters,
                    &function.signature,
                    Some((
                        function.phase == nir::program::FunctionPhase::Comptime,
                        function.is_unsafe,
                    )),
                )
            };
        self.validate_function_inputs_in_scope(parameters, arguments, frames, fuel)
            .ok()?;
        let signature = self
            .function_substitution_bounded(parameters, arguments, frames, fuel)
            .ok()?
            .instantiate_bounded(signature, fuel)
            .ok()?;
        Some(std::borrow::Cow::Owned(match qualifiers {
            Some((comptime, is_unsafe)) => signature
                .with_comptime_phase(comptime)
                .with_unsafe_call_requirement(is_unsafe),
            None => signature,
        }))
    }

    pub(super) fn admit_callable_identity(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
        source: SourceRange,
    ) -> Result<(), Error> {
        if self.normal_forms.contains_key(&declaration) {
            return if arguments.is_empty() {
                Ok(())
            } else {
                Err(self.failure(ErrorKind::Type, Some(source)))
            };
        }
        if let Some(identity) = self.runtime_function_identities.get(&declaration) {
            // Transport-only metadata is published after producer-side
            // formation checks. It cannot authorize a runtime invocation.
            return self.admit_function_inputs(&identity.type_parameters, arguments, source);
        }
        let definition = self
            .functions
            .get(&declaration)
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(source)))?;
        self.admit_function_inputs(&definition.type_parameters, arguments, source)?;
        for requirement in &definition.requirements {
            let requirement =
                self.specialize_function_predicate(definition, arguments, requirement, source)?;
            if let Some(requirement) =
                self.bind_invocation_contract(&requirement, None, source, 0)?
            {
                self.check_type_contract(&requirement, source)?;
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
        Ok(())
    }

    pub(super) fn function_substitution_bounded<'inputs>(
        &self,
        parameters: &[DeclarationId],
        arguments: &'inputs [Term],
        frames: &'inputs [Vec<nir::Sort>],
        fuel: &mut usize,
    ) -> Result<nir::DeclarationSubstitution<'inputs>, nir::type_computations::Error> {
        *fuel = fuel
            .checked_sub(parameters.len())
            .ok_or(nir::type_computations::Error::Budget)?;
        let mut domains = Vec::with_capacity(parameters.len());
        for parameter in parameters {
            let domain = self
                .declaration_parameter_domains
                .get(parameter)
                .unwrap_or(&Term::Primitive(Primitive::Type));
            nir::type_computations::charge_term(domain, fuel)?;
            domains.push(domain.clone());
        }
        nir::DeclarationSubstitution::bounded(parameters, &domains, arguments, frames, fuel)
    }

    pub(super) fn function_substitution<'inputs>(
        &mut self,
        parameters: &[DeclarationId],
        arguments: &'inputs [Term],
        source: SourceRange,
    ) -> Result<nir::DeclarationSubstitution<'inputs>, Error> {
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = self.function_substitution_bounded(parameters, arguments, &[], &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        result.map_err(|failure| {
            self.failure(
                if matches!(failure, nir::type_computations::Error::Budget) {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                },
                Some(source),
            )
        })
    }

    pub(super) fn specialize_function_predicate(
        &mut self,
        definition: &nir::program::Function,
        arguments: &[Term],
        predicate: &nir::program::TypePredicate,
        source: SourceRange,
    ) -> Result<nir::program::TypePredicate, Error> {
        let Term::Function { parameters, .. } = definition.signature.callable_signature() else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        let frames = vec![parameters.iter().map(|p| p.sort).collect()];
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = self
            .function_substitution_bounded(
                &definition.type_parameters,
                arguments,
                &frames,
                &mut fuel,
            )
            .and_then(|substitution| {
                substitution.instantiate_predicate_bounded(predicate, &mut fuel)
            });
        self.budget.steps -= (before - fuel) as u64;
        result.map_err(|error| {
            self.failure(
                if matches!(error, nir::type_computations::Error::Budget) {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::Type
                },
                Some(source),
            )
        })
    }

    pub(super) fn evaluate_instance_default(
        &mut self,
        construction: SourceRange,
        argument: &nir::program::CallArgument,
        callee: &Value,
        defaults: &mut Environment,
        caller: &mut Environment,
    ) -> Result<Flow, Error> {
        let value = self.instance_default_expression(construction, &argument.value, callee)?;
        let prepared = nir::program::CallArgument {
            value,
            passing: argument.passing,
            parameter: argument.parameter,
            is_default: true,
        };
        self.evaluate_default(construction, &prepared, defaults, caller)
    }

    pub(super) fn instance_default_expression(
        &mut self,
        construction: SourceRange,
        expression: &Expr,
        callee: &Value,
    ) -> Result<Expr, Error> {
        let Value::Closure(closure) = callee else {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(construction)));
        };
        let (declaration, arguments) = closure
            .declaration_instance()
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(construction)))?;
        let definition = self
            .functions
            .get(&declaration)
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(construction)))?;
        let binders = self.input_binders(&definition.type_parameters);
        self.charge_copy_work(Some(expression.source), |fuel| {
            nir::copy_budget::charge_substitution(expression, &binders, arguments, fuel)
        })?;
        let substitution =
            self.function_substitution(&definition.type_parameters, arguments, expression.source)?;
        let mut value = expression.clone();
        value
            .rewrite_types(&mut |ty| substitution.instantiate(ty))
            .map_err(|_| self.failure(ErrorKind::Type, Some(expression.source)))?;
        Ok(value)
    }

    pub(super) fn bound_default_expression(
        &mut self,
        expression: &Expr,
        binders: &[nir::DeclarationParameter],
        arguments: &[Term],
    ) -> Result<Expr, Error> {
        self.charge_copy_work(Some(expression.source), |fuel| {
            nir::copy_budget::charge_substitution(expression, binders, arguments, fuel)
        })?;
        expression
            .instantiate_parameters(binders, arguments)
            .map_err(|_| self.failure(ErrorKind::Type, Some(expression.source)))
    }

    pub(super) fn evaluate_bound_default(
        &mut self,
        construction: SourceRange,
        argument: &nir::program::CallArgument,
        binders: &[nir::DeclarationParameter],
        arguments: &[Term],
        defaults: &mut Environment,
        caller: &mut Environment,
    ) -> Result<Flow, Error> {
        let value = self.bound_default_expression(&argument.value, binders, arguments)?;
        let prepared = nir::program::CallArgument {
            passing: argument.passing,
            parameter: argument.parameter,
            value,
            is_default: true,
        };
        self.evaluate_default(construction, &prepared, defaults, caller)
    }

    pub(super) fn trait_default_inputs(
        &mut self,
        method: &Expr,
    ) -> Result<(Vec<nir::DeclarationParameter>, Vec<Term>), Error> {
        let view = comptime_compiler::methods::View::expression(method)
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(method.source)))?;
        self.trait_method_default_inputs(view)
    }
    pub(super) fn trait_method_default_inputs(
        &mut self,
        method: comptime_compiler::methods::View<'_>,
    ) -> Result<(Vec<nir::DeclarationParameter>, Vec<Term>), Error> {
        let comptime_compiler::methods::View {
            interface,
            member,
            subject,
            interface_arguments,
            method_arguments,
            ..
        } = method;
        let (binders, _) = self
            .method_contracts
            .get(&(interface, member))
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(method.source)))?;
        let count = binders.len();
        if count != 1 + interface_arguments.len() + method_arguments.len() {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(method.source)));
        }
        let binders = binders.clone();
        self.integer_operation(Some(method.source), |budget| budget.consume(count))?;
        let arguments = std::iter::once(subject.clone())
            .chain(interface_arguments.iter().cloned())
            .chain(method_arguments.iter().cloned())
            .collect();
        Ok((binders, arguments))
    }

    pub(super) fn admit_dynamic_trait_instance(
        &mut self,
        declaration: DeclarationId,
        input: &Term,
        output: &Term,
        source: SourceRange,
    ) -> Result<Option<(DeclarationId, usize)>, Error> {
        self.integer_operation(Some(source), |budget| budget.consume(1))?;
        let Some(origin) = self.method_origins.get(&declaration) else {
            return Ok(None);
        };
        let interface =
            origin.ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
        let (parameters, contract) = self
            .method_contracts
            .get(&(interface, declaration))
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
        if !self.comptime_only
            || !contract.signature.is_comptime_callable()
            || !output.is_comptime_callable()
        {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        let count = parameters.len();
        let interface_count = count
            .checked_sub(1 + contract.type_parameters.len())
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
        let Term::Tuple(domains) = input else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        if domains.len() != count || count < 1 + interface_count {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(Some((interface, interface_count)))
    }

    pub(super) fn admit_dynamic_function_instance(
        &self,
        declaration: DeclarationId,
        input: &Term,
        output: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        let definition = self
            .functions
            .get(&declaration)
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(source)))?;
        if definition.phase != nir::program::FunctionPhase::Comptime
            || !output.is_comptime_callable()
        {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        let Term::Tuple(domains) = input else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        if domains.len() != definition.type_parameters.len() {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(())
    }

    // The tuple here is the NIR clause packet, not a language tuple-type rule.
    // Current values are checked even when the resulting instance is cached.
    pub(super) fn dynamic_instance_arguments(
        &mut self,
        input: &Term,
        packet: Value,
        source: SourceRange,
    ) -> Result<Vec<Term>, Error> {
        let (Term::Tuple(domains), Value::Tuple(values)) = (input, packet) else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        if values.len() != domains.len() {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        self.integer_operation(Some(source), |budget| budget.consume(values.len()))?;
        let mut arguments = Vec::with_capacity(values.len());
        for (value, domain) in values.into_iter().zip(domains) {
            if !self.has_type(&value, domain) {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            }
            arguments.push(self.residual_input_term(value, source)?);
        }
        Ok(arguments)
    }

    pub(super) fn dynamic_trait_method(
        &mut self,
        member: DeclarationId,
        input: &Term,
        output: &Term,
        origin: (DeclarationId, usize),
        packet: Value,
        source: SourceRange,
    ) -> Result<comptime_compiler::methods::Method, Error> {
        let arguments = self.dynamic_instance_arguments(input, packet, source)?;
        let (interface, count) = origin;
        if arguments.len() < 1 + count {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
        }
        Ok(comptime_compiler::methods::Method {
            interface,
            member,
            subject: arguments[0].clone(),
            interface_arguments: arguments[1..1 + count].to_vec(),
            method_arguments: arguments[1 + count..].to_vec(),
            output: output.clone(),
            source,
        })
    }

    pub(super) fn dynamic_instance_expression(
        &mut self,
        declaration: DeclarationId,
        input: &Term,
        output: &Term,
        origin: Option<(DeclarationId, usize)>,
        packet: Value,
        source: SourceRange,
    ) -> Result<Expr, Error> {
        let arguments = self.dynamic_instance_arguments(input, packet, source)?;
        let kind = if let Some((interface, count)) = origin {
            if arguments.len() < 1 + count {
                return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
            }
            K::TraitMethod {
                interface,
                member: declaration,
                subject: arguments[0].clone(),
                interface_arguments: arguments[1..1 + count].to_vec(),
                method_arguments: arguments[1 + count..].to_vec(),
            }
        } else {
            K::FunctionInstance {
                declaration,
                arguments,
            }
        };
        Ok(Expr {
            source,
            ty: output.clone(),
            kind,
        })
    }

    pub(super) fn prepare_dynamic_trait_instance(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<PreparedTraitInstance, Error> {
        let K::ComptimeFunctionInstance {
            declaration,
            subject,
        } = &expr.kind
        else {
            return Ok(PreparedTraitInstance::Unrelated);
        };
        let Some(origin) =
            self.admit_dynamic_trait_instance(*declaration, &subject.ty, &expr.ty, expr.source)?
        else {
            return Ok(PreparedTraitInstance::Unrelated);
        };
        let packet = match self.eval(subject, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(PreparedTraitInstance::Control(flow)),
        };
        Ok(PreparedTraitInstance::Ready(
            self.dynamic_instance_expression(
                *declaration,
                &subject.ty,
                &expr.ty,
                Some(origin),
                packet,
                expr.source,
            )?,
        ))
    }

    pub(super) fn eval_comptime_function_instance(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        match self.prepare_dynamic_trait_instance(expr, environment)? {
            PreparedTraitInstance::Ready(method) => {
                return self.eval_trait_method(&method, environment, false);
            }
            PreparedTraitInstance::Control(flow) => return Ok(flow),
            PreparedTraitInstance::Unrelated => {}
        }
        let K::ComptimeFunctionInstance {
            declaration,
            subject,
        } = &expr.kind
        else {
            unreachable!()
        };
        self.admit_dynamic_function_instance(*declaration, &subject.ty, &expr.ty, expr.source)?;
        let packet = match self.eval(subject, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        let arguments = self.dynamic_instance_arguments(&subject.ty, packet, expr.source)?;
        // Revalidate declaration domains and requirements, including cache hits.
        self.function_instance_value(*declaration, &arguments, &expr.ty, expr.source)
            .map(Flow::Value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn package() -> Package {
        let id = DeclarationId {
            package: 1,
            local: 1,
        };
        let parameter = DeclarationId {
            package: 1,
            local: 2,
        };
        Package {
            function_parameters: vec![nir::function_parameters::Declaration {
                declaration: id,
                parameters: vec![parameter],
                domains: vec![Term::Primitive(Primitive::Bool)],
            }],
            functions: vec![nir::program::Function {
                declaration: id,
                name: "select".into(),
                phase: nir::program::FunctionPhase::Comptime,
                inline: false,
                pure: false,
                is_unsafe: false,
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(Term::Primitive(Primitive::Bool)),
                },
                parameters: vec![],
                type_parameters: vec![parameter],
                defaults: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                entry_snapshots: vec![],
                capabilities: vec![],
                requirements: vec![],
                body: Expr {
                    source: SourceRange { start: 1, end: 2 },
                    ty: Term::Primitive(Primitive::Bool),
                    kind: K::ComptimeValue(Term::Rigid {
                        declaration: parameter,
                        sort: nir::Sort::ComptimeValue,
                    }),
                },
            }],
            ..Default::default()
        }
    }
    fn expression(package: &Package) -> Expr {
        let source = SourceRange { start: 3, end: 4 };
        let field = Expr {
            source,
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Local(LocalId { owner: 2, index: 0 }),
        };
        Expr {
            source,
            ty: package.functions[0].callable_type(),
            kind: K::ComptimeFunctionInstance {
                declaration: package.functions[0].declaration,
                subject: Box::new(Expr {
                    source,
                    ty: Term::Tuple(vec![field.ty.clone()]),
                    kind: K::Tuple(vec![field]),
                }),
            },
        }
    }
    #[test]
    fn linear_instance_admission_precedes_effects_and_recovers() {
        let package = package();
        let trace = LocalId { owner: 2, index: 1 };
        for compiled in [false, true] {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            for valid in [false, true, false, true] {
                let mut body = expression(&package);
                let source = body.source;
                let K::ComptimeFunctionInstance { subject, .. } = &mut body.kind else {
                    unreachable!()
                };
                let packet = subject.as_ref().clone();
                subject.kind = K::Block {
                    statements: vec![Expr {
                        source,
                        ty: Term::Primitive(Primitive::Void),
                        kind: K::Assign {
                            place: nir::program::Place {
                                root: trace,
                                fields: vec![],
                            },
                            value: Box::new(Expr {
                                source,
                                ty: Term::Primitive(Primitive::Bool),
                                kind: K::Literal(Literal::Bool(true)),
                            }),
                        },
                    }],
                    result: Some(Box::new(packet)),
                };
                if !valid {
                    subject.ty = Term::Primitive(Primitive::Bool);
                }
                let mut env = Environment::from([
                    (LocalId { owner: 2, index: 0 }, Value::Bool(true)),
                    (trace, Value::Bool(false)),
                ]);
                let result = if compiled {
                    let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                    let dump = code.to_string();
                    assert!(dump.contains("(begin-instance "));
                    assert!(dump.contains("(finish-instance "));
                    vm.execute_code(&code, &mut env)
                } else {
                    vm.eval(&body, &mut env)
                };
                if valid {
                    assert!(matches!(result.unwrap(), Flow::Value(Value::Closure(_))));
                } else {
                    assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
                }
                assert_eq!(
                    vm.local_value(&env, trace, Some(source)).unwrap(),
                    Value::Bool(valid)
                );
            }
        }
    }

    #[test]
    fn failed_input_never_populates_instance_cache_and_can_recover() {
        let package = package();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut env = Environment::new();
        env.insert(LocalId { owner: 2, index: 0 }, Value::Char('x'));
        assert_eq!(
            vm.eval(&expression(&package), &mut env).unwrap_err().kind,
            ErrorKind::Type
        );
        assert!(vm.instances.is_empty());
        env.insert(LocalId { owner: 2, index: 0 }, Value::Bool(true));
        assert!(matches!(
            vm.eval(&expression(&package), &mut env).unwrap(),
            Flow::Value(Value::Closure(_))
        ));
        assert_eq!(vm.instances.len(), 1);
        vm.comptime_only = false;
        assert_eq!(
            vm.eval(&expression(&package), &mut env).unwrap_err().kind,
            ErrorKind::PhaseMismatch
        );
    }
    #[test]
    fn cached_instance_does_not_skip_current_contract_outcome() {
        let mut package = package();
        package.functions[0].requirements = vec![nir::program::TypePredicate::Equal(
            Term::Primitive(Primitive::Bool),
            Term::Primitive(Primitive::Char),
        )];
        // Cache reuse must not turn a former proof into a current proof.
        for compiled in [false, true] {
            for dynamic in [false, true] {
                let outcome = std::cell::Cell::new(ContractOutcome::Satisfied);
                let solver = |_: &nir::program::TypePredicate| outcome.get();
                let mut vm = Interpreter::for_comptime(&package, Budget::default())
                    .unwrap()
                    .with_type_solver(&solver);
                let mut body = expression(&package);
                if !dynamic {
                    body.kind = K::FunctionInstance {
                        declaration: package.functions[0].declaration,
                        arguments: vec![Term::Bool(true)],
                    };
                }
                let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
                let mut env =
                    Environment::from([(LocalId { owner: 2, index: 0 }, Value::Bool(true))]);
                let execute = |vm: &mut Interpreter<'_>, env: &mut Environment| {
                    if compiled {
                        vm.execute_code(&code, env)
                    } else {
                        vm.eval(&body, env)
                    }
                };
                let Flow::Value(Value::Closure(first)) = execute(&mut vm, &mut env).unwrap() else {
                    panic!("expected formed function instance")
                };
                assert_eq!(vm.instances.len(), 1);
                outcome.set(ContractOutcome::Rejected);
                assert_eq!(
                    execute(&mut vm, &mut env).unwrap_err().kind,
                    ErrorKind::TypeContractRejected
                );
                assert_eq!(vm.instances.len(), 1);
                outcome.set(ContractOutcome::Satisfied);
                let Flow::Value(Value::Closure(recovered)) = execute(&mut vm, &mut env).unwrap()
                else {
                    panic!("expected recovered function instance")
                };
                assert!(Arc::ptr_eq(&first, &recovered));
                assert_eq!(vm.instances.len(), 1);
            }
        }
    }
}

#[cfg(test)]
mod retained_signature_tests {
    use super::*;

    fn signature() -> Term {
        Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        }
    }
    fn argument(index: u32) -> Term {
        Term::Integer(comptime_runtime::Integer::from_u128(index as u128))
    }
    fn graph(count: u32) -> iir::Package {
        let mut interner = nir::TypeInterner::default();
        iir::Package {
            callable_identities: (0..count)
                .map(|index| {
                    iir::CallableIdentity::intern(
                        iir::InstanceKey {
                            declaration: DeclarationId {
                                package: 17,
                                local: 1,
                            },
                            arguments: (vec![argument(index)]).into(),
                        },
                        signature(),
                        &mut interner,
                        &mut 100_000,
                    )
                    .unwrap()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn retained_signature_views_borrow_the_interned_core() {
        let graph = graph(2);
        let vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        let identity = &graph.callable_identities[1];
        let view = vm
            .normal_callable_signature_view(
                identity.key.declaration,
                &identity.key.arguments,
                &[],
                &mut 100,
            )
            .unwrap();
        assert!(matches!(view, std::borrow::Cow::Borrowed(_)));
        assert!(std::ptr::eq(
            view.as_ref(),
            identity.signature.core().body()
        ));
        assert!(
            vm.normal_callable_signature_view(
                identity.key.declaration,
                &identity.key.arguments,
                &[],
                &mut 1,
            )
            .is_none()
        );
        assert!(
            vm.normal_callable_signature_view(
                identity.key.declaration,
                &identity.key.arguments,
                &[],
                &mut 100,
            )
            .is_some()
        );
    }

    #[test]
    fn retained_signature_lookup_depends_on_key_size_not_catalog_size() {
        let declaration = DeclarationId {
            package: 17,
            local: 1,
        };
        let mut baseline = None;
        for count in [2, 102, 2002] {
            let graph = graph(count);
            let vm = Interpreter::from_iir(
                &graph,
                Budget {
                    steps: 1_000_000,
                    depth: 128,
                },
            )
            .unwrap();
            let mut fuel = 100;
            assert_eq!(
                vm.normal_callable_signature(declaration, &[argument(1)], &[], &mut fuel),
                Some(signature())
            );
            let used = 100 - fuel;
            if let Some(baseline) = baseline {
                assert_eq!(used, baseline);
            } else {
                baseline = Some(used);
            }
            // Different instances of the same declaration must not alias.
            assert!(
                vm.normal_callable_signature(declaration, &[argument(count)], &[], &mut 100)
                    .is_none()
            );
            assert!(
                vm.normal_callable_signature(declaration, &[], &[], &mut 100)
                    .is_none()
            );
            assert!(
                vm.normal_callable_signature(declaration, &[argument(1)], &[], &mut 1)
                    .is_none()
            );
            assert!(
                vm.normal_callable_signature(declaration, &[argument(1)], &[], &mut 100)
                    .is_some()
            );
            assert!(vm.events().is_empty());
        }
    }

    #[test]
    fn retained_signature_index_is_world_local_and_bounds_query_hash_work() {
        let first = graph(2);
        let mut second = graph(2);
        second.callable_identities[1] = iir::CallableIdentity::intern(
            second.callable_identities[1].key.clone(),
            signature().with_unsafe_call_requirement(true),
            &mut nir::TypeInterner::default(),
            &mut 100_000,
        )
        .unwrap();
        let a = Interpreter::from_iir(&first, Budget::default()).unwrap();
        let b = Interpreter::from_iir(&second, Budget::default()).unwrap();
        let declaration = first.callable_identities[1].key.declaration;
        assert!(
            !a.normal_callable_signature(declaration, &[argument(1)], &[], &mut 100)
                .unwrap()
                .is_unsafe_callable()
        );
        assert!(
            b.normal_callable_signature(declaration, &[argument(1)], &[], &mut 100)
                .unwrap()
                .is_unsafe_callable()
        );
        assert!(
            a.normal_callable_signature(
                declaration,
                &[Term::String("x".repeat(4096).into())],
                &[],
                &mut 100
            )
            .is_none()
        );
        assert!(
            a.normal_callable_signature(declaration, &[argument(1)], &[], &mut 100)
                .is_some()
        );
    }
}
