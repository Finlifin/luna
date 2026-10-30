//! Owned nominal/alias plans separate input collection, admission and body execution.
use super::*;
use normal_plan::{InputPlan, Progress};

#[derive(Clone)]
pub(super) enum Expression<'e> {
    Borrowed(&'e Expr),
    Owned(Arc<Expr>),
}
impl std::ops::Deref for Expression<'_> {
    type Target = Expr;
    fn deref(&self) -> &Expr {
        match self {
            Self::Borrowed(expr) => expr,
            Self::Owned(expr) => expr,
        }
    }
}

enum Contract<'a> {
    Alias(&'a nir::type_aliases::Contract),
    Struct(generated_structs::StructDefinition<'a>),
    Enum(generated_enums::EnumDefinition<'a>),
    Newtype(&'a nir::newtypes::Newtype),
}
macro_rules! contract_data {
    ($value:expr, $def:ident, $body:expr) => {
        match $value {
            Contract::Alias($def) => $body,
            Contract::Struct($def) => $body,
            Contract::Enum($def) => $body,
            Contract::Newtype($def) => $body,
        }
    };
}
impl Contract<'_> {
    fn parameters(&self) -> Result<Vec<nir::DeclarationParameter>, nir::TypeError> {
        contract_data!(self, def, def.parameter_binders())
    }
    fn domains(&self) -> &[Term] {
        contract_data!(self, def, &def.parameter_types)
    }
    fn requirements(&self) -> &[nir::program::TypePredicate] {
        contract_data!(self, def, &def.requirements)
    }
    fn capabilities(&self) -> &[nir::program::TraitRequirement] {
        contract_data!(self, def, &def.capabilities)
    }
}

enum Admitted {
    Value(Value),
    Body(Arc<Expr>),
}
pub(super) struct Frame<'a, 'e> {
    expression: Expression<'e>,
    contract: Contract<'a>,
    inputs: Option<nominal_inputs::Frame<'e>>,
    body: Option<Arc<Expr>>,
    waiting_body: bool,
    terminal: bool,
}
impl fmt::Display for Frame<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(nominal-plan-frame (terminal {}) (waiting-body {})",
            self.terminal, self.waiting_body
        )?;
        if let Some(inputs) = &self.inputs {
            write!(f, " {inputs}")?;
        }
        if let Some(body) = &self.body {
            write!(f, " (body {body})")?;
        }
        write!(f, ")")
    }
}
impl<'a, 'e> Frame<'a, 'e> {
    pub(super) fn new(
        vm: &mut Interpreter<'a>,
        expression: Expression<'e>,
        environment: &Environment,
    ) -> Result<Self, Error> {
        let expr = &*expression;
        let (K::NominalType {
            preparations,
            bindings,
            order,
            declaration,
            arguments,
        }
        | K::CheckTypeAlias {
            preparations,
            bindings,
            order,
            declaration,
            arguments,
            ..
        }) = &expr.kind
        else {
            unreachable!()
        };
        vm.integer_operation(Some(expr.source), |budget| {
            budget.consume(arguments.len().saturating_add(preparations.len()))
        })?;
        for preparation in preparations {
            if let K::LetPattern {
                pattern: nir::program::Pattern::Tuple(fields),
                ..
            } = &preparation.expression.kind
            {
                vm.integer_operation(Some(expr.source), |budget| budget.consume(fields.len()))?;
            }
        }
        let locals = expr
            .kind
            .nominal_local_bindings()
            .ok_or_else(|| vm.failure(ErrorKind::Type, Some(expr.source)))?;
        if locals.iter().any(|id| environment.contains_key(id)) {
            return Err(vm.failure(ErrorKind::Type, Some(expr.source)));
        }
        let contract = if matches!(expr.kind, K::CheckTypeAlias { .. }) {
            Contract::Alias(
                *vm.alias_contracts
                    .get(declaration)
                    .ok_or_else(|| vm.failure(ErrorKind::Type, Some(expr.source)))?,
            )
        } else if let Some(def) = vm.structs.get(declaration) {
            Contract::Struct(def.clone())
        } else if let Some(def) = vm.enums.get(declaration) {
            Contract::Enum(def.clone())
        } else if let Some(def) = vm.newtypes.get(declaration) {
            Contract::Newtype(*def)
        } else {
            return Err(vm.failure(ErrorKind::Type, Some(expr.source)));
        };
        let parameters = contract
            .parameters()
            .map_err(|_| vm.failure(ErrorKind::Type, Some(expr.source)))?;
        let domains = contract.domains();
        let expected = if arguments
            .iter()
            .any(nir::program::NormalFormArgument::is_diverging)
            || preparations
                .iter()
                .any(|p| p.expression.ty == Term::Primitive(Primitive::Never))
        {
            Primitive::Never
        } else if let K::CheckTypeAlias { body, .. } = &expr.kind {
            match body {
                None => Primitive::Void,
                Some(body) if body.ty == Term::Primitive(Primitive::Type) => Primitive::Type,
                Some(body) if body.ty == Term::Primitive(Primitive::Never) => Primitive::Never,
                _ => return Err(vm.failure(ErrorKind::Type, Some(expr.source))),
            }
        } else {
            Primitive::Type
        };
        if arguments.len() != parameters.len() || expr.ty != Term::Primitive(expected) {
            return Err(vm.failure(ErrorKind::Type, Some(expr.source)));
        }
        vm.integer_operation(Some(expr.source), |budget| {
            budget.consume(arguments.len().saturating_add(1))
        })?;
        if let Some(order) = order {
            vm.integer_operation(Some(expr.source), |budget| budget.consume(order.len()))?;
            if order.len() != arguments.len() {
                return Err(vm.failure(ErrorKind::Type, Some(expr.source)));
            }
            let mut seen = vec![false; arguments.len()];
            for &slot in order {
                if slot >= seen.len() || seen[slot] {
                    return Err(vm.failure(ErrorKind::Type, Some(expr.source)));
                }
                seen[slot] = true;
            }
        }
        let specialize = bindings.is_some()
            || domains.iter().any(|domain| {
                parameters
                    .iter()
                    .any(|parameter| domain.mentions_rigid(parameter.declaration))
            });
        if specialize {
            for argument in arguments {
                nir::copy_budget::charge_argument_type_rewrite(argument, &mut vm.budget.steps)
                    .map_err(|error| {
                        vm.failure(
                            match error {
                                nir::copy_budget::CopyBudgetExhausted::Steps => {
                                    ErrorKind::StepsExhausted
                                }
                                nir::copy_budget::CopyBudgetExhausted::Depth => {
                                    ErrorKind::DepthExhausted
                                }
                            },
                            Some(expr.source),
                        )
                    })?;
            }
        }
        let before = usize::try_from(vm.budget.steps).unwrap_or(usize::MAX);
        let mut remaining = before;
        let charged = domains
            .iter()
            .try_for_each(|domain| nir::type_computations::charge_term(domain, &mut remaining));
        vm.budget.steps -= (before - remaining) as u64;
        charged.map_err(|_| vm.failure(ErrorKind::StepsExhausted, Some(expr.source)))?;
        let inputs = nominal_inputs::Frame::new(
            expression.clone(),
            parameters,
            domains.to_vec(),
            specialize,
        );
        Ok(Self {
            expression,
            contract,
            inputs: Some(inputs),
            body: None,
            waiting_body: false,
            terminal: false,
        })
    }
}
impl Contract<'_> {
    fn finish(
        &self,
        vm: &mut Interpreter<'_>,
        expr: &Expr,
        parameters: Vec<nir::DeclarationParameter>,
        types: Vec<Term>,
    ) -> Result<Admitted, Error> {
        let (K::NominalType { declaration, .. } | K::CheckTypeAlias { declaration, .. }) =
            &expr.kind
        else {
            unreachable!()
        };
        let domains = self.domains();
        let requirements = self.requirements();
        let capabilities = self.capabilities();
        let before = usize::try_from(vm.budget.steps).unwrap_or(usize::MAX);
        let mut remaining = before;
        let ids = parameters.iter().map(|p| p.declaration).collect::<Vec<_>>();
        // Captured newtype binders follow their own capture-first admission
        // order; do not replace it with the ordinary declaration telescope.
        let valid = if let Contract::Newtype(def) = self {
            def.validate_completed_arguments_with_evidence_in(
                &types,
                &mut remaining,
                |ty, variant, fuel| {
                    vm.normal_value_fields(ty, variant, fuel).ok_or_else(|| {
                        if *fuel == 0 {
                            nir::NominalAdmissionError::Budget
                        } else {
                            nir::NominalAdmissionError::Invalid(
                                "nominal value layout unavailable".into(),
                            )
                        }
                    })
                },
                |value, domain, _| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if vm.abstract_value_parameters.get(declaration)==Some(domain)), vm.target)
        } else {
            nir::validate_declaration_arguments_with_evidence_in(
                &ids,
                domains,
                &types,
                &mut remaining,
                |ty, variant, fuel| {
                    vm.normal_value_fields(ty, variant, fuel).ok_or_else(|| {
                        if *fuel == 0 {
                            nir::NominalAdmissionError::Budget
                        } else {
                            nir::NominalAdmissionError::Invalid(
                                "nominal value layout unavailable".into(),
                            )
                        }
                    })
                },
                |value, domain, _| matches!(value, Term::Rigid {declaration,sort:nir::Sort::ComptimeValue} if vm.abstract_value_parameters.get(declaration)==Some(domain)),
                vm.target,
            )
        };
        vm.budget.steps -= (before - remaining) as u64;
        valid.map_err(|error| {
            vm.failure(
                match error {
                    nir::NominalAdmissionError::Budget => ErrorKind::StepsExhausted,
                    nir::NominalAdmissionError::Invalid(_) => ErrorKind::Type,
                },
                Some(expr.source),
            )
        })?;
        for requirement in requirements {
            vm.tick(Some(expr.source))?;
            let predicate = requirement
                .instantiate_parameters(&parameters, &types)
                .map_err(|_| vm.failure(ErrorKind::Type, Some(expr.source)))?;
            vm.check_type_contract(&predicate, expr.source)?;
        }
        if !vm.capabilities_hold_parameters(capabilities, &parameters, &types) {
            return Err(vm.failure(ErrorKind::TypeContractUnresolved, Some(expr.source)));
        }
        if let K::CheckTypeAlias { body, .. } = &expr.kind {
            return if let Some(body) = body {
                nir::copy_budget::charge_type_rewrite(body, &mut vm.budget.steps).map_err(
                    |error| {
                        vm.failure(
                            match error {
                                nir::copy_budget::CopyBudgetExhausted::Steps => {
                                    ErrorKind::StepsExhausted
                                }
                                nir::copy_budget::CopyBudgetExhausted::Depth => {
                                    ErrorKind::DepthExhausted
                                }
                            },
                            Some(body.source),
                        )
                    },
                )?;
                let mut specialized = (**body).clone();
                specialized.rewrite_types(&mut |ty| {
                    vm.instantiate_contextual_domain(ty, &parameters, &types, body.source)
                })?;
                Ok(Admitted::Body(Arc::new(specialized)))
            } else {
                Ok(Admitted::Value(Value::Unit))
            };
        }
        Ok(Admitted::Value(Value::Type(Term::Nominal {
            declaration: *declaration,
            arguments: types,
        })))
    }
}

impl Frame<'_, '_> {
    fn next(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
    ) -> Result<Progress, Error> {
        if self.terminal || self.waiting_body {
            return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.expression.source)));
        }
        if let Some(inputs) = &mut self.inputs {
            match inputs.next_input(vm, environment)? {
                Progress::InputsComplete => {}
                progress => return Ok(progress),
            }
            let (parameters, types) = self.inputs.take().unwrap().finish_parts();
            match self
                .contract
                .finish(vm, &self.expression, parameters, types)?
            {
                Admitted::Value(value) => return Ok(Progress::Complete(Flow::Value(value))),
                Admitted::Body(body) => self.body = Some(body),
            }
        }
        let body = self.body.as_ref().ok_or_else(|| {
            vm.failure(ErrorKind::InvalidControlFlow, Some(self.expression.source))
        })?;
        self.waiting_body = true;
        if let Some(frame) = vm.start_expression_input(body, environment)? {
            return Ok(Progress::Input(frame));
        }
        vm.eval(body, environment).map(Progress::Complete)
    }
}
impl InputPlan for Frame<'_, '_> {
    fn next_input(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
    ) -> Result<Progress, Error> {
        let result = self.next(vm, environment);
        if !matches!(&result, Ok(Progress::Input(_))) {
            self.terminal = true;
        }
        result
    }
    fn accept_input(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
        result: bytecode::InputResult,
    ) -> Result<Option<Flow>, Error> {
        if self.terminal {
            return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.expression.source)));
        }
        if self.waiting_body {
            self.terminal = true;
            *environment = result.environment;
            return result.flow.map(Some);
        }
        let result = self
            .inputs
            .as_mut()
            .ok_or_else(|| vm.failure(ErrorKind::InvalidControlFlow, Some(self.expression.source)))?
            .accept_input(vm, environment, result);
        if !matches!(&result, Ok(None)) {
            self.terminal = true;
        }
        result
    }
    fn cleanup(&self, environment: &mut Environment) {
        for binding in self
            .expression
            .kind
            .nominal_local_bindings()
            .expect("validated nominal plan")
        {
            environment.remove(&binding);
        }
    }
}
