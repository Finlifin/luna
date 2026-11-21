//! Transactional publication of completed, concrete nominal descriptions.
use comptime_interpretor::{
    Budget, DefinitionAdmissionSession, EnumPublicationError, Error, ErrorKind,
    NewtypePublicationError, NominalBatch, PreparedEnumAdmission, PreparedNewtypeAdmission,
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

impl PublishedDefinitions {
    /// Transfer completed builder products to the package returned by an
    /// invocation. Source checking has a separate transactional receiver.
    pub fn append_to(self, package: &mut Package) {
        use nir::normal_forms::NominalValueDeclaration as D;
        for batch in self.structs {
            for def in batch.iter() {
                package
                    .formed_nullary_nominals
                    .admit_checked(D::Struct(def));
            }
            package.structs.extend(batch.iter().cloned());
        }
        for batch in self.enums {
            for def in batch.iter() {
                package.formed_nullary_nominals.admit_checked(D::Enum(def));
            }
            package.enums.extend(batch.iter().cloned());
        }
        for batch in self.newtypes {
            for def in batch.iter() {
                package
                    .formed_nullary_nominals
                    .admit_checked(D::Newtype(def));
            }
            package.newtypes.extend(batch.iter().cloned());
        }
    }
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

enum Candidate {
    Structs(PreparedStructAdmission),
    Enums(PreparedEnumAdmission),
    Newtypes(PreparedNewtypeAdmission),
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
        self.prepare(NominalBatch::Structs(definitions), true, source, budget)
    }
    pub fn prepare_enums<'s>(
        &'s self,
        definitions: Arc<[Enum]>,
        source: SourceRange,
        budget: &mut Budget,
    ) -> Result<PreparedDefinitions<'s>, Error> {
        self.prepare(NominalBatch::Enums(definitions), true, source, budget)
    }
    pub fn prepare_newtypes<'s>(
        &'s self,
        definitions: Arc<[nir::newtypes::Newtype]>,
        source: SourceRange,
        budget: &mut Budget,
    ) -> Result<PreparedDefinitions<'s>, Error> {
        self.prepare(NominalBatch::Newtypes(definitions), true, source, budget)
    }
    /// Import into this invocation's admission and finite-storage worlds. The
    /// originating invocation already owns publication to the enclosing package.
    pub(super) fn import(
        &self,
        batch: &comptime_interpretor::NominalBatch,
        source: SourceRange,
        budget: &mut Budget,
    ) -> Result<(), Error> {
        self.prepare(batch.clone(), false, source, budget)?.commit()
    }

    fn prepare<'s>(
        &'s self,
        batch: NominalBatch,
        newly_created: bool,
        source: SourceRange,
        budget: &mut Budget,
    ) -> Result<PreparedDefinitions<'s>, Error> {
        self.set_diagnostic(None);
        let result = (|| {
            let count = batch.len();
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
                match &batch {
                    NominalBatch::Structs(s) => {
                        Candidate::Structs(admission.prepare(s.clone(), budget)?)
                    }
                    NominalBatch::Enums(e) => {
                        Candidate::Enums(admission.prepare_enums(e.clone(), budget)?)
                    }
                    NominalBatch::Newtypes(n) => {
                        Candidate::Newtypes(admission.prepare_newtypes(n.clone(), budget)?)
                    }
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
                let (structs, enums, newtypes) = match &batch {
                    NominalBatch::Structs(s) => (s.clone(), Arc::from([]), Arc::from([])),
                    NominalBatch::Enums(e) => (Arc::from([]), e.clone(), Arc::from([])),
                    NominalBatch::Newtypes(n) => (Arc::from([]), Arc::from([]), n.clone()),
                };
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
                    let finish = || {
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
                        if newly_created {
                            match batch {
                                NominalBatch::Structs(s) => published.structs.push(s),
                                NominalBatch::Enums(e) => published.enums.push(e),
                                NominalBatch::Newtypes(n) => published.newtypes.push(n),
                            }
                        }
                        Ok::<_, Error>(())
                    };
                    let admission = admission.as_mut().unwrap();
                    let result = match candidate {
                        Candidate::Structs(receipt) => admission
                            .commit_with(receipt, finish)
                            .map_err(|error| match error {
                                StructPublicationError::Admission(error)
                                | StructPublicationError::Commit(error) => error,
                            }),
                        Candidate::Newtypes(receipt) => admission
                            .commit_newtypes_with(receipt, finish)
                            .map_err(|error| match error {
                                NewtypePublicationError::Admission(error)
                                | NewtypePublicationError::Commit(error) => error,
                            }),
                        Candidate::Enums(receipt) => admission
                            .commit_enums_with(receipt, finish)
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
