//! Checked source computations, separated from their invocation. This is an
//! owned preparation result, not a Type or an inference-variable snapshot.
use super::restore::{restore_required_packet, restore_required_type_input};
use super::*;
use comptime_interpretor::InvocationValue;
use std::sync::Arc;

/// Own the declarations and their derived index together. Once published into
/// a preparation, this world is immutable; invocation state lives elsewhere.
pub(super) struct RequiredWorld {
    package: n::Package,
    implementation_index: crate::implementation::Index,
}
impl RequiredWorld {
    pub(super) fn new(package: n::Package, span: Span) -> Result<Arc<Self>, Error> {
        let implementation_index = implementation_index(&package, span)?;
        Ok(Arc::new(Self {
            package,
            implementation_index,
        }))
    }
}

pub(in crate::program) struct PreparedRequired {
    pub(super) world: Arc<RequiredWorld>,
    pub entry: DeclarationId,
    pub inputs: Vec<(n::LocalId, Term, Option<Term>)>,
    pub result_type: Term,
    pub scoped_parameters: Vec<nir::DeclarationParameter>,
    pub scoped_arguments: Vec<Term>,
    pub input_frames: Vec<Vec<Sort>>,
    pub invocation_assumptions: Vec<n::TraitRequirement>,
    pub abstract_parameters: Vec<DeclarationId>,
    pub abstract_value_parameters: Vec<(DeclarationId, Term)>,
    pub context: crate::ptc::Context,
    pub in_signature: bool,
    pub copy_authority: Option<(DeclarationId, DeclarationId, std::sync::Arc<n::Package>)>,
}

impl PreparedRequired {
    pub(in crate::program) fn definition_identity_cursor(&self) -> u32 {
        self.world.package.definition_identity_cursor
    }

    /// Observe the carried invocation inputs and code without consulting a
    /// current Checker, copying the snapshot, or running its computations.
    /// Ambient abstract binders authorize transport; they are not input uses.
    pub(in crate::program) fn uses_declaration_inputs(
        &self,
        declarations: &[DeclarationId],
        steps: &mut u64,
        span: Span,
    ) -> Result<bool, Error> {
        if declarations.is_empty() {
            return Ok(false);
        }
        *steps = steps
            .checked_sub(declarations.len() as u64)
            .ok_or_else(|| error(span, "prepared input selection budget exhausted"))?;
        let declarations = declarations.iter().copied().collect::<BTreeSet<_>>();
        let terms = self
            .inputs
            .iter()
            .flat_map(|(_, domain, value)| std::iter::once(domain).chain(value))
            .chain(std::iter::once(&self.result_type))
            .chain(&self.scoped_arguments)
            .chain(self.invocation_assumptions.iter().flat_map(|requirement| {
                std::iter::once(&requirement.subject).chain(&requirement.interface_arguments)
            }));
        for term in terms {
            if !input_observation::term(term, steps, span)?.is_disjoint(&declarations) {
                return Ok(true);
            }
        }
        // The preparation has already selected its callable closure. Inspect
        // the contracts/defaults carried with those functions as well as their
        // signatures and bodies; do not scan the surrounding declaration world.
        for function in &self.world.package.functions {
            if !input_observation::function(function, steps, span)?.is_disjoint(&declarations) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(in crate::program) fn fork(&self) -> Self {
        Self {
            world: Arc::clone(&self.world),
            entry: self.entry,
            inputs: self.inputs.clone(),
            result_type: self.result_type.clone(),
            scoped_parameters: self.scoped_parameters.clone(),
            scoped_arguments: self.scoped_arguments.clone(),
            input_frames: self.input_frames.clone(),
            invocation_assumptions: self.invocation_assumptions.clone(),
            abstract_parameters: self.abstract_parameters.clone(),
            abstract_value_parameters: self.abstract_value_parameters.clone(),
            context: self.context.clone(),
            in_signature: self.in_signature,
            copy_authority: self.copy_authority.clone(),
        }
    }

    /// Supply only captured, pending declaration inputs. Check the complete
    /// assignment before mutating the snapshot; rejected input routing leaves
    /// the preparation available for a corrected invocation.
    pub(in crate::program) fn bind_inputs(
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
        self.invoke_with_live_inputs(Vec::new(), steps, span)
    }

    /// Supply invocation-local values without exporting or rebuilding their
    /// graphs. Frozen captures remain normal inputs; only pending captures may
    /// be supplied. The preparation itself never retains a live graph.
    pub(in crate::program) fn invoke_with_live_inputs(
        self,
        values: Vec<(n::LocalId, Value)>,
        steps: &mut u64,
        span: Span,
    ) -> Result<(Value, Term), Error> {
        self.invoke_in_world(values, steps, span, None)
    }

    pub(in crate::program) fn invoke_publishing(
        self,
        values: Vec<(n::LocalId, Value)>,
        steps: &mut u64,
        span: Span,
        package_identity: u64,
        next_identity: &mut u32,
    ) -> Result<((Value, Term), crate::definitions::PublishedDefinitions), Error> {
        let world = Arc::clone(&self.world);
        let publisher = crate::definitions::AllocatedDefinitionPublisher::new(
            &world.package,
            package_identity,
            *next_identity,
            usize::MAX,
        );
        let result = self.invoke_in_world(values, steps, span, Some(&publisher));
        let diagnostic = publisher.last_diagnostic();
        let (definitions, next) = publisher.finish();
        *next_identity = next;
        let result = result.map_err(|failure| match diagnostic {
            Some(message) => error(span, format!("{failure}; {message}")),
            None => failure,
        })?;
        Ok((result, definitions))
    }

    fn invoke_in_world(
        self,
        values: Vec<(n::LocalId, Value)>,
        steps: &mut u64,
        span: Span,
        builder: Option<&dyn crate::definitions::StructBuildService>,
    ) -> Result<(Value, Term), Error> {
        let world = Arc::clone(&self.world);
        let package = &world.package;
        let implementation_index = &world.implementation_index;
        let build = |request: &comptime_interpretor::StructBuildRequest, fuel: &mut u64| {
            builder
                .expect("configured builder service")
                .prepare(request, fuel)
        };
        let build_newtype = |request: &comptime_interpretor::NewtypeBuildRequest,
                             fuel: &mut u64| {
            builder
                .expect("configured builder service")
                .prepare_newtype(request, fuel)
        };
        let build_enum = |request: &comptime_interpretor::EnumBuildRequest, fuel: &mut u64| {
            builder
                .expect("configured builder service")
                .prepare_enum(request, fuel)
        };
        let mut live = BTreeMap::new();
        for (binding, value) in values {
            match self.inputs.iter().find(|(id, _, _)| *id == binding) {
                Some((_, _, None)) => {}
                Some(_) => return Err(error(span, "deferred comptime input is already supplied")),
                None => return Err(error(span, "unknown deferred comptime input")),
            }
            if live.insert(binding, value).is_some() {
                return Err(error(span, "duplicate deferred comptime input"));
            }
        }
        let PreparedRequired {
            world: _,
            entry,
            inputs,
            result_type,
            scoped_parameters,
            scoped_arguments,
            input_frames,
            invocation_assumptions,
            abstract_parameters,
            abstract_value_parameters,
            context,
            in_signature,
            copy_authority,
        } = self;
        // Pending source captures are not values. Reject before constructing an
        // interpreter, so an incomplete invocation cannot perform effects.
        let inputs = inputs.into_iter().map(|(binding, ty, value)| {
            let value = match value {
                Some(term) => Some(InvocationValue::Normal(term)),
                None => live.remove(&binding).map(InvocationValue::Live),
            };
            value.map(|value| (ty, value)).ok_or_else(|| error(span,
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
        let type_solver = |predicate: &n::TypePredicate, remaining: &mut u64| {
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
                index: implementation_index,
                implementations: &package.implementations,
                traits: &package.traits,
                assumptions: &invocation_assumptions,
                context: proof_context,
            };
            let mut proof_budget = crate::ptc::Budget {
                steps: crate::ptc::Budget::default()
                    .steps
                    .min(usize::try_from(*remaining).unwrap_or(usize::MAX)),
                ..Default::default()
            };
            let before = proof_budget.steps;
            let outcome = world.check_predicate_metered(predicate, &mut proof_budget);
            *remaining = remaining.saturating_sub((before - proof_budget.steps) as u64);
            match outcome {
                Ok(crate::ptc::Outcome::Entailed(_)) => ContractOutcome::Satisfied,
                Ok(crate::ptc::Outcome::NotEntailed(_)) => ContractOutcome::Rejected,
                _ => ContractOutcome::Unknown,
            }
        };
        let copy_failure = std::cell::RefCell::new(None::<String>);
        let pointer_copy = |ty: &Term, remaining: &mut u64| {
            let Some((copy, drop, resources)) = &copy_authority else {
                *copy_failure.borrow_mut() = Some(
                    "aggregate pointer transfer requires bound Copy and Drop protocols".into(),
                );
                return false;
            };
            let mut fuel = usize::try_from(*remaining).unwrap_or(usize::MAX);
            let before = fuel;
            let world = crate::capabilities::World {
                index: implementation_index,
                implementations: &resources.implementations,
                traits: &resources.traits,
                assumptions: &invocation_assumptions,
                context: invocation_type_context,
            };
            let evidence = crate::verification::copy::acquire(
                resources,
                &world,
                *copy,
                *drop,
                ty,
                crate::ptc::Budget {
                    steps: fuel,
                    ..Default::default()
                },
                &mut fuel,
            );
            *remaining = remaining.saturating_sub((before - fuel) as u64);
            let allowed = evidence.is_ok();
            *copy_failure.borrow_mut() = evidence.err();
            allowed
        };
        let with_copy_failure = |message: String| match copy_failure.borrow().as_ref() {
            Some(reason) => format!("{message}; pointer Copy evidence: {reason}"),
            None => message,
        };
        let associated_resolver = |access: &n::Expr, remaining: &mut u64| {
            let world = crate::capabilities::World {
                index: implementation_index,
                implementations: &package.implementations,
                traits: &package.traits,
                assumptions: &invocation_assumptions,
                context: invocation_type_context,
            };
            crate::materialization::resolve_associated_value_for_invocation(
                &world, access, remaining,
            )
        };
        let method_resolver = |callee: &n::Expr, remaining: &mut u64| {
            let mut fuel = usize::try_from(*remaining).unwrap_or(usize::MAX);
            let before = fuel;
            let result = crate::materialization::resolve_method_identity_in_context(
                &package,
                callee,
                invocation_type_context,
                &invocation_assumptions,
                &mut fuel,
            );
            *remaining -= (before - fuel) as u64;
            result.map_err(|_| comptime_interpretor::Error {
                kind: comptime_interpretor::ErrorKind::UnmaterializedDispatch,
                source: Some(callee.source),
            })
        };
        if result_type == p(P::Type) {
            // Required Type computations use the same committed scheduler as
            // saved residual calls. Source capture has already excluded any
            // unavailable runtime input; no provisional Type is manufactured.
            use crate::type_computations::{PreparedInput, Session, State};
            let mut context = crate::InferenceContext::default();
            let mut budget = Budget {
                steps: *steps,
                depth: 128,
            };
            let session = Session::new_in_scope_with_pointer_copy(
                &package,
                &context,
                &mut budget,
                &invocation_assumptions,
                &abstract_parameters,
                &abstract_value_parameters,
                Some(crate::type_computations::ProofSolver::Budgeted(
                    &type_solver,
                )),
                Some(&pointer_copy),
            );
            *steps = budget.steps;
            let mut session = session.map_err(|failure| {
                error(
                    span,
                    format!("required comptime preparation failed: {failure}"),
                )
            })?;
            session = session
                .with_method_resolver(&method_resolver)
                .with_associated_resolver(&associated_resolver);
            if builder.is_some() {
                session = session
                    .with_struct_builder(&build)
                    .with_enum_builder(&build_enum)
                    .with_newtype_builder(&build_newtype);
            }
            let output = session
                .enqueue_prepared(
                    &mut context,
                    entry,
                    inputs
                        .into_iter()
                        .map(|(ty, value)| PreparedInput {
                            ty: crate::InferenceTerm::Stable(ty),
                            value,
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
            let value = result.map_err(|message| error(span, with_copy_failure(message)))?;
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
        let mut preparation_budget = Budget {
            steps: std::mem::take(steps),
            depth: 128,
        };
        let prepared = Interpreter::for_comptime_in_budget(
            &package,
            &invocation_assumptions,
            &mut preparation_budget,
        );
        *steps = preparation_budget.steps;
        let mut interpreter = prepared
            .map_err(|e| error(span, format!("required comptime preparation failed: {e}")))?;
        if builder.is_some() {
            interpreter = interpreter
                .with_struct_builder(&build)
                .with_enum_builder(&build_enum)
                .with_newtype_builder(&build_newtype);
        }
        interpreter = interpreter
            .with_method_resolver(&method_resolver)
            .with_associated_resolver(&associated_resolver);
        interpreter = interpreter.with_budgeted_type_solver(&type_solver);
        interpreter = interpreter.with_budgeted_pointer_copy_permission(&pointer_copy);
        let scope = interpreter
            .authorize_abstract_type_parameters(&abstract_parameters)
            .and_then(|()| {
                interpreter.authorize_abstract_value_parameters(&abstract_value_parameters)
            });
        *steps = interpreter.remaining_steps();
        scope.map_err(|e| error(span, format!("required comptime scope failed: {e}")))?;
        let value = interpreter.invoke_with_prepared_arguments(entry, inputs);
        *steps = interpreter.remaining_steps();
        let value=value.map_err(|e| {
            if e.kind == comptime_interpretor::ErrorKind::MissingBinding
                && in_signature {
                return error(span,"type computation depends on an unavailable function input; invocation-time type elaboration is not yet implemented");
            }
            error(span,with_copy_failure(format!("required comptime evaluation failed: {e}")))
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
        write!(f, ") (abstract-value-parameters")?;
        for (id, domain) in &self.abstract_value_parameters {
            write!(f, " ({id} {domain})")?;
        }
        write!(
            f,
            ") (in-signature {}) (context {}) (implementation-index {}) (program {})",
            self.in_signature, self.context, self.world.implementation_index, self.world.package
        )?;
        if let Some((copy, drop, package)) = &self.copy_authority {
            write!(f, " (copy-authority (copy {copy}) (drop {drop}) {package})")?;
        }
        write!(f, ")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl PreparedRequired {
        fn package_mut(&mut self) -> &mut n::Package {
            &mut Arc::get_mut(&mut self.world)
                .expect("unique test fixture")
                .package
        }
    }

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
                is_unsafe: false,
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
        PreparedRequired {
            world: RequiredWorld::new(package, Span::default()).unwrap(),
            entry,
            inputs: vec![],
            result_type,
            scoped_parameters: vec![],
            scoped_arguments: vec![],
            input_frames: vec![],
            invocation_assumptions: vec![],
            abstract_parameters: vec![],
            abstract_value_parameters: vec![],
            context: crate::ptc::Context::default(),
            in_signature: false,
            copy_authority: None,
        }
    }

    #[test]
    fn prepared_forks_share_declarations_and_isolate_input_assignments() {
        let span = Span::default();
        let mut prepared = literal_preparation(false);
        let binding = n::LocalId {
            owner: 11,
            index: 0,
        };
        let function = &mut prepared.package_mut().functions[0];
        function.parameters = vec![binding];
        function.signature = Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::ComptimeValue,
                ty: p(P::Bool),
            }],
            result: Box::new(p(P::Bool)),
        };
        function.body.kind = K::Local(binding);
        prepared.inputs = vec![(binding, p(P::Bool), None)];
        let owner = Arc::downgrade(&prepared.world);
        let mut first = prepared.fork();
        let mut second = prepared.fork();
        assert!(Arc::ptr_eq(&prepared.world, &first.world));
        assert!(std::ptr::eq(
            &first.world.implementation_index,
            &second.world.implementation_index
        ));
        assert!(
            first
                .bind_inputs(
                    vec![(binding, Term::Bool(true)), (binding, Term::Bool(false))],
                    span
                )
                .is_err()
        );
        assert_eq!(first.inputs, prepared.inputs);
        first
            .bind_inputs(vec![(binding, Term::Bool(true))], span)
            .unwrap();
        assert!(prepared.inputs[0].2.is_none());
        assert!(second.inputs[0].2.is_none());
        second
            .bind_inputs(vec![(binding, Term::Bool(false))], span)
            .unwrap();
        drop(prepared);
        assert!(owner.upgrade().is_some());
        assert_eq!(
            first.invoke(&mut 100_000, span).unwrap().0,
            Value::Bool(true)
        );
        assert!(owner.upgrade().is_some());
        assert_eq!(
            second.invoke(&mut 100_000, span).unwrap().0,
            Value::Bool(false)
        );
        assert!(owner.upgrade().is_none());
    }

    #[test]
    fn prepared_worlds_keep_same_numbered_declarations_separate() {
        let span = Span::default();
        let first = literal_preparation(false);
        let mut second = literal_preparation(false);
        second.package_mut().functions[0].body.kind = K::Literal(n::Literal::Bool(false));
        assert_eq!(first.entry, second.entry);
        assert!(!Arc::ptr_eq(&first.world, &second.world));
        for _ in 0..3 {
            assert_eq!(
                first.fork().invoke(&mut 100_000, span).unwrap().0,
                Value::Bool(true)
            );
            assert_eq!(
                second.fork().invoke(&mut 100_000, span).unwrap().0,
                Value::Bool(false)
            );
        }
    }

    fn frozen_types(fields: Vec<nir::Term>) -> nir::Term {
        nir::Term::NominalValue {
            ty: Box::new(nir::Term::NormalForm {
                head: nir::DeclarationId {
                    package: 7,
                    local: 12,
                },
                sort: nir::Sort::Type,
                arguments: vec![nir::Term::Primitive(nir::Primitive::Type)],
            }),
            variant: None,
            fields,
        }
    }

    #[test]
    fn prepared_input_observation_keeps_carried_contract_and_operand_dependencies() {
        let input = DeclarationId {
            package: 7,
            local: 90,
        };
        let rigid = Term::Rigid {
            declaration: input,
            sort: Sort::Type,
        };
        let placements: &[fn(&mut PreparedRequired, Term)] = &[
            |p, term| p.result_type = term,
            |p, term| {
                p.inputs.push((
                    n::LocalId {
                        owner: 11,
                        index: 0,
                    },
                    term,
                    None,
                ))
            },
            |p, term| {
                p.inputs.push((
                    n::LocalId {
                        owner: 11,
                        index: 0,
                    },
                    p.result_type.clone(),
                    Some(term),
                ))
            },
            |p, term| p.scoped_arguments.push(term),
            |p, term| {
                p.invocation_assumptions.push(n::TraitRequirement {
                    subject: term,
                    interface: p.entry,
                    interface_arguments: vec![],
                })
            },
            |p, term| {
                p.invocation_assumptions.push(n::TraitRequirement {
                    subject: p.result_type.clone(),
                    interface: p.entry,
                    interface_arguments: vec![term],
                })
            },
            |p, term| p.package_mut().functions[0].signature = term,
            |p, term| p.package_mut().functions[0].body.kind = K::StagedValue(term),
            |p, term| {
                p.package_mut().functions[0]
                    .requirements
                    .push(n::TypePredicate::Not(Box::new(n::TypePredicate::Equal(
                        term,
                        Term::Primitive(P::Bool),
                    ))))
            },
            |p, term| {
                let entry = p.entry;
                p.package_mut().functions[0]
                    .capabilities
                    .push(n::TraitRequirement {
                        subject: term,
                        interface: entry,
                        interface_arguments: vec![],
                    })
            },
            |p, term| {
                let mut predicate = p.package_mut().functions[0].body.clone();
                predicate.kind = K::TypeValue(term);
                p.package_mut().functions[0].preconditions.push(predicate);
            },
            |p, term| {
                let mut predicate = p.package_mut().functions[0].body.clone();
                predicate.kind = K::TypeValue(term);
                p.package_mut().functions[0].postconditions.push(predicate);
            },
            |p, term| {
                let mut default = p.package_mut().functions[0].body.clone();
                default.kind = K::TypeValue(term);
                p.package_mut().functions[0].defaults.push((0, default));
            },
        ];
        for (index, place) in placements.iter().enumerate() {
            let mut prepared = literal_preparation(true);
            assert!(
                !prepared
                    .uses_declaration_inputs(&[input], &mut 1000, Span::default())
                    .unwrap()
            );
            place(&mut prepared, rigid.clone());
            let before = prepared.to_string();
            assert!(
                prepared
                    .uses_declaration_inputs(&[input], &mut 1000, Span::default())
                    .unwrap(),
                "placement {index}"
            );
            assert_eq!(prepared.to_string(), before);
        }
    }

    #[test]
    fn prepared_input_observation_ignores_ambient_authority_and_declaration_names() {
        let input = DeclarationId {
            package: 7,
            local: 90,
        };
        let mut prepared = literal_preparation(true);
        prepared.abstract_parameters.push(input);
        prepared.abstract_value_parameters.push((input, p(P::Bool)));
        prepared.scoped_parameters.push(nir::DeclarationParameter {
            declaration: input,
            sort: Sort::Type,
        });
        prepared.scoped_arguments.push(p(P::U8));
        let function = &mut prepared.package_mut().functions[0];
        function.type_parameters.push(input);
        function.capabilities.push(n::TraitRequirement {
            subject: p(P::Bool),
            interface: input,
            interface_arguments: vec![],
        });
        assert!(
            !prepared
                .uses_declaration_inputs(&[input], &mut 1000, Span::default())
                .unwrap()
        );
        assert!(
            !prepared
                .uses_declaration_inputs(&[], &mut 0, Span::default())
                .unwrap()
        );
        let mut exhausted = 3;
        assert!(
            prepared
                .uses_declaration_inputs(&[input], &mut exhausted, Span::default())
                .is_err()
        );
        assert!(exhausted < 3);
        assert!(
            !prepared
                .uses_declaration_inputs(&[input], &mut 1000, Span::default())
                .unwrap()
        );
    }

    #[test]
    fn prepared_input_observation_preserves_shared_code_and_recovers_from_exhaustion() {
        let input = DeclarationId {
            package: 7,
            local: 90,
        };
        let mut prepared = literal_preparation(true);
        let mut body = prepared.package_mut().functions[0].body.clone();
        body.kind = K::StagedValue(Term::Rigid {
            declaration: input,
            sort: Sort::Type,
        });
        let body = std::sync::Arc::new(body);
        prepared.package_mut().functions[0].body.kind = K::Closure {
            parameters: vec![],
            captures: vec![],
            body: body.clone(),
        };
        let before = prepared.to_string();
        let mut exhausted = 6;
        assert!(
            prepared
                .uses_declaration_inputs(&[input], &mut exhausted, Span::default())
                .is_err()
        );
        assert!(exhausted < 6);
        assert_eq!(prepared.to_string(), before);
        assert!(
            prepared
                .uses_declaration_inputs(&[input], &mut 1000, Span::default())
                .unwrap()
        );
        let K::Closure { body: observed, .. } = &prepared.package_mut().functions[0].body.kind
        else {
            panic!()
        };
        assert!(std::sync::Arc::ptr_eq(&body, observed));
        assert_eq!(std::sync::Arc::strong_count(&body), 2);
        assert!(matches!(observed.kind, K::StagedValue(_)));
        assert_eq!(prepared.to_string(), before);
    }

    #[test]
    fn required_value_and_type_proofs_consume_the_invocation_budget() {
        for type_result in [false, true] {
            let make = || {
                let mut prepared = literal_preparation(type_result);
                prepared.package_mut().functions[0].requirements = vec![n::TypePredicate::And(
                    (0..64)
                        .map(|_| {
                            n::TypePredicate::Not(Box::new(n::TypePredicate::Equal(
                                p(P::Bool),
                                p(P::U8),
                            )))
                        })
                        .collect(),
                )];
                prepared
            };
            let mut baseline = 100_000;
            literal_preparation(type_result)
                .invoke(&mut baseline, Span::default())
                .unwrap();
            let mut enough = 100_000;
            make().invoke(&mut enough, Span::default()).unwrap();
            assert!(
                enough + 64 < baseline,
                "baseline={baseline}, with proof={enough}"
            );
            let mut limited = 32;
            assert!(make().invoke(&mut limited, Span::default()).is_err());
            assert!(limited < 32);
            let mut recovery = 100_000;
            make().invoke(&mut recovery, Span::default()).unwrap();
        }
    }

    #[test]
    fn type_invocations_transport_abstract_inputs_but_do_not_observe_them() {
        let declaration = DeclarationId {
            package: 7,
            local: 90,
        };
        let binding = n::LocalId {
            owner: 11,
            index: 0,
        };
        for observe in [false, true] {
            let mut prepared = literal_preparation(true);
            prepared.abstract_value_parameters = vec![(declaration, p(P::Bool))];
            prepared.inputs = vec![(
                binding,
                p(P::Bool),
                Some(Term::Rigid {
                    declaration,
                    sort: Sort::ComptimeValue,
                }),
            )];
            let function = &mut prepared.package_mut().functions[0];
            function.parameters = vec![binding];
            function.signature = Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: p(P::Bool),
                }],
                result: Box::new(p(P::Type)),
            };
            if observe {
                let result = function.body.clone();
                function.body.kind = K::If {
                    condition: Box::new(n::Expr {
                        source: result.source,
                        ty: p(P::Bool),
                        kind: K::Local(binding),
                    }),
                    then_branch: Box::new(result.clone()),
                    else_branch: Some(Box::new(result)),
                };
            }
            let mut steps = 10_000;
            let result = prepared.invoke(&mut steps, Span::default());
            assert!(steps < 10_000);
            if observe {
                assert!(format!("{:?}", result.unwrap_err()).contains("AbstractValueUnresolved"));
            } else {
                assert_eq!(result.unwrap(), (Value::Type(p(P::U8)), p(P::Type)));
            }
        }
    }

    #[test]
    fn abstract_type_invocation_failure_is_terminal_and_does_not_poison_other_calls() {
        use crate::type_computations::{PreparedInput, Session, State};
        let declaration = DeclarationId {
            package: 7,
            local: 90,
        };
        let binding = n::LocalId {
            owner: 11,
            index: 0,
        };
        let mut prepared = literal_preparation(true);
        let function = &mut prepared.package_mut().functions[0];
        function.parameters = vec![binding];
        function.signature = Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Runtime,
                ty: p(P::Bool),
            }],
            result: Box::new(p(P::Type)),
        };
        let output = function.body.clone();
        function.body.kind = K::If {
            condition: Box::new(n::Expr {
                source: output.source,
                ty: p(P::Bool),
                kind: K::Local(binding),
            }),
            then_branch: Box::new(output.clone()),
            else_branch: Some(Box::new(output)),
        };
        let mut context = crate::InferenceContext::default();
        let mut session = Session::new_in_scope_with_pointer_copy(
            &prepared.world.package,
            &context,
            &mut Budget {
                steps: 10_000,
                depth: 128,
            },
            &[],
            &[],
            &[(declaration, p(P::Bool))],
            None,
            None,
        )
        .unwrap();
        let mut failed = Vec::new();
        for (id, domain) in [
            (declaration, p(P::Bool)),
            (
                DeclarationId {
                    package: 7,
                    local: 91,
                },
                p(P::Bool),
            ),
            (declaration, p(P::U8)),
        ] {
            let output = session
                .enqueue_prepared(
                    &mut context,
                    prepared.entry,
                    vec![PreparedInput {
                        ty: crate::InferenceTerm::Stable(p(P::Bool)),
                        value: InvocationValue::Live(Value::AbstractValue {
                            declaration: id,
                            ty: domain,
                        }),
                    }],
                )
                .unwrap();
            assert!(matches!(
                session.resume(&mut context, output).unwrap(),
                State::Failed(_)
            ));
            assert!(context.freeze(&crate::InferenceTerm::Meta(output)).is_err());
            let remaining = session.remaining_steps();
            let diagnostic = format!("{}", session.state(output).unwrap());
            assert_eq!(
                format!("{}", session.resume(&mut context, output).unwrap()),
                diagnostic
            );
            assert_eq!(session.remaining_steps(), remaining);
            failed.push(output);
        }
        let output = session
            .enqueue_prepared(
                &mut context,
                prepared.entry,
                vec![PreparedInput {
                    ty: crate::InferenceTerm::Stable(p(P::Bool)),
                    value: InvocationValue::Live(Value::Bool(true)),
                }],
            )
            .unwrap();
        assert_eq!(
            session.resume(&mut context, output).unwrap(),
            &State::Complete(p(P::U8))
        );
        assert_eq!(
            context.freeze(&crate::InferenceTerm::Meta(output)).unwrap(),
            p(P::U8)
        );
        for output in failed {
            assert!(matches!(session.state(output), Some(State::Failed(_))));
        }
    }

    #[test]
    fn rejected_scope_preparation_debits_the_shared_budget_on_both_paths() {
        for type_result in [false, true] {
            let mut prepared = literal_preparation(type_result);
            let id = DeclarationId {
                package: 7,
                local: 90,
            };
            prepared.abstract_parameters = vec![id];
            prepared.abstract_value_parameters = vec![(id, p(P::Bool))];
            let mut steps = 1000;
            assert!(prepared.invoke(&mut steps, Span::default()).is_err());
            assert!(steps < 1000, "failed scope setup refunded its work");
        }
    }

    #[test]
    fn prepared_owned_inputs_keep_writeback_in_type_and_data_calls() {
        for type_result in [false, true] {
            let mut prepared = literal_preparation(type_result);
            let binding = n::LocalId {
                owner: 11,
                index: 0,
            };
            let result_type = prepared.result_type.clone();
            let function = &mut prepared.package_mut().functions[0];
            function.parameters = vec![binding];
            function.signature = Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: p(P::Bool),
                }],
                result: Box::new(result_type),
            };
            let result = function.body.clone();
            function.body.kind = K::Block {
                statements: vec![n::Expr {
                    source: result.source,
                    ty: p(P::Void),
                    kind: K::Assign {
                        place: n::Place {
                            root: binding,
                            fields: vec![],
                        },
                        value: Box::new(n::Expr {
                            source: result.source,
                            ty: p(P::Bool),
                            kind: K::Literal(n::Literal::Bool(false)),
                        }),
                    },
                }],
                result: Some(Box::new(result)),
            };
            prepared.inputs = vec![(binding, p(P::Bool), None)];
            let owner = comptime_interpretor::Storage::new(Value::Bool(true));
            let mut steps = 10_000;
            prepared
                .invoke_with_live_inputs(
                    vec![(binding, Value::Storage(owner.clone()))],
                    &mut steps,
                    Span::default(),
                )
                .unwrap();
            assert_eq!(owner.read().unwrap(), Value::Bool(false));
            assert!(steps < 10_000);
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
        let function = &mut prepared.package_mut().functions[0];
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
            let function = &mut prepared.package_mut().functions[0];
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
    fn prepared_live_inputs_keep_shared_closures_across_type_and_value_invocations() {
        use std::sync::Arc;
        let span = Span::default();
        for type_result in [false, true] {
            let mut prepared = literal_preparation(type_result);
            let result_type = prepared.result_type.clone();
            let selector = n::LocalId {
                owner: 11,
                index: 0,
            };
            let frozen = n::LocalId {
                owner: 11,
                index: 1,
            };
            let flag = n::LocalId {
                owner: 11,
                index: 2,
            };
            let expr = |ty, kind| n::Expr {
                ty,
                kind,
                source: n::SourceRange { start: 0, end: 1 },
            };
            let boolean = |value| expr(p(P::Bool), K::Literal(n::Literal::Bool(value)));
            let read = || expr(p(P::Bool), K::Local(flag));
            let callback_type = Term::Closure {
                environment: vec![p(P::Bool)],
                signature: Box::new(Term::ComptimeFunction(Box::new(Term::Function {
                    parameters: vec![],
                    result: Box::new(result_type.clone()),
                }))),
            };
            let toggle = expr(
                p(P::Void),
                K::Assign {
                    place: n::Place {
                        root: flag,
                        fields: vec![],
                    },
                    value: Box::new(expr(
                        p(P::Bool),
                        K::If {
                            condition: Box::new(read()),
                            then_branch: Box::new(boolean(false)),
                            else_branch: Some(Box::new(boolean(true))),
                        },
                    )),
                },
            );
            let selected = if type_result {
                expr(
                    p(P::Type),
                    K::If {
                        condition: Box::new(read()),
                        then_branch: Box::new(expr(p(P::Type), K::TypeValue(p(P::Bool)))),
                        else_branch: Some(Box::new(expr(p(P::Type), K::TypeValue(p(P::U8))))),
                    },
                )
            } else {
                read()
            };
            let mut factory = prepared.package_mut().clone();
            factory.functions[0].signature = Term::Function {
                parameters: vec![],
                result: Box::new(callback_type.clone()),
            };
            factory.functions[0].body = expr(
                callback_type.clone(),
                K::Closure {
                    parameters: vec![],
                    captures: vec![n::Capture {
                        binding: flag,
                        ty: p(P::Bool),
                        value: Some(Box::new(boolean(false))),
                    }],
                    body: Arc::new(expr(
                        result_type.clone(),
                        K::Block {
                            statements: vec![toggle],
                            result: Some(Box::new(selected)),
                        },
                    )),
                },
            );
            let mut producer = Interpreter::for_comptime(&factory, Budget::default()).unwrap();
            let callback = producer.invoke(prepared.entry, vec![]).unwrap();
            let function = &mut prepared.package_mut().functions[0];
            function.parameters = vec![selector, frozen];
            function.signature = Term::Function {
                parameters: vec![
                    Parameter {
                        sort: Sort::ComptimeValue,
                        ty: callback_type.clone(),
                    },
                    Parameter {
                        sort: Sort::ComptimeValue,
                        ty: p(P::Bool),
                    },
                ],
                result: Box::new(result_type.clone()),
            };
            function.body = expr(
                result_type.clone(),
                K::Call {
                    callee: Box::new(expr(callback_type.clone(), K::Local(selector))),
                    arguments: vec![],
                    parameter_bindings: vec![],
                },
            );
            prepared.inputs = vec![
                (selector, callback_type, None),
                (frozen, p(P::Bool), Some(Term::Bool(true))),
            ];
            let mut steps = 10_000;
            // Routing errors must leave the shared callback untouched.
            for invalid in [
                vec![(frozen, Value::Bool(false))],
                vec![(selector, callback.clone()), (selector, callback.clone())],
                vec![(
                    n::LocalId {
                        owner: 99,
                        index: 0,
                    },
                    callback.clone(),
                )],
            ] {
                let before = steps;
                assert!(
                    prepared
                        .fork()
                        .invoke_with_live_inputs(invalid, &mut steps, span)
                        .is_err()
                );
                assert_eq!(steps, before);
            }
            for expected in [true, false, true] {
                let (value, ty) = prepared
                    .fork()
                    .invoke_with_live_inputs(vec![(selector, callback.clone())], &mut steps, span)
                    .unwrap();
                assert_eq!(ty, result_type);
                if type_result {
                    assert_eq!(
                        value,
                        Value::Type(p(if expected { P::Bool } else { P::U8 }))
                    );
                } else {
                    assert_eq!(value, Value::Bool(expected));
                }
            }
            assert!(steps < 10_000);
            assert!(
                prepared.inputs[0].2.is_none(),
                "invocation must not modify the reusable preparation"
            );
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
            arguments: vec![frozen_types(vec![p(P::Bool), p(P::U8)])],
        };
        let prepared = PreparedRequired {
            world: RequiredWorld::new(n::Package::default(), Span::default()).unwrap(),
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
            abstract_value_parameters: vec![],
            context: crate::ptc::Context::default(),
            in_signature: true,
            copy_authority: None,
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

fn implementation_index(
    package: &n::Package,
    span: Span,
) -> Result<crate::implementation::Index, Error> {
    let mut index = crate::implementation::Index::for_builtin_packages(
        package.target,
        package.builtin_implementation_packages.iter().copied(),
    );
    index.set_normal_form_metadata(package.normal_forms.clone());
    index.sync_nominal_layouts(&package.structs, &package.enums, &package.newtypes);
    for (id, domain) in package
        .declaration_parameter_domains(&mut 100_000)
        .map_err(|message| error(span, message))?
    {
        index
            .register_parameter_domain(id, domain)
            .map_err(|message| error(span, message))?;
    }
    for implementation in &package.implementations {
        if implementation.interface.is_none() {
            continue;
        }
        index
            .insert(
                crate::implementation::ImplementationHead {
                    parameter_types: implementation.parameter_types.clone(),
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
