//! Text backing-region dependencies. This analysis does not grant memory access
//! or prove that a caller-supplied region is live; it preserves that obligation.
use super::*;

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

struct LoopRegions {
    identity: LocalId,
    bindings: Vec<LocalId>,
    exits: Option<Locals>,
    backedges: Option<Locals>,
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

/// Derive conditional return dependencies for a concrete, non-recursive named
/// body. Unsupported transfers fail closed, never become empty/static evidence.
pub fn infer_text_regions(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<TextRegionSummary, String> {
    Query {
        package,
        fuel,
        active: vec![],
        reads: vec![],
        loops: vec![],
        cache: HashMap::new(),
        metadata: Round::default(),
    }
    .function(declaration, arguments)
}
struct Query<'a, 'f> {
    package: &'a Package,
    fuel: &'f mut usize,
    active: Vec<iir::InstanceKey>,
    reads: Vec<BTreeSet<TextRegionOrigin>>,
    loops: Vec<LoopRegions>,
    cache: HashMap<iir::InstanceKey, TextRegionSummary>,
    metadata: Round<'static>,
}
impl Query<'_, '_> {
    fn pattern_value(
        &mut self,
        value: &Expr,
        locals: &mut Locals,
        depth: usize,
    ) -> Result<Regions, String> {
        // Candidate rollback cannot restore writes into an existing binding.
        // Private temporaries inside an observation block are still permitted.
        nir::visit::expression(value, self.fuel, |expr, _| {
            if matches!(&expr.kind, K::Assign { place, .. } if locals.contains_key(&place.root)) {
                return Err("text pattern observation writes an existing binding".into());
            }
            Ok(())
        })?;
        let outer_loops = std::mem::take(&mut self.loops);
        let mut returned = None;
        let result = self.expr(value, locals, &mut returned, depth);
        self.loops = outer_loops;
        let result = result?;
        if returned.is_some() {
            return Err("text pattern observation has an unsupported return".into());
        }
        result.ok_or_else(|| "text pattern observation does not complete normally".into())
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
                self.pattern_value(condition, locals, depth + 1)?;
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
                let computed_regions = self.pattern_value(computed, locals, depth + 1)?;
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

    fn function(
        &mut self,
        declaration: DeclarationId,
        arguments: &[Term],
    ) -> Result<TextRegionSummary, String> {
        charge(self.fuel, 1)?;
        for argument in arguments {
            bound_type(argument, self.fuel)?;
        }
        let key = iir::InstanceKey {
            declaration,
            arguments: arguments.to_vec(),
        };
        if let Some(summary) = self.cache.get(&key) {
            if let Some(returned) = &summary.returned {
                charge_regions(returned, self.fuel)?;
            }
            charge(self.fuel, summary.reads.len())?;
            return Ok(summary.clone());
        }
        if self.active.len() >= 64 || self.active.contains(&key) {
            return Err("recursive text region inference requires a fixed-point contract".into());
        }
        let (body, arguments) = self.package.runtime_body_checked(declaration, arguments)?;
        if body.phase != FunctionPhase::Runtime || body.type_parameters.len() != arguments.len() {
            return Err("text region inference requires a concrete runtime body".into());
        }
        nir::visit::expressions(body, self.fuel, |expr, fuel| bound_type(&expr.ty, fuel))?;
        bound_type(&body.signature, self.fuel)?;
        let binders =
            self.metadata
                .binders(Some(self.package), &body.type_parameters, self.fuel)?;
        let body = body
            .instantiate_parameters(&binders, arguments)
            .map_err(|e| e.to_string())?;
        let Term::Function { parameters, result } = body.signature.callable_signature() else {
            return Err("text region inference requires a callable signature".into());
        };
        if parameters.len() != body.parameters.len() {
            return Err("text region parameter identity mismatch".into());
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
        if !body.preconditions.is_empty() || !body.postconditions.is_empty() {
            return Err("text region program-contract transport is not implemented".into());
        }
        self.active.push(key.clone());
        self.reads.push(BTreeSet::new());
        let mut returned = None;
        let caller_loops = std::mem::take(&mut self.loops);
        let checked = self.expr(&body.body, &mut locals, &mut returned, 0);
        self.loops = caller_loops;
        self.active.pop();
        let reads = self.reads.pop().unwrap();
        merge(&mut returned, checked?, self.fuel)?;
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
            arguments: key.arguments.clone(),
            reads,
            returned,
        };
        self.cache.insert(key, summary.clone());
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
                merge(returned, value, self.fuel)?;
                Ok(None)
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
            } => {
                let (declaration, types) = match &callee.kind {
                    K::Function(id) => (*id, &[][..]),
                    K::FunctionInstance {
                        declaration,
                        arguments,
                    } => (*declaration, arguments.as_slice()),
                    _ => return Err("text region call requires a named concrete callee".into()),
                };
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
                for argument in arguments {
                    let value = if argument.is_default {
                        let mut scope = self.locals_copy(&prepared)?;
                        let caller_loops = std::mem::take(&mut self.loops);
                        let mut default_returns = None;
                        let result =
                            self.expr(&argument.value, &mut scope, &mut default_returns, next);
                        self.loops = caller_loops;
                        let result = result?;
                        if default_returns.is_some() {
                            return Err(
                                "text default argument cannot return from its caller".into()
                            );
                        }
                        result
                    } else {
                        self.expr(&argument.value, locals, returned, next)?
                    };
                    let Some(value) = value else {
                        return Ok(None);
                    };
                    if has_defaults {
                        charge_regions(&value, self.fuel)?;
                        prepared.insert(parameter_bindings[argument.parameter], value.clone());
                    }
                    if actual.insert(argument.parameter, value).is_some() {
                        return Err("duplicate text region call argument".into());
                    }
                }
                let summary = self.function(declaration, types)?;
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
