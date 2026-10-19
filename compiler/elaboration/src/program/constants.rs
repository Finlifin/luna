//! Completed module and associated constants. Cached expressions reconstruct frozen value
//! graphs; active inference, caller premises and interpreter storage do not enter.
use super::*;
use comptime_interpretor::Value;

#[derive(Default)]
struct Materialization {
    // Nominal generator fields use the same recursive data boundary as tuples.
    generator_fields: bool,
    type_scope: BTreeSet<DeclarationId>,
    // Keys identify interpreter closures only during this one snapshot. Their
    // addresses never enter NIR, diagnostics, persistent caches or dumps.
    closures: BTreeMap<usize, (n::LocalId, Term)>,
    active: BTreeSet<usize>,
    bindings: Vec<n::Expr>,
}

// Initialization evidence, not an executable IR or a live interpreter store.
pub(super) struct StagedPatternSnapshot {
    parts: Vec<(n::Pattern, Term, Term)>,
    staged: BTreeSet<n::LocalId>,
}

// Patterns have already been resolved and checked. Collect their private local
// identities, but check every embedded computation with the same availability
// rules as the surrounding staged argument. Matching itself stays in the VM.
fn staged_pattern_inputs<'a>(
    pattern: &'a n::Pattern,
    bindings: &mut BTreeSet<n::LocalId>,
    expressions: &mut Vec<&'a n::Expr>,
    fuel: &mut u64,
    span: Span,
) -> Result<(), Error> {
    let mut pending = vec![pattern];
    while let Some(pattern) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or_else(|| error(span, "staged pattern structure budget exhausted"))?;
        match pattern {
            n::Pattern::Wild | n::Pattern::Unit | n::Pattern::Literal(_) => {}
            n::Pattern::Tuple(fields)
            | n::Pattern::TypeTuple(fields)
            | n::Pattern::Or(fields)
            | n::Pattern::Record { fields, .. }
            | n::Pattern::AnonymousRecord { fields, .. }
            | n::Pattern::Variant { fields, .. }
            | n::Pattern::NormalForm {
                arguments: fields, ..
            } => pending.extend(fields),
            n::Pattern::Observe { binding, inner } | n::Pattern::Binding { binding, inner } => {
                bindings.insert(*binding);
                pending.push(inner);
            }
            n::Pattern::Remap {
                pattern,
                bindings: remapping,
            } => {
                bindings.extend(remapping.iter().map(|(_, to)| *to));
                pending.push(pattern);
            }
            n::Pattern::Guard { pattern, condition } => {
                pending.push(pattern);
                expressions.push(condition);
            }
            n::Pattern::Then { first, value, next } => {
                pending.push(first);
                expressions.push(value);
                pending.push(next);
            }
        }
    }
    Ok(())
}

// Unknown scalar encodings cannot eliminate a candidate. Known literals use
// the runtime matcher's equality (notably numeric IEEE equality, not bits).
fn completed_literal_mismatch(
    literal: &n::Literal,
    symbol: &Term,
    fuel: &mut usize,
) -> Result<bool, String> {
    nir::type_computations::charge_term(symbol, fuel).map_err(|e| e.to_string())?;
    let mut budget = comptime_runtime::Budget {
        steps: *fuel,
        max_limbs: 4096,
    };
    let result = (|| {
        Ok::<_, comptime_runtime::Error>(match (literal, symbol) {
            (n::Literal::Bool(a), Term::Bool(b)) => a != b,
            (n::Literal::Char(a), Term::Char(b)) => a != b,
            (n::Literal::String(a), Term::String(b)) | (n::Literal::Symbol(a), Term::Symbol(b)) => {
                budget.consume(a.len())?;
                a.as_str() != b.as_ref()
            }
            (n::Literal::Float32(a), Term::Float32(b)) => f32::from_bits(*a) != f32::from_bits(*b),
            (n::Literal::Float64(a), Term::Float64(b)) => f64::from_bits(*a) != f64::from_bits(*b),
            (n::Literal::SignedInteger(a), Term::Integer(b)) => {
                comptime_runtime::Integer::from_i128(*a) != *b
            }
            (n::Literal::UnsignedInteger(a), Term::Integer(b)) => {
                comptime_runtime::Integer::from_u128(*a) != *b
            }
            (n::Literal::Integer(a), Term::Integer(b)) => {
                comptime_runtime::Integer::parse_literal(a, &mut budget)? != *b
            }
            (n::Literal::Float(a), Term::Float(b)) => {
                let expected =
                    comptime_runtime::decimal::DecimalValue::parse_literal(a, &mut budget)?;
                !expected.compare(b, &mut budget)?.is_eq()
            }
            (n::Literal::BitVec { width, words }, Term::BitVec(value)) => {
                budget.consume(words.len())?;
                *width != value.width() || words.as_slice() != value.words()
            }
            _ => false,
        })
    })();
    *fuel = budget.steps;
    result.map_err(|e| e.to_string())
}

// Collect value identities without running pattern observations. An Or keeps
// only identities supplied by every candidate after simultaneous remapping.
fn completed_subject_bindings(
    pattern: &n::Pattern,
    symbol: &Term,
    fuel: &mut usize,
) -> Result<BTreeMap<n::LocalId, Term>, String> {
    enum Work<'a> {
        Visit(&'a n::Pattern, &'a Term),
        Bind(n::LocalId, &'a Term),
        Combine(usize, bool),
        Remap(&'a [(n::LocalId, n::LocalId)]),
    }
    let mut pending = vec![Work::Visit(pattern, symbol)];
    // Possibility is separate from structural value evidence: a failing
    // sibling must not erase another field while checking the success body.
    let mut values: Vec<(bool, BTreeMap<n::LocalId, Term>)> = Vec::new();
    while let Some(work) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("completed pattern evidence budget exhausted")?;
        match work {
            Work::Bind(binding, symbol) => {
                nir::type_computations::charge_term(symbol, fuel).map_err(|e| e.to_string())?;
                values.last_mut().unwrap().1.insert(binding, symbol.clone());
            }
            Work::Combine(count, intersect) => {
                let start = values.len() - count;
                let children = values.drain(start..);
                let mut result: Option<BTreeMap<n::LocalId, Term>> = if intersect {
                    None
                } else {
                    Some(BTreeMap::new())
                };
                let mut possible = !intersect;
                for (child_possible, child) in children {
                    if intersect {
                        if !child_possible {
                            continue;
                        }
                        possible = true;
                    } else {
                        possible &= child_possible;
                    }
                    let Some(current) = result.as_mut() else {
                        result = Some(child);
                        continue;
                    };
                    *fuel = fuel
                        .checked_sub(current.len() + child.len())
                        .ok_or("completed pattern evidence budget exhausted")?;
                    if intersect {
                        for value in current.values().chain(child.values()) {
                            nir::type_computations::charge_term(value, fuel)
                                .map_err(|e| e.to_string())?;
                        }
                        current.retain(|binding, value| child.get(binding) == Some(value));
                    } else {
                        current.extend(child);
                    }
                }
                values.push((possible, result.unwrap_or_default()));
            }
            Work::Remap(mapping) => {
                *fuel = fuel
                    .checked_sub(mapping.len())
                    .ok_or("completed pattern evidence budget exhausted")?;
                let current = &mut values.last_mut().unwrap().1;
                let mut moved = Vec::new();
                for (from, to) in mapping {
                    let value = current.get(from);
                    if let Some(value) = value {
                        nir::type_computations::charge_term(value, fuel)
                            .map_err(|e| e.to_string())?;
                    }
                    moved.push((*to, value.cloned()));
                }
                for (from, _) in mapping {
                    current.remove(from);
                }
                for (to, value) in moved {
                    if let Some(value) = value {
                        current.insert(to, value);
                    } else {
                        current.remove(&to);
                    }
                }
            }
            Work::Visit(pattern, symbol) => {
                if let n::Pattern::Literal(literal) = pattern {
                    if completed_literal_mismatch(literal, symbol, fuel)? {
                        values.push((false, BTreeMap::new()));
                        continue;
                    }
                }
                match pattern {
                    n::Pattern::Binding { binding, inner } => {
                        pending.push(Work::Bind(*binding, symbol));
                        pending.push(Work::Visit(inner, symbol));
                        continue;
                    }
                    n::Pattern::Guard { pattern, .. } | n::Pattern::Then { first: pattern, .. } => {
                        pending.push(Work::Visit(pattern, symbol));
                        continue;
                    }
                    n::Pattern::Remap { pattern, bindings } => {
                        pending.push(Work::Remap(bindings));
                        pending.push(Work::Visit(pattern, symbol));
                        continue;
                    }
                    n::Pattern::Or(patterns) => {
                        *fuel = fuel
                            .checked_sub(patterns.len())
                            .ok_or("completed pattern evidence budget exhausted")?;
                        pending.push(Work::Combine(patterns.len(), true));
                        pending.extend(patterns.iter().map(|pattern| Work::Visit(pattern, symbol)));
                        continue;
                    }
                    _ => {}
                }
                if let (
                    n::Pattern::Variant {
                        declaration, index, ..
                    },
                    Term::NominalValue {
                        ty,
                        variant: Some(actual),
                        ..
                    },
                ) = (pattern, symbol)
                {
                    if matches!(ty.as_ref(), Term::Nominal { declaration: owner, .. } if owner == declaration)
                        && usize::try_from(*actual).ok() != Some(*index)
                    {
                        values.push((false, BTreeMap::new()));
                        continue;
                    }
                }
                let fields: Option<Vec<_>> = match (pattern, symbol) {
                    (n::Pattern::Tuple(patterns), Term::TupleValue(fields))
                        if patterns.len() == fields.len() =>
                    {
                        Some(patterns.iter().zip(fields).collect())
                    }
                    (
                        n::Pattern::AnonymousRecord {
                            names,
                            fields: patterns,
                        },
                        Term::RecordValue(fields),
                    ) if patterns.len() == fields.len()
                        && names.iter().eq(fields.iter().map(|(name, _)| name)) =>
                    {
                        Some(
                            patterns
                                .iter()
                                .zip(fields.iter().map(|(_, value)| value))
                                .collect(),
                        )
                    }
                    (
                        n::Pattern::Record {
                            fields: patterns, ..
                        },
                        Term::NominalValue {
                            variant: None,
                            fields,
                            ..
                        },
                    ) if patterns.len() == fields.len() => {
                        Some(patterns.iter().zip(fields).collect())
                    }
                    (
                        n::Pattern::Variant {
                            declaration,
                            index,
                            fields: patterns,
                        },
                        Term::NominalValue {
                            ty,
                            variant: Some(actual),
                            fields,
                        },
                    ) if matches!(ty.as_ref(), Term::Nominal { declaration: owner, .. } if owner == declaration)
                        && usize::try_from(*actual).ok() == Some(*index)
                        && patterns.len() == fields.len() =>
                    {
                        Some(patterns.iter().zip(fields).collect())
                    }
                    _ => None,
                };
                if let Some(fields) = fields {
                    *fuel = fuel
                        .checked_sub(fields.len())
                        .ok_or("completed pattern evidence budget exhausted")?;
                    pending.push(Work::Combine(fields.len(), false));
                    pending.extend(
                        fields
                            .into_iter()
                            .map(|(pattern, symbol)| Work::Visit(pattern, symbol)),
                    );
                } else {
                    values.push((true, BTreeMap::new()));
                }
            }
        }
    }
    Ok(values.pop().map(|(_, values)| values).unwrap_or_default())
}

impl Checker<'_, '_> {
    pub(super) fn materialize_local_constant(
        &mut self,
        value: Value,
        ty: Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let type_scope = self.local_constant_type_scope(span)?;
        self.materialize_local_constant_in_scope(value, ty, span, type_scope)
    }

    fn materialize_local_constant_in_scope(
        &mut self,
        value: Value,
        ty: Term,
        span: Span,
        type_scope: BTreeSet<DeclarationId>,
    ) -> Result<n::Expr, Error> {
        if matches!(value, Value::Type(_) | Value::TypeList(_)) {
            return self.materialize_generator_data(value, ty, span);
        }
        // Declaration identities are ordinary constant data, not quoted-code
        // evidence. Reuse module-constant reconstruction and phase validation.
        let mut snapshot = Materialization {
            type_scope,
            ..Materialization::default()
        };
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
    pub(super) fn materialize_call_values(
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
            .map(|(value, ty)| self.constant_expression(value, ty, span, 0, &mut snapshot))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((snapshot.bindings, values))
    }

    pub(super) fn materialize_generator_data(
        &mut self,
        value: Value,
        ty: Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        self.materialize_generator_data_at_depth(value, ty, span, 0)
    }

    fn materialize_generator_data_at_depth(
        &mut self,
        value: Value,
        ty: Term,
        span: Span,
        depth: usize,
    ) -> Result<n::Expr, Error> {
        if depth >= 128 {
            return Err(error(
                span,
                "generator data materialization nesting limit exceeded",
            ));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "generator data materialization budget exhausted"))?;
        // Structural containers can hold captureless callbacks. Reconstruct
        // each field through the same data boundary; pointers and captured
        // closure graphs remain rejected recursively.
        if let Value::Tuple(values) = value {
            let Term::Tuple(types) = self.structural_type(&ty, span)? else {
                return Err(error(span, "generator tuple data has no tuple layout"));
            };
            if values.len() != types.len() {
                return Err(error(span, "generator tuple data arity mismatch"));
            }
            let fields = values
                .into_iter()
                .zip(types)
                .map(|(value, ty)| {
                    self.materialize_generator_data_at_depth(value, ty, span, depth + 1)
                })
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(n::Expr {
                ty,
                source: super::source(span),
                kind: K::Tuple(fields),
            });
        }
        if let Value::AnonymousRecord(values) = value {
            let Term::Record(types) = self.structural_type(&ty, span)? else {
                return Err(error(span, "generator record data has no record layout"));
            };
            if values.len() != types.len() {
                return Err(error(span, "generator record data arity mismatch"));
            }
            let fields = values
                .into_iter()
                .zip(types)
                .map(|((name, value), (expected, ty))| {
                    if name != expected {
                        return Err(error(span, "generator record data field mismatch"));
                    }
                    self.materialize_generator_data_at_depth(value, ty, span, depth + 1)
                })
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(n::Expr {
                ty,
                source: super::source(span),
                kind: K::AnonymousRecord { fields },
            });
        }
        if matches!(
            value,
            Value::Record { .. } | Value::Variant { .. } | Value::Newtype { .. }
        ) {
            let mut snapshot = Materialization {
                generator_fields: true,
                ..Materialization::default()
            };
            let result = self.constant_expression(value, ty, span, depth, &mut snapshot)?;
            if !snapshot.bindings.is_empty() {
                return Err(error(
                    span,
                    "generator data graph lifting is not yet implemented",
                ));
            }
            return Ok(result);
        }
        // Local construction data can retain the enclosing Type telescope;
        // immutable module exports use the stricter closed-value boundary.
        if matches!(value, Value::Type(_) | Value::TypeList(_)) {
            let symbol = self.required_normal_term(value, span)?;
            let kind = match symbol {
                Term::TypeListValue(elements) if self.is_type_list_type(&ty) => {
                    K::TypeListValue(elements)
                }
                symbol if ty == p(P::Type) => K::TypeValue(symbol),
                _ => {
                    return Err(error(
                        span,
                        "local Type data does not match its declared domain",
                    ));
                }
            };
            return Ok(n::Expr {
                source: super::source(span),
                ty,
                kind,
            });
        }
        // Captureless comptime callbacks carry no live
        // capture graph. Anonymous bodies and generic instances reuse constant
        // reconstruction as local callable constants.
        let captureless_callable = match &value {
            Value::Function(_) => true,
            Value::Closure(closure) => closure.captured_values().len() == 0,
            _ => false,
        };
        if ty.is_comptime_callable() && captureless_callable {
            return self.materialize_local_constant(value, ty, span);
        }
        if ty.is_comptime_callable() && matches!(value, Value::Closure(_)) {
            let scope = self.local_constant_type_scope(span)?;
            self.validate_closure_snapshot(
                &value,
                span,
                depth,
                &mut BTreeSet::new(),
                &mut BTreeSet::new(),
                &scope,
            )?;
            return self.materialize_local_constant_in_scope(value, ty, span, scope);
        }
        // A data argument is a completed value, not a live interpreter graph.
        // In particular, a nominal wrapper cannot smuggle mutable storage or
        // closure identities across the quote boundary.
        let copy_work=value.immutable_copy_work()
            .ok_or_else(||error(span,"code generator data cannot be frozen: unsupported or excessively deep immutable value"))?;
        self.required_steps = self
            .required_steps
            .checked_sub(copy_work as u64)
            .ok_or_else(|| error(span, "code generator data freezing budget exhausted"))?;
        let mut budget = comptime_runtime::Budget {
            steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
            max_limbs: 4096,
        };
        let before = budget.steps;
        let frozen = value.clone().into_normal_term(&mut budget);
        self.required_steps -= (before - budget.steps) as u64;
        frozen.map_err(|failure| {
            error(
                span,
                format!("code generator data cannot be frozen: {failure:?}"),
            )
        })?;
        let mut snapshot = Materialization::default();
        let value = self.constant_expression(value, ty, span, 0, &mut snapshot)?;
        if !snapshot.bindings.is_empty() {
            return Err(error(
                span,
                "generator data graph lifting is not yet implemented",
            ));
        }
        Ok(value)
    }

    pub(super) fn freeze_dependency_input(
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
    pub(super) fn freeze_completed_dependency(
        &mut self,
        value: Value,
        ty: Term,
        span: Span,
    ) -> Result<(n::Expr, Term), Error> {
        // A call-site dependency may retain the caller's Type telescope. It is
        // not an exported, closed module constant; use the same scoped boundary
        // as normal-form inputs, after required evaluation restores those slots.
        if matches!(value, Value::Type(_) | Value::TypeList(_)) {
            let symbol = self.required_normal_term(value, span)?;
            let kind = match &symbol {
                Term::TypeListValue(elements) => {
                    if !self.is_type_list_type(&ty) {
                        return Err(error(
                            span,
                            "type-list dependency requires the bound meta.List<Type>",
                        ));
                    }
                    K::TypeListValue(elements.clone())
                }
                _ => {
                    if ty != p(P::Type) {
                        return Err(error(span, "Type dependency requires a Type domain"));
                    }
                    K::TypeValue(symbol.clone())
                }
            };
            return Ok((
                n::Expr {
                    source: super::source(span),
                    ty,
                    kind,
                },
                symbol,
            ));
        }
        if let Value::Closure(closure) = &value
            && closure.declaration_instance().is_none()
        {
            let type_scope = self.local_constant_type_scope(span)?;
            self.validate_closure_snapshot(
                &value,
                span,
                0,
                &mut BTreeSet::new(),
                &mut BTreeSet::new(),
                &type_scope,
            )?;
            let expression = self.materialize_local_constant_in_scope(
                value,
                ty.clone(),
                span,
                type_scope.clone(),
            )?;
            let serial = self.next_open_instance;
            self.next_open_instance = serial
                .checked_add(1)
                .ok_or_else(|| error(span, "closure snapshot instance limit exceeded"))?;
            let factory = DeclarationId {
                package: u64::MAX - 1,
                local: serial,
            };
            self.generated_functions.push(n::Function {
                declaration: factory,
                name: "<closure-snapshot>".into(),
                phase: n::FunctionPhase::Comptime,
                inline: false,
                pure: false,
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(ty),
                },
                parameters: vec![],
                body: expression.clone(),
                preconditions: vec![],
                postconditions: vec![],
                entry_snapshots: vec![],
                result_binding: None,
                type_parameters: type_scope.iter().copied().collect(),
                requirements: vec![],
                capabilities: vec![],
                defaults: vec![],
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
        let value = self.constant_expression(value, ty, span, 0, &mut snapshot)?;
        if !snapshot.bindings.is_empty() {
            return Err(error(
                span,
                "dependent input graph materialization is not yet supported",
            ));
        }
        Ok((value, symbol))
    }
    pub(super) fn validate_generator_live_callbacks(
        &mut self,
        callbacks: &[&Value],
        span: Span,
    ) -> Result<(), Error> {
        if callbacks.is_empty() {
            return Ok(());
        }
        let scope = self.local_constant_type_scope(span)?;
        let mut active = BTreeSet::new();
        let mut completed = BTreeSet::new();
        for callback in callbacks {
            self.validate_closure_snapshot(callback, span, 0, &mut active, &mut completed, &scope)?;
        }
        Ok(())
    }

    fn local_constant_type_scope(&mut self, span: Span) -> Result<BTreeSet<DeclarationId>, Error> {
        let mut type_scope: BTreeSet<_> = self
            .signature_owner
            .and_then(|owner| self.type_parameters.get(&owner))
            .into_iter()
            .flat_map(|parameters| self.input_binders(parameters))
            .filter(|parameter| parameter.sort == Sort::Type)
            .map(|parameter| parameter.declaration)
            .collect();
        for argument in self.alias_substitution.values() {
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let checked = nir::type_computations::charge_term(argument, &mut fuel);
            self.required_steps -= (before - fuel) as u64;
            checked.map_err(|cause| error(span, cause.to_string()))?;
            type_scope.extend(
                argument
                    .rigid_parameters()
                    .map_err(|cause| error(span, cause.to_string()))?
                    .into_iter()
                    .filter_map(|(id, sort)| (sort == Sort::Type).then_some(id)),
            );
        }
        Ok(type_scope)
    }

    /// Prove that reconstructing this graph cannot reset state observable by a
    /// callback. Shared nodes are checked once; cycles and mutable capture roots
    /// remain unsupported instead of being silently copied.
    fn validate_closure_snapshot(
        &mut self,
        value: &Value,
        span: Span,
        depth: usize,
        active: &mut BTreeSet<usize>,
        completed: &mut BTreeSet<usize>,
        type_scope: &BTreeSet<DeclarationId>,
    ) -> Result<(), Error> {
        if depth >= 128 || self.required_steps == 0 {
            return Err(error(span, "closure snapshot graph budget exhausted"));
        }
        self.required_steps -= 1;
        match value {
            Value::Storage(cell) => {
                let value = cell
                    .read()
                    .map_err(|cause| error(span, format!("capture read failed: {cause:?}")))?;
                self.validate_closure_snapshot(
                    &value,
                    span,
                    depth + 1,
                    active,
                    completed,
                    type_scope,
                )
            }
            Value::Tuple(fields) | Value::Record { fields, .. } | Value::Variant { fields, .. } => {
                if let Value::Record { arguments, .. } | Value::Variant { arguments, .. } = value {
                    if arguments.len() as u64 > self.required_steps {
                        return Err(error(span, "closure snapshot graph budget exhausted"));
                    }
                    for argument in arguments {
                        let argument = self.copy_alias_type(argument, span)?;
                        self.validate_closure_snapshot(
                            &Value::Type(argument),
                            span,
                            depth + 1,
                            active,
                            completed,
                            type_scope,
                        )?;
                    }
                }
                if fields.len() as u64 > self.required_steps {
                    return Err(error(span, "closure snapshot graph budget exhausted"));
                }
                for field in fields {
                    self.validate_closure_snapshot(
                        field,
                        span,
                        depth + 1,
                        active,
                        completed,
                        type_scope,
                    )?;
                }
                Ok(())
            }
            Value::Newtype { ty, value } => {
                let ty = self.copy_alias_type(ty, span)?;
                self.validate_closure_snapshot(
                    &Value::Type(ty),
                    span,
                    depth + 1,
                    active,
                    completed,
                    type_scope,
                )?;
                self.validate_closure_snapshot(
                    value,
                    span,
                    depth + 1,
                    active,
                    completed,
                    type_scope,
                )
            }
            Value::AnonymousRecord(fields) => {
                if fields.len() as u64 > self.required_steps {
                    return Err(error(span, "closure snapshot graph budget exhausted"));
                }
                for (_, field) in fields {
                    self.validate_closure_snapshot(
                        field,
                        span,
                        depth + 1,
                        active,
                        completed,
                        type_scope,
                    )?;
                }
                Ok(())
            }
            Value::Closure(closure) => {
                if !closure.ty().is_comptime_callable() {
                    return Err(error(span, "closure snapshot requires comptime captures"));
                }
                let identity = std::sync::Arc::as_ptr(closure) as usize;
                if completed.contains(&identity) {
                    return Ok(());
                }
                if !active.insert(identity) {
                    return Err(error(
                        span,
                        "cyclic closure environment cannot be snapshotted",
                    ));
                }
                let result = (|| {
                    if closure.declaration_instance().is_some() {
                        let mut budget = comptime_runtime::Budget {
                            steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                            max_limbs: 4096,
                        };
                        let before = budget.steps;
                        let frozen = value
                            .clone()
                            .into_scoped_invocation_term(&mut budget, type_scope);
                        self.required_steps -= (before - budget.steps) as u64;
                        frozen.map_err(|cause| {
                            error(
                                span,
                                format!(
                                    "closure snapshot requires completed instance inputs: {cause:?}"
                                ),
                            )
                        })?;
                    }
                    let width = closure.captured_values().len();
                    self.required_steps = self
                        .required_steps
                        .checked_sub(width as u64)
                        .ok_or_else(|| error(span, "closure snapshot graph budget exhausted"))?;
                    let captured: BTreeSet<_> =
                        closure.captured_values().map(|(id, _)| id).collect();
                    for (_, capture) in closure.captured_values() {
                        self.validate_closure_snapshot(
                            capture,
                            span,
                            depth + 1,
                            active,
                            completed,
                            type_scope,
                        )?;
                    }
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let before = fuel;
                    let checked = nir::visit::expression_in_closure_environment(
                        closure.body(),
                        &mut fuel,
                        |expr, _| {
                            match &expr.kind {
                            K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. }
                                if captured.contains(&place.root) => return Err("dependent closure input mutates captured state; shared environment transport is not yet implemented".into()),
                            K::AddressOf(place) => {
                                let mut root = place.as_ref();
                                while let K::Field { value, .. } | K::AnonymousField { value, .. } = &root.kind { root = value; }
                                if matches!(&root.kind, K::Local(binding) if captured.contains(binding)) {
                                    return Err("dependent closure input exposes captured storage; shared environment transport is not yet implemented".into());
                                }
                            }
                            _ => {}
                        }
                            Ok(())
                        },
                    );
                    self.required_steps -= (before - fuel) as u64;
                    checked.map_err(|message| error(span, message))
                })();
                active.remove(&identity);
                if result.is_ok() {
                    completed.insert(identity);
                }
                result
            }
            Value::TypeParameter(_)
            | Value::Type(_)
            | Value::TypeList(_)
            | Value::Unit
            | Value::Bool(_)
            | Value::Char(_)
            | Value::String(_)
            | Value::Symbol(_)
            | Value::Integer(_)
            | Value::UnsignedInteger(_)
            | Value::SignedInteger(_)
            | Value::Float(_)
            | Value::Float32(_)
            | Value::Float64(_)
            | Value::BitVec(_) => {
                let mut budget = comptime_runtime::Budget {
                    steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                    max_limbs: 4096,
                };
                let before = budget.steps;
                let frozen = value
                    .clone()
                    .into_scoped_normal_term(&mut budget, type_scope);
                self.required_steps -= (before - budget.steps) as u64;
                frozen.map(|_| ()).map_err(|cause| {
                    error(
                        span,
                        format!("closure snapshot requires completed capture data: {cause:?}"),
                    )
                })
            }
            Value::Function(_) => Ok(()),
            _ => Err(error(
                span,
                "dependent closure input needs an immutable capture graph; shared resources and pointer transport are not yet implemented",
            )),
        }
    }

    pub(super) fn constant_value(&mut self, owner: OwnerId, span: Span) -> Result<n::Expr, Error> {
        // A generic member is a computation in its container's environment,
        // not a completed module constant. Keep it residual until the enclosing
        // required invocation supplies those inputs; never cache it by owner.
        if let Some(parent) = self.self_containers.get(&owner).copied()
            && self
                .package
                .item(owner)
                .is_some_and(|item| matches!(item.kind, ItemKind::Const(..)))
            && self
                .package
                .item(parent)
                .is_some_and(|item| matches!(item.kind, ItemKind::Struct(_) | ItemKind::Enum(_)))
            && self
                .type_parameters
                .get(&parent)
                .is_some_and(|parameters| !parameters.is_empty())
        {
            if self.active_constants.len() >= 64 || !self.active_constants.insert(owner) {
                return Err(error(
                    span,
                    "cyclic or excessively deep constant dependency",
                ));
            }
            let saved_owner = self.signature_owner.replace(owner);
            let saved_context = self.context.clone();
            let saved_mode = std::mem::replace(&mut self.in_required_comptime, true);
            let result = (|| {
                self.ensure_contract(parent, span)?;
                self.context = self.definition_context(parent);
                let ItemKind::Const(annotation, initializer) =
                    self.package.item(owner).unwrap().kind
                else {
                    return Err(error(span, "generic member is not a constant"));
                };
                let expected = if matches!(annotation.kind, H::TyPlaceholder) {
                    None
                } else {
                    Some(self.type_expr(annotation)?)
                };
                let value = self.expr(initializer, expected.as_ref())?;
                // Check the symbolic initializer without selecting dispatch or
                // invoking it: those operations may need the container inputs.
                // A runtime function identity may be carried, but not called.
                let mut fuel = self.required_steps.min(100_000) as usize;
                let before = fuel;
                let checked = nir::visit::expression(&value, &mut fuel, |node, _| {
                    if let K::Call { callee, .. } = &node.kind
                        && !callee.ty.is_comptime_callable()
                    {
                        return Err(
                            "runtime function is unavailable in a comptime constant initializer"
                                .into(),
                        );
                    }
                    Ok(())
                });
                self.required_steps -= (before - fuel) as u64;
                checked.map_err(|message| error(initializer.span, message))?;
                Ok(value)
            })();
            self.signature_owner = saved_owner;
            self.context = saved_context;
            self.in_required_comptime = saved_mode;
            self.active_constants.remove(&owner);
            return result;
        }
        // Trait defaults are evaluated separately for each selected implementation.
        let selected_associated = self.selected_associated_implementation.is_some()
            && matches!(
                self.package.item(owner).map(|item| &item.kind),
                Some(ItemKind::Associated(_))
            );
        if let Some(value) = self.constants.get(&owner).filter(|_| !selected_associated) {
            charge_copy(value, &mut self.required_steps, span)?;
            return Ok(value.clone());
        }
        // Selected associated recursion is keyed by (implementation, member)
        // in selected_associated_value, rather than by the shared default owner.
        if !selected_associated
            && (self.active_constants.len() >= 64 || !self.active_constants.insert(owner))
        {
            return Err(error(
                span,
                "cyclic or excessively deep constant dependency",
            ));
        }
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_checking = std::mem::replace(&mut self.checking_contract, false);
        self.context = self
            .initializer_implementation()
            .and_then(|implementation| self.associated_owner(implementation.declaration))
            .map(|owner| self.definition_context(owner))
            .unwrap_or_else(|| self.nominal_context());
        let result = (|| {
            let item = self.package.item(owner).unwrap();
            let (annotation, initializer) = match &item.kind {
                ItemKind::Const(annotation, initializer) => (*annotation, *initializer),
                ItemKind::Associated(associated) => (
                    associated.ty,
                    associated.default.ok_or_else(|| {
                        error(
                            span,
                            "required associated value needs a selected implementation",
                        )
                    })?,
                ),
                _ => {
                    return Err(error(
                        span,
                        "declaration is not a constant or associated value",
                    ));
                }
            };
            let expected = if matches!(annotation.kind, H::TyPlaceholder) {
                None
            } else {
                Some(self.type_expr(annotation)?)
            };
            let (value, ty) = self.required_typed_expression(initializer, expected.as_ref())?;
            let mut snapshot = Materialization::default();
            let result = self.constant_expression(value, ty, initializer.span, 0, &mut snapshot)?;
            if snapshot.bindings.is_empty() {
                Ok(result)
            } else {
                let span = source(initializer.span);
                let ty = result.ty.clone();
                let body = n::Expr {
                    source: span,
                    ty: ty.clone(),
                    kind: K::Block {
                        statements: snapshot.bindings,
                        result: Some(Box::new(result)),
                    },
                };
                // Reconstruct the graph in a private invocation namespace. A
                // cached graph's binding IDs must never overwrite caller locals.
                let callee = n::Expr {
                    source: span,
                    ty: Term::ComptimeFunction(Box::new(Term::Function {
                        parameters: vec![],
                        result: Box::new(ty.clone()),
                    })),
                    kind: K::Closure {
                        parameters: vec![],
                        captures: vec![],
                        body: std::sync::Arc::new(body),
                    },
                };
                Ok(n::Expr {
                    source: span,
                    ty,
                    kind: K::Call {
                        callee: Box::new(callee),
                        arguments: vec![],
                        parameter_bindings: vec![],
                    },
                })
            }
        })();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.locals = saved_locals;
        self.checking_contract = saved_checking;
        if !selected_associated {
            self.active_constants.remove(&owner);
        }
        if let Ok(value) = &result {
            charge_copy(value, &mut self.required_steps, span)?;
            if !selected_associated {
                self.constants.insert(owner, value.clone());
            }
        }
        result
    }

    pub(super) fn record_staged_pattern(
        &mut self,
        pattern: &n::Pattern,
        value: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        let mut origin = value;
        let mut projections = Vec::new();
        while let K::AnonymousField { value, .. } | K::Field { value, .. } = &origin.kind {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "staged projection provenance budget exhausted"))?;
            projections.push(origin);
            origin = value;
        }
        let dependencies = if let K::Local(input) = origin.kind {
            BTreeSet::from([input])
        } else {
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let result = iir::staging::structural_boolean_dependencies(value, &mut fuel);
            self.required_steps -= (before - fuel) as u64;
            let Some(dependencies) = result.map_err(|e| error(span, e))? else {
                return Ok(());
            };
            dependencies
        };
        if dependencies.iter().any(|input| {
            !self.value_input_bindings.contains_key(input)
                && !self.type_input_bindings.contains_key(input)
                && !self.derived_staged_bindings.contains(input)
        }) {
            return Ok(());
        }
        let mut pending = vec![pattern];
        let mut bindings = Vec::new();
        while let Some(pattern) = pending.pop() {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "staged binding provenance budget exhausted"))?;
            match pattern {
                n::Pattern::Wild => {}
                n::Pattern::Binding { binding, inner } => {
                    bindings.push(*binding);
                    pending.push(inner);
                }
                n::Pattern::Tuple(fields)
                | n::Pattern::AnonymousRecord { fields, .. }
                | n::Pattern::Record { fields, .. } => pending.extend(fields),
                n::Pattern::Guard { pattern, .. } | n::Pattern::Then { first: pattern, .. } => {
                    pending.push(pattern)
                }
                // Computed Then bindings do not come from the subject.
                // Alternative values are compared separately below; neither
                // case erases evidence from structural sibling bindings.
                _ => {}
            }
        }
        self.derived_staged_bindings.extend(bindings);
        // Retain a completed input's value identity for immutable aliases.
        // Unavailable declaration-side Bound inputs remain provenance only.
        if let K::Local(input) = origin.kind {
            if let Some((_, ty, symbol)) = self
                .specialization_inputs
                .iter()
                .find(|(id, _, _)| *id == input)
                .cloned()
            {
                self.require_expression(&origin.ty, &ty, span)?;
                let mut selected = &symbol;
                for projection in projections.iter().rev() {
                    selected = match (&projection.kind, selected) {
                        (K::AnonymousField { name, index, .. }, Term::RecordValue(fields)) => {
                            fields.get(*index).filter(|(label, _)| label == name).map(|(_, value)| value)
                        }
                        (K::Field { value, declaration, index }, Term::NominalValue { ty, variant: None, fields })
                            if **ty == value.ty && matches!(ty.as_ref(), Term::Nominal { declaration: owner, .. } if owner == declaration) => fields.get(*index),
                        _ => None,
                    }.ok_or_else(|| error(span, "completed input projection disagrees with its checked layout"))?;
                }
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let bindings = completed_subject_bindings(pattern, selected, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                for (binding, symbol) in bindings.map_err(|e| error(span, e))? {
                    let ty = self
                        .locals
                        .get(&binding)
                        .cloned()
                        .ok_or_else(|| error(span, "staged alias binding has no domain"))?;
                    if ty == p(P::Type) {
                        self.type_input_bindings.insert(binding, symbol.clone());
                    } else {
                        self.value_input_bindings
                            .insert(binding, (symbol.clone(), ty.clone()));
                    }
                    self.derived_staged_bindings.insert(binding);
                    self.specialization_inputs
                        .retain(|(id, _, _)| *id != binding);
                    self.specialization_inputs.push((binding, ty, symbol));
                }
            }
        }
        Ok(())
    }

    pub(super) fn completed_pattern_inputs(
        &mut self,
        pattern: &n::Pattern,
        value: &n::Expr,
        span: Span,
    ) -> Result<Option<StagedPatternSnapshot>, Error> {
        if self.in_required_comptime || self.specialization_inputs.is_empty() {
            return Ok(None);
        }
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = (|| {
            let mut root = value;
            let mut projections = Vec::new();
            while let K::Field { value, .. } | K::AnonymousField { value, .. } = &root.kind {
                fuel = fuel
                    .checked_sub(1)
                    .ok_or("staged pattern projection budget exhausted")?;
                projections.push(root);
                root = value;
            }
            let K::Local(binding) = root.kind else {
                return Ok(None);
            };
            let Some((_, ty, mut symbol)) = self
                .specialization_inputs
                .iter()
                .find(|(id, _, _)| *id == binding)
                .map(|(id, ty, symbol)| (id, ty, symbol))
            else {
                return Ok(None);
            };
            if *ty != root.ty {
                return Err("staged pattern input domain mismatch".into());
            }
            for projection in projections.iter().rev() {
                symbol = match (&projection.kind, symbol) {
                    (K::AnonymousField {name,index,..}, Term::RecordValue(fields)) => fields.get(*index).filter(|(label,_)|label==name).map(|(_,v)|v),
                    (K::Field {value,declaration,index}, Term::NominalValue {ty,variant:None,fields})
                        if **ty==value.ty && matches!(ty.as_ref(),Term::Nominal {declaration:owner,..} if owner==declaration) => fields.get(*index),
                    _ => None,
                }.ok_or("staged pattern projection layout mismatch")?;
            }
            let metadata = n::Package {
                structs: self.structs.values().cloned().collect(),
                enums: self.enums.values().cloned().collect(),
                newtypes: self.newtypes.values().cloned().collect(),
                normal_forms: self.normal_form_metadata(),
                ..Default::default()
            };
            let frames = vec![self.call_inference_scope().iter().map(|p| p.sort).collect()];
            if !matches!(
                nir::phases::validate_runtime_storage(&value.ty, &metadata, &frames),
                Err("comptime-only value cannot inhabit runtime storage; Splice is required")
            ) {
                return Ok(None);
            }
            // Keep runtime subpatterns intact, including residual subjects and
            // their Drop. Only split containers that have no runtime layout.
            nir::type_computations::charge_term(symbol, &mut fuel).map_err(|e| e.to_string())?;
            let mut pending = vec![(pattern.clone(), value.ty.clone(), symbol.clone())];
            let mut snapshot = StagedPatternSnapshot {
                parts: Vec::new(),
                staged: BTreeSet::new(),
            };
            while let Some((pattern, ty, symbol)) = pending.pop() {
                fuel = fuel
                    .checked_sub(1)
                    .ok_or("staged pattern initialization budget exhausted")?;
                if nir::phases::validate_runtime_storage(&ty, &metadata, &frames).is_ok() {
                    snapshot.parts.push((pattern, ty, symbol));
                    continue;
                }
                if let n::Pattern::Binding { binding, inner } = pattern {
                    snapshot.staged.insert(binding);
                    pending.push((*inner, ty, symbol));
                    continue;
                }
                let shape = self.structural_type(&ty, span).map_err(|e| e.message)?;
                let (types, values) = match (shape, symbol) {
                    (Term::Tuple(types), Term::TupleValue(values)) => (types, values),
                    (Term::Record(types), Term::RecordValue(values)) => {
                        if types
                            .iter()
                            .map(|(n, _)| n)
                            .ne(values.iter().map(|(n, _)| n))
                        {
                            return Err("staged pattern record labels mismatch".into());
                        }
                        (
                            types.into_iter().map(|(_, t)| t).collect(),
                            values.into_iter().map(|(_, v)| v).collect(),
                        )
                    }
                    (
                        _,
                        Term::NominalValue {
                            ty: actual,
                            variant,
                            fields,
                        },
                    ) if *actual == ty => {
                        let types = nir::normal_forms::nominal_value_fields_budgeted(
                            &metadata, &ty, variant, &mut fuel,
                        )
                        .ok_or("staged pattern nominal layout unavailable or budget exhausted")?;
                        (types, fields)
                    }
                    _ if matches!(pattern, n::Pattern::Wild) => continue,
                    _ => return Ok(None),
                };
                let patterns = match pattern {
                    n::Pattern::Wild => vec![n::Pattern::Wild; types.len()],
                    n::Pattern::Tuple(fields)
                    | n::Pattern::Record { fields, .. }
                    | n::Pattern::AnonymousRecord { fields, .. } => fields,
                    _ => return Ok(None),
                };
                if patterns.len() != types.len() || types.len() != values.len() {
                    return Err("staged pattern layout arity mismatch".into());
                }
                pending.extend(
                    patterns
                        .into_iter()
                        .zip(types)
                        .zip(values)
                        .map(|((p, t), v)| (p, t, v))
                        .rev(),
                );
            }
            Ok::<_, String>(Some(snapshot))
        })();
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|e| error(span, e))
    }

    pub(super) fn materialize_mixed_staged_patterns(
        &mut self,
        statements: &mut Vec<n::Expr>,
        tail: Option<&n::Expr>,
        source: &hir::Block<'_>,
        initialized: &BTreeMap<usize, StagedPatternSnapshot>,
    ) -> Result<(), Error> {
        if initialized.is_empty() {
            return Ok(());
        }
        // Plan alias erasure without shifting the initialization snapshots.
        // Consumed forwarding aliases must not keep phase-only fields live.
        // Pure staged subpackets may themselves be forwarding aliases.
        // Preserve only snapshots that own a runtime part; empty snapshots
        // remain eligible for the existing pure, unguarded alias erasure.
        let preserved = initialized
            .iter()
            .filter(|(_, snapshot)| !snapshot.parts.is_empty())
            .map(|(index, _)| *index)
            .collect();
        let removed = self.consumed_staged_aliases(statements, tail, source.span, &preserved)?;
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let used = (|| {
            // Count statement references so replacing a frozen, phase-only
            // subject can release its parent without discarding its guard or
            // continuation. Owning runtime subpackets never enter this queue.
            let mut counts: BTreeMap<n::LocalId, usize> = BTreeMap::new();
            let mut references = BTreeMap::new();
            for (index, statement) in statements.iter().enumerate() {
                if removed.contains(&index) {
                    continue;
                }
                let used = nir::preparation::references(statement, &mut fuel)?;
                fuel = fuel
                    .checked_sub(used.len())
                    .ok_or("staged pattern liveness budget exhausted")?;
                for binding in &used {
                    *counts.entry(*binding).or_default() += 1;
                }
                references.insert(index, used);
            }
            if let Some(tail) = tail {
                for binding in nir::preparation::references(tail, &mut fuel)? {
                    *counts.entry(binding).or_default() += 1;
                }
            }
            let mut pending = Vec::new();
            let mut waiters: BTreeMap<n::LocalId, Vec<usize>> = BTreeMap::new();
            for (index, snapshot) in initialized {
                if removed.contains(index) || !snapshot.parts.is_empty() {
                    continue;
                }
                pending.push(*index);
                fuel = fuel
                    .checked_sub(snapshot.staged.len())
                    .ok_or("staged pattern liveness budget exhausted")?;
                for binding in &snapshot.staged {
                    waiters.entry(*binding).or_default().push(*index);
                }
            }
            let mut replaced = BTreeSet::new();
            while let Some(index) = pending.pop() {
                fuel = fuel
                    .checked_sub(1)
                    .ok_or("staged pattern liveness budget exhausted")?;
                if replaced.contains(&index) {
                    continue;
                }
                let snapshot = &initialized[&index];
                fuel = fuel
                    .checked_sub(snapshot.staged.len())
                    .ok_or("staged pattern liveness budget exhausted")?;
                if snapshot
                    .staged
                    .iter()
                    .any(|binding| counts.get(binding).is_some_and(|count| *count != 0))
                {
                    continue;
                }
                let K::LetPattern {
                    pattern,
                    guard,
                    otherwise,
                    ..
                } = &statements[index].kind
                else {
                    return Err(
                        "staged pattern statement changed before liveness planning".to_string()
                    );
                };
                let mut retained = BTreeSet::new();
                for expression in nir::visit::pattern_observations(pattern, &mut fuel)?
                    .into_iter()
                    .chain(guard.as_deref())
                    .chain(otherwise.as_deref())
                {
                    retained.extend(nir::preparation::references(expression, &mut fuel)?);
                }
                for binding in references[&index].difference(&retained) {
                    fuel = fuel
                        .checked_sub(1)
                        .ok_or("staged pattern liveness budget exhausted")?;
                    let count = counts
                        .get_mut(binding)
                        .ok_or("missing staged pattern reference count")?;
                    *count = count
                        .checked_sub(1)
                        .ok_or("staged pattern reference count underflow")?;
                    if *count == 0 {
                        if let Some(waiting) = waiters.get(binding) {
                            fuel = fuel
                                .checked_sub(waiting.len())
                                .ok_or("staged pattern liveness budget exhausted")?;
                            pending.extend(waiting);
                        }
                    }
                }
                replaced.insert(index);
            }
            Ok::<_, String>(
                counts
                    .into_iter()
                    .filter(|(_, count)| *count != 0)
                    .map(|(binding, _)| binding)
                    .collect::<BTreeSet<_>>(),
            )
        })();
        self.required_steps -= (before - fuel) as u64;
        let used = used.map_err(|e| error(source.span, e))?;
        let mut replacements: BTreeMap<_, Vec<n::Expr>> =
            removed.iter().map(|index| (*index, Vec::new())).collect();
        for (index, snapshot) in initialized {
            if removed.contains(index) {
                continue;
            }
            if snapshot.staged.iter().any(|id| used.contains(id)) {
                continue;
            }
            if snapshot.parts.is_empty()
                && matches!(
                    &statements[*index].kind,
                    K::LetPattern {
                        guard: None,
                        otherwise: None,
                        ..
                    }
                )
            {
                replacements.insert(*index, Vec::new());
                continue;
            }
            let source = &source.stmts[*index];
            let (input_source, guard, otherwise) = match &statements[*index].kind {
                K::LetPattern {
                    value,
                    guard,
                    otherwise,
                    ..
                } => {
                    if let Some(value) = guard {
                        charge_copy(value, &mut self.required_steps, source.span)?;
                    }
                    if let Some(value) = otherwise {
                        charge_copy(value, &mut self.required_steps, source.span)?;
                    }
                    (value.source, guard.clone(), otherwise.clone())
                }
                _ => {
                    return Err(error(
                        source.span,
                        "staged pattern statement changed before materialization",
                    ));
                }
            };
            let mut values = Vec::new();
            for (_, ty, symbol) in &snapshot.parts {
                let binding = n::LocalId {
                    owner: u32::MAX,
                    index: self.next_temporary_binding,
                };
                self.next_temporary_binding =
                    self.next_temporary_binding.checked_add(1).ok_or_else(|| {
                        error(source.span, "staged pattern temporary identity exhausted")
                    })?;
                let root = n::Expr {
                    source: super::source(source.span),
                    ty: ty.clone(),
                    kind: K::Local(binding),
                };
                values.push(self.required_normal_input(
                    source,
                    root,
                    vec![(binding, ty.clone(), symbol.clone())],
                )?);
            }
            let (mut prefix, mut values) = self.materialize_call_values(values, source.span)?;
            // Generated constructors need separate cleanup boundaries. Keep
            // their diagnostic origin without sharing the binding's source id.
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let relocated = (|| {
                for value in &mut values {
                    nir::visit::expression_mut(value, &mut fuel, |expression, _| {
                        let index = u32::try_from(self.expansion_sources.len())
                            .map_err(|_| "staged pattern source identity exhausted".to_string())?;
                        self.expansion_sources.push(n::ExpansionSource {
                            original: expression.source,
                            site: super::source(source.span),
                        });
                        expression.source = n::SourceRange::generated(index);
                        Ok(())
                    })?;
                }
                Ok::<_, String>(())
            })();
            self.required_steps -= (before - fuel) as u64;
            relocated.map_err(|e| error(source.span, e))?;
            let ty =
                self.tuple_value_type(values.iter().map(|v| v.ty.clone()).collect(), source.span)?;
            prefix.push(n::Expr {
                source: super::source(source.span),
                ty: p(P::Void),
                kind: K::LetPattern {
                    pattern: n::Pattern::Tuple(
                        snapshot
                            .parts
                            .iter()
                            .map(|(pattern, _, _)| pattern.clone())
                            .collect(),
                    ),
                    value: Box::new(n::Expr {
                        source: input_source,
                        ty,
                        kind: K::Tuple(values),
                    }),
                    guard,
                    otherwise,
                },
            });
            replacements.insert(*index, prefix);
        }
        if !replacements.is_empty() {
            let mut result = Vec::new();
            for (index, statement) in std::mem::take(statements).into_iter().enumerate() {
                if let Some(replacement) = replacements.remove(&index) {
                    result.extend(replacement);
                } else {
                    result.push(statement);
                }
            }
            *statements = result;
        }
        Ok(())
    }

    // A completed callable input has no runtime storage. Once every use of
    // an immutable alias was consumed by staging, its pure local read can be
    // erased. Keep live/escaped aliases for the normal phase checks; never
    // erase a producer call or a capture initializer here.
    pub(super) fn erase_consumed_staged_aliases(
        &mut self,
        statements: &mut Vec<n::Expr>,
        tail: Option<&n::Expr>,
        span: Span,
    ) -> Result<(), Error> {
        let removed = self.consumed_staged_aliases(statements, tail, span, &BTreeSet::new())?;
        let mut index = 0;
        statements.retain(|_| {
            let keep = !removed.contains(&index);
            index += 1;
            keep
        });
        Ok(())
    }

    fn consumed_staged_aliases(
        &mut self,
        statements: &[n::Expr],
        tail: Option<&n::Expr>,
        span: Span,
        preserved: &BTreeSet<usize>,
    ) -> Result<BTreeSet<usize>, Error> {
        if self.specialization_inputs.is_empty() || !statements.iter().any(|statement| matches!(
            &statement.kind,
            K::Let { value, .. } | K::LetPattern { value, guard: None, otherwise: None, .. }
                if matches!(value.kind, K::Local(_) | K::Field { .. } | K::AnonymousField { .. })
        )) {
            return Ok(BTreeSet::new());
        }
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = (|| {
            let mut used = match tail {
                Some(value) => nir::preparation::references(value, &mut fuel)?,
                None => BTreeSet::new(),
            };
            let mut removed = BTreeSet::new();
            let mut metadata = None;
            for (index, statement) in statements.iter().enumerate().rev() {
                fuel = fuel
                    .checked_sub(1)
                    .ok_or("staged alias erasure budget exhausted")?;
                // Mixed pattern subjects still own runtime fields, including
                // unread ones. Their snapshots must be materialized before
                // deciding whether the resulting runtime binding is unused.
                if preserved.contains(&index) {
                    used.extend(nir::preparation::references(statement, &mut fuel)?);
                    continue;
                }
                let candidate = match &statement.kind {
                    K::Let { binding, value } => Some((vec![*binding], value)),
                    K::LetPattern {
                        pattern,
                        value,
                        guard: None,
                        otherwise: None,
                    } => {
                        let mut pending = vec![pattern];
                        let mut bindings = Vec::new();
                        let mut pure = true;
                        while let Some(pattern) = pending.pop() {
                            fuel = fuel
                                .checked_sub(1)
                                .ok_or("staged pattern erasure budget exhausted")?;
                            match pattern {
                                n::Pattern::Wild => {}
                                n::Pattern::Binding { binding, inner } => {
                                    bindings.push(*binding);
                                    pending.push(inner);
                                }
                                n::Pattern::Tuple(fields)
                                | n::Pattern::Record { fields, .. }
                                | n::Pattern::AnonymousRecord { fields, .. } => {
                                    pending.extend(fields)
                                }
                                _ => {
                                    pure = false;
                                    break;
                                }
                            }
                        }
                        (pure && !bindings.is_empty()).then_some((bindings, value))
                    }
                    _ => None,
                };
                if let Some((bindings, value)) = candidate {
                    if matches!(
                        value.kind,
                        K::Local(_) | K::Field { .. } | K::AnonymousField { .. }
                    ) && bindings.iter().all(|binding| {
                        !used.contains(binding)
                            && self
                                .specialization_inputs
                                .iter()
                                .any(|(id, _, _)| id == binding)
                    }) {
                        // Runtime-capable values retain their ordinary
                        // storage/cleanup behavior even if their origin was
                        // staged. Only erase values that cannot inhabit it.
                        let comptime_only = if value.ty.is_comptime_callable() {
                            true
                        } else {
                            let metadata = metadata.get_or_insert_with(|| n::Package {
                                structs: self.structs.values().cloned().collect(),
                                enums: self.enums.values().cloned().collect(),
                                newtypes: self.newtypes.values().cloned().collect(),
                                normal_forms: self.normal_form_metadata(),
                                ..Default::default()
                            });
                            let frames =
                                vec![self.call_inference_scope().iter().map(|p| p.sort).collect()];
                            matches!(
                                nir::phases::validate_runtime_storage(&value.ty, metadata, &frames),
                                Err(
                                    "comptime-only value cannot inhabit runtime storage; Splice is required"
                                )
                            )
                        };
                        if comptime_only {
                            removed.insert(index);
                            continue;
                        }
                    }
                }
                used.extend(nir::preparation::references(statement, &mut fuel)?);
            }
            Ok::<_, String>(removed)
        })();
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|failure| error(span, failure))
    }

    pub(super) fn check_failure_branch(&mut self, body: &Expr<'_>) -> Result<n::Expr, Error> {
        if self.specialization_inputs.is_empty() {
            return self.expr(body, None);
        }
        let saved = (
            self.specialization_inputs.clone(),
            self.type_input_bindings.clone(),
            self.value_input_bindings.clone(),
        );
        let checked = self.expr(body, None);
        let isolated = (|| {
            let value = match &checked {
                Ok(value) => value,
                Err(_) => return Ok(true),
            };
            if !is_never(&value.ty) {
                return Ok(false);
            }
            let mut loops = BTreeSet::new();
            let mut transfers = BTreeSet::new();
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let result =
                nir::visit::expression_in_closure_environment(value, &mut fuel, |value, _| {
                    match &value.kind {
                        K::Loop { identity, .. } => {
                            loops.insert(*identity);
                        }
                        K::Break(identity) | K::Continue(identity) => {
                            transfers.insert(*identity);
                        }
                        _ => {}
                    }
                    Ok(())
                });
            self.required_steps -= (before - fuel) as u64;
            result.map_err(|e| error(body.span, e))?;
            // A transfer to an enclosing loop can reach later iterations or
            // code after that loop. Keep its invalidations in the outer flow.
            Ok::<_, Error>(transfers.is_subset(&loops))
        })();
        if isolated.as_ref().is_ok_and(|isolated| *isolated) || isolated.is_err() {
            (
                self.specialization_inputs,
                self.type_input_bindings,
                self.value_input_bindings,
            ) = saved;
        }
        isolated?;
        checked
    }

    pub(super) fn invalidate_staged_storage(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        // Comptime execution owns its live store; symbolic declaration inputs
        // are provenance, not cached runtime storage values.
        if self.in_required_comptime || self.specialization_inputs.is_empty() {
            return Ok(());
        }
        let mut invalidated = BTreeSet::new();
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = iir::visit::expression_pruned(value, &mut fuel, |value, fuel| {
            match &value.kind {
                K::Assign { place, .. }
                | K::BitAssign { place, .. }
                | K::BitUpdate { place, .. } => {
                    invalidated.insert(place.root);
                }
                K::AddressOf(value) => {
                    let mut root = value.as_ref();
                    while let K::Field { value, .. } | K::AnonymousField { value, .. } = &root.kind
                    {
                        *fuel = fuel
                            .checked_sub(1)
                            .ok_or("staged address invalidation budget exhausted")?;
                        root = value;
                    }
                    if let K::Local(binding) = root.kind {
                        invalidated.insert(binding);
                    }
                }
                K::Closure { captures, .. } => {
                    for capture in captures {
                        *fuel = fuel
                            .checked_sub(1)
                            .ok_or("staged capture invalidation budget exhausted")?;
                        if capture.value.is_none() {
                            invalidated.insert(capture.binding);
                        }
                    }
                    return Ok(false);
                }
                _ => {}
            }
            Ok(true)
        });
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|failure| error(span, failure))?;
        invalidated.retain(|binding| {
            self.specialization_inputs
                .iter()
                .any(|(id, _, _)| id == binding)
        });
        self.specialization_inputs
            .retain(|(binding, _, _)| !invalidated.contains(binding));
        self.value_input_bindings
            .retain(|binding, _| !invalidated.contains(binding));
        self.type_input_bindings
            .retain(|binding, _| !invalidated.contains(binding));
        // Provenance is not a completed value. Keep it on lexical IR ids so
        // ordinary staged calls can defer to materialization, which checks
        // intervening writes/escapes. Required type formation must instead use
        // the completed evidence removed above and cannot reuse a stale value.
        Ok(())
    }

    pub(super) fn freeze_staged_call_inputs(
        &mut self,
        mut call: n::Expr,
        source: &Expr<'_>,
    ) -> Result<n::Expr, Error> {
        if self.in_required_comptime
            || self.preparing_spread
                == Some(n::SourceRange {
                    start: source.span.lo().0,
                    end: source.span.hi().0,
                })
        {
            return Ok(call);
        }
        let K::Call {
            callee,
            arguments,
            parameter_bindings,
        } = &mut call.kind
        else {
            return Ok(call);
        };
        let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
            return Ok(call);
        };
        if !parameters.iter().any(|p| p.sort != Sort::Runtime) {
            return Ok(call);
        }
        let staged_bindings: BTreeSet<_> = self
            .signature_owner
            .and_then(|owner| self.package.item(owner))
            .map(|item| match &item.kind {
                ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) => sig
                    .params
                    .iter()
                    .filter(|parameter| parameter.ty.is_comptime())
                    .map(|parameter| local(parameter.hir_id))
                    .collect(),
                _ => BTreeSet::new(),
            })
            .unwrap_or_default();
        let type_bindings = self
            .signature_owner
            .and_then(|owner| self.type_parameters.get(&owner))
            .cloned()
            .unwrap_or_default();
        let mut inputs = Vec::new();
        let mut prepared: BTreeMap<n::LocalId, n::Expr> = BTreeMap::new();
        let mut deferred_bindings = BTreeSet::new();
        for argument in arguments {
            if parameters[argument.parameter].sort == Sort::Runtime {
                continue;
            }
            if argument.is_default {
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let replaced = iir::staging::prepare_input_with_layouts(
                    iir::staging::Layouts::from_index(&self.normal_form_metadata(), &self.newtypes),
                    &argument.value,
                    &prepared,
                    &mut fuel,
                );
                self.required_steps -= (before - fuel) as u64;
                argument.value = replaced.map_err(|e| error(source.span, e))?;
            }
            let deferred = self.staged_input_can_defer(
                &argument.value,
                argument.is_default,
                &deferred_bindings,
                &staged_bindings,
                &type_bindings,
                source.span,
            )?;
            if deferred {
                if let Some(binding) = parameter_bindings.get(argument.parameter) {
                    deferred_bindings.insert(*binding);
                }
                continue;
            }
            let bindings = if argument.is_default {
                inputs.clone()
            } else {
                vec![]
            };
            let (value, ty) =
                self.required_normal_input(source, argument.value.clone(), bindings)?;
            let mut budget = comptime_runtime::Budget {
                steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                max_limbs: 4096,
            };
            let before = budget.steps;
            // This is an invocation input, not an exported data constant.
            // Preserve named comptime callable identity through specialization.
            let symbolic = value
                .clone()
                .into_scoped_invocation_term(&mut budget, &BTreeSet::new());
            self.required_steps -= (before - budget.steps) as u64;
            let symbolic = symbolic.map_err(|e| {
                error(
                    source.span,
                    format!("staged argument cannot be frozen: {e:?}"),
                )
            })?;
            let mut snapshot = Materialization::default();
            argument.value =
                self.constant_expression(value, ty.clone(), source.span, 0, &mut snapshot)?;
            if !snapshot.bindings.is_empty() {
                return Err(error(
                    source.span,
                    "staged argument graph materialization is not yet supported",
                ));
            }
            if let Some(binding) = parameter_bindings.get(argument.parameter) {
                inputs.push((*binding, ty, symbolic));
                prepared.insert(*binding, argument.value.clone());
            }
        }
        Ok(call)
    }

    fn staged_input_can_defer(
        &mut self,
        value: &n::Expr,
        is_default: bool,
        deferred_bindings: &BTreeSet<n::LocalId>,
        staged_bindings: &BTreeSet<n::LocalId>,
        type_bindings: &[nir::DeclarationId],
        span: Span,
    ) -> Result<bool, Error> {
        let mut deferred = false;
        let mut structural = true;
        let mut block_bindings = BTreeSet::new();
        let mut local_loops = BTreeSet::new();
        let mut pending = vec![value];
        while let Some(value) = pending.pop() {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "staged input structure budget exhausted"))?;
            match &value.kind {
                K::Local(binding) if is_default && deferred_bindings.contains(binding) => {
                    deferred = true
                }
                K::Local(binding)
                    if staged_bindings.contains(binding)
                        || self.derived_staged_bindings.contains(binding) =>
                {
                    deferred = true
                }
                K::Local(binding) if block_bindings.contains(binding) => {}
                K::Local(binding) if self.required_runtime_parameter(*binding) => {
                    return Err(error(
                        span,
                        "required comptime computation cannot read a runtime parameter; defer does not change the parameter's phase",
                    ));
                }
                K::Call {
                    callee, arguments, ..
                } if callee.ty.is_comptime_callable() => {
                    pending.push(callee);
                    pending.extend(arguments.iter().map(|argument| &argument.value));
                }
                K::Call { .. } => {
                    return Err(error(
                        span,
                        "callable type is unavailable in this execution phase: runtime call in a comptime input",
                    ));
                }
                K::Function(_) if value.ty.is_comptime_callable() => {}
                K::Closure { captures, .. } if value.ty.is_comptime_callable() => {
                    // The body is separately phase-checked. Its parameters
                    // belong to invocation; only environment construction
                    // reads the surrounding stage here. Derived locals use
                    // the same provenance as direct argument references;
                    // specialization still checks writes/address escape
                    // before installing an owned capture snapshot.
                    deferred |= type_bindings.iter().any(|id| value.ty.mentions_rigid(*id));
                    for capture in captures {
                        self.required_steps =
                            self.required_steps.checked_sub(1).ok_or_else(|| {
                                error(span, "staged capture structure budget exhausted")
                            })?;
                        if let Some(value) = &capture.value {
                            pending.push(value);
                        } else if staged_bindings.contains(&capture.binding)
                            || self.derived_staged_bindings.contains(&capture.binding)
                            || (is_default && deferred_bindings.contains(&capture.binding))
                        {
                            deferred = true;
                        } else if !block_bindings.contains(&capture.binding) {
                            structural = false;
                        }
                    }
                }
                K::FunctionInstance { arguments, .. } if value.ty.is_comptime_callable() => {
                    deferred |= arguments
                        .iter()
                        .any(|ty| type_bindings.iter().any(|id| ty.mentions_rigid(*id)));
                }
                K::TraitMethod {
                    subject,
                    interface_arguments,
                    method_arguments,
                    ..
                } if value.ty.is_comptime_callable() => {
                    deferred |= std::iter::once(subject)
                        .chain(interface_arguments)
                        .chain(method_arguments)
                        .any(|ty| type_bindings.iter().any(|id| ty.mentions_rigid(*id)));
                }
                K::AssociatedValue {
                    interface, subject, ..
                } => {
                    deferred |= std::iter::once(subject)
                        .chain(&interface.arguments)
                        .any(|ty| type_bindings.iter().any(|id| ty.mentions_rigid(*id)));
                }
                K::TypeList(values)
                | K::Tuple(values)
                | K::Record { fields: values, .. }
                | K::AnonymousRecord { fields: values }
                | K::Variant { fields: values, .. } => pending.extend(values),
                K::Not(value)
                | K::Discard(value)
                | K::RequirePredicate(value)
                | K::EnsurePredicate(value)
                | K::CheckPredicate(value)
                | K::Snapshot(value)
                | K::TakeBits(value)
                | K::SymbolName(value)
                | K::EnumName(value)
                | K::BitConvert(value)
                | K::NewtypeConvert(value)
                | K::BitSlice { value, .. }
                | K::Field { value, .. }
                | K::AnonymousField { value, .. } => pending.push(value),
                // Place fields are checked structural projections, never
                // pointer dereferences or computations on caller storage.
                K::Assign { place, value } | K::BitAssign { place, value, .. }
                    if block_bindings.contains(&place.root) =>
                {
                    pending.push(value)
                }
                K::BitUpdate {
                    place,
                    selections,
                    value,
                    ..
                } if block_bindings.contains(&place.root) => {
                    pending.extend(selections.iter().flat_map(n::BitSelection::expressions));
                    pending.push(value);
                }
                K::BitWindow { value, start, end } => {
                    pending.extend([value.as_ref(), start.as_ref(), end.as_ref()])
                }
                K::BitEqual(left, right) | K::BitConcat(left, right) | K::BitIndex(left, right) => {
                    pending.push(left);
                    pending.push(right);
                }
                K::And(left, right) | K::Or(left, right)
                    if matches!(value.ty, Term::Primitive(P::BitVec(_))) =>
                {
                    pending.push(left);
                    pending.push(right);
                }
                K::Loop { identity, body } => {
                    local_loops.insert(*identity);
                    pending.push(body);
                }
                K::Break(identity) | K::Continue(identity) if local_loops.contains(identity) => {}
                K::And(left, right)
                | K::Or(left, right)
                | K::ScalarEqual(left, right)
                | K::ScalarLess(left, right)
                | K::ScalarLessEqual(left, right)
                | K::ScalarGreater(left, right)
                | K::ScalarGreaterEqual(left, right)
                    if value.ty == p(P::Bool) =>
                {
                    pending.push(left);
                    pending.push(right);
                }
                K::FloatNegate(inner) if matches!(value.ty, Term::Primitive(P::F32 | P::F64)) => {
                    pending.push(inner)
                }
                K::FloatBinary { left, right, .. }
                    if matches!(value.ty, Term::Primitive(P::F32 | P::F64)) =>
                {
                    pending.push(left);
                    pending.push(right);
                }
                K::IntegerNegate(inner)
                    if n::machine_integer_type(value).is_some() || value.ty == p(P::Integer) =>
                {
                    pending.push(inner)
                }
                K::IntegerBinary { op, left, right }
                    if value.ty == p(P::Integer)
                        || (op.is_comparison() && value.ty == p(P::Bool))
                        || (n::machine_integer_type(value).is_some()
                            && matches!(
                                op,
                                n::IntegerOp::Add
                                    | n::IntegerOp::Subtract
                                    | n::IntegerOp::Multiply
                                    | n::IntegerOp::DivideTrunc
                                    | n::IntegerOp::RemainderTrunc
                            )) =>
                {
                    pending.push(left);
                    pending.push(right);
                }
                K::If {
                    condition,
                    then_branch,
                    else_branch,
                } => {
                    pending.push(condition);
                    pending.push(then_branch);
                    pending.extend(else_branch.as_deref());
                }
                K::Match { value, arms } => {
                    pending.push(value);
                    for arm in arms {
                        staged_pattern_inputs(
                            &arm.pattern,
                            &mut block_bindings,
                            &mut pending,
                            &mut self.required_steps,
                            span,
                        )?;
                        pending.extend(arm.guard.as_ref());
                        pending.push(&arm.body);
                    }
                }
                K::Block { statements, result } | K::CallPreparation { statements, result } => {
                    // These are already resolved, uniquely identified locals.
                    // Only these block-owned locals may be updated. Captured
                    // caller storage remains outside staged input evaluation.
                    pending.extend(result.as_deref());
                    for statement in statements.iter().rev() {
                        if let K::Let { binding, value } = &statement.kind {
                            block_bindings.insert(*binding);
                            pending.push(value);
                        } else {
                            pending.push(statement);
                        }
                    }
                }
                K::LetPattern {
                    pattern,
                    value,
                    guard,
                    otherwise,
                } => {
                    staged_pattern_inputs(
                        pattern,
                        &mut block_bindings,
                        &mut pending,
                        &mut self.required_steps,
                        span,
                    )?;
                    pending.push(value);
                    pending.extend(guard.as_deref());
                    pending.extend(otherwise.as_deref());
                }
                K::TypeValue(ty) => {
                    deferred |= type_bindings.iter().any(|id| ty.mentions_rigid(*id));
                    deferred |= !ty
                        .substitution_parameters()
                        .map_err(|e| error(span, e.to_string()))?
                        .is_empty();
                }
                K::TypeListValue(types) => {
                    for ty in types {
                        deferred |= type_bindings.iter().any(|id| ty.mentions_rigid(*id));
                        deferred |= !ty
                            .substitution_parameters()
                            .map_err(|e| error(span, e.to_string()))?
                            .is_empty();
                    }
                }
                K::NormalFormType {
                    arguments, plan, ..
                } => {
                    if let Some(plan) = plan {
                        if !plan.valid(arguments.len()) {
                            structural = false;
                            break;
                        }
                        block_bindings.extend(plan.local_bindings().unwrap());
                        pending.extend(plan.preparations.iter().map(|p| &p.expression));
                    }
                    for argument in arguments {
                        for pattern in argument.patterns() {
                            staged_pattern_inputs(
                                pattern,
                                &mut block_bindings,
                                &mut pending,
                                &mut self.required_steps,
                                span,
                            )?;
                        }
                        pending.extend(argument.expressions());
                    }
                }
                K::Literal(_) | K::Unit => {}
                _ => {
                    structural = false;
                    break;
                }
            }
        }
        Ok(deferred && structural)
    }

    pub(super) fn freeze_staged_preparation(
        &mut self,
        mut expression: n::Expr,
        source: &Expr<'_>,
    ) -> Result<n::Expr, Error> {
        if self.in_required_comptime {
            return Ok(expression);
        }
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let plan = iir::staging::preparation_with_layouts(
            iir::staging::Layouts::from_index(&self.normal_form_metadata(), &self.newtypes),
            &expression,
            &mut fuel,
        );
        self.required_steps -= (before - fuel) as u64;
        let Some(plan) = plan.map_err(|e| error(source.span, e))? else {
            return Ok(expression);
        };
        let staged_bindings: BTreeSet<_> = self
            .signature_owner
            .and_then(|owner| self.package.item(owner))
            .map(|item| match &item.kind {
                ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) => sig
                    .params
                    .iter()
                    .filter(|parameter| parameter.ty.is_comptime())
                    .map(|parameter| local(parameter.hir_id))
                    .collect(),
                _ => BTreeSet::new(),
            })
            .unwrap_or_default();
        let type_bindings = self
            .signature_owner
            .and_then(|owner| self.type_parameters.get(&owner))
            .cloned()
            .unwrap_or_default();
        if self.staged_input_can_defer(
            &plan.expression,
            false,
            &BTreeSet::new(),
            &staged_bindings,
            &type_bindings,
            source.span,
        )? {
            return Ok(expression);
        };
        charge_copy(&plan.expression, &mut self.required_steps, source.span)?;
        let (value, ty) = self.required_normal_input(source, plan.expression.clone(), vec![])?;
        let mut snapshot = Materialization::default();
        let completed = self.constant_expression(value, ty, source.span, 0, &mut snapshot)?;
        if !snapshot.bindings.is_empty() {
            return Err(error(
                source.span,
                "staged preparation graph materialization is not yet supported",
            ));
        }
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = plan.apply_with_layouts(
            iir::staging::Layouts::from_index(&self.normal_form_metadata(), &self.newtypes),
            &mut expression,
            completed,
            &mut fuel,
        );
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|e| error(source.span, e))?;
        Ok(expression)
    }

    fn constant_nominal_field(
        &mut self,
        value: Value,
        ty: Term,
        span: Span,
        depth: usize,
        snapshot: &mut Materialization,
    ) -> Result<n::Expr, Error> {
        if snapshot.generator_fields {
            self.materialize_generator_data_at_depth(value, ty, span, depth)
        } else {
            self.constant_expression(value, ty, span, depth, snapshot)
        }
    }

    fn constant_expression(
        &mut self,
        value: Value,
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
            Value::AbstractValue { .. } => {
                return Err(error(
                    span,
                    "abstract comptime value is not a completed constant",
                ));
            }
            Value::Newtype { ty: actual, value } => {
                if actual != ty {
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
                K::NewtypeConvert(Box::new(self.constant_nominal_field(
                    *value,
                    representation,
                    span,
                    depth + 1,
                    snapshot,
                )?))
            }
            Value::TypeParameter(declaration) if snapshot.type_scope.contains(&declaration) => {
                K::TypeValue(Term::Rigid {
                    declaration,
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
                let Term::Record(types) = &ty else {
                    return Err(error(
                        span,
                        "anonymous record constant requires a record type",
                    ));
                };
                if values.len() != types.len()
                    || values
                        .iter()
                        .zip(types)
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
                        self.constant_expression(value, ty.clone(), span, depth + 1, snapshot)
                    })
                    .collect::<Result<_, _>>()?;
                K::AnonymousRecord { fields }
            }
            Value::Pointer(_) => {
                return Err(error(
                    span,
                    "storage pointer cannot be materialized as a constant",
                ));
            }
            Value::Storage(_) => {
                return Err(error(
                    span,
                    "runtime storage cannot be materialized as a constant",
                ));
            }
            Value::Type(value) => K::TypeValue(value),
            Value::TypeList(types) => {
                if !self.is_type_list_type(&ty) {
                    return Err(error(
                        span,
                        "type-list constant requires the bound meta.List<Type>",
                    ));
                }
                for element in &types {
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let before = fuel;
                    let checked = iir::completed_type(element, &BTreeSet::new(), &mut fuel);
                    self.required_steps -= (before - fuel) as u64;
                    checked.map_err(|e| error(span, e))?;
                    if element.sort() != Sort::Type {
                        return Err(error(span, "type-list element is not Type"));
                    }
                }
                K::TypeListValue(types)
            }
            Value::Unit => K::Unit,
            Value::Bool(value) => K::Literal(n::Literal::Bool(value)),
            Value::Char(value) => K::Literal(n::Literal::Char(value)),
            Value::Symbol(value) => {
                self.required_steps = self
                    .required_steps
                    .checked_sub(value.len() as u64)
                    .ok_or_else(|| error(span, "constant materialization budget exhausted"))?;
                K::Literal(n::Literal::Symbol(value.to_string()))
            }
            Value::String(value) => {
                self.required_steps = self
                    .required_steps
                    .checked_sub(value.len() as u64)
                    .ok_or_else(|| error(span, "constant materialization budget exhausted"))?;
                K::Literal(n::Literal::String(value.to_string()))
            }
            Value::SignedInteger(value) => K::Literal(n::Literal::SignedInteger(value)),
            Value::UnsignedInteger(value) => K::Literal(n::Literal::UnsignedInteger(value)),
            Value::Float32(value) => K::Literal(n::Literal::Float32(value)),
            Value::Float64(value) => K::Literal(n::Literal::Float64(value)),
            Value::Float(value) => {
                if ty != p(P::Float) {
                    return Err(error(
                        span,
                        "exact Float constant requires the Float domain",
                    ));
                }
                let mut budget = comptime_runtime::Budget {
                    steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                    max_limbs: 4096,
                };
                let start = budget.steps;
                let spelling = value.to_literal(&mut budget);
                self.required_steps -= (start - budget.steps) as u64;
                K::Literal(n::Literal::Float(
                    spelling.map_err(|e| error(span, e.to_string()))?,
                ))
            }
            Value::Integer(value) => {
                let mut budget = comptime_runtime::Budget {
                    steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                    max_limbs: 4096,
                };
                let start = budget.steps;
                let spelling = value.to_hex_literal(&mut budget);
                self.required_steps -= (start - budget.steps) as u64;
                K::Literal(n::Literal::Integer(
                    spelling.map_err(|e| error(span, e.to_string()))?,
                ))
            }
            Value::BitVec(value) => K::Literal(n::Literal::BitVec {
                width: value.width(),
                words: value.words().to_vec(),
            }),
            Value::Tuple(values) => {
                let shape = self.structural_type(&ty, span)?;
                let Term::Tuple(types) = &shape else {
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
                            self.constant_expression(value, ty.clone(), span, depth + 1, snapshot)
                        })
                        .collect::<Result<_, _>>()?,
                )
            }
            Value::Record {
                arguments,
                declaration,
                fields,
            } => {
                if ty
                    != (Term::Nominal {
                        declaration,
                        arguments: arguments.clone(),
                    })
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
                    declaration,
                    fields: fields
                        .into_iter()
                        .zip(types)
                        .map(|(value, ty)| {
                            self.constant_nominal_field(value, ty, span, depth + 1, snapshot)
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
                if ty
                    != (Term::Nominal {
                        declaration,
                        arguments: arguments.clone(),
                    })
                {
                    return Err(error(span, "constant enum identity mismatch"));
                }
                let definition = self
                    .enums
                    .get(&declaration)
                    .ok_or_else(|| error(span, "constant enum descriptor is missing"))?;
                let variant = definition
                    .variants
                    .get(index)
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
                    declaration,
                    index,
                    fields: fields
                        .into_iter()
                        .zip(types)
                        .map(|(value, ty)| {
                            self.constant_nominal_field(value, ty, span, depth + 1, snapshot)
                        })
                        .collect::<Result<_, _>>()?,
                }
            }
            Value::Function(declaration) => K::Function(declaration),
            Value::RuntimeFunction {
                declaration,
                arguments,
            } => K::FunctionInstance {
                declaration,
                arguments,
            },
            Value::RuntimeTraitMethod(identity) => {
                let comptime_interpretor::TraitMethodIdentity {
                    interface,
                    subject,
                    member,
                    method_arguments,
                } = *identity;
                K::TraitMethod {
                    interface: interface.declaration,
                    interface_arguments: interface.arguments,
                    subject,
                    member,
                    method_arguments,
                }
            }
            Value::Closure(closure) => {
                if let Some((declaration, arguments)) = closure.declaration_instance() {
                    K::FunctionInstance {
                        declaration,
                        arguments: arguments.to_vec(),
                    }
                } else {
                    let identity = std::sync::Arc::as_ptr(&closure) as usize;
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
                    if !snapshot.active.insert(identity) {
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
                        let value = if let Value::Storage(cell) = value {
                            cell.read().map_err(|e| {
                                error(span, format!("constant capture snapshot failed: {e:?}"))
                            })?
                        } else {
                            value.clone()
                        };
                        captures.push(n::Capture {
                            binding,
                            ty: capture_type.clone(),
                            value: Some(Box::new(self.constant_expression(
                                value,
                                capture_type.clone(),
                                span,
                                depth + 1,
                                snapshot,
                            )?)),
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
            }
        };
        Ok(n::Expr {
            source: source(span),
            ty,
            kind,
        })
    }
}

pub(super) fn charge_copy(value: &n::Expr, steps: &mut u64, span: Span) -> Result<(), Error> {
    enum Node<'a> {
        Expr(&'a n::Expr),
        Type(&'a Term),
    }
    let mut work = vec![(Node::Expr(value), 0usize)];
    while let Some((node, depth)) = work.pop() {
        if depth >= 128 || *steps == 0 {
            return Err(error(span, "constant copy budget exhausted"));
        }
        *steps -= 1;
        let size = match node {
            Node::Expr(expr) => {
                work.push((Node::Type(&expr.ty), depth + 1));
                match &expr.kind {
                    K::Call {
                        callee,
                        arguments,
                        parameter_bindings,
                    } => {
                        work.push((Node::Expr(callee), depth + 1));
                        work.extend(
                            arguments
                                .iter()
                                .map(|argument| (Node::Expr(&argument.value), depth + 1)),
                        );
                        parameter_bindings.len()
                    }
                    K::Block { statements, result } | K::CallPreparation { statements, result } => {
                        work.extend(statements.iter().map(|expr| (Node::Expr(expr), depth + 1)));
                        if let Some(result) = result {
                            work.push((Node::Expr(result), depth + 1));
                        }
                        0
                    }
                    K::Let { value, .. } => {
                        work.push((Node::Expr(value), depth + 1));
                        0
                    }
                    K::Closure {
                        parameters,
                        captures,
                        ..
                    } => {
                        for capture in captures {
                            work.push((Node::Type(&capture.ty), depth + 1));
                            if let Some(value) = &capture.value {
                                work.push((Node::Expr(value), depth + 1));
                            }
                        }
                        // Expr::clone shares the immutable Arc body. Captured
                        // initializers and parameter bindings are actual copies.
                        parameters.len()
                    }
                    K::FunctionInstance { arguments, .. } => {
                        work.extend(arguments.iter().map(|ty| (Node::Type(ty), depth + 1)));
                        0
                    }
                    K::TraitMethod {
                        subject,
                        interface_arguments,
                        method_arguments,
                        ..
                    } => {
                        work.push((Node::Type(subject), depth + 1));
                        work.extend(
                            interface_arguments
                                .iter()
                                .chain(method_arguments)
                                .map(|ty| (Node::Type(ty), depth + 1)),
                        );
                        0
                    }
                    K::TypeValue(ty) => {
                        work.push((Node::Type(ty), depth + 1));
                        0
                    }
                    K::TypeListValue(types) => {
                        work.extend(types.iter().map(|ty| (Node::Type(ty), depth + 1)));
                        0
                    }
                    K::TypeList(items)
                    | K::Tuple(items)
                    | K::AnonymousRecord { fields: items }
                    | K::Record { fields: items, .. }
                    | K::Variant { fields: items, .. } => {
                        work.extend(items.iter().map(|e| (Node::Expr(e), depth + 1)));
                        0
                    }
                    K::Literal(
                        n::Literal::Symbol(s) | n::Literal::String(s) | n::Literal::Integer(s),
                    ) => s.len(),
                    K::Literal(n::Literal::BitVec { words, .. }) => words.len(),
                    _ => 0,
                }
            }
            Node::Type(ty) => match ty {
                Term::AssociatedType {
                    arguments, subject, ..
                } => {
                    work.push((Node::Type(subject), depth + 1));
                    work.extend(arguments.iter().map(|t| (Node::Type(t), depth + 1)));
                    0
                }
                Term::NominalValue { ty, fields, .. } => {
                    work.push((Node::Type(ty), depth + 1));
                    work.extend(fields.iter().map(|field| (Node::Type(field), depth + 1)));
                    0
                }
                Term::Record(fields) | Term::RecordValue(fields) => {
                    work.extend(fields.iter().map(|(_, ty)| (Node::Type(ty), depth + 1)));
                    fields.iter().map(|(name, _)| name.len()).sum()
                }
                Term::Tuple(items)
                | Term::TypeListValue(items)
                | Term::TypeListConcat(items)
                | Term::TupleValue(items)
                | Term::Nominal {
                    arguments: items, ..
                }
                | Term::NormalForm {
                    arguments: items, ..
                } => {
                    work.extend(items.iter().map(|t| (Node::Type(t), depth + 1)));
                    0
                }
                Term::Pointer(t) | Term::ComptimeFunction(t) => {
                    work.push((Node::Type(t), depth + 1));
                    0
                }
                Term::Function { parameters, result } => {
                    work.push((Node::Type(result), depth + 1));
                    work.extend(parameters.iter().map(|p| (Node::Type(&p.ty), depth + 1)));
                    0
                }
                Term::Closure {
                    environment,
                    signature,
                } => {
                    work.push((Node::Type(signature), depth + 1));
                    work.extend(environment.iter().map(|t| (Node::Type(t), depth + 1)));
                    0
                }
                Term::String(s) | Term::Symbol(s) => s.len(),
                Term::Integer(n) => n.bit_len().div_ceil(32),
                Term::Float(value) => value
                    .digits()
                    .len()
                    .saturating_add(value.exponent().bit_len().div_ceil(32)),
                Term::BitVec(v) => v.words().len(),
                _ => 0,
            },
        };
        *steps = steps
            .checked_sub(size as u64)
            .ok_or_else(|| error(span, "constant copy budget exhausted"))?;
        if work.len() as u64 > *steps {
            return Err(error(span, "constant copy budget exhausted"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_value_copies_are_bounded() {
        let value = n::Expr {
            source: source(Span::default()),
            ty: p(P::Str),
            kind: K::Literal(n::Literal::String("λ".into())),
        };
        assert!(charge_copy(&value, &mut 3, Span::default()).is_err());
        let mut steps = 4;
        charge_copy(&value, &mut steps, Span::default()).unwrap();
        assert_eq!(steps, 0);
    }
    #[test]
    fn graph_snapshot_copy_budget_visits_bound_values() {
        let binding = n::LocalId {
            owner: u32::MAX,
            index: 0,
        };
        let span = Span::default();
        let value = n::Expr {
            source: source(span),
            ty: p(P::Str),
            kind: K::Block {
                statements: vec![n::Expr {
                    source: source(span),
                    ty: p(P::Void),
                    kind: K::Let {
                        binding,
                        value: Box::new(n::Expr {
                            source: source(span),
                            ty: p(P::Str),
                            kind: K::Literal(n::Literal::String("x".repeat(128))),
                        }),
                    },
                }],
                result: Some(Box::new(n::Expr {
                    source: source(span),
                    ty: p(P::Str),
                    kind: K::Local(binding),
                })),
            },
        };
        assert!(charge_copy(&value, &mut 64, span).is_err());
        charge_copy(&value, &mut 256, span).unwrap();
    }

    #[test]
    fn callable_copy_budget_includes_identity_arguments_absent_from_signature() {
        let span = Span::default();
        let member = DeclarationId {
            package: 1,
            local: 1,
        };
        for large_slot in 0..3 {
            let mut types = vec![p(P::Bool); 3];
            types[large_slot] = Term::Tuple(vec![p(P::Bool); 128]);
            let value = n::Expr {
                source: source(span),
                ty: Term::Function {
                    parameters: vec![],
                    result: Box::new(p(P::Bool)),
                },
                kind: K::TraitMethod {
                    interface: member,
                    member,
                    subject: types.remove(0),
                    interface_arguments: vec![types.remove(0)],
                    method_arguments: types,
                },
            };
            assert!(charge_copy(&value, &mut 64, span).is_err());
            charge_copy(&value, &mut 512, span).unwrap();
        }
    }

    #[test]
    fn associated_copy_budget_visits_subject_and_interface_inputs() {
        for subject_large in [false, true] {
            let large = Term::Tuple(vec![p(P::Bool); 128]);
            let ty = Term::AssociatedType {
                origin: Box::new(nir::AssociatedTypeOrigin {
                    interface: DeclarationId {
                        package: 1,
                        local: 1,
                    },
                    member: DeclarationId {
                        package: 1,
                        local: 2,
                    },
                }),
                subject: Box::new(if subject_large {
                    large.clone()
                } else {
                    p(P::Bool)
                }),
                arguments: if subject_large { vec![] } else { vec![large] },
            };
            let value = n::Expr {
                source: source(Span::default()),
                ty: p(P::Type),
                kind: K::TypeValue(ty.clone()),
            };
            assert!(charge_copy(&value, &mut 64, Span::default()).is_err());
            charge_copy(&value, &mut 512, Span::default()).unwrap();
            let mut budget = comptime_runtime::Budget {
                steps: 64,
                ..Default::default()
            };
            assert!(super::super::normal::clone_known_term(&ty, &mut budget).is_none());
        }
    }
}
