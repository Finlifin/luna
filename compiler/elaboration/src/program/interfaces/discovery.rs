//! Concrete member source discovery preserves lexical visibility and phase tiers.
use super::*;

impl Checker<'_, '_> {
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
                    Err(message) => {
                        return Err(error(
                            span,
                            format!("trait source `{}`: {message}", entry.name),
                        ));
                    }
                }
            }
            if visible.is_empty() {
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
                    if contract.comptime_only != preferred
                        || !contract.methods.iter().any(|method| method.name == name)
                    {
                        continue;
                    }
                    let allowance = Budget {
                        steps: budget.steps.min(10_000),
                        ..budget
                    };
                    budget.steps -= allowance.steps;
                    let inferred = head.infer_interface_from_subject(
                        subject,
                        &self.context,
                        allowance,
                        self.implementation_index.parameter_domains(),
                        Some(self.implementation_index.nominal_layouts()),
                    );
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
                    if sources.iter().any(|old| self.same_interface(old, &source)) {
                        continue;
                    }
                    let allowance = Budget {
                        steps: budget.steps.min(10_000),
                        ..budget
                    };
                    budget.steps -= allowance.steps;
                    let resolution = self
                        .resolve_trait_capability(&source, subject, allowance)
                        .map_err(|message| error(span, message))?;
                    match resolution.selection {
                        crate::implementation::Selection::Selected(_) => sources.push(source),
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
