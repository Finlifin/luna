//! Destination evidence for source-ordered clause operands. An observed Type
//! can contextualize an earlier operand. Its supplier still runs and admits
//! its actual value at its own source position.
use super::*;

pub(super) struct ClauseDomains<'a, 'hir> {
    owner: OwnerId,
    parameters: &'a [DeclarationId],
    domains: &'a [Term],
    prefix: &'a [Term],
    subject: Option<&'a Term>,
    sources: Vec<Option<&'hir Expr<'hir>>>,
    observed: BTreeMap<usize, Term>,
}

impl<'a, 'hir> ClauseDomains<'a, 'hir> {
    pub(super) fn new(
        owner: OwnerId,
        parameters: &'a [DeclarationId],
        domains: &'a [Term],
        prefix: &'a [Term],
        subject: Option<&'a Term>,
        clauses: &[hir::clause::ClauseParam<'_>],
        args: &[Arg<'hir>],
        span: Span,
    ) -> Self {
        let sources = if args.iter().any(|arg| matches!(arg, Arg::Named(..))) {
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
            Checker::route_type_slots(clauses, routed, span).unwrap_or_default()
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
        let domain = &self.domains[slot];
        let waiting = input_observation::pending_domain_inputs(
            domain,
            self.parameters.iter().enumerate().map(|(index, id)| {
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
            let index = self.parameters.iter().position(|p| *p == id).unwrap();
            if self.domains[index] == p(P::Type)
                && let Some(source) = self.sources.get_mut(index).and_then(Option::take)
                && let Some(ty) = checker.generator_static_type_hint(source, 0, None)?
            {
                self.observed.insert(index, ty);
            } else {
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

    pub(super) fn confirm(
        &self,
        checker: &Checker<'_, '_>,
        slot: usize,
        actual: &Term,
        span: Span,
    ) -> Result<(), Error> {
        if let Some(ty) = self.observed.get(&slot) {
            checker.require(actual, ty, span)?;
        }
        Ok(())
    }
}
