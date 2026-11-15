//! Entry, body and exit share one activation and one execution driver.
//! Preparing a condition specializes code; it never executes that condition.
use super::*;

pub(super) enum Progress<'a> {
    Entry(usize),
    Body {
        code: Body<'a>,
        cache_key: Option<DeclarationId>,
    },
    Exit {
        next: usize,
        value: Value,
        returned: bool,
    },
}

impl Progress<'_> {
    pub(super) fn body(&self) -> Option<(&Body<'_>, Option<DeclarationId>)> {
        match self {
            Self::Body { code, cache_key } => Some((code, *cache_key)),
            Self::Entry(_) | Self::Exit { .. } => None,
        }
    }
}

impl fmt::Display for Progress<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entry(next) => write!(f, "(entry (next {next}))"),
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
            Progress::Body { .. } => None,
            Progress::Exit { next, .. } if self.cleanup_schedule.is_none() => Some((false, *next)),
            Progress::Exit { .. } => None,
        };
        if let Some((entry, index)) = condition
            && let Some(origin) = frame.origin
        {
            let definition = self
                .invocation_definition(origin, &frame.function)
                .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
            let conditions = if entry {
                &definition.preconditions
            } else {
                &definition.postconditions
            };
            if let Some(condition) = conditions.get(index) {
                let mut condition =
                    self.copy_invocation_condition(condition, definition, &frame.function)?;
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
                let (code, cache_key) = self.prepare_invocation_body(frame)?;
                frame.progress = Progress::Body { code, cache_key };
                if let Some(origin) = frame.origin {
                    self.events.push(Event::EnterFunction(origin));
                }
                Ok(None)
            }
            Progress::Body { .. } => Ok(None),
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
                if self.cleanup_schedule.is_some() {
                    if !returned {
                        self.check_scheduled_result(&value, source, &mut frame.environment)?;
                    }
                } else if let Some(origin) = frame.origin {
                    let definition = self
                        .invocation_definition(origin, &frame.function)
                        .ok_or_else(|| self.failure(ErrorKind::UnknownFunction, None))?;
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
}
