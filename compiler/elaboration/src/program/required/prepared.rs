//! Checked source computations, separated from their invocation. This is an
//! owned preparation result, not a Type or an inference-variable snapshot.
use super::*;

pub(in crate::program) struct PreparedRequired {
    pub package: n::Package,
    pub implementation_index: crate::implementation::Index,
    pub entry: DeclarationId,
    pub inputs: Vec<(n::LocalId, Term, Option<Term>)>,
    pub result_type: Term,
    pub scoped_parameters: Vec<nir::DeclarationParameter>,
    pub scoped_arguments: Vec<Term>,
    pub input_frames: Vec<Vec<Sort>>,
    pub invocation_assumptions: Vec<n::TraitRequirement>,
    pub abstract_parameters: Vec<DeclarationId>,
    pub context: crate::ptc::Context,
    pub in_signature: bool,
}

impl PreparedRequired {
    pub(in crate::program) fn fork(&self, span: Span) -> Result<Self, Error> {
        Ok(Self {
            package: self.package.clone(),
            implementation_index: implementation_index(&self.package, span)?,
            entry: self.entry,
            inputs: self.inputs.clone(),
            result_type: self.result_type.clone(),
            scoped_parameters: self.scoped_parameters.clone(),
            scoped_arguments: self.scoped_arguments.clone(),
            input_frames: self.input_frames.clone(),
            invocation_assumptions: self.invocation_assumptions.clone(),
            abstract_parameters: self.abstract_parameters.clone(),
            context: self.context.clone(),
            in_signature: self.in_signature,
        })
    }

    /// Supply only captured, pending declaration inputs. Check the complete
    /// assignment before mutating the snapshot; rejected input routing leaves
    /// the preparation available for a corrected invocation.
    pub(super) fn bind_inputs(
        &mut self,
        values: Vec<(n::LocalId, Term)>,
        span: Span,
    ) -> Result<(), Error> {
        let mut assigned = BTreeSet::new();
        for (binding, _) in &values {
            if !assigned.insert(*binding) {
                return Err(error(span, "duplicate deferred comptime input"));
            }
            match self.inputs.iter().find(|(id, _, _)| id == binding) {
                Some((_, _, None)) => {}
                Some(_) => return Err(error(span, "deferred comptime input is already supplied")),
                None => return Err(error(span, "unknown deferred comptime input")),
            }
        }
        for (binding, value) in values {
            self.inputs
                .iter_mut()
                .find(|(id, _, _)| *id == binding)
                .unwrap()
                .2 = Some(value);
        }
        Ok(())
    }

    // Invocation uses only the checked snapshot and the caller's shared budget.
    // No current source-checker world participates in trait selection.
    pub(in crate::program) fn invoke(
        self,
        steps: &mut u64,
        span: Span,
    ) -> Result<(Value, Term), Error> {
        self.invoke_with_inputs(Vec::new(), steps, span)
    }

    pub(in crate::program) fn invoke_with_inputs(
        mut self,
        values: Vec<(n::LocalId, Term)>,
        steps: &mut u64,
        span: Span,
    ) -> Result<(Value, Term), Error> {
        self.bind_inputs(values, span)?;
        let PreparedRequired {
            package,
            implementation_index,
            entry,
            inputs,
            result_type,
            scoped_parameters,
            scoped_arguments,
            input_frames,
            invocation_assumptions,
            abstract_parameters,
            context,
            in_signature,
        } = self;
        // Pending source captures are not values. Reject before constructing an
        // interpreter, so an incomplete invocation cannot perform effects.
        let inputs = inputs.into_iter().map(|(binding, ty, value)| {
            value.map(|value| (binding, ty, value)).ok_or_else(|| error(span,
                "type computation depends on an unavailable function input; invocation-time type elaboration is not yet implemented"))
        }).collect::<Result<Vec<_>, _>>()?;
        let invocation_type_context = &context;
        let scoped_context = if scoped_parameters.is_empty() {
            None
        } else {
            let mut context = invocation_type_context.theory_in_scope(input_frames.clone());
            for assumption in invocation_type_context.assumptions() {
                context.assume(assumption.clone());
            }
            Some(context)
        };
        let type_solver = |predicate: &n::TypePredicate| {
            use comptime_interpretor::ContractOutcome;
            let restored;
            let (predicate, proof_context) = if scoped_parameters.is_empty() {
                (predicate, invocation_type_context)
            } else {
                let Ok(value) = predicate.instantiate_parameters_in_scope(
                    &scoped_parameters,
                    &scoped_arguments,
                    &input_frames,
                ) else {
                    return ContractOutcome::Unknown;
                };
                restored = value;
                (&restored, scoped_context.as_ref().unwrap())
            };
            let world = crate::capabilities::World {
                index: &implementation_index,
                implementations: &package.implementations,
                traits: &package.traits,
                assumptions: &invocation_assumptions,
                context: proof_context,
            };
            match world.check_predicate(predicate, crate::ptc::Budget::default()) {
                Ok(crate::ptc::Outcome::Entailed(_)) => ContractOutcome::Satisfied,
                Ok(crate::ptc::Outcome::NotEntailed(_)) => ContractOutcome::Rejected,
                _ => ContractOutcome::Unknown,
            }
        };
        if result_type == p(P::Type) {
            // Required Type computations use the same committed scheduler as
            // saved residual calls. Source capture has already excluded any
            // unavailable runtime input; no provisional Type is manufactured.
            use crate::type_computations::{Input, Session, State};
            let mut context = crate::InferenceContext::default();
            let mut session = Session::new_in_scope(
                &package,
                &context,
                Budget {
                    steps: *steps,
                    depth: 128,
                },
                &invocation_assumptions,
                &abstract_parameters,
                Some(&type_solver),
            )
            .map_err(|failure| {
                error(
                    span,
                    format!("required comptime preparation failed: {failure}"),
                )
            })?;
            let output = session
                .enqueue(
                    &mut context,
                    entry,
                    inputs
                        .iter()
                        .map(|(_, ty, value)| Input {
                            ty: crate::InferenceTerm::Stable(ty.clone()),
                            value: crate::InferenceTerm::Stable(value.clone()),
                        })
                        .collect(),
                )
                .map_err(|failure| {
                    error(
                        span,
                        format!("required type computation could not be queued: {failure}"),
                    )
                })?;
            let result = match session.resume(&mut context, output) {
                Ok(State::Complete(ty)) => Ok(Value::Type(ty.clone())),
                Ok(State::Failed(failure)) => {
                    Err(format!("required comptime evaluation failed: {failure}"))
                }
                Err(failure) => Err(format!("required comptime evaluation failed: {failure}")),
                Ok(State::Waiting(_)) => {
                    Err("required type computation has unresolved inputs".into())
                }
            };
            *steps = session.remaining_steps();
            let value = result.map_err(|message| error(span, message))?;
            let Value::Type(ty) = value else {
                unreachable!()
            };
            let ty = restore_required_type_input(
                steps,
                ty,
                &scoped_parameters,
                &scoped_arguments,
                &input_frames,
                span,
            )?;
            return Ok((Value::Type(ty), result_type));
        }
        let mut interpreter = Interpreter::for_comptime_with_assumptions(
            &package,
            &invocation_assumptions,
            Budget {
                steps: *steps,
                depth: 128,
            },
        )
        .map_err(|e| error(span, format!("required comptime preparation failed: {e}")))?;
        interpreter = interpreter.with_type_solver(&type_solver);
        interpreter = interpreter
            .with_abstract_type_parameters(&abstract_parameters)
            .map_err(|e| error(span, format!("required comptime type scope failed: {e}")))?;
        let value = if inputs.is_empty() {
            interpreter.invoke(entry, vec![])
        } else {
            interpreter.invoke_with_normal_arguments(
                entry,
                inputs
                    .iter()
                    .map(|(_, ty, value)| (ty.clone(), value.clone()))
                    .collect(),
            )
        };
        *steps = interpreter.remaining_steps();
        let value=value.map_err(|e| {
            if e.kind == comptime_interpretor::ErrorKind::MissingBinding
                && in_signature {
                return error(span,"type computation depends on an unavailable function input; invocation-time type elaboration is not yet implemented");
            }
            error(span,format!("required comptime evaluation failed: {e}"))
        })?;
        let value = restore_required_packet(
            steps,
            value,
            &scoped_parameters,
            &scoped_arguments,
            &input_frames,
            span,
            0,
        )?;
        Ok((value, result_type))
    }
}

impl std::fmt::Display for PreparedRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(prepared-required (entry {}) (result-type {}) (inputs",
            self.entry, self.result_type
        )?;
        for (binding, ty, value) in &self.inputs {
            write!(f, " (input {binding} {ty} ")?;
            match value {
                Some(value) => write!(f, "(available {value})")?,
                None => write!(f, "(pending)")?,
            }
            write!(f, ")")?;
        }
        write!(f, ") (scope-substitution")?;
        for (parameter, argument) in self.scoped_parameters.iter().zip(&self.scoped_arguments) {
            write!(
                f,
                " (binding {} {} {argument})",
                parameter.declaration, parameter.sort
            )?;
        }
        write!(f, ") (input-frames")?;
        for frame in &self.input_frames {
            write!(f, " (frame")?;
            for sort in frame {
                write!(f, " {sort}")?;
            }
            write!(f, ")")?;
        }
        write!(f, ") (assumptions")?;
        for assumption in &self.invocation_assumptions {
            write!(f, " {assumption}")?;
        }
        write!(f, ") (abstract-parameters")?;
        for parameter in &self.abstract_parameters {
            write!(f, " {parameter}")?;
        }
        write!(
            f,
            ") (in-signature {}) (context {}) (implementation-index {}) (program {}))",
            self.in_signature, self.context, self.implementation_index, self.package
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literal_preparation(type_result: bool) -> PreparedRequired {
        let entry = DeclarationId {
            package: 7,
            local: 11,
        };
        let result_type = p(if type_result { P::Type } else { P::Bool });
        let body = n::Expr {
            ty: result_type.clone(),
            source: n::SourceRange { start: 0, end: 1 },
            kind: if type_result {
                K::TypeValue(p(P::U8))
            } else {
                K::Literal(n::Literal::Bool(true))
            },
        };
        let package = n::Package {
            functions: vec![n::Function {
                phase: n::FunctionPhase::Comptime,
                inline: false,
                pure: true,
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                entry_snapshots: vec![],
                capabilities: vec![],
                requirements: vec![],
                type_parameters: vec![],
                declaration: entry,
                name: "prepared".into(),
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(result_type.clone()),
                },
                parameters: vec![],
                body,
                defaults: vec![],
            }],
            ..Default::default()
        };
        let implementation_index = implementation_index(&package, Span::default()).unwrap();
        PreparedRequired {
            package,
            implementation_index,
            entry,
            inputs: vec![],
            result_type,
            scoped_parameters: vec![],
            scoped_arguments: vec![],
            input_frames: vec![],
            invocation_assumptions: vec![],
            abstract_parameters: vec![],
            context: crate::ptc::Context::default(),
            in_signature: false,
        }
    }

    #[test]
    fn prepared_invocation_needs_no_source_checker_and_charges_both_execution_paths() {
        for type_result in [false, true] {
            let prepared = literal_preparation(type_result);
            let mut steps = 1000;
            let (value, ty) = prepared.invoke(&mut steps, Span::default()).unwrap();
            assert!(steps < 1000);
            if type_result {
                assert!(matches!(value, Value::Type(ref value) if *value == p(P::U8)));
                assert_eq!(ty, p(P::Type));
            } else {
                assert!(matches!(value, Value::Bool(true)));
                assert_eq!(ty, p(P::Bool));
            }
            let mut exhausted = 0;
            assert!(
                literal_preparation(type_result)
                    .invoke(&mut exhausted, Span::default())
                    .is_err()
            );
            assert_eq!(exhausted, 0);
        }
    }

    fn pending_boolean_preparation() -> (PreparedRequired, n::LocalId) {
        let mut prepared = literal_preparation(false);
        let binding = n::LocalId {
            owner: 11,
            index: 0,
        };
        let function = &mut prepared.package.functions[0];
        function.parameters = vec![binding];
        function.signature = Term::Function {
            parameters: vec![Parameter {
                sort: Sort::ComptimeValue,
                ty: p(P::Bool),
            }],
            result: Box::new(p(P::Bool)),
        };
        function.body.kind = K::Local(binding);
        prepared.inputs = vec![(binding, p(P::Bool), None)];
        (prepared, binding)
    }

    #[test]
    fn pending_source_inputs_bind_atomically_before_invocation() {
        let span = Span::default();
        let (pending, _) = pending_boolean_preparation();
        let mut steps = 1000;
        assert!(
            pending
                .invoke(&mut steps, span)
                .unwrap_err()
                .message
                .contains("unavailable function input")
        );
        assert_eq!(steps, 1000, "an unavailable input must not start execution");
        let (mut pending, binding) = pending_boolean_preparation();
        assert!(pending.to_string().contains("(pending)"));
        assert!(
            pending
                .bind_inputs(
                    vec![(binding, Term::Bool(true)), (binding, Term::Bool(false))],
                    span
                )
                .is_err()
        );
        assert!(pending.inputs[0].2.is_none());
        let foreign = n::LocalId {
            owner: binding.owner + 1,
            ..binding
        };
        assert!(
            pending
                .bind_inputs(
                    vec![(binding, Term::Bool(true)), (foreign, Term::Bool(false))],
                    span
                )
                .is_err()
        );
        assert!(pending.inputs[0].2.is_none());
        pending
            .bind_inputs(vec![(binding, Term::Bool(false))], span)
            .unwrap();
        assert!(
            pending
                .bind_inputs(vec![(binding, Term::Bool(true))], span)
                .is_err()
        );
        assert!(
            matches!(pending.invoke(&mut steps, span).unwrap(), (Value::Bool(false), ty) if ty == p(P::Bool))
        );
        assert!(steps < 1000);
        let (mut pending, binding) = pending_boolean_preparation();
        pending
            .bind_inputs(vec![(binding, p(P::U8))], span)
            .unwrap();
        assert!(
            pending.invoke(&mut 1000, span).is_err(),
            "binding a value does not bypass input domain checking"
        );
    }

    #[test]
    fn pending_type_capture_is_specialized_per_invocation() {
        let span = Span::default();
        for ty in [p(P::U8), p(P::Bool)] {
            let mut prepared = literal_preparation(true);
            let binding = n::LocalId {
                owner: 11,
                index: 0,
            };
            let function = &mut prepared.package.functions[0];
            function.parameters = vec![binding];
            function.signature = Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Type,
                    ty: p(P::Type),
                }],
                result: Box::new(p(P::Type)),
            };
            function.body.kind = K::Local(binding);
            prepared.inputs = vec![(binding, p(P::Type), None)];
            let mut steps = 1000;
            let (value, result) = prepared
                .invoke_with_inputs(vec![(binding, ty.clone())], &mut steps, span)
                .unwrap();
            assert!(matches!(value, Value::Type(value) if value == ty));
            assert_eq!(result, p(P::Type));
            assert!(steps < 1000);
        }
    }

    #[test]
    fn prepared_required_dump_preserves_scope_and_escaped_inputs() {
        let declaration = DeclarationId {
            package: 7,
            local: 11,
        };
        let tuple = Term::NormalForm {
            head: declaration,
            sort: Sort::Type,
            arguments: vec![Term::TypeListValue(vec![p(P::Bool), p(P::U8)])],
        };
        let prepared = PreparedRequired {
            package: n::Package::default(),
            implementation_index: crate::implementation::Index::default(),
            entry: declaration,
            inputs: vec![(
                n::LocalId {
                    owner: 11,
                    index: 0,
                },
                p(P::Str),
                Some(Term::String("quoted \"value\"\\\n(parentheses)".into())),
            )],
            result_type: p(P::Type),
            scoped_parameters: vec![nir::DeclarationParameter {
                declaration,
                sort: Sort::Type,
            }],
            scoped_arguments: vec![tuple.clone()],
            input_frames: vec![vec![Sort::Type, Sort::ComptimeValue]],
            invocation_assumptions: vec![],
            abstract_parameters: vec![declaration],
            context: crate::ptc::Context::default(),
            in_signature: true,
        };
        let dump = prepared.to_string();
        assert_eq!(dump, prepared.to_string());
        assert!(dump.contains(&format!("(binding {declaration} {} {tuple})", Sort::Type)));
        assert!(dump.contains("(in-signature true)"));
        assert!(dump.contains("\\\"value\\\""));
        assert!(dump.contains("\\n(parentheses)"));
        // Count structural parentheses only outside strings, respecting escapes.
        let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
        for ch in dump.chars() {
            if quoted {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    quoted = false;
                }
            } else {
                match ch {
                    '"' => quoted = true,
                    '(' => depth += 1,
                    ')' => depth = depth.checked_sub(1).expect("unmatched closing parenthesis"),
                    _ => {}
                }
            }
        }
        assert_eq!(depth, 0);
        assert!(!quoted && !escaped);
    }
}

fn restore_required_packet(
    steps: &mut u64,
    value: Value,
    parameters: &[nir::DeclarationParameter],
    arguments: &[Term],
    frames: &[Vec<Sort>],
    span: Span,
    depth: usize,
) -> Result<Value, Error> {
    if depth >= 128 {
        return Err(error(span, "required type packet nesting limit exceeded"));
    }
    *steps = steps
        .checked_sub(1)
        .ok_or_else(|| error(span, "required type packet budget exhausted"))?;
    Ok(match value {
        Value::TypeParameter(declaration) => Value::Type(restore_required_type_input(
            steps,
            Term::Rigid {
                declaration,
                sort: Sort::Type,
            },
            parameters,
            arguments,
            frames,
            span,
        )?),
        Value::Type(ty) => Value::Type(restore_required_type_input(
            steps, ty, parameters, arguments, frames, span,
        )?),
        Value::TypeList(types) => Value::TypeList(
            types
                .into_iter()
                .map(|ty| {
                    restore_required_type_input(steps, ty, parameters, arguments, frames, span)
                })
                .collect::<Result<_, _>>()?,
        ),
        Value::Tuple(values) => Value::Tuple(
            values
                .into_iter()
                .map(|value| {
                    restore_required_packet(
                        steps,
                        value,
                        parameters,
                        arguments,
                        frames,
                        span,
                        depth + 1,
                    )
                })
                .collect::<Result<_, _>>()?,
        ),
        value => value,
    })
}

fn restore_required_type_input(
    steps: &mut u64,
    ty: Term,
    parameters: &[nir::DeclarationParameter],
    arguments: &[Term],
    frames: &[Vec<Sort>],
    span: Span,
) -> Result<Term, Error> {
    if parameters.is_empty() {
        return Ok(ty);
    }
    let mut fuel = usize::try_from(*steps).unwrap_or(usize::MAX);
    let before = fuel;
    let bounded = (|| {
        nir::type_computations::charge_term(&ty, &mut fuel)?;
        fuel = fuel
            .checked_sub(parameters.len())
            .ok_or(nir::type_computations::Error::Budget)?;
        Ok::<_, nir::type_computations::Error>(())
    })();
    *steps -= (before - fuel) as u64;
    bounded.map_err(|e| error(span, e.to_string()))?;
    ty.instantiate_parameters_in_scope(parameters, arguments, frames)
        .map_err(|e| error(span, e.to_string()))
}

pub(super) fn implementation_index(
    package: &n::Package,
    span: Span,
) -> Result<crate::implementation::Index, Error> {
    let mut index = crate::implementation::Index::default();
    for implementation in &package.implementations {
        if implementation.interface.is_none() {
            continue;
        }
        index
            .insert(
                crate::implementation::ImplementationHead {
                    declaration: implementation.declaration,
                    interface: implementation.interface,
                    interface_arguments: implementation.interface_arguments.clone(),
                    subject: implementation.subject.clone(),
                    type_parameters: implementation.type_parameters.clone(),
                    requirements: implementation.requirements.clone(),
                },
                crate::ptc::Budget::default(),
            )
            .map_err(|failure| {
                error(
                    span,
                    format!("required comptime implementation index: {failure:?}"),
                )
            })?;
    }
    Ok(index)
}
