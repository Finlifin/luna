//! Private required invocation during concrete staged-input preparation.
use super::*;
use comptime_interpretor::{ContractOutcome, Interpreter, Value};

// Reuse checked code and metadata, never an interpreter, environment or result.
// The scratch root and dispatch table are cleared between independent invocations.
pub(super) struct Snapshot {
    package: n::Package,
    entry_index: usize,
}

impl Snapshot {
    fn clear(&mut self) {
        self.package.dispatch_instances.clear();
        let entry = &mut self.package.functions[self.entry_index];
        entry.signature = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(nir::Primitive::Void)),
        };
        entry.body = n::Expr {
            kind: K::Unit,
            ty: Term::Primitive(nir::Primitive::Void),
            source: n::SourceRange { start: 0, end: 0 },
        };
    }
}

#[derive(Clone)]
struct ClosureIdentity(std::sync::Arc<comptime_interpretor::Closure>);
impl PartialEq for ClosureIdentity {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for ClosureIdentity {}
impl std::hash::Hash for ClosureIdentity {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::ptr::hash(std::sync::Arc::as_ptr(&self.0), state);
    }
}

#[derive(Default)]
struct FrozenGraph {
    // Retaining each Arc makes identity stable until the whole snapshot finishes.
    // None marks a currently active closure and detects recursive environments.
    closures: std::collections::HashMap<ClosureIdentity, Option<(n::LocalId, Term)>>,
    bindings: Vec<n::Expr>,
}

impl Builder<'_> {
    pub(super) fn invoke_staged_input(&mut self, expression: &n::Expr) -> Result<n::Expr, String> {
        self.tick(0)?;
        let mut snapshot = match self.invocation_snapshot.take() {
            Some(snapshot) => snapshot,
            None => self.new_invocation_snapshot()?,
        };
        let result = self.invoke_with_snapshot(expression, &mut snapshot);
        snapshot.clear();
        self.invocation_snapshot = Some(snapshot);
        result
    }

    fn new_invocation_snapshot(&mut self) -> Result<Snapshot, String> {
        // The source package is already checked. Preserve its implementation
        // world in this private snapshot; publish only the frozen result.
        for function in self
            .package
            .functions
            .iter()
            .chain(&self.package.quote_templates)
        {
            self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or("staged snapshot budget exhausted")?;
            iir::visit::expressions(function, &mut self.fuel, |_, _| Ok(()))?;
            crate::associated::charge_projection_copy(&function.signature, &mut self.fuel)
                .map_err(|_| "staged snapshot type budget exhausted")?;
        }
        let mut snapshot = self.package.clone();
        // Retained quote bodies still implement trait members in this private
        // world. Preserve their full contracts for interpreter validation;
        // their runtime phase prevents invocation during comptime evaluation.
        // No signature-only identity may erase a quote method precondition.
        snapshot.functions.append(&mut snapshot.quote_templates);
        let mut entry = Id {
            package: u64::MAX,
            local: u32::MAX,
        };
        while snapshot.functions.iter().any(|f| f.declaration == entry)
            || snapshot
                .runtime_function_identities
                .iter()
                .any(|f| f.declaration == entry)
        {
            entry.local = entry
                .local
                .checked_sub(1)
                .ok_or("no staged invocation identity available")?;
        }
        let entry_index = snapshot.functions.len();
        snapshot.functions.push(n::Function {
            declaration: entry,
            name: "<staged-input>".into(),
            phase: n::FunctionPhase::Comptime,
            inline: false,
            pure: false,
            is_unsafe: false,
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(nir::Primitive::Void)),
            },
            parameters: vec![],
            type_parameters: vec![],
            body: n::Expr {
                kind: K::Unit,
                ty: Term::Primitive(nir::Primitive::Void),
                source: n::SourceRange { start: 0, end: 0 },
            },
            defaults: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
        });
        snapshot.dispatch_instances.clear();
        Ok(Snapshot {
            package: snapshot,
            entry_index,
        })
    }

    fn invoke_with_snapshot(
        &mut self,
        expression: &n::Expr,
        scratch: &mut Snapshot,
    ) -> Result<n::Expr, String> {
        iir::visit::expression(expression, &mut self.fuel, |_, _| Ok(()))?;
        crate::associated::charge_projection_copy(&expression.ty, &mut self.fuel)
            .map_err(|_| "staged result type budget exhausted")?;
        let snapshot = &mut scratch.package;
        let root = &mut snapshot.functions[scratch.entry_index];
        root.signature = Term::Function {
            parameters: vec![],
            result: Box::new(expression.ty.clone()),
        };
        root.body = expression.clone();
        let entry = root.declaration;
        let roots = [iir::InstanceKey {
            declaration: entry,
            arguments: vec![],
        }];
        let prepared = build(
            snapshot,
            Budget {
                steps: self.fuel,
                ..self.budget
            },
            Roots::Dispatch(&roots),
            BTreeSet::new(),
            &[],
            Some((self.world.context(), self.world.assumptions())),
            None,
        )?;
        snapshot.dispatch_instances = prepared.0;
        self.fuel = prepared.4;
        let solver = |predicate: &n::TypePredicate| {
            let world = crate::capabilities::World {
                index: self.world.index(),
                implementations: &snapshot.implementations,
                traits: &snapshot.traits,
                assumptions: self.world.assumptions(),
                context: self.world.context(),
            };
            match world.check_predicate(predicate, self.budget) {
                Ok(crate::ptc::Outcome::Entailed(_)) => ContractOutcome::Satisfied,
                Ok(crate::ptc::Outcome::NotEntailed(_)) => ContractOutcome::Rejected,
                _ => ContractOutcome::Unknown,
            }
        };
        let mut interpreter = Interpreter::for_comptime_with_assumptions(
            snapshot,
            self.world.assumptions(),
            comptime_interpretor::Budget {
                steps: self.fuel as u64,
                depth: 128,
            },
        )
        .map_err(|error| format!("staged comptime preparation: {error}"))?
        .with_type_solver(&solver);
        let value = interpreter.invoke(entry, vec![]);
        self.fuel = usize::try_from(interpreter.remaining_steps()).unwrap_or(usize::MAX);
        let value = value.map_err(|error| format!("staged comptime invocation: {error}"))?;
        self.freeze_staged_value(value, &expression.ty, expression.source, 0)
    }

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
        crate::associated::charge_projection_copy(ty, &mut self.fuel)
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
        self.tick(depth)?;
        crate::associated::charge_projection_copy(ty, &mut self.fuel)
            .map_err(|_| "staged value type copy budget exhausted")?;
        let kind = match value {
            Value::FunctionInstance {
                declaration,
                arguments,
            } if !ty.is_comptime_callable()
                && matches!(ty.callable_signature(), Term::Function { .. }) =>
            {
                for argument in arguments {
                    nir::type_computations::charge_term(argument, &mut self.fuel)
                        .map_err(|error| error.to_string())?;
                    iir::completed_type(argument, &BTreeSet::new(), &mut self.fuel)?;
                }
                K::FunctionInstance {
                    declaration: *declaration,
                    arguments: arguments.clone(),
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
                    nir::type_computations::charge_term(argument, &mut self.fuel)
                        .map_err(|error| error.to_string())?;
                    iir::completed_type(argument, &BTreeSet::new(), &mut self.fuel)?;
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
                    && ty.is_comptime_callable() == closure.ty().is_comptime_callable()
                    && closure.captured_values().len() == 0
                    && closure.declaration_instance().is_some() =>
            {
                let (declaration, arguments) = closure.declaration_instance().unwrap();
                for argument in arguments {
                    nir::type_computations::charge_term(argument, &mut self.fuel)
                        .map_err(|error| error.to_string())?;
                    iir::completed_type(argument, &BTreeSet::new(), &mut self.fuel)?;
                }
                K::FunctionInstance {
                    declaration,
                    arguments: arguments.to_vec(),
                }
            }
            Value::Closure(closure)
                if matches!(ty.callable_signature(), Term::Function { .. })
                    && ty.is_comptime_callable() == closure.ty().is_comptime_callable() =>
            {
                let identity = ClosureIdentity(closure.clone());
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
                let mut steps = self.fuel as u64;
                let checked = nir::copy_budget::charge_type_rewrite(closure.body(), &mut steps);
                self.fuel = usize::try_from(steps).unwrap_or(usize::MAX);
                checked.map_err(|error| {
                    format!("staged closure body exceeds copy limits: {error:?}")
                })?;
                self.fuel = self
                    .fuel
                    .checked_sub(capture_types.len() + closure.parameters().len())
                    .ok_or("staged closure environment budget exhausted")?;
                let mut captures = Vec::new();
                for ((binding, value), capture_type) in closure.captured_values().zip(capture_types)
                {
                    let captured = if let Value::Storage(cell) = value {
                        let mut steps = self.fuel as u64;
                        let result = cell.inspect_owned(&mut steps, 128, |value, steps| {
                            self.fuel = usize::try_from(*steps).unwrap_or(usize::MAX);
                            let result = self.freeze_staged_value_in(
                                value,
                                capture_type,
                                source,
                                depth + 1,
                                graph,
                            );
                            *steps = self.fuel as u64;
                            result
                        });
                        self.fuel = usize::try_from(steps).unwrap_or(usize::MAX);
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
                let definition = self
                    .package
                    .newtypes
                    .iter()
                    .find(|definition| definition.declaration == *declaration)
                    .ok_or("staged newtype declaration is missing")?;
                self.fuel = self
                    .fuel
                    .checked_sub(self.package.newtypes.len())
                    .ok_or("staged newtype layout budget exhausted")?;
                let representation = definition
                    .representation_budgeted(arguments, &[], &mut self.fuel)
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
                    steps: self.fuel,
                    max_limbs: 4096,
                };
                let literal = scalar.freeze_literal(&mut budget);
                self.fuel = budget.steps;
                K::Literal(
                    literal
                        .map_err(|e| e.to_string())?
                        .expect("scalar value representation"),
                )
            }
            Value::Type(v) => {
                crate::associated::charge_projection_copy(v, &mut self.fuel)
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
                self.fuel = self
                    .fuel
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
                self.fuel = self
                    .fuel
                    .checked_sub(entries.len())
                    .ok_or("staged Object entry budget exhausted")?;
                let mut names = Vec::with_capacity(entries.len());
                let mut fields = Vec::with_capacity(entries.len());
                for entry in entries {
                    self.fuel = self
                        .fuel
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
                    self.fuel = self
                        .fuel
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
        nir::type_computations::charge_term(ty, &mut self.fuel)
            .map_err(|error| error.to_string())?;
        if !matches!(ty, Term::Nominal { declaration: expected, arguments: expected_arguments }
            if *expected == declaration && expected_arguments == arguments)
        {
            return Err("staged nominal result identity mismatch".into());
        }
        let types = nir::normal_forms::nominal_value_fields_budgeted(
            self.package,
            ty,
            variant,
            &mut self.fuel,
        )
        .ok_or("staged nominal layout is invalid or exceeds the budget")?;
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
            K::Assign { .. } | K::AddressOf(_) | K::Closure { .. } | K::TemplateEqual(..)
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
