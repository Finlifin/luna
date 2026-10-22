//! Candidate-local inference state. This is not the PTC solver or a rollback
//! mechanism for comptime side effects. Only committed stable terms cross into
//! NIR; callers must separately settle obligations and check scope/contracts.
use nir::{Sort, Term, TypeError};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
fn unique() -> u64 {
    NEXT_ID
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .expect("inference identity space exhausted")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MetaId(u64);

/// A local computation of a symbolic term, not an executable comptime call.
/// Instantiate removes one bound frame using NIR's capture-avoiding operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InferenceTerm {
    Stable(Term),
    Meta(MetaId),
    Instantiate {
        template: Term,
        arguments: Vec<InferenceTerm>,
    },
}

#[derive(Clone)]
struct Variable {
    sort: Sort,
    value: Option<InferenceTerm>,
}

pub struct InferenceContext {
    identity: u64,
    revision: u64,
    parent: Option<(u64, u64)>,
    variables: BTreeMap<MetaId, Variable>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum InferenceError {
    ForeignVariable(MetaId),
    Unresolved(MetaId),
    AlreadyAssigned(MetaId),
    Occurs(MetaId),
    WrongSort { expected: Sort, actual: Sort },
    InvalidSubstitution(TypeError),
    StaleCandidate,
    Budget,
}

impl Default for InferenceContext {
    fn default() -> Self {
        Self {
            identity: unique(),
            revision: 0,
            parent: None,
            variables: BTreeMap::new(),
        }
    }
}

impl InferenceContext {
    pub fn fresh(&mut self, sort: Sort) -> MetaId {
        let id = MetaId(unique());
        self.variables.insert(id, Variable { sort, value: None });
        self.changed();
        id
    }

    fn changed(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("inference revision exhausted");
    }

    /// Private copy of the entire inference store. Other candidate-owned state
    /// (obligations, diagnostics, facts) must join the transaction before a
    /// higher-level candidate can be considered committed.
    pub fn trial(&self) -> Self {
        Self {
            identity: unique(),
            revision: 0,
            parent: Some((self.identity, self.revision)),
            variables: self.variables.clone(),
        }
    }

    pub fn commit(&mut self, candidate: Self) -> Result<(), InferenceError> {
        if candidate.parent != Some((self.identity, self.revision)) {
            return Err(InferenceError::StaleCandidate);
        }
        self.variables = candidate.variables;
        self.changed();
        Ok(())
    }

    pub fn assign(&mut self, id: MetaId, value: InferenceTerm) -> Result<(), InferenceError> {
        let variable = self
            .variables
            .get(&id)
            .ok_or(InferenceError::ForeignVariable(id))?;
        if variable.value.is_some() {
            return Err(InferenceError::AlreadyAssigned(id));
        }
        let expected = variable.sort;
        self.check_occurs(id, &value)?;
        let actual = self.sort(&value)?;
        if expected != actual {
            return Err(InferenceError::WrongSort { expected, actual });
        }
        self.variables.get_mut(&id).unwrap().value = Some(value);
        self.changed();
        Ok(())
    }

    pub fn sort(&self, term: &InferenceTerm) -> Result<Sort, InferenceError> {
        match term {
            InferenceTerm::Stable(term) => Ok(term.sort()),
            InferenceTerm::Meta(id) => self
                .variables
                .get(id)
                .map(|v| v.sort)
                .ok_or(InferenceError::ForeignVariable(*id)),
            InferenceTerm::Instantiate { template, .. } => Ok(template.sort()),
        }
    }

    /// Actual unsolved leaves, following candidate-local aliases. Waiters must
    /// recompute this after an assignment because a leaf may become an alias.
    pub fn unresolved(&self, term: &InferenceTerm) -> Result<BTreeSet<MetaId>, InferenceError> {
        let mut pending = vec![term];
        let mut seen = BTreeSet::new();
        let mut result = BTreeSet::new();
        while let Some(term) = pending.pop() {
            match term {
                InferenceTerm::Stable(_) => {}
                InferenceTerm::Instantiate {
                    template,
                    arguments,
                } => {
                    for index in template
                        .substitution_parameters()
                        .map_err(InferenceError::InvalidSubstitution)?
                        .keys()
                    {
                        pending.push(arguments.get(*index).ok_or(
                            InferenceError::InvalidSubstitution(TypeError::MissingArgument(*index)),
                        )?);
                    }
                }
                InferenceTerm::Meta(id) if seen.insert(*id) => {
                    let variable = self
                        .variables
                        .get(id)
                        .ok_or(InferenceError::ForeignVariable(*id))?;
                    match &variable.value {
                        Some(value) => pending.push(value),
                        None => {
                            result.insert(*id);
                        }
                    }
                }
                InferenceTerm::Meta(_) => {}
            }
        }
        Ok(result)
    }

    /// Expose an assigned alias or a fully settled instantiation without
    /// interpreting normal-form heads or assigning rigid parameters.
    pub fn resolve(&self, term: &InferenceTerm) -> Result<InferenceTerm, InferenceError> {
        let mut term = term;
        loop {
            match term {
                InferenceTerm::Meta(id) => {
                    let variable = self
                        .variables
                        .get(id)
                        .ok_or(InferenceError::ForeignVariable(*id))?;
                    match &variable.value {
                        Some(value) => term = value,
                        None => return Ok(term.clone()),
                    }
                }
                InferenceTerm::Instantiate {
                    template:
                        Term::Bound {
                            depth: 0,
                            index,
                            sort,
                        },
                    arguments,
                } => {
                    let value =
                        arguments
                            .get(*index)
                            .ok_or(InferenceError::InvalidSubstitution(
                                TypeError::MissingArgument(*index),
                            ))?;
                    let actual = self.sort(value)?;
                    if actual != *sort {
                        return Err(InferenceError::WrongSort {
                            expected: *sort,
                            actual,
                        });
                    }
                    term = value;
                }
                _ => break,
            }
        }
        match self.freeze(term) {
            Ok(stable) => Ok(InferenceTerm::Stable(stable)),
            Err(InferenceError::Unresolved(_)) => Ok(term.clone()),
            Err(error) => Err(error),
        }
    }

    fn check_occurs(&self, target: MetaId, term: &InferenceTerm) -> Result<(), InferenceError> {
        let mut pending = vec![term];
        let mut seen = BTreeSet::new();
        while let Some(term) = pending.pop() {
            match term {
                InferenceTerm::Stable(_) => {}
                InferenceTerm::Meta(id) => {
                    if *id == target {
                        return Err(InferenceError::Occurs(target));
                    }
                    let variable = self
                        .variables
                        .get(id)
                        .ok_or(InferenceError::ForeignVariable(*id))?;
                    if seen.insert(*id) {
                        if let Some(value) = &variable.value {
                            pending.push(value);
                        }
                    }
                }
                InferenceTerm::Instantiate {
                    template,
                    arguments,
                } => {
                    for (index, expected) in template
                        .substitution_parameters()
                        .map_err(InferenceError::InvalidSubstitution)?
                    {
                        let argument =
                            arguments
                                .get(index)
                                .ok_or(InferenceError::InvalidSubstitution(
                                    TypeError::MissingArgument(index),
                                ))?;
                        let actual = self.sort(argument)?;
                        if actual != expected {
                            return Err(InferenceError::WrongSort { expected, actual });
                        }
                        pending.push(argument);
                    }
                }
            }
        }
        Ok(())
    }

    /// Export one settled term. This deliberately does not claim the entire
    /// definition is frozen: required obligations still belong to its region.
    pub fn freeze(&self, term: &InferenceTerm) -> Result<Term, InferenceError> {
        self.freeze_cached(term, &mut BTreeMap::new())
    }

    fn freeze_cached(
        &self,
        term: &InferenceTerm,
        cache: &mut BTreeMap<MetaId, Term>,
    ) -> Result<Term, InferenceError> {
        // Explicit postorder evaluation avoids Rust stack growth along long
        // candidate alias chains. Only semantically referenced slots are read.
        enum Work<'a> {
            Visit(&'a InferenceTerm),
            Cache(MetaId),
            Substitute(&'a Term, usize, Vec<usize>),
        }
        let mut work = vec![Work::Visit(term)];
        let mut values = Vec::new();
        while let Some(action) = work.pop() {
            match action {
                Work::Visit(InferenceTerm::Stable(term)) => values.push(term.clone()),
                Work::Visit(InferenceTerm::Meta(id)) => {
                    if let Some(value) = cache.get(id) {
                        values.push(value.clone());
                        continue;
                    }
                    let variable = self
                        .variables
                        .get(id)
                        .ok_or(InferenceError::ForeignVariable(*id))?;
                    let value = variable
                        .value
                        .as_ref()
                        .ok_or(InferenceError::Unresolved(*id))?;
                    work.push(Work::Cache(*id));
                    work.push(Work::Visit(value));
                }
                Work::Visit(InferenceTerm::Instantiate {
                    template,
                    arguments,
                }) => {
                    let indices = template
                        .substitution_parameters()
                        .map_err(InferenceError::InvalidSubstitution)?
                        .into_keys()
                        .collect::<Vec<_>>();
                    work.push(Work::Substitute(template, arguments.len(), indices.clone()));
                    for index in indices.into_iter().rev() {
                        let argument =
                            arguments
                                .get(index)
                                .ok_or(InferenceError::InvalidSubstitution(
                                    TypeError::MissingArgument(index),
                                ))?;
                        work.push(Work::Visit(argument));
                    }
                }
                Work::Cache(id) => {
                    cache.insert(id, values.last().expect("visited value").clone());
                }
                Work::Substitute(template, arity, indices) => {
                    let mut arguments = vec![None; arity];
                    for index in indices.into_iter().rev() {
                        arguments[index] =
                            Some(values.pop().expect("visited substitution argument"));
                    }
                    values.push(
                        template
                            .substitute_partial(&arguments)
                            .map_err(InferenceError::InvalidSubstitution)?,
                    );
                }
            }
        }
        debug_assert_eq!(values.len(), 1);
        Ok(values.pop().expect("visited root"))
    }
}

impl fmt::Display for MetaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(meta {})", self.0)
    }
}
impl fmt::Display for InferenceTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stable(term) => write!(f, "(stable {term})"),
            Self::Meta(id) => id.fmt(f),
            Self::Instantiate {
                template,
                arguments,
            } => {
                write!(f, "(instantiate (template {template}) (arguments")?;
                for argument in arguments {
                    write!(f, " {argument}")?;
                }
                f.write_str("))")
            }
        }
    }
}
impl fmt::Display for InferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ForeignVariable(id) => write!(f, "unknown inference variable {id}"),
            Self::Unresolved(id) => write!(f, "unresolved inference variable {id}"),
            Self::AlreadyAssigned(id) => write!(f, "inference variable {id} already assigned"),
            Self::Occurs(id) => write!(f, "cyclic assignment of {id}"),
            Self::WrongSort { expected, actual } => write!(f, "expected {expected}, got {actual}"),
            Self::InvalidSubstitution(error) => error.fmt(f),
            Self::Budget => f.write_str("inference structure budget exhausted"),
            Self::StaleCandidate => {
                f.write_str("candidate does not belong to the current parent revision")
            }
        }
    }
}
impl std::error::Error for InferenceError {}

pub mod associated;
pub mod capabilities;
pub mod materialization;
pub mod program;

pub mod calls;
pub mod implementation;
pub mod matching;
pub mod obligations;
pub mod ptc;

mod coverage;

mod bit_coverage;
mod float_coverage;
mod interval_coverage;
mod machine_float_coverage;
mod symbol_coverage;

/// Program-state arithmetic reasoning, separate from the type proposition engine.
pub mod verification;

pub mod effects;

pub mod splice;

pub mod key_protocols;

pub mod arithmetic;
pub mod gradual;
pub mod type_computations;

mod inference_budget;

pub mod indexing;
pub mod list_patterns;

pub mod literal_conversion;
