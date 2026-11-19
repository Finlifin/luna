//! Entry, body and exit share one activation and one execution driver.
//! Preparing a condition specializes code; it never executes that condition.
use super::*;

pub(super) enum Progress<'a> {
    Entry(usize),
    NeedBody,
    Body {
        code: Body<'a>,
        cache_key: Option<CodeOrigin>,
    },
    Exit {
        next: usize,
        value: Value,
        returned: bool,
    },
}

impl Progress<'_> {
    pub(super) fn body(&self) -> Option<(&Body<'_>, Option<CodeOrigin>)> {
        match self {
            Self::Body { code, cache_key } => Some((code, *cache_key)),
            Self::Entry(_) | Self::NeedBody | Self::Exit { .. } => None,
        }
    }
}

impl fmt::Display for Progress<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entry(next) => write!(f, "(entry (next {next}))"),
            Self::NeedBody => f.write_str("(need-body)"),
            Self::Body { code, cache_key } => {
                f.write_str("(body (cache-key")?;
                if let Some(identity) = cache_key {
                    write!(f, " {identity}")?;
                }
                write!(f, ") {code})")
            }
            Self::Exit {
                next,
                returned,
                value,
            } => {
                write!(
                    f,
                    "(exit (next {next}) (returned {returned}) (result {value}))"
                )
            }
        }
    }
}

impl<'a> Interpreter<'a> {
    /// Select the next expression without leaving this invocation's scope.
    /// A completed exit returns its retained result; no body is replayed.
    pub(super) fn prepare_invocation_step(
        &mut self,
        frame: &mut InvocationFrame<'a>,
    ) -> Result<Option<Value>, Error> {
        frame.condition = None;
        let condition = match &frame.progress {
            Progress::Entry(index) => Some((true, *index)),
            Progress::Body { .. } | Progress::NeedBody => None,
            Progress::Exit { next, .. } if self.cleanup_schedule.is_none() => Some((false, *next)),
            Progress::Exit { .. } => None,
        };
        if let Some((entry, index)) = condition
            && let Some(definition) = frame.target.declaration()
        {
            let conditions = if entry {
                &definition.preconditions
            } else {
                &definition.postconditions
            };
            if let Some(condition) = conditions.get(index) {
                let mut condition =
                    self.copy_invocation_condition(condition, frame.target.substitution())?;
                if let Some(scope) = &frame.type_scope {
                    self.specialize_input_types(&mut condition, scope)?;
                }
                frame.condition = Some(Arc::new(Expr {
                    source: condition.source,
                    ty: Term::Primitive(Primitive::Void),
                    kind: K::CheckPredicate(Box::new(condition)),
                }));
                return Ok(None);
            }
        }
        match &mut frame.progress {
            Progress::Entry(_) => {
                frame.progress = Progress::NeedBody;
                Ok(None)
            }
            Progress::Body { .. } | Progress::NeedBody => Ok(None),
            Progress::Exit {
                value, returned, ..
            } => {
                if !*returned {
                    self.cleanup_site(
                        frame.source,
                        iir::cleanup::Boundary::FunctionEnd,
                        &mut frame.environment,
                    )?;
                }
                Ok(Some(std::mem::replace(value, Value::Unit)))
            }
        }
    }

    pub(super) fn install_invocation_body(
        &mut self,
        frame: &mut InvocationFrame<'a>,
    ) -> Result<(), Error> {
        let (code, cache_key) = self.prepare_invocation_body(frame)?;
        frame.progress = Progress::Body { code, cache_key };
        if let Some(definition) = frame.target.declaration() {
            self.events
                .push(Event::EnterFunction(definition.declaration));
        }
        Ok(())
    }

    pub(super) fn accept_invocation_step(
        &mut self,
        frame: &mut InvocationFrame<'a>,
        flow: Flow,
    ) -> Result<Option<Value>, Error> {
        let source = frame
            .condition
            .as_deref()
            .map_or(frame.source, |condition| condition.source);
        match &mut frame.progress {
            Progress::Entry(index) | Progress::Exit { next: index, .. } => {
                if !matches!(flow, Flow::Value(Value::Unit)) {
                    return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source)));
                }
                *index += 1;
            }
            Progress::NeedBody => unreachable!("awaiting code has no evaluated expression"),
            Progress::Body { .. } => {
                let (value, returned) = match flow {
                    Flow::Value(value) => (value, false),
                    Flow::Return(value) => (value, true),
                    _ => return Err(self.failure(ErrorKind::InvalidControlFlow, Some(source))),
                };
                let delivered_type = if let Term::Computation { result, .. } = &frame.result_type {
                    result.as_ref()
                } else {
                    &frame.result_type
                };
                self.check_value_type(&value, delivered_type, Some(source))?;
                if let Some(definition) = frame.target.declaration()
                    && !definition.result_guarantees.is_empty()
                {
                    let substitution = frame.target.substitution();
                    let type_scope = frame.type_scope.as_ref();
                    self.verify_result_guarantees(
                        &definition.result_guarantees,
                        definition.result_type_binder,
                        substitution,
                        type_scope,
                        &value,
                        source,
                    )?;
                }
                if self.cleanup_schedule.is_some() {
                    if !returned {
                        self.check_scheduled_result(&value, source, &mut frame.environment)?;
                    }
                } else if let Some(definition) = frame.target.declaration() {
                    if !definition.postconditions.is_empty()
                        && let Some(binding) = definition.result_binding
                    {
                        frame.environment.insert(binding, value.clone());
                    }
                }
                frame.progress = Progress::Exit {
                    next: 0,
                    value,
                    returned,
                };
            }
        }
        self.prepare_invocation_step(frame)
    }

    fn verify_result_guarantees(
        &mut self,
        guarantees: &[nir::program::TypePredicate],
        result_type_binder: Option<DeclarationId>,
        substitution: Option<&nir::DeclarationSubstitution<'_>>,
        type_scope: Option<&nir::FunctionInputBinding>,
        value: &Value,
        source: SourceRange,
    ) -> Result<(), Error> {
        let Some(binder) = result_type_binder else {
            return Err(self.failure(ErrorKind::ProgramContractRejected, Some(source)));
        };
        match value {
            Value::Type(actual_ty) => {
                self.check_type_description(actual_ty, Some(source))?;
                let binder_ids = [binder];
                let binder_domains = [Term::Primitive(Primitive::Type)];
                let actual_args = [actual_ty.clone()];
                let empty_frames: [Vec<nir::Sort>; 0] = [];
                for guarantee in guarantees {
                    let mut predicate = if let Some(substitution) = substitution {
                        self.query_admission(Some(source), |_, fuel| {
                            Ok(substitution.instantiate_predicate_bounded(guarantee, fuel)?)
                        })?
                    } else {
                        guarantee.clone()
                    };
                    if let Some(scope) = type_scope {
                        self.bind_invocation_contract(&mut predicate, scope, source, 0)?;
                    }
                    let instantiated = self.query_admission(Some(source), |_, fuel| {
                        let substitution = nir::DeclarationSubstitution::bounded(
                            &binder_ids,
                            &binder_domains,
                            &actual_args,
                            &empty_frames,
                            fuel,
                        )?;
                        Ok(substitution.instantiate_predicate_bounded(&predicate, fuel)?)
                    })?;
                    let outcome = match instantiated.trivial_truth() {
                        Some(true) => ContractOutcome::Satisfied,
                        Some(false) => ContractOutcome::Rejected,
                        None => self
                            .solve_type_predicate(&instantiated)
                            .map_err(|kind| self.failure(kind, Some(source)))?,
                    };
                    if outcome != ContractOutcome::Satisfied {
                        return Err(self.failure(ErrorKind::ProgramContractRejected, Some(source)));
                    }
                }
                Ok(())
            }
            Value::TypeParameter(id) if self.abstract_type_parameters.contains(id) => {
                let actual_ty = Term::Rigid {
                    declaration: *id,
                    sort: nir::Sort::Type,
                };
                let binder_ids = [binder];
                let binder_domains = [Term::Primitive(Primitive::Type)];
                let actual_args = [actual_ty];
                let empty_frames: [Vec<nir::Sort>; 0] = [];
                for guarantee in guarantees {
                    let mut predicate = if let Some(substitution) = substitution {
                        self.query_admission(Some(source), |_, fuel| {
                            Ok(substitution.instantiate_predicate_bounded(guarantee, fuel)?)
                        })?
                    } else {
                        guarantee.clone()
                    };
                    if let Some(scope) = type_scope {
                        self.bind_invocation_contract(&mut predicate, scope, source, 0)?;
                    }
                    let instantiated = self.query_admission(Some(source), |_, fuel| {
                        let substitution = nir::DeclarationSubstitution::bounded(
                            &binder_ids,
                            &binder_domains,
                            &actual_args,
                            &empty_frames,
                            fuel,
                        )?;
                        Ok(substitution.instantiate_predicate_bounded(&predicate, fuel)?)
                    })?;
                    let outcome = match instantiated.trivial_truth() {
                        Some(true) => ContractOutcome::Satisfied,
                        Some(false) => ContractOutcome::Rejected,
                        None => self
                            .solve_type_predicate(&instantiated)
                            .map_err(|kind| self.failure(kind, Some(source)))?,
                    };
                    if outcome != ContractOutcome::Satisfied {
                        return Err(self.failure(ErrorKind::ProgramContractRejected, Some(source)));
                    }
                }
                Ok(())
            }
            Value::Description(data) => {
                if !self.admits_description(data) {
                    return Err(self.failure(ErrorKind::ProgramContractRejected, Some(source)));
                }
                let Some(world) = self.declaration_world else {
                    return Err(self.failure(ErrorKind::ProgramContractRejected, Some(source)));
                };
                let subject_term = Term::Rigid {
                    declaration: binder,
                    sort: nir::Sort::Type,
                };
                let before = self.budget.steps;
                let subject = world
                    .refinements
                    .subject(data, &mut self.budget.steps)
                    .map_err(|_| {
                        self.failure(
                            if self.budget.steps == 0 && before > 0 {
                                ErrorKind::StepsExhausted
                            } else {
                                ErrorKind::Type
                            },
                            Some(source),
                        )
                    })?;
                let mut premises = Vec::new();
                if let Some(app) = subject.application
                    && self
                        .normal_forms
                        .get(&app.head())
                        .is_some_and(|m| m.role == nir::normal_forms::Role::Tuple)
                {
                    premises.push(nir::program::TypePredicate::Kind(
                        subject_term.clone(),
                        nir::TypeKind::Tuple,
                    ));
                }
                for req in subject.requirements() {
                    let instantiated = self.query_admission(Some(source), |_, fuel| {
                        Ok(req.instantiate(&subject_term, &[], fuel)?)
                    })?;
                    premises.push(instantiated);
                }
                for guarantee in guarantees {
                    let mut predicate = if let Some(substitution) = substitution {
                        self.query_admission(Some(source), |_, fuel| {
                            Ok(substitution.instantiate_predicate_bounded(guarantee, fuel)?)
                        })?
                    } else {
                        guarantee.clone()
                    };
                    if let Some(scope) = type_scope {
                        self.bind_invocation_contract(&mut predicate, scope, source, 0)?;
                    }
                    if premises.iter().any(|p| p == &predicate) {
                        continue;
                    }
                    let goal = if premises.is_empty() {
                        predicate
                    } else {
                        nir::program::TypePredicate::Or(vec![
                            nir::program::TypePredicate::Not(Box::new(
                                nir::program::TypePredicate::And(premises.clone()),
                            )),
                            predicate,
                        ])
                    };
                    let outcome = match goal.trivial_truth() {
                        Some(true) => ContractOutcome::Satisfied,
                        Some(false) => ContractOutcome::Rejected,
                        None => self
                            .solve_type_predicate(&goal)
                            .map_err(|kind| self.failure(kind, Some(source)))?,
                    };
                    if outcome != ContractOutcome::Satisfied {
                        return Err(self.failure(ErrorKind::ProgramContractRejected, Some(source)));
                    }
                }
                Ok(())
            }
            _ => Err(self.failure(ErrorKind::ProgramContractRejected, Some(source))),
        }
    }
}
