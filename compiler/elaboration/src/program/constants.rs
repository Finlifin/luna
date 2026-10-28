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

// Immutable analysis evidence, not a runtime value or a serialized interpreter
// environment. Unknown fields have no replacement expression. Shared snapshots
// avoid copying nested aggregate aliases, while depth and traversal stay bounded.
// Equality compares evidence only: two Unknown leaves do not prove equal values.
#[derive(Debug, PartialEq)]
pub(super) struct PartialStagedValue {
    depth: usize,
    data: PartialStagedData,
}
#[derive(Debug, PartialEq)]
enum PartialStagedData {
    Unknown,
    Known(Term),
    Aggregate(PartialStagedShape, Vec<std::sync::Arc<PartialStagedValue>>),
}
#[derive(Debug, PartialEq)]
enum PartialStagedShape {
    Tuple,
    Record(DeclarationId),
    Variant(DeclarationId, usize),
    AnonymousRecord(Vec<String>),
}
impl PartialStagedValue {
    fn unknown() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            depth: 0,
            data: PartialStagedData::Unknown,
        })
    }
    fn known(value: Term) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            depth: 0,
            data: PartialStagedData::Known(value),
        })
    }
    fn charge(&self, fuel: &mut usize) -> Result<(), String> {
        let mut pending = vec![self];
        while let Some(value) = pending.pop() {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("partial pattern evidence budget exhausted")?;
            match &value.data {
                PartialStagedData::Known(value) => {
                    nir::type_computations::charge_term(value, fuel).map_err(|e| e.to_string())?;
                }
                PartialStagedData::Aggregate(shape, fields) => {
                    *fuel = fuel
                        .checked_sub(fields.len())
                        .ok_or("partial pattern evidence budget exhausted")?;
                    if let PartialStagedShape::AnonymousRecord(names) = shape {
                        for name in names {
                            *fuel = fuel
                                .checked_sub(name.len())
                                .ok_or("partial pattern evidence budget exhausted")?;
                        }
                    }
                    pending.extend(fields.iter().map(|field| field.as_ref()));
                }
                PartialStagedData::Unknown => {}
            }
        }
        Ok(())
    }
}
impl std::fmt::Display for PartialStagedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.data {
            PartialStagedData::Unknown => write!(f, "(unknown)"),
            PartialStagedData::Known(value) => write!(f, "(known {value})"),
            PartialStagedData::Aggregate(shape, fields) => {
                match shape {
                    PartialStagedShape::Tuple => write!(f, "(tuple")?,
                    PartialStagedShape::Record(id) => {
                        write!(f, "(record (declaration {} {})", id.package, id.local)?
                    }
                    PartialStagedShape::Variant(id, index) => write!(
                        f,
                        "(variant (declaration {} {}) (index {index})",
                        id.package, id.local
                    )?,
                    PartialStagedShape::AnonymousRecord(names) => {
                        write!(f, "(anonymous-record (names")?;
                        for name in names {
                            write!(f, " {:?}", name)?;
                        }
                        write!(f, ")")?;
                    }
                }
                for field in fields {
                    write!(f, " {field}")?;
                }
                write!(f, ")")
            }
        }
    }
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
            n::Pattern::CodeEvidence { .. } => {
                return Err(error(span, "unresolved pattern-code evidence"));
            }
            n::Pattern::OptionalPresent(inner) | n::Pattern::ErrorResult { inner, .. } => {
                pending.push(inner)
            }
            n::Pattern::OptionalAbsent
            | n::Pattern::Wild
            | n::Pattern::Unit
            | n::Pattern::Literal(_) => {}
            n::Pattern::Tuple(fields)
            | n::Pattern::TypeTuple(fields)
            | n::Pattern::Or(fields)
            | n::Pattern::NominalType { fields, .. }
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
    completed_subject_evidence(pattern, symbol, fuel).map(|(_, bindings)| bindings)
}

fn completed_subject_evidence(
    pattern: &n::Pattern,
    symbol: &Term,
    fuel: &mut usize,
) -> Result<(bool, BTreeMap<n::LocalId, Term>), String> {
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
    Ok(values.pop().unwrap_or((true, BTreeMap::new())))
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
            Value::Record { .. }
                | Value::Variant { .. }
                | Value::Object(_)
                | Value::Newtype { .. }
                | Value::Optional { .. }
                | Value::ErrorResult { .. }
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
        // Runtime declaration identities are transportable data even though
        // they are not scalar normal terms. Reconstruct the typed identity;
        // the ordinary phase gate still forbids calling it during generation.
        if matches!(&ty, Term::Function { .. })
            && matches!(
                value,
                Value::RuntimeFunction { .. } | Value::RuntimeTraitMethod(_)
            )
        {
            return self.materialize_local_constant(value, ty, span);
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
                is_unsafe: false,
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
                        let mut budget = comptime_runtime::Budget {
                            steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                            max_limbs: 4096,
                        };
                        let before = budget.steps;
                        let checked = comptime_interpretor::validate_snapshot_term(
                            argument,
                            &mut budget,
                            type_scope,
                        );
                        self.required_steps -= (before - budget.steps) as u64;
                        checked.map_err(|cause| {
                            error(
                                span,
                                format!(
                                    "closure snapshot requires completed nominal inputs: {cause:?}"
                                ),
                            )
                        })?;
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
            Value::Optional { ty, value } => {
                let ty = self.copy_alias_type(ty, span)?;
                self.validate_closure_snapshot(
                    &Value::Type(ty),
                    span,
                    depth + 1,
                    active,
                    completed,
                    type_scope,
                )?;
                if let Some(value) = value {
                    self.validate_closure_snapshot(
                        value,
                        span,
                        depth + 1,
                        active,
                        completed,
                        type_scope,
                    )?;
                }
                Ok(())
            }
            Value::Newtype { ty, value } | Value::ErrorResult { ty, value, .. } => {
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
            Value::RuntimeFunction { .. } | Value::RuntimeTraitMethod(_) => {
                let mut budget = comptime_runtime::Budget {
                    steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                    max_limbs: 4096,
                };
                let before = budget.steps;
                let checked = value.validate_runtime_identity_snapshot(&mut budget, type_scope);
                self.required_steps -= (before - budget.steps) as u64;
                checked.map_err(|cause| {
                    error(
                        span,
                        format!(
                            "closure snapshot requires completed runtime identity inputs: {cause:?}"
                        ),
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
            && (self
                .type_parameters
                .get(&parent)
                .is_some_and(|parameters| !parameters.is_empty())
                || !self.nominal_capture_ids(parent).is_empty())
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

    fn partial_staged_value(
        &self,
        value: &n::Expr,
        depth: usize,
        fuel: &mut usize,
    ) -> Result<std::sync::Arc<PartialStagedValue>, String> {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("partial pattern evidence budget exhausted")?;
        if depth >= 128 {
            return Ok(PartialStagedValue::unknown());
        }
        if let K::Local(binding) = value.kind {
            if self.locals.contains_key(&binding)
                && let Some(value) = self.partial_staged_bindings.get(&binding)
            {
                return Ok(value.clone());
            }
        }
        // Follow the already checked field route. This inspects a snapshot,
        // never a pointer or a replay of the initializer that produced it.
        if let K::Field { value: base, .. } | K::AnonymousField { value: base, .. } = &value.kind {
            let base = self.partial_staged_value(base, depth + 1, fuel)?;
            let known = match (&value.kind, &base.data) {
                (
                    K::Field {
                        declaration, index, ..
                    },
                    PartialStagedData::Aggregate(PartialStagedShape::Record(actual), fields),
                ) if declaration == actual => {
                    return Ok(fields
                        .get(*index)
                        .cloned()
                        .unwrap_or_else(PartialStagedValue::unknown));
                }
                (
                    K::AnonymousField { name, index, .. },
                    PartialStagedData::Aggregate(
                        PartialStagedShape::AnonymousRecord(names),
                        fields,
                    ),
                ) => {
                    if let Some(actual) = names.get(*index) {
                        *fuel = fuel
                            .checked_sub(name.len().saturating_add(actual.len()))
                            .ok_or("partial field observation budget exhausted")?;
                        if name == actual {
                            return Ok(fields
                                .get(*index)
                                .cloned()
                                .unwrap_or_else(PartialStagedValue::unknown));
                        }
                    }
                    None
                }
                (
                    K::Field {
                        declaration, index, ..
                    },
                    PartialStagedData::Known(Term::NominalValue {
                        ty,
                        variant: None,
                        fields,
                    }),
                ) if matches!(ty.as_ref(), Term::Nominal { declaration: actual, .. } if declaration == actual) => {
                    fields.get(*index)
                }
                (
                    K::AnonymousField { name, index, .. },
                    PartialStagedData::Known(Term::RecordValue(fields)),
                ) => {
                    if let Some((actual, value)) = fields.get(*index) {
                        *fuel = fuel
                            .checked_sub(name.len().saturating_add(actual.len()))
                            .ok_or("partial field observation budget exhausted")?;
                        (name == actual).then_some(value)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            return match known {
                Some(value) => {
                    nir::type_computations::charge_term(value, fuel).map_err(|e| e.to_string())?;
                    Ok(PartialStagedValue::known(value.clone()))
                }
                None => Ok(PartialStagedValue::unknown()),
            };
        }
        let mut budget = comptime_runtime::Budget {
            steps: *fuel,
            max_limbs: 4096,
        };
        let symbol = self.known_normal_input(value, 0, &mut budget);
        *fuel = budget.steps;
        if let Some(symbol) = symbol {
            return Ok(PartialStagedValue::known(symbol));
        }
        let (shape, fields) = match (&value.kind, &value.ty) {
            (K::Tuple(fields), _) => (PartialStagedShape::Tuple, fields),
            (
                K::Record {
                    declaration,
                    fields,
                },
                Term::Nominal {
                    declaration: actual,
                    ..
                },
            ) if declaration == actual => (PartialStagedShape::Record(*declaration), fields),
            (
                K::Variant {
                    declaration,
                    index,
                    fields,
                },
                Term::Nominal {
                    declaration: actual,
                    ..
                },
            ) if declaration == actual => {
                (PartialStagedShape::Variant(*declaration, *index), fields)
            }
            (K::AnonymousRecord { fields }, Term::Record(types)) if fields.len() == types.len() => {
                let mut names = Vec::new();
                for (name, _) in types {
                    *fuel = fuel
                        .checked_sub(name.len().saturating_add(1))
                        .ok_or("partial pattern evidence budget exhausted")?;
                    names.push(name.clone());
                }
                (PartialStagedShape::AnonymousRecord(names), fields)
            }
            _ => return Ok(PartialStagedValue::unknown()),
        };
        let mut children = Vec::new();
        let mut height = 1;
        for field in fields {
            let child = self.partial_staged_value(field, depth + 1, fuel)?;
            height = height.max(child.depth + 1);
            children.push(child);
        }
        if height >= 128 {
            return Ok(PartialStagedValue::unknown());
        }
        Ok(std::sync::Arc::new(PartialStagedValue {
            depth: height,
            data: PartialStagedData::Aggregate(shape, children),
        }))
    }

    /// Keep possibility separate from binding evidence. A failed structural
    /// sibling does not erase the facts used to check a successful continuation.
    fn known_staged_pattern_bindings(
        &self,
        pattern: &n::Pattern,
        value: &std::sync::Arc<PartialStagedValue>,
        depth: usize,
        fuel: &mut usize,
    ) -> Result<
        (
            bool,
            BTreeMap<n::LocalId, std::sync::Arc<PartialStagedValue>>,
        ),
        String,
    > {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("partial pattern evidence budget exhausted")?;
        if depth >= 128 {
            return Ok((true, BTreeMap::new()));
        }
        if let PartialStagedData::Known(symbol) = &value.data {
            let (possible, bindings) = completed_subject_evidence(pattern, symbol, fuel)?;
            return Ok((
                possible,
                bindings
                    .into_iter()
                    .map(|(id, symbol)| (id, PartialStagedValue::known(symbol)))
                    .collect(),
            ));
        }
        match pattern {
            n::Pattern::Binding { binding, inner } => {
                let (possible, mut bindings) =
                    self.known_staged_pattern_bindings(inner, value, depth + 1, fuel)?;
                bindings.insert(*binding, value.clone());
                Ok((possible, bindings))
            }
            n::Pattern::Guard { pattern: inner, .. } | n::Pattern::Then { first: inner, .. } => {
                self.known_staged_pattern_bindings(inner, value, depth + 1, fuel)
            }
            n::Pattern::Remap { pattern, bindings } => {
                let (possible, mut result) =
                    self.known_staged_pattern_bindings(pattern, value, depth + 1, fuel)?;
                *fuel = fuel
                    .checked_sub(bindings.len())
                    .ok_or("partial pattern evidence budget exhausted")?;
                let moved: Vec<_> = bindings
                    .iter()
                    .filter_map(|(from, to)| result.get(from).map(|value| (*to, value.clone())))
                    .collect();
                for (from, to) in bindings {
                    result.remove(from);
                    result.remove(to);
                }
                result.extend(moved);
                Ok((possible, result))
            }
            n::Pattern::Or(patterns) => {
                let mut result: Option<BTreeMap<n::LocalId, std::sync::Arc<PartialStagedValue>>> =
                    None;
                for pattern in patterns {
                    let (possible, candidate) =
                        self.known_staged_pattern_bindings(pattern, value, depth + 1, fuel)?;
                    if !possible {
                        continue;
                    }
                    if let Some(current) = &mut result {
                        *fuel = fuel
                            .checked_sub(current.len() + candidate.len())
                            .ok_or("partial pattern evidence budget exhausted")?;
                        for value in current.values().chain(candidate.values()) {
                            value.charge(fuel)?;
                        }
                        current.retain(|binding, value| candidate.get(binding) == Some(value));
                    } else {
                        result = Some(candidate);
                    }
                }
                Ok((result.is_some(), result.unwrap_or_default()))
            }
            pattern => {
                let PartialStagedData::Aggregate(shape, fields) = &value.data else {
                    return Ok((true, BTreeMap::new()));
                };
                let patterns = match (pattern, shape) {
                    (n::Pattern::Tuple(patterns), PartialStagedShape::Tuple) => patterns,
                    (
                        n::Pattern::Record {
                            declaration,
                            fields,
                        },
                        PartialStagedShape::Record(actual),
                    ) if declaration == actual => fields,
                    (
                        n::Pattern::Variant {
                            declaration,
                            index,
                            fields,
                        },
                        PartialStagedShape::Variant(actual, selected),
                    ) if declaration == actual && index == selected => fields,
                    (
                        n::Pattern::Variant {
                            declaration, index, ..
                        },
                        PartialStagedShape::Variant(actual, selected),
                    ) if declaration != actual || index != selected => {
                        return Ok((false, BTreeMap::new()));
                    }
                    (
                        n::Pattern::AnonymousRecord { names, fields },
                        PartialStagedShape::AnonymousRecord(actual),
                    ) => {
                        for name in names.iter().chain(actual) {
                            *fuel = fuel
                                .checked_sub(name.len().saturating_add(1))
                                .ok_or("partial pattern evidence budget exhausted")?;
                        }
                        if names != actual {
                            return Ok((true, BTreeMap::new()));
                        }
                        fields
                    }
                    _ => return Ok((true, BTreeMap::new())),
                };
                if patterns.len() != fields.len() {
                    return Ok((true, BTreeMap::new()));
                }
                let mut result = BTreeMap::new();
                let mut possible = true;
                for (pattern, field) in patterns.iter().zip(fields) {
                    let (child_possible, bindings) =
                        self.known_staged_pattern_bindings(pattern, field, depth + 1, fuel)?;
                    possible &= child_possible;
                    result.extend(bindings);
                }
                Ok((possible, result))
            }
        }
    }

    /// Snapshot only structural evidence before the iterator input is delivered.
    /// The original expression remains in the loop's once-only input binding.
    pub(super) fn observed_tuple_elements(
        &mut self,
        value: &n::Expr,
        count: usize,
        span: Span,
    ) -> Result<Option<Vec<std::sync::Arc<PartialStagedValue>>>, Error> {
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = (|| {
            let value = self.partial_staged_value(value, 0, &mut fuel)?;
            match &value.data {
                PartialStagedData::Known(Term::TupleValue(fields)) if fields.len() == count => {
                    let mut result = Vec::new();
                    for field in fields {
                        nir::type_computations::charge_term(field, &mut fuel)
                            .map_err(|e| e.to_string())?;
                        result.push(PartialStagedValue::known(field.clone()));
                    }
                    Ok(Some(result))
                }
                PartialStagedData::Aggregate(PartialStagedShape::Tuple, fields)
                    if fields.len() == count =>
                {
                    fuel = fuel
                        .checked_sub(count)
                        .ok_or("tuple observation budget exhausted")?;
                    Ok(Some(fields.clone()))
                }
                _ => Ok(None),
            }
        })();
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|message: String| error(span, message))
    }

    /// Ordinary iteration bindings still own storage and capture by the usual
    /// rules. Do not publish these facts as specialization inputs or substitute
    /// their executable local reads with constant expressions.
    pub(super) fn record_observed_pattern(
        &mut self,
        pattern: &n::Pattern,
        value: &std::sync::Arc<PartialStagedValue>,
        span: Span,
    ) -> Result<(), Error> {
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = self.known_staged_pattern_bindings(pattern, value, 0, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        self.partial_staged_bindings
            .extend(result.map_err(|e| error(span, e))?.1);
        Ok(())
    }

    pub(super) fn observed_expression_type(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        if value.ty != p(P::Type) {
            return Ok(None);
        }
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = (|| {
            let value = self.partial_staged_value(value, 0, &mut fuel)?;
            let PartialStagedData::Known(ty) = &value.data else {
                return Ok(None);
            };
            if ty.sort() != Sort::Type {
                return Ok(None);
            }
            nir::type_computations::charge_term(ty, &mut fuel).map_err(|e| e.to_string())?;
            Ok(Some(ty.clone()))
        })();
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|e: String| error(span, e))
    }

    pub(super) fn observed_local_type(
        &mut self,
        binding: n::LocalId,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        if self.locals.get(&binding) != Some(&p(P::Type)) {
            return Ok(None);
        }
        let Some(value) = self.partial_staged_bindings.get(&binding) else {
            return Ok(None);
        };
        let PartialStagedData::Known(ty) = &value.data else {
            return Ok(None);
        };
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = nir::type_computations::charge_term(ty, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|e| error(span, e.to_string()))?;
        Ok(Some(ty.clone()))
    }

    /// Retain known const outputs without replacing ordinary let storage.
    /// Unknown siblings stay unknown; construction, matching and all calls
    /// still execute in the original initializer at invocation time.
    pub(super) fn record_known_staged_pattern(
        &mut self,
        pattern: &n::Pattern,
        value: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let bindings = self
            .partial_staged_value(value, 0, &mut fuel)
            .and_then(|value| self.known_staged_pattern_bindings(pattern, &value, 0, &mut fuel));
        self.required_steps -= (before - fuel) as u64;
        for (binding, value) in bindings.map_err(|e| error(span, e))?.1 {
            self.partial_staged_bindings.insert(binding, value.clone());
            let PartialStagedData::Known(symbol) = &value.data else {
                continue;
            };
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let charged = nir::type_computations::charge_term(symbol, &mut fuel);
            self.required_steps -= (before - fuel) as u64;
            charged.map_err(|e| error(span, e.to_string()))?;
            let symbol = symbol.clone();
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
        Ok(())
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
            let result =
                iir::staging::structural_boolean_dependencies_in(value, self.target, &mut fuel);
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
                | n::Pattern::NominalType { fields, .. }
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
                target: self.target,
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
                    | n::Pattern::NominalType { fields, .. }
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
                                | n::Pattern::NominalType { fields, .. }
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
                                target: self.target,
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
        if self.specialization_inputs.is_empty() && self.partial_staged_bindings.is_empty() {
            return self.expr(body, None);
        }
        let saved = (
            self.specialization_inputs.clone(),
            self.type_input_bindings.clone(),
            self.partial_staged_bindings.clone(),
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
                self.partial_staged_bindings,
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
        if self.partial_staged_bindings.is_empty()
            && (self.in_required_comptime || self.specialization_inputs.is_empty())
        {
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
                || self.partial_staged_bindings.contains_key(binding)
        });
        self.partial_staged_bindings
            .retain(|binding, _| !invalidated.contains(binding));
        // Invocation-time observations are invalidated even during required
        // computation; symbolic declaration inputs keep their existing rules.
        if self.in_required_comptime {
            return Ok(());
        }
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
        let mut staged_bindings: BTreeSet<_> = self
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
        if self.describing_open_call || self.checking_open_behavior {
            self.required_steps = self
                .required_steps
                .checked_sub(self.open_staged_locals.len() as u64)
                .ok_or_else(|| error(source.span, "staged local dependency budget exhausted"))?;
            staged_bindings.extend(self.open_staged_locals.iter().copied());
        }
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
                    iir::staging::Layouts::from_index(&self.normal_form_metadata(), &self.newtypes)
                        .with_target(self.target),
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
                | K::UnsafeScope(value)
                | K::Discard(value)
                | K::RequirePredicate(value)
                | K::EnsurePredicate(value)
                | K::CheckPredicate(value)
                | K::Snapshot(value)
                | K::TakeBits(value)
                | K::TextQuery { subject: value, .. }
                | K::IntegerRepresentation(value)
                | K::ComptimeIntegerConvert(value)
                | K::SymbolName(value)
                | K::EnumName(value)
                | K::BitConvert(value)
                | K::ErrorConvert { value: value, .. }
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
                | K::TemplateEqual(left, right)
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
                K::TypeValue(ty) | K::ComptimeValue(ty) => {
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
            iir::staging::Layouts::from_index(&self.normal_form_metadata(), &self.newtypes)
                .with_target(self.target),
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
            iir::staging::Layouts::from_index(&self.normal_form_metadata(), &self.newtypes)
                .with_target(self.target),
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
            Value::List { ty: actual, values } => {
                if actual != ty {
                    return Err(error(span, "constant list identity mismatch"));
                }
                let element = self
                    .list_element_type(&ty)
                    .cloned()
                    .ok_or_else(|| error(span, "constant list requires bound meta.List"))?;
                self.required_steps = self
                    .required_steps
                    .checked_sub(values.len() as u64)
                    .ok_or_else(|| error(span, "constant list budget exhausted"))?;
                K::TypeList(
                    values
                        .into_iter()
                        .map(|value| {
                            self.constant_nominal_field(
                                value,
                                element.clone(),
                                span,
                                depth + 1,
                                snapshot,
                            )
                        })
                        .collect::<Result<_, _>>()?,
                )
            }
            Value::Optional { ty: actual, value } => {
                if actual != ty {
                    return Err(error(span, "constant optional identity mismatch"));
                }
                let Term::Optional(payload) = &ty else {
                    return Err(error(span, "constant optional type missing"));
                };
                match value {
                    None => K::OptionalAbsent,
                    Some(value) => K::OptionalPresent(Box::new(self.constant_nominal_field(
                        *value,
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
                if actual != ty {
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
                            .get(index)
                            .ok_or_else(|| error(span, "constant error result tag invalid"))?,
                    ),
                };
                K::ErrorConvert {
                    conversion,
                    value: Box::new(self.constant_nominal_field(
                        *value,
                        payload.clone(),
                        span,
                        depth + 1,
                        snapshot,
                    )?),
                }
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
            Value::Object(object) => {
                if ty != p(P::Object) {
                    return Err(error(span, "Object constant identity mismatch"));
                }
                let entries = object.into_entries();
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
                            (Some(key), value)
                        }
                    };
                    names.push(name);
                    fields.push(self.constant_nominal_field(
                        value.value,
                        value.ty,
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
    nir::copy_budget::charge_expression(value, steps)
        .map_err(|_| error(span, "constant copy budget exhausted"))
}

#[cfg(test)]
mod tests {

    #[test]
    fn partial_snapshot_dump_and_budget_preserve_unknown_fields() {
        let unknown = super::PartialStagedValue::unknown();
        let value = super::PartialStagedValue {
            depth: 1,
            data: super::PartialStagedData::Aggregate(
                super::PartialStagedShape::AnonymousRecord(vec![
                    "label\"\n".into(),
                    "pending".into(),
                ]),
                vec![
                    super::PartialStagedValue::known(nir::Term::Bool(true)),
                    unknown.clone(),
                ],
            ),
        };
        let dump = value.to_string();
        assert_eq!(
            dump,
            "(anonymous-record (names \"label\\\"\\n\" \"pending\") (known (bool true)) (unknown))"
        );
        assert!(value.charge(&mut 1).is_err());
        value.charge(&mut 128).unwrap();
        assert_eq!(unknown.to_string(), "(unknown)");
        assert_eq!(value.to_string(), dump);
    }
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

    #[test]
    fn contextual_constructor_copy_charges_unselected_literals_and_patterns() {
        use n::NormalFormArgument as A;
        let span = Span::default();
        let location = source(span);
        let large = "x".repeat(1024);
        let input = A::Match {
            source: location,
            value: n::Expr {
                source: location,
                ty: p(P::Str),
                kind: K::Literal(n::Literal::String("x".into())),
            },
            arms: vec![
                n::NormalMatchArm {
                    pattern: n::Pattern::Literal(n::Literal::String(large.clone())),
                    guard: None,
                    body: A::Integer {
                        source: location,
                        spelling: "1".into(),
                        negative: false,
                    },
                },
                n::NormalMatchArm {
                    pattern: n::Pattern::Wild,
                    guard: None,
                    body: A::Integer {
                        source: location,
                        spelling: "7".repeat(1024),
                        negative: false,
                    },
                },
            ],
        };
        for nominal in [true, false] {
            let arguments = vec![input.clone()];
            let head = DeclarationId {
                package: 1,
                local: 1,
            };
            let value = n::Expr {
                source: location,
                ty: p(P::Type),
                kind: if nominal {
                    K::NominalType {
                        preparations: vec![],
                        bindings: None,
                        order: None,
                        declaration: head,
                        arguments,
                    }
                } else {
                    K::NormalFormType {
                        head,
                        arguments,
                        plan: None,
                    }
                },
            };
            assert!(charge_copy(&value, &mut 128, span).is_err());
            charge_copy(&value, &mut 4096, span).unwrap();
        }
    }

    #[test]
    fn contextual_constructor_copy_bounds_width_and_depth() {
        use n::NormalFormArgument as A;
        let span = Span::default();
        let location = source(span);
        let root = |argument| n::Expr {
            source: location,
            ty: p(P::Type),
            kind: K::NominalType {
                preparations: vec![],
                bindings: None,
                order: None,
                declaration: DeclarationId {
                    package: 1,
                    local: 1,
                },
                arguments: vec![argument],
            },
        };
        let wide = root(A::Tuple {
            source: location,
            type_head: None,
            items: vec![A::Unit { source: location }; 512],
        });
        assert!(charge_copy(&wide, &mut 64, span).is_err());
        charge_copy(&wide, &mut 4096, span).unwrap();
        let mut deep = A::Unit { source: location };
        for _ in 0..nir::copy_budget::MAX_COPY_DEPTH + 1 {
            deep = A::Block {
                source: location,
                statements: vec![],
                result: Some(Box::new(deep)),
            };
        }
        assert!(charge_copy(&root(deep), &mut 10000, span).is_err());
        charge_copy(&root(A::Unit { source: location }), &mut 64, span).unwrap();
    }

    #[test]
    fn constant_copy_charges_exact_float_literal_spelling() {
        let span = Span::default();
        let value = n::Expr {
            source: source(span),
            ty: p(P::Float),
            kind: K::Literal(n::Literal::Float(format!("0.{}1", "0".repeat(1024)))),
        };
        assert!(charge_copy(&value, &mut 128, span).is_err());
        charge_copy(&value, &mut 2048, span).unwrap();
    }
}

impl Checker<'_, '_> {
    pub(super) fn constant_object_entry(
        &mut self,
        object: &hir::Expr<'_>,
        index: usize,
        expected: Option<&Term>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        // Only completed ordinary constants are cached by owner. Associated
        // defaults and generic residuals depend on an invocation environment.
        if let H::ResolvedDef { def_id, .. } = object.kind
            && let Some(owner) = self.package.definition_owner(def_id)
            && matches!(
                self.package.item(owner).map(|item| &item.kind),
                Some(ItemKind::Const(..))
            )
            && let Some(cached) = self.constants.get(&owner)
            && let Some(value) = cached_object_entry(cached, index, &mut self.required_steps, span)?
        {
            if let Some(expected) = expected {
                self.require_expression(&value.ty, expected, span)?;
            }
            return Ok(value);
        }
        let (value, _) = self.required_typed_expression(object, Some(&p(P::Object)))?;
        let Value::Object(object) = value else {
            return Err(error(
                span,
                "generated input group did not produce an Object",
            ));
        };
        let entries = object.into_entries();
        self.required_steps = self
            .required_steps
            .checked_sub(entries.len() as u64)
            .ok_or_else(|| error(span, "generated input extraction budget exhausted"))?;
        let entry = entries
            .into_iter()
            .nth(index)
            .ok_or_else(|| error(span, "generated input index is outside its input group"))?;
        let value = match entry {
            comptime_interpretor::ObjectEntry::Value(value)
            | comptime_interpretor::ObjectEntry::Property { value, .. } => value,
        };
        if let Some(expected) = expected {
            self.require_expression(&value.ty, expected, span)?;
        }
        self.materialize_local_constant(value.value, value.ty, span)
    }
}

/// A flat cached Object is already a checked constant expression. Snapshots with
/// reconstruction bindings wrap the Object in a call or block; extracting a
/// child from those wrappers would lose its environment and is not permitted.
fn cached_object_entry(
    object: &n::Expr,
    index: usize,
    steps: &mut u64,
    span: Span,
) -> Result<Option<n::Expr>, Error> {
    let K::Object { fields, .. } = &object.kind else {
        return Ok(None);
    };
    if object.ty != p(P::Object) {
        return Ok(None);
    }
    let field = fields
        .get(index)
        .ok_or_else(|| error(span, "generated input index is outside its input group"))?;
    charge_copy(field, steps, span)?;
    Ok(Some(field.clone()))
}

#[cfg(test)]
mod object_entry_tests {
    use super::*;
    use rustc_span::DUMMY_SP;

    fn object() -> n::Expr {
        n::Expr {
            source: source(DUMMY_SP),
            ty: p(P::Object),
            kind: K::Object {
                names: vec![Some("large".into()), Some("flag".into())],
                fields: vec![
                    n::Expr {
                        source: source(DUMMY_SP),
                        ty: p(P::Str),
                        kind: K::Literal(n::Literal::String("x".repeat(65536))),
                    },
                    n::Expr {
                        source: source(DUMMY_SP),
                        ty: p(P::Bool),
                        kind: K::Literal(n::Literal::Bool(true)),
                    },
                ],
            },
        }
    }

    #[test]
    fn unrelated_large_entries_do_not_consume_the_selected_entry_budget() {
        let input = object();
        let mut steps = 32;
        let field = cached_object_entry(&input, 1, &mut steps, DUMMY_SP)
            .unwrap()
            .unwrap();
        assert_eq!(field.ty, p(P::Bool));
        assert!(matches!(field.kind, K::Literal(n::Literal::Bool(true))));
        assert!(steps < 32);
        assert!(cached_object_entry(&input, 0, &mut 32, DUMMY_SP).is_err());
        assert_eq!(input, object());
    }

    #[test]
    fn invalid_indices_and_budget_failures_leave_the_snapshot_reusable() {
        let input = object();
        let error = cached_object_entry(&input, 2, &mut 32, DUMMY_SP).unwrap_err();
        assert!(error.message.contains("outside its input group"));
        assert!(cached_object_entry(&input, 1, &mut 0, DUMMY_SP).is_err());
        assert!(
            cached_object_entry(&input, 1, &mut 32, DUMMY_SP)
                .unwrap()
                .is_some()
        );
        assert_eq!(input, object());
    }

    #[test]
    fn reconstruction_wrappers_are_not_stripped_from_cached_objects() {
        let wrapped = n::Expr {
            source: source(DUMMY_SP),
            ty: p(P::Object),
            kind: K::Block {
                statements: vec![],
                result: Some(Box::new(object())),
            },
        };
        let mut steps = 32;
        assert!(
            cached_object_entry(&wrapped, 1, &mut steps, DUMMY_SP)
                .unwrap()
                .is_none()
        );
        assert_eq!(steps, 32);
    }
}
