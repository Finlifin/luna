//! Completed module and associated constants. Cached expressions reconstruct frozen value
//! graphs; active inference, caller premises and interpreter storage do not enter.
use super::*;
use comptime_interpretor::Value;

mod initializers;
mod materialization;

use materialization::Materialization;

mod observations;
mod prepared_observations;

use observations::ObservationScope;
pub(super) use prepared_observations::PreparedObservations;

#[cfg(test)]
mod input_fact_tests;

// Initialization evidence, not an executable IR or a live interpreter store.
pub(super) struct StagedPatternSnapshot {
    parts: Vec<(n::Pattern, Term, Term)>,
    staged: BTreeSet<n::LocalId>,
}

impl std::fmt::Display for StagedPatternSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(staged-pattern-snapshot (parts")?;
        for (pattern, domain, value) in &self.parts {
            write!(f, " (part {pattern} (domain {domain}) (value {value}))")?;
        }
        f.write_str(") (staged")?;
        for binding in &self.staged {
            write!(f, " {binding}")?;
        }
        f.write_str("))")
    }
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
    Known(std::sync::Arc<Term>),
    Aggregate(PartialStagedShape, Vec<std::sync::Arc<PartialStagedValue>>),
}

#[derive(Clone, Debug, PartialEq)]
enum PartialStagedShape {
    Tuple,
    // Fields carry a guaranteed prefix. Exact means it also covers the tail.
    List { ty: Term, exact: bool },
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

    pub(super) fn known(value: impl Into<std::sync::Arc<Term>>) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            depth: 0,
            data: PartialStagedData::Known(value.into()),
        })
    }

    fn known_term(&self) -> Option<&Term> {
        match &self.data {
            PartialStagedData::Known(term) => Some(term.as_ref()),
            _ => None,
        }
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
                    if let PartialStagedShape::List { ty, .. } = shape {
                        nir::type_computations::charge_term(ty, fuel).map_err(|e| e.to_string())?;
                    }
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
                    PartialStagedShape::List { ty, exact } => {
                        write!(f, "(list (type {ty}) (exact {exact})")?
                    }
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

// Pending identity inputs are observations, not permission to execute or export
// an abstract value. The enclosing structural/phase checks remain independent.
fn staged_term_is_pending(
    value: &Term,
    declaration_inputs: &[DeclarationId],
    steps: &mut u64,
    span: Span,
) -> Result<bool, Error> {
    let inputs = input_observation::term_inputs(value, steps, span)?;
    if !inputs.parameters.is_empty() {
        return Ok(true);
    }
    for declaration in declaration_inputs {
        *steps = steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "staged input dependency budget exhausted"))?;
        if inputs.declarations.contains_key(declaration) {
            return Ok(true);
        }
    }
    Ok(false)
}

impl Checker<'_, '_> {
    fn partial_staged_value(
        &self,
        value: &n::Expr,
        depth: usize,
        fuel: &mut usize,
    ) -> Result<std::sync::Arc<PartialStagedValue>, String> {
        self.partial_staged_value_in(value, depth, fuel, &ObservationScope::default())
    }

    fn partial_staged_value_in(
        &self,
        value: &n::Expr,
        depth: usize,
        fuel: &mut usize,
        scope: &ObservationScope,
    ) -> Result<std::sync::Arc<PartialStagedValue>, String> {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("partial pattern evidence budget exhausted")?;
        if depth >= 128 {
            return Err("partial pattern evidence depth exhausted".into());
        }
        if let K::Local(binding) = value.kind {
            if let Some(value) = scope.values.get(&binding) {
                return Ok(value.clone());
            }
            if self.locals.contains_key(&binding)
                && let Some(value) = self.input_facts.partial.get(&binding)
            {
                return Ok(value.clone());
            }
        }
        if let K::Call {
            callee, arguments, ..
        } = &value.kind
        {
            if let Some(observed) =
                self.partial_staged_call(callee, arguments, depth, fuel, scope)?
            {
                return Ok(observed);
            }
        }
        if let K::ErrorConvert {
            conversion,
            value: operand,
        } = &value.kind
        {
            let observed = self.partial_staged_value_in(operand, depth + 1, fuel, scope)?;
            let Some(payload) = observed.known_term() else {
                return Ok(PartialStagedValue::unknown());
            };
            let result =
                self.observe_error_conversion(*conversion, &operand.ty, &value.ty, payload, fuel);
            return Ok(normal::observation::optional(result)
                .map_err(|reason| reason.to_string())?
                .map_or_else(PartialStagedValue::unknown, PartialStagedValue::known));
        }
        if let K::ListConcat(left, right) = &value.kind {
            if self.list_element_type(&value.ty).is_none()
                || left.ty != value.ty
                || right.ty != value.ty
            {
                return Ok(PartialStagedValue::unknown());
            }
            let left = self.partial_staged_value_in(left, depth + 1, fuel, scope)?;
            let right = self.partial_staged_value_in(right, depth + 1, fuel, scope)?;
            return PartialStagedValue::concat(
                &left,
                &right,
                &value.ty,
                self.protocols.meta_list,
                fuel,
            );
        }
        if let K::ListLength(input) = &value.kind {
            let input = self.partial_staged_value_in(input, depth + 1, fuel, scope)?;
            return input.list_length(self.protocols.meta_list, fuel);
        }
        if let K::TypeListIndex(values, index) = &value.kind {
            if self.list_element_type(&values.ty) != Some(&value.ty) {
                return Ok(PartialStagedValue::unknown());
            }
            let sequence = self.partial_staged_value_in(values, depth + 1, fuel, scope)?;
            let index = self.partial_staged_value_in(index, depth + 1, fuel, scope)?;
            let Some(Term::Integer(index)) = index.known_term() else {
                return Ok(PartialStagedValue::unknown());
            };
            *fuel = fuel
                .checked_sub(index.bit_len().div_ceil(32).saturating_add(1))
                .ok_or("partial list observation budget exhausted")?;
            let Some(index) = index
                .to_u128()
                .ok()
                .and_then(|index| usize::try_from(index).ok())
            else {
                return Ok(PartialStagedValue::unknown());
            };
            let known = match (&sequence.data, sequence.known_term()) {
                (PartialStagedData::Aggregate(PartialStagedShape::List { ty, .. }, fields), _) => {
                    nir::type_computations::charge_term(ty, fuel).map_err(|e| e.to_string())?;
                    return Ok(if *ty == values.ty {
                        fields
                            .get(index)
                            .cloned()
                            .unwrap_or_else(PartialStagedValue::unknown)
                    } else {
                        PartialStagedValue::unknown()
                    });
                }
                (
                    _,
                    Some(Term::NominalValue {
                        ty,
                        variant: None,
                        fields,
                    }),
                ) => {
                    nir::type_computations::charge_term(ty, fuel).map_err(|e| e.to_string())?;
                    if **ty == values.ty {
                        fields.get(index)
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
        // Follow the already checked field route. This inspects a snapshot,
        // never a pointer or a replay of the initializer that produced it.
        if let K::Field { value: base, .. } | K::AnonymousField { value: base, .. } = &value.kind {
            let base = self.partial_staged_value_in(base, depth + 1, fuel, scope)?;
            let known = match (&value.kind, &base.data, base.known_term()) {
                (
                    K::Field {
                        declaration, index, ..
                    },
                    PartialStagedData::Aggregate(PartialStagedShape::Record(actual), fields),
                    _,
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
                    _,
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
                    _,
                    Some(Term::NominalValue {
                        ty,
                        variant: None,
                        fields,
                    }),
                ) if matches!(ty.as_ref(), Term::Nominal { declaration: actual, .. } if declaration == actual) => {
                    fields.get(*index)
                }
                (K::AnonymousField { name, index, .. }, _, Some(Term::RecordValue(fields))) => {
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
        let symbol = normal::observation::hint(self.observe_normal_input_in(
            value,
            0,
            &mut budget,
            &scope.known,
        ));
        *fuel = budget.steps;
        if let Some(symbol) = symbol.map_err(|failure| failure.to_string())? {
            return Ok(PartialStagedValue::known(symbol));
        }
        match &value.kind {
            K::If {
                then_branch,
                else_branch: Some(other),
                ..
            } => {
                let left = self.partial_staged_value_in(then_branch, depth + 1, fuel, scope)?;
                let right = self.partial_staged_value_in(other, depth + 1, fuel, scope)?;
                return PartialStagedValue::common(
                    &left,
                    &right,
                    self.protocols.meta_list,
                    depth + 1,
                    fuel,
                );
            }
            K::Match { arms, .. } if !arms.is_empty() => {
                let mut common =
                    self.partial_staged_value_in(&arms[0].body, depth + 1, fuel, scope)?;
                for arm in &arms[1..] {
                    let next = self.partial_staged_value_in(&arm.body, depth + 1, fuel, scope)?;
                    common = PartialStagedValue::common(
                        &common,
                        &next,
                        self.protocols.meta_list,
                        depth + 1,
                        fuel,
                    )?;
                }
                return Ok(common);
            }
            K::Block {
                statements,
                result: Some(result),
            } => {
                let mut scoped = scope.fork(fuel)?;
                for statement in statements {
                    match &statement.kind {
                        K::Let { binding, value } if self.immutable.contains(binding) => {
                            let observed =
                                self.partial_staged_value_in(value, depth + 1, fuel, &scoped)?;
                            scoped.insert(*binding, observed, fuel)?;
                        }
                        K::LetPattern {
                            pattern,
                            value,
                            guard: None,
                            otherwise: None,
                        } => {
                            let observed =
                                self.partial_staged_value_in(value, depth + 1, fuel, &scoped)?;
                            let (_, bindings) = self.known_staged_pattern_bindings(
                                pattern,
                                &observed,
                                depth + 1,
                                fuel,
                            )?;
                            if bindings.keys().any(|id| !self.immutable.contains(id)) {
                                return Ok(PartialStagedValue::unknown());
                            }
                            for (binding, value) in bindings {
                                scoped.insert(binding, value, fuel)?;
                            }
                        }
                        _ => return Ok(PartialStagedValue::unknown()),
                    }
                }
                return self.partial_staged_value_in(result, depth + 1, fuel, &scoped);
            }
            _ => {}
        }
        let (shape, fields) = match (&value.kind, &value.ty) {
            (K::Tuple(fields), _) => (PartialStagedShape::Tuple, fields),
            (K::TypeList(fields), ty) if self.list_element_type(ty).is_some() => {
                nir::type_computations::charge_term(ty, fuel).map_err(|e| e.to_string())?;
                (
                    PartialStagedShape::List {
                        ty: ty.clone(),
                        exact: true,
                    },
                    fields,
                )
            }
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
            let child = self.partial_staged_value_in(field, depth + 1, fuel, scope)?;
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
            match (&value.data, value.known_term()) {
                (_, Some(Term::TupleValue(fields))) if fields.len() == count => {
                    let mut result = Vec::new();
                    for field in fields {
                        nir::type_computations::charge_term(field, &mut fuel)
                            .map_err(|e| e.to_string())?;
                        result.push(PartialStagedValue::known(field.clone()));
                    }
                    Ok(Some(result))
                }
                (PartialStagedData::Aggregate(PartialStagedShape::Tuple, fields), _)
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

    /// Project Type identities from checked tuple evidence without evaluating
    /// its producer. A field's Type domain alone never supplies its value.
    pub(super) fn observed_tuple_type_inputs(
        &mut self,
        value: &n::Expr,
        domains: &[Term],
        span: Span,
    ) -> Result<Vec<Option<Term>>, Error> {
        let evidence = self.observed_tuple_elements(value, domains.len(), span)?;
        let mut result = Vec::with_capacity(domains.len());
        for (index, domain) in domains.iter().enumerate() {
            let known = evidence.as_ref().and_then(|fields| fields.get(index));
            let ty = if *domain == p(P::Type) {
                match known.map(|field| &field.data) {
                    Some(PartialStagedData::Known(ty)) if ty.sort() == Sort::Type => {
                        Some(self.copy_alias_type(ty, span)?)
                    }
                    _ => None,
                }
            } else {
                None
            };
            result.push(ty);
        }
        Ok(result)
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
        self.input_facts
            .partial
            .extend(result.map_err(|e| error(span, e))?.1);
        Ok(())
    }

    /// Observe an immutable ordinary comptime binding without promoting its
    /// storage or reads to specialization inputs. Calls in its initializer
    /// remain invocation work; unknown fields do not erase known siblings.
    pub(super) fn record_observed_initializer(
        &mut self,
        pattern: &n::Pattern,
        value: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let observed = self.partial_staged_value(value, 0, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        let observed = observed.map_err(|e| error(span, e))?;
        self.record_observed_pattern(pattern, &observed, span)
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
            let Some(ty) = value.known_term() else {
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

    /// Let nested expression checks observe earlier prepared input identities.
    /// The source producers remain in the call plan; these facts do not supply
    /// values to required invocation or establish permission to execute them.
    pub(super) fn with_prepared_input_observations<R>(
        &mut self,
        facts: &BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        span: Span,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let scope = PreparedObservations::enter(self, facts, span)?;
        let result = check(self);
        scope.leave(self);
        result
    }

    pub(super) fn observed_lexical_identity(
        &mut self,
        binding: n::LocalId,
        span: Span,
    ) -> Result<Option<std::sync::Arc<Term>>, Error> {
        if !self.locals.contains_key(&binding) || !self.immutable.contains(&binding) {
            return Ok(None);
        }
        if let Some((_, supplied)) =
            self.input_facts
                .supplied
                .shared(binding, &mut self.required_steps, span)?
        {
            return Ok(supplied.cloned());
        }
        Ok(self
            .input_facts
            .partial
            .get(&binding)
            .and_then(|value| match &value.data {
                PartialStagedData::Known(term) => Some(term.clone()),
                _ => None,
            }))
    }

    /// Completed construction data may supply a nested required invocation,
    /// without replacing the enclosing expression's ordinary local reads.
    pub(super) fn observed_generator_input(&self, binding: n::LocalId) -> Option<(&Term, &Term)> {
        if !self.generator_data.contains_key(&binding) || !self.immutable.contains(&binding) {
            return None;
        }
        let domain = self.locals.get(&binding)?;
        let PartialStagedData::Known(value) = &self.input_facts.partial.get(&binding)?.data else {
            return None;
        };
        Some((domain, value))
    }

    pub(super) fn observed_local_type(
        &mut self,
        binding: n::LocalId,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        if self.locals.get(&binding) != Some(&p(P::Type)) {
            return Ok(None);
        }
        let Some(value) = self.input_facts.partial.get(&binding) else {
            return Ok(None);
        };
        let Some(ty) = value.known_term() else {
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
            self.input_facts.partial.insert(binding, value.clone());
            let Some(symbol) = value.known_term() else {
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
                self.input_facts.types.insert(binding, symbol.clone());
            } else {
                self.input_facts
                    .values
                    .insert(binding, (symbol.clone(), ty.clone()));
            }
            self.input_facts.derived.insert(binding);
            self.input_facts
                .specialized
                .retain(|(id, _, _)| *id != binding);
            self.input_facts.specialized.push((binding, ty, symbol));
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
            !self.input_facts.is_staged(input) && !self.input_facts.derived.contains(input)
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
        self.input_facts.derived.extend(bindings);
        // Retain a completed input's value identity for immutable aliases.
        // Unavailable declaration-side Bound inputs remain provenance only.
        if let K::Local(input) = origin.kind {
            if let Some((_, ty, symbol)) = self
                .input_facts
                .specialized
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
                        self.input_facts.types.insert(binding, symbol.clone());
                    } else {
                        self.input_facts
                            .values
                            .insert(binding, (symbol.clone(), ty.clone()));
                    }
                    self.input_facts.derived.insert(binding);
                    self.input_facts
                        .specialized
                        .retain(|(id, _, _)| *id != binding);
                    self.input_facts.specialized.push((binding, ty, symbol));
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
        if self.in_required_comptime || self.input_facts.specialized.is_empty() {
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
                .input_facts
                .specialized
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
        tail_references: &BTreeSet<n::LocalId>,
        source: &hir::Block<'_>,
        initialized: &block_regions::StagedStatements<'_>,
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
            .filter(|(_, (_, snapshot))| !snapshot.parts.is_empty())
            .map(|(index, _)| *index)
            .collect();
        let removed =
            self.consumed_staged_aliases(statements, tail_references, source.span, &preserved)?;
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
            fuel = fuel
                .checked_sub(tail_references.len())
                .ok_or("staged pattern liveness budget exhausted")?;
            for binding in tail_references {
                *counts.entry(*binding).or_default() += 1;
            }
            let mut pending = Vec::new();
            let mut waiters: BTreeMap<n::LocalId, Vec<usize>> = BTreeMap::new();
            for (index, (_, snapshot)) in initialized {
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
                let (_, snapshot) = &initialized[&index];
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
        for (index, (statement_source, snapshot)) in initialized {
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
            let source = *statement_source;
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
        tail_references: &BTreeSet<n::LocalId>,
        span: Span,
    ) -> Result<(), Error> {
        let removed =
            self.consumed_staged_aliases(statements, tail_references, span, &BTreeSet::new())?;
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
        tail_references: &BTreeSet<n::LocalId>,
        span: Span,
        preserved: &BTreeSet<usize>,
    ) -> Result<BTreeSet<usize>, Error> {
        if self.input_facts.specialized.is_empty() || !statements.iter().any(|statement| matches!(
            &statement.kind,
            K::Let { value, .. } | K::LetPattern { value, guard: None, otherwise: None, .. }
                if matches!(value.kind, K::Local(_) | K::Field { .. } | K::AnonymousField { .. })
        )) {
            return Ok(BTreeSet::new());
        }
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = (|| {
            fuel = fuel
                .checked_sub(tail_references.len())
                .ok_or("staged alias erasure budget exhausted")?;
            let mut used = tail_references.clone();
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
                                .input_facts
                                .specialized
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
        if self.input_facts.is_empty() {
            return self.expr(body, None);
        }
        let saved = self.input_facts.clone();
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
            self.input_facts = saved;
        }
        isolated?;
        checked
    }

    pub(super) fn invalidate_staged_storage(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        // Required and deferred staged computations own their invocation store.
        // Checking either cannot invalidate the enclosing runtime input snapshot;
        // construction observations still need invalidation inside that store.
        let staged = self.in_required_comptime || self.in_staged_input;
        if self.input_facts.partial.is_empty()
            && self.input_facts.supplied.is_empty()
            && (staged
                || (self.input_facts.specialized.is_empty()
                    && self.input_facts.parameters.is_empty()))
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
                K::ConstAddress { .. } => {}
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
                K::Call { .. } if self.in_required_comptime && !self.generator_data.is_empty() => {
                    // A call can mutate construction storage through aliases.
                    // Only observation snapshots are invalidated here.
                    *fuel = fuel
                        .checked_sub(self.input_facts.partial.len())
                        .ok_or("construction observation invalidation budget exhausted")?;
                    invalidated.extend(self.input_facts.partial.keys().copied());
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
        self.input_facts
            .invalidate(invalidated, staged, &mut self.required_steps, span)
    }

    pub(super) fn freeze_staged_call_inputs(
        &mut self,
        mut call: n::Expr,
        source: &Expr<'_>,
    ) -> Result<n::Expr, Error> {
        // A comptime call executes its inputs in the current invocation.
        // Explicit comptime parameters do not create another staging boundary;
        // freezing here would read caller storage before that invocation exists.
        if matches!(&call.kind, K::Call { callee, .. } if callee.ty.is_comptime_callable()) {
            return Ok(call);
        }
        if self.in_required_comptime
            || self.preparing_inputs.as_ref().is_some_and(|plan| {
                plan.site
                    == n::SourceRange {
                        start: source.span.lo().0,
                        end: source.span.hi().0,
                    }
            })
        {
            return Ok(call);
        }
        let K::Call {
            callee,
            arguments,
            parameter_bindings,
            ..
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
        // Only an ordinary staged operand becomes a required root. Environment
        // operands stay on the runtime call with their acquisition bindings.
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
        let declaration_inputs = self.visible_comptime_parameters(source.span)?;
        // Later defaults observe the mutable preparation frame. The argument
        // retained on the call still delivers the value selected for that slot.
        let mut prepared: BTreeMap<n::LocalId, (n::Expr, Term)> = BTreeMap::new();
        let mut deferred_bindings = BTreeSet::new();
        for argument in arguments {
            if parameters[argument.parameter].sort == Sort::Runtime {
                continue;
            }
            let mut writable = BTreeSet::new();
            if argument.is_default && !(prepared.is_empty() && deferred_bindings.is_empty()) {
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let replacement = (|| -> Result<_, String> {
                    let access = nir::free_bindings::access(
                        &argument.value,
                        |id| prepared.contains_key(&id) || deferred_bindings.contains(&id),
                        &mut fuel,
                    )?;
                    writable = access.writable;
                    let mut locals = BTreeMap::new();
                    for id in access.readonly {
                        if let Some((value, _)) = prepared.get(&id) {
                            let mut remaining = fuel as u64;
                            let result = nir::copy_budget::charge_expression(value, &mut remaining);
                            fuel = remaining as usize;
                            result.map_err(|e| {
                                format!("staged default capture copy budget exhausted: {e:?}")
                            })?;
                            locals.insert(id, value.clone());
                        }
                    }
                    iir::staging::prepare_input_with_layouts(
                        iir::staging::Layouts::from_index(
                            &self.normal_form_metadata(),
                            &self.newtypes,
                        )
                        .with_target(self.target),
                        &argument.value,
                        &locals,
                        &mut fuel,
                    )
                })();
                self.required_steps -= (before - fuel) as u64;
                argument.value = replacement.map_err(|e| error(source.span, e))?;
            }
            let deferred = self.staged_input_can_defer_in(
                &argument.value,
                argument.is_default,
                &deferred_bindings,
                &staged_bindings,
                &declaration_inputs,
                writable.clone(),
                source.span,
            )?;
            if deferred {
                for id in &writable {
                    prepared.remove(id);
                    deferred_bindings.insert(*id);
                }
                if let Some(binding) = parameter_bindings.get(argument.parameter) {
                    deferred_bindings.insert(*binding);
                }
                continue;
            }
            let bindings = if argument.is_default {
                prepared
                    .iter()
                    .map(|(&id, (value, symbol))| (id, value.ty.clone(), symbol.clone()))
                    .collect()
            } else {
                vec![]
            };
            if !writable.is_empty() {
                let (packet, outputs) = dependent_inputs::DefaultOutputs::pack(
                    self,
                    argument.value.clone(),
                    writable,
                    |id| prepared.get(&id).map(|(v, _)| &v.ty),
                    source.span,
                )?;
                let (value, _) = self.required_normal_input(source, packet, bindings)?;
                let mut writes = Vec::new();
                let (result, symbol) =
                    outputs.finish(self, value, source.span, |id, value, symbol| {
                        prepared.insert(id, (value.clone(), symbol));
                        writes.push((id, value));
                    })?;
                if let Some(&id) = parameter_bindings.get(argument.parameter) {
                    prepared.insert(id, (result.clone(), symbol));
                }
                argument.value = dependent_inputs::DefaultOutputs::replay(result, writes);
                continue;
            }
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
                self.constant_expression(&value, ty.clone(), source.span, 0, &mut snapshot)?;
            if !snapshot.bindings.is_empty() {
                return Err(error(
                    source.span,
                    "staged argument graph materialization is not yet supported",
                ));
            }
            if let Some(binding) = parameter_bindings.get(argument.parameter) {
                prepared.insert(*binding, (argument.value.clone(), symbolic));
            }
        }
        Ok(call)
    }

    pub(super) fn staged_input_can_defer(
        &mut self,
        value: &n::Expr,
        is_default: bool,
        deferred_bindings: &BTreeSet<n::LocalId>,
        staged_bindings: &BTreeSet<n::LocalId>,
        declaration_inputs: &[nir::DeclarationId],
        span: Span,
    ) -> Result<bool, Error> {
        self.staged_input_can_defer_in(
            value,
            is_default,
            deferred_bindings,
            staged_bindings,
            declaration_inputs,
            BTreeSet::new(),
            span,
        )
    }

    fn staged_input_can_defer_in(
        &mut self,
        value: &n::Expr,
        is_default: bool,
        deferred_bindings: &BTreeSet<n::LocalId>,
        staged_bindings: &BTreeSet<n::LocalId>,
        declaration_inputs: &[nir::DeclarationId],
        mut block_bindings: BTreeSet<n::LocalId>,
        span: Span,
    ) -> Result<bool, Error> {
        let mut deferred = !block_bindings.is_disjoint(deferred_bindings);
        let mut structural = true;
        let mut local_loops = BTreeSet::new();
        let mut pending = vec![value];
        while let Some(value) = pending.pop() {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "staged input structure budget exhausted"))?;
            // Empty aggregates can depend on declaration values solely through
            // their domain. Observe every construction's domain as well as
            // the explicit identity operands below, without reading a value.
            deferred |= staged_term_is_pending(
                &value.ty,
                declaration_inputs,
                &mut self.required_steps,
                span,
            )?;
            match &value.kind {
                K::Local(binding) if is_default && deferred_bindings.contains(binding) => {
                    deferred = true
                }
                K::Local(binding)
                    if staged_bindings.contains(binding)
                        || self.input_facts.derived.contains(binding) =>
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
                // Forming a named identity does not execute its body. Both
                // callable phases may be data inputs; the Call arm above
                // independently rejects execution of runtime code.
                K::Function(_) => {}
                K::ComptimeFunctionInstance { inputs, .. } => {
                    pending.extend(inputs.expressions().rev())
                }
                K::Closure { captures, .. } if value.ty.is_comptime_callable() => {
                    // The body is separately phase-checked. Its parameters
                    // belong to invocation; only environment construction
                    // reads the surrounding stage here. Derived locals use
                    // the same provenance as direct argument references;
                    // specialization still checks writes/address escape
                    // before installing an owned capture snapshot.
                    for capture in captures {
                        self.required_steps =
                            self.required_steps.checked_sub(1).ok_or_else(|| {
                                error(span, "staged capture structure budget exhausted")
                            })?;
                        if let Some(value) = &capture.value {
                            pending.push(value);
                        } else if staged_bindings.contains(&capture.binding)
                            || self.input_facts.derived.contains(&capture.binding)
                            || (is_default && deferred_bindings.contains(&capture.binding))
                        {
                            deferred = true;
                        } else if !block_bindings.contains(&capture.binding) {
                            structural = false;
                        }
                    }
                }
                K::FunctionInstance { arguments, .. } => {
                    for argument in arguments {
                        deferred |= staged_term_is_pending(
                            argument,
                            declaration_inputs,
                            &mut self.required_steps,
                            span,
                        )?;
                    }
                }
                K::TraitMethod {
                    subject,
                    interface_arguments,
                    method_arguments,
                    ..
                } => {
                    for argument in std::iter::once(subject)
                        .chain(interface_arguments)
                        .chain(method_arguments)
                    {
                        deferred |= staged_term_is_pending(
                            argument,
                            declaration_inputs,
                            &mut self.required_steps,
                            span,
                        )?;
                    }
                }
                K::AssociatedValue {
                    interface, subject, ..
                } => {
                    for argument in std::iter::once(subject).chain(&interface.arguments) {
                        deferred |= staged_term_is_pending(
                            argument,
                            declaration_inputs,
                            &mut self.required_steps,
                            span,
                        )?;
                    }
                }
                K::TypeList(values)
                | K::Tuple(values)
                | K::Record { fields: values, .. }
                | K::AnonymousRecord { fields: values }
                | K::Variant { fields: values, .. } => pending.extend(values),
                K::Not(value)
                | K::OptionalPresent(value)
                | K::OptionalType(value)
                | K::UnsafeScope(value)
                | K::Discard(value)
                | K::RequirePredicate(value)
                | K::EnsurePredicate(value)
                | K::CheckPredicate(value)
                | K::Snapshot(value)
                | K::TakeBits(value)
                | K::TextQuery { subject: value, .. }
                | K::IntegerRepresentation(value)
                | K::TryIntegerConvert(value)
                | K::ComptimeIntegerConvert(value)
                | K::SymbolName(value)
                | K::EnumName(value)
                | K::TypeKind { operand: value, .. }
                | K::TypeLayout { operand: value, .. }
                | K::BitConvert(value)
                | K::ErrorConvert { value, .. }
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
                    pending.extend(result.as_deref());
                    pending.extend(statements.iter().rev());
                }
                K::Let { binding, value } => {
                    // The checked producer owns this fresh local in both a
                    // lexical block and a clause packet. Inspect its initializer
                    // before deciding whether the subsequent reads can freeze.
                    block_bindings.insert(*binding);
                    pending.push(value);
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
                K::TypeValue(ty) | K::ComptimeValue(ty) | K::StagedValue(ty) => {
                    deferred |= staged_term_is_pending(
                        ty,
                        declaration_inputs,
                        &mut self.required_steps,
                        span,
                    )?;
                }
                K::NormalFormType {
                    arguments, plan, ..
                } => {
                    if let Some(plan) = plan {
                        let Some(schedule) =
                            self.checked_input_schedule(plan, arguments.len(), span)?
                        else {
                            structural = false;
                            break;
                        };
                        block_bindings.extend(schedule.locals);
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
                K::Literal(_) | K::Unit | K::OptionalAbsent => {}
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
        // Spread/collection producers share the callee's execution boundary
        // with direct inputs. A comptime call must retain this whole packet,
        // including caller-local writes and exits, for its invocation.
        let comptime_call = matches!(&expression.kind,
            K::CallPreparation { result: Some(call), .. }
                if matches!(&call.kind, K::Call { callee, .. } if callee.ty.is_comptime_callable()));
        if self.in_required_comptime || comptime_call {
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
        let declaration_inputs = self.visible_comptime_parameters(source.span)?;
        if self.staged_input_can_defer(
            &plan.expression,
            false,
            &BTreeSet::new(),
            &staged_bindings,
            &declaration_inputs,
            source.span,
        )? {
            return Ok(expression);
        };
        charge_copy(&plan.expression, &mut self.required_steps, source.span)?;
        let (value, ty) = self.required_normal_input(source, plan.expression.clone(), vec![])?;
        let mut snapshot = Materialization::default();
        let completed = self.constant_expression(&value, ty, source.span, 0, &mut snapshot)?;
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
                    body: A::integer(location, "1".into(), false),
                },
                n::NormalMatchArm {
                    pattern: n::Pattern::Wild,
                    guard: None,
                    body: A::integer(location, "7".repeat(1024), false),
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
            items: vec![A::unit(location); 512],
        });
        assert!(charge_copy(&wide, &mut 64, span).is_err());
        charge_copy(&wide, &mut 4096, span).unwrap();
        let mut deep = A::unit(location);
        for _ in 0..nir::copy_budget::MAX_COPY_DEPTH + 1 {
            deep = A::Block {
                source: location,
                statements: vec![],
                result: Some(Box::new(deep)),
            };
        }
        assert!(charge_copy(&root(deep), &mut 10000, span).is_err());
        charge_copy(&root(A::unit(location)), &mut 64, span).unwrap();
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
