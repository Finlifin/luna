//! Named declarations collect contextual inputs in the caller before forming their result.
use super::*;
use input_collection::{InputPlan, Plan, PlanView, Progress, Target};

enum Contract<'a> {
    Alias(&'a nir::type_aliases::Contract),
    Struct(generated_structs::StructDefinition<'a>),
    Enum(generated_enums::EnumDefinition<'a>),
    Newtype(generated_newtypes::NewtypeDefinition<'a>),
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

enum Receiver<'a> {
    Type(Contract<'a>),
    Lexical,
    Callable(dynamic_instances::FunctionTarget),
}

enum Admitted {
    Value(Value),
    Body(Arc<Expr>),
}

pub(super) struct Frame<'a, 'e> {
    plan: Plan<'e>,
    inputs: Option<(Receiver<'a>, declaration_inputs::Frame<'e>)>,
    locals: Arc<[LocalId]>,
    body: Option<Arc<Expr>>,
    waiting_body: bool,
    terminal: bool,
}

impl fmt::Display for Frame<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(declaration-plan-frame (terminal {}) (waiting-body {})",
            self.terminal, self.waiting_body
        )?;
        if let Some((_, inputs)) = &self.inputs {
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
        plan: Plan<'e>,
        environment: &Environment,
    ) -> Result<Self, Error> {
        let view = plan.view();
        let locals = plan.admit_scope(vm, environment)?;
        let (receiver, parameters, domains, referenced) = match view.target {
            Target::Callable(declaration) => {
                let target = vm.admit_dynamic_function_instance(
                    declaration,
                    view.arguments.len(),
                    None,
                    view.ty,
                    view.source,
                )?;
                let definition = vm
                    .functions
                    .declaration(&declaration)
                    .expect("source declaration admitted for contextual formation");
                let (domains, referenced) = vm.query_admission(Some(view.source), |vm, fuel| {
                    let borrowed = function_inputs::input_domains(
                        &vm.declaration_parameter_domains,
                        &definition.type_parameters,
                        fuel,
                    )?;
                    copy_domains(view, &borrowed, fuel)
                })?;
                let parameters = nir::declaration_parameters(&definition.type_parameters, &domains)
                    .map_err(|_| vm.failure(ErrorKind::Type, Some(view.source)))?;
                (Receiver::Callable(target), parameters, domains, referenced)
            }
            Target::Lexical {
                parameters,
                body,
                projections,
            } => {
                let valid = admitted_work(vm, view.source, |fuel| {
                    nir::program::check_lexical_type_inputs(
                        parameters,
                        view.arguments,
                        view.ty,
                        &body.ty,
                        projections,
                        fuel,
                    )
                })?;
                if !vm.comptime_only || !valid {
                    return Err(vm.failure(ErrorKind::InvalidCallPlan, Some(view.source)));
                }
                vm.effect_charge(parameters.len(), Some(view.source))?;
                let borrowed: Vec<_> = view
                    .arguments
                    .iter()
                    .map(|input| &input.expression().expect("checked lexical operand").ty)
                    .collect();
                let (domains, referenced) = vm.query_admission(Some(view.source), |_, fuel| {
                    copy_domains(view, &borrowed, fuel)
                })?;
                let binders = nir::declaration_parameters(parameters, &domains)
                    .map_err(|_| vm.failure(ErrorKind::Type, Some(view.source)))?;
                (Receiver::Lexical, binders, domains, referenced)
            }
            Target::Alias { .. } | Target::Nominal(_) => {
                let contract = Contract::open(vm, view, plan.inputs_diverge())?;
                let parameters = contract
                    .parameters()
                    .map_err(|_| vm.failure(ErrorKind::Type, Some(view.source)))?;
                let (domains, referenced) = vm.query_admission(Some(view.source), |_, fuel| {
                    copy_domains(view, contract.domains(), fuel)
                })?;
                (Receiver::Type(contract), parameters, domains, referenced)
            }
            _ => unreachable!("named declaration input receiver"),
        };
        if view.arguments.len() != parameters.len() {
            return Err(vm.failure(ErrorKind::Type, Some(view.source)));
        }
        vm.integer_operation(Some(view.source), |budget| {
            budget.consume(view.arguments.len().saturating_add(1))
        })?;
        let specialize = view.schedule.bindings.is_some()
            || parameters
                .iter()
                .any(|p| referenced.contains(&p.declaration));
        let inputs = declaration_inputs::Frame::new(
            plan.clone(),
            parameters,
            domains,
            specialize,
            locals.clone(),
        );
        Ok(Self {
            plan,
            inputs: Some((receiver, inputs)),
            locals,
            body: None,
            waiting_body: false,
            terminal: false,
        })
    }
}

/// Observe dependencies during the copy preflight. Explicit lexical bindings
/// already require specialization, so their domains only need the copy charge.
fn copy_domains<D: std::borrow::Borrow<Term>>(
    view: PlanView<'_>,
    domains: &[D],
    fuel: &mut usize,
) -> Result<(Vec<Term>, BTreeSet<DeclarationId>), nir::NominalAdmissionError> {
    let mut referenced = BTreeSet::new();
    let copied = domains
        .iter()
        .map(|domain| {
            let domain = domain.borrow();
            if view.schedule.bindings.is_some() {
                nir::type_computations::charge_term(domain, fuel)?;
            } else {
                nir::type_computations::rigid_declarations_bounded(domain, fuel, &mut referenced)?;
            }
            Ok(domain.clone())
        })
        .collect::<Result<_, nir::NominalAdmissionError>>()?;
    Ok((copied, referenced))
}

impl<'a> Contract<'a> {
    fn open(vm: &Interpreter<'a>, view: PlanView<'_>, diverges: bool) -> Result<Self, Error> {
        let (contract, result) = match view.target {
            Target::Alias { declaration, body } => {
                let result = match body {
                    _ if diverges => Primitive::Never,
                    None => Primitive::Void,
                    Some(body) if body.ty == Term::Primitive(Primitive::Type) => Primitive::Type,
                    Some(body) if body.ty == Term::Primitive(Primitive::Never) => Primitive::Never,
                    _ => return Err(vm.failure(ErrorKind::Type, Some(view.source))),
                };
                let alias = vm
                    .alias_contracts
                    .get(&declaration)
                    .ok_or_else(|| vm.failure(ErrorKind::Type, Some(view.source)))?;
                (Self::Alias(alias), result)
            }
            Target::Nominal(id) => {
                let contract = if let Some(def) = vm.structs.get(&id) {
                    Self::Struct(def.clone())
                } else if let Some(def) = vm.enums.get(&id) {
                    Self::Enum(def.clone())
                } else if let Some(def) = vm.newtypes.handle(&id) {
                    Self::Newtype(def)
                } else {
                    return Err(vm.failure(ErrorKind::Type, Some(view.source)));
                };
                (
                    contract,
                    if diverges {
                        Primitive::Never
                    } else {
                        Primitive::Type
                    },
                )
            }
            _ => unreachable!("nominal or alias contract"),
        };
        if *view.ty != Term::Primitive(result) {
            return Err(vm.failure(ErrorKind::Type, Some(view.source)));
        }
        Ok(contract)
    }
}

impl Contract<'_> {
    fn finish(
        &self,
        vm: &mut Interpreter<'_>,
        view: PlanView<'_>,
        types: Vec<Term>,
    ) -> Result<Admitted, Error> {
        let declaration = match view.target {
            Target::Alias { declaration, .. } | Target::Nominal(declaration) => declaration,
            _ => unreachable!("nominal target"),
        };
        let domains = self.domains();
        let requirements = self.requirements();
        let capabilities = self.capabilities();
        let admitted = vm.query_admission(Some(view.source), |vm, fuel| {
            vm.with_admission_queries(|queries| {
                let ids = match self {
                    Contract::Alias(def) => &def.parameters,
                    Contract::Struct(def) => &def.type_parameters,
                    Contract::Enum(def) => &def.type_parameters,
                    Contract::Newtype(def) => {
                        return def.admit_arguments_in_scope(&types, &[], fuel, queries);
                    }
                };
                nir::admit_declaration_arguments_with_queries_in(
                    ids, domains, &types, fuel, queries,
                )
            })
        })?;
        for requirement in requirements {
            vm.tick(Some(view.source))?;
            let predicate = admitted_work(vm, view.source, |fuel| {
                admitted.instantiate_predicate(requirement, fuel)
            })?;
            vm.check_type_contract(&predicate, view.source)?;
        }
        let capabilities = capabilities
            .iter()
            .map(|requirement| {
                admitted_work(vm, view.source, |fuel| {
                    admitted.instantiate_requirement(requirement, fuel)
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        for capability in &capabilities {
            vm.check_capability_contract(capability, view.source)?;
        }
        if let Target::Alias { body, .. } = view.target {
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
                let mut specialized = body.clone();
                specialized.rewrite_types(&mut |ty| {
                    admitted_work(vm, body.source, |fuel| admitted.instantiate(ty, fuel))
                })?;
                Ok(Admitted::Body(Arc::new(specialized)))
            } else {
                Ok(Admitted::Value(Value::Unit))
            };
        }
        Ok(Admitted::Value(Value::Type(Term::Nominal {
            declaration,
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
            return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.plan.view().source)));
        }
        if let Some((_, inputs)) = &mut self.inputs {
            match inputs.next_input(vm, environment)? {
                Progress::InputsComplete => {}
                progress => return Ok(progress),
            }
            let (receiver, inputs) = self.inputs.take().expect("completed declaration inputs");
            let view = self.plan.view();
            let (domains, types) = inputs.finish().ok_or_else(|| {
                let kind = match receiver {
                    Receiver::Lexical => ErrorKind::TypeContractUnresolved,
                    _ => ErrorKind::Type,
                };
                vm.failure(kind, Some(view.source))
            })?;
            let admitted = match receiver {
                Receiver::Lexical => {
                    let Target::Lexical {
                        parameters, body, ..
                    } = view.target
                    else {
                        unreachable!()
                    };
                    let substitution = vm.query_admission(Some(view.source), |vm, fuel| {
                        vm.with_admission_queries(|queries| {
                            nir::admit_declaration_arguments_with_queries_in(
                                parameters, &domains, &types, fuel, queries,
                            )
                            .map(nir::AdmittedArguments::into_substitution)
                        })
                    })?;
                    let before = usize::try_from(vm.budget.steps).unwrap_or(usize::MAX);
                    let mut fuel = before;
                    let result =
                        dynamic_instances::instantiate_code(&substitution, body, &mut fuel);
                    vm.budget.steps -= (before - fuel) as u64;
                    let body = result.map_err(|kind| vm.failure(kind, Some(body.source)))?;
                    Admitted::Body(Arc::new(body))
                }
                Receiver::Type(contract) => contract.finish(vm, view, types)?,
                Receiver::Callable(target) => Admitted::Value(
                    vm.finish_dynamic_function_instance(target, &types, view.ty, view.source)?,
                ),
            };
            match admitted {
                Admitted::Value(value) => return Ok(Progress::Complete(Flow::Value(value))),
                Admitted::Body(body) => self.body = Some(body),
            }
        }
        let body = self.body.as_ref().ok_or_else(|| {
            vm.failure(ErrorKind::InvalidControlFlow, Some(self.plan.view().source))
        })?;
        self.waiting_body = true;
        if let Some(frame) = vm.start_expression_input(body, environment)? {
            return Ok(Progress::Input(frame));
        }
        vm.eval(body, environment).map(Progress::Complete)
    }
}

impl InputPlan for Frame<'_, '_> {
    fn cancel(&mut self, environment: &mut Environment) {
        self.terminal = true;
        self.cleanup(environment);
    }

    fn next_input(
        &mut self,
        vm: &mut Interpreter<'_>,
        environment: &mut Environment,
    ) -> Result<Progress, Error> {
        let result = self.next(vm, environment);
        if !matches!(&result, Ok(Progress::Input(_) | Progress::DomainDelivered)) {
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
            return Err(vm.failure(ErrorKind::InvalidControlFlow, Some(self.plan.view().source)));
        }
        if self.waiting_body {
            self.terminal = true;
            *environment = result.environment;
            return result.flow.map(Some);
        }
        let result = self
            .inputs
            .as_mut()
            .map(|(_, inputs)| inputs)
            .ok_or_else(|| {
                vm.failure(ErrorKind::InvalidControlFlow, Some(self.plan.view().source))
            })?
            .accept_input(vm, environment, result);
        if !matches!(&result, Ok(None)) {
            self.terminal = true;
        }
        result
    }

    fn cleanup(&self, environment: &mut Environment) {
        for binding in self.locals.iter() {
            environment.remove(binding);
        }
    }
}

fn admitted_work<T>(
    vm: &mut Interpreter<'_>,
    source: SourceRange,
    work: impl FnOnce(&mut usize) -> Result<T, nir::type_computations::Error>,
) -> Result<T, Error> {
    let before = usize::try_from(vm.budget.steps).unwrap_or(usize::MAX);
    let mut remaining = before;
    let result = work(&mut remaining);
    vm.budget.steps -= (before - remaining) as u64;
    result.map_err(|error| {
        vm.failure(
            if matches!(error, nir::type_computations::Error::Budget) {
                ErrorKind::StepsExhausted
            } else {
                ErrorKind::Type
            },
            Some(source),
        )
    })
}
