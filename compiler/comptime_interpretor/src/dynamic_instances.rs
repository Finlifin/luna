//! Clause computations use the ordinary function-instance cache and contracts.
use super::*;

pub(super) enum PreparedTraitInstance {
    Unrelated,
    Ready(Expr),
    Control(Flow),
}

impl Interpreter<'_> {
    pub(super) fn evaluate_instance_default(
        &mut self,
        construction: SourceRange,
        argument: &nir::program::CallArgument,
        callee: &Value,
        defaults: &mut Environment,
        caller: &mut Environment,
    ) -> Result<Flow, Error> {
        let Value::Closure(closure) = callee else {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(construction)));
        };
        let (declaration, arguments) = closure
            .declaration_instance()
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(construction)))?;
        let definition = self
            .functions
            .get(&declaration)
            .copied()
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(construction)))?;
        let binders = self.input_binders(&definition.type_parameters);
        self.evaluate_bound_default(
            construction,
            argument,
            &binders,
            arguments,
            defaults,
            caller,
        )
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
        self.charge_copy_work(Some(argument.value.source), |fuel| {
            nir::copy_budget::charge_substitution(&argument.value, binders, arguments, fuel)
        })?;
        let value = argument
            .value
            .instantiate_parameters(binders, arguments)
            .map_err(|_| self.failure(ErrorKind::Type, Some(argument.value.source)))?;
        let prepared = nir::program::CallArgument {
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
        let K::TraitMethod {
            interface,
            member,
            subject,
            interface_arguments,
            method_arguments,
        } = &method.kind
        else {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(method.source)));
        };
        let (binders, _) = self
            .method_contracts
            .get(&(*interface, *member))
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
        let work = self.method_contracts.len();
        self.integer_operation(Some(expr.source), |budget| budget.consume(work))?;
        let mut candidates = self
            .method_contracts
            .iter()
            .filter(|((_, member), _)| member == declaration);
        let Some((&(interface, member), (parameters, contract))) = candidates.next() else {
            return Ok(PreparedTraitInstance::Unrelated);
        };
        if candidates.next().is_some() {
            return Err(self.failure(ErrorKind::InvalidCallPlan, Some(expr.source)));
        }
        if !self.comptime_only
            || !contract.signature.is_comptime_callable()
            || !expr.ty.is_comptime_callable()
        {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
        }
        let count = parameters.len();
        let interface_count = count
            .checked_sub(1 + contract.type_parameters.len())
            .ok_or_else(|| self.failure(ErrorKind::InvalidCallPlan, Some(expr.source)))?;
        let Term::Tuple(domains) = &subject.ty else {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        };
        if domains.len() != count || count < 1 + interface_count {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let values = match self.eval(subject, environment)? {
            Flow::Value(Value::Tuple(values)) => values,
            Flow::Value(_) => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
            flow => return Ok(PreparedTraitInstance::Control(flow)),
        };
        if values.len() != domains.len() {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        self.integer_operation(Some(expr.source), |budget| budget.consume(values.len()))?;
        let mut inputs = Vec::with_capacity(values.len());
        for (value, domain) in values.into_iter().zip(domains) {
            if !self.has_type(&value, domain) {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            }
            inputs.push(self.residual_input_term(value, expr.source)?);
        }
        Ok(PreparedTraitInstance::Ready(Expr {
            source: expr.source,
            ty: expr.ty.clone(),
            kind: K::TraitMethod {
                interface,
                member,
                subject: inputs[0].clone(),
                interface_arguments: inputs[1..1 + interface_count].to_vec(),
                method_arguments: inputs[1 + interface_count..].to_vec(),
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
            subject,
        } = &expr.kind
        else {
            unreachable!()
        };
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
        }
        let definition = self
            .functions
            .get(declaration)
            .copied()
            .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, Some(expr.source)))?;
        if definition.phase != nir::program::FunctionPhase::Comptime
            || !expr.ty.is_comptime_callable()
        {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
        }
        let Term::Tuple(domains) = &subject.ty else {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        };
        if domains.len() != definition.type_parameters.len() {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let packet = match self.eval(subject, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        let Value::Tuple(values) = packet else {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        };
        if values.len() != domains.len() {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        self.integer_operation(Some(expr.source), |budget| budget.consume(values.len()))?;
        let mut arguments = Vec::with_capacity(values.len());
        for (value, domain) in values.into_iter().zip(domains) {
            if !self.has_type(&value, domain) {
                return Err(self.failure(ErrorKind::Type, Some(expr.source)));
            }
            arguments.push(self.residual_input_term(value, expr.source)?);
        }
        // Instance creation validates declared domains and requirements on each
        // evaluation, including cache hits. No partially admitted key is stored.
        self.eval_function_instance(
            &Expr {
                source: expr.source,
                ty: expr.ty.clone(),
                kind: K::FunctionInstance {
                    declaration: *declaration,
                    arguments,
                },
            },
            environment,
        )
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
        // A controlled solver observes the transport boundary, not type theory.
        let outcome = std::cell::Cell::new(ContractOutcome::Satisfied);
        let solver = |_: &nir::program::TypePredicate| outcome.get();
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_type_solver(&solver);
        let mut env = Environment::new();
        env.insert(LocalId { owner: 2, index: 0 }, Value::Bool(true));
        vm.eval(&expression(&package), &mut env).unwrap();
        assert_eq!(vm.instances.len(), 1);
        outcome.set(ContractOutcome::Rejected);
        assert_eq!(
            vm.eval(&expression(&package), &mut env).unwrap_err().kind,
            ErrorKind::TypeContractRejected
        );
        assert_eq!(vm.instances.len(), 1);
        outcome.set(ContractOutcome::Satisfied);
        vm.eval(&expression(&package), &mut env).unwrap();
        assert_eq!(vm.instances.len(), 1);
    }
}
