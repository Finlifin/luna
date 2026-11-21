//! Destination evidence for source-ordered clause operands. An observed Type
//! can contextualize an earlier operand. Its supplier still runs and admits
//! its actual value in ordinary explicit-argument or selected-default order.
use super::*;

#[derive(Clone, Copy)]
enum Supplier<'hir> {
    Explicit(&'hir Expr<'hir>),
    Default(&'hir Expr<'hir>),
}

pub(super) struct ClauseDomains<'a, 'hir> {
    owner: OwnerId,
    parameters: &'a [DeclarationId],
    domains: &'a [Term],
    prefix: &'a [Term],
    subject: Option<&'a Term>,
    sources: Vec<Option<Supplier<'hir>>>,
    pub(super) observed: BTreeMap<usize, Term>,
}

impl<'a, 'hir> ClauseDomains<'a, 'hir> {
    pub(super) fn new(
        owner: OwnerId,
        parameters: &'a [DeclarationId],
        domains: &'a [Term],
        prefix: &'a [Term],
        subject: Option<&'a Term>,
        clauses: &[hir::clause::ClauseParam<'hir>],
        args: &[Arg<'hir>],
        span: Span,
    ) -> Self {
        let has_type_default =
            clauses
                .iter()
                .zip(&domains[prefix.len()..])
                .any(|(clause, domain)| {
                    *domain == p(P::Type)
                        && matches!(clause.kind, hir::clause::ClauseParamKind::Optional(..))
                });
        let sources = if has_type_default || args.iter().any(|arg| matches!(arg, Arg::Named(..))) {
            // A computed spread has no observed width. Named slots on either
            // side are still fixed; later positional routes await the packet.
            let mut positioned = true;
            let mut routed = Vec::new();
            for arg in args {
                match arg {
                    Arg::Named(name, value) => routed.push((Some(*name), *value)),
                    Arg::Positional(value) if positioned => routed.push((None, *value)),
                    Arg::Expand(value) if positioned => match value.kind {
                        H::Tuple(fields) => routed.extend(fields.iter().map(|field| (None, field))),
                        H::Unit => {}
                        _ => positioned = false,
                    },
                    _ => {}
                }
            }
            Checker::route_type_slots(clauses, routed, span)
                .unwrap_or_default()
                .into_iter()
                .zip(clauses)
                .map(|(explicit, clause)| match (explicit, &clause.kind) {
                    (Some(value), _) => Some(Supplier::Explicit(value)),
                    (None, hir::clause::ClauseParamKind::Optional(_, _, value)) => {
                        Some(Supplier::Default(value))
                    }
                    _ => None,
                })
                .collect()
        } else {
            vec![]
        };
        Self {
            owner,
            parameters,
            domains,
            prefix,
            subject,
            sources,
            observed: BTreeMap::new(),
        }
    }

    pub(super) fn checking_domain(
        &mut self,
        checker: &mut Checker<'_, '_>,
        slot: usize,
        supplied: &[Option<Term>],
        span: Span,
    ) -> Result<(Term, bool), Error> {
        let parameters = &self.parameters[self.prefix.len()..];
        let domain = &self.domains[self.prefix.len() + slot];
        let waiting = input_observation::pending_domain_inputs(
            domain,
            parameters.iter().enumerate().map(|(index, id)| {
                (
                    *id,
                    supplied[index].is_some() || self.observed.contains_key(&index),
                )
            }),
            &mut checker.required_steps,
            span,
        )?;
        let mut ready = true;
        for id in waiting {
            let index = parameters.iter().position(|p| *p == id).unwrap();
            if !self.observe_type(checker, index, supplied, 0)? {
                ready = false;
            }
        }
        let known = supplied
            .iter()
            .enumerate()
            .map(|(index, value)| value.as_ref().or_else(|| self.observed.get(&index)));
        let domain = checker.supplied_trait_domain(
            self.owner,
            domain,
            known,
            self.prefix,
            self.subject,
            span,
        )?;
        Ok((domain, ready))
    }

    fn observe_type(
        &mut self,
        checker: &mut Checker<'_, '_>,
        slot: usize,
        supplied: &[Option<Term>],
        depth: usize,
    ) -> Result<bool, Error> {
        if supplied[slot].is_some() || self.observed.contains_key(&slot) {
            return Ok(true);
        }
        if depth >= 64 || self.domains[self.prefix.len() + slot] != p(P::Type) {
            return Ok(false);
        }
        let Some(source) = self.sources.get(slot).copied().flatten() else {
            return Ok(false);
        };
        let hint = match source {
            Supplier::Explicit(source) => {
                checker.generator_static_type_hint(source, depth, None)?
            }
            Supplier::Default(source) => {
                self.default_identity(checker, source, slot, supplied, depth)?
            }
        };
        if let Some(ty) = hint {
            self.observed.insert(slot, ty);
            return Ok(true);
        }
        Ok(false)
    }

    fn default_identity(
        &mut self,
        checker: &mut Checker<'_, '_>,
        source: &Expr<'_>,
        slot: usize,
        supplied: &[Option<Term>],
        depth: usize,
    ) -> Result<Option<Term>, Error> {
        // Describe the default in its declaration with rigid prefix inputs.
        // Only after leaving that scope can we import the call's identities.
        let owner = self.owner;
        let template = checker.in_formal_declaration(owner, |this| {
            this.context = this.definition_context(owner);
            this.in_required_comptime = true;
            let count = self.prefix.len() + slot;
            this.required_steps = this
                .required_steps
                .checked_sub(count as u64)
                .ok_or_else(|| error(source.span, "clause default evidence budget exhausted"))?;
            let formals: Vec<_> = self.parameters[..count]
                .iter()
                .zip(self.domains)
                .map(|(&declaration, domain)| Term::Rigid {
                    declaration,
                    sort: Sort::of_comptime_domain(domain),
                })
                .collect();
            this.generator_static_type_hint(source, depth, Some((self.parameters, &formals)))
        })?;
        let Some(template) = template else {
            return Ok(None);
        };
        let dependencies = checker.term_declaration_inputs(&template, source.span)?;
        if dependencies.is_empty() {
            return Ok(Some(template));
        }
        let count = self.prefix.len() + slot;
        checker.required_steps = checker
            .required_steps
            .checked_sub(count as u64)
            .ok_or_else(|| error(source.span, "clause default dependency budget exhausted"))?;
        let mut ids = Vec::new();
        let mut domains = Vec::new();
        let mut values = Vec::new();
        for (index, &id) in self.parameters[..count].iter().enumerate() {
            if !dependencies.contains(&id) {
                continue;
            }
            let value = if let Some(value) = self.prefix.get(index) {
                value
            } else {
                let earlier = index - self.prefix.len();
                if !self.observe_type(checker, earlier, supplied, depth + 1)? {
                    return Ok(None);
                }
                supplied[earlier]
                    .as_ref()
                    .unwrap_or_else(|| &self.observed[&earlier])
            };
            ids.push(id);
            domains.push(&self.domains[index]);
            values.push(checker.copy_alias_type(value, source.span)?);
        }
        if ids.len() != dependencies.len() {
            return Ok(None);
        }
        let frames = checker.call_input_frames();
        nominal_fields::metered(&mut checker.required_steps, |fuel| {
            let substitution = nir::DeclarationSubstitution::bounded_domains(
                &ids,
                &domains,
                &values,
                nir::DeclarationInputScope::Caller(&frames),
                fuel,
            )?;
            substitution.instantiate_bounded(&template, fuel).map(Some)
        })
        .map_err(|failure| error(source.span, failure.to_string()))
    }

    pub(super) fn confirm(
        &mut self,
        checker: &Checker<'_, '_>,
        slot: usize,
        actual: &Term,
        span: Span,
    ) -> Result<(), Error> {
        if let Some(ty) = self.observed.remove(&slot) {
            checker.require_type_identity(actual, &ty, span)?;
        }
        Ok(())
    }
}
