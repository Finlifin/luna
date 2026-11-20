//! Reconstruct a completed value as typed NIR in the receiving source world.
//! All fields of one graph share closure bindings until its root is complete.
use super::*;
use comptime_interpretor::ClosureIdentity;
use std::collections::{HashMap, HashSet};

mod captures;

#[derive(Default)]
pub(super) struct Materialization {
    // Generator arguments can contain callbacks anywhere in their owned graph.
    generator: bool,
    type_scope: BTreeSet<DeclarationId>,
    // Owning identities remain valid across all values materialized in this
    // snapshot; no addresses enter NIR, diagnostics or persistent caches.
    closures: HashMap<ClosureIdentity, (n::LocalId, Term)>,
    active: HashSet<ClosureIdentity>,
    pub(super) bindings: Vec<n::Expr>,
}

impl Checker<'_, '_> {
    pub(in crate::program) fn materialize_local_constant(
        &mut self,
        value: Value,
        ty: Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let type_scope = self.local_constant_type_scope(span)?;
        self.materialize_local_constant_in_scope(&value, ty, span, type_scope)
    }

    fn materialize_local_constant_in_scope(
        &mut self,
        value: &Value,
        ty: Term,
        span: Span,
        type_scope: BTreeSet<DeclarationId>,
    ) -> Result<n::Expr, Error> {
        let snapshot = Materialization {
            type_scope,
            ..Materialization::default()
        };
        self.materialize_snapshot(value, ty, span, snapshot)
    }

    fn materialize_snapshot(
        &mut self,
        value: &Value,
        ty: Term,
        span: Span,
        mut snapshot: Materialization,
    ) -> Result<n::Expr, Error> {
        let value = self.constant_expression(value, ty, span, 0, &mut snapshot)?;
        if snapshot.bindings.is_empty() {
            return Ok(value);
        }
        Ok(n::Expr {
            source: super::source(span),
            ty: value.ty.clone(),
            kind: K::Block {
                statements: snapshot.bindings,
                result: Some(Box::new(value)),
            },
        })
    }

    /// Materialize the final argument graph together, after annotation effects.
    /// One sharing table spans all values, including sibling callback inputs.
    pub(in crate::program) fn materialize_call_values(
        &mut self,
        values: Vec<(Value, Term)>,
        span: Span,
    ) -> Result<(Vec<n::Expr>, Vec<n::Expr>), Error> {
        let mut snapshot = Materialization {
            type_scope: self.local_constant_type_scope(span)?,
            ..Materialization::default()
        };
        let values = values
            .into_iter()
            .map(|(value, ty)| self.constant_expression(&value, ty, span, 0, &mut snapshot))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((snapshot.bindings, values))
    }

    pub(in crate::program) fn materialize_generator_data(
        &mut self,
        value: Value,
        ty: Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let snapshot = Materialization {
            generator: true,
            type_scope: self.local_constant_type_scope(span)?,
            ..Materialization::default()
        };
        self.materialize_snapshot(&value, ty, span, snapshot)
    }

    pub(in crate::program) fn freeze_dependency_input(
        &mut self,
        source: &Expr<'_>,
        input: n::Expr,
        bindings: Vec<(n::LocalId, Term, Term)>,
    ) -> Result<(n::Expr, Term), Error> {
        let (value, ty) = self.required_normal_input(source, input, bindings)?;
        self.freeze_completed_dependency(value, ty, source.span)
    }

    /// Reuse a value already evaluated by a packet or ordinary input producer.
    /// In particular, never re-evaluate a selected field outside its packet.
    pub(in crate::program) fn freeze_completed_dependency(
        &mut self,
        value: Value,
        ty: Term,
        span: Span,
    ) -> Result<(n::Expr, Term), Error> {
        // A call-site dependency may retain the caller's Type telescope. It is
        // not an exported, closed module constant; use the same scoped boundary
        // as normal-form inputs, after required evaluation restores those slots.
        if matches!(value, Value::Type(_)) {
            let symbol = self.required_normal_term(value, span)?;
            if ty != p(P::Type) {
                return Err(error(span, "Type dependency requires a Type domain"));
            }
            let kind = K::TypeValue(symbol.clone());
            return Ok((
                n::Expr {
                    source: super::source(span),
                    ty,
                    kind,
                },
                symbol,
            ));
        }
        if let Value::Closure(_) = &value {
            let type_scope = self.local_constant_type_scope(span)?;
            self.validate_closure_snapshot(
                &value,
                span,
                0,
                &mut HashSet::new(),
                &mut HashSet::new(),
                &type_scope,
            )?;
            let expression = self.materialize_local_constant_in_scope(
                &value,
                ty.clone(),
                span,
                type_scope.clone(),
            )?;
            let factory = self.generated_functions.fresh_identity(span)?;
            self.generated_functions.publish(n::Function {
                interface: nir::program::FunctionDeclaration {
                    source: source(span),
                    declaration: factory,
                    name: "<closure-snapshot>".into(),
                    phase: n::FunctionPhase::Comptime,
                    inline: false,
                    pure: false,
                    is_unsafe: false,
                    signature: Term::Function {
                        parameters: vec![],
                        result: Box::new(ty),
                    },
                    parameters: vec![],
                    env_parameters: vec![],
                    preconditions: vec![],
                    postconditions: vec![],
                    entry_snapshots: vec![],
                    result_binding: None,
                    result_type_binder: None,
                    result_guarantees: vec![],
                    type_parameters: type_scope.iter().copied().collect(),
                    requirements: vec![],
                    capabilities: vec![],
                    defaults: vec![],
                },
                body: expression.clone(),
            });
            return Ok((
                expression,
                Term::ClosureSnapshot {
                    factory,
                    arguments: type_scope
                        .into_iter()
                        .map(|declaration| Term::Rigid {
                            declaration,
                            sort: Sort::Type,
                        })
                        .collect(),
                },
            ));
        }
        let symbol = self.required_normal_term(value.clone(), span)?;
        let mut snapshot = Materialization::default();
        let value = self.constant_expression(&value, ty, span, 0, &mut snapshot)?;
        if !snapshot.bindings.is_empty() {
            return Err(error(
                span,
                "dependent input graph materialization is not yet supported",
            ));
        }
        Ok((value, symbol))
    }
    fn charge_constant_type_copy(&mut self, ty: &Term, span: Span) -> Result<(), Error> {
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let checked = nir::type_computations::charge_term(ty, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        checked.map_err(|cause| error(span, cause.to_string()))
    }

    pub(super) fn constant_expression(
        &mut self,
        value: &Value,
        ty: Term,
        span: Span,
        depth: usize,
        snapshot: &mut Materialization,
    ) -> Result<n::Expr, Error> {
        if depth >= 128 || self.required_steps == 0 {
            return Err(error(span, "constant materialization budget exhausted"));
        }
        self.required_steps -= 1;
        let kind = match value {
            Value::Code(_) => {
                return Err(error(
                    span,
                    "contextual code cannot be materialized as ordinary data",
                ));
            }
            Value::List { ty: actual, values } => {
                if actual != &ty {
                    return Err(error(span, "constant list identity mismatch"));
                }
                let element = self
                    .list_element_type(&ty)
                    .ok_or_else(|| error(span, "constant list requires bound meta.List"))?;
                self.required_steps = self
                    .required_steps
                    .checked_sub(values.len() as u64)
                    .ok_or_else(|| error(span, "constant list budget exhausted"))?;
                K::TypeList(
                    values
                        .into_iter()
                        .map(|value| {
                            let domain = self.copy_alias_type(element, span)?;
                            self.constant_expression(value, domain, span, depth + 1, snapshot)
                        })
                        .collect::<Result<_, _>>()?,
                )
            }
            Value::Optional { ty: actual, value } => {
                if actual != &ty {
                    return Err(error(span, "constant optional identity mismatch"));
                }
                let Term::Optional(payload) = &ty else {
                    return Err(error(span, "constant optional type missing"));
                };
                match value {
                    None => K::OptionalAbsent,
                    Some(value) => K::OptionalPresent(Box::new(self.constant_expression(
                        value,
                        (**payload).clone(),
                        span,
                        depth + 1,
                        snapshot,
                    )?)),
                }
            }
            Value::AbstractValue { .. } => {
                return Err(error(
                    span,
                    "abstract comptime value is not a completed constant",
                ));
            }
            Value::ErrorResult {
                ty: actual,
                error: branch,
                value,
            } => {
                if actual != &ty {
                    return Err(error(span, "constant error result identity mismatch"));
                }
                let Term::ErrorResult { errors, result } = &ty else {
                    return Err(error(span, "constant error result type missing"));
                };
                let (conversion, payload) = match branch {
                    None => (nir::error_results::Conversion::Success, result.as_ref()),
                    Some(index) => (
                        nir::error_results::Conversion::Failure,
                        errors
                            .get(*index)
                            .ok_or_else(|| error(span, "constant error result tag invalid"))?,
                    ),
                };
                K::ErrorConvert {
                    conversion,
                    value: Box::new(self.constant_expression(
                        value,
                        payload.clone(),
                        span,
                        depth + 1,
                        snapshot,
                    )?),
                }
            }
            Value::Newtype { ty: actual, value } => {
                if actual != &ty {
                    return Err(error(span, "constant newtype identity mismatch"));
                }
                self.ensure_nominal_type(&ty, span)?;
                let Term::Nominal {
                    declaration,
                    arguments,
                } = &ty
                else {
                    return Err(error(span, "newtype constant requires a nominal type"));
                };
                let definition = self
                    .newtypes
                    .get(declaration)
                    .ok_or_else(|| error(span, "missing newtype descriptor"))?;
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let representation = definition.representation_budgeted(arguments, &[], &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                let representation = representation.map_err(|e| error(span, e.to_string()))?;
                K::NewtypeConvert(Box::new(self.constant_expression(
                    value,
                    representation,
                    span,
                    depth + 1,
                    snapshot,
                )?))
            }
            Value::TypeParameter(declaration)
                if ty == p(P::Type) && snapshot.type_scope.contains(declaration) =>
            {
                K::TypeValue(Term::Rigid {
                    declaration: *declaration,
                    sort: Sort::Type,
                })
            }
            Value::TypeParameter(_) => {
                return Err(error(
                    span,
                    "abstract type input cannot be exported as a completed constant",
                ));
            }
            Value::AnonymousRecord(values) => {
                let shape = self.structural_type(&ty, span)?;
                let Term::Record(types) = shape else {
                    return Err(error(
                        span,
                        "anonymous record constant requires a record type",
                    ));
                };
                if values.len() != types.len()
                    || values
                        .iter()
                        .zip(&types)
                        .any(|((name, _), (expected, _))| name != expected)
                {
                    return Err(error(
                        span,
                        "anonymous record constant fields disagree with its type",
                    ));
                }
                let fields = values
                    .into_iter()
                    .zip(types)
                    .map(|((_, value), (_, ty))| {
                        self.constant_expression(value, ty, span, depth + 1, snapshot)
                    })
                    .collect::<Result<_, _>>()?;
                K::AnonymousRecord { fields }
            }
            Value::Object(object) => {
                if ty != p(P::Object) {
                    return Err(error(span, "Object constant identity mismatch"));
                }
                let entries = object.entries();
                self.required_steps = self
                    .required_steps
                    .checked_sub(entries.len() as u64)
                    .ok_or_else(|| error(span, "constant materialization budget exhausted"))?;
                let mut names = Vec::with_capacity(entries.len());
                let mut fields = Vec::with_capacity(entries.len());
                for entry in entries {
                    let (name, value) = match entry {
                        comptime_interpretor::ObjectEntry::Value(value) => (None, value),
                        comptime_interpretor::ObjectEntry::Property { key, value } => {
                            self.required_steps = self
                                .required_steps
                                .checked_sub(key.len() as u64)
                                .ok_or_else(|| {
                                    error(span, "constant materialization budget exhausted")
                                })?;
                            (Some(key.clone()), value)
                        }
                    };
                    names.push(name);
                    self.charge_constant_type_copy(&value.ty, span)?;
                    fields.push(self.constant_expression(
                        &value.value,
                        value.ty.clone(),
                        span,
                        depth + 1,
                        snapshot,
                    )?);
                }
                K::Object { names, fields }
            }
            Value::Pointer(_) => {
                return Err(error(
                    span,
                    "storage pointer cannot be materialized as a constant",
                ));
            }
            Value::ComptimeReference(_) => {
                return Err(error(
                    span,
                    "active comptime value reference cannot be materialized as a constant",
                ));
            }
            Value::Storage(_) => {
                return Err(error(
                    span,
                    "runtime storage cannot be materialized as a constant",
                ));
            }
            Value::Type(value) => {
                if ty != p(P::Type) || value.sort() != Sort::Type {
                    return Err(error(
                        span,
                        "Type data requires a type identity in the Type domain",
                    ));
                }
                self.charge_constant_type_copy(value, span)?;
                value
                    .check_scope(&self.call_input_frames())
                    .map_err(|cause| error(span, cause.to_string()))?;
                K::TypeValue(value.clone())
            }
            Value::Description(description) => {
                if ty != p(P::Type) {
                    return Err(error(span, "type description requires the Type domain"));
                }
                if !self.refinements.contains(description) {
                    return Err(error(
                        span,
                        "type description belongs to another declaration world",
                    ));
                }
                K::Literal(n::Literal::Description(description.clone()))
            }
            Value::Unit => {
                if !matches!(ty, Term::Primitive(P::Void | P::Unit)) {
                    return Err(error(span, "unit value requires a unit domain"));
                }
                K::Unit
            }
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
                let mut budget = comptime_runtime::Budget {
                    steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                    max_limbs: 4096,
                };
                let before = budget.steps;
                let literal = scalar.freeze_literal(&mut budget);
                self.required_steps -= (before - budget.steps) as u64;
                let literal = literal
                    .map_err(|e| error(span, e.to_string()))?
                    .expect("scalar value representation");
                if !literal.has_primitive_type_in(&ty, self.target) {
                    return Err(error(
                        span,
                        "completed scalar disagrees with its declared domain",
                    ));
                }
                K::Literal(literal)
            }
            Value::Tuple(values) => {
                let shape = self.structural_type(&ty, span)?;
                let Term::Tuple(types) = shape else {
                    return Err(error(span, "constant tuple type mismatch"));
                };
                if values.len() != types.len() {
                    return Err(error(span, "constant tuple arity mismatch"));
                }
                K::Tuple(
                    values
                        .into_iter()
                        .zip(types)
                        .map(|(value, ty)| {
                            self.constant_expression(value, ty, span, depth + 1, snapshot)
                        })
                        .collect::<Result<_, _>>()?,
                )
            }
            Value::Record {
                arguments,
                declaration,
                fields,
            } => {
                if !matches!(&ty, Term::Nominal {declaration: expected, arguments: inputs}
                    if expected == declaration && inputs == arguments)
                {
                    return Err(error(span, "constant record identity mismatch"));
                }
                let definition = self
                    .structs
                    .get(&declaration)
                    .ok_or_else(|| error(span, "constant record descriptor is missing"))?;
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let types = nir::normal_forms::instantiate_value_fields(
                    &definition.type_parameters,
                    &definition.parameter_types,
                    &arguments,
                    definition.fields.iter().map(|(_, ty)| ty),
                    &mut fuel,
                );
                self.required_steps -= (before - fuel) as u64;
                let types = types.ok_or_else(|| {
                    error(
                        span,
                        "constant record layout is invalid or exceeds the materialization budget",
                    )
                })?;
                if fields.len() != types.len() {
                    return Err(error(span, "constant record field count mismatch"));
                }
                K::Record {
                    declaration: *declaration,
                    fields: fields
                        .into_iter()
                        .zip(types)
                        .map(|(value, ty)| {
                            self.constant_expression(value, ty, span, depth + 1, snapshot)
                        })
                        .collect::<Result<_, _>>()?,
                }
            }
            Value::Variant {
                arguments,
                declaration,
                index,
                fields,
            } => {
                if !matches!(&ty, Term::Nominal {declaration: expected, arguments: inputs}
                    if expected == declaration && inputs == arguments)
                {
                    return Err(error(span, "constant enum identity mismatch"));
                }
                let definition = self
                    .enums
                    .get(&declaration)
                    .ok_or_else(|| error(span, "constant enum descriptor is missing"))?;
                let variant = definition
                    .variants
                    .get(*index)
                    .ok_or_else(|| error(span, "constant enum variant is missing"))?;
                if fields.len() != variant.fields.len() {
                    return Err(error(span, "constant enum field count mismatch"));
                }
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let types = nir::normal_forms::instantiate_value_fields(
                    &definition.type_parameters,
                    &definition.parameter_types,
                    &arguments,
                    variant.fields.iter(),
                    &mut fuel,
                );
                self.required_steps -= (before - fuel) as u64;
                let types = types.ok_or_else(|| {
                    error(
                        span,
                        "constant enum layout is invalid or exceeds the materialization budget",
                    )
                })?;
                K::Variant {
                    declaration: *declaration,
                    index: *index,
                    fields: fields
                        .into_iter()
                        .zip(types)
                        .map(|(value, ty)| {
                            self.constant_expression(value, ty, span, depth + 1, snapshot)
                        })
                        .collect::<Result<_, _>>()?,
                }
            }
            Value::Function(declaration) => K::Function(*declaration),
            Value::IirCallable(_) | Value::FunctionInstance { .. } => {
                let (declaration, arguments) = value.declaration_instance().unwrap();
                for argument in arguments {
                    self.charge_constant_type_copy(argument, span)?;
                }
                K::FunctionInstance {
                    declaration,
                    arguments: arguments.to_vec(),
                }
            }
            Value::RuntimeTraitMethod(identity) => {
                return self.materialized_trait_method(identity, &ty, span);
            }
            Value::Closure(closure) => {
                let identity = ClosureIdentity::new(closure);
                if let Some((binding, previous_type)) = snapshot.closures.get(&identity) {
                    if previous_type != &ty {
                        return Err(error(span, "shared constant closure type mismatch"));
                    }
                    return Ok(n::Expr {
                        source: source(span),
                        ty,
                        kind: K::Local(*binding),
                    });
                }
                if snapshot.generator && snapshot.active.is_empty() {
                    self.validate_closure_snapshot(
                        value,
                        span,
                        depth,
                        &mut HashSet::new(),
                        &mut HashSet::new(),
                        &snapshot.type_scope,
                    )?;
                }
                if !snapshot.active.insert(identity.clone()) {
                    return Err(error(
                        span,
                        "cyclic closure environment cannot be materialized as a constant",
                    ));
                }
                let types: &[Term] = match &ty {
                    Term::Closure { environment, .. } => environment,
                    Term::ComptimeFunction(_) => &[],
                    _ => {
                        return Err(error(
                            span,
                            "constant closure has no comptime callable type",
                        ));
                    }
                };
                if types.len() != closure.captured_values().len() {
                    return Err(error(span, "constant closure environment shape mismatch"));
                }
                let mut captures = Vec::new();
                for ((binding, value), capture_type) in closure.captured_values().zip(types) {
                    // Only owned capture cells can be snapshotted. Pointers
                    // and free runtime storage remain invalid constants.
                    self.charge_constant_type_copy(capture_type, span)?;
                    let expression = if let Value::Storage(cell) = value {
                        let mut steps = self.required_steps;
                        let checked = cell.inspect_owned(&mut steps, 128, |value, steps| {
                            self.required_steps = *steps;
                            let result = self.constant_expression(
                                value,
                                capture_type.clone(),
                                span,
                                depth + 1,
                                snapshot,
                            );
                            *steps = self.required_steps;
                            result
                        });
                        self.required_steps = steps;
                        checked.map_err(|e| {
                            error(span, format!("constant capture snapshot failed: {e:?}"))
                        })??
                    } else {
                        self.constant_expression(
                            value,
                            capture_type.clone(),
                            span,
                            depth + 1,
                            snapshot,
                        )?
                    };
                    captures.push(n::Capture {
                        binding,
                        ty: capture_type.clone(),
                        value: Some(Box::new(expression)),
                    });
                }
                let index = self.next_temporary_binding;
                self.next_temporary_binding = index
                    .checked_add(1)
                    .ok_or_else(|| error(span, "constant closure binding space exhausted"))?;
                let binding = n::LocalId {
                    owner: u32::MAX,
                    index,
                };
                let value = n::Expr {
                    source: source(span),
                    ty: ty.clone(),
                    kind: K::Closure {
                        parameters: closure.parameters().to_vec(),
                        captures,
                        // The body is immutable, already elaborated and specialized.
                        body: closure.body().clone(),
                    },
                };
                snapshot.bindings.push(n::Expr {
                    source: source(span),
                    ty: p(P::Void),
                    kind: K::Let {
                        binding,
                        value: Box::new(value),
                    },
                });
                snapshot.active.remove(&identity);
                snapshot.closures.insert(identity, (binding, ty.clone()));
                K::Local(binding)
            }
        };
        Ok(n::Expr {
            source: source(span),
            ty,
            kind,
        })
    }
}
