//! Private required invocation during concrete staged-input preparation.
use super::*;
use comptime_interpretor::{ClosureIdentity, CodeSession, Interpreter, Value};

// Immutable source declarations outlive individual detached invocation bodies.
// Each invocation still owns its VM, environment, phase checks and result graph.
pub(super) struct Snapshot {
    source: DeclarationSnapshot,
    code: CodeSession,
    from_published_scope: bool,
}

impl Snapshot {
    fn capture(
        source: DeclarationSource<'_>,
        fuel: &mut usize,
    ) -> Result<Self, nir::copy_budget::CopyBudgetExhausted> {
        let source = source.retain_for_invocation(fuel)?;
        let code = CodeSession::new(source.package().clone());
        Ok(Self {
            source,
            code,
            from_published_scope: false,
        })
    }
    fn for_scope<'s>(
        slot: &'s mut Option<Self>,
        scope: &type_resolution::dispatch::DeclarationScope<'_>,
        fuel: &mut usize,
    ) -> Result<&'s mut Self, nir::copy_budget::CopyBudgetExhausted> {
        let current = scope.published_snapshot();
        let replace = slot.as_ref().is_none_or(|existing| match current {
            Some(source) => !std::sync::Arc::ptr_eq(existing.source.package(), source.package()),
            None => existing.from_published_scope,
        });
        if replace {
            let mut captured = Self::capture(scope.declaration_source(), fuel)?;
            captured.from_published_scope = current.is_some();
            *slot = Some(captured);
        }
        Ok(slot.as_mut().expect("source snapshot was captured"))
    }
}

struct InvocationFailure(&'static str, comptime_interpretor::Error, Option<String>);

impl std::fmt::Display for InvocationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "staged comptime {}: {}", self.0, self.1)?;
        if let Some(reason) = &self.2 {
            write!(f, "; definition publication: {reason}")?;
        }
        Ok(())
    }
}

impl Snapshot {
    fn execute(
        &mut self,
        expression: &n::Expr,
        world: crate::capabilities::World<'_>,
        limits: Budget,
        fuel: &mut usize,
        publication: Option<(&dyn Expansion, u64)>,
    ) -> Result<(Value, Option<(n::PublishedDefinitions, u32)>), InvocationFailure> {
        let snapshot = self.source.package();
        // The checked expression is source code, not a concrete IIR closure.
        // Resolve each demanded member in the same immutable declaration world;
        // rebuilding every concrete implementation here duplicates outer work.
        let source_queries = SourceInvocationQueries::in_source(
            DeclarationSource::Snapshot(&self.source),
            world.context,
            world.assumptions,
        );
        let solver = |predicate: &n::TypePredicate, remaining: &mut u64| {
            source_queries.check(
                predicate,
                Budget {
                    steps: usize::MAX,
                    ..limits
                },
                remaining,
            )
        };
        let associated_resolver =
            |access: &n::Expr, remaining: &mut u64| source_queries.associated(access, remaining);
        let method_resolver =
            |callee: &n::Expr, remaining: &mut u64| source_queries.resolve(callee, remaining);
        let mut invocation_budget = comptime_interpretor::Budget {
            steps: std::mem::take(fuel) as u64,
            depth: 128,
        };
        let publisher = publication
            .and_then(|(provider, owner)| provider.definition_publisher(snapshot, owner, &solver));
        let prepared = Interpreter::for_comptime_resolving_in_scope_in_session(
            snapshot,
            &mut self.code,
            world.assumptions,
            &[],
            &[],
            comptime_interpretor::SourceResolvers {
                types: &solver,
                methods: &method_resolver,
                associated: &associated_resolver,
            },
            &mut invocation_budget,
        );
        *fuel = usize::try_from(invocation_budget.steps).unwrap_or(usize::MAX);
        let mut interpreter =
            prepared.map_err(|error| InvocationFailure("preparation", error, None))?;
        if let Some(service) = publisher.as_deref() {
            interpreter = interpreter.with_definition_service(service);
        }
        let value = interpreter.with_code_session(&mut self.code, |vm| {
            vm.invoke_comptime_expression(expression)
        });
        *fuel = usize::try_from(interpreter.remaining_steps()).unwrap_or(usize::MAX);
        drop(interpreter);
        let value = value.map_err(|error| {
            InvocationFailure(
                "invocation",
                error,
                publisher.as_ref().and_then(|p| p.last_diagnostic()),
            )
        })?;
        Ok((value, publisher.map(|p| p.finish())))
    }
}

pub(super) fn type_initializer_host<'a>(
    expansion: Option<&'a dyn Expansion>,
    retained: &'a mut Option<Snapshot>,
) -> impl FnMut(
    &type_resolution::dispatch::DeclarationScope<'_>,
    &Term,
    Id,
    n::SourceRange,
    &mut Budget,
) -> Result<crate::associated::TypeInitializerResult, nir::NominalAdmissionError>
+ 'a {
    move |scope, key, implementation, site, budget| {
        let Term::AssociatedType {
            origin,
            arguments,
            subject,
        } = key
        else {
            return Err("expected an associated Type projection".into());
        };
        crate::associated::charge_projection_copy(key, &mut budget.steps)?;
        let read = n::Expr {
            source: site,
            ty: Term::Primitive(nir::Primitive::Type),
            kind: K::AssociatedValue {
                interface: n::InterfaceInstance {
                    declaration: origin.interface,
                    arguments: arguments.clone(),
                },
                subject: subject.as_ref().clone(),
                member: origin.member,
            },
        };
        let snapshot = Snapshot::for_scope(retained, scope, &mut budget.steps)
            .map_err(|_| nir::NominalAdmissionError::Budget)?;
        match snapshot.execute(
            &read,
            scope.capabilities(),
            *budget,
            &mut budget.steps,
            expansion.map(|p| (p, implementation.package)),
        ) {
            Ok((Value::Type(value), publication)) => {
                Ok(crate::associated::TypeInitializerResult { value, publication })
            }
            Ok(_) => Err("associated Type initializer did not return a type identity".into()),
            Err(InvocationFailure(
                _,
                comptime_interpretor::Error {
                    kind: comptime_interpretor::ErrorKind::StepsExhausted,
                    ..
                },
                _,
            )) => Err(nir::NominalAdmissionError::Budget),
            Err(failure) => Err(failure.to_string().into()),
        }
    }
}

#[derive(Default)]
struct FrozenGraph {
    // Retaining each Arc makes identity stable until the whole snapshot finishes.
    // None marks a currently active closure and detects recursive environments.
    closures: std::collections::HashMap<ClosureIdentity, Option<(n::LocalId, Term)>>,
    bindings: Vec<n::Expr>,
}

impl Builder<'_, '_> {
    /// Keep completed data cheap; all computation uses the ordinary source VM.
    /// Shape recognition does not validate a value. Validation/copy failures
    /// propagate directly and must never trigger a second execution attempt.
    pub(super) fn complete_staged_input(
        &mut self,
        expression: &n::Expr,
    ) -> Result<n::Expr, String> {
        let mut data = true;
        iir::visit::expression_pruned(expression, &mut self.fuel, |expr, _| {
            if !data {
                return Ok(false);
            }
            data = matches!(
                expr.kind,
                K::Unit
                    | K::TypeValue(_)
                    | K::Function(_)
                    | K::FunctionInstance { .. }
                    | K::TraitMethod { .. }
                    | K::Tuple(_)
                    | K::TypeList(_)
                    | K::Record { .. }
                    | K::AnonymousRecord { .. }
                    | K::Variant { .. }
                    | K::OptionalAbsent
                    | K::OptionalPresent(_)
                    | K::Literal(_)
            ) && !matches!(
                expr.kind,
                K::Literal(n::Literal::PendingInteger { .. } | n::Literal::PendingFloat { .. })
            );
            Ok(data)
        })?;
        if !data {
            return self.invoke_staged_input(expression);
        }
        iir::staging::canonical_input_with_layouts(
            self.world.scope().declaration_source().package().into(),
            expression,
            &mut self.fuel,
        )?;
        let mut remaining = self.fuel as u64;
        let copied = nir::copy_budget::charge_expression(expression, &mut remaining);
        self.fuel = usize::try_from(remaining).unwrap_or(usize::MAX);
        copied.map_err(|limit| format!("staged data copy budget exhausted: {limit:?}"))?;
        Ok(expression.clone())
    }

    pub(super) fn invoke_staged_input(&mut self, expression: &n::Expr) -> Result<n::Expr, String> {
        self.tick(0)?;
        Snapshot::for_scope(
            &mut self.invocation_snapshot,
            self.world.scope(),
            &mut self.fuel,
        )
        .map_err(|error| format!("staged source retention budget exhausted: {error:?}"))?;
        let mut snapshot = self
            .invocation_snapshot
            .take()
            .expect("source snapshot was captured");
        let result = self.invoke_with_snapshot(expression, &mut snapshot);
        self.invocation_snapshot = Some(snapshot);
        result
    }

    fn invoke_with_snapshot(
        &mut self,
        expression: &n::Expr,
        retained: &mut Snapshot,
    ) -> Result<n::Expr, String> {
        iir::visit::expression(expression, &mut self.fuel, |_, _| Ok(()))?;
        crate::associated::charge_projection_copy(&expression.ty, &mut self.fuel)
            .map_err(|_| "staged result type budget exhausted")?;
        let owner = self.expression_owner;
        let (value, generated) = retained
            .execute(
                expression,
                self.world.capabilities(),
                self.budget,
                &mut self.fuel,
                self.expansion.zip(owner),
            )
            .map_err(|failure| failure.to_string())?;
        let mut budget = Budget {
            steps: self.fuel,
            ..self.budget
        };
        let freeze = |package: &n::Package, budget: &mut Budget| {
            ValueFreezer { package, budget }.freeze_staged_value(
                value,
                &expression.ty,
                expression.source,
                0,
            )
        };
        let result = match generated {
            Some((definitions, next)) if !definitions.is_empty() => {
                self.world.with_published_nominals(
                    definitions,
                    owner.expect("configured publisher has an owner"),
                    next,
                    &mut budget,
                    freeze,
                )
            }
            _ => freeze(
                self.world.scope().declaration_source().package(),
                &mut budget,
            ),
        };
        self.fuel = budget.steps;
        result
    }
}

struct ValueFreezer<'p, 'b> {
    package: &'p n::Package,
    budget: &'b mut Budget,
}

impl ValueFreezer<'_, '_> {
    fn freeze_staged_value(
        &mut self,
        value: Value,
        ty: &Term,
        source: n::SourceRange,
        depth: usize,
    ) -> Result<n::Expr, String> {
        let mut graph = FrozenGraph::default();
        let result = self.freeze_staged_value_in(&value, ty, source, depth, &mut graph)?;
        if graph.bindings.is_empty() {
            return Ok(result);
        }
        crate::associated::charge_projection_copy(ty, &mut self.budget.steps)
            .map_err(|_| "staged snapshot result type copy budget exhausted")?;
        // All generated bindings are private to this block. Capture initializers
        // refer only to these bindings; closure bodies use their own environments.
        Ok(n::Expr {
            source,
            ty: ty.clone(),
            kind: K::Block {
                statements: graph.bindings,
                result: Some(Box::new(result)),
            },
        })
    }

    fn freeze_staged_value_in(
        &mut self,
        value: &Value,
        ty: &Term,
        source: n::SourceRange,
        depth: usize,
        graph: &mut FrozenGraph,
    ) -> Result<n::Expr, String> {
        if depth >= self.budget.max_depth.min(128) || self.budget.steps == 0 {
            return Err("dispatch materialization budget exhausted".into());
        }
        self.budget.steps -= 1;
        crate::associated::charge_projection_copy(ty, &mut self.budget.steps)
            .map_err(|_| "staged value type copy budget exhausted")?;
        let kind = match value {
            Value::IirCallable(_) | Value::FunctionInstance { .. }
                if matches!(ty.callable_signature(), Term::Function { .. }) =>
            {
                if let Value::IirCallable(handle) = value
                    && ty.is_comptime_callable() != handle.ty().is_comptime_callable()
                {
                    return Err("staged callable phase mismatch".into());
                }
                let (declaration, arguments) = value.declaration_instance().unwrap();
                // The source VM has checked this result against `ty`. Named
                // identities can belong to either phase; freezing preserves
                // the original signature rather than granting runtime entry.
                for argument in arguments {
                    nir::type_computations::charge_term(argument, &mut self.budget.steps)
                        .map_err(|error| error.to_string())?;
                    iir::completed_type(argument, &BTreeSet::new(), &mut self.budget.steps)?;
                }
                K::FunctionInstance {
                    declaration,
                    arguments: arguments.to_vec(),
                }
            }
            Value::RuntimeTraitMethod(identity)
                if !ty.is_comptime_callable()
                    && matches!(ty.callable_signature(), Term::Function { .. }) =>
            {
                let comptime_interpretor::TraitMethodIdentity {
                    interface,
                    subject,
                    member,
                    method_arguments,
                } = identity.as_ref();
                for argument in std::iter::once(subject)
                    .chain(&interface.arguments)
                    .chain(method_arguments)
                {
                    nir::type_computations::charge_term(argument, &mut self.budget.steps)
                        .map_err(|error| error.to_string())?;
                    iir::completed_type(argument, &BTreeSet::new(), &mut self.budget.steps)?;
                }
                K::TraitMethod {
                    interface: interface.declaration,
                    interface_arguments: interface.arguments.clone(),
                    subject: subject.clone(),
                    member: *member,
                    method_arguments: method_arguments.clone(),
                }
            }
            Value::Function(declaration)
                if matches!(ty.callable_signature(), Term::Function { .. }) =>
            {
                K::Function(*declaration)
            }
            Value::Closure(closure)
                if matches!(ty.callable_signature(), Term::Function { .. })
                    && ty.is_comptime_callable() == closure.ty().is_comptime_callable() =>
            {
                let identity = ClosureIdentity::new(closure);
                if let Some(previous) = graph.closures.get(&identity) {
                    let (binding, previous_type) = previous
                        .as_ref()
                        .ok_or("cyclic staged closure environment")?;
                    if previous_type != ty {
                        return Err("shared staged closure type mismatch".into());
                    }
                    return Ok(n::Expr {
                        source,
                        ty: ty.clone(),
                        kind: K::Local(*binding),
                    });
                }
                let capture_types: &[Term] = match ty {
                    Term::Closure { environment, .. } => environment,
                    _ => &[],
                };
                if capture_types.len() != closure.captured_values().len() {
                    return Err("staged closure environment shape mismatch".into());
                }
                graph.closures.insert(identity.clone(), None);
                let mut steps = self.budget.steps as u64;
                let checked = nir::copy_budget::charge_type_rewrite(closure.body(), &mut steps);
                self.budget.steps = usize::try_from(steps).unwrap_or(usize::MAX);
                checked.map_err(|error| {
                    format!("staged closure body exceeds copy limits: {error:?}")
                })?;
                self.budget.steps = self
                    .budget
                    .steps
                    .checked_sub(capture_types.len() + closure.parameters().len())
                    .ok_or("staged closure environment budget exhausted")?;
                let mut captures = Vec::new();
                for ((binding, value), capture_type) in closure.captured_values().zip(capture_types)
                {
                    let captured = if let Value::Storage(cell) = value {
                        let mut steps = self.budget.steps as u64;
                        let result = cell.inspect_owned(&mut steps, 128, |value, steps| {
                            self.budget.steps = usize::try_from(*steps).unwrap_or(usize::MAX);
                            let result = self.freeze_staged_value_in(
                                value,
                                capture_type,
                                source,
                                depth + 1,
                                graph,
                            );
                            *steps = self.budget.steps as u64;
                            result
                        });
                        self.budget.steps = usize::try_from(steps).unwrap_or(usize::MAX);
                        result.map_err(|e| format!("staged capture storage: {e:?}"))??
                    } else {
                        self.freeze_staged_value_in(value, capture_type, source, depth + 1, graph)?
                    };
                    captures.push(n::Capture {
                        binding,
                        ty: capture_type.clone(),
                        value: Some(Box::new(captured)),
                    });
                }
                let binding = n::LocalId {
                    owner: u32::MAX,
                    index: u32::try_from(graph.bindings.len())
                        .map_err(|_| "staged closure binding space exhausted")?,
                };
                let value = n::Expr {
                    source,
                    ty: ty.clone(),
                    kind: K::Closure {
                        parameters: closure.parameters().to_vec(),
                        captures,
                        body: closure.body().clone(),
                    },
                };
                graph.bindings.push(n::Expr {
                    source,
                    ty: Term::Primitive(nir::Primitive::Void),
                    kind: K::Let {
                        binding,
                        value: Box::new(value),
                    },
                });
                graph.closures.insert(identity, Some((binding, ty.clone())));
                K::Local(binding)
            }
            Value::Optional { ty: actual, value } => {
                if actual != ty {
                    return Err("staged optional identity mismatch".into());
                }
                let Term::Optional(payload) = ty else {
                    return Err("staged optional type missing".into());
                };
                match value {
                    None => K::OptionalAbsent,
                    Some(value) => K::OptionalPresent(Box::new(self.freeze_staged_value_in(
                        value,
                        payload,
                        source,
                        depth + 1,
                        graph,
                    )?)),
                }
            }
            Value::ErrorResult {
                ty: actual,
                error,
                value,
            } => {
                if actual != ty {
                    return Err("staged error result identity mismatch".into());
                }
                let Term::ErrorResult { errors, result } = ty else {
                    return Err("staged error result type missing".into());
                };
                let (conversion, domain) = match error {
                    None => (nir::error_results::Conversion::Success, result.as_ref()),
                    Some(index) => (
                        nir::error_results::Conversion::Failure,
                        errors
                            .get(*index)
                            .ok_or("staged error result tag invalid")?,
                    ),
                };
                K::ErrorConvert {
                    conversion,
                    value: Box::new(self.freeze_staged_value_in(
                        value,
                        domain,
                        source,
                        depth + 1,
                        graph,
                    )?),
                }
            }
            Value::Newtype { ty: actual, value } => {
                if actual != ty {
                    return Err("staged newtype identity mismatch".into());
                }
                let Term::Nominal {
                    declaration,
                    arguments,
                } = ty
                else {
                    return Err("staged newtype requires a nominal type".into());
                };
                let package = self.package;
                let definition = package
                    .newtypes
                    .iter()
                    .find(|definition| definition.declaration == *declaration)
                    .ok_or("staged newtype declaration is missing")?;
                self.budget.steps = self
                    .budget
                    .steps
                    .checked_sub(package.newtypes.len())
                    .ok_or("staged newtype layout budget exhausted")?;
                let representation = definition
                    .representation_budgeted(arguments, &[], &mut self.budget.steps)
                    .map_err(|error| error.to_string())?;
                K::NewtypeConvert(Box::new(self.freeze_staged_value_in(
                    value,
                    &representation,
                    source,
                    depth + 1,
                    graph,
                )?))
            }
            Value::Unit => K::Unit,
            scalar @ (Value::Bool(_)
            | Value::Char(_)
            | Value::Symbol(_)
            | Value::String(_)
            | Value::SignedInteger(_)
            | Value::UnsignedInteger(_)
            | Value::Float16(_)
            | Value::BFloat16(_)
            | Value::Float32(_)
            | Value::Float64(_)
            | Value::Float(_)
            | Value::Integer(_)
            | Value::BitVec(_)) => {
                // Preserve the existing exact floating-domain checks. This
                // serialization helper does not confer type admission.
                let expected = match scalar {
                    Value::Float(_) => Some(nir::Primitive::Float),
                    Value::Float16(_) => Some(nir::Primitive::F16),
                    Value::BFloat16(_) => Some(nir::Primitive::Bf16),
                    Value::Float32(_) => Some(nir::Primitive::F32),
                    Value::Float64(_) => Some(nir::Primitive::F64),
                    _ => None,
                };
                if expected.is_some_and(|domain| *ty != Term::Primitive(domain)) {
                    return Err("staged floating value identity mismatch".into());
                }
                let mut budget = comptime_runtime::Budget {
                    steps: self.budget.steps,
                    max_limbs: 4096,
                };
                let literal = scalar.freeze_literal(&mut budget);
                self.budget.steps = budget.steps;
                K::Literal(
                    literal
                        .map_err(|e| e.to_string())?
                        .expect("scalar value representation"),
                )
            }
            Value::Type(v) => {
                crate::associated::charge_projection_copy(v, &mut self.budget.steps)
                    .map_err(|_| "staged Type payload copy budget exhausted")?;
                K::TypeValue(v.clone())
            }
            Value::List { ty: actual, values } => {
                if actual != ty {
                    return Err("staged list type identity mismatch".into());
                }
                let element = self
                    .package
                    .normal_forms
                    .iter()
                    .find_map(|head| head.list_element(ty))
                    .cloned()
                    .ok_or("staged list requires trusted meta.List")?;
                self.budget.steps = self
                    .budget
                    .steps
                    .checked_sub(values.len())
                    .ok_or("staged list budget exhausted")?;
                K::TypeList(
                    values
                        .into_iter()
                        .map(|value| {
                            self.freeze_staged_value_in(value, &element, source, depth + 1, graph)
                        })
                        .collect::<Result<_, _>>()?,
                )
            }
            Value::Object(object) => {
                if *ty != Term::Primitive(nir::Primitive::Object) {
                    return Err("staged Object identity mismatch".into());
                }
                let entries = object.entries();
                self.budget.steps = self
                    .budget
                    .steps
                    .checked_sub(entries.len())
                    .ok_or("staged Object entry budget exhausted")?;
                let mut names = Vec::with_capacity(entries.len());
                let mut fields = Vec::with_capacity(entries.len());
                for entry in entries {
                    self.budget.steps = self
                        .budget
                        .steps
                        .checked_sub(entry.key().map_or(0, str::len))
                        .ok_or("staged Object key budget exhausted")?;
                    names.push(entry.key().map(str::to_owned));
                    let value = entry.value();
                    fields.push(self.freeze_staged_value_in(
                        &value.value,
                        &value.ty,
                        source,
                        depth + 1,
                        graph,
                    )?);
                }
                K::Object { names, fields }
            }
            Value::AnonymousRecord(values) => {
                let Term::Record(types) = ty else {
                    return Err("staged record result type mismatch".into());
                };
                if values.len() != types.len() {
                    return Err("staged record result arity mismatch".into());
                }
                let mut fields = Vec::new();
                for ((name, value), (expected_name, ty)) in values.into_iter().zip(types) {
                    self.budget.steps = self
                        .budget
                        .steps
                        .checked_sub(name.len())
                        .ok_or("staged record budget exhausted")?;
                    if name != expected_name {
                        return Err("staged record result label mismatch".into());
                    }
                    fields.push(self.freeze_staged_value_in(
                        value,
                        ty,
                        source,
                        depth + 1,
                        graph,
                    )?);
                }
                K::AnonymousRecord { fields }
            }
            Value::Tuple(values) => {
                let Some(types) = nir::normal_forms::tuple_fields(Some(self.package), ty) else {
                    return Err("staged tuple result type mismatch".into());
                };
                if types.len() != values.len() {
                    return Err("staged tuple result arity mismatch".into());
                }
                K::Tuple(
                    values
                        .into_iter()
                        .zip(types)
                        .map(|(v, t)| self.freeze_staged_value_in(v, t, source, depth + 1, graph))
                        .collect::<Result<_, _>>()?,
                )
            }
            Value::Record {
                declaration,
                arguments,
                fields,
            } => {
                let fields = self.freeze_staged_nominal_fields(
                    *declaration,
                    arguments,
                    fields,
                    None,
                    ty,
                    source,
                    depth,
                    graph,
                )?;
                K::Record {
                    declaration: *declaration,
                    fields,
                }
            }
            Value::Variant {
                declaration,
                arguments,
                index,
                fields,
            } => {
                let variant = u32::try_from(*index)
                    .map_err(|_| "staged enum variant index exceeds representation")?;
                let fields = self.freeze_staged_nominal_fields(
                    *declaration,
                    arguments,
                    fields,
                    Some(variant),
                    ty,
                    source,
                    depth,
                    graph,
                )?;
                K::Variant {
                    declaration: *declaration,
                    index: *index,
                    fields,
                }
            }
            _ => {
                return Err(
                    "staged invocation result cannot yet be frozen as an instance input".into(),
                );
            }
        };
        Ok(n::Expr {
            kind,
            ty: ty.clone(),
            source,
        })
    }

    fn freeze_staged_nominal_fields(
        &mut self,
        declaration: Id,
        arguments: &[Term],
        fields: &[Value],
        variant: Option<u32>,
        ty: &Term,
        source: n::SourceRange,
        depth: usize,
        graph: &mut FrozenGraph,
    ) -> Result<Vec<n::Expr>, String> {
        nir::type_computations::charge_term(ty, &mut self.budget.steps)
            .map_err(|error| error.to_string())?;
        if !matches!(ty, Term::Nominal { declaration: expected, arguments: expected_arguments }
            if *expected == declaration && expected_arguments == arguments)
        {
            return Err("staged nominal result identity mismatch".into());
        }
        let types = nir::normal_forms::query_nominal_value_fields(
            self.package,
            ty,
            variant,
            &mut self.budget.steps,
        )
        .map_err(|error| format!("staged nominal layout: {error}"))?;
        if fields.len() != types.len() {
            return Err("staged nominal field count mismatch".into());
        }
        fields
            .into_iter()
            .zip(types)
            .map(|(value, ty)| self.freeze_staged_value_in(value, &ty, source, depth + 1, graph))
            .collect()
    }
}

// Inline argument frames retain ordinary evaluation order. A named template
// reference itself has no runtime evaluation, storage or environment, so its
// immutable aliases can be substituted before specializing the exposed calls.
// Do not propagate through writes, address-taking or closure capture.
pub(super) fn forward_template_aliases(
    expression: &mut n::Expr,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    fuel: &mut usize,
) -> Result<(), String> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut unavailable = BTreeSet::new();
    let mut declarations = Vec::new();
    iir::visit::expression_pruned(expression, fuel, |expr, fuel| {
        if matches!(
            expr.kind,
            K::Assign { .. }
                | K::AddressOf(_)
                | K::ConstAddress { .. }
                | K::Closure { .. }
                | K::TemplateEqual(..)
        ) {
            unavailable.extend(nir::free_bindings::collect(expr, fuel)?);
            return Ok(false);
        }
        if let K::Let { binding, value } = &expr.kind {
            if matches!(value.ty.callable_signature(), Term::Function { parameters, .. }
                if parameters.iter().any(|p| p.sort != nir::Sort::Runtime))
                && matches!(
                    value.kind,
                    K::Function(_)
                        | K::FunctionInstance { .. }
                        | K::TraitMethod { .. }
                        | K::Local(_)
                        | K::If { .. }
                        | K::Match { .. }
                        | K::Block { .. }
                        | K::CallPreparation { .. }
                )
            {
                let mut remaining = *fuel as u64;
                nir::copy_budget::charge_expression(value, &mut remaining)
                    .map_err(|_| "template alias collection budget exhausted")?;
                *fuel = remaining as usize;
                declarations.push((*binding, value.as_ref().clone()));
            }
        }
        Ok(true)
    })?;
    let mut values = BTreeMap::new();
    let mut selections = BTreeMap::new();
    loop {
        let mut changed = false;
        for (binding, initializer) in &declarations {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("template alias budget exhausted")?;
            if unavailable.contains(binding) || values.contains_key(binding) {
                continue;
            }
            if matches!(
                initializer.kind,
                K::If { .. } | K::Match { .. } | K::Block { .. } | K::CallPreparation { .. }
            ) {
                let mut remaining = *fuel as u64;
                nir::copy_budget::charge_expression(initializer, &mut remaining)
                    .map_err(|_| "template selection alias budget exhausted")?;
                *fuel = remaining as usize;
                let mut candidate = initializer.clone();
                iir::visit::expression_mut_pruned(&mut candidate, fuel, |expr, fuel| {
                    if matches!(expr.kind, K::Closure { .. }) {
                        return Ok(false);
                    }
                    if let K::Local(binding) = expr.kind {
                        if let Some(value) = values.get(&binding) {
                            let mut remaining = *fuel as u64;
                            nir::copy_budget::charge_expression(value, &mut remaining)
                                .map_err(|_| "template selection alias copy budget exhausted")?;
                            *fuel = remaining as usize;
                            let source = expr.source;
                            *expr = value.clone();
                            expr.source = source;
                        }
                    }
                    Ok(true)
                })?;
                if let Some((selection, dispatch)) =
                    super::conditional_calls::freeze_template_selection(
                        &candidate, next, sources, fuel,
                    )?
                {
                    selections.insert(*binding, selection);
                    values.insert(*binding, dispatch);
                    changed = true;
                }
                continue;
            }
            let value = if let K::Local(source) = initializer.kind {
                let Some(value) = values.get(&source) else {
                    continue;
                };
                value
            } else {
                initializer
            };
            let mut remaining = *fuel as u64;
            nir::copy_budget::charge_expression(value, &mut remaining)
                .map_err(|_| "template alias copy budget exhausted")?;
            *fuel = remaining as usize;
            values.insert(*binding, value.clone());
            changed = true;
        }
        if !changed {
            break;
        }
    }
    iir::visit::expression_mut_pruned(expression, fuel, |expr, fuel| {
        if matches!(expr.kind, K::Closure { .. }) {
            return Ok(false);
        }
        if let K::Local(binding) = expr.kind {
            if let Some(value) = values.get(&binding) {
                let mut remaining = *fuel as u64;
                nir::copy_budget::charge_expression(value, &mut remaining)
                    .map_err(|_| "template alias copy budget exhausted")?;
                *fuel = remaining as usize;
                let source = expr.source;
                *expr = value.clone();
                expr.source = source;
            }
        } else if let K::Let { binding, .. } = expr.kind {
            if values.contains_key(&binding) {
                if let Some(selection) = selections.remove(&binding) {
                    *expr = selection;
                } else {
                    expr.kind = K::Unit;
                }
            }
        }
        Ok(true)
    })
}

#[cfg(test)]
mod source_tests;
