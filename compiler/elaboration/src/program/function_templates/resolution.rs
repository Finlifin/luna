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
            let source_origin = self.source.source_origin(old);
            if source_origin.is_some()
                || projection.is_some()
                || deferred.is_some()
                || access.is_some()
            {
                evidence.push((
                    new,
                    ResolutionEvidence {
                        source_origin,
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
