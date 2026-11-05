//! Transactional publication of completed, concrete nominal descriptions.
use comptime_interpretor::{
    Budget, DefinitionAdmissionSession, EnumPublicationError, Error, ErrorKind,
    NewtypePublicationError, PreparedEnumAdmission, PreparedNewtypeAdmission,
    PreparedStructAdmission, StructPublicationError,
};
use nir::program::{Enum, Package, SourceRange, Struct};
use std::{cell::RefCell, fmt, sync::Arc};

/// Retained additions, in publication order. Identities are reserved by the host.
#[derive(Default)]
pub struct PublishedDefinitions {
    pub structs: Vec<Arc<[Struct]>>,
    pub enums: Vec<Arc<[Enum]>>,
    pub newtypes: Vec<Arc<[nir::newtypes::Newtype]>>,
}

/// One immutable candidate. Its descriptors cannot be replaced after checking.
/// Dropping it abandons publication; commit performs only prepaid installation.
pub struct PreparedDefinitions<'a> {
    commit: Box<dyn FnOnce() -> Result<(), Error> + 'a>,
    count: usize,
}
impl PreparedDefinitions<'_> {
    pub fn commit(self) -> Result<(), Error> {
        (self.commit)()
    }
}
impl fmt::Display for PreparedDefinitions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(prepared-definitions (count {}))", self.count)
    }
}

enum Kind {
    Struct,
    Enum,
    Newtype,
}

enum Candidate {
    Structs(Arc<[Struct]>, PreparedStructAdmission),
    Enums(Arc<[Enum]>, PreparedEnumAdmission),
    Newtypes(Arc<[nir::newtypes::Newtype]>, PreparedNewtypeAdmission),
}

/// Combines formation admission and finite-storage proofs in one committed world.
/// Accepts concrete, body-free descriptors only; this is not source elaboration.
/// The host must reserve identities, including those outside the baseline.
pub struct CheckedDefinitionPublisher<'a> {
    baseline: &'a Package,
    type_solver: Option<&'a comptime_interpretor::BudgetedTypeSolver<'a>>,
    published: RefCell<PublishedDefinitions>,
    diagnostic: RefCell<Option<String>>,
    admission: RefCell<Option<DefinitionAdmissionSession<'a>>>,
    representations: RefCell<Option<crate::representations::IncrementalChecker<'a>>>,
}
impl<'a> CheckedDefinitionPublisher<'a> {
    pub fn new(baseline: &'a Package) -> Self {
        Self {
            baseline,
            type_solver: None,
            published: RefCell::new(PublishedDefinitions::default()),
            diagnostic: RefCell::new(None),
            admission: RefCell::new(None),
            representations: RefCell::new(None),
        }
    }
    pub fn new_with_type_solver(
        baseline: &'a Package,
        solver: &'a comptime_interpretor::BudgetedTypeSolver<'a>,
    ) -> Self {
        Self {
            type_solver: Some(solver),
            ..Self::new(baseline)
        }
    }
    pub fn published_definition_count(&self) -> usize {
        let published = self.published.borrow();
        published
            .structs
            .iter()
            .map(|batch| batch.len())
            .sum::<usize>()
            + published
                .enums
                .iter()
                .map(|batch| batch.len())
                .sum::<usize>()
            + published
                .newtypes
                .iter()
                .map(|batch| batch.len())
                .sum::<usize>()
    }
    pub fn last_diagnostic(&self) -> Option<String> {
        self.diagnostic.borrow().clone()
    }
    pub fn into_definitions(self) -> PublishedDefinitions {
        self.published.into_inner()
    }
    pub(super) fn set_diagnostic(&self, diagnostic: Option<String>) {
        *self.diagnostic.borrow_mut() = diagnostic;
    }
    pub fn prepare_structs<'s>(
        &'s self,
        definitions: Arc<[Struct]>,
        source: SourceRange,
        budget: &mut Budget,
    ) -> Result<PreparedDefinitions<'s>, Error> {
        self.prepare(
            definitions,
            Arc::from([]),
            Arc::from([]),
            Kind::Struct,
            source,
            budget,
        )
    }
    pub fn prepare_enums<'s>(
        &'s self,
        definitions: Arc<[Enum]>,
        source: SourceRange,
        budget: &mut Budget,
    ) -> Result<PreparedDefinitions<'s>, Error> {
        self.prepare(
            Arc::from([]),
            definitions,
            Arc::from([]),
            Kind::Enum,
            source,
            budget,
        )
    }
    pub fn prepare_newtypes<'s>(
        &'s self,
        definitions: Arc<[nir::newtypes::Newtype]>,
        source: SourceRange,
        budget: &mut Budget,
    ) -> Result<PreparedDefinitions<'s>, Error> {
        self.prepare(
            Arc::from([]),
            Arc::from([]),
            definitions,
            Kind::Newtype,
            source,
            budget,
        )
    }
    fn prepare<'s>(
        &'s self,
        structs: Arc<[Struct]>,
        enums: Arc<[Enum]>,
        newtypes: Arc<[nir::newtypes::Newtype]>,
        kind: Kind,
        source: SourceRange,
        budget: &mut Budget,
    ) -> Result<PreparedDefinitions<'s>, Error> {
        self.set_diagnostic(None);
        let result = (|| {
            let count = structs.len() + enums.len() + newtypes.len();
            let candidate = {
                let mut admission = self.admission.borrow_mut();
                if admission.is_none() {
                    *admission = Some(match self.type_solver {
                        Some(solver) => DefinitionAdmissionSession::new_with_type_solver(
                            self.baseline,
                            solver,
                            budget,
                        )?,
                        None => DefinitionAdmissionSession::new(self.baseline, budget)?,
                    });
                }
                let admission = admission.as_mut().unwrap();
                match kind {
                    Kind::Enum => Candidate::Enums(
                        Arc::clone(&enums),
                        admission.prepare_enums(Arc::clone(&enums), budget)?,
                    ),
                    Kind::Struct => Candidate::Structs(
                        Arc::clone(&structs),
                        admission.prepare(Arc::clone(&structs), budget)?,
                    ),
                    Kind::Newtype => Candidate::Newtypes(
                        Arc::clone(&newtypes),
                        admission.prepare_newtypes(Arc::clone(&newtypes), budget)?,
                    ),
                }
            };
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
                    .prepare_nominals(structs, enums, newtypes, &mut fuel)
            })();
            budget.steps -= (before - fuel) as u64;
            let representation = checked.map_err(|failure| {
                self.set_diagnostic(Some(failure.to_string()));
                Error {
                    kind: match failure.kind {
                        crate::representations::FailureKind::Budget => ErrorKind::StepsExhausted,
                        crate::representations::FailureKind::Invalid => ErrorKind::Type,
                        crate::representations::FailureKind::Stale => ErrorKind::InvalidCallPlan,
                    },
                    source: Some(source),
                }
            })?;
            Ok(PreparedDefinitions {
                count,
                commit: Box::new(move || {
                    // Obtain all mutable guards before either world can change.
                    let mut published = self.published.borrow_mut();
                    let mut representations = self.representations.borrow_mut();
                    let mut admission = self.admission.borrow_mut();
                    let finish =
                        |published: &mut PublishedDefinitions, structs, enums, newtypes| {
                            representations
                                .as_mut()
                                .unwrap()
                                .commit(representation)
                                .map_err(|failure| {
                                    self.set_diagnostic(Some(failure.to_string()));
                                    Error {
                                        kind: ErrorKind::InvalidCallPlan,
                                        source: Some(source),
                                    }
                                })?;
                            if let Some(batch) = structs {
                                published.structs.push(batch);
                            }
                            if let Some(batch) = enums {
                                published.enums.push(batch);
                            }
                            if let Some(batch) = newtypes {
                                published.newtypes.push(batch);
                            }
                            Ok::<_, Error>(())
                        };
                    let result = match candidate {
                        Candidate::Structs(definitions, receipt) => admission
                            .as_mut()
                            .unwrap()
                            .commit_with(receipt, || {
                                finish(&mut published, Some(definitions), None, None)
                            })
                            .map_err(|error| match error {
                                StructPublicationError::Admission(error)
                                | StructPublicationError::Commit(error) => error,
                            }),
                        Candidate::Newtypes(definitions, receipt) => admission
                            .as_mut()
                            .unwrap()
                            .commit_newtypes_with(receipt, || {
                                finish(&mut published, None, None, Some(definitions))
                            })
                            .map_err(|error| match error {
                                NewtypePublicationError::Admission(error)
                                | NewtypePublicationError::Commit(error) => error,
                            }),
                        Candidate::Enums(definitions, receipt) => admission
                            .as_mut()
                            .unwrap()
                            .commit_enums_with(receipt, || {
                                finish(&mut published, None, Some(definitions), None)
                            })
                            .map_err(|error| match error {
                                EnumPublicationError::Admission(error)
                                | EnumPublicationError::Commit(error) => error,
                            }),
                    };
                    result.map_err(|mut error| {
                        error.source = Some(source);
                        if error.kind == ErrorKind::InvalidCallPlan {
                            self.set_diagnostic(Some("checked definition candidate is stale; prepare again against the committed world".into()));
                        }
                        error
                    })
                }),
            })
        })();
        result.map_err(|mut error: Error| {
            error.source = Some(source);
            if self.last_diagnostic().is_none() {
                self.set_diagnostic(Some(format!(
                    "checked definition publication failed: {error}"
                )));
            }
            error
        })
    }
}
impl fmt::Display for CheckedDefinitionPublisher<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(checked-definition-publisher (definitions {}))",
            self.published_definition_count()
        )
    }
}
