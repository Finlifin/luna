//! Candidate-owned type obligations. Waiting for inference information is
//! distinct from a stable PTC Unknown; neither is a successful obligation.
use crate::ptc::{self, Budget, Formula, Outcome, TypeKind};
use crate::{InferenceContext, InferenceError, InferenceTerm, MetaId, fresh_identity};
use nir::{DeclarationId, Sort, Term};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

fn scope_frames(scope: &[(Sort, Option<Term>)]) -> Vec<Vec<Sort>> {
    if scope.is_empty() {
        vec![]
    } else {
        vec![scope.iter().map(|p| p.0).collect()]
    }
}
fn scoped_equality(
    a: Term,
    b: Term,
    scope: &[(Sort, Option<Term>)],
    native_scope: bool,
) -> Result<Formula, InferenceError> {
    if native_scope {
        let frames = scope_frames(scope);
        a.check_scope(&frames)
            .map_err(InferenceError::InvalidSubstitution)?;
        b.check_scope(&frames)
            .map_err(InferenceError::InvalidSubstitution)?;
        return Ok(Formula::equal(a, b));
    }
    if !scope.is_empty() && a.check_scope(&[]).is_ok() && b.check_scope(&[]).is_ok() {
        return Ok(Formula::equal(a, b));
    }
    let close = |ty: Term| -> Result<Term, InferenceError> {
        let ty = if scope.is_empty() {
            ty
        } else {
            Term::Function {
                parameters: scope
                    .iter()
                    .enumerate()
                    .map(|(slot, (sort, ty))| {
                        Ok(nir::Parameter {
                            sort: *sort,
                            ty: ty.clone().ok_or(InferenceError::InvalidSubstitution(
                                nir::TypeError::MissingArgument(slot),
                            ))?,
                        })
                    })
                    .collect::<Result<Vec<_>, InferenceError>>()?,
                result: Box::new(ty),
            }
        };
        ty.check_scope(&[])
            .map_err(InferenceError::InvalidSubstitution)?;
        Ok(ty)
    };
    Ok(Formula::equal(close(a)?, close(b)?))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObligationId(u64);

#[derive(Clone, Debug)]
pub enum Requirement {
    Stable(Formula),
    Equal(InferenceTerm, InferenceTerm),
    Kind(InferenceTerm, TypeKind),
    Not(Box<Requirement>),
    And(Vec<Requirement>),
    Or(Vec<Requirement>),
}
impl Requirement {
    fn dependencies(
        &self,
        inference: &InferenceContext,
    ) -> Result<BTreeSet<MetaId>, InferenceError> {
        let mut result = BTreeSet::new();
        let mut pending = vec![self];
        while let Some(requirement) = pending.pop() {
            match requirement {
                Self::Stable(_) => {}
                Self::Equal(a, b) => {
                    result.extend(inference.unresolved(a)?);
                    result.extend(inference.unresolved(b)?);
                }
                Self::Kind(ty, _) => result.extend(inference.unresolved(ty)?),
                Self::Not(inner) => pending.push(inner),
                Self::And(items) | Self::Or(items) => pending.extend(items),
            }
        }
        Ok(result)
    }
    fn freeze(
        &self,
        inference: &InferenceContext,
        scope: &[(Sort, Option<Term>)],
        native_scope: bool,
    ) -> Result<Formula, InferenceError> {
        Ok(match self {
            Self::Stable(formula) => formula.clone(),
            Self::Equal(a, b) => scoped_equality(
                inference.freeze(a)?,
                inference.freeze(b)?,
                scope,
                native_scope,
            )?,
            Self::Kind(ty, kind) => Formula::kind(inference.freeze(ty)?, *kind),
            Self::Not(inner) => inner.freeze(inference, scope, native_scope)?.negate(),
            Self::And(items) => Formula::And(
                items
                    .iter()
                    .map(|item| item.freeze(inference, scope, native_scope))
                    .collect::<Result<_, _>>()?,
            ),
            Self::Or(items) => Formula::Or(
                items
                    .iter()
                    .map(|item| item.freeze(inference, scope, native_scope))
                    .collect::<Result<_, _>>()?,
            ),
        })
    }
    fn dependencies_metered(
        &self,
        inference: &InferenceContext,
        budget: &mut Budget,
    ) -> Result<BTreeSet<MetaId>, InferenceError> {
        let mut result = BTreeSet::new();
        let mut pending = vec![self];
        while let Some(requirement) = pending.pop() {
            budget.steps = budget.steps.checked_sub(1).ok_or(InferenceError::Budget)?;
            match requirement {
                Self::Stable(_) => {}
                Self::Equal(a, b) => {
                    result.extend(inference.unresolved_with_limits(
                        a,
                        &mut budget.steps,
                        budget.max_depth,
                    )?);
                    result.extend(inference.unresolved_with_limits(
                        b,
                        &mut budget.steps,
                        budget.max_depth,
                    )?);
                }
                Self::Kind(ty, _) => result.extend(inference.unresolved_with_limits(
                    ty,
                    &mut budget.steps,
                    budget.max_depth,
                )?),
                Self::Not(inner) => pending.push(inner),
                Self::And(items) | Self::Or(items) => pending.extend(items),
            }
        }
        Ok(result)
    }
    fn freeze_metered(
        &self,
        inference: &InferenceContext,
        scope: &[(Sort, Option<Term>)],
        native_scope: bool,
        budget: &mut Budget,
        depth: usize,
    ) -> Result<Formula, InferenceError> {
        if depth >= budget.max_depth.min(128) {
            return Err(InferenceError::Budget);
        }
        budget.steps = budget.steps.checked_sub(1).ok_or(InferenceError::Budget)?;
        if matches!(self, Self::Equal(..)) {
            for _ in 0..2 {
                for (_, domain) in scope {
                    budget.steps = budget.steps.checked_sub(1).ok_or(InferenceError::Budget)?;
                    if let Some(ty) = domain {
                        nir::type_computations::charge_term_with_depth(
                            ty,
                            &mut budget.steps,
                            budget.max_depth,
                        )
                        .map_err(|_| InferenceError::Budget)?;
                    }
                }
            }
        }
        Ok(match self {
            Self::Stable(formula) => {
                if !ptc::within_limits(
                    formula,
                    budget.max_depth.saturating_sub(depth),
                    &mut budget.steps,
                ) {
                    return Err(InferenceError::Budget);
                }
                formula.clone()
            }
            Self::Equal(a, b) => scoped_equality(
                inference.freeze_with_limits(a, &mut budget.steps, budget.max_depth)?,
                inference.freeze_with_limits(b, &mut budget.steps, budget.max_depth)?,
                scope,
                native_scope,
            )?,
            Self::Kind(ty, kind) => Formula::kind(
                inference.freeze_with_limits(ty, &mut budget.steps, budget.max_depth)?,
                *kind,
            ),
            Self::Not(inner) => inner
                .freeze_metered(inference, scope, native_scope, budget, depth + 1)?
                .negate(),
            Self::And(items) => Formula::And(
                items
                    .iter()
                    .map(|item| {
                        item.freeze_metered(inference, scope, native_scope, budget, depth + 1)
                    })
                    .collect::<Result<_, _>>()?,
            ),
            Self::Or(items) => Formula::Or(
                items
                    .iter()
                    .map(|item| {
                        item.freeze_metered(inference, scope, native_scope, budget, depth + 1)
                    })
                    .collect::<Result<_, _>>()?,
            ),
        })
    }
}

#[derive(Clone, Debug)]
pub enum State {
    Ready,
    Waiting(BTreeSet<MetaId>),
    Solved(Outcome),
    InvalidInference(String),
}
impl State {
    pub fn is_proven(&self) -> bool {
        matches!(self, Self::Solved(Outcome::Entailed(_)))
    }
}
#[derive(Clone, Debug)]
struct Obligation {
    requirement: Arc<Requirement>,
    state: State,
    attempts: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Inference(InferenceError),
    StaleCandidate,
    Unsettled(ObligationId),
    InvalidContext,
}
impl From<InferenceError> for Error {
    fn from(error: InferenceError) -> Self {
        Self::Inference(error)
    }
}

pub struct Engine {
    identity: u64,
    revision: u64,
    parent: Option<(u64, u64)>,
    inference: InferenceContext,
    context: ptc::Context,
    scope: Arc<Vec<(Sort, Option<Term>)>>,
    obligations: Arc<BTreeMap<ObligationId, Arc<Obligation>>>,
    ready: Arc<BTreeSet<ObligationId>>,
    watchers: Arc<BTreeMap<MetaId, Arc<BTreeSet<ObligationId>>>>,
}
impl Default for Engine {
    fn default() -> Self {
        Self {
            identity: fresh_identity(),
            revision: 0,
            parent: None,
            inference: InferenceContext::default(),
            context: ptc::Context::default(),
            scope: Arc::default(),
            obligations: Arc::default(),
            ready: Arc::default(),
            watchers: Arc::default(),
        }
    }
}
impl Engine {
    pub(crate) fn is_tuple_head(&self, head: nir::DeclarationId) -> bool {
        self.context.is_tuple_head(head)
    }
    pub(crate) fn is_type_list_domain(&self, ty: &Term) -> bool {
        self.context.is_list_type(ty) && nir::normal_forms::type_list_input_type(ty)
    }

    /// Read the payload of an already stage-authorized signature. This is a
    /// checking hint, not declaration formation or a new proof of layout.
    pub(crate) fn tuple_input<'a>(&self, ty: &'a Term) -> Option<(Option<&'a Term>, &'a [Term])> {
        let Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments,
        } = ty
        else {
            return None;
        };
        if !self.is_tuple_head(*head) {
            return None;
        }
        let [input] = arguments.as_slice() else {
            return None;
        };
        let (domain, fields) = match input {
            Term::TypeListValue(fields) => (None, fields.as_slice()),
            Term::NominalValue {
                ty,
                variant: None,
                fields,
            } if self.is_type_list_domain(ty) => (Some(ty.as_ref()), fields.as_slice()),
            _ => return None,
        };
        fields
            .iter()
            .all(|field| field.sort() == Sort::Type)
            .then_some((domain, fields))
    }
    pub fn is_nominal_constructor(&self, head: nir::DeclarationId) -> bool {
        self.context.is_nominal_constructor(head)
    }
    pub(crate) fn with_call_scope(context: ptc::Context, scope: Vec<nir::Parameter>) -> Self {
        Self {
            context,
            scope: scope
                .into_iter()
                .map(|parameter| (parameter.sort, Some(parameter.ty)))
                .collect::<Vec<_>>()
                .into(),
            ..Self::default()
        }
    }

    /// Sparse caller domains are lexical inputs, never fabricated types. Closed
    /// assumptions can enter this scope unchanged; assumptions in another open
    /// scope cannot be silently discarded or reinterpreted.
    pub(crate) fn with_sparse_call_scope(
        context: ptc::Context,
        scope: Vec<(Sort, Option<Term>)>,
    ) -> Result<Self, Error> {
        let frames = scope_frames(&scope);
        for (slot, (sort, domain)) in scope.iter().enumerate() {
            if let Some(domain) = domain {
                domain
                    .check_scope(&[scope[..slot].iter().map(|entry| entry.0).collect()])
                    .map_err(|error| {
                        Error::Inference(InferenceError::InvalidSubstitution(error))
                    })?;
                if domain.sort() != Sort::Type {
                    return Err(Error::InvalidContext);
                }
            } else if *sort == Sort::Type {
                return Err(Error::InvalidContext);
            }
        }
        let context = if context.scope() == frames.as_slice() {
            context
        } else if context.scope().is_empty() {
            let mut scoped = context.theory_in_scope(frames);
            let mut budget = Budget::default();
            for assumption in context.assumptions() {
                context
                    .validate_formula(assumption, &mut budget)
                    .map_err(|_| Error::InvalidContext)?;
                scoped.assume(assumption.clone());
            }
            scoped
        } else {
            return Err(Error::InvalidContext);
        };
        Ok(Self {
            context,
            scope: Arc::new(scope),
            ..Self::default()
        })
    }

    pub(crate) fn check_context_type(&self, ty: &Term) -> Result<(), Error> {
        ty.check_scope(&scope_frames(&self.scope))
            .map_err(|error| Error::Inference(InferenceError::InvalidSubstitution(error)))
    }
    pub(crate) fn assume_type_equality(&mut self, a: Term, b: Term) -> Result<(), Error> {
        self.assume(scoped_equality(
            a,
            b,
            &self.scope,
            self.context.scope() == scope_frames(&self.scope).as_slice(),
        )?);
        Ok(())
    }
    pub fn with_context(context: ptc::Context) -> Self {
        Self {
            context,
            ..Self::default()
        }
    }
    /// Match a whole call/interface in one transaction. Later arguments can
    /// settle variables needed by earlier obligations before any commit.
    pub fn match_many(
        &self,
        relations: &[(InferenceTerm, InferenceTerm)],
        budget: Budget,
    ) -> Result<(Self, Vec<ObligationId>), crate::matching::MatchError> {
        self.match_many_metered(relations, &mut { budget })
    }
    /// Failed trials preserve the parent and return their consumed work.
    pub fn match_many_metered(
        &self,
        relations: &[(InferenceTerm, InferenceTerm)],
        budget: &mut Budget,
    ) -> Result<(Self, Vec<ObligationId>), crate::matching::MatchError> {
        let mut candidate = self.trial();
        loop {
            let before = candidate.inference.revision;
            for (left, right) in relations {
                crate::matching::propose_metered(
                    &mut candidate.inference,
                    left,
                    right,
                    &candidate.context,
                    &scope_frames(&candidate.scope),
                    budget,
                )?;
            }
            // New assignments can disambiguate an earlier structural row.
            // Retry only after real progress, under one transaction budget.
            if candidate.inference.revision == before {
                break;
            }
        }
        candidate.invalidate();
        let ids = relations
            .iter()
            .map(|(a, b)| candidate.require(Requirement::Equal(a.clone(), b.clone())))
            .collect();
        Ok((candidate, ids))
    }
    /// Produce an isolated candidate with structural inference proposals and
    /// an obligation on the original complete relation. The parent is never
    /// changed, including on budget exhaustion or a partially matched shape.
    pub fn match_types(
        &self,
        left: InferenceTerm,
        right: InferenceTerm,
        budget: Budget,
    ) -> Result<(Self, ObligationId), crate::matching::MatchError> {
        let mut candidate = self.trial();
        crate::matching::propose(
            &mut candidate.inference,
            &left,
            &right,
            &candidate.context,
            &scope_frames(&candidate.scope),
            budget,
        )?;
        // Assignments made by matching can affect existing region obligations.
        candidate.invalidate();
        let obligation = candidate.require(Requirement::Equal(left, right));
        Ok((candidate, obligation))
    }
    fn changed(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("obligation revision exhausted");
    }
    pub fn fresh(&mut self, sort: Sort) -> MetaId {
        let id = self.inference.fresh(sort);
        self.changed();
        id
    }
    pub fn assign(&mut self, id: MetaId, value: InferenceTerm) -> Result<(), Error> {
        self.inference.assign(id, value)?;
        let affected = self.watchers.get(&id).cloned().unwrap_or_default();
        for obligation in affected.iter().copied() {
            self.enqueue(obligation);
        }
        self.changed();
        Ok(())
    }
    fn enqueue(&mut self, id: ObligationId) {
        if let State::Waiting(dependencies) = &self.obligations[&id].state {
            for dependency in dependencies {
                if let Some(waiters) = Arc::make_mut(&mut self.watchers).get_mut(dependency) {
                    Arc::make_mut(waiters).remove(&id);
                }
            }
        }
        let old = &self.obligations[&id];
        let ready = Arc::new(Obligation {
            requirement: old.requirement.clone(),
            state: State::Ready,
            attempts: old.attempts,
        });
        Arc::make_mut(&mut self.obligations).insert(id, ready);
        Arc::make_mut(&mut self.ready).insert(id);
    }
    fn invalidate(&mut self) {
        for id in self.obligations.keys().copied().collect::<Vec<_>>() {
            self.enqueue(id);
        }
        self.changed();
    }
    /// Add an already-established fact valid in this region. Candidate
    /// applicability conditions belong in `require`, never in `assume`.
    /// The caller owns provenance/branch legality; PTC does not invent it.
    pub fn assume(&mut self, fact: Formula) {
        self.context.assume(fact);
        // Countermodels and proof support belong to the previous exact scope.
        self.invalidate();
    }
    pub fn admit_congruence(&mut self, head: DeclarationId) {
        self.context.admit_congruence(head);
        self.invalidate();
    }
    pub fn require(&mut self, requirement: Requirement) -> ObligationId {
        let id = ObligationId(fresh_identity());
        Arc::make_mut(&mut self.obligations).insert(
            id,
            Arc::new(Obligation {
                requirement: Arc::new(requirement),
                state: State::Ready,
                attempts: 0,
            }),
        );
        Arc::make_mut(&mut self.ready).insert(id);
        self.changed();
        id
    }
    pub fn state(&self, id: ObligationId) -> Option<&State> {
        self.obligations.get(&id).map(|o| &o.state)
    }
    /// Read-only diagnostic snapshot, with a separate bounded expansion budget.
    /// Reporting cannot retry a proof or turn an unknown outcome into refutation.
    pub fn describe_requirement(&self, id: ObligationId) -> Option<(String, &'static str)> {
        let obligation = self.obligations.get(&id)?;
        let status = match &obligation.state {
            State::Ready => "not checked",
            State::Waiting(_) => "waiting for inference",
            State::Solved(Outcome::Entailed(_)) => "proven",
            State::Solved(Outcome::NotEntailed(_)) => "not entailed",
            State::Solved(Outcome::Unknown { .. }) => "solver inconclusive",
            State::Solved(Outcome::IllFormed(_)) => "ill-formed constraint",
            State::InvalidInference(_) => "invalid inference",
        };
        let requirement = match obligation.requirement.freeze_metered(
            &self.inference,
            &self.scope,
            self.context.scope() == scope_frames(&self.scope).as_slice(),
            &mut Budget {
                steps: 4096,
                max_depth: 64,
            },
            0,
        ) {
            Ok(formula) => formula.to_string(),
            Err(error) => format!("(requirement-unavailable {error:?})"),
        };
        Some((requirement, status))
    }
    /// A proved counterexample can discard a candidate; waiting or Unknown
    /// obligations cannot. This does not turn refutation into an assumption.
    pub fn has_refuted_obligation(&self) -> bool {
        self.obligations
            .values()
            .any(|obligation| matches!(obligation.state, State::Solved(Outcome::NotEntailed(_))))
    }
    pub fn attempts(&self, id: ObligationId) -> Option<usize> {
        self.obligations.get(&id).map(|o| o.attempts)
    }

    /// Process at most `limit` ready obligations. PTC has a separate per-query
    /// budget; an idle queue never repeatedly polls unresolved variables.
    pub fn advance(&mut self, limit: usize, budget: Budget) -> usize {
        let mut processed = 0;
        while processed < limit {
            let Some(id) = Arc::make_mut(&mut self.ready).pop_first() else {
                break;
            };
            let obligation =
                Arc::make_mut(Arc::make_mut(&mut self.obligations).get_mut(&id).unwrap());
            obligation.attempts += 1;
            obligation.state = match obligation.requirement.dependencies(&self.inference) {
                Err(error) => State::InvalidInference(error.to_string()),
                Ok(dependencies) if !dependencies.is_empty() => {
                    for dependency in &dependencies {
                        Arc::make_mut(
                            Arc::make_mut(&mut self.watchers)
                                .entry(*dependency)
                                .or_default(),
                        )
                        .insert(id);
                    }
                    State::Waiting(dependencies)
                }
                Ok(_) => match obligation.requirement.freeze(
                    &self.inference,
                    &self.scope,
                    self.context.scope() == scope_frames(&self.scope).as_slice(),
                ) {
                    Ok(formula) => State::Solved(self.context.prove(formula, budget)),
                    Err(error) => State::InvalidInference(error.to_string()),
                },
            };
            processed += 1;
        }
        if processed != 0 {
            self.changed();
        }
        processed
    }
    /// Advance ready obligations under one shared proof allowance. Unprocessed
    /// entries remain queued when the allowance is exhausted.
    pub fn advance_metered(&mut self, limit: usize, budget: &mut Budget) -> usize {
        let mut processed = 0;
        while processed < limit && budget.steps > 0 {
            let Some(id) = Arc::make_mut(&mut self.ready).pop_first() else {
                break;
            };
            budget.steps -= 1;
            let obligation =
                Arc::make_mut(Arc::make_mut(&mut self.obligations).get_mut(&id).unwrap());
            obligation.attempts += 1;
            let mut paused = false;
            obligation.state = match obligation
                .requirement
                .dependencies_metered(&self.inference, budget)
            {
                Err(InferenceError::Budget) => {
                    paused = true;
                    State::Ready
                }
                Err(error) => State::InvalidInference(error.to_string()),
                Ok(dependencies) if !dependencies.is_empty() => {
                    for dependency in &dependencies {
                        Arc::make_mut(
                            Arc::make_mut(&mut self.watchers)
                                .entry(*dependency)
                                .or_default(),
                        )
                        .insert(id);
                    }
                    State::Waiting(dependencies)
                }
                Ok(_) => match obligation.requirement.freeze_metered(
                    &self.inference,
                    &self.scope,
                    self.context.scope() == scope_frames(&self.scope).as_slice(),
                    budget,
                    0,
                ) {
                    Ok(formula) => State::Solved(self.context.prove_metered(formula, budget)),
                    Err(InferenceError::Budget) => {
                        paused = true;
                        State::Ready
                    }
                    Err(error) => State::InvalidInference(error.to_string()),
                },
            };
            processed += 1;
            if paused {
                Arc::make_mut(&mut self.ready).insert(id);
                break;
            }
        }
        if processed != 0 {
            self.changed();
        }
        processed
    }
    /// Explicit retry, e.g. after raising a search budget. No busy-loop retries.
    pub fn retry_unknown(&mut self) {
        let ids: Vec<_> = self
            .obligations
            .iter()
            .filter_map(|(id, obligation)| {
                matches!(obligation.state, State::Solved(Outcome::Unknown { .. })).then_some(*id)
            })
            .collect();
        if !ids.is_empty() {
            for id in ids {
                self.enqueue(id);
            }
            self.changed();
        }
    }
    /// All candidate tables are shared until written. Individual requirements
    /// and untouched obligations remain shared even after a table detaches.
    pub fn trial(&self) -> Self {
        Self {
            identity: fresh_identity(),
            revision: 0,
            parent: Some((self.identity, self.revision)),
            inference: self.inference.trial(),
            context: self.context.fork(),
            scope: self.scope.clone(),
            obligations: self.obligations.clone(),
            ready: self.ready.clone(),
            watchers: self.watchers.clone(),
        }
    }
    fn check_settled(&self) -> Result<(), Error> {
        if let Some((id, _)) = self.obligations.iter().find(|(_, o)| !o.state.is_proven()) {
            return Err(Error::Unsettled(*id));
        }
        if !matches!(
            self.context.prove(Formula::True, Budget::default()),
            Outcome::Entailed(_)
        ) {
            return Err(Error::InvalidContext);
        }
        Ok(())
    }
    /// Only a fully accepted candidate publishes its inference, facts and
    /// evidence together. Failed attempts leave the parent untouched.
    pub fn commit(&mut self, candidate: Self) -> Result<(), Error> {
        if candidate.parent != Some((self.identity, self.revision)) {
            return Err(Error::StaleCandidate);
        }
        candidate.check_settled()?;
        self.inference.commit(candidate.inference)?;
        self.context = candidate.context;
        self.obligations = candidate.obligations;
        self.ready = candidate.ready;
        self.watchers = candidate.watchers;
        self.changed();
        Ok(())
    }
    pub(crate) fn waits_only_for_inference(&self) -> bool {
        let mut waiting = false;
        for obligation in self.obligations.values() {
            match &obligation.state {
                State::Waiting(_) => waiting = true,
                state if state.is_proven() => {}
                _ => return false,
            }
        }
        waiting
    }
    // Read-only candidate inspection. These are NOT export operations: callers
    // must still settle the original obligations before publishing any NIR.
    pub fn inspect(&self, term: &InferenceTerm) -> Result<InferenceTerm, Error> {
        Ok(self.inference.resolve(term)?)
    }
    pub(crate) fn inspect_type(&self, term: &InferenceTerm) -> Result<Term, Error> {
        Ok(self.inference.freeze(term)?)
    }
    /// Candidate-local observation, not an export of unsettled obligations.
    pub(crate) fn inspect_type_metered(
        &self,
        term: &InferenceTerm,
        budget: &mut Budget,
    ) -> Result<Term, InferenceError> {
        self.inference
            .freeze_with_limits(term, &mut budget.steps, budget.max_depth)
    }
    /// Export contains stable NIR terms only, never active variables or waiters.
    pub fn freeze(&self, term: &InferenceTerm) -> Result<Term, Error> {
        self.check_settled()?;
        Ok(self.inference.freeze(term)?)
    }
}

impl fmt::Display for Requirement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stable(formula) => write!(f, "(stable {formula})"),
            Self::Equal(a, b) => write!(f, "(type-eq {a} {b})"),
            Self::Kind(ty, kind) => write!(f, "(type-kind {ty} {kind})"),
            Self::Not(inner) => write!(f, "(not {inner})"),
            Self::And(items) | Self::Or(items) => {
                f.write_str(if matches!(self, Self::And(_)) {
                    "(and"
                } else {
                    "(or"
                })?;
                for item in items {
                    write!(f, " {item}")?;
                }
                f.write_str(")")
            }
        }
    }
}
impl fmt::Display for Engine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(obligations")?;
        if !self.scope.is_empty() {
            f.write_str(" (scope")?;
            for (sort, domain) in self.scope.iter() {
                match domain {
                    Some(ty) => write!(f, " (parameter {sort} {ty})")?,
                    None => write!(f, " (parameter {sort} (unknown-domain))")?,
                }
            }
            f.write_str(")")?;
        }
        for (id, obligation) in self.obligations.iter() {
            write!(
                f,
                " (obligation {} (attempts {}) (requirement {}) (state ",
                id.0, obligation.attempts, obligation.requirement
            )?;
            match &obligation.state {
                State::Ready => f.write_str("ready")?,
                State::Waiting(dependencies) => {
                    f.write_str("(waiting")?;
                    for dependency in dependencies {
                        write!(f, " {dependency}")?;
                    }
                    f.write_str(")")?;
                }
                State::Solved(outcome) => write!(f, "{outcome}")?,
                State::InvalidInference(message) => write!(f, "(invalid-inference {message:?})")?,
            }
            f.write_str("))")?;
        }
        f.write_str(")")
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    #[test]
    fn candidate_wakeups_preserve_parent_waiters_and_share_untouched_proofs() {
        let mut parent = Engine::default();
        let variable = parent.fresh(Sort::Type);
        let boolean = InferenceTerm::Stable(Term::Primitive(nir::Primitive::Bool));
        let ids = (0..64)
            .map(|_| {
                parent.require(Requirement::Equal(
                    InferenceTerm::Meta(variable),
                    boolean.clone(),
                ))
            })
            .collect::<Vec<_>>();
        let settled = parent.require(Requirement::Stable(Formula::True));
        parent.advance(65, Budget::default());
        let mut child = parent.trial();
        let sibling = parent.trial();
        assert!(Arc::ptr_eq(&parent.obligations, &child.obligations));
        assert!(Arc::ptr_eq(&parent.watchers, &child.watchers));
        assert!(Arc::ptr_eq(&parent.ready, &child.ready));
        assert!(Arc::ptr_eq(&parent.scope, &child.scope));
        child.assign(variable, boolean).unwrap();
        assert!(Arc::ptr_eq(
            &parent.obligations[&settled],
            &child.obligations[&settled]
        ));
        assert!(!Arc::ptr_eq(&parent.obligations, &child.obligations));
        for id in &ids {
            assert!(matches!(parent.state(*id), Some(State::Waiting(_))));
            assert!(matches!(child.state(*id), Some(State::Ready)));
            assert!(Arc::ptr_eq(
                &parent.obligations[id].requirement,
                &child.obligations[id].requirement
            ));
        }
        assert!(parent.ready.is_empty());
        assert_eq!(parent.watchers[&variable].len(), 64);
        assert!(child.watchers[&variable].is_empty());
        child.advance(64, Budget::default());
        parent.commit(child).unwrap();
        for id in ids {
            assert!(parent.state(id).unwrap().is_proven());
            assert!(matches!(sibling.state(id), Some(State::Waiting(_))));
        }
        assert_eq!(parent.commit(sibling), Err(Error::StaleCandidate));
    }
}
