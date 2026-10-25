//! Reusable invocation of one concrete pure comptime function in an owned
//! compilation snapshot. Callers supply fully prepared arguments; defaults and
//! input evaluation are outside this cache. Optional paired source Copy/Drop
//! bindings authorize pointer transfers in this immutable compilation snapshot.
use crate::comptime_keys::{Protocol, Snapshot};
use comptime_executor::{Cache, Context, Error, KeyProtocol, Outcome, QueryId, Scope};
use comptime_interpretor::{Budget, Interpreter, Value};
use elaboration::{capabilities::World, implementation::Index, ptc};
use intrinsic::{LangItem, LangItemDef, LangItems};
use nir::{
    DeclarationId, Term,
    program::{FunctionPhase, Package, TypePredicate},
};

pub struct Invocation {
    package: Package,
    declaration: DeclarationId,
    arguments: Vec<Term>,
    signature: Term,
    items: LangItems,
    resources: Option<(DeclarationId, DeclarationId)>,
    schemas: Vec<Schema>,
    requirements: Vec<TypePredicate>,
    capacity: usize,
    context: ptc::Context,
    index: Index,
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
    pub fn prepare(
        package: &Package,
        items: &LangItems,
        declaration: DeclarationId,
        arguments: Vec<Term>,
        capacity: usize,
        budget: ptc::Budget,
    ) -> Result<Self, String> {
        let resources = match (items.get(LangItem::Copy), items.get(LangItem::Drop)) {
            (None, None) => None,
            (Some(LangItemDef::UserDef(copy)), Some(LangItemDef::UserDef(drop))) => {
                crate::resources::ProtocolContext::selected(
                    package,
                    crate::resources::Selection::Identity(copy),
                    Some(crate::resources::Selection::Identity(drop)),
                    budget,
                )?;
                Some((copy, drop))
            }
            _ => {
                return Err(
                    "memoized pointer transfers require paired source Copy and Drop bindings"
                        .into(),
                );
            }
        };
        let mut matches = package
            .functions
            .iter()
            .filter(|f| f.declaration == declaration);
        let function = matches.next().ok_or("memoized function is missing")?;
        if matches.next().is_some() || function.phase != FunctionPhase::Comptime || !function.pure {
            return Err("memoization requires one ordinary pure comptime function".into());
        }
        let mut binder_fuel = budget.steps;
        let binders = package.declaration_binders(&function.type_parameters, &mut binder_fuel)?;
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
        let context = ptc::Context::from_package(package)?;
        let index = crate::implementation_world::index(&package, budget)
            .map_err(|error| format!("memoization implementation index: {error}"))?;
        let world = World {
            index: &index,
            context: &context,
            traits: &package.traits,
            implementations: &package.implementations,
            assumptions: &[],
        };
        let requirements = function.requirements.iter().cloned().chain(
            function
                .capabilities
                .iter()
                .cloned()
                .map(TypePredicate::Trait),
        );
        let mut deferred = Vec::new();
        let mut admission = Context::new(budget.steps);
        let frames = vec![parameters.iter().map(|p| p.sort).collect()];
        for predicate in requirements {
            let predicate = predicate
                .instantiate_parameters(&binders, &arguments)
                .map_err(|e| e.to_string())?;
            let mut open = false;
            map_requirement(
                &predicate,
                &mut |term| {
                    term.check_scope(&frames)
                        .map_err(|e| Error::Protocol(e.to_string()))?;
                    open |= term.check_scope(&[]).is_err();
                    Ok(term.clone())
                },
                &mut admission,
                0,
            )
            .map_err(|e| format!("memoized contract admission: {e:?}"))?;
            if open {
                deferred.push(predicate);
                continue;
            }
            if !matches!(
                world.check_predicate(&predicate, budget)?,
                ptc::Outcome::Entailed(_)
            ) {
                return Err("memoized function type contract is not established".into());
            }
        }
        let protocols = if closed {
            parameters
                .iter()
                .map(|p| Protocol::prepare(package, items, &p.ty, budget))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            vec![]
        };
        let schemas = if closed {
            vec![Schema {
                inputs: parameters.iter().map(|p| p.ty.clone()).collect(),
                result: (**result).clone(),
                protocols: (0..protocols.len()).collect(),
            }]
        } else {
            vec![]
        };
        let mut staging_fuel = budget.steps;
        let dispatch =
            if crate::invocation::needs_input_staging(package, declaration, &mut staging_fuel)? {
                // Ordinary Type inputs select the concrete dispatch world at a miss.
                Vec::new()
            } else {
                elaboration::materialization::dispatch_instances_for_roots(
                    package,
                    &[iir::InstanceKey {
                        declaration,
                        arguments: arguments.clone(),
                    }],
                    budget,
                )?
            };
        let mut package = package.clone();
        package.dispatch_instances = dispatch;
        Ok(Self {
            package,
            declaration,
            arguments,
            signature,
            items: items.clone(),
            resources,
            schemas,
            requirements: deferred,
            capacity,
            context,
            index,
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
    }
    pub fn cached_results(&self) -> usize {
        self.cache.len()
    }
    pub fn invoke(
        &mut self,
        values: Vec<Value>,
        context: &mut Context,
    ) -> Result<Outcome<Value>, Error<String>> {
        let (types, result_type) = self.bind_inputs(&values, context)?;
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
            let prepared = types
                .iter()
                .map(|ty| Protocol::prepare(&self.package, &self.items, ty, self.budget))
                .collect::<Result<Vec<_>, _>>()
                .map_err(Error::Protocol)?;
            let inputs = values
                .into_iter()
                .zip(&prepared)
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
        let world = World {
            index: &self.index,
            context: &self.context,
            traits: &self.package.traits,
            implementations: &self.package.implementations,
            assumptions: &[],
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
                &self.package,
                &world,
                self.resources,
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
                &self.package,
                &world,
                self.resources,
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
        let world = World {
            index: &self.index,
            context: &self.context,
            traits: &self.package.traits,
            implementations: &self.package.implementations,
            assumptions: &[],
        };
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
            if !matches!(outcome.map_err(Error::Protocol)?, ptc::Outcome::Entailed(_)) {
                return Err(Error::Protocol(
                    "memoized function bound type contract is not established".into(),
                ));
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
    package: &Package,
    world: &World<'_>,
    resources: Option<(DeclarationId, DeclarationId)>,
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
    let solver = |predicate: &TypePredicate, remaining: &mut u64| {
        crate::implementation_world::solve_metered(world, predicate, budget, remaining)
    };
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
                    .immutable_copy_work()
                    .ok_or("unbounded invocation input")?;
                context.spend::<String>(work).map_err(|e| e.to_string())?;
            }
            let values = inputs.iter().map(|input| input.value().clone()).collect();
            let (prepared, graph, entry, type_arguments, values) =
                prepare_bound_entry(package, declaration, arguments, values, budget, context)?;
            let steps = u64::try_from(context.remaining()).unwrap_or(u64::MAX);
            let mut vm_budget = Budget { steps, depth: 128 };
            let prepared_vm = match &graph {
                Some(graph) => Interpreter::from_iir_for_comptime_metered(graph, &mut vm_budget),
                None => Interpreter::for_comptime(prepared.as_ref(), vm_budget),
            };
            let copy_failure = std::cell::RefCell::new(None::<String>);
            let pointer_copy = |ty: &Term, remaining: &mut u64| {
                let Some((copy, drop)) = resources else {
                    return false;
                };
                let mut fuel = usize::try_from(*remaining).unwrap_or(usize::MAX);
                let before = fuel;
                let evidence = elaboration::verification::copy::acquire(
                    package,
                    world,
                    copy,
                    drop,
                    ty,
                    ptc::Budget {
                        steps: fuel,
                        ..budget
                    },
                    &mut fuel,
                );
                *remaining = remaining.saturating_sub((before - fuel) as u64);
                let allowed = evidence.is_ok();
                *copy_failure.borrow_mut() = evidence.err();
                allowed
            };
            let mut vm = match prepared_vm {
                Ok(vm) => vm
                    .with_budgeted_type_solver(&solver)
                    .with_budgeted_pointer_copy_permission(&pointer_copy),
                Err(error) => {
                    context
                        .spend::<String>((steps - vm_budget.steps) as usize)
                        .map_err(|e| e.to_string())?;
                    return Err(error.to_string());
                }
            };
            let mut admission_work = 0;
            let result = (|| {
                let value = vm.invoke_instantiated(entry, type_arguments, values)?;
                let publish = if value.immutable_copy_work().is_none() {
                    false
                } else {
                    let validation = if graph.is_some() {
                        // IIR intentionally lacks source formation metadata.
                        // Admit the result against the same owned source world.
                        let before = vm.remaining_steps();
                        let mut admission = Interpreter::for_comptime(
                            prepared.as_ref(),
                            Budget {
                                steps: before,
                                depth: 128,
                            },
                        )?
                        .with_budgeted_type_solver(&solver);
                        let result = admission.validate_immutable_input(&value, result_type);
                        admission_work = before - admission.remaining_steps();
                        result
                    } else {
                        vm.validate_immutable_input(&value, result_type)
                    };
                    match validation {
                        Ok(()) => true,
                        Err(error) if error.kind == comptime_interpretor::ErrorKind::Type => false,
                        Err(error) => return Err(error),
                    }
                };
                Ok((value, publish))
            })();
            context
                .spend::<String>((steps - vm.remaining_steps() + admission_work) as usize)
                .map_err(|e| e.to_string())?;
            result.map_err(
                |e: comptime_interpretor::Error| match copy_failure.borrow().as_ref() {
                    Some(reason) => format!("{e}; pointer Copy evidence: {reason}"),
                    None => e.to_string(),
                },
            )
        },
    )
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

// Specialize in a private snapshot: the original declaration must remain usable
// by other input bindings, and no staged function identity escapes into the key.
fn prepare_bound_entry<'a>(
    package: &'a Package,
    declaration: DeclarationId,
    arguments: &[Term],
    values: Vec<Value>,
    budget: ptc::Budget,
    context: &mut Context,
) -> Result<
    (
        std::borrow::Cow<'a, Package>,
        Option<iir::Package>,
        DeclarationId,
        Vec<Term>,
        Vec<Value>,
    ),
    String,
> {
    let function = package
        .functions
        .iter()
        .find(|f| f.declaration == declaration)
        .ok_or("missing memoized declaration")?;
    let mut fuel = context.remaining();
    let before = fuel;
    let staged = crate::invocation::needs_input_staging(package, declaration, &mut fuel);
    context
        .spend::<String>(before - fuel)
        .map_err(|e| format!("{e:?}"))?;
    if !staged? {
        return Ok((
            std::borrow::Cow::Borrowed(package),
            None,
            declaration,
            arguments.to_vec(),
            values,
        ));
    }
    let mut fuel = context.remaining();
    let before = fuel;
    let result: Result<_, String> = (|| {
        let binders = package.declaration_binders(&function.type_parameters, &mut fuel)?;
        let mut template = function
            .instantiate_parameters(&binders, arguments)
            .map_err(|e| e.to_string())?;
        template.type_parameters.clear();
        let Term::Function { parameters, .. } = template.signature.callable_signature() else {
            unreachable!()
        };
        let mut inputs = Vec::new();
        let mut retained = Vec::new();
        for (parameter, value) in parameters.iter().zip(values) {
            if parameter.sort == nir::Sort::Type {
                let Value::Type(ty) = value else {
                    return Err("staged Type input is not Type".into());
                };
                inputs.push(Some(nir::program::Expr {
                    ty: Term::Primitive(nir::Primitive::Type),
                    kind: nir::program::ExprKind::TypeValue(ty),
                    source: template.body.source,
                }));
            } else {
                inputs.push(None);
                retained.push(value);
            }
        }
        let mut bound =
            iir::staging::bind_inputs_with_layouts(package, &template, &inputs, &mut fuel)?;
        let mut identity = DeclarationId {
            package: u64::MAX,
            local: u32::MAX,
        };
        while package.functions.iter().any(|f| f.declaration == identity)
            || package
                .runtime_function_identities
                .iter()
                .any(|f| f.declaration == identity)
        {
            identity.local = identity
                .local
                .checked_sub(1)
                .ok_or("no private invocation identity available")?;
        }
        bound.declaration = identity;
        bound.name = "<memoized-input>".into();
        for function in &package.functions {
            iir::visit::expressions(function, &mut fuel, |_, _| Ok(()))?;
        }
        let mut prepared = package.clone();
        prepared.functions.push(bound);
        Ok((prepared, identity, retained))
    })();
    context
        .spend::<String>(before - fuel)
        .map_err(|e| format!("{e:?}"))?;
    let (mut prepared, entry, values) = result?;
    let dispatch_budget = ptc::Budget {
        steps: budget.steps.min(context.remaining()),
        max_depth: budget.max_depth,
    };
    context
        .spend::<String>(dispatch_budget.steps)
        .map_err(|e| format!("{e:?}"))?;
    let graph = elaboration::materialization::instantiate(
        &prepared,
        &[iir::InstanceKey {
            declaration: entry,
            arguments: vec![],
        }],
        std::collections::BTreeSet::new(),
        dispatch_budget,
    )?;
    prepared.dispatch_instances = graph.dispatch.clone();
    Ok((
        std::borrow::Cow::Owned(prepared),
        Some(graph),
        entry,
        vec![],
        values,
    ))
}
