//! Concrete, body-free struct publication against completed declaration facts.
//! This path does not reenter source elaboration or execute an initializer.
use comptime_interpretor::{
    Budget, Error, ErrorKind, PreparedStructBuild, StructAdmissionSession, StructBuildRequest,
    StructPublicationError,
};
use nir::{DeclarationId, program as n};
use std::{cell::RefCell, sync::Arc};

/// A committed baseline plus immutable generated descriptions. Identities must
/// be reserved by the enclosing compiler world, including declarations omitted
/// from the baseline snapshot. This service never invents a package namespace.
///
/// Only concrete fields are supported: no body, default, clause, attribute or
/// implementation is silently converted into an already-checked description.
pub struct CheckedStructPublisher<'a> {
    baseline: &'a n::Package,
    published: RefCell<Vec<Arc<[n::Struct]>>>,
    diagnostic: RefCell<Option<String>>,
    admission: RefCell<Option<StructAdmissionSession<'a>>>,
    representations: RefCell<Option<crate::representations::IncrementalChecker<'a>>>,
}
impl<'a> CheckedStructPublisher<'a> {
    pub fn new(baseline: &'a n::Package) -> Self {
        Self {
            baseline,
            published: RefCell::new(Vec::new()),
            diagnostic: RefCell::new(None),
            admission: RefCell::new(None),
            representations: RefCell::new(None),
        }
    }
    pub fn published_definition_count(&self) -> usize {
        self.published.borrow().len()
    }
    pub fn last_diagnostic(&self) -> Option<String> {
        self.diagnostic.borrow().clone()
    }
    pub fn into_definitions(self) -> Vec<Arc<[n::Struct]>> {
        self.published.into_inner()
    }

    /// Prepare against the last committed world. Dropping the result does not
    /// publish; competing candidates must prepare again after another commits.
    pub fn prepare<'s>(
        &'s self,
        identity: DeclarationId,
        request: &StructBuildRequest,
        steps: &mut u64,
    ) -> Result<PreparedStructBuild<'s>, Error> {
        *self.diagnostic.borrow_mut() = None;
        let mut budget = Budget {
            steps: std::mem::take(steps),
            depth: 128,
        };
        let result = self.prepare_inner(identity, request, &mut budget);
        // A panic keeps the caller's allowance reserved. Ordinary errors retain
        // actual preparation/admission cost, never refunding rejected work.
        *steps = budget.steps;
        if let Err(error) = &result {
            if self.diagnostic.borrow().is_none() {
                *self.diagnostic.borrow_mut() =
                    Some(format!("checked struct publication failed: {error}"));
            }
        }
        result
    }

    fn prepare_inner<'s>(
        &'s self,
        identity: DeclarationId,
        request: &StructBuildRequest,
        budget: &mut Budget,
    ) -> Result<PreparedStructBuild<'s>, Error> {
        let published = self.published.borrow();
        let revision = published.len();
        let source = request.source;
        let (definitions, receipt, representation) = (|| {
            let mut admission = self.admission.borrow_mut();
            if admission.is_none() {
                *admission = Some(StructAdmissionSession::new(self.baseline, budget)?);
            }
            let admission = admission.as_mut().unwrap();
            // Preflight only the new description. Old field trees stay borrowed
            // or shared; the admission VM retains only committed descriptions.
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
            let receipt = admission.prepare(Arc::clone(&definitions), budget)?;
            // Type formation above is the same completed-Type admission used
            // for delivered HIR field types. Finite storage is an independent
            // obligation, shared with ordinary source declaration checking.
            let before = usize::try_from(budget.steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let checked = (|| {
                let mut representations = self.representations.borrow_mut();
                if representations.is_none() {
                    *representations = Some(crate::representations::IncrementalChecker::new(
                        self.baseline,
                        &mut fuel,
                    )?);
                }
                representations
                    .as_ref()
                    .unwrap()
                    .prepare(Arc::clone(&definitions), &mut fuel)
            })();
            charge(budget, before - fuel)?;
            let representation = checked.map_err(|message| {
                *self.diagnostic.borrow_mut() = Some(message.into());
                Error {
                    kind: ErrorKind::Type,
                    source: Some(source),
                }
            })?;
            Ok((definitions, receipt, representation))
        })()
        .map_err(|mut error: Error| {
            error.source = Some(source);
            error
        })?;
        drop(published);
        let committed = Arc::clone(&definitions);
        Ok(PreparedStructBuild {
            definitions,
            result: identity,
            commit: Box::new(move || {
                let mut published = self.published.borrow_mut();
                if published.len() != revision {
                    *self.diagnostic.borrow_mut() = Some("checked struct candidate is stale; prepare again against the committed world".into());
                    return Err(Error {
                        kind: ErrorKind::InvalidCallPlan,
                        source: Some(source),
                    });
                }
                let mut admission = self.admission.borrow_mut();
                match admission.as_mut().unwrap().commit_with(receipt, || {
                    self.representations
                        .borrow_mut()
                        .as_mut()
                        .unwrap()
                        .commit(representation)
                        .map_err(|message| {
                            *self.diagnostic.borrow_mut() = Some(message.into());
                            Error {
                                kind: ErrorKind::InvalidCallPlan,
                                source: Some(source),
                            }
                        })?;
                    published.push(committed);
                    Ok::<_, Error>(())
                }) {
                    Ok(()) => Ok(()),
                    Err(StructPublicationError::Admission(mut error)) => {
                        error.source = Some(source);
                        Err(error)
                    }
                    Err(StructPublicationError::Commit(error)) => Err(error),
                }
            }),
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
