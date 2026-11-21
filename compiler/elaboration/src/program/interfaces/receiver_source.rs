//! Source evidence for routing a method call on a Type-valued expression.
//! Classifying the receiver never invokes a producer or supplies its arguments.
use super::*;

enum Source {
    Nominal(OwnerId),
    // A computation has a known Type domain only when an alias declares it.
    Expression,
    ComputedAlias,
    Other,
}

impl Checker<'_, '_> {
    // Ordinary calls and contextual classification share this decision. A
    // resolved container member already has a source; probing the metatype
    // first could silently replace that member with a different function.
    pub(in crate::program) fn computed_type_receiver_domain(
        &mut self,
        value: &Expr<'_>,
        projection: hir::HirId,
    ) -> Result<Option<Term>, Error> {
        if self.package.member_projections.contains_key(&projection) {
            return Ok(None);
        }
        if matches!(
            value.kind,
            H::ResolvedDef { .. } | H::Projection(..) | H::NFApplication(..)
        ) {
            match self.method_receiver_source(value)? {
                Source::ComputedAlias => return Ok(Some(p(P::Type))),
                Source::Nominal(parent)
                    if self
                        .receiver_source_member(projection, parent, value.span)?
                        .is_some() =>
                {
                    return Ok(None);
                }
                _ => {}
            }
        }
        if Self::is_computed_method_subject(value)
            && self.local_field_domain(value)? == Some(p(P::Type))
        {
            Ok(Some(p(P::Type)))
        } else {
            Ok(None)
        }
    }

    // A deferred name can occur in several containers. Only the declaration
    // authorized for this container at this use site supplies source evidence.
    fn receiver_source_member(
        &mut self,
        projection: hir::HirId,
        parent: OwnerId,
        span: Span,
    ) -> Result<Option<hir::common::DefId>, Error> {
        let Some(candidates) = self
            .package
            .deferred_member_projections
            .get(&projection)
            .or_else(|| self.package.member_access.get(&projection))
        else {
            return Ok(None);
        };
        self.required_steps = self
            .required_steps
            .checked_sub(candidates.len() as u64)
            .ok_or_else(|| error(span, "type receiver classification budget exhausted"))?;
        let parent = self.package.owner_definition(parent);
        let mut members = candidates
            .iter()
            .filter_map(|(container, member)| (Some(*container) == parent).then_some(*member));
        let first = members.next();
        Ok(first.filter(|id| members.all(|other| other == *id)))
    }

    fn method_receiver_source(&mut self, value: &Expr<'_>) -> Result<Source, Error> {
        enum Step {
            Apply,
            LeaveAlias(OwnerId),
            Select(hir::HirId),
        }
        // Unwind applications and aliases before selecting a member on their
        // result. This uses the same work budget for nested and chained paths.
        let mut head = value;
        let mut steps = Vec::new();
        let mut active_aliases = BTreeSet::new();
        let mut member = None;
        'visit: loop {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(head.span, "type receiver classification budget exhausted"))?;
            let id = if let Some(selected) = member.take() {
                Some(selected)
            } else {
                match head.kind {
                    H::ResolvedDef { def_id, .. } => Some(def_id),
                    H::NFApplication(base, _) => {
                        steps.push(Step::Apply);
                        head = base;
                        continue;
                    }
                    H::Projection(base, _) => {
                        if let Some(id) = self.package.member_projections.get(&head.hir_id) {
                            Some(*id)
                        } else if self
                            .package
                            .deferred_member_projections
                            .contains_key(&head.hir_id)
                        {
                            steps.push(Step::Select(head.hir_id));
                            head = base;
                            continue;
                        } else {
                            return Ok(Source::Other);
                        }
                    }
                    _ => None,
                }
            };
            let mut source = if let Some(id) = id {
                let Some(owner) = self.package.definition_owner(id) else {
                    return Ok(Source::Other);
                };
                match &self.package.item(owner).unwrap().kind {
                    ItemKind::TypeAlias(target, ..) => {
                        if !active_aliases.insert(owner) {
                            return Err(error(head.span, "cyclic type alias in method receiver"));
                        }
                        steps.push(Step::LeaveAlias(owner));
                        head = target;
                        continue;
                    }
                    ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Newtype(..) => {
                        Source::Nominal(owner)
                    }
                    _ => Source::Other,
                }
            } else if Self::is_computed_method_subject(head) {
                Source::Expression
            } else {
                Source::Other
            };
            while let Some(step) = steps.pop() {
                match step {
                    Step::Apply
                        if !matches!(source, Source::Nominal(_) | Source::ComputedAlias) =>
                    {
                        source = Source::Expression;
                    }
                    Step::Apply => {}
                    Step::LeaveAlias(owner) => {
                        active_aliases.remove(&owner);
                        if matches!(source, Source::Expression) {
                            source = Source::ComputedAlias;
                        }
                    }
                    Step::Select(site) => {
                        let Source::Nominal(parent) = source else {
                            return Ok(Source::Other);
                        };
                        member = self.receiver_source_member(site, parent, head.span)?;
                        if member.is_none() {
                            return Ok(Source::Other);
                        }
                        continue 'visit;
                    }
                }
            }
            return Ok(source);
        }
    }
}
