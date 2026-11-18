//! Reusable invocation of one concrete pure comptime function in an owned
//! compilation snapshot. Result caching is temporarily limited to calls whose
//! bound output type is Type; other calls bypass key preparation and lookup.
//! Callers supply fully prepared arguments; defaults and
//! input evaluation are outside this cache. Runtime resource protocols do not
//! authorize comptime value-reference operations or cache admission.
use crate::comptime_keys::{Protocol, Snapshot};
use comptime_executor::{Cache, Context, Error, KeyProtocol, Outcome, QueryId, Scope};
use comptime_interpretor::{Budget, Interpreter, Value};
use elaboration::{
    materialization::{ClosedDeclarations, DeclarationSnapshot, DeclarationSource},
    ptc,
};
use intrinsic::LangItems;
use nir::{
    DeclarationId, Term,
    program::{FunctionPhase, Package, TypePredicate},
};

pub struct Invocation {
    declarations: ClosedDeclarations,
    code_session: comptime_interpretor::CodeSession,
    stats: crate::invocation::InvocationStats,
    declaration: DeclarationId,
    arguments: Vec<Term>,
    signature: Term,
    items: LangItems,
    schemas: Vec<Schema>,
    requirements: Vec<TypePredicate>,
    capacity: usize,
    protocols: ExactInputs,
    cache: Cache<Snapshot, Value>,
    budget: ptc::Budget,
}
struct Schema {
    inputs: Vec<Term>,
    result: Term,
    protocols: Vec<usize>,
}
impl std::fmt::Display for Schema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(binding (inputs")?;
        for ty in &self.inputs {
            write!(f, " {ty}")?;
        }
        write!(f, ") (result {}) (protocols", self.result)?;
        for slot in &self.protocols {
            write!(f, " {slot}")?;
        }
        f.write_str("))")
    }
}
struct ExactInputs(Vec<Protocol>);
impl ExactInputs {
    fn protocol(
        &mut self,
        value: &Snapshot,
        context: &mut Context,
    ) -> Result<&mut Protocol, Error<String>> {
        for protocol in &mut self.0 {
            context.spend::<String>(1)?;
            if protocol.owns(value) {
                return Ok(protocol);
            }
        }
        Err(Error::Protocol(
            "input snapshot has no protocol in this invocation".into(),
        ))
    }
}
impl KeyProtocol<Snapshot> for ExactInputs {
    type Error = String;
    fn hash(&mut self, value: &Snapshot, context: &mut Context) -> Result<u64, Error<String>> {
        self.protocol(value, context)?.hash(value, context)
    }
    fn equivalent(
        &mut self,
        a: &Snapshot,
        b: &Snapshot,
        context: &mut Context,
    ) -> Result<bool, Error<String>> {
        if !self.protocol(a, context)?.equivalent(a, b, context)? {
            return Ok(false);
        }
        // Source Eq remains required. This additional dependency discriminator
        // establishes compatibility for queries that can inspect every field.
        // It is intentionally not an alternative source equality definition.
        let work = a
            .value()
            .immutable_copy_work()
            .and_then(|a| {
                b.value()
                    .immutable_copy_work()
                    .and_then(|b| a.checked_add(b))
            })
            .ok_or_else(|| Error::Protocol("unbounded cache input comparison".into()))?;
        context.spend::<String>(work)?;
        Ok(a.value() == b.value())
    }
}
impl Invocation {
    /// Capture and prepare one invocation under a shared construction allowance.
    /// Later execution uses its own cumulative Context and retained query limits.
    pub fn prepare(
        package: &Package,
        items: &LangItems,
        declaration: DeclarationId,
        arguments: Vec<Term>,
        capacity: usize,
        budget: ptc::Budget,
    ) -> Result<Self, String> {
        let mut remaining = budget.steps;
        let source = DeclarationSource::Borrowed(package)
            .retain_declarations(&mut remaining)
            .map_err(|error| format!("memoization source retention budget exhausted: {error:?}"))?;
        Self::prepare_in_source_metered(
            source,
            items,
            declaration,
            arguments,
            capacity,
            budget,
            &mut remaining,
        )
    }

    /// Reuse the exact source owner across callers. Preparation proves this
    /// invocation's requirements; sharing a source grants no cache admission.
    pub fn prepare_in_source(
        source: DeclarationSnapshot,
        items: &LangItems,
        declaration: DeclarationId,
        arguments: Vec<Term>,
        capacity: usize,
        budget: ptc::Budget,
    ) -> Result<Self, String> {
        Self::prepare_in_source_metered(
            source,
            items,
            declaration,
            arguments,
            capacity,
            budget,
            &mut { budget.steps },
        )
    }

    /// Report construction work without reducing the independent query limits
    /// retained for future invocations. Failed preparation publishes no cache.
    pub fn prepare_in_source_metered(
        source: DeclarationSnapshot,
        items: &LangItems,
        declaration: DeclarationId,
        arguments: Vec<Term>,
        capacity: usize,
        budget: ptc::Budget,
        remaining: &mut usize,
    ) -> Result<Self, String> {
        let mut work = ptc::Budget {
            steps: budget.steps.min(*remaining),
            ..budget
        };
        let before = work.steps;
        let prepared = (|| {
            let declarations = source
                .closed(&mut work)
                .map_err(|error| format!("memoization declaration preparation: {error}"))?;
            let function = declarations
                .function(declaration, &mut work.steps)?
                .ok_or("memoized function is missing")?;
            if function.phase != FunctionPhase::Comptime || !function.pure {
                return Err("memoization requires one ordinary pure comptime function".into());
            }
            let binders = declarations.input_binders(&function.type_parameters, &mut work.steps)?;
            nir::copy_budget::charge_type_substitution(
                &function.signature,
                &binders,
                &arguments,
                &mut work.steps,
            )
            .map_err(|error| format!("memoized signature budget exhausted: {error:?}"))?;
            let signature = function
                .signature
                .instantiate_parameters(&binders, &arguments)
                .map_err(|e| e.to_string())?;
            let Term::Function { parameters, result } = &signature else {
                return Err("memoized function signature is invalid".into());
            };
            signature.check_scope(&[]).map_err(|e| e.to_string())?;
            let closed = parameters
                .iter()
                .map(|p| &p.ty)
                .chain(std::iter::once(result.as_ref()))
                .all(|ty| ty.check_scope(&[]).is_ok());
            let world = declarations.capabilities();
            work.steps = work
                .steps
                .checked_sub(
                    function
                        .requirements
                        .len()
                        .saturating_add(function.capabilities.len()),
                )
                .ok_or("memoized requirement collection budget exhausted")?;
            for predicate in &function.requirements {
                nir::copy_budget::charge_predicate_substitution(
                    predicate,
                    &[],
                    &[],
                    &mut work.steps,
                )
                .map_err(|error| {
                    format!("memoized requirement copy budget exhausted: {error:?}")
                })?;
            }
            for capability in &function.capabilities {
                for term in
                    std::iter::once(&capability.subject).chain(&capability.interface_arguments)
                {
                    nir::copy_budget::charge_type_substitution(term, &[], &[], &mut work.steps)
                        .map_err(|error| {
                            format!("memoized capability copy budget exhausted: {error:?}")
                        })?;
                }
            }
            let requirements = function.requirements.iter().cloned().chain(
                function
                    .capabilities
                    .iter()
                    .cloned()
                    .map(TypePredicate::Trait),
            );
            let mut deferred = Vec::new();
            work.steps = work
                .steps
                .checked_sub(parameters.len())
                .ok_or("memoized input frame budget exhausted")?;
            let frames = vec![parameters.iter().map(|p| p.sort).collect()];
            for predicate in requirements {
                nir::copy_budget::charge_predicate_substitution(
                    &predicate,
                    &binders,
                    &arguments,
                    &mut work.steps,
                )
                .map_err(|error| {
                    format!("memoized contract specialization budget exhausted: {error:?}")
                })?;
                let predicate = predicate
                    .instantiate_parameters(&binders, &arguments)
                    .map_err(|e| e.to_string())?;
                nir::copy_budget::charge_predicate_substitution(
                    &predicate,
                    &[],
                    &[],
                    &mut work.steps,
                )
                .map_err(|error| {
                    format!("memoized contract inspection budget exhausted: {error:?}")
                })?;
                let mut admission = Context::new(work.steps);
                let mut open = false;
                let inspected = map_requirement(
                    &predicate,
                    &mut |term| {
                        term.check_scope(&frames)
                            .map_err(|e| Error::Protocol(e.to_string()))?;
                        open |= term.check_scope(&[]).is_err();
                        Ok(term.clone())
                    },
                    &mut admission,
                    0,
                );
                work.steps = admission.remaining();
                inspected.map_err(|e| format!("memoized contract admission: {e:?}"))?;
                if open {
                    deferred.push(predicate);
                    continue;
                }
                if !matches!(
                    world.check_predicate_metered(&predicate, &mut work)?,
                    ptc::Outcome::Entailed(_)
                ) {
                    return Err("memoized function type contract is not established".into());
                }
            }
            let cacheable = closed && **result == Term::Primitive(nir::Primitive::Type);
            let code_session = comptime_interpretor::CodeSession::new(source.package().clone());
            let protocols = if cacheable {
                parameters
                    .iter()
                    .map(|p| {
                        Protocol::prepare_in_source_with_session_metered(
                            source.clone(),
                            items,
                            &p.ty,
                            budget,
                            &mut work.steps,
                            &code_session,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                vec![]
            };
            let schemas = if cacheable {
                work.steps = work
                    .steps
                    .checked_sub(parameters.len().saturating_add(protocols.len()))
                    .ok_or("memoized schema budget exhausted")?;
                for ty in parameters
                    .iter()
                    .map(|p| &p.ty)
                    .chain(std::iter::once(result.as_ref()))
                {
                    nir::copy_budget::charge_type_substitution(ty, &[], &[], &mut work.steps)
                        .map_err(|error| {
                            format!("memoized schema copy budget exhausted: {error:?}")
                        })?;
                }
                vec![Schema {
                    inputs: parameters.iter().map(|p| p.ty.clone()).collect(),
                    result: (**result).clone(),
                    protocols: (0..protocols.len()).collect(),
                }]
            } else {
                vec![]
            };
            work.steps = work
                .steps
                .checked_sub(items.len())
                .ok_or("memoized language bindings budget exhausted")?;
            Ok(Self {
                declarations,
                code_session,
                stats: Default::default(),
                declaration,
                arguments,
                signature,
                items: items.clone(),
                schemas,
                requirements: deferred,
                capacity,
                protocols: ExactInputs(protocols),
                cache: Cache::new(
                    Scope {
                        definitions: 0,
                        protocols: 0,
                        target: 0,
                    },
                    capacity,
                ),
                budget,
            })
        })();
        *remaining -= before - work.steps;
        prepared
    }
    pub fn last_execution_stats(&self) -> crate::invocation::InvocationStats {
        self.stats
    }
    pub fn cached_results(&self) -> usize {
        self.cache.len()
    }
    pub fn invoke(
        &mut self,
        values: Vec<Value>,
        context: &mut Context,
    ) -> Result<Outcome<Value>, Error<String>> {
        self.stats = Default::default();
        let (types, result_type) = self.bind_inputs(&values, context)?;
        // Gate before preparing Eq/Hash, snapshots or cache keys. A dependent
        // result is classified only after binding this invocation's inputs.
        if result_type != Term::Primitive(nir::Primitive::Type) {
            let (value, _) = evaluate(
                &self.declarations,
                &mut self.code_session,
                &mut self.stats,
                self.budget,
                self.declaration,
                &self.arguments,
                &result_type,
                values,
                context,
                false,
            )?;
            return Ok(Outcome {
                value: std::sync::Arc::new(value),
                reused: false,
            });
        }
        let mut transient = None;
        let (operation, inputs) = if let Some(index) = self
            .schemas
            .iter()
            .position(|s| s.inputs == types && s.result == result_type)
        {
            let slots = self.schemas[index].protocols.clone();
            let inputs = values
                .into_iter()
                .zip(&slots)
                .map(|(value, slot)| self.protocols.0[*slot].admit(value, context))
                .collect::<Result<Vec<_>, _>>()?;
            (index as u32, inputs)
        } else {
            context.spend::<String>(types.len().saturating_add(1))?;
            let mut protocol_work = self.budget.steps.min(context.remaining());
            let before = protocol_work;
            let prepared = types
                .iter()
                .map(|ty| {
                    Protocol::prepare_in_source_with_session_metered(
                        self.declarations.source().clone(),
                        &self.items,
                        ty,
                        self.budget,
                        &mut protocol_work,
                        &self.code_session,
                    )
                })
                .collect::<Result<Vec<_>, _>>();
            context.spend::<String>(before - protocol_work)?;
            let mut prepared = prepared.map_err(Error::Protocol)?;
            let inputs = values
                .into_iter()
                .zip(&mut prepared)
                .map(|(value, protocol)| protocol.admit(value, context))
                .collect::<Result<Vec<_>, _>>()?;
            if self.schemas.len() < self.capacity && self.schemas.len() < u32::MAX as usize {
                let operation = self.schemas.len() as u32;
                let start = self.protocols.0.len();
                let slots: Vec<_> = (start..start + prepared.len()).collect();
                self.protocols.0.extend(prepared);
                self.schemas.push(Schema {
                    inputs: types,
                    result: result_type.clone(),
                    protocols: slots,
                });
                (operation, inputs)
            } else {
                transient = Some(ExactInputs(prepared));
                (0, inputs)
            }
        };
        if let Some(mut protocols) = transient {
            let mut cache = Cache::new(
                Scope {
                    definitions: 0,
                    protocols: 0,
                    target: 0,
                },
                0,
            );
            execute(
                &self.declarations,
                &mut self.code_session,
                &mut self.stats,
                self.budget,
                self.declaration,
                &self.arguments,
                &result_type,
                &mut cache,
                &mut protocols,
                operation,
                inputs,
                context,
            )
        } else {
            execute(
                &self.declarations,
                &mut self.code_session,
                &mut self.stats,
                self.budget,
                self.declaration,
                &self.arguments,
                &result_type,
                &mut self.cache,
                &mut self.protocols,
                operation,
                inputs,
                context,
            )
        }
    }
    fn bind_inputs(
        &self,
        values: &[Value],
        context: &mut Context,
    ) -> Result<(Vec<Term>, Term), Error<String>> {
        let Term::Function { parameters, result } = &self.signature else {
            unreachable!()
        };
        if values.len() != parameters.len() {
            return Err(Error::Protocol(
                "memoized call argument count mismatch".into(),
            ));
        }
        context.spend::<String>(values.len().saturating_add(1))?;
        let mut supplied = vec![None; parameters.len()];
        let mut scope: Option<nir::FunctionInputBinding> = None;
        let mut types = Vec::with_capacity(parameters.len());
        for (index, (parameter, value)) in parameters.iter().zip(values).enumerate() {
            let expected = match &scope {
                Some(scope) => scope.rebind(&parameter.ty),
                None => parameter.ty.substitute_partial(&supplied),
            }
            .map_err(|e| Error::Protocol(e.to_string()))?;
            expected.check_scope(&[]).map_err(|_| {
                Error::Protocol("memoized parameter still needs value-dependent binding".into())
            })?;
            if parameter.sort == nir::Sort::Type
                || (parameter.sort == nir::Sort::ComptimeValue
                    && expected == Term::Primitive(nir::Primitive::Type))
            {
                let Value::Type(ty) = value else {
                    return Err(Error::Protocol(
                        "memoized Type parameter requires a Type value".into(),
                    ));
                };
                let work = value
                    .immutable_copy_work()
                    .ok_or_else(|| Error::Protocol("unbounded Type input".into()))?;
                context.spend::<String>(work)?;
                supplied[index] = Some(ty.clone());
                scope = Some(
                    self.signature
                        .prepare_function_inputs(&supplied)
                        .map_err(|e| Error::Protocol(e.to_string()))?,
                );
            }
            types.push(expected);
        }
        let result = match &scope {
            Some(scope) => scope.rebind(result),
            None => result.substitute_partial(&supplied),
        }
        .map_err(|e| Error::Protocol(e.to_string()))?;
        result.check_scope(&[]).map_err(|_| {
            Error::Protocol("memoized result still needs value-dependent binding".into())
        })?;
        let world = self.declarations.capabilities();
        for requirement in &self.requirements {
            let predicate = map_requirement(
                requirement,
                &mut |term| {
                    let bound = match &scope {
                        Some(scope) => scope.rebind(term),
                        None => term.substitute_partial(&supplied),
                    }
                    .map_err(|e| Error::Protocol(e.to_string()))?;
                    bound
                        .check_scope(&[])
                        .map_err(|e| Error::Protocol(e.to_string()))?;
                    Ok(bound)
                },
                context,
                0,
            )?;
            if predicate.trivial_truth() == Some(true) {
                continue;
            }
            let mut proof_budget = ptc::Budget {
                steps: self.budget.steps.min(context.remaining()),
                max_depth: self.budget.max_depth,
            };
            let before = proof_budget.steps;
            let outcome = world.check_predicate_metered(&predicate, &mut proof_budget);
            context.spend::<String>(before - proof_budget.steps)?;
            match outcome {
                Ok(ptc::Outcome::Entailed(_)) => {}
                Err(nir::NominalAdmissionError::Budget)
                | Ok(ptc::Outcome::Unknown {
                    reason: ptc::UnknownReason::BudgetExhausted,
                    ..
                }) => return Err(Error::BudgetExhausted),
                Err(error) => return Err(Error::Protocol(error.to_string())),
                Ok(outcome) => {
                    return Err(Error::Protocol(format!(
                        "memoized function bound type contract is not established: {outcome}"
                    )));
                }
            }
        }
        Ok((types, result))
    }
}
impl std::fmt::Display for Invocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(memoized-invocation (declaration {}) (arguments",
            self.declaration
        )?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        write!(
            f,
            ") (signature {}) (cached-results {}) (requirements",
            self.signature,
            self.cache.len()
        )?;
        for requirement in &self.requirements {
            write!(f, " {requirement}")?;
        }
        f.write_str(") (bindings")?;
        for schema in &self.schemas {
            write!(f, " {schema}")?;
        }
        f.write_str("))")
    }
}

fn execute(
    declarations: &ClosedDeclarations,
    code_session: &mut comptime_interpretor::CodeSession,
    stats: &mut crate::invocation::InvocationStats,
    budget: ptc::Budget,
    declaration: DeclarationId,
    arguments: &[Term],
    result_type: &Term,
    cache: &mut Cache<Snapshot, Value>,
    protocols: &mut ExactInputs,
    operation: u32,
    inputs: Vec<Snapshot>,
    context: &mut Context,
) -> Result<Outcome<Value>, Error<String>> {
    cache.query_pure_admitted(
        QueryId {
            package: declaration.package,
            declaration: declaration.local,
            operation,
        },
        inputs,
        context,
        protocols,
        |inputs, context| {
            for input in inputs {
                let work = input
                    .value()
                    .immutable_clone_work()
                    .ok_or_else(|| Error::Evaluation("unbounded invocation input".into()))?;
                context.spend::<String>(work)?;
            }
            let values = inputs.iter().map(|input| input.value().clone()).collect();
            evaluate(
                declarations,
                code_session,
                stats,
                budget,
                declaration,
                arguments,
                result_type,
                values,
                context,
                true,
            )
        },
    )
}

fn evaluate(
    declarations: &ClosedDeclarations,
    code_session: &mut comptime_interpretor::CodeSession,
    stats: &mut crate::invocation::InvocationStats,
    budget: ptc::Budget,
    declaration: DeclarationId,
    arguments: &[Term],
    result_type: &Term,
    values: Vec<Value>,
    context: &mut Context,
    cache_result: bool,
) -> Result<(Value, bool), Error<String>> {
    let package = declarations.source().package();
    let source_queries =
        elaboration::materialization::SourceInvocationQueries::in_closed(declarations);
    let solver = |predicate: &TypePredicate, remaining: &mut u64| {
        source_queries.check(predicate, budget, remaining)
    };
    let method_resolver = |callee: &nir::program::Expr, remaining: &mut u64| {
        source_queries.resolve(callee, remaining)
    };
    let associated_resolver = |access: &nir::program::Expr, remaining: &mut u64| {
        source_queries.associated(access, remaining)
    };
    let steps = u64::try_from(context.remaining()).unwrap_or(u64::MAX);
    let mut vm_budget = Budget { steps, depth: 128 };
    let prepared_vm = Interpreter::for_comptime_resolving_in_session(
        package,
        code_session,
        comptime_interpretor::SourceResolvers {
            types: &solver,
            methods: &method_resolver,
            associated: &associated_resolver,
        },
        &mut vm_budget,
    );
    let mut vm = match prepared_vm {
        Ok(vm) => vm,
        Err(error) => {
            context.spend::<String>((steps - vm_budget.steps) as usize)?;
            return Err(evaluation_error(error));
        }
    };
    let result = vm.with_code_session(code_session, |vm| {
        let value = vm.invoke_instantiated(declaration, arguments.to_vec(), values)?;
        let publish = if !cache_result || value.immutable_copy_work().is_none() {
            false
        } else {
            let validation = vm.validate_immutable_input(&value, result_type);
            match validation {
                Ok(()) => true,
                Err(error)
                    if matches!(
                        error.kind,
                        comptime_interpretor::ErrorKind::Type
                            | comptime_interpretor::ErrorKind::FunctionInputRejected(_)
                    ) =>
                {
                    false
                }
                Err(error) => return Err(error),
            }
        };
        Ok((value, publish))
    });
    *stats = crate::invocation::InvocationStats {
        bytecode_compilations: vm.bytecode_compilations(),
        bytecode_invocations: vm.bytecode_invocations(),
    };
    context.spend::<String>((steps - vm.remaining_steps()) as usize)?;
    result.map_err(evaluation_error)
}

fn evaluation_error(error: comptime_interpretor::Error) -> Error<String> {
    use comptime_interpretor::ErrorKind;
    if matches!(
        error.kind,
        ErrorKind::StepsExhausted | ErrorKind::DepthExhausted
    ) {
        return Error::BudgetExhausted;
    }
    Error::Evaluation(error.to_string())
}

// Only type predicates enter this transformation; state contracts stay in the VM.
fn map_requirement(
    predicate: &TypePredicate,
    map: &mut impl FnMut(&Term) -> Result<Term, Error<String>>,
    context: &mut Context,
    depth: usize,
) -> Result<TypePredicate, Error<String>> {
    use TypePredicate as P;
    context.spend::<String>(1)?;
    if depth >= 128 {
        return Err(Error::Protocol("memoized contract depth exhausted".into()));
    }
    Ok(match predicate {
        P::Bool(value) => P::Bool(*value),
        P::Equal(a, b) => P::Equal(map(a)?, map(b)?),
        P::Trait(requirement) => {
            let mut bound = requirement.clone();
            bound.subject = map(&requirement.subject)?;
            bound.interface_arguments = requirement
                .interface_arguments
                .iter()
                .map(&mut *map)
                .collect::<Result<_, _>>()?;
            P::Trait(bound)
        }
        P::Not(inner) => P::Not(Box::new(map_requirement(inner, map, context, depth + 1)?)),
        P::And(parts) | P::Or(parts) => {
            let parts = parts
                .iter()
                .map(|part| map_requirement(part, map, context, depth + 1))
                .collect::<Result<_, _>>()?;
            if matches!(predicate, P::And(_)) {
                P::And(parts)
            } else {
                P::Or(parts)
            }
        }
    })
}
