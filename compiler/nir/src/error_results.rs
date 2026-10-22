//! Checked closed error-row layouts and branch remapping, independent of effect
//! continuations. These are representation plans, not executable NIR values yet.
//! Type identity evidence comes from elaboration: structural inequality alone
//! must never prove that two symbolic error labels are distinct.
use crate::{Sort, Term};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identity {
    Same,
    Distinct,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Budget,
    InvalidType,
    InvalidParameters,
    UnresolvedIdentity,
    MissingLabel,
    InvalidTag,
    IncompatibleSuccess,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::Budget => "budget-exhausted",
            Self::InvalidType => "invalid-type",
            Self::InvalidParameters => "invalid-parameters",
            Self::UnresolvedIdentity => "unresolved-label-identity",
            Self::MissingLabel => "missing-label",
            Self::InvalidTag => "invalid-tag",
            Self::IncompatibleSuccess => "incompatible-success-type",
        };
        write!(f, "(error-result-failure (kind {kind}))")
    }
}
impl std::error::Error for Error {}

fn charge(ty: &Term, fuel: &mut usize) -> Result<(), Error> {
    crate::type_computations::charge_term(ty, fuel).map_err(|_| Error::Budget)
}
fn type_input(ty: &Term, fuel: &mut usize) -> Result<(), Error> {
    charge(ty, fuel)?;
    if ty.sort() != Sort::Type || ty.check_scope(&[]).is_err() {
        return Err(Error::InvalidType);
    }
    Ok(())
}
fn same(
    left: &Term,
    right: &Term,
    fuel: &mut usize,
    compare: &mut impl FnMut(&Term, &Term, &mut usize) -> Result<Identity, Error>,
) -> Result<bool, Error> {
    charge(left, fuel)?;
    charge(right, fuel)?;
    if left == right {
        return Ok(true);
    }
    match compare(left, right, fuel)? {
        Identity::Same => Ok(true),
        Identity::Distinct => Ok(false),
        Identity::Unknown => Err(Error::UnresolvedIdentity),
    }
}

/// Unique labels in deterministic first-occurrence order. Order is layout,
/// not row identity; consumers must use checked remapping between layouts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    labels: Vec<Term>,
}
impl Row {
    pub fn new(
        labels: Vec<Term>,
        fuel: &mut usize,
        compare: &mut impl FnMut(&Term, &Term, &mut usize) -> Result<Identity, Error>,
    ) -> Result<Self, Error> {
        let mut row = Self { labels: Vec::new() };
        for label in labels {
            type_input(&label, fuel)?;
            if row.find(&label, fuel, compare)?.is_none() {
                row.labels.push(label);
            }
        }
        Ok(row)
    }
    pub fn labels(&self) -> &[Term] {
        &self.labels
    }
    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }
    fn find(
        &self,
        label: &Term,
        fuel: &mut usize,
        compare: &mut impl FnMut(&Term, &Term, &mut usize) -> Result<Identity, Error>,
    ) -> Result<Option<usize>, Error> {
        // Uniqueness is already established for this row. An exact member
        // does not need new distinctness proofs against preceding labels.
        charge(label, fuel)?;
        for (index, candidate) in self.labels.iter().enumerate() {
            charge(candidate, fuel)?;
            if candidate == label {
                return Ok(Some(index));
            }
        }
        for (index, candidate) in self.labels.iter().enumerate() {
            if same(candidate, label, fuel, compare)? {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }
    /// Mapping from each source error tag to its destination tag. Payloads
    /// retain their full type identity; this does not inject a success value.
    pub fn embedding_into(
        &self,
        target: &Self,
        fuel: &mut usize,
        compare: &mut impl FnMut(&Term, &Term, &mut usize) -> Result<Identity, Error>,
    ) -> Result<Embedding, Error> {
        let mut tags = Vec::new();
        for label in &self.labels {
            tags.push(
                target
                    .find(label, fuel, compare)?
                    .ok_or(Error::MissingLabel)?,
            );
        }
        Ok(Embedding { tags })
    }
    /// Remove handled labels from the input row. The output row of handlers
    /// must be constructed separately and may contain these same labels again.
    pub fn subtract(
        &self,
        handled: &Self,
        fuel: &mut usize,
        compare: &mut impl FnMut(&Term, &Term, &mut usize) -> Result<Identity, Error>,
    ) -> Result<Residual, Error> {
        handled.embedding_into(self, fuel, compare)?;
        let mut labels = Vec::new();
        let mut tags = Vec::new();
        for label in &self.labels {
            if handled.find(label, fuel, compare)?.is_some() {
                tags.push(None);
            } else {
                charge(label, fuel)?;
                tags.push(Some(labels.len()));
                labels.push(label.clone());
            }
        }
        Ok(Residual {
            row: Self { labels },
            tags,
        })
    }
}
impl fmt::Display for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(error-row (labels")?;
        for label in &self.labels {
            write!(f, " {label}")?;
        }
        f.write_str("))")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Embedding {
    tags: Vec<usize>,
}
impl Embedding {
    pub fn map_error(&self, tag: usize) -> Result<usize, Error> {
        self.tags.get(tag).copied().ok_or(Error::InvalidTag)
    }
}
impl fmt::Display for Embedding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(error-embedding (tags")?;
        for tag in &self.tags {
            write!(f, " {tag}")?;
        }
        f.write_str("))")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Residual {
    row: Row,
    tags: Vec<Option<usize>>,
}
impl Residual {
    pub fn row(&self) -> &Row {
        &self.row
    }
    /// None denotes a handled input, not an absent or silently dropped error.
    pub fn map_error(&self, tag: usize) -> Result<Option<usize>, Error> {
        self.tags.get(tag).copied().ok_or(Error::InvalidTag)
    }
}
impl fmt::Display for Residual {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(error-residual {} (tags", self.row)?;
        for tag in &self.tags {
            match tag {
                Some(tag) => write!(f, " (remaining {tag})")?,
                None => f.write_str(" (handled)")?,
            }
        }
        f.write_str("))")
    }
}

/// Closed result representation. Empty qualification retains the plain success representation; nested
/// qualifications merge rows without inspecting or unfolding the success head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultType {
    errors: Row,
    success: Term,
}
impl ResultType {
    /// Admit a type term into a closed layout using semantic label evidence.
    pub fn from_term(
        term: &Term,
        fuel: &mut usize,
        compare: &mut impl FnMut(&Term, &Term, &mut usize) -> Result<Identity, Error>,
    ) -> Result<Self, Error> {
        type_input(term, fuel)?;
        let mut labels = Vec::new();
        let mut success = term;
        while let Term::ErrorResult { errors, result } = success {
            for error in errors {
                charge(error, fuel)?;
                labels.push(error.clone());
            }
            success = result;
        }
        charge(success, fuel)?;
        Ok(Self {
            errors: Row::new(labels, fuel, compare)?,
            success: success.clone(),
        })
    }
    pub fn to_term(&self, fuel: &mut usize) -> Result<Term, Error> {
        charge(&self.success, fuel)?;
        for error in &self.errors.labels {
            charge(error, fuel)?;
        }
        Ok(Term::error_result(
            self.errors.labels.clone(),
            self.success.clone(),
        ))
    }
    pub fn plain(success: Term, fuel: &mut usize) -> Result<Self, Error> {
        type_input(&success, fuel)?;
        if matches!(success, Term::ErrorResult { .. }) {
            return Err(Error::InvalidType);
        }
        Ok(Self {
            errors: Row { labels: Vec::new() },
            success,
        })
    }
    pub fn qualify(
        errors: Row,
        inner: Self,
        fuel: &mut usize,
        compare: &mut impl FnMut(&Term, &Term, &mut usize) -> Result<Identity, Error>,
    ) -> Result<Self, Error> {
        let mut labels = errors.labels;
        labels.extend(inner.errors.labels);
        let errors = Row::new(labels, fuel, compare)?;
        Ok(Self {
            errors,
            success: inner.success,
        })
    }
    /// A plain success injection and a qualified-result embedding have
    /// different runtime operations, even when both are accepted at a call.
    pub fn transport_into(
        &self,
        target: &Self,
        fuel: &mut usize,
        compare: &mut impl FnMut(&Term, &Term, &mut usize) -> Result<Identity, Error>,
    ) -> Result<Transport, Error> {
        self.check_success_transport(target, fuel, compare)?;
        if self.is_plain() {
            Ok(if target.is_plain() {
                Transport::Plain
            } else {
                Transport::InjectSuccess
            })
        } else {
            Ok(Transport::Embed(self.errors.embedding_into(
                &target.errors,
                fuel,
                compare,
            )?))
        }
    }
    pub fn errors(&self) -> &Row {
        &self.errors
    }
    pub fn is_plain(&self) -> bool {
        self.errors.is_empty()
    }
    pub fn success(&self) -> &Term {
        &self.success
    }
    /// Success transport requires its own type proof; row embedding alone
    /// never authorizes reinterpreting a different success payload.
    pub fn check_success_transport(
        &self,
        target: &Self,
        fuel: &mut usize,
        compare: &mut impl FnMut(&Term, &Term, &mut usize) -> Result<Identity, Error>,
    ) -> Result<(), Error> {
        if same(self.success(), target.success(), fuel, compare)? {
            Ok(())
        } else {
            Err(Error::IncompatibleSuccess)
        }
    }
}
impl fmt::Display for ResultType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_plain() {
            write!(f, "(plain-result {})", self.success)
        } else {
            write!(
                f,
                "(error-result {} (success {}))",
                self.errors, self.success
            )
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transport {
    Plain,
    InjectSuccess,
    Embed(Embedding),
}
impl fmt::Display for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plain => f.write_str("(result-transport plain)"),
            Self::InjectSuccess => f.write_str("(result-transport inject-success)"),
            Self::Embed(plan) => write!(f, "(result-transport (embed {plan}))"),
        }
    }
}

/// A declaration-side row is not a layout: labels may become identical after
/// mixed-sort substitution. No error tags are assigned until instantiation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultTemplate {
    parameters: Vec<crate::DeclarationParameter>,
    labels: Vec<Term>,
    success: Term,
}
impl ResultTemplate {
    pub fn new(
        parameters: Vec<crate::DeclarationParameter>,
        labels: Vec<Term>,
        success: Term,
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        *fuel = fuel.checked_sub(parameters.len()).ok_or(Error::Budget)?;
        let arguments: Vec<_> = parameters
            .iter()
            .map(|parameter| Term::Rigid {
                declaration: parameter.declaration,
                sort: parameter.sort,
            })
            .collect();
        // This checks the entire frame, even if no type references a parameter.
        crate::type_computations::instantiate_parameters_bounded(
            &success,
            &parameters,
            &arguments,
            &[],
            fuel,
        )
        .map_err(substitution_error)?;
        let frame: Vec<_> = parameters.iter().map(|parameter| parameter.sort).collect();
        for ty in labels.iter().chain(std::iter::once(&success)) {
            charge(ty, fuel)?;
            *fuel = fuel.checked_sub(parameters.len()).ok_or(Error::Budget)?;
            let abstracted = ty
                .abstract_parameters(&parameters)
                .map_err(|_| Error::InvalidParameters)?;
            if abstracted.sort() != Sort::Type
                || abstracted
                    .check_scope(std::slice::from_ref(&frame))
                    .is_err()
            {
                return Err(Error::InvalidType);
            }
            if crate::type_computations::has_rigid_parameters_bounded(&abstracted, fuel)
                .map_err(substitution_error)?
            {
                return Err(Error::InvalidParameters);
            }
        }
        Ok(Self {
            parameters,
            labels,
            success,
        })
    }
    pub fn instantiate(
        &self,
        arguments: &[Term],
        fuel: &mut usize,
        compare: &mut impl FnMut(&Term, &Term, &mut usize) -> Result<Identity, Error>,
    ) -> Result<InstantiatedResult, Error> {
        let success = crate::type_computations::instantiate_parameters_bounded(
            &self.success,
            &self.parameters,
            arguments,
            &[],
            fuel,
        )
        .map_err(substitution_error)?;
        let inner = ResultType::from_term(&success, fuel, compare)?;
        let mut labels = Vec::new();
        for label in &self.labels {
            labels.push(
                crate::type_computations::instantiate_parameters_bounded(
                    label,
                    &self.parameters,
                    arguments,
                    &[],
                    fuel,
                )
                .map_err(substitution_error)?,
            );
        }
        // Keep occurrence identities until normalization yields the mapping.
        for label in &labels {
            charge(label, fuel)?;
        }
        let row = Row::new(labels.clone(), fuel, compare)?;
        let mut tags = Vec::new();
        for label in &labels {
            tags.push(row.find(label, fuel, compare)?.ok_or(Error::MissingLabel)?);
        }
        Ok(InstantiatedResult {
            result: ResultType::qualify(row, inner, fuel, compare)?,
            occurrences: Embedding { tags },
        })
    }
}
fn substitution_error(error: crate::type_computations::Error) -> Error {
    match error {
        crate::type_computations::Error::Budget => Error::Budget,
        _ => Error::InvalidParameters,
    }
}
impl fmt::Display for ResultTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(error-result-template (parameters")?;
        for parameter in &self.parameters {
            write!(
                f,
                " (parameter (declaration {}) (sort {}))",
                parameter.declaration, parameter.sort
            )?;
        }
        f.write_str(") (labels")?;
        for label in &self.labels {
            write!(f, " {label}")?;
        }
        write!(f, ") (success {}))", self.success)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstantiatedResult {
    result: ResultType,
    occurrences: Embedding,
}
impl InstantiatedResult {
    pub fn result(&self) -> &ResultType {
        &self.result
    }
    /// Translate a source row occurrence, including duplicate occurrences,
    /// into the instantiated error tag. This is not a runtime payload cast.
    pub fn occurrence_tag(&self, occurrence: usize) -> Result<usize, Error> {
        self.occurrences.map_error(occurrence)
    }
}
impl fmt::Display for InstantiatedResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(instantiated-error-result {} (occurrences {}))",
            self.result, self.occurrences
        )
    }
}
