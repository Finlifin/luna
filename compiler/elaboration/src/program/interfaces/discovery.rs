//! Concrete member source discovery preserves lexical visibility and phase tiers.
use super::*;

impl Checker<'_, '_> {
    // Inspect declaration edges before inferring inputs: an unresolved impl
    // matters only to tiers that can actually supply the requested member.
    fn inherited_member_tier_available(
        &self,
        root: nir::DeclarationId,
        name: &str,
        preferred: bool,
        span: Span,
        fuel: &mut usize,
    ) -> Result<bool, Error> {
        let mut pending = vec![root];
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            *fuel = fuel
                .checked_sub(1)
                .ok_or_else(|| error(span, "member source search budget exhausted"))?;
            if !seen.insert(id) {
                continue;
            }
            let contract = self
                .checked_traits
                .iter()
                .find(|t| t.declaration == id)
                .ok_or_else(|| error(span, "unknown interface in member lookup"))?;
            if contract.methods.iter().any(|m| m.name == name)
                || contract.associated.iter().any(|a| a.name == name)
            {
                if contract.comptime_only == preferred {
                    return Ok(true);
                }
            } else {
                *fuel = fuel
                    .checked_sub(contract.parents.len())
                    .ok_or_else(|| error(span, "member source search budget exhausted"))?;
                pending.extend(contract.parents.iter().map(|parent| parent.declaration));
            }
        }
        Ok(false)
    }

    pub(super) fn discovered_member_interface(
        &mut self,
        subject: &Term,
        name: &str,
        site: hir::HirId,
        span: Span,
    ) -> Result<Option<n::InterfaceInstance>, Error> {
        let Some(plan) = self.package.trait_sources.get(&site).cloned() else {
            return Ok(None);
        };
        let mut budget = Budget {
            steps: usize::try_from(self.required_steps)
                .unwrap_or(usize::MAX)
                .min(100_000),
            ..Budget::default()
        };
        let initial = budget.steps;
        let result = (|| {
            plan.charge(&mut budget.steps)
                .map_err(|message| error(span, message))?;
            if !plan.pending_exports.is_empty()
                || plan
                    .names
                    .iter()
                    .any(|name| !name.pending_exports.is_empty())
            {
                return Err(error(
                    span,
                    "trait source visibility awaits generated exports",
                ));
            }
            let mut visible = BTreeSet::new();
            for entry in &plan.names {
                match &entry.fallback {
                    Ok(Some(id)) => {
                        if let Some(owner) = self.package.definition_owner(*id) {
                            visible.insert(declaration(
                                self.package,
                                &self.package_identities,
                                owner,
                            ));
                        }
                    }
                    Ok(None) => {}
                    Err(_) if entry.ambiguous_traits.is_some() => {}
                    Err(message) => {
                        return Err(error(
                            span,
                            format!("trait source `{}`: {message}", entry.name),
                        ));
                    }
                }
            }
            if visible.is_empty()
                && !plan
                    .names
                    .iter()
                    .any(|entry| entry.ambiguous_traits.is_some())
            {
                return Ok(None);
            }
            if !self.interfaces_ready {
                return Err(error(
                    span,
                    "required comptime evaluation is waiting for the trait implementation world",
                ));
            }
            let candidates = self
                .implementation_index
                .candidates_metered(None, &[], subject, &mut budget)
                .map_err(|failure| {
                    error(span, format!("receiver source search failed: {failure:?}"))
                })?;
            let comptime = self.comptime_member_context();
            for preferred in if comptime {
                vec![true, false]
            } else {
                vec![false]
            } {
                for entry in &plan.names {
                    let Some(traits) = &entry.ambiguous_traits else {
                        continue;
                    };
                    for id in traits {
                        let owner = self.package.definition_owner(*id).ok_or_else(|| {
                            error(span, "ambiguous trait has no definition owner")
                        })?;
                        let interface = declaration(self.package, &self.package_identities, owner);
                        let contract = self
                            .checked_traits
                            .iter()
                            .find(|t| t.declaration == interface)
                            .ok_or_else(|| {
                                error(span, "ambiguous trait has no checked contract")
                            })?;
                        if !comptime && contract.comptime_only {
                            continue;
                        }
                        if self.inherited_member_tier_available(
                            interface,
                            name,
                            preferred,
                            span,
                            &mut budget.steps,
                        )? {
                            let message = entry.fallback.as_ref().err().ok_or_else(|| {
                                error(span, "invalid ambiguous trait source evidence")
                            })?;
                            return Err(error(
                                span,
                                format!("trait source `{}`: {message}", entry.name),
                            ));
                        }
                    }
                }
                let mut sources = Vec::new();
                let mut pending = false;
                for id in candidates.declarations() {
                    budget.steps = budget
                        .steps
                        .checked_sub(1)
                        .ok_or_else(|| error(span, "receiver source budget exhausted"))?;
                    let Some(head) = self.implementation_index.head(*id) else {
                        return Err(error(span, "receiver source index is stale"));
                    };
                    let Some(interface) = head.interface.filter(|id| visible.contains(id)) else {
                        continue;
                    };
                    let Some(contract) = self
                        .checked_traits
                        .iter()
                        .find(|t| t.declaration == interface)
                    else {
                        return Err(error(span, "visible trait has no checked contract"));
                    };
                    if (!comptime && contract.comptime_only)
                        || !self.inherited_member_tier_available(
                            interface,
                            name,
                            preferred,
                            span,
                            &mut budget.steps,
                        )?
                    {
                        continue;
                    }
                    let mut allowance = Budget {
                        steps: budget.steps.min(10_000),
                        ..budget
                    };
                    let reserved = allowance.steps;
                    let inferred = head.infer_interface_from_subject_metered(
                        subject,
                        &self.context,
                        &mut allowance,
                        self.implementation_index.parameter_domains(),
                        Some(self.implementation_index.nominal_layouts()),
                    );
                    budget.steps -= reserved - allowance.steps;
                    let source = match inferred {
                        Ok(source) => source,
                        Err(
                            crate::implementation::TypeAssessment::Excluded(_)
                            | crate::implementation::TypeAssessment::ValueIndexMismatch { .. }
                            | crate::implementation::TypeAssessment::ForbiddenSubject(_),
                        ) => continue,
                        Err(_) => {
                            pending = true;
                            continue;
                        }
                    };
                    let inherited = self.inherited_member_sources_metered(
                        subject,
                        &source,
                        name,
                        span,
                        &mut budget.steps,
                    )?;
                    let inherited: Vec<_> = inherited
                        .into_iter()
                        .filter(|origin| {
                            self.checked_traits.iter().any(|contract| {
                                contract.declaration == origin.declaration
                                    && contract.comptime_only == preferred
                                    && (contract.methods.iter().any(|method| method.name == name)
                                        || contract
                                            .associated
                                            .iter()
                                            .any(|member| member.name == name))
                            })
                        })
                        .collect();
                    if inherited.is_empty() {
                        continue;
                    }
                    let resolution = self
                        .resolve_trait_capability_metered(&source, subject, &mut budget)
                        .map_err(|message| error(span, message))?;
                    match resolution.selection {
                        crate::implementation::Selection::Selected(_) => {
                            for origin in inherited {
                                if !sources.iter().any(|old| self.same_interface(old, &origin)) {
                                    sources.push(origin);
                                }
                            }
                        }
                        crate::implementation::Selection::NoApplicableCandidate => {}
                        _ => pending = true,
                    }
                }
                if pending {
                    return Err(error(
                        span,
                        "trait member source remains unresolved; cannot select a fallback",
                    ));
                }
                if sources.len() > 1 {
                    return Err(error(
                        span,
                        "ambiguous visible trait member; specify its interface source",
                    ));
                }
                if let Some(source) = sources.pop() {
                    return Ok(Some(source));
                }
            }
            Ok(None)
        })();
        self.required_steps = self
            .required_steps
            .saturating_sub((initial - budget.steps) as u64);
        result
    }
}
