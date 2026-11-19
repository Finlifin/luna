//! Clause computations use the ordinary function-instance cache and contracts.
use super::*;
use std::ops::ControlFlow;

/// A source declaration can form an instance only after its operands finish.
pub(super) enum FunctionTarget {
    Declaration(DeclarationId),
    Concrete(Arc<IirCallable>),
}

pub(super) enum PreparedTraitInstance {
    Unrelated,
    Ready(Expr),
    Control(Flow),
}

pub(super) fn instantiate_code(
    substitution: &nir::DeclarationSubstitution<'_>,
    expression: &Expr,
    fuel: &mut usize,
) -> Result<Expr, ErrorKind> {
    substitution
        .instantiate_expression_bounded(expression, fuel)
        .map_err(|failure| match failure {
            nir::ExpressionInstantiationError::Copy(
                nir::copy_budget::CopyBudgetExhausted::Steps,
            ) => ErrorKind::StepsExhausted,
            nir::ExpressionInstantiationError::Copy(
                nir::copy_budget::CopyBudgetExhausted::Depth,
            ) => ErrorKind::DepthExhausted,
            nir::ExpressionInstantiationError::Scope(_) => ErrorKind::Type,
        })
}

impl Interpreter<'_> {
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
        if self.graph.is_some() {
            let admitted = self.query_admission(Some(source), |vm, fuel| {
                Ok(vm
                    .admitted_function_signature(declaration, arguments, &[], fuel)?
                    .is_some())
            })?;
            return if admitted {
                Ok(())
            } else {
                Err(self.failure(ErrorKind::UnknownFunction, Some(source)))
            };
        }

        let formation = self
            .functions
            .formation(&declaration)
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(source)))?;
        self.check_source_formation(formation, arguments, source)
    }

    pub(super) fn check_source_formation(
        &mut self,
        formation: nir::callable_source::Formation<'_>,
        arguments: &[Term],
        source: SourceRange,
    ) -> Result<(), Error> {
        let frames = [self.ordinary_input_frame(formation.signature, source)?];
        let inputs = self.admit_function_inputs(
            formation.parameters,
            arguments,
            nir::DeclarationInputScope::ClosedInBody(&frames),
            source,
        )?;
        self.check_callable_requirements(formation, &inputs, source)
    }

    pub(super) fn check_callable_requirements(
        &mut self,
        formation: nir::callable_source::Formation<'_>,
        inputs: &nir::AdmittedArguments<'_>,
        source: SourceRange,
    ) -> Result<(), Error> {
        for requirement in formation.requirements {
            let requirement = self.query_admission(Some(source), |_, fuel| {
                Ok(inputs.instantiate_predicate(requirement, fuel)?)
            })?;
            if let Some(requirement) =
                self.bind_invocation_contract(&requirement, None, source, 0)?
            {
                self.check_type_contract(&requirement, source)?;
            }
        }
        for capability in formation.capabilities {
            let bound = self.query_admission(Some(source), |_, fuel| {
                Ok(inputs.instantiate_requirement(capability, fuel)?)
            })?;
            let open = bound.subject.check_scope(&[]).is_err()
                || bound
                    .interface_arguments
                    .iter()
                    .any(|ty| ty.check_scope(&[]).is_err());
            if self.comptime_only && open {
                continue;
            }
            self.check_capability_contract(&bound, source)?;
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
        let domains =
            function_inputs::input_domains(&self.declaration_parameter_domains, parameters, fuel)?;
        nir::DeclarationSubstitution::bounded_domains(
            parameters,
            &domains,
            arguments,
            nir::DeclarationInputScope::ClosedInBody(frames),
            fuel,
        )
    }

    pub(super) fn specialize_function_expression(
        &mut self,
        expression: &Expr,
        parameters: &[DeclarationId],
        arguments: &[Term],
        frames: &[Vec<nir::Sort>],
    ) -> Result<Expr, Error> {
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = self
            .function_substitution_bounded(parameters, arguments, frames, &mut fuel)
            .map_err(|failure| match failure {
                nir::type_computations::Error::Budget => ErrorKind::StepsExhausted,
                _ => ErrorKind::Type,
            })
            .and_then(|substitution| instantiate_code(&substitution, expression, &mut fuel));
        self.budget.steps -= (before - fuel) as u64;
        result.map_err(|kind| self.failure(kind, Some(expression.source)))
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
        let (declaration, arguments) = callee
            .declaration_instance()
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(construction)))?;
        let definition = self
            .functions
            .declaration(&declaration)
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(construction)))?;
        let frame = self.ordinary_input_frame(&definition.signature, construction)?;
        self.specialize_function_expression(
            expression,
            &definition.type_parameters,
            arguments,
            &[frame],
        )
    }

    pub(super) fn ordinary_input_frame(
        &mut self,
        signature: &Term,
        source: SourceRange,
    ) -> Result<Vec<nir::Sort>, Error> {
        self.query_admission(Some(source), |_, fuel| {
            Ok(nir::function_parameters::ordinary_frame(signature, fuel)?)
        })
    }

    pub(super) fn evaluate_bound_default(
        &mut self,
        construction: SourceRange,
        argument: &nir::program::CallArgument,
        inputs: &method_catalog::Inputs<Term>,
        defaults: &mut Environment,
        caller: &mut Environment,
    ) -> Result<Flow, Error> {
        let value = self.specialize_function_expression(
            &argument.value,
            &inputs.parameters,
            &inputs.arguments,
            std::slice::from_ref(&inputs.frame),
        )?;
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
    ) -> Result<method_catalog::Inputs<Term>, Error> {
        let view = comptime_compiler::methods::View::expression(method)
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(method.source)))?;
        self.trait_method_default_inputs(view)
    }
    pub(super) fn trait_method_default_inputs(
        &mut self,
        method: comptime_compiler::methods::View<'_>,
    ) -> Result<method_catalog::Inputs<Term>, Error> {
        let (inputs, _) = self.method_inputs(method)?;
        self.charge_copy_work(Some(method.source), |fuel| {
            for term in &inputs.arguments {
                nir::copy_budget::charge_type_substitution(term, &[], &[], fuel)?;
            }
            Ok(())
        })?;
        Ok(method_catalog::Inputs {
            parameters: inputs.parameters,
            member: inputs.member,
            arguments: inputs.arguments.into_iter().cloned().collect(),
            frame: inputs.frame,
        })
    }

    pub(super) fn admit_dynamic_trait_instance(
        &mut self,
        declaration: DeclarationId,
        arity: usize,
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
        if arity != count {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(Some((interface, interface_count)))
    }

    pub(super) fn admit_dynamic_function_instance(
        &mut self,
        declaration: DeclarationId,
        arity: usize,
        link: Option<Arc<[Term]>>,
        output: &Term,
        source: SourceRange,
    ) -> Result<FunctionTarget, Error> {
        if !self.comptime_only || !output.is_comptime_callable() {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        if self.graph.is_some() {
            let arguments =
                link.ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
            let handle = self.iir_callable_handle(&iir::InstanceKey {
                declaration,
                arguments,
            })?;
            if handle.ty() != output {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            }
            return Ok(FunctionTarget::Concrete(handle));
        }
        let definition = self
            .functions
            .declaration(&declaration)
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(source)))?;
        if definition.phase != nir::program::FunctionPhase::Comptime {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        if arity != definition.type_parameters.len() {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(FunctionTarget::Declaration(declaration))
    }

    pub(super) fn finish_dynamic_function_instance(
        &mut self,
        target: FunctionTarget,
        arguments: &[Term],
        output: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        match target {
            FunctionTarget::Declaration(id) => {
                self.function_instance_value(id, arguments, output, source)
            }
            FunctionTarget::Concrete(handle) => {
                self.charge_copy_work(Some(source), |fuel| {
                    for term in arguments {
                        nir::copy_budget::charge_term_copy(term, fuel, 128)?;
                    }
                    Ok(())
                })?;
                if handle.key().arguments.as_ref() != arguments {
                    return Err(self.failure(ErrorKind::InvalidCallPlan, Some(source)));
                }
                Ok(Value::IirCallable(handle))
            }
        }
    }

    pub(super) fn freeze_instance_argument(
        &mut self,
        value: Value,
        domain: &Term,
        source: SourceRange,
    ) -> Result<Term, Error> {
        self.tick(Some(source))?;
        self.check_value_type(&value, domain, Some(source))?;
        self.residual_input_term(value, source)
    }

    fn eval_clause_operands(
        &mut self,
        inputs: &nir::program::ClauseOperands,
        environment: &mut Environment,
        source: SourceRange,
    ) -> Result<ControlFlow<Flow, Vec<Term>>, Error> {
        let result: Result<_, Error> = environment.scoped(|environment| {
            for statement in &inputs.statements {
                match self.eval(statement, environment)? {
                    Flow::Value(_) => {}
                    flow => return Ok(ControlFlow::Break(flow)),
                }
            }
            let result = self.with_prepared_frame(source, |vm| {
                let mut values = Vec::new();
                for (slot, operand) in inputs.arguments.iter().enumerate() {
                    match vm.eval(operand, environment)? {
                        Flow::Value(value) => {
                            vm.retain_prepared(slot, operand.source, &value)?;
                            values.push(value);
                        }
                        flow => return Ok(ControlFlow::Break(flow)),
                    }
                }
                Ok(ControlFlow::Continue(values))
            })?;
            if result.is_continue() {
                self.cleanup_block(source, environment)?;
            }
            Ok(result)
        });
        let values = match result? {
            ControlFlow::Continue(values) => values,
            ControlFlow::Break(flow) => return Ok(ControlFlow::Break(flow)),
        };
        let arguments = values
            .into_iter()
            .zip(&inputs.arguments)
            .map(|(value, operand)| self.freeze_instance_argument(value, &operand.ty, source))
            .collect::<Result<_, _>>()?;
        Ok(ControlFlow::Continue(arguments))
    }

    pub(super) fn dynamic_trait_method(
        &self,
        member: DeclarationId,
        output: &Term,
        (interface, count): (DeclarationId, usize),
        arguments: Vec<Term>,
        source: SourceRange,
    ) -> Result<comptime_compiler::methods::Method, Error> {
        let mut arguments = arguments.into_iter();
        let subject = arguments
            .next()
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(source)))?;
        Ok(comptime_compiler::methods::Method {
            interface,
            member,
            subject,
            interface_arguments: arguments.by_ref().take(count).collect(),
            method_arguments: arguments.collect(),
            output: output.clone(),
            source,
        })
    }

    pub(super) fn prepare_dynamic_trait_instance(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<PreparedTraitInstance, Error> {
        let K::ComptimeFunctionInstance {
            declaration,
            inputs,
        } = &expr.kind
        else {
            return Ok(PreparedTraitInstance::Unrelated);
        };
        let Some(origin) = self.admit_dynamic_trait_instance(
            *declaration,
            inputs.arguments.len(),
            &expr.ty,
            expr.source,
        )?
        else {
            return Ok(PreparedTraitInstance::Unrelated);
        };
        let arguments = match self.eval_clause_operands(inputs, environment, expr.source)? {
            ControlFlow::Continue(arguments) => arguments,
            ControlFlow::Break(flow) => return Ok(PreparedTraitInstance::Control(flow)),
        };
        let method =
            self.dynamic_trait_method(*declaration, &expr.ty, origin, arguments, expr.source)?;
        Ok(PreparedTraitInstance::Ready(Expr {
            source: method.source,
            ty: method.output,
            kind: K::TraitMethod {
                interface: method.interface,
                member: method.member,
                subject: method.subject,
                interface_arguments: method.interface_arguments,
                method_arguments: method.method_arguments,
            },
        }))
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
            inputs,
        } = &expr.kind
        else {
            unreachable!()
        };
        let mut link = None;
        if self.graph.is_some() {
            self.charge_copy_work(Some(expr.source), |fuel| {
                link = inputs.copy_observed_arguments(fuel)?.map(Arc::from);
                Ok(())
            })?;
        }
        let target = self.admit_dynamic_function_instance(
            *declaration,
            inputs.arguments.len(),
            link,
            &expr.ty,
            expr.source,
        )?;
        let arguments = match self.eval_clause_operands(inputs, environment, expr.source)? {
            ControlFlow::Continue(arguments) => arguments,
            ControlFlow::Break(flow) => return Ok(flow),
        };
        // Source formation checks the declaration. Graph formation checks the actual key.
        self.finish_dynamic_function_instance(target, &arguments, &expr.ty, expr.source)
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
                interface: nir::program::FunctionDeclaration {
                    source: nir::program::SourceRange { start: 0, end: 0 },
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
                },
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
                inputs: nir::program::ClauseOperands {
                    observations: Default::default(),
                    statements: vec![],
                    arguments: vec![field],
                },
            },
        }
    }
    #[test]
    fn nested_clause_scopes_do_work_independent_of_unrelated_caller_locals() {
        let package = package();
        let mut body = expression(&package);
        let K::ComptimeFunctionInstance { inputs, .. } = &mut body.kind else {
            unreachable!()
        };
        let temporary = LocalId { owner: 3, index: 0 };
        let field = &mut inputs.arguments[0];
        inputs.statements.push(Expr {
            source: field.source,
            ty: Term::Primitive(Primitive::Void),
            kind: K::Let {
                binding: temporary,
                value: Box::new(field.clone()),
            },
        });
        field.kind = K::Local(temporary);
        for _ in 0..32 {
            body = Expr {
                source: body.source,
                ty: body.ty.clone(),
                kind: K::Block {
                    statements: vec![],
                    result: Some(Box::new(body)),
                },
            };
        }
        let code = Arc::new(comptime_compiler::compile(&body, &mut 10000).unwrap());
        for compiled in [false, true] {
            let mut costs = Vec::new();
            for width in [1, 4096] {
                let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                let mut env: Environment = (0..width)
                    .map(|index| (LocalId { owner: 2, index }, Value::Bool(true)))
                    .collect();
                let before = vm.remaining_steps();
                let flow = if compiled {
                    vm.execute_code(&code, &mut env)
                } else {
                    vm.eval(&body, &mut env)
                }
                .unwrap();
                assert!(
                    matches!(flow, Flow::Value(Value::FunctionInstance { arguments, .. })
                    if arguments == [Term::Bool(true)])
                );
                assert_eq!(env.len(), width as usize);
                assert!(!env.contains_key(&temporary));
                costs.push(before - vm.remaining_steps());
            }
            assert_eq!(costs[0], costs[1]);
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
                let K::ComptimeFunctionInstance { inputs, .. } = &mut body.kind else {
                    unreachable!()
                };
                inputs.statements.push(Expr {
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
                });
                if !valid {
                    inputs.arguments.clear();
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
                    assert!(matches!(
                        result.unwrap(),
                        Flow::Value(Value::FunctionInstance { .. })
                    ));
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
        let Flow::Value(value) = vm.eval(&expression(&package), &mut env).unwrap() else {
            panic!("instance formation returned control")
        };
        assert!(matches!(value, Value::FunctionInstance { .. }));
        assert!(vm.instances.is_empty());
        assert_eq!(vm.invoke_value(value, vec![]).unwrap(), Value::Bool(true));
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
                let Flow::Value(first) = execute(&mut vm, &mut env).unwrap() else {
                    panic!("expected formed function instance")
                };
                assert!(matches!(first, Value::FunctionInstance { .. }));
                assert!(vm.instances.is_empty());
                assert_eq!(
                    vm.invoke_value(first.clone(), vec![]).unwrap(),
                    Value::Bool(true)
                );
                let prepared = vm
                    .instances
                    .lookup(package.functions[0].declaration, &[Term::Bool(true)])
                    .unwrap()
                    .clone();
                assert_eq!(vm.instances.len(), 1);
                outcome.set(ContractOutcome::Rejected);
                assert_eq!(
                    execute(&mut vm, &mut env).unwrap_err().kind,
                    ErrorKind::TypeContractRejected
                );
                assert_eq!(vm.instances.len(), 1);
                outcome.set(ContractOutcome::Satisfied);
                let Flow::Value(recovered) = execute(&mut vm, &mut env).unwrap() else {
                    panic!("expected recovered function instance")
                };
                assert_eq!(first, recovered);
                assert_eq!(
                    vm.invoke_value(recovered, vec![]).unwrap(),
                    Value::Bool(true)
                );
                assert!(Arc::ptr_eq(
                    &prepared,
                    vm.instances
                        .lookup(package.functions[0].declaration, &[Term::Bool(true)])
                        .unwrap()
                ));
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
    fn retained_identity_admission_is_indexed_budgeted_and_recoverable() {
        let mut costs = Vec::new();
        for count in [2, 128] {
            let graph = graph(count);
            let mut vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
            let key = &graph.callable_identities[1].key;
            let source = SourceRange { start: 1, end: 2 };
            vm.budget.steps = 1;
            assert_eq!(
                vm.admit_callable_identity(key.declaration, &key.arguments, source)
                    .unwrap_err()
                    .kind,
                ErrorKind::StepsExhausted
            );
            vm.budget.steps = 100;
            assert_eq!(
                vm.admit_callable_identity(key.declaration, &[argument(count + 1)], source)
                    .unwrap_err()
                    .kind,
                ErrorKind::UnknownFunction
            );
            let before = vm.budget.steps;
            vm.admit_callable_identity(key.declaration, &key.arguments, source)
                .unwrap();
            costs.push(before - vm.budget.steps);
            assert!(vm.events().is_empty());
        }
        assert_eq!(costs[0], costs[1]);
        assert!(costs[0] > 0);
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
