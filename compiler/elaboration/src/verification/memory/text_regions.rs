//! Text backing-region dependencies. This analysis does not grant memory access
//! or prove that a caller-supplied region is live; it preserves that obligation.
use super::*;
use std::collections::{HashSet, VecDeque};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TextRegionOrigin {
    Static,
    Parameter {
        parameter: usize,
        fields: Vec<usize>,
    },
}
impl fmt::Display for TextRegionOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Static => f.write_str("(static-text-region)"),
            Self::Parameter { parameter, fields } => {
                write!(f, "(parameter-text-region {parameter} (fields")?;
                display_owned_path(fields, f)?;
                f.write_str("))")
            }
        }
    }
}
type Regions = BTreeMap<Vec<usize>, BTreeSet<TextRegionOrigin>>;
type Locals = BTreeMap<LocalId, Regions>;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum CallbackKey {
    Named(iir::InstanceKey),
    Closure(usize),
}
impl fmt::Display for CallbackKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(key) => key.fmt(f),
            Self::Closure(id) => write!(f, "(closed-text-callback {id})"),
        }
    }
}
#[derive(Clone, PartialEq)]
struct CodeValue {
    expression: std::sync::Arc<Expr>,
    closure: Option<usize>,
}
impl CodeValue {
    fn new(expression: Expr) -> Self {
        Self {
            expression: std::sync::Arc::new(expression),
            closure: None,
        }
    }
}
struct ClosedClosure {
    expression: std::sync::Arc<Expr>,
    definitions: BTreeMap<LocalId, CodeValue>,
    changed: BTreeSet<LocalId>,
    cost: usize,
}
fn index_code_expression(
    expr: &Expr,
    fuel: &mut usize,
    definitions: &mut BTreeMap<LocalId, CodeValue>,
    seen: &mut BTreeSet<LocalId>,
    changed: &mut BTreeSet<LocalId>,
) -> Result<(), String> {
    match &expr.kind {
        K::Let { binding, value } => {
            if !seen.insert(*binding) {
                changed.insert(*binding);
            }
            if matches!(
                &value.kind,
                K::Closure { .. } | K::Function(_) | K::FunctionInstance { .. } | K::Local(_)
            ) {
                nir::visit::expression(value, fuel, |_, _| Ok(()))?;
                definitions.insert(*binding, CodeValue::new((**value).clone()));
            }
        }
        K::Assign { place, .. } => {
            changed.insert(place.root);
        }
        K::Closure { captures, .. } => {
            for capture in captures {
                if capture.value.is_some() {
                    changed.insert(capture.binding);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SummaryKey {
    instance: iir::InstanceKey,
    callbacks: Vec<(usize, CallbackKey)>,
}
impl fmt::Display for SummaryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(text-summary-key {} (callbacks", self.instance)?;
        for (parameter, callback) in &self.callbacks {
            write!(f, " (parameter {parameter} {callback})")?;
        }
        f.write_str("))")
    }
}

struct LoopRegions {
    identity: LocalId,
    bindings: Vec<LocalId>,
    exits: Option<Locals>,
    backedges: Option<Locals>,
}
struct ExitRegions {
    bindings: Vec<LocalId>,
    locals: Option<Locals>,
}

fn join_locals(
    target: &mut Option<Locals>,
    source: &Locals,
    bindings: &[LocalId],
    fuel: &mut usize,
) -> Result<(), String> {
    let target = target.get_or_insert_with(Locals::new);
    for binding in bindings {
        charge(fuel, 1)?;
        let value = source
            .get(binding)
            .ok_or("text loop lost an outer binding")?;
        charge_regions(value, fuel)?;
        let mut joined = target.remove(binding);
        merge(&mut joined, Some(value.clone()), fuel)?;
        target.insert(*binding, joined.unwrap());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRegionSummary {
    pub declaration: DeclarationId,
    pub arguments: Vec<Term>,
    /// Character reads in explicit expressions and named callees. Cleanup
    /// footprints and region liveness must be composed separately; this set is
    /// not a complete memory-access permission or a cleanup certificate.
    pub reads: BTreeSet<TextRegionOrigin>,
    /// None means no normal return. Empty maps/sets never mean static storage;
    /// guarded leaves can be absent on a particular returned variant.
    pub returned: Option<BTreeMap<Vec<usize>, BTreeSet<TextRegionOrigin>>>,
}
impl fmt::Display for TextRegionSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(text-region-summary (declaration {}) (arguments",
            self.declaration
        )?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        f.write_str(") (reads")?;
        for origin in &self.reads {
            write!(f, " {origin}")?;
        }
        f.write_str(") (returned")?;
        match &self.returned {
            None => f.write_str(" (no-return)")?,
            Some(fields) => {
                for (path, origins) in fields {
                    f.write_str(" (view (fields")?;
                    display_owned_path(path, f)?;
                    f.write_str(") (origins")?;
                    for origin in origins {
                        write!(f, " {origin}")?;
                    }
                    f.write_str("))")?;
                }
            }
        }
        f.write_str("))")
    }
}

fn charge(fuel: &mut usize, amount: usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(amount)
        .ok_or("text region inference budget exhausted")?;
    Ok(())
}
fn charge_regions(regions: &Regions, fuel: &mut usize) -> Result<(), String> {
    for (path, origins) in regions {
        charge(fuel, path.len().saturating_add(1))?;
        for origin in origins {
            charge(
                fuel,
                match origin {
                    TextRegionOrigin::Static => 1,
                    TextRegionOrigin::Parameter { fields, .. } => fields.len().saturating_add(1),
                },
            )?;
        }
    }
    Ok(())
}
fn merge(
    target: &mut Option<Regions>,
    source: Option<Regions>,
    fuel: &mut usize,
) -> Result<(), String> {
    if let Some(source) = source {
        charge_regions(&source, fuel)?;
        let target = target.get_or_insert_with(BTreeMap::new);
        for (path, origins) in source {
            target.entry(path).or_default().extend(origins);
        }
    }
    Ok(())
}
fn project(regions: Regions, prefix: &[usize], fuel: &mut usize) -> Result<Regions, String> {
    charge_regions(&regions, fuel)?;
    Ok(regions
        .into_iter()
        .filter_map(|(path, origins)| {
            path.strip_prefix(prefix)
                .map(|rest| (rest.to_vec(), origins))
        })
        .collect())
}
fn prefix(regions: Regions, prefix: &[usize], fuel: &mut usize) -> Result<Regions, String> {
    charge_regions(&regions, fuel)?;
    let mut output = Regions::new();
    for (path, origins) in regions {
        charge(fuel, prefix.len())?;
        output.insert(prefix.iter().copied().chain(path).collect(), origins);
    }
    Ok(output)
}

/// Derive conditional return dependencies by a bounded fixed point over concrete
/// named instances. Unsupported transfers fail closed, never become evidence.
pub fn infer_text_regions(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<TextRegionSummary, String> {
    Query {
        package,
        fuel,
        current: None,
        local_callables: BTreeMap::new(),
        closed_closures: vec![],
        changed_callables: BTreeSet::new(),
        pending: VecDeque::new(),
        scheduled: HashSet::new(),
        dependents: HashMap::new(),
        reads: vec![],
        loops: vec![],
        exits: vec![],
        cache: HashMap::new(),
        metadata: Round::default(),
    }
    .solve(declaration, arguments)
}
struct Query<'a, 'f> {
    package: &'a Package,
    fuel: &'f mut usize,
    current: Option<SummaryKey>,
    local_callables: BTreeMap<LocalId, CodeValue>,
    closed_closures: Vec<ClosedClosure>,
    changed_callables: BTreeSet<LocalId>,
    pending: VecDeque<SummaryKey>,
    scheduled: HashSet<SummaryKey>,
    dependents: HashMap<SummaryKey, Vec<SummaryKey>>,
    reads: Vec<BTreeSet<TextRegionOrigin>>,
    loops: Vec<LoopRegions>,
    exits: Vec<Option<ExitRegions>>,
    cache: HashMap<SummaryKey, TextRegionSummary>,
    metadata: Round<'static>,
}
impl Query<'_, '_> {
    fn record_exit(&mut self, locals: &Locals) -> Result<(), String> {
        if let Some(Some(exit)) = self.exits.last_mut() {
            join_locals(&mut exit.locals, locals, &exit.bindings, self.fuel)?;
        }
        Ok(())
    }

    fn contract(&mut self, predicate: &Expr, locals: &Locals) -> Result<(), String> {
        if !nir::program::checkable_predicate(predicate) {
            return Err("invalid text region program predicate".into());
        }
        let mut observation = self.locals_copy(locals)?;
        self.observe_value(predicate, &mut observation, 0)?;
        Ok(())
    }

    fn observe_value(
        &mut self,
        value: &Expr,
        locals: &mut Locals,
        depth: usize,
    ) -> Result<Regions, String> {
        // Candidate rollback cannot restore writes into an existing binding.
        // Private temporaries inside an observation block are still permitted.
        nir::visit::expression(value, self.fuel, |expr, _| {
            if matches!(&expr.kind, K::Assign { place, .. } if locals.contains_key(&place.root)) {
                return Err("text observation writes an existing binding".into());
            }
            Ok(())
        })?;
        let outer_loops = std::mem::take(&mut self.loops);
        let mut returned = None;
        let result = self.expr(value, locals, &mut returned, depth);
        self.loops = outer_loops;
        let result = result?;
        if returned.is_some() {
            return Err("text observation has an unsupported return".into());
        }
        result.ok_or_else(|| "text observation does not complete normally".into())
    }

    // Return success knowledge only after accounting for the checks reached in
    // source order. A later known mismatch cannot erase earlier character reads.
    fn bind_pattern(
        &mut self,
        pattern: &Pattern,
        ty: &Term,
        regions: &Regions,
        value: Option<&Expr>,
        locals: &mut Locals,
        depth: usize,
    ) -> Result<Option<bool>, String> {
        if depth >= 128 {
            return Err("text region pattern depth exhausted".into());
        }
        charge(self.fuel, 1)?;
        match pattern {
            Pattern::Wild | Pattern::Unit => Ok(Some(true)),
            Pattern::Literal(literal) => {
                if matches!(literal, nir::program::Literal::String(_)) {
                    charge_regions(regions, self.fuel)?;
                    for origins in regions.values() {
                        self.reads
                            .last_mut()
                            .unwrap()
                            .extend(origins.iter().cloned());
                    }
                }
                Ok(value.and_then(|value| known_pattern(pattern, value, 0)))
            }
            Pattern::Binding { binding, inner } => {
                let matched = self.bind_pattern(inner, ty, regions, value, locals, depth + 1)?;
                if matched != Some(false) {
                    charge_regions(regions, self.fuel)?;
                    locals.insert(*binding, regions.clone());
                }
                Ok(matched)
            }
            Pattern::Tuple(patterns)
            | Pattern::Record {
                fields: patterns, ..
            }
            | Pattern::AnonymousRecord {
                fields: patterns, ..
            } => {
                let fields = owned_fields(Some(self.package), ty, self.fuel)?;
                if patterns.len() != fields.len() {
                    return Err("text pattern field arity mismatch".into());
                }
                let values = value.and_then(|value| match &value.kind {
                    K::Tuple(fields) | K::Record { fields, .. } | K::AnonymousRecord { fields } => {
                        Some(fields.as_slice())
                    }
                    _ => None,
                });
                let mut matched = Some(true);
                for (index, (pattern, ty)) in patterns.iter().zip(fields).enumerate() {
                    charge_regions(regions, self.fuel)?;
                    let nested = project(regions.clone(), &[index], self.fuel)?;
                    match self.bind_pattern(
                        pattern,
                        &ty,
                        &nested,
                        values.and_then(|v| v.get(index)),
                        locals,
                        depth + 1,
                    )? {
                        Some(false) => return Ok(Some(false)),
                        None => matched = None,
                        Some(true) => (),
                    }
                }
                Ok(matched)
            }
            Pattern::OptionalAbsent
            | Pattern::OptionalPresent(_)
            | Pattern::Variant { .. }
            | Pattern::ErrorResult { .. } => {
                let (tag, patterns, fields) =
                    owned_variant_pattern(Some(self.package), pattern, ty, self.fuel)?;
                let constructor = match value {
                    Some(value)
                        if matches!(
                            &value.kind,
                            K::Variant { .. }
                                | K::OptionalAbsent
                                | K::OptionalPresent(_)
                                | K::ErrorConvert {
                                    conversion: nir::error_results::Conversion::Success
                                        | nir::error_results::Conversion::Failure,
                                    ..
                                }
                        ) =>
                    {
                        Some(owned_constructor(value, self.fuel)?)
                    }
                    _ => None,
                };
                if constructor
                    .as_ref()
                    .is_some_and(|(_, actual)| *actual != Some(tag))
                {
                    return Ok(Some(false));
                }
                let mut matched = constructor.as_ref().map(|_| true);
                for (index, (pattern, ty)) in patterns.iter().zip(fields).enumerate() {
                    charge_regions(regions, self.fuel)?;
                    let nested = project(regions.clone(), &[VARIANT_FIELD, tag, index], self.fuel)?;
                    let value = constructor
                        .as_ref()
                        .and_then(|(values, _)| values.get(index));
                    match self.bind_pattern(pattern, &ty, &nested, value, locals, depth + 1)? {
                        Some(false) => return Ok(Some(false)),
                        None => matched = None,
                        Some(true) => (),
                    }
                }
                Ok(matched)
            }
            Pattern::Remap { pattern, bindings } => {
                let matched = self.bind_pattern(pattern, ty, regions, value, locals, depth + 1)?;
                if matched == Some(false) {
                    return Ok(matched);
                }
                let mut remapped = Locals::new();
                for (from, to) in bindings {
                    let value = locals
                        .get(from)
                        .ok_or("text pattern remap source is unavailable")?;
                    charge_regions(value, self.fuel)?;
                    remapped.insert(*to, value.clone());
                }
                locals.extend(remapped);
                Ok(matched)
            }
            Pattern::Observe { binding, inner } => {
                charge_regions(regions, self.fuel)?;
                let saved = locals.insert(*binding, regions.clone());
                let matched = self.bind_pattern(inner, ty, regions, value, locals, depth + 1);
                locals.remove(binding);
                if let Some(saved) = saved {
                    locals.insert(*binding, saved);
                }
                matched
            }
            Pattern::Guard { pattern, condition } => {
                let matched = self.bind_pattern(pattern, ty, regions, value, locals, depth + 1)?;
                if matched == Some(false) {
                    return Ok(matched);
                }
                self.observe_value(condition, locals, depth + 1)?;
                Ok(match constant_bool(condition, 0) {
                    Some(false) => Some(false),
                    Some(true) => matched,
                    None => None,
                })
            }
            Pattern::Then {
                first,
                value: computed,
                next,
            } => {
                let first = self.bind_pattern(first, ty, regions, value, locals, depth + 1)?;
                if first == Some(false) {
                    return Ok(first);
                }
                let computed_regions = self.observe_value(computed, locals, depth + 1)?;
                let next = self.bind_pattern(
                    next,
                    &computed.ty,
                    &computed_regions,
                    Some(computed),
                    locals,
                    depth + 1,
                )?;
                Ok(match (first, next) {
                    (_, Some(false)) => Some(false),
                    (Some(true), Some(true)) => Some(true),
                    _ => None,
                })
            }
            Pattern::Or(alternatives) => {
                let input = self.locals_copy(locals)?;
                let mut successes: Option<Locals> = None;
                let mut matched = Some(false);
                for alternative in alternatives {
                    let mut candidate = self.locals_copy(&input)?;
                    let success = self.bind_pattern(
                        alternative,
                        ty,
                        regions,
                        value,
                        &mut candidate,
                        depth + 1,
                    )?;
                    if success == Some(false) {
                        continue;
                    }
                    if let Some(previous) = successes.as_mut() {
                        // Only common exported identities survive alternative
                        // joins; failed candidate-local aliases never escape.
                        previous.retain(|id, _| candidate.contains_key(id));
                        let bindings: Vec<_> = previous.keys().copied().collect();
                        join_locals(&mut successes, &candidate, &bindings, self.fuel)?;
                    } else {
                        successes = Some(candidate);
                    }
                    matched = None;
                    if success == Some(true) {
                        matched = Some(true);
                        break;
                    }
                }
                if let Some(successes) = successes {
                    *locals = successes;
                }
                Ok(matched)
            }
            _ => Err("text region type-level pattern transport is not implemented".into()),
        }
    }

    fn index_local_callables(&mut self, body: &Function) -> Result<(), String> {
        let mut definitions = BTreeMap::new();
        let mut seen: BTreeSet<_> = body.parameters.iter().copied().collect();
        let mut changed = BTreeSet::new();
        nir::visit::expressions(body, self.fuel, |expr, fuel| {
            index_code_expression(expr, fuel, &mut definitions, &mut seen, &mut changed)
        })?;
        definitions.retain(|binding, _| !changed.contains(binding));
        self.local_callables = definitions;
        self.changed_callables = changed;
        Ok(())
    }

    fn local_callee(&mut self, binding: LocalId) -> Result<CodeValue, String> {
        self.known_local_callee(binding)?.ok_or_else(|| {
            "text region call requires a named concrete callee or stable local callable".into()
        })
    }

    fn known_local_callee(&mut self, binding: LocalId) -> Result<Option<CodeValue>, String> {
        let mut binding = binding;
        loop {
            charge(self.fuel, 1)?;
            let Some(value) = self.local_callables.get(&binding) else {
                return Ok(None);
            };
            if let K::Local(next) = value.expression.kind {
                binding = next;
            } else {
                return Ok(Some(value.clone()));
            }
        }
    }

    fn callable_input(&mut self, value: &Expr) -> Result<Option<CodeValue>, String> {
        self.callable_input_at(value, 0)
    }
    fn callable_input_at(
        &mut self,
        value: &Expr,
        depth: usize,
    ) -> Result<Option<CodeValue>, String> {
        let value = match &value.kind {
            K::Local(binding) => self.known_local_callee(*binding)?,
            K::Function(_) | K::FunctionInstance { .. } | K::Closure { .. } => {
                nir::visit::expression(value, self.fuel, |_, _| Ok(()))?;
                Some(CodeValue::new(value.clone()))
            }
            _ => None,
        };
        value
            .map(|value| self.close_callable(value, depth))
            .transpose()
    }
    fn close_callable(&mut self, mut value: CodeValue, depth: usize) -> Result<CodeValue, String> {
        if depth >= 128 {
            return Err("text callback environment depth exhausted".into());
        }
        charge(self.fuel, 1)?;
        if value.closure.is_some() {
            return Ok(value);
        }
        let K::Closure {
            parameters,
            captures,
            body,
        } = &value.expression.kind
        else {
            return Ok(value);
        };
        let before = *self.fuel;
        let mut definitions = BTreeMap::new();
        let mut changed = BTreeSet::new();
        let mut seen = parameters.iter().copied().collect();
        nir::visit::expression(body, self.fuel, |expr, fuel| {
            index_code_expression(expr, fuel, &mut definitions, &mut seen, &mut changed)
        })?;
        definitions.retain(|binding, _| !changed.contains(binding));
        for capture in captures {
            charge(self.fuel, 1)?;
            if changed.contains(&capture.binding) {
                continue;
            }
            let code = if let Some(initializer) = &capture.value {
                self.callable_input_at(initializer, depth + 1)?
            } else {
                self.known_local_callee(capture.binding)?
                    .map(|code| self.close_callable(code, depth + 1))
                    .transpose()?
            };
            if let Some(code) = code {
                definitions.insert(capture.binding, code);
            }
        }
        let cost = before.saturating_sub(*self.fuel).saturating_add(1);
        for (id, known) in self.closed_closures.iter().enumerate() {
            charge(self.fuel, known.cost.saturating_add(cost))?;
            if known.expression == value.expression
                && known.definitions == definitions
                && known.changed == changed
            {
                value.closure = Some(id);
                return Ok(value);
            }
        }
        value.closure = Some(self.closed_closures.len());
        self.closed_closures.push(ClosedClosure {
            expression: value.expression.clone(),
            definitions,
            changed,
            cost,
        });
        Ok(value)
    }

    fn default_input(
        &mut self,
        value: &Expr,
        prepared: &Locals,
        prepared_code: &BTreeMap<LocalId, CodeValue>,
        depth: usize,
    ) -> Result<(Option<Regions>, Option<CodeValue>), String> {
        let mut scope = self.locals_copy(prepared)?;
        charge(
            self.fuel,
            prepared_code.len().saturating_add(prepared.len()),
        )?;
        let mut definitions = prepared_code.clone();
        let mut seen = prepared.keys().copied().collect();
        let mut changed = BTreeSet::new();
        nir::visit::expression(value, self.fuel, |expr, fuel| {
            index_code_expression(expr, fuel, &mut definitions, &mut seen, &mut changed)
        })?;
        definitions.retain(|binding, _| !changed.contains(binding));
        let caller_code = std::mem::replace(&mut self.local_callables, definitions);
        let caller_changed = std::mem::replace(&mut self.changed_callables, changed);
        let caller_loops = std::mem::take(&mut self.loops);
        self.exits.push(None);
        let mut returned = None;
        let result = (|| {
            let code = self.callable_input(value)?;
            let result = self.expr(value, &mut scope, &mut returned, depth)?;
            if returned.is_some() {
                return Err("text default argument cannot return from its caller".into());
            }
            Ok((result, code))
        })();
        self.exits.pop();
        self.loops = caller_loops;
        self.local_callables = caller_code;
        self.changed_callables = caller_changed;
        result
    }

    fn schedule(&mut self, key: SummaryKey) {
        if self.scheduled.insert(key.clone()) {
            self.pending.push_back(key);
        }
    }

    fn solve(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
    ) -> Result<TextRegionSummary, String> {
        self.function(declaration, arguments)?;
        let mut deferred = Vec::<(SummaryKey, String)>::new();
        while let Some(key) = self.pending.pop_front() {
            charge(self.fuel, 1)?;
            self.scheduled.remove(&key);
            self.current = Some(key.clone());
            let result = self.analyze(&key);
            self.current = None;
            deferred.retain(|(instance, _)| instance != &key);
            let mut summary = match result {
                Ok(summary) => summary,
                Err(error) => {
                    // Observations require normal completion. A newly discovered
                    // callee can still be bottom until its own work item runs.
                    // Retry on dependency growth, but never export a failed body.
                    deferred.push((key, error));
                    continue;
                }
            };
            let previous = &self.cache[&key];
            charge(self.fuel, previous.reads.len())?;
            summary.reads.extend(previous.reads.iter().cloned());
            if let Some(regions) = &previous.returned {
                charge_regions(regions, self.fuel)?;
            }
            merge(&mut summary.returned, previous.returned.clone(), self.fuel)?;
            if &summary != previous {
                self.cache.insert(key.clone(), summary);
                if let Some(callers) = self.dependents.get(&key) {
                    charge(self.fuel, callers.len())?;
                    let callers: Vec<_> = callers.iter().cloned().collect();
                    for caller in callers {
                        self.schedule(caller);
                    }
                }
            }
        }
        if let Some((_, error)) = deferred.into_iter().next() {
            return Err(error);
        }
        self.function(declaration, arguments)
    }

    fn function(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
    ) -> Result<TextRegionSummary, String> {
        self.function_with_callbacks(declaration, arguments, &[])
    }

    fn function_with_callbacks(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
        callbacks: &[(usize, CallbackKey)],
    ) -> Result<TextRegionSummary, String> {
        charge(self.fuel, callbacks.len().saturating_add(1))?;
        for (_, callback) in callbacks {
            if let CallbackKey::Named(callback) = callback {
                for argument in &callback.arguments {
                    bound_type(argument, self.fuel)?;
                }
            }
        }
        for argument in arguments {
            bound_type(argument, self.fuel)?;
        }
        let key = SummaryKey {
            instance: iir::InstanceKey {
                declaration,
                arguments: arguments.to_vec(),
            },
            callbacks: callbacks.to_vec(),
        };
        if let Some(caller) = &self.current {
            let callers = self.dependents.entry(key.clone()).or_default();
            charge(self.fuel, callers.len().saturating_add(1))?;
            if !callers.contains(caller) {
                callers.push(caller.clone());
            }
        }
        if !self.cache.contains_key(&key) {
            // Bottom means no observed normal return yet, not static storage.
            // Every discovered instance is analyzed before a result escapes.
            self.cache.insert(
                key.clone(),
                TextRegionSummary {
                    declaration,
                    arguments: arguments.to_vec(),
                    reads: BTreeSet::new(),
                    returned: None,
                },
            );
            self.schedule(key.clone());
        }
        let summary = &self.cache[&key];
        if let Some(returned) = &summary.returned {
            charge_regions(returned, self.fuel)?;
        }
        charge(self.fuel, summary.reads.len())?;
        Ok(summary.clone())
    }

    fn analyze(&mut self, key: &SummaryKey) -> Result<TextRegionSummary, String> {
        let declaration = key.instance.declaration;
        let arguments = key.instance.arguments.as_slice();
        let (body, arguments) = self.package.runtime_body_checked(declaration, arguments)?;
        if body.phase != FunctionPhase::Runtime || body.type_parameters.len() != arguments.len() {
            return Err("text region inference requires a concrete runtime body".into());
        }
        nir::visit::expressions(body, self.fuel, |expr, fuel| bound_type(&expr.ty, fuel))?;
        charge(self.fuel, body.entry_snapshots.len())?;
        bound_type(&body.signature, self.fuel)?;
        let binders =
            self.metadata
                .binders(Some(self.package), &body.type_parameters, self.fuel)?;
        let body = body
            .instantiate_parameters(&binders, arguments)
            .map_err(|e| e.to_string())?;
        self.index_local_callables(&body)?;
        let Term::Function { parameters, result } = body.signature.callable_signature() else {
            return Err("text region inference requires a callable signature".into());
        };
        if parameters.len() != body.parameters.len() {
            return Err("text region parameter identity mismatch".into());
        }
        for (parameter, callback) in &key.callbacks {
            let binding = *body
                .parameters
                .get(*parameter)
                .ok_or("text callback parameter is missing")?;
            if !self.changed_callables.contains(&binding) {
                let code = match callback {
                    CallbackKey::Named(callback) => {
                        let kind = if callback.arguments.is_empty() {
                            K::Function(callback.declaration)
                        } else {
                            K::FunctionInstance {
                                declaration: callback.declaration,
                                arguments: callback.arguments.clone(),
                            }
                        };
                        CodeValue::new(Expr {
                            source: body.body.source,
                            ty: parameters[*parameter].ty.clone(),
                            kind,
                        })
                    }
                    CallbackKey::Closure(id) => {
                        let closed = self
                            .closed_closures
                            .get(*id)
                            .ok_or("text callback code is unavailable")?;
                        CodeValue {
                            expression: closed.expression.clone(),
                            closure: Some(*id),
                        }
                    }
                };
                self.local_callables.insert(binding, code);
            }
        }
        let mut locals = Locals::new();
        for (parameter, (id, ty)) in body.parameters.iter().zip(parameters).enumerate() {
            let mut regions = Regions::new();
            for leaf in borrowed_input_layout(Some(self.package), &ty.ty, self.fuel)? {
                if let BorrowedInputLeaf::Text(leaf) = leaf {
                    regions.insert(
                        leaf.input_fields.clone(),
                        BTreeSet::from([TextRegionOrigin::Parameter {
                            parameter,
                            fields: leaf.input_fields,
                        }]),
                    );
                }
            }
            locals.insert(*id, regions);
        }
        for snapshot in &body.entry_snapshots {
            let value = locals
                .get(&snapshot.parameter)
                .ok_or("text entry snapshot parameter is unavailable")?;
            charge_regions(value, self.fuel)?;
            if locals.insert(snapshot.binding, value.clone()).is_some() {
                return Err("text entry snapshot binding is not fresh".into());
            }
        }
        self.reads.push(BTreeSet::new());
        self.exits
            .push((!body.postconditions.is_empty()).then(|| ExitRegions {
                bindings: locals.keys().copied().collect(),
                locals: None,
            }));
        let mut returned = None;
        let caller_loops = std::mem::take(&mut self.loops);
        let checked = (|| {
            for predicate in &body.preconditions {
                self.contract(predicate, &locals)?;
            }
            let completed = self.expr(&body.body, &mut locals, &mut returned, 0)?;
            if completed.is_some() {
                self.record_exit(&locals)?;
            }
            merge(&mut returned, completed, self.fuel)?;
            if let Some(origins) = &returned {
                if let Some(exit) = self.exits.last_mut().unwrap().as_mut() {
                    let mut environment = exit
                        .locals
                        .take()
                        .ok_or("text postcondition exit environment is unavailable")?;
                    if let Some(binding) = body.result_binding {
                        charge_regions(origins, self.fuel)?;
                        environment.insert(binding, origins.clone());
                    }
                    for predicate in &body.postconditions {
                        self.contract(predicate, &environment)?;
                    }
                }
            }
            Ok::<(), String>(())
        })();
        self.loops = caller_loops;
        self.exits.pop();
        let reads = self.reads.pop().unwrap();
        checked?;
        if let Some(returned) = &returned {
            for leaf in borrowed_input_layout(Some(self.package), result, self.fuel)? {
                if let BorrowedInputLeaf::Text(leaf) = leaf {
                    if leaf.guards.is_empty()
                        && returned
                            .get(&leaf.input_fields)
                            .is_none_or(BTreeSet::is_empty)
                    {
                        return Err("returned text view has no backing-region origin".into());
                    }
                }
            }
        }
        let summary = TextRegionSummary {
            declaration,
            arguments: key.instance.arguments.clone(),
            reads,
            returned,
        };
        Ok(summary)
    }

    fn translate(
        &mut self,
        origins: &BTreeSet<TextRegionOrigin>,
        actual: &BTreeMap<usize, Regions>,
    ) -> Result<BTreeSet<TextRegionOrigin>, String> {
        let mut translated = BTreeSet::new();
        for origin in origins {
            charge(self.fuel, 1)?;
            match origin {
                TextRegionOrigin::Static => {
                    translated.insert(TextRegionOrigin::Static);
                }
                TextRegionOrigin::Parameter { parameter, fields } => {
                    charge(self.fuel, fields.len())?;
                    let value = actual
                        .get(parameter)
                        .ok_or("missing text region call argument")?;
                    if let Some(origins) = value.get(fields) {
                        charge(self.fuel, origins.len())?;
                        translated.extend(origins.iter().cloned());
                    }
                }
            }
        }
        Ok(translated)
    }

    fn locals_copy(&mut self, locals: &Locals) -> Result<Locals, String> {
        charge(self.fuel, locals.len())?;
        for regions in locals.values() {
            charge_regions(regions, self.fuel)?;
        }
        Ok(locals.clone())
    }
    fn expr(
        &mut self,
        expr: &Expr,
        locals: &mut Locals,
        returned: &mut Option<Regions>,
        depth: usize,
    ) -> Result<Option<Regions>, String> {
        if depth >= 128 {
            return Err("text region expression depth exhausted".into());
        }
        charge(self.fuel, 1)?;
        bound_type(&expr.ty, self.fuel)?;
        let next = depth + 1;
        let mut empty = Some(Regions::new());
        match &expr.kind {
            K::Literal(nir::program::Literal::String(_)) => Ok(Some(BTreeMap::from([(
                vec![],
                BTreeSet::from([TextRegionOrigin::Static]),
            )]))),
            K::Literal(_) | K::Unit | K::Function(_) | K::FunctionInstance { .. } => Ok(empty),
            K::Local(id) => {
                let regions = locals.get(id).ok_or("text region local is unavailable")?;
                charge_regions(regions, self.fuel)?;
                Ok(Some(regions.clone()))
            }
            K::SymbolName(value) | K::EnumName(value) => {
                if self.expr(value, locals, returned, next)?.is_none() {
                    return Ok(None);
                }
                Ok(Some(BTreeMap::from([(
                    vec![],
                    BTreeSet::from([TextRegionOrigin::Static]),
                )])))
            }
            K::Tuple(fields) | K::Record { fields, .. } | K::AnonymousRecord { fields } => {
                for (index, field) in fields.iter().enumerate() {
                    let Some(value) = self.expr(field, locals, returned, next)? else {
                        return Ok(None);
                    };
                    merge(
                        &mut empty,
                        Some(prefix(value, &[index], self.fuel)?),
                        self.fuel,
                    )?;
                }
                Ok(empty)
            }
            K::Variant { index, fields, .. } => {
                for (field, value) in fields.iter().enumerate() {
                    let Some(value) = self.expr(value, locals, returned, next)? else {
                        return Ok(None);
                    };
                    merge(
                        &mut empty,
                        Some(prefix(value, &[VARIANT_FIELD, *index, field], self.fuel)?),
                        self.fuel,
                    )?;
                }
                Ok(empty)
            }
            K::OptionalAbsent => Ok(empty),
            K::OptionalPresent(value) => self
                .expr(value, locals, returned, next)?
                .map(|value| prefix(value, &[VARIANT_FIELD, 1, 0], self.fuel))
                .transpose(),
            K::Field { value, index, .. } | K::AnonymousField { value, index, .. } => self
                .expr(value, locals, returned, next)?
                .map(|value| project(value, &[*index], self.fuel))
                .transpose(),
            K::Let { binding, value } => {
                let Some(value) = self.expr(value, locals, returned, next)? else {
                    return Ok(None);
                };
                locals.insert(*binding, value);
                Ok(empty)
            }
            K::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => {
                let Some(regions) = self.expr(value, locals, returned, next)? else {
                    return Ok(None);
                };
                let bindings: Vec<_> = locals.keys().copied().collect();
                let mut candidate = self.locals_copy(locals)?;
                let matched = self.bind_pattern(
                    pattern,
                    &value.ty,
                    &regions,
                    Some(value),
                    &mut candidate,
                    next,
                )?;
                let mut success = matched != Some(false);
                let mut failure = if matched != Some(true) {
                    Some(self.locals_copy(locals)?)
                } else {
                    None
                };
                if success {
                    if let Some(guard) = guard {
                        if self.expr(guard, &mut candidate, returned, next)?.is_none() {
                            success = false;
                        } else {
                            let condition = constant_bool(guard, 0);
                            if condition != Some(true) {
                                join_locals(&mut failure, &candidate, &bindings, self.fuel)?;
                            }
                            success = condition != Some(false);
                        }
                    }
                }
                if let (Some(mut failure), Some(otherwise)) = (failure, otherwise) {
                    // Failure receives only the outer environment. Candidate
                    // names must not shadow outer bindings in the else branch.
                    if self
                        .expr(otherwise, &mut failure, returned, next)?
                        .is_some()
                    {
                        return Err(
                            "text let-else failure branch must not complete normally".into()
                        );
                    }
                }
                if success {
                    *locals = candidate;
                    Ok(empty)
                } else {
                    Ok(None)
                }
            }
            K::Assign { place, value } => {
                let Some(value) = self.expr(value, locals, returned, next)? else {
                    return Ok(None);
                };
                charge(self.fuel, place.fields.len())?;
                let path: Vec<_> = place.fields.iter().map(|(_, index)| *index).collect();
                let replacement = prefix(value, &path, self.fuel)?;
                let target = locals
                    .get_mut(&place.root)
                    .ok_or("text region assignment root is unavailable")?;
                charge_regions(target, self.fuel)?;
                // A write replaces exactly this subtree, including an absent
                // optional payload. Unrelated sibling dependencies survive.
                target.retain(|fields, _| !fields.starts_with(&path));
                target.extend(replacement);
                Ok(empty)
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let mut scope = self.locals_copy(locals)?;
                for statement in statements {
                    if self.expr(statement, &mut scope, returned, next)?.is_none() {
                        return Ok(None);
                    }
                }
                let result = match result {
                    Some(result) => self.expr(result, &mut scope, returned, next),
                    None => Ok(empty),
                }?;
                if result.is_some() {
                    // Only outer bindings escape the lexical scope. Writes
                    // made before a return have no fallthrough successor.
                    for (id, value) in locals.iter_mut() {
                        *value = scope.remove(id).ok_or("text region scope lost a local")?;
                    }
                }
                Ok(result)
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                if self.expr(condition, locals, returned, next)?.is_none() {
                    return Ok(None);
                }
                if let Some(condition) = constant_bool(condition, 0) {
                    return match (condition, else_branch) {
                        (true, _) => self.expr(then_branch, locals, returned, next),
                        (false, Some(branch)) => self.expr(branch, locals, returned, next),
                        (false, None) => Ok(empty),
                    };
                }
                let mut left = self.locals_copy(locals)?;
                let mut right = self.locals_copy(locals)?;
                let mut result = self.expr(then_branch, &mut left, returned, next)?;
                let otherwise = match else_branch {
                    Some(branch) => self.expr(branch, &mut right, returned, next)?,
                    None => empty,
                };
                for (id, value) in locals.iter_mut() {
                    let mut joined = None;
                    if result.is_some() {
                        merge(&mut joined, left.remove(id), self.fuel)?;
                    }
                    if otherwise.is_some() {
                        merge(&mut joined, right.remove(id), self.fuel)?;
                    }
                    if let Some(joined) = joined {
                        *value = joined;
                    }
                }
                merge(&mut result, otherwise, self.fuel)?;
                Ok(result)
            }
            K::Match { value, arms } => {
                let Some(regions) = self.expr(value, locals, returned, next)? else {
                    return Ok(None);
                };
                let bindings: Vec<_> = locals.keys().copied().collect();
                let mut pending = Some(self.locals_copy(locals)?);
                let mut outgoing = None;
                let mut result = None;
                for arm in arms {
                    let Some(input) = pending.take() else { break };
                    let mut pattern = &arm.pattern;
                    let mut guards = Vec::new();
                    while let Pattern::Guard {
                        pattern: inner,
                        condition,
                    } = pattern
                    {
                        charge(self.fuel, 1)?;
                        guards.push(condition.as_ref());
                        pattern = inner;
                    }
                    guards.reverse();
                    if let Some(guard) = &arm.guard {
                        guards.push(guard);
                    }
                    let mut scope = self.locals_copy(&input)?;
                    let matches = self.bind_pattern(
                        pattern,
                        &value.ty,
                        &regions,
                        Some(value),
                        &mut scope,
                        next,
                    )?;
                    if matches == Some(false) {
                        pending = Some(input);
                        continue;
                    }
                    if matches != Some(true) {
                        pending = Some(input);
                    }
                    let mut accepted = true;
                    for guard in guards {
                        if self.expr(guard, &mut scope, returned, next)?.is_none() {
                            accepted = false;
                            break;
                        }
                        let condition = constant_bool(guard, 0);
                        if condition != Some(true) {
                            // Guard effects survive candidate failure; pattern
                            // bindings do not escape into the next candidate.
                            join_locals(&mut pending, &scope, &bindings, self.fuel)?;
                        }
                        if condition == Some(false) {
                            accepted = false;
                            break;
                        }
                    }
                    if !accepted {
                        continue;
                    }
                    let body = self.expr(&arm.body, &mut scope, returned, next)?;
                    if body.is_some() {
                        join_locals(&mut outgoing, &scope, &bindings, self.fuel)?;
                    }
                    merge(&mut result, body, self.fuel)?;
                }
                if let Some(outgoing) = outgoing {
                    *locals = outgoing;
                }
                Ok(result)
            }
            K::Loop { identity, body } => {
                let mut head = self.locals_copy(locals)?;
                let bindings: Vec<_> = head.keys().copied().collect();
                let mut exits = None;
                loop {
                    let mut iteration = self.locals_copy(&head)?;
                    self.loops.push(LoopRegions {
                        identity: *identity,
                        bindings: bindings.clone(),
                        exits: None,
                        backedges: None,
                    });
                    let result = self.expr(body, &mut iteration, returned, next);
                    let mut flow = self.loops.pop().unwrap();
                    if result?.is_some() {
                        join_locals(&mut flow.backedges, &iteration, &bindings, self.fuel)?;
                    }
                    if let Some(outgoing) = flow.exits {
                        join_locals(&mut exits, &outgoing, &bindings, self.fuel)?;
                    }
                    let Some(backedges) = flow.backedges else {
                        break;
                    };
                    let mut joined = Some(self.locals_copy(&head)?);
                    join_locals(&mut joined, &backedges, &bindings, self.fuel)?;
                    let joined = joined.unwrap();
                    if joined == head {
                        break;
                    }
                    head = joined;
                }
                if let Some(exits) = exits {
                    *locals = exits;
                    Ok(empty)
                } else {
                    Ok(None)
                }
            }
            K::Break(identity) | K::Continue(identity) => {
                let flow = self
                    .loops
                    .iter_mut()
                    .rev()
                    .find(|flow| flow.identity == *identity)
                    .ok_or("text region loop target is unavailable")?;
                let target = if matches!(&expr.kind, K::Break(_)) {
                    &mut flow.exits
                } else {
                    &mut flow.backedges
                };
                join_locals(target, locals, &flow.bindings, self.fuel)?;
                Ok(None)
            }
            K::Return(value) => {
                let value = match value {
                    Some(value) => self.expr(value, locals, returned, next)?,
                    None => empty,
                };
                if value.is_some() {
                    self.record_exit(locals)?;
                }
                merge(returned, value, self.fuel)?;
                Ok(None)
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
            } => {
                let resolved = match &callee.kind {
                    K::Local(binding) => Some(self.local_callee(*binding)?),
                    _ => None,
                };
                let code = resolved
                    .as_ref()
                    .map(|code| code.expression.as_ref())
                    .unwrap_or(callee);
                if let K::Closure {
                    parameters,
                    captures,
                    body,
                } = &code.kind
                {
                    charge(self.fuel, parameters.len().saturating_add(captures.len()))?;
                    validate_argument_plan(arguments, parameter_bindings, parameters.len())?;
                    if arguments.iter().any(|argument| argument.is_default) {
                        return Err("text closure default arguments are not supported".into());
                    }
                    // Construct the environment before evaluating inputs: an
                    // argument may replace a caller slot already captured here.
                    let Some(environment) = self.expr(callee, locals, returned, next)? else {
                        return Ok(None);
                    };
                    let mut scope = Locals::new();
                    for (index, capture) in captures.iter().enumerate() {
                        charge_regions(&environment, self.fuel)?;
                        let value = project(environment.clone(), &[index], self.fuel)?;
                        if scope.insert(capture.binding, value).is_some() {
                            return Err("duplicate text closure capture binding".into());
                        }
                    }
                    let mut input_code = Vec::new();
                    for argument in arguments {
                        let callable = self.callable_input(&argument.value)?;
                        let Some(value) = self.expr(&argument.value, locals, returned, next)?
                        else {
                            return Ok(None);
                        };
                        if scope
                            .insert(parameters[argument.parameter], value)
                            .is_some()
                        {
                            return Err("text closure parameter overlaps another binding".into());
                        }
                        input_code.push((parameters[argument.parameter], callable));
                    }
                    let closed_context = resolved.as_ref().and_then(|code| code.closure);
                    let previous_context = if let Some(id) = closed_context {
                        let context = &self.closed_closures[id];
                        charge(self.fuel, context.cost)?;
                        Some((
                            std::mem::replace(
                                &mut self.local_callables,
                                context.definitions.clone(),
                            ),
                            std::mem::replace(&mut self.changed_callables, context.changed.clone()),
                        ))
                    } else {
                        None
                    };
                    // Install invocation-local code only after every input has
                    // been evaluated in the caller's environment.
                    for (binding, callable) in &mut input_code {
                        let old = if self.changed_callables.contains(binding) {
                            self.local_callables.remove(binding)
                        } else if let Some(code) = callable.take() {
                            self.local_callables.insert(*binding, code)
                        } else {
                            self.local_callables.remove(binding)
                        };
                        *callable = old;
                    }
                    let outer_loops = std::mem::take(&mut self.loops);
                    self.exits.push(None);
                    let mut closure_returns = None;
                    let completed = self.expr(body, &mut scope, &mut closure_returns, next);
                    self.exits.pop();
                    self.loops = outer_loops;
                    for (binding, previous) in input_code {
                        self.local_callables.remove(&binding);
                        if let Some(previous) = previous {
                            self.local_callables.insert(binding, previous);
                        }
                    }
                    if let Some((definitions, changed)) = previous_context {
                        self.local_callables = definitions;
                        self.changed_callables = changed;
                    }
                    merge(&mut closure_returns, completed?, self.fuel)?;
                    return Ok(closure_returns);
                }
                let (declaration, types) = match &code.kind {
                    K::Function(id) => (*id, &[][..]),
                    K::FunctionInstance {
                        declaration,
                        arguments,
                    } => (*declaration, arguments.as_slice()),
                    _ => return Err("text region call requires a named concrete callee".into()),
                };
                if self.expr(callee, locals, returned, next)?.is_none() {
                    return Ok(None);
                }
                let (body, _) = self.package.runtime_body_checked(declaration, types)?;
                charge(
                    self.fuel,
                    arguments.len().saturating_add(parameter_bindings.len()),
                )?;
                validate_argument_plan(arguments, parameter_bindings, body.parameters.len())?;
                let has_defaults = arguments.iter().any(|argument| argument.is_default);
                if has_defaults && parameter_bindings != &body.parameters {
                    return Err("text default parameter frame does not match callee".into());
                }
                let mut actual = BTreeMap::new();
                let mut prepared = Locals::new();
                let mut prepared_code = BTreeMap::new();
                let mut callbacks = Vec::new();
                for argument in arguments {
                    let (value, callback) = if argument.is_default {
                        self.default_input(&argument.value, &prepared, &prepared_code, next)?
                    } else {
                        let code = self.callable_input(&argument.value)?;
                        (self.expr(&argument.value, locals, returned, next)?, code)
                    };
                    let Some(value) = value else {
                        return Ok(None);
                    };
                    if has_defaults {
                        let binding = parameter_bindings[argument.parameter];
                        charge_regions(&value, self.fuel)?;
                        prepared.insert(binding, value.clone());
                        if let Some(code) = &callback {
                            prepared_code.insert(binding, code.clone());
                        }
                    }
                    if let Some(callback) = callback {
                        let instance = if let Some(id) = callback.closure {
                            Some(CallbackKey::Closure(id))
                        } else {
                            match &callback.expression.kind {
                                K::Function(declaration) => {
                                    Some(CallbackKey::Named(iir::InstanceKey {
                                        declaration: *declaration,
                                        arguments: vec![],
                                    }))
                                }
                                K::FunctionInstance {
                                    declaration,
                                    arguments,
                                } => Some(CallbackKey::Named(iir::InstanceKey {
                                    declaration: *declaration,
                                    arguments: arguments.clone(),
                                })),
                                _ => None,
                            }
                        };
                        if let Some(instance) = instance {
                            callbacks.push((argument.parameter, instance));
                        }
                    }
                    if actual.insert(argument.parameter, value).is_some() {
                        return Err("duplicate text region call argument".into());
                    }
                }
                callbacks.sort_by_key(|(parameter, _)| *parameter);
                let summary = self.function_with_callbacks(declaration, types, &callbacks)?;
                let reads = self.translate(&summary.reads, &actual)?;
                self.reads.last_mut().unwrap().extend(reads);
                let Some(result) = summary.returned else {
                    return Ok(None);
                };
                for (path, origins) in result {
                    let translated = self.translate(&origins, &actual)?;
                    empty.as_mut().unwrap().insert(path, translated);
                }
                Ok(empty)
            }
            K::TextQuery { subject, query } => {
                let Some(value) = self.expr(subject, locals, returned, next)? else {
                    return Ok(None);
                };
                if *query == nir::program::TextQueryKind::Byte {
                    let origins = value
                        .get(&vec![0])
                        .filter(|origins| !origins.is_empty())
                        .ok_or("text byte read has no backing-region origin")?;
                    charge(self.fuel, origins.len())?;
                    self.reads
                        .last_mut()
                        .unwrap()
                        .extend(origins.iter().cloned());
                }
                Ok(empty)
            }
            K::NewtypeConvert(value) => {
                let Some(regions) = self.expr(value, locals, returned, next)? else {
                    return Ok(None);
                };
                let conversion = nir::newtypes::conversion(
                    &self.package.newtypes,
                    &value.ty,
                    &expr.ty,
                    &[],
                    self.fuel,
                )
                .map_err(|error| error.to_string())?
                .ok_or("invalid text view newtype conversion edge")?;
                let wraps = conversion.direction() == nir::newtypes::Direction::Wrap;
                Ok(Some(if wraps {
                    prefix(regions, &[0], self.fuel)?
                } else {
                    project(regions, &[0], self.fuel)?
                }))
            }
            K::ErrorConvert {
                conversion:
                    nir::error_results::Conversion::Embed | nir::error_results::Conversion::Product,
                value,
            } => {
                let mapping = error_tag_mapping(expr, Some(self.package), self.fuel)?;
                let Some(regions) = self.expr(value, locals, returned, next)? else {
                    return Ok(None);
                };
                charge_regions(&regions, self.fuel)?;
                let mut transported = Regions::new();
                for (mut path, origins) in regions {
                    // Repack the destination slot, not its backing origin:
                    // caller parameter paths still identify the original input.
                    remap_error_path(&mut path, &mapping, self.fuel)?;
                    transported.entry(path).or_default().extend(origins);
                }
                Ok(Some(transported))
            }
            K::ErrorConvert { .. } => {
                let (fields, tag) = owned_constructor(expr, self.fuel)?;
                let [value] = fields else {
                    return Err("text error transport requires one payload".into());
                };
                self.expr(value, locals, returned, next)?
                    .map(|value| prefix(value, &[VARIANT_FIELD, tag.unwrap(), 0], self.fuel))
                    .transpose()
            }
            K::Closure { captures, .. } => {
                for (index, capture) in captures.iter().enumerate() {
                    let value = if let Some(value) = &capture.value {
                        let Some(value) = self.expr(value, locals, returned, next)? else {
                            return Ok(None);
                        };
                        value
                    } else {
                        let value = locals
                            .get(&capture.binding)
                            .ok_or("text capture origin is unavailable")?;
                        charge_regions(value, self.fuel)?;
                        value.clone()
                    };
                    merge(
                        &mut empty,
                        Some(prefix(value, &[index], self.fuel)?),
                        self.fuel,
                    )?;
                }
                Ok(empty)
            }
            K::Snapshot(value) | K::UnsafeScope(value) => self.expr(value, locals, returned, next),
            K::Discard(value)
            | K::RequirePredicate(value)
            | K::EnsurePredicate(value)
            | K::CheckPredicate(value)
            | K::Not(value)
            | K::FloatNegate(value)
            | K::IntegerNegate(value)
            | K::BitConvert(value)
            | K::TakeBits(value)
            | K::IntegerRepresentation(value)
            | K::BitSlice { value, .. } => Ok(self
                .expr(value, locals, returned, next)?
                .map(|_| Regions::new())),
            K::IntegerBinary { left, right, .. }
            | K::FloatBinary { left, right, .. }
            | K::BitEqual(left, right)
            | K::BitConcat(left, right)
            | K::BitIndex(left, right)
            | K::TemplateEqual(left, right) => {
                for operand in [left, right] {
                    if self.expr(operand, locals, returned, next)?.is_none() {
                        return Ok(None);
                    }
                }
                Ok(empty)
            }
            K::BitWindow { value, start, end } => {
                for operand in [value, start, end] {
                    if self.expr(operand, locals, returned, next)?.is_none() {
                        return Ok(None);
                    }
                }
                Ok(empty)
            }
            K::Intrinsic { inputs, .. } => {
                // Only admitted scalar operations have no hidden memory
                // behavior. Their source-ordered inputs may still read text.
                nir::intrinsics::checked(expr).map_err(|e| e.to_string())?;
                for input in inputs {
                    if self.expr(input, locals, returned, next)?.is_none() {
                        return Ok(None);
                    }
                }
                Ok(empty)
            }
            K::And(left, right) | K::Or(left, right) => {
                if self.expr(left, locals, returned, next)?.is_none() {
                    return Ok(None);
                }
                let runs_when = matches!(&expr.kind, K::And(..));
                match constant_bool(left, 0) {
                    Some(value) if value != runs_when => Ok(empty),
                    Some(_) => Ok(self
                        .expr(right, locals, returned, next)?
                        .map(|_| Regions::new())),
                    None => {
                        let mut taken = self.locals_copy(locals)?;
                        if self.expr(right, &mut taken, returned, next)?.is_some() {
                            let bindings: Vec<_> = locals.keys().copied().collect();
                            let mut joined = Some(self.locals_copy(locals)?);
                            join_locals(&mut joined, &taken, &bindings, self.fuel)?;
                            *locals = joined.unwrap();
                        }
                        // The short-circuited path still continues even when
                        // evaluating the right operand would return or diverge.
                        Ok(empty)
                    }
                }
            }
            K::ScalarEqual(a, b)
            | K::ScalarLess(a, b)
            | K::ScalarLessEqual(a, b)
            | K::ScalarGreater(a, b)
            | K::ScalarGreaterEqual(a, b) => {
                for operand in [a, b] {
                    let Some(regions) = self.expr(operand, locals, returned, next)? else {
                        return Ok(None);
                    };
                    // Text comparison observes characters, not just the view
                    // descriptor. Non-text scalar operands have no regions.
                    charge_regions(&regions, self.fuel)?;
                    for origins in regions.into_values() {
                        self.reads.last_mut().unwrap().extend(origins);
                    }
                }
                Ok(empty)
            }
            _ => Err(format!(
                "text region transfer is not implemented at {}",
                expr.source
            )),
        }
    }
}
