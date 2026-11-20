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
        enum ResultSubject<'a> {
            Exact(nir::DeclarationSubstitution<'a>),
            Gradual(crate::description_evidence::DescriptionEvidence),
        }
        let ids = [binder];
        let domains = [Term::Primitive(Primitive::Type)];
        let exact = |vm: &mut Self, ty| {
            vm.query_admission(Some(source), |_, fuel| {
                Ok(ResultSubject::Exact(nir::DeclarationSubstitution::bounded(
                    &ids,
                    &domains,
                    std::slice::from_ref(ty),
                    &[],
                    fuel,
                )?))
            })
        };
        let rigid;
        let mut subject = match value {
            Value::Type(ty) => {
                self.check_type_description(ty, Some(source))?;
                exact(self, ty)?
            }
            Value::TypeParameter(id) if self.abstract_type_parameters.contains(id) => {
                rigid = Term::Rigid {
                    declaration: *id,
                    sort: nir::Sort::Type,
                };
                exact(self, &rigid)?
            }
            Value::Description(data) => {
                ResultSubject::Gradual(self.description_evidence(data, Some(binder), source)?)
            }
            _ => return Err(self.failure(ErrorKind::ProgramContractRejected, Some(source))),
        };
        for guarantee in guarantees {
            let mut predicate = if let Some(inputs) = substitution {
                self.query_admission(Some(source), |_, fuel| {
                    Ok(inputs.instantiate_predicate_bounded(guarantee, fuel)?)
                })?
            } else {
                self.charge_copy_work(Some(source), |fuel| {
                    nir::copy_budget::charge_predicate_substitution(guarantee, &[], &[], fuel)
                })?;
                guarantee.clone()
            };
            if let Some(scope) = type_scope {
                self.bind_invocation_contract(&mut predicate, scope, source, 0)?;
            }
            let established = match &mut subject {
                ResultSubject::Gradual(proof) => proof.entails(self, predicate, source)?,
                ResultSubject::Exact(result) => {
                    let goal = self.query_admission(Some(source), |_, fuel| {
                        Ok(result.instantiate_predicate_bounded(&predicate, fuel)?)
                    })?;
                    match goal.trivial_truth() {
                        Some(truth) => truth,
                        None => {
                            self.solve_type_predicate(&goal)
                                .map_err(|kind| self.failure(kind, Some(source)))?
                                == ContractOutcome::Satisfied
                        }
                    }
                }
            };
            if !established {
                return Err(self.failure(ErrorKind::ProgramContractRejected, Some(source)));
            }
        }
        Ok(())
    }
}
