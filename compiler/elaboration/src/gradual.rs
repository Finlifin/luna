//! Definition-side type descriptions, distinct from immutable type identity.
//! Any is intentional loss of precision, never an unsolved inference variable
//! or an equality axiom. Compatibility alone supplies no runtime conversion,
//! layout, member lookup, or trait evidence.
use crate::ptc::{self, Context, Formula, Outcome};
use crate::{InferenceContext, InferenceError, InferenceTerm, MetaId};
use nir::{Sort, Term};
use std::{collections::BTreeSet, fmt};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Description {
    Any,
    Known(InferenceTerm),
}

/// Stable ordinary-elaboration information. This is deliberately not a NIR
/// Type: Any has no layout, equality identity, or implicit conversion to Term.
/// Keeping inference terms out of this representation allows saved source
/// plans to outlive the inference world which produced their descriptions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrozenDescription {
    Any,
    Known(Term),
}

/// Definition-side telescope. It preserves declaration binders and stage tags,
/// but deliberately has no conversion into an immutable function Type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureDescription {
    parameters: Vec<(Sort, FrozenDescription)>,
    result: FrozenDescription,
}
impl SignatureDescription {
    pub fn from_residual(
        signature: &nir::deferred_signatures::Signature,
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        *fuel = fuel
            .checked_sub(signature.inputs().len().saturating_add(1))
            .ok_or(Error::Budget)?;
        let parameters = signature
            .inputs()
            .iter()
            .map(|input| {
                Ok((
                    input.sort,
                    FrozenDescription::from_annotation(&input.annotation, fuel)?,
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let result = FrozenDescription::from_annotation(signature.result(), fuel)?;
        Ok(Self { parameters, result })
    }
    pub fn parameters(&self) -> &[(Sort, FrozenDescription)] {
        &self.parameters
    }
    pub fn result(&self) -> &FrozenDescription {
        &self.result
    }
    /// Read a parameter or the final result slot, retaining declaration scope.
    pub fn annotation(&self, slot: usize) -> Option<&FrozenDescription> {
        if slot == self.parameters.len() {
            Some(&self.result)
        } else {
            self.parameters.get(slot).map(|(_, ty)| ty)
        }
    }
}
impl fmt::Display for SignatureDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(signature-description (parameters")?;
        for (sort, description) in &self.parameters {
            write!(f, " (parameter {sort} {description})")?;
        }
        write!(f, ") (result {}))", self.result)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Freeze {
    Ready(FrozenDescription),
    Waiting(BTreeSet<MetaId>),
}

impl Description {
    /// Freeze precision, not solver activity. An unassigned inference variable
    /// remains a waiter; it must never silently turn into Any. Declaration
    /// binders are permitted only in the supplied type context's telescope.
    pub fn freeze(
        &self,
        inference: &InferenceContext,
        types: &Context,
        budget: &mut ptc::Budget,
    ) -> Result<Freeze, Error> {
        if inference.parent.is_some() {
            return Err(Error::SpeculativeContext);
        }
        budget.steps = budget.steps.checked_sub(1).ok_or(Error::Budget)?;
        let mut waiting = BTreeSet::new();
        let known = resolve(self, inference, types, &mut waiting, budget)?;
        if !waiting.is_empty() {
            return Ok(Freeze::Waiting(waiting));
        }
        Ok(Freeze::Ready(match known {
            Some(ty) => FrozenDescription::Known(ty),
            None => FrozenDescription::Any,
        }))
    }
}
impl FrozenDescription {
    /// Inspect saved syntax-level knowledge without allocating a computation
    /// task. Repeated computed annotations remain Any, even when they name the
    /// same residual; this supplies no equality between their eventual Types.
    pub fn from_annotation(
        annotation: &nir::deferred_signatures::Annotation,
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        Ok(match annotation {
            nir::deferred_signatures::Annotation::Known(ty) => {
                nir::type_computations::charge_term(ty, fuel).map_err(|_| Error::Budget)?;
                if ty.sort() != Sort::Type {
                    return Err(Error::Scope(nir::TypeError::WrongSort {
                        expected: Sort::Type,
                        actual: ty.sort(),
                    }));
                }
                Self::Known(ty.clone())
            }
            nir::deferred_signatures::Annotation::Computed(_) => Self::Any,
        })
    }
    /// Re-enter an elaboration world without allocating inference variables or
    /// making equality assumptions. Scope is checked by the consuming context.
    pub fn description(&self) -> Description {
        match self {
            Self::Any => Description::Any,
            Self::Known(ty) => Description::Known(InferenceTerm::Stable(ty.clone())),
        }
    }
    /// Instantiate declaration binders lazily, preserving intentional Any and
    /// the distinction between it and unresolved caller inputs.
    pub fn instantiate(&self, inputs: Vec<InferenceTerm>) -> Description {
        match self {
            Self::Any => Description::Any,
            Self::Known(ty) => Description::Known(InferenceTerm::Instantiate {
                template: ty.clone(),
                arguments: inputs,
            }),
        }
    }
}
impl fmt::Display for FrozenDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => write!(f, "(any-type-description)"),
            Self::Known(ty) => write!(f, "(known-type-description {ty})"),
        }
    }
}
impl fmt::Display for Freeze {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready(description) => write!(f, "(frozen-description {description})"),
            Self::Waiting(inputs) => {
                write!(f, "(description-freeze-waiting")?;
                for input in inputs {
                    write!(f, " {input}")?;
                }
                write!(f, ")")
            }
        }
    }
}
#[derive(Debug)]
pub enum Compatibility {
    /// Instantiation and delivery checks remain necessary. This is not a proof.
    Gradual,
    /// Only Entailed is evidence of exact equality. Keep refutation/Unknown
    /// distinct so callers cannot turn failure to prove into a conversion.
    Exact(Outcome),
    Waiting(BTreeSet<MetaId>),
}
#[derive(Debug)]
pub enum Join {
    Ready(Description),
    Waiting(BTreeSet<MetaId>),
    Unproven(Outcome),
}
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    SpeculativeContext,
    Inference(InferenceError),
    Scope(nir::TypeError),
    InvalidRequirement(ptc::ValidationError),
    Budget,
}
impl From<InferenceError> for Error {
    fn from(value: InferenceError) -> Self {
        if value == InferenceError::Budget {
            Self::Budget
        } else {
            Self::Inference(value)
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(description-error {:?})", format!("{self:?}"))
    }
}
impl std::error::Error for Error {}

/// Resolve ordinary inference without treating a waiting metavariable as Any.
/// Validation also applies when the other operand is Any.
fn resolve(
    description: &Description,
    inference: &InferenceContext,
    types: &Context,
    waiting: &mut BTreeSet<MetaId>,
    budget: &mut ptc::Budget,
) -> Result<Option<Term>, Error> {
    let Description::Known(value) = description else {
        return Ok(None);
    };
    let actual = inference.sort(value)?;
    if actual != Sort::Type {
        return Err(InferenceError::WrongSort {
            expected: Sort::Type,
            actual,
        }
        .into());
    }
    let unresolved =
        inference.unresolved_with_limits(value, &mut budget.steps, budget.max_depth)?;
    if !unresolved.is_empty() {
        waiting.extend(unresolved);
        return Ok(None);
    }
    let value = inference.freeze_with_limits(value, &mut budget.steps, budget.max_depth)?;
    value.check_scope(types.scope()).map_err(Error::Scope)?;
    Ok(Some(value))
}
pub fn compatible(
    left: &Description,
    right: &Description,
    inference: &InferenceContext,
    types: &Context,
    budget: &mut ptc::Budget,
) -> Result<Compatibility, Error> {
    budget.steps = budget.steps.checked_sub(1).ok_or(Error::Budget)?;
    let mut waiting = BTreeSet::new();
    let left = resolve(left, inference, types, &mut waiting, budget)?;
    let right = resolve(right, inference, types, &mut waiting, budget)?;
    if !waiting.is_empty() {
        return Ok(Compatibility::Waiting(waiting));
    }
    match (left, right) {
        (Some(left), Some(right)) => Ok(Compatibility::Exact(
            types.prove_metered(Formula::equal(left, right), budget),
        )),
        _ => Ok(Compatibility::Gradual),
    }
}
/// This is the no-target rule only. Target-directed conversions, branch facts
/// and final representation selection remain separate obligations.
pub fn join_without_target(
    left: &Description,
    right: &Description,
    inference: &InferenceContext,
    types: &Context,
    budget: &mut ptc::Budget,
) -> Result<Join, Error> {
    budget.steps = budget.steps.checked_sub(1).ok_or(Error::Budget)?;
    let mut waiting = BTreeSet::new();
    // Validate both branches before using non-completion to remove one from
    // the value join. A diverging branch cannot hide an invalid description.
    let left_ty = resolve(left, inference, types, &mut waiting, budget)?;
    let right_ty = resolve(right, inference, types, &mut waiting, budget)?;
    if !waiting.is_empty() {
        return Ok(Join::Waiting(waiting));
    }
    let (Some(left_ty), Some(right_ty)) = (left_ty, right_ty) else {
        return Ok(Join::Ready(Description::Any));
    };
    if left_ty == Term::Primitive(nir::Primitive::Never) {
        return Ok(Join::Ready(right.clone()));
    }
    if right_ty == Term::Primitive(nir::Primitive::Never) {
        return Ok(Join::Ready(left.clone()));
    }
    match types.prove_metered(Formula::equal(left_ty.clone(), right_ty.clone()), budget) {
        Outcome::Entailed(_) => return Ok(Join::Ready(left.clone())),
        Outcome::NotEntailed(_)
        | Outcome::Unknown {
            reason: ptc::UnknownReason::UnvalidatedModel,
            ..
        } => {}
        outcome => return Ok(Join::Unproven(outcome)),
    }
    // This is a control-flow join rule, not type equality or a conversion.
    // Symbolic non-completion needs proof in this branch's supplied context;
    // neither a missing proof nor an opaque head's body is such evidence.
    for (ty, other) in [(&left_ty, right), (&right_ty, left)] {
        match ty {
            Term::Primitive(_) => continue,
            _ => {}
        }
        match types.prove_metered(
            Formula::equal(ty.clone(), Term::Primitive(nir::Primitive::Never)),
            budget,
        ) {
            Outcome::Entailed(_) => return Ok(Join::Ready(other.clone())),
            Outcome::NotEntailed(_)
            | Outcome::Unknown {
                reason: ptc::UnknownReason::UnvalidatedModel,
                ..
            } => {}
            outcome => return Ok(Join::Unproven(outcome)),
        }
    }
    // Missing equality/non-completion evidence permits loss of precision,
    // not a proof of inequality. Resource exhaustion returned above instead.
    Ok(Join::Ready(Description::Any))
}
impl fmt::Display for Description {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => write!(f, "(any-type-description)"),
            Self::Known(value) => write!(f, "(known-type-description {value})"),
        }
    }
}
impl fmt::Display for Compatibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gradual => write!(f, "(gradual-compatibility)"),
            Self::Exact(outcome) => write!(f, "(exact-compatibility {outcome})"),
            Self::Waiting(inputs) => {
                write!(f, "(compatibility-waiting")?;
                for input in inputs {
                    write!(f, " {input}")?;
                }
                write!(f, ")")
            }
        }
    }
}
impl fmt::Display for Join {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready(value) => write!(f, "(description-join {value})"),
            Self::Unproven(outcome) => write!(f, "(description-join-unproven {outcome})"),
            Self::Waiting(inputs) => {
                write!(f, "(description-join-waiting")?;
                for input in inputs {
                    write!(f, " {input}")?;
                }
                write!(f, ")")
            }
        }
    }
}

/// An extracted type proposition with one declaration-scoped Type subject.
/// Executable predicates and program-state specifications are not Formula and
/// cannot be submitted through this interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refinement {
    pub subject: nir::DeclarationId,
    pub requirement: Formula,
}
#[derive(Debug)]
pub enum RefinementCheck {
    NeedsInstantiation,
    Waiting(BTreeSet<MetaId>),
    Checked(Outcome),
}

/// Hypothetical facts available while checking a refined parameter's body.
/// The subject is a fresh declaration-local Type variable chosen by the
/// elaborator, never the identity of Any or an unsolved invocation metavariable.
/// Instance admission deliberately uses the borrowed parent, not these facts.
pub struct RefinementScope<'a> {
    parent: &'a Context,
    body: Context,
    requirement: Refinement,
}
impl<'a> RefinementScope<'a> {
    pub fn new(
        parent: &'a Context,
        subject: nir::DeclarationId,
        refinements: &[Refinement],
        budget: &mut ptc::Budget,
    ) -> Result<Self, Error> {
        budget.steps = budget
            .steps
            .checked_sub(refinements.len().saturating_add(1))
            .ok_or(Error::Budget)?;
        let actual = Term::Rigid {
            declaration: subject,
            sort: Sort::Type,
        };
        let mut requirements = Vec::with_capacity(refinements.len());
        // Validate every requirement before installing any hypothesis. In
        // particular, an earlier false fact cannot mask a malformed later one.
        for refinement in refinements {
            if !ptc::within_limits(
                &refinement.requirement,
                budget.max_depth.min(128),
                &mut budget.steps,
            ) {
                return Err(Error::Budget);
            }
            let requirement = refinement.instantiate(&actual, parent, budget)?;
            parent
                .validate_formula(&requirement, budget)
                .map_err(Error::InvalidRequirement)?;
            requirements.push(requirement);
        }
        let requirement = Refinement {
            subject,
            requirement: Formula::And(requirements),
        };
        for assumption in parent.assumptions() {
            if !ptc::within_limits(assumption, budget.max_depth.min(128), &mut budget.steps) {
                return Err(Error::Budget);
            }
        }
        if !ptc::within_limits(
            &requirement.requirement,
            budget.max_depth.min(128),
            &mut budget.steps,
        ) {
            return Err(Error::Budget);
        }
        let mut body = parent.fork();
        body.assume(requirement.requirement.clone());
        Ok(Self {
            parent,
            body,
            requirement,
        })
    }

    /// Prove an operation's type-level precondition under definition hypotheses.
    /// This produces no layout, member origin, dispatch route or conversion.
    pub fn prove(&self, goal: Formula, budget: &mut ptc::Budget) -> Outcome {
        self.body.prove_metered(goal, budget)
    }

    /// Recheck admission without the parameter's own hypothetical facts.
    /// Unknown descriptions remain obligations; only Checked(Entailed) proves
    /// that a concrete instance meets the complete refinement conjunction.
    pub fn check_instance(
        &self,
        description: &Description,
        inference: &InferenceContext,
        budget: &mut ptc::Budget,
    ) -> Result<RefinementCheck, Error> {
        self.requirement
            .check(description, inference, self.parent, budget)
    }
}
impl fmt::Display for RefinementScope<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(definition-refinement-scope {})", self.requirement)
    }
}
impl Refinement {
    pub fn check(
        &self,
        description: &Description,
        inference: &InferenceContext,
        types: &Context,
        budget: &mut ptc::Budget,
    ) -> Result<RefinementCheck, Error> {
        if !ptc::within_limits(
            &self.requirement,
            budget.max_depth.min(128),
            &mut budget.steps,
        ) {
            return Err(Error::Budget);
        }
        // Validate the declaration-side subject and scope even if no concrete
        // instance exists yet. Deferral cannot hide a malformed requirement.
        let abstract_requirement = self.instantiate(
            &Term::Rigid {
                declaration: self.subject,
                sort: Sort::Type,
            },
            types,
            budget,
        )?;
        types
            .validate_formula(&abstract_requirement, budget)
            .map_err(Error::InvalidRequirement)?;
        let mut waiting = BTreeSet::new();
        let actual = resolve(description, inference, types, &mut waiting, budget)?;
        if !waiting.is_empty() {
            return Ok(RefinementCheck::Waiting(waiting));
        }
        let Some(actual) = actual else {
            return Ok(RefinementCheck::NeedsInstantiation);
        };
        let requirement = self.instantiate(&actual, types, budget)?;
        // Requirements are goals, never assumptions used to prove themselves.
        Ok(RefinementCheck::Checked(
            types.prove_metered(requirement, budget),
        ))
    }
    fn instantiate(
        &self,
        actual: &Term,
        types: &Context,
        budget: &mut ptc::Budget,
    ) -> Result<Formula, Error> {
        nir::type_computations::charge_term(actual, &mut budget.steps)
            .map_err(|_| Error::Budget)?;
        let probe = Formula::kind(actual.clone(), ptc::TypeKind::Boolean);
        if !ptc::within_limits(&probe, budget.max_depth.min(128), &mut budget.steps) {
            return Err(Error::Budget);
        }
        let parameters = [nir::DeclarationParameter {
            declaration: self.subject,
            sort: Sort::Type,
        }];
        let arguments = [actual.clone()];
        let term = |ty: &Term| {
            ty.instantiate_parameters_in_scope(&parameters, &arguments, types.scope())
                .map_err(Error::Scope)
        };
        fn visit(
            formula: &Formula,
            subject: nir::DeclarationId,
            term: &impl Fn(&Term) -> Result<Term, Error>,
        ) -> Result<Formula, Error> {
            Ok(match formula {
                Formula::True => Formula::True,
                Formula::False => Formula::False,
                Formula::Equal(ptc::Equality(a, b)) => Formula::equal(term(a)?, term(b)?),
                Formula::Kind(query) => Formula::kind(term(&query.ty)?, query.kind),
                Formula::Trait(query) => Formula::Trait(ptc::TraitQuery {
                    interface: query.interface,
                    subject: term(&query.subject)?,
                    arguments: query.arguments.iter().map(term).collect::<Result<_, _>>()?,
                }),
                Formula::IntLe(query) => {
                    if query.left.variable == Some(subject) || query.right.variable == Some(subject)
                    {
                        return Err(Error::Inference(InferenceError::WrongSort {
                            expected: Sort::ComptimeValue,
                            actual: Sort::Type,
                        }));
                    }
                    Formula::IntLe(query.clone())
                }
                Formula::Not(inner) => visit(inner, subject, term)?.negate(),
                Formula::And(parts) => Formula::And(
                    parts
                        .iter()
                        .map(|p| visit(p, subject, term))
                        .collect::<Result<_, _>>()?,
                ),
                Formula::Or(parts) => Formula::Or(
                    parts
                        .iter()
                        .map(|p| visit(p, subject, term))
                        .collect::<Result<_, _>>()?,
                ),
            })
        }
        visit(&self.requirement, self.subject, &term)
    }
}
impl fmt::Display for Refinement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(type-description-refinement (subject {}) (requirement {}))",
            self.subject, self.requirement
        )
    }
}
impl fmt::Display for RefinementCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NeedsInstantiation => write!(f, "(refinement-needs-instantiation)"),
            Self::Checked(outcome) => write!(f, "(refinement-check {outcome})"),
            Self::Waiting(inputs) => {
                write!(f, "(refinement-waiting")?;
                for input in inputs {
                    write!(f, " {input}")?;
                }
                write!(f, ")")
            }
        }
    }
}
