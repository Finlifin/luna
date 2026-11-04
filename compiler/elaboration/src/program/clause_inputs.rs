//! Source-order preparation of ordinary clause inputs and collection values.
use super::*;

#[derive(Default)]
struct ClauseCollection {
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

    fn push_element(&mut self, value: n::Expr) {
        if let Some(CollectionPart::Elements(fields)) = self.parts.last_mut() {
            fields.push(value);
        } else {
            self.parts.push(CollectionPart::Elements(vec![value]));
        }
    }
}

impl Checker<'_, '_> {
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
    ) -> Result<Vec<(Option<hir::common::Ident>, n::Expr)>, Error> {
        if domains.iter().all(|domain| *domain == p(P::Type)) {
            return self.prepared_type_inputs(args, count, statements, span);
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
        let mut inputs = Vec::new();
        let mut collector = ClauseCollection::default();
        let mut known_binders = Vec::new();
        let mut known_values = Vec::new();
        let mut pending_types = parameters
            .iter()
            .zip(domains)
            .filter_map(|(parameter, domain)| (*domain == p(P::Type)).then_some(*parameter))
            .collect::<BTreeSet<_>>();
        // Captured values can determine a nominal domain just like captured
        // Types. Concrete captures are already substituted by the projection;
        // any remaining dependency must wait for the selected instance.
        pending_types.extend(self.nominal_captures(owner, span)?);
        for (argument_index, arg) in args.iter().enumerate() {
            if let Arg::Expand(input) = arg {
                let mut next = positional;
                let next_slot = Self::type_input_slot(&clauses, None, &mut next, span)?;
                if Some(next_slot) == collector_slot {
                    collector.check_append(input.span)?;
                    let domain = self.instantiate_trait_domain(
                        &domains[next_slot],
                        &known_binders,
                        &known_values,
                        span,
                    )?;
                    let dependent = pending_types.iter().any(|id| domain.mentions_rigid(*id));
                    let value = self.expr(input, (!dependent).then_some(&domain))?;
                    let value = self.bind_type_input(value, statements, input.span)?;
                    collector.parts.push(CollectionPart::Spread(value));
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
                    if !pending_types.iter().any(|id| domain.mentions_rigid(*id)) {
                        self.require(&field, &domain, input.span)?;
                    }
                    if domains[slot] == p(P::Type) {
                        pending_types.insert(parameters[slot]);
                    }
                    let binding = self.spread_binding(input.span)?;
                    self.locals.insert(binding, field.clone());
                    pattern.push(n::Pattern::Binding {
                        binding,
                        inner: Box::new(n::Pattern::Wild),
                    });
                    inputs.push((
                        None,
                        n::Expr {
                            source: packet.source,
                            ty: field,
                            kind: K::Local(binding),
                        },
                    ));
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
            if Some(slot) == collector_slot {
                let whole = name.is_some();
                if whole {
                    collector.check_whole(input.span)?;
                } else {
                    collector.check_append(input.span)?;
                }
                let input_domain = if whole {
                    domain.clone()
                } else {
                    self.list_element_type(&domain).cloned().ok_or_else(|| {
                        error(
                            input.span,
                            "variadic clause input requires a bound meta.List<T>",
                        )
                    })?
                };
                let dependent = pending_types
                    .iter()
                    .any(|id| input_domain.mentions_rigid(*id));
                let value = self.expr(input, (!dependent).then_some(&input_domain))?;
                let value = self.bind_type_input(value, statements, input.span)?;
                if whole {
                    collector.whole = Some(value);
                } else {
                    collector.push_element(value);
                }
                continue;
            }
            let dependent = pending_types.iter().any(|id| domain.mentions_rigid(*id));
            let value = self.expr(input, (!dependent).then_some(&domain))?;
            if let K::TypeValue(ty) = &value.kind {
                known_binders.push(nir::DeclarationParameter {
                    declaration: parameters[slot],
                    sort: Sort::Type,
                });
                known_values.push(ty.clone());
                pending_types.remove(&parameters[slot]);
            } else if domains[slot] == p(P::Type) {
                pending_types.insert(parameters[slot]);
            }
            inputs.push((name, self.bind_type_input(value, statements, span)?));
        }
        if let Some(slot) = collector_slot {
            let domain =
                self.instantiate_trait_domain(&domains[slot], &known_binders, &known_values, span)?;
            let value = self.finish_clause_collection(collector, domain, statements, span)?;
            inputs.push((Some(clauses[slot].name), value));
        }
        Ok(inputs)
    }

    fn finish_clause_collection(
        &mut self,
        collection: ClauseCollection,
        domain: Term,
        statements: &mut Vec<n::Expr>,
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
                        ty: domain.clone(),
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
        // All producers have already been bound in source order. A balanced
        // assembly keeps many spreads from creating a linear-depth NIR tree or
        // repeatedly copying the entire accumulated prefix.
        while parts.len() > 1 {
            let mut next = Vec::with_capacity(parts.len().div_ceil(2));
            let mut values = parts.into_iter();
            while let Some(left) = values.next() {
                next.push(if let Some(right) = values.next() {
                    n::Expr {
                        source: source(span),
                        ty: domain.clone(),
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
        self.bind_type_input(value, statements, span)
    }
}
