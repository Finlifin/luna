//! Source struct requests adapted to the shared concrete definition publisher.
use super::CheckedDefinitionPublisher;
use comptime_interpretor::{Budget, Error, ErrorKind, PreparedStructBuild, StructBuildRequest};
use nir::{DeclarationId, program as n};
use std::sync::Arc;

/// Concrete struct request adapter. Identity reservation belongs to the host.
pub struct CheckedStructPublisher<'a> {
    inner: CheckedDefinitionPublisher<'a>,
}
impl<'a> CheckedStructPublisher<'a> {
    pub fn new(baseline: &'a n::Package) -> Self {
        Self {
            inner: CheckedDefinitionPublisher::new(baseline),
        }
    }
    pub fn published_definition_count(&self) -> usize {
        self.inner.published_definition_count()
    }
    pub fn last_diagnostic(&self) -> Option<String> {
        self.inner.last_diagnostic()
    }
    pub fn into_definitions(self) -> Vec<Arc<[n::Struct]>> {
        self.inner.into_definitions().structs
    }
    pub fn prepare<'s>(
        &'s self,
        identity: DeclarationId,
        request: &StructBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedStructBuild<'s>, Error> {
        self.inner.set_diagnostic(None);
        let mut budget = Budget {
            steps: std::mem::take(steps),
            depth: 128,
        };
        let result = self.prepare_inner(identity, request, &mut budget);
        *steps = budget.steps;
        result.map_err(|mut error| {
            error.source = Some(request.source);
            if self.last_diagnostic().is_none() {
                self.inner
                    .set_diagnostic(Some(format!("checked struct publication failed: {error}")));
            }
            error
        })
    }
    fn prepare_inner<'s>(
        &'s self,
        identity: DeclarationId,
        request: &StructBuildRequest,
        budget: &mut Budget,
    ) -> Result<PreparedStructBuild<'s>, Error> {
        let source = request.source;
        let before = usize::try_from(budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let copied = (|| {
            fuel = fuel
                .checked_sub(request.name.len().saturating_add(request.fields.len()))
                .ok_or(())?;
            for field in &request.fields {
                fuel = fuel
                    .checked_sub(field.name.len().saturating_add(1))
                    .ok_or(())?;
                nir::type_computations::charge_term(&field.ty, &mut fuel).map_err(|_| ())?;
            }
            Ok::<_, ()>(())
        })();
        charge(budget, before - fuel)?;
        copied.map_err(|_| Error {
            kind: ErrorKind::StepsExhausted,
            source: Some(source),
        })?;
        let definitions: Arc<[n::Struct]> = vec![n::Struct {
            declaration: identity,
            name: request.name.clone(),
            type_parameters: vec![],
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            fields: request
                .fields
                .iter()
                .map(|field| (field.name.clone(), field.ty.clone()))
                .collect(),
        }]
        .into();

        let prepared = self
            .inner
            .prepare_structs(Arc::clone(&definitions), source, budget)?;
        Ok(PreparedStructBuild {
            definitions,
            result: identity,
            commit: Box::new(move || prepared.commit()),
        })
    }
}
impl std::fmt::Display for CheckedStructPublisher<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(checked-struct-publisher (definitions {}))",
            self.published_definition_count()
        )
    }
}
fn charge(budget: &mut Budget, work: usize) -> Result<(), Error> {
    budget.steps = budget
        .steps
        .checked_sub(u64::try_from(work).unwrap_or(u64::MAX))
        .ok_or(Error {
            kind: ErrorKind::StepsExhausted,
            source: None,
        })?;
    Ok(())
}
