//! Preserve resolver-selected candidates when moving code to fresh identities.
use super::*;
use hir::definitions::ResolutionEvidence;
use std::sync::Arc;

type Candidates = Arc<[(hir::common::DefId, hir::common::DefId)]>;

impl Rebind<'_, '_> {
    fn candidates(&mut self, source: Option<&Candidates>) -> Result<Option<Candidates>, Error> {
        let Some(source) = source else {
            return Ok(None);
        };
        self.budget = self.budget.checked_sub(source.len()).ok_or_else(|| {
            error(
                rustc_span::DUMMY_SP,
                "generated member resolution budget exhausted",
            )
        })?;
        if self.definitions.is_empty() {
            return Ok(Some(source.clone()));
        }
        Ok(Some(
            source
                .iter()
                .map(|(container, member)| {
                    (
                        self.definitions
                            .get(container)
                            .copied()
                            .unwrap_or(*container),
                        self.definitions.get(member).copied().unwrap_or(*member),
                    )
                })
                .collect(),
        ))
    }

    pub(super) fn resolution(&mut self) -> Result<Vec<(HirId, ResolutionEvidence)>, Error> {
        self.budget = self
            .budget
            .checked_sub(self.bindings.len())
            .ok_or_else(|| {
                error(
                    rustc_span::DUMMY_SP,
                    "generated member resolution budget exhausted",
                )
            })?;
        let mut bindings: Vec<_> = self.bindings.iter().map(|(a, b)| (*a, *b)).collect();
        bindings.sort_by_key(|(old, _)| (old.owner, old.local_id));
        let mut evidence = Vec::new();
        for (old, new) in bindings {
            let projection = self
                .source
                .member_projections
                .get(&old)
                .map(|id| self.definitions.get(id).copied().unwrap_or(*id));
            let deferred = self.candidates(self.source.deferred_member_projections.get(&old))?;
            let access = self.candidates(self.source.member_access.get(&old))?;
            let trait_sources = self
                .source
                .trait_sources
                .get(&old)
                .map(|source| {
                    source
                        .charge(&mut self.budget)
                        .map_err(|message| error(rustc_span::DUMMY_SP, message))?;
                    if !source.names.iter().any(|name| {
                        matches!(name.fallback,
                    Ok(Some(id)) if self.definitions.contains_key(&id))
                    }) {
                        return Ok(source.clone());
                    }
                    let mut plan = (**source).clone();
                    for name in &mut plan.names {
                        if let Ok(Some(id)) = &mut name.fallback {
                            *id = self.definitions.get(id).copied().unwrap_or(*id);
                        }
                    }
                    Ok::<_, Error>(Arc::new(plan))
                })
                .transpose()?;
            let source_origin = self.source.source_origin(old);
            if source_origin.is_some()
                || projection.is_some()
                || deferred.is_some()
                || access.is_some()
                || trait_sources.is_some()
            {
                evidence.push((
                    new,
                    ResolutionEvidence {
                        source_origin,
                        trait_sources,
                        projection,
                        deferred,
                        access,
                    },
                ));
            }
        }
        Ok(evidence)
    }
}
