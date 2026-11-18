//! Owned nominal/alias plans separate input collection, admission and body execution.
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

enum Admitted {
    Value(Value),
    Body(Arc<Expr>),
}
pub(super) struct Frame<'a, 'e> {
    plan: Plan<'e>,
    contract: Contract<'a>,
    inputs: Option<nominal_inputs::Frame<'e>>,
    locals: Arc<[LocalId]>,
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
        plan: Plan<'e>,
        environment: &Environment,
    ) -> Result<Self, Error> {
        let view = plan.view();
        let declaration = match view.target {
            Target::Nominal(id)
            | Target::Alias {
                declaration: id, ..
            } => id,
            _ => unreachable!("nominal target"),
        };
        let locals = plan.admit_scope(vm, environment)?;
        let contract = if matches!(view.target, Target::Alias { .. }) {
            Contract::Alias(
                vm.alias_contracts
                    .get(&declaration)
                    .ok_or_else(|| vm.failure(ErrorKind::Type, Some(view.source)))?,
            )
        } else if let Some(def) = vm.structs.get(&declaration) {
            Contract::Struct(def.clone())
        } else if let Some(def) = vm.enums.get(&declaration) {
            Contract::Enum(def.clone())
        } else if let Some(def) = vm.newtypes.handle(&declaration) {
            Contract::Newtype(def)
        } else {
            return Err(vm.failure(ErrorKind::Type, Some(view.source)));
        };
        let parameters = contract
            .parameters()
            .map_err(|_| vm.failure(ErrorKind::Type, Some(view.source)))?;
        let domains = contract.domains();
        let expected = if plan.inputs_diverge() {
            Primitive::Never
        } else if let Target::Alias { body, .. } = view.target {
            match body {
                None => Primitive::Void,
                Some(body) if body.ty == Term::Primitive(Primitive::Type) => Primitive::Type,
                Some(body) if body.ty == Term::Primitive(Primitive::Never) => Primitive::Never,
                _ => return Err(vm.failure(ErrorKind::Type, Some(view.source))),
            }
        } else {
            Primitive::Type
        };
        if view.arguments.len() != parameters.len() || *view.ty != Term::Primitive(expected) {
            return Err(vm.failure(ErrorKind::Type, Some(view.source)));
        }
        vm.integer_operation(Some(view.source), |budget| {
            budget.consume(view.arguments.len().saturating_add(1))
        })?;
        let before = usize::try_from(vm.budget.steps).unwrap_or(usize::MAX);
        let mut remaining = before;
        let mut referenced = BTreeSet::new();
        let charged = domains.iter().try_for_each(|domain| {
            if view.schedule.bindings.is_some() {
                nir::type_computations::charge_term(domain, &mut remaining)
            } else {
                // The copy preflight also observes rigid inputs. A domain can
                // contain them beneath an opaque head or in a closure snapshot.
                nir::type_computations::rigid_declarations_bounded(
                    domain,
                    &mut remaining,
                    &mut referenced,
                )
            }
        });
        vm.budget.steps -= (before - remaining) as u64;
        charged.map_err(|_| vm.failure(ErrorKind::StepsExhausted, Some(view.source)))?;
        let specialize = view.schedule.bindings.is_some()
            || parameters
                .iter()
                .any(|p| referenced.contains(&p.declaration));
        let inputs = nominal_inputs::Frame::new(
            plan.clone(),
            parameters,
            domains.to_vec(),
            specialize,
            locals.clone(),
        );
        Ok(Self {
            plan,
            contract,
            inputs: Some(inputs),
            locals,
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
        if let Some(inputs) = &mut self.inputs {
            match inputs.next_input(vm, environment)? {
                Progress::InputsComplete => {}
                progress => return Ok(progress),
            }
            let types = self
                .inputs
                .take()
                .unwrap()
                .finish()
                .ok_or_else(|| vm.failure(ErrorKind::Type, Some(self.plan.view().source)))?;
            match self.contract.finish(vm, self.plan.view(), types)? {
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
