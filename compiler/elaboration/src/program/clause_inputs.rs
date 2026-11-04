//! Source-order preparation of ordinary clause inputs and collection values.
use super::*;

/// Static identities assist dependent domain checking without replacing the
/// source-ordered producers or the ordinary values retained in their slots.
pub(super) struct PreparedClauseInputs {
    pub inputs: Vec<(Option<hir::common::Ident>, n::Expr)>,
    pub identities: BTreeMap<DeclarationId, Term>,
}

impl std::fmt::Display for PreparedClauseInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(prepared-clause-inputs (inputs")?;
        for (name, value) in &self.inputs {
            if let Some(name) = name {
                write!(f, " (named {:?} {value})", name.name.to_string())?;
            } else {
                write!(f, " (positional {value})")?;
            }
        }
        f.write_str(") (identities")?;
        for (parameter, value) in &self.identities {
            write!(f, " (input {parameter} {value})")?;
        }
        f.write_str("))")
    }
}

/// Routing state is independent of expression checking and execution. Open
/// descriptions and concrete NF preparation consume the same clause slots.
/// A tuple packet supplies fixed inputs; a tail spread supplies the collector.
pub(super) struct ClauseInputRouting<'a, 'h> {
    clauses: &'a [hir::clause::ClauseParam<'h>],
    supplied: Vec<bool>,
    positional: usize,
    collector: Option<usize>,
    whole_collector: bool,
}

impl<'a, 'h> ClauseInputRouting<'a, 'h> {
    pub(super) fn new(clauses: &'a [hir::clause::ClauseParam<'h>], inputs: &[Arg<'_>]) -> Self {
        let collector = clauses
            .last()
            .filter(|clause| matches!(clause.kind, hir::clause::ClauseParamKind::Varadic(..)))
            .map(|_| clauses.len() - 1);
        let whole_collector = collector.is_some_and(|slot| {
            inputs.iter().any(|input|
            matches!(input, Arg::Named(name, _) if name.name == clauses[slot].name.name))
        });
        Self {
            clauses,
            supplied: vec![false; clauses.len()],
            positional: 0,
            collector,
            whole_collector,
        }
    }

    pub(super) fn at_collector(&self) -> bool {
        let next = (self.positional..self.clauses.len()).find(|slot| {
            !matches!(
                self.clauses[*slot].kind,
                hir::clause::ClauseParamKind::Optional(..)
            )
        });
        next.is_some() && next == self.collector
    }

    pub(super) fn slot(
        &mut self,
        name: Option<hir::common::Ident>,
        span: Span,
    ) -> Result<usize, Error> {
        let slot = Checker::type_input_slot(self.clauses, name, &mut self.positional, span)
            .map_err(|_| error(span, "unknown named normal-form argument"))?;
        let supplied = self
            .supplied
            .get_mut(slot)
            .ok_or_else(|| error(span, "normal-form argument count mismatch"))?;
        if Some(slot) == self.collector && name.is_none() {
            if self.whole_collector {
                return Err(error(
                    span,
                    "cannot combine a complete type-list argument with collected inputs",
                ));
            }
        } else if std::mem::replace(supplied, true) {
            return Err(error(span, "duplicate normal-form argument"));
        }
        Ok(slot)
    }

    pub(super) fn packet_slot(&mut self, span: Span) -> Result<usize, Error> {
        // Packet arity belongs to the fixed prefix. Do not silently turn an
        // excess tuple field into a list element with a different protocol.
        if self.at_collector() {
            return Err(error(span, "normal-form argument count mismatch"));
        }
        self.slot(None, span)
    }

    pub(super) fn missing_required(&self) -> bool {
        self.clauses
            .iter()
            .zip(&self.supplied)
            .any(|(clause, supplied)| {
                !supplied
                    && !matches!(
                        clause.kind,
                        hir::clause::ClauseParamKind::Optional(..)
                            | hir::clause::ClauseParamKind::Varadic(..)
                    )
            })
    }
}

impl std::fmt::Display for ClauseInputRouting<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(clause-input-routing (positional {}) (whole-collector {}) (supplied",
            self.positional, self.whole_collector
        )?;
        for (slot, supplied) in self.supplied.iter().enumerate() {
            if *supplied {
                write!(f, " {slot}")?;
            }
        }
        f.write_str("))")
    }
}

#[derive(Default)]
pub(super) struct ClauseCollection {
    whole: Option<n::Expr>,
    parts: Vec<CollectionPart>,
}

enum CollectionPart {
    Elements(Vec<n::Expr>),
    Spread(n::Expr),
}

impl ClauseCollection {
    fn check_whole(&self, span: Span) -> Result<(), Error> {
        if self.whole.is_some() || !self.parts.is_empty() {
            return Err(error(span, "duplicate or mixed whole collector supply"));
        }
        Ok(())
    }

    fn check_append(&self, span: Span) -> Result<(), Error> {
        if self.whole.is_some() {
            return Err(error(span, "cannot append to a whole collector supply"));
        }
        Ok(())
    }

    pub(super) fn push_element(&mut self, value: n::Expr) {
        if let Some(CollectionPart::Elements(fields)) = self.parts.last_mut() {
            fields.push(value);
        } else {
            self.parts.push(CollectionPart::Elements(vec![value]));
        }
    }
    pub(super) fn push_spread(&mut self, value: n::Expr) {
        self.parts.push(CollectionPart::Spread(value));
    }
}

impl std::fmt::Display for ClauseCollection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(clause-collection (whole")?;
        if let Some(value) = &self.whole {
            write!(f, " {value}")?;
        }
        f.write_str(") (parts")?;
        for part in &self.parts {
            match part {
                CollectionPart::Elements(elements) => {
                    f.write_str(" (elements")?;
                    for value in elements {
                        write!(f, " {value}")?;
                    }
                    f.write_str(")")?;
                }
                CollectionPart::Spread(value) => write!(f, " (spread {value})")?,
            }
        }
        f.write_str("))")
    }
}

/// A pending check keeps resolved source bindings and its original execution
/// position. It is not executable NIR and introduces no second expression tree.
struct PendingClauseInput<'a, 'h> {
    source_index: usize,
    slot: usize,
    name: Option<hir::common::Ident>,
    input: &'a Expr<'h>,
    spread: bool,
}

struct PreparedClauseInput {
    slot: usize,
    name: Option<hir::common::Ident>,
    value: n::Expr,
    spread: bool,
}

impl std::fmt::Display for PendingClauseInput<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(pending-clause-input (source-index {}) (slot {}) (name",
            self.source_index, self.slot
        )?;
        if let Some(name) = self.name {
            write!(f, " {:?}", name.name.as_str())?;
        }
        write!(f, ") (spread {}) {})", self.spread, source(self.input.span))
    }
}

impl std::fmt::Display for PreparedClauseInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(prepared-clause-input (slot {}) (name", self.slot)?;
        if let Some(name) = self.name {
            write!(f, " {:?}", name.name.as_str())?;
        }
        write!(f, ") (spread {}) {})", self.spread, self.value)
    }
}

impl Checker<'_, '_> {
    pub(super) fn copy_clause_identity(&mut self, value: &Term, span: Span) -> Result<Term, Error> {
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let copied = nir::copy_budget::clone_term(value, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        copied.map_err(|failure| error(span, format!("clause identity copy failed: {failure:?}")))
    }

    pub(super) fn clause_has_collector(&self, owner: OwnerId) -> bool {
        self.type_clause_params(owner)
            .last()
            .is_some_and(|parameter| {
                matches!(parameter.kind, hir::clause::ClauseParamKind::Varadic(..))
            })
    }
    pub(super) fn clause_fixed_prefix_count(&self, owner: OwnerId) -> usize {
        self.type_clause_params(owner)
            .iter()
            .filter(|clause| {
                !matches!(
                    clause.kind,
                    hir::clause::ClauseParamKind::Optional(..)
                        | hir::clause::ClauseParamKind::Varadic(..)
                )
            })
            .count()
    }

    pub(super) fn clause_prefix_literal_spreads<'h>(
        &self,
        owner: OwnerId,
        inputs: &[Arg<'h>],
    ) -> Option<Vec<Arg<'h>>> {
        let whole_collector = self.type_clause_params(owner).last().is_some_and(|clause| {
            inputs
                .iter()
                .any(|input| matches!(input, Arg::Named(name, _) if name.name == clause.name.name))
        });
        let mut remaining = self.clause_fixed_prefix_count(owner);
        let mut expanded = Vec::new();
        let mut changed = false;
        for input in inputs {
            match input {
                Arg::Expand(value) if remaining > 0 => match value.kind {
                    H::Tuple(fields) if fields.len() <= remaining || whole_collector => {
                        remaining = remaining.saturating_sub(fields.len());
                        expanded.extend(fields.iter().map(Arg::Positional));
                        changed = true;
                    }
                    H::Unit => {
                        changed = true;
                    }
                    _ => {
                        // A computed packet can determine the remaining prefix
                        // later. Do not flatten subsequent tail list inputs.
                        remaining = 0;
                        expanded.push(input.clone());
                    }
                },
                Arg::Positional(_) => {
                    remaining = remaining.saturating_sub(1);
                    expanded.push(input.clone());
                }
                _ => expanded.push(input.clone()),
            }
        }
        changed.then_some(expanded)
    }

    pub(super) fn prepared_clause_inputs_in_domains(
        &mut self,
        owner: OwnerId,
        args: &[Arg<'_>],
        count: usize,
        statements: &mut Vec<n::Expr>,
        span: Span,
        domains: &[Term],
        parameters: &[DeclarationId],
    ) -> Result<PreparedClauseInputs, Error> {
        if domains.iter().all(|domain| *domain == p(P::Type)) {
            return self
                .prepared_type_inputs(args, count, statements, span)
                .map(|inputs| PreparedClauseInputs {
                    inputs,
                    identities: BTreeMap::new(),
                });
        }
        let collector_slot = self
            .clause_has_collector(owner)
            .then(|| self.type_clause_params(owner).len() - 1);
        let expanded = if collector_slot.is_some() {
            self.clause_prefix_literal_spreads(owner, args)
        } else {
            normal::literal_spreads(args)
        };
        if let Some(expanded) = expanded {
            return self.prepared_clause_inputs_in_domains(
                owner, &expanded, count, statements, span, domains, parameters,
            );
        }
        let clauses = self.type_clause_params(owner);
        let mut positional = 0;
        let mut prepared: Vec<Vec<PreparedClauseInput>> =
            (0..args.len()).map(|_| Vec::new()).collect();
        let mut preparations: Vec<Vec<n::Expr>> = (0..args.len()).map(|_| Vec::new()).collect();
        let mut waiting = Vec::new();
        let mut known_binders = Vec::new();
        let mut known_values = Vec::new();
        let mut identities = BTreeMap::new();
        let mut failed_observations = BTreeMap::new();
        let mut domain_dependencies = BTreeSet::new();
        for domain in domains {
            let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let charged = nir::type_computations::charge_term(domain, &mut fuel);
            self.required_steps -= (before - fuel) as u64;
            charged.map_err(|failure| error(span, failure.to_string()))?;
            domain_dependencies.extend(
                domain
                    .rigid_parameters()
                    .map_err(|failure| error(span, failure.to_string()))?
                    .into_keys(),
            );
        }
        let mut pending_domains = parameters
            .iter()
            .zip(domains)
            .filter_map(|(parameter, domain)| {
                (*domain == p(P::Type) || domain_dependencies.contains(parameter))
                    .then_some(*parameter)
            })
            .collect::<BTreeSet<_>>();
        // Captured values can determine a nominal domain just like captured
        // Types. Concrete captures are already substituted by the projection;
        // any remaining dependency must wait for the selected instance.
        pending_domains.extend(self.nominal_captures(owner, span)?);
        for (argument_index, arg) in args.iter().enumerate() {
            let statements = &mut preparations[argument_index];
            if let Arg::Expand(input) = arg {
                let mut next = positional;
                let next_slot = Self::type_input_slot(&clauses, None, &mut next, span)?;
                if Some(next_slot) == collector_slot {
                    let domain = self.instantiate_trait_domain(
                        &domains[next_slot],
                        &known_binders,
                        &known_values,
                        span,
                    )?;
                    let dependent = pending_domains.iter().any(|id| domain.mentions_rigid(*id));
                    if dependent {
                        waiting.push(PendingClauseInput {
                            source_index: argument_index,
                            slot: next_slot,
                            name: None,
                            input,
                            spread: true,
                        });
                    } else {
                        let value = self.expr(input, Some(&domain))?;
                        let value = self.bind_type_input(value, statements, input.span)?;
                        prepared[argument_index].push(PreparedClauseInput {
                            slot: next_slot,
                            name: None,
                            value,
                            spread: true,
                        });
                    }
                    positional = next;
                    continue;
                }
                let packet = self.expr(input, None)?;
                let positional_domains = clauses
                    .iter()
                    .zip(domains)
                    .filter_map(|(clause, domain)| {
                        (!matches!(clause.kind, hir::clause::ClauseParamKind::Optional(..)))
                            .then_some(domain.clone())
                    })
                    .collect::<Vec<_>>();
                let fields = self.prepared_packet_fields(
                    &packet,
                    args,
                    argument_index,
                    &positional_domains,
                    input.span,
                )?;
                let mut pattern = Vec::with_capacity(fields.len());
                for field in fields {
                    let slot = Self::type_input_slot(&clauses, None, &mut positional, span)?;
                    if Some(slot) == collector_slot {
                        return Err(error(
                            input.span,
                            "tuple input spread cannot cross into a list collector",
                        ));
                    }
                    let domain = domains
                        .get(slot)
                        .ok_or_else(|| error(span, "type argument count mismatch"))?;
                    let domain = self.instantiate_trait_domain(
                        &domain,
                        &known_binders,
                        &known_values,
                        span,
                    )?;
                    if !pending_domains.iter().any(|id| domain.mentions_rigid(*id)) {
                        self.require(&field, &domain, input.span)?;
                    }
                    if domains[slot] == p(P::Type) {
                        pending_domains.insert(parameters[slot]);
                    }
                    let binding = self.spread_binding(input.span)?;
                    self.locals.insert(binding, field.clone());
                    pattern.push(n::Pattern::Binding {
                        binding,
                        inner: Box::new(n::Pattern::Wild),
                    });
                    prepared[argument_index].push(PreparedClauseInput {
                        slot,
                        name: None,
                        value: n::Expr {
                            source: packet.source,
                            ty: field,
                            kind: K::Local(binding),
                        },
                        spread: false,
                    });
                }
                // Destructure once even when the packet contains no fields:
                // its producer's effects and early exits still belong here.
                statements.push(n::Expr {
                    source: packet.source,
                    ty: if is_never(&packet.ty) {
                        p(P::Never)
                    } else {
                        p(P::Void)
                    },
                    kind: K::LetPattern {
                        pattern: n::Pattern::Tuple(pattern),
                        value: Box::new(packet),
                        guard: None,
                        otherwise: None,
                    },
                });
                continue;
            }
            let (name, input) = match arg {
                Arg::Positional(input) => (None, *input),
                Arg::Named(name, input) => (Some(*name), *input),
                _ => {
                    return Err(error(span, "unsupported nominal input routing protocol"));
                }
            };
            let slot = Self::type_input_slot(&clauses, name, &mut positional, span)?;
            let domain = domains
                .get(slot)
                .ok_or_else(|| error(span, "type argument count mismatch"))?;
            let domain =
                self.instantiate_trait_domain(&domain, &known_binders, &known_values, span)?;
            let input_domain = if Some(slot) == collector_slot && name.is_none() {
                self.list_element_type(&domain).cloned().ok_or_else(|| {
                    error(
                        input.span,
                        "variadic clause input requires a bound meta.List<T>",
                    )
                })?
            } else {
                domain
            };
            let dependent = pending_domains
                .iter()
                .any(|id| input_domain.mentions_rigid(*id));
            if dependent {
                waiting.push(PendingClauseInput {
                    source_index: argument_index,
                    slot,
                    name,
                    input,
                    spread: false,
                });
                continue;
            }
            let value = self.expr(input, Some(&input_domain))?;
            let needed_for_domain = domain_dependencies.contains(&parameters[slot]);
            let known = if domains[slot] == p(P::Type)
                || (needed_for_domain && (Some(slot) != collector_slot || name.is_some()))
            {
                let mut budget = comptime_runtime::Budget {
                    steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                    max_limbs: 4096,
                };
                let before = budget.steps;
                let observed =
                    self.observe_normal_input_in(&value, 0, &mut budget, &BTreeMap::new());
                self.required_steps -= (before - budget.steps) as u64;
                let known = match observed {
                    Ok(value) => Some(value),
                    Err(normal::observation::Failure::Unavailable) => None,
                    Err(failure) => {
                        failed_observations.insert(parameters[slot], failure);
                        None
                    }
                };
                if let Some(ty) = &known {
                    known_binders.push(nir::DeclarationParameter {
                        declaration: parameters[slot],
                        sort: ty.sort(),
                    });
                    known_values.push(self.copy_clause_identity(ty, span)?);
                    identities.insert(parameters[slot], self.copy_clause_identity(ty, span)?);
                    pending_domains.remove(&parameters[slot]);
                } else {
                    pending_domains.insert(parameters[slot]);
                }
                known
            } else {
                None
            };
            let mut value = self.bind_type_input(value, statements, span)?;
            if let Some(ty) = known.filter(|_| domains[slot] == p(P::Type)) {
                // The source producer remains in its original preparation slot.
                // Static identity evidence is not permission to erase execution.
                value.kind = K::TypeValue(ty);
            }
            prepared[argument_index].push(PreparedClauseInput {
                slot,
                name,
                value,
                spread: false,
            });
        }
        // Check each pending source once after collecting static input identities.
        // Checking order never determines the order of generated preparations.
        for pending in waiting {
            let mut domain = self.instantiate_trait_domain(
                &domains[pending.slot],
                &known_binders,
                &known_values,
                span,
            )?;
            if Some(pending.slot) == collector_slot && pending.name.is_none() && !pending.spread {
                domain = self.list_element_type(&domain).cloned().ok_or_else(|| {
                    error(
                        pending.input.span,
                        "variadic clause input requires a bound meta.List<T>",
                    )
                })?;
            }
            let dependent = pending_domains.iter().any(|id| domain.mentions_rigid(*id));
            let value = self
                .expr(pending.input, (!dependent).then_some(&domain))
                .map_err(|failure| {
                    // Observation is optional unless a check needs the missing domain.
                    // Do not replace unrelated errors or reject independently typed inputs.
                    if failure.needs_context {
                        if let Some((_, observation)) = failed_observations
                            .iter()
                            .find(|(id, _)| domain.mentions_rigid(**id))
                        {
                            return error(
                                pending.input.span,
                                format!(
                                    "{}; required Type observation failed: {observation}",
                                    failure.message
                                ),
                            );
                        }
                    }
                    failure
                })?;
            let value = self.bind_type_input(
                value,
                &mut preparations[pending.source_index],
                pending.input.span,
            )?;
            prepared[pending.source_index].push(PreparedClauseInput {
                slot: pending.slot,
                name: pending.name,
                value,
                spread: pending.spread,
            });
        }
        let mut inputs = Vec::new();
        let mut collector = ClauseCollection::default();
        for (values, mut preparation) in prepared.into_iter().zip(preparations) {
            statements.append(&mut preparation);
            for input in values {
                if Some(input.slot) != collector_slot {
                    inputs.push((input.name, input.value));
                } else if input.name.is_some() {
                    collector.check_whole(span)?;
                    collector.whole = Some(input.value);
                } else {
                    collector.check_append(span)?;
                    if input.spread {
                        collector.push_spread(input.value);
                    } else {
                        collector.push_element(input.value);
                    }
                }
            }
        }
        if let Some(slot) = collector_slot {
            let domain =
                self.instantiate_trait_domain(&domains[slot], &known_binders, &known_values, span)?;
            let value = self.finish_clause_collection(collector, domain, statements, span)?;
            inputs.push((Some(clauses[slot].name), value));
        }
        Ok(PreparedClauseInputs { inputs, identities })
    }

    fn finish_clause_collection(
        &mut self,
        collection: ClauseCollection,
        domain: Term,
        statements: &mut Vec<n::Expr>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        // Whole supply already owns its source-position binding. Only a new
        // assembly needs another binding in the ordinary clause packet.
        let whole = collection.whole.is_some();
        let value = self.clause_collection_expression(collection, domain, span)?;
        if whole {
            Ok(value)
        } else {
            self.bind_type_input(value, statements, span)
        }
    }

    /// Assemble already checked inputs without moving their producers. Both
    /// ordinary clause packets and opaque head plans use this expression.
    pub(super) fn clause_collection_expression(
        &mut self,
        collection: ClauseCollection,
        domain: Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        if let Some(value) = collection.whole {
            self.require_expression(&value.ty, &domain, span)?;
            return Ok(value);
        }
        let element = self
            .list_element_type(&domain)
            .cloned()
            .ok_or_else(|| error(span, "variadic clause input requires a bound meta.List<T>"))?;
        let mut parts = Vec::with_capacity(collection.parts.len());
        for part in collection.parts {
            let value = match part {
                CollectionPart::Elements(fields) => {
                    for field in &fields {
                        self.require_expression(&field.ty, &element, span)?;
                    }
                    n::Expr {
                        source: source(span),
                        ty: if fields.iter().any(|field| is_never(&field.ty)) {
                            p(P::Never)
                        } else {
                            domain.clone()
                        },
                        kind: K::TypeList(fields),
                    }
                }
                CollectionPart::Spread(value) => {
                    self.require_expression(&value.ty, &domain, span)?;
                    value
                }
            };
            parts.push(value);
        }
        // Left-to-right leaf order is retained even when a caller leaves its
        // contiguous tail producers inline. Balanced assembly avoids a linear
        // NIR depth and repeated copies of the entire accumulated prefix.
        while parts.len() > 1 {
            let mut next = Vec::with_capacity(parts.len().div_ceil(2));
            let mut values = parts.into_iter();
            while let Some(left) = values.next() {
                next.push(if let Some(right) = values.next() {
                    n::Expr {
                        source: source(span),
                        ty: if is_never(&left.ty) || is_never(&right.ty) {
                            p(P::Never)
                        } else {
                            domain.clone()
                        },
                        kind: K::ListConcat(Box::new(left), Box::new(right)),
                    }
                } else {
                    left
                });
            }
            parts = next;
        }
        let value = parts.pop().unwrap_or_else(|| n::Expr {
            source: source(span),
            ty: domain,
            kind: K::TypeList(Vec::new()),
        });
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_clause_dump_separates_executable_inputs_from_identity_evidence() {
        let id = |local| DeclarationId { package: 7, local };
        let symbol = Term::Symbol("quote\"\nline".into());
        let prepared = PreparedClauseInputs {
            inputs: vec![(
                None,
                n::Expr {
                    source: n::SourceRange { start: 0, end: 1 },
                    ty: p(P::Bool),
                    kind: K::Local(n::LocalId { owner: 1, index: 2 }),
                },
            )],
            identities: BTreeMap::from([(id(9), symbol), (id(3), Term::Bool(true))]),
        };
        let dump = prepared.to_string();
        assert_eq!(dump, prepared.to_string());
        assert!(dump.starts_with("(prepared-clause-inputs (inputs (positional "));
        assert!(dump.contains("(local 1 2)"), "{dump}");
        assert!(dump.contains("quote\\\"\\nline"), "{dump}");
        assert!(dump.find(&id(3).to_string()).unwrap() < dump.find(&id(9).to_string()).unwrap());
        assert!(dump.contains(") (identities (input "), "{dump}");
    }
}
