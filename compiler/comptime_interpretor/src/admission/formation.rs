//! Formation of immutable types within the receiving callable telescope.
use super::*;
use crate::invocation_contracts::{formation_capability_is_closed, formation_predicate_is_closed};
use crate::type_descriptions::Telescope;

impl Interpreter<'_> {
    pub(crate) fn admit_type(&mut self, ty: &Term, depth: usize) -> Result<(), Error> {
        self.admit_in_telescope(ty, depth, &mut Telescope::default())
    }

    // The external proof query accepts propositions in the invocation's
    // world. A callable's local binder must not capture an invocation input.
    pub(crate) fn check_formation_predicate(
        &mut self,
        predicate: &nir::program::TypePredicate,
        nested: bool,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.tick(Some(source))?;
        if nested
            && predicate.trivial_truth().is_none()
            && !formation_predicate_is_closed(predicate)
        {
            return Err(self.failure(ErrorKind::TypeContractUnresolved, Some(source)));
        }
        self.check_type_contract(predicate, source)
    }

    fn formation_inputs<R>(
        &self,
        parameters: &[DeclarationId],
        domains: &[Term],
        arguments: &[Term],
        frames: &Telescope<'_>,
        fuel: &mut usize,
        project: impl FnOnce(
            &nir::AdmittedArguments<'_>,
            &mut usize,
        ) -> Result<R, nir::NominalAdmissionError>,
    ) -> Result<R, nir::NominalAdmissionError> {
        self.with_scoped_queries(&frames.parameters, |queries| {
            let inputs = nir::admit_declaration_arguments_in_scope(
                parameters,
                domains,
                arguments,
                &frames.sorts,
                fuel,
                queries,
            )?;
            project(&inputs, fuel)
        })
    }

    fn check_formation_contracts(
        &mut self,
        contracts: FormationContracts,
        nested: bool,
        source: SourceRange,
    ) -> Result<(), Error> {
        for predicate in contracts.requirements {
            self.check_formation_predicate(&predicate, nested, source)?;
        }
        for capability in contracts.capabilities {
            if nested && !formation_capability_is_closed(&capability) {
                return Err(self.failure(ErrorKind::TypeContractUnresolved, Some(source)));
            }
            self.check_capability_contract(&capability, source)?;
        }
        Ok(())
    }

    fn admit_in_telescope<'t>(
        &mut self,
        ty: &'t Term,
        depth: usize,
        frames: &mut Telescope<'t>,
    ) -> Result<(), Error> {
        if depth >= 128 {
            return Err(self.failure(ErrorKind::DepthExhausted, None));
        }
        self.tick(None)?;
        let source = SourceRange { start: 0, end: 0 };
        match ty {
            Term::Nominal {
                declaration,
                arguments,
            } => {
                if self.graph.is_some() {
                    let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                    let mut fuel = before;
                    let charged = nir::type_computations::charge_term(ty, &mut fuel);
                    self.budget.steps -= (before - fuel) as u64;
                    charged.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
                    let definition = self
                        .concrete_nominal(ty)
                        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                    // Concrete descriptors already substituted their declaration telescope.
                    let (requirements, capabilities) = match &definition {
                        ConcreteNominal::Struct(d) => (&d.requirements, &d.capabilities),
                        ConcreteNominal::Enum(d) => (&d.requirements, &d.capabilities),
                        ConcreteNominal::Newtype(d) => (&d.requirements, &d.capabilities),
                    };
                    for predicate in requirements {
                        self.tick(None)?;
                        self.check_type_contract(predicate, source)?;
                    }
                    for predicate in capabilities {
                        self.check_capability_contract(predicate, source)?;
                    }
                    for argument in arguments {
                        self.admit_in_telescope(argument, depth + 1, frames)?;
                    }
                    return Ok(());
                }
                let requirements = self.query_admission(None, |vm, fuel| {
                    let (parameters, domains, requirements, capabilities) =
                        if let Some(s) = vm.structs.get(declaration) {
                            (
                                &s.type_parameters,
                                &s.parameter_types,
                                &s.requirements,
                                &s.capabilities,
                            )
                        } else if let Some(e) = vm.enums.get(declaration) {
                            (
                                &e.type_parameters,
                                &e.parameter_types,
                                &e.requirements,
                                &e.capabilities,
                            )
                        } else if let Some(n) = vm.newtypes.get(declaration) {
                            return vm.with_scoped_queries(&frames.parameters, |queries| {
                                let inputs = n.admit_arguments_in_scope(
                                    arguments,
                                    &frames.sorts,
                                    fuel,
                                    queries,
                                )?;
                                FormationContracts::instantiate(
                                    &inputs,
                                    &n.requirements,
                                    &n.capabilities,
                                    fuel,
                                )
                            });
                        } else {
                            return Err("unknown nominal declaration".into());
                        };
                    vm.formation_inputs(
                        parameters,
                        domains,
                        arguments,
                        frames,
                        fuel,
                        |inputs, fuel| {
                            FormationContracts::instantiate(
                                inputs,
                                requirements,
                                capabilities,
                                fuel,
                            )
                        },
                    )
                })?;
                self.check_formation_contracts(requirements, !frames.sorts.is_empty(), source)?;
                for ty in arguments {
                    self.admit_in_telescope(ty, depth + 1, frames)?;
                }
            }
            Term::NormalForm {
                head, arguments, ..
            } => {
                let metadata = self
                    .normal_forms
                    .get(head)
                    .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
                self.query_descriptions(None, |vm, fuel| {
                    vm.with_scoped_queries(&frames.parameters, |queries| {
                        metadata.check_arguments(arguments, &frames.sorts, fuel, queries)
                    })
                    .map_err(type_descriptions::application_error)
                })?;
                self.check_normal_requirements(metadata, arguments, &frames.sorts, source)?;
                for ty in arguments {
                    self.admit_in_telescope(ty, depth + 1, frames)?;
                }
            }
            Term::MethodValue(method) => {
                self.admit_runtime_method_identity(method)?;
                for argument in method.terms() {
                    self.admit_in_telescope(argument, depth + 1, frames)?;
                }
            }
            Term::FunctionValue {
                declaration,
                arguments,
            } => {
                self.admit_callable_identity(*declaration, arguments, source)?;
                for argument in arguments {
                    self.admit_in_telescope(argument, depth + 1, frames)?;
                }
            }
            Term::Tuple(types) | Term::ClosureContextValue(types) | Term::TupleValue(types) => {
                for ty in types {
                    self.admit_in_telescope(ty, depth + 1, frames)?;
                }
            }
            Term::OptionalValue { ty, value } => {
                self.admit_in_telescope(ty, depth + 1, frames)?;
                if let Some(value) = value {
                    self.admit_in_telescope(value, depth + 1, frames)?;
                }
            }
            Term::ErrorValue { ty, label, value } => {
                self.admit_in_telescope(ty, depth + 1, frames)?;
                if let Some(label) = label {
                    self.admit_in_telescope(label, depth + 1, frames)?;
                }
                self.admit_in_telescope(value, depth + 1, frames)?;
            }
            Term::ListConcat { ty, parts: fields } | Term::NominalValue { ty, fields, .. } => {
                self.admit_in_telescope(ty, depth + 1, frames)?;
                for field in fields {
                    self.admit_in_telescope(field, depth + 1, frames)?;
                }
            }
            Term::Record(fields) | Term::RecordValue(fields) => {
                for (_, ty) in fields {
                    self.admit_in_telescope(ty, depth + 1, frames)?;
                }
            }
            Term::Optional(ty)
            | Term::Pointer(ty)
            | Term::ComptimeFunction(ty)
            | Term::NormalFormFunction(ty, _)
            | Term::UnsafeFunction(ty) => self.admit_in_telescope(ty, depth + 1, frames)?,
            Term::Function { parameters, result } => {
                frames.enter();
                for (index, p) in parameters.iter().enumerate() {
                    self.admit_in_telescope(&p.ty, depth + 1, frames)?;
                    frames.advance(parameters, index);
                }
                self.admit_in_telescope(result, depth + 1, frames)?;
                frames.leave();
            }
            Term::Closure {
                environment,
                signature,
            } => {
                for ty in environment {
                    self.admit_in_telescope(ty, depth + 1, frames)?;
                }
                self.admit_in_telescope(signature, depth + 1, frames)?;
            }
            Term::ContextualClosure { context, signature } => {
                self.admit_in_telescope(context, depth + 1, frames)?;
                self.admit_in_telescope(signature, depth + 1, frames)?;
            }
            Term::ErrorResult { errors, result } => {
                for label in errors {
                    self.admit_in_telescope(label, depth + 1, frames)?;
                }
                self.admit_in_telescope(result, depth + 1, frames)?;
            }
            Term::Computation { effects, result } => {
                // Row entries preserve declaration identity, order and repeated
                // occurrences. Formation does not install a handler or grant an
                // effect invocation capability.
                for operation in effects {
                    self.tick(None)?;
                    let (signature, contracts) = self.query_admission(None, |vm, fuel| {
                        let declaration = vm
                            .effect_declarations
                            .get(&operation.declaration)
                            .ok_or("unknown effect declaration")?;
                        vm.formation_inputs(
                            &declaration.type_parameters,
                            &declaration.parameter_types,
                            &operation.arguments,
                            frames,
                            fuel,
                            |inputs, fuel| {
                                let signature = inputs.instantiate(&declaration.signature, fuel)?;
                                let contracts = FormationContracts::instantiate(
                                    inputs,
                                    &declaration.requirements,
                                    &declaration.capabilities,
                                    fuel,
                                )?;
                                Ok((signature, contracts))
                            },
                        )
                    })?;
                    if signature.sort() != nir::Sort::Type {
                        return Err(self.failure(ErrorKind::Type, None));
                    }
                    self.query_descriptions(None, |vm, fuel| {
                        vm.inspect_scoped_term(&signature, &frames.parameters, fuel)
                    })?;
                    for predicate in contracts.requirements {
                        self.check_formation_predicate(
                            &predicate,
                            !frames.sorts.is_empty(),
                            source,
                        )?;
                    }
                    for capability in contracts.capabilities {
                        self.check_formation_predicate(
                            &nir::program::TypePredicate::Trait(capability),
                            !frames.sorts.is_empty(),
                            source,
                        )?;
                    }
                    for argument in &operation.arguments {
                        self.admit_in_telescope(argument, depth + 1, frames)?;
                    }
                }
                self.admit_in_telescope(result, depth + 1, frames)?;
            }
            Term::Bound { depth, index, sort } => {
                let bound = frames
                    .parameters
                    .iter()
                    .rev()
                    .nth(*depth as usize)
                    .and_then(|frame| frame.get(*index));
                if !bound.is_some_and(|p| p.sort == *sort) {
                    return Err(self.failure(ErrorKind::Type, None));
                }
            }
            Term::Rigid {
                declaration,
                sort: nir::Sort::Type,
            } if self.abstract_type_parameters.contains(declaration) => {}
            Term::Rigid {
                declaration,
                sort: nir::Sort::ComptimeValue,
            } if self
                .abstract_value_parameters
                .get(declaration)
                .is_some_and(|domain| !matches!(domain, Term::Lift(_))) => {}
            Term::Rigid {
                declaration,
                sort: nir::Sort::Runtime,
            } if self
                .abstract_value_parameters
                .get(declaration)
                .is_some_and(|domain| matches!(domain, Term::Lift(_))) => {}
            Term::Runtime(id) if self.runtime_domains().contains_key(id) => {}
            Term::AssociatedType { .. }
            | Term::Rigid { .. }
            | Term::Runtime(_)
            | Term::Value { .. }
            | Term::Hole { .. } => return Err(self.failure(ErrorKind::Type, None)),
            _ => {}
        }
        Ok(())
    }
}

struct FormationContracts {
    requirements: Vec<nir::program::TypePredicate>,
    capabilities: Vec<nir::program::TraitRequirement>,
}

impl FormationContracts {
    fn instantiate(
        inputs: &nir::AdmittedArguments<'_>,
        requirements: &[nir::program::TypePredicate],
        capabilities: &[nir::program::TraitRequirement],
        fuel: &mut usize,
    ) -> Result<Self, nir::NominalAdmissionError> {
        Ok(Self {
            requirements: requirements
                .iter()
                .map(|p| inputs.instantiate_predicate(p, fuel))
                .collect::<Result<_, _>>()?,
            capabilities: capabilities
                .iter()
                .map(|c| inputs.instantiate_requirement(c, fuel))
                .collect::<Result<_, _>>()?,
        })
    }
}
