//! Type-only applicability. These proofs do not discharge phase, package
//! coherence, associated members or runtime environment delivery obligations.
use super::{Applicability, Error, ImplementationHead, Index, Selection};
use crate::InferenceTerm as I;
use crate::obligations::{Engine, State};
use crate::ptc::{Budget, Context, Formula, Outcome, Proof};
use nir::program::TypePredicate;
use nir::{DeclarationId, Sort, Term};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

// Eligibility need not reveal one concrete shape. A finite disjunction of
// admissible shapes can authorize matching while the subject stays symbolic.
fn eligible_shape(context: &Context, ty: &Term) -> bool {
    matches!(
        ty,
        Term::Primitive(_)
            | Term::ErrorResult { .. }
            | Term::Nominal { .. }
            | Term::Tuple(_)
            | Term::Record(_)
            | Term::Optional(_)
            | Term::Pointer(_)
            | Term::Function { .. }
            | Term::ComptimeFunction(_)
            | Term::NormalFormFunction(..)
            | Term::UnsafeFunction(_)
    ) || context.is_tuple_type(ty)
        || context.is_list_type(ty)
}

fn prove_eligible_alternatives(
    context: &Context,
    subject: &Term,
    budget: &mut Budget,
) -> Option<Proof> {
    budget.steps = budget.steps.checked_sub(context.assumptions().len())?;
    let mut pending: Vec<_> = context.assumptions().iter().collect();
    let mut seen = std::collections::HashSet::new();
    let mut alternatives = Vec::new();
    while let Some(formula) = pending.pop() {
        budget.steps = budget.steps.checked_sub(1)?;
        match formula {
            Formula::Equal(crate::ptc::Equality(a, b)) => {
                for ty in [a, b] {
                    nir::type_computations::charge_term(ty, &mut budget.steps).ok()?;
                    if eligible_shape(context, ty) && seen.insert(ty.clone()) {
                        nir::type_computations::charge_term(subject, &mut budget.steps).ok()?;
                        alternatives.push(Formula::equal(subject.clone(), ty.clone()));
                    }
                }
            }
            Formula::And(parts) | Formula::Or(parts) => {
                budget.steps = budget.steps.checked_sub(parts.len())?;
                pending.extend(parts);
            }
            Formula::Not(inner) => pending.push(inner),
            _ => {}
        }
    }
    if alternatives.is_empty() {
        return None;
    }
    match context.prove_metered(Formula::Or(alternatives), budget) {
        Outcome::Entailed(proof) => Some(proof),
        _ => None,
    }
}

fn eligible_alternatives(context: &Context, subject: &Term, formula: &Formula) -> bool {
    matches!(formula, Formula::Or(parts) if !parts.is_empty() && parts.iter().all(|part|
        matches!(part, Formula::Equal(crate::ptc::Equality(original, shape))
            if original == subject && eligible_shape(context, shape))))
}

/// Equality is queried only for Type domains, with the caller's shared budget.
pub(crate) fn equivalent_domains(
    context: &Context,
    left: &Term,
    right: &Term,
    max_depth: usize,
    fuel: &mut usize,
) -> bool {
    if left.sort() != nir::Sort::Type || right.sort() != nir::Sort::Type {
        return false;
    }
    if nir::type_computations::charge_term(left, fuel).is_err()
        || nir::type_computations::charge_term(right, fuel).is_err()
    {
        return false;
    }
    let mut budget = Budget {
        steps: *fuel,
        max_depth,
    };
    let result = context.prove_metered(
        crate::ptc::Formula::equal(left.clone(), right.clone()),
        &mut budget,
    );
    *fuel = budget.steps;
    matches!(result, Outcome::Entailed(_))
}

#[derive(Clone, Debug)]
pub struct TypeEvidence {
    pub implementation: DeclarationId,
    pub type_arguments: Vec<Term>,
    pub head_proofs: Vec<Proof>,
    pub requirement_proofs: Vec<Proof>,
    pub eligibility_proof: Option<Proof>,
}
/// A matched head is not an applicable implementation. Its instantiated
/// requirements remain explicit obligations; no implicit conversion publishes
/// its partial proof as TypeEvidence.
#[derive(Clone, Debug)]
pub struct HeadMatch {
    evidence: TypeEvidence,
    requirements: Vec<TypePredicate>,
    requirement_budget: Budget,
}
impl HeadMatch {
    pub fn type_arguments(&self) -> &[Term] {
        &self.evidence.type_arguments
    }
    pub fn requirements(&self) -> &[TypePredicate] {
        &self.requirements
    }
}
impl fmt::Display for HeadMatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(implementation-head-match (head-evidence {}) (pending-requirements",
            self.evidence
        )?;
        for requirement in &self.requirements {
            write!(f, " {requirement}")?;
        }
        f.write_str("))")
    }
}
impl ImplementationHead {
    /// Replay head matching only. Success does not discharge the retained
    /// requirements or authorize selection/dispatch of this implementation.
    pub fn verify_head_match(
        &self,
        matched: &HeadMatch,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
    ) -> Result<(), crate::ptc::ProofError> {
        self.verify_head_match_with_domains(
            matched,
            subject,
            interface_arguments,
            context,
            budget,
            &BTreeMap::new(),
        )
    }

    /// Replay only head matching in the original declaration-domain environment.
    /// Retained requirements remain obligations, even when they are false.
    pub fn verify_head_match_with_domains(
        &self,
        matched: &HeadMatch,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
        domains: &BTreeMap<DeclarationId, Term>,
    ) -> Result<(), crate::ptc::ProofError> {
        self.verify_head_match_with_layouts(
            matched,
            subject,
            interface_arguments,
            context,
            budget,
            domains,
            None,
        )
    }

    pub fn verify_head_match_with_layouts(
        &self,
        matched: &HeadMatch,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
        domains: &BTreeMap<DeclarationId, Term>,
        layouts: Option<&nir::program::Package>,
    ) -> Result<(), crate::ptc::ProofError> {
        use crate::ptc::ProofError;
        let remaining = validate(self, subject, interface_arguments, budget)
            .map_err(|_| ProofError::BudgetExhausted)?;
        let remaining = validate(
            self,
            subject,
            &matched.evidence.type_arguments,
            Budget {
                steps: remaining,
                ..budget
            },
        )
        .map_err(|_| ProofError::BudgetExhausted)?;
        if self.interface.is_none() || !matched.evidence.requirement_proofs.is_empty() {
            return Err(ProofError::Invalid);
        }
        let mut remaining = remaining;
        let substitution = nir::DeclarationSubstitution::bounded(
            &self.type_parameters,
            &self.parameter_types,
            &matched.evidence.type_arguments,
            &[],
            &mut remaining,
        )
        .map_err(substitution_proof_error)?;
        let expected = self
            .requirements
            .iter()
            .map(|requirement| {
                substitution.instantiate_predicate_bounded(requirement, &mut remaining)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(substitution_proof_error)?;
        if matched.requirements != expected {
            return Err(ProofError::Invalid);
        }
        let head = ImplementationHead {
            parameter_types: self.parameter_types.clone(),
            declaration: self.declaration,
            interface: self.interface,
            interface_arguments: self.interface_arguments.clone(),
            subject: self.subject.clone(),
            type_parameters: self.type_parameters.clone(),
            requirements: vec![],
        };
        head.verify_type_evidence_with_layouts(
            &matched.evidence,
            subject,
            interface_arguments,
            context,
            Budget {
                steps: remaining,
                ..budget
            },
            domains,
            layouts,
        )
    }
}
#[derive(Clone, Debug)]
pub enum TypeAssessment {
    Applicable(TypeEvidence),
    /// A refutation, not merely a countermodel to the positive requirement.
    Excluded(Proof),
    /// Distinct fixed scalar indices cannot denote the same interface instance.
    /// This is a checked identity mismatch, not a program-value PTC equation.
    ValueIndexMismatch {
        position: usize,
        expected: Term,
        actual: Term,
    },
    ForbiddenSubject(Term),
    Pending(String),
    Invalid(String),
}
impl TypeAssessment {
    pub fn applicability(&self) -> Applicability {
        match self {
            Self::Applicable(_) => Applicability::Applicable,
            Self::Excluded(_) | Self::ValueIndexMismatch { .. } | Self::ForbiddenSubject(_) => {
                Applicability::Excluded
            }
            Self::Pending(_) | Self::Invalid(_) => Applicability::Pending,
        }
    }
}
#[derive(Clone, Debug)]
pub struct TypeResolution {
    pub(super) index_identity: u64,
    pub(super) index_revision: u64,
    pub selection: Selection,
    pub assessments: BTreeMap<DeclarationId, TypeAssessment>,
}

// Bound the entire borrowed input before recursive substitution or cloning.
pub(super) fn validate(
    head: &ImplementationHead,
    subject: &Term,
    arguments: &[Term],
    budget: Budget,
) -> Result<usize, String> {
    enum Work<'a> {
        Term(&'a Term),
        Predicate(&'a TypePredicate),
    }
    let mut fuel = budget.steps;
    if arguments
        .len()
        .saturating_add(head.interface_arguments.len())
        .saturating_add(head.requirements.len())
        .saturating_add(head.parameter_types.len())
        .saturating_add(2)
        > fuel
    {
        return Err("input budget exhausted".into());
    }
    let mut pending = vec![(Work::Term(subject), 0), (Work::Term(&head.subject), 0)];
    pending.extend(
        arguments
            .iter()
            .chain(&head.interface_arguments)
            .chain(&head.parameter_types)
            .map(|t| (Work::Term(t), 0)),
    );
    pending.extend(head.requirements.iter().map(|p| (Work::Predicate(p), 0)));
    while let Some((work, depth)) = pending.pop() {
        if fuel == 0 || depth >= budget.max_depth.min(128) {
            return Err("input budget exhausted".into());
        }
        fuel -= 1;
        match work {
            Work::Term(Term::AssociatedType {
                arguments, subject, ..
            }) => {
                if arguments.len().saturating_add(1) > fuel {
                    return Err("input budget exhausted".into());
                }
                pending.push((Work::Term(subject), depth + 1));
                pending.extend(arguments.iter().map(|t| (Work::Term(t), depth + 1)));
            }
            Work::Term(Term::OptionalValue { ty, value }) => {
                pending.push((Work::Term(ty), depth + 1));
                if let Some(value) = value {
                    pending.push((Work::Term(value), depth + 1));
                }
            }
            Work::Term(Term::ErrorValue { ty, label, value }) => {
                pending.push((Work::Term(ty), depth + 1));
                if let Some(label) = label {
                    pending.push((Work::Term(label), depth + 1));
                }
                pending.push((Work::Term(value), depth + 1));
            }
            Work::Term(Term::NominalValue { ty, fields, .. }) => {
                if fields.len().saturating_add(1) > fuel {
                    return Err("input budget exhausted".into());
                }
                pending.push((Work::Term(ty), depth + 1));
                pending.extend(fields.iter().map(|field| (Work::Term(field), depth + 1)));
            }
            Work::Term(Term::Record(fields) | Term::RecordValue(fields)) => {
                if fields.len() > fuel {
                    return Err("input budget exhausted".into());
                }
                for (name, ty) in fields {
                    fuel = fuel
                        .checked_sub(name.len())
                        .ok_or("input budget exhausted")?;
                    pending.push((Work::Term(ty), depth + 1));
                }
            }
            Work::Term(
                Term::Tuple(items)
                | Term::ClosureContextValue(items)
                | Term::TypeListValue(items)
                | Term::TypeListConcat(items)
                | Term::TupleValue(items)
                | Term::Nominal {
                    arguments: items, ..
                }
                | Term::NormalForm {
                    arguments: items, ..
                },
            ) => {
                if items.len() > fuel {
                    return Err("input budget exhausted".into());
                }
                pending.extend(items.iter().map(|t| (Work::Term(t), depth + 1)));
            }
            Work::Term(
                Term::Optional(t)
                | Term::Pointer(t)
                | Term::ComptimeFunction(t)
                | Term::NormalFormFunction(t, _)
                | Term::UnsafeFunction(t),
            ) => pending.push((Work::Term(t), depth + 1)),
            Work::Term(Term::Symbol(name) | Term::String(name)) => {
                fuel = fuel
                    .checked_sub(name.len())
                    .ok_or("input budget exhausted")?;
            }
            Work::Term(Term::Function { parameters, result }) => {
                if parameters.len() > fuel {
                    return Err("input budget exhausted".into());
                }
                pending.extend(parameters.iter().map(|p| (Work::Term(&p.ty), depth + 1)));
                pending.push((Work::Term(result), depth + 1));
            }
            Work::Term(Term::ErrorResult { errors, result }) => {
                if errors.len().saturating_add(1) > fuel {
                    return Err("input budget exhausted".into());
                }
                pending.push((Work::Term(result), depth + 1));
                pending.extend(errors.iter().map(|ty| (Work::Term(ty), depth + 1)));
            }
            Work::Term(Term::Computation { effects, result }) => {
                fuel = fuel
                    .checked_sub(effects.len())
                    .ok_or("input budget exhausted")?;
                let width = effects
                    .iter()
                    .try_fold(1usize, |sum, op| sum.checked_add(op.arguments.len()))
                    .ok_or("input budget exhausted")?;
                if width > fuel {
                    return Err("input budget exhausted".into());
                }
                pending.push((Work::Term(result), depth + 1));
                for operation in effects {
                    pending.extend(
                        operation
                            .arguments
                            .iter()
                            .map(|t| (Work::Term(t), depth + 1)),
                    );
                }
            }
            Work::Term(Term::ContextualClosure { context, signature }) => {
                if fuel < 2 {
                    return Err("input budget exhausted".into());
                }
                pending.push((Work::Term(context), depth + 1));
                pending.push((Work::Term(signature), depth + 1));
            }
            Work::Term(Term::Closure {
                environment,
                signature,
            }) => {
                if environment.len() > fuel {
                    return Err("input budget exhausted".into());
                }
                pending.extend(environment.iter().map(|t| (Work::Term(t), depth + 1)));
                pending.push((Work::Term(signature), depth + 1));
            }
            Work::Predicate(TypePredicate::Trait(requirement)) => {
                if requirement.interface_arguments.len() > fuel {
                    return Err("input budget exhausted".into());
                }
                pending.push((Work::Term(&requirement.subject), depth + 1));
                pending.extend(
                    requirement
                        .interface_arguments
                        .iter()
                        .map(|term| (Work::Term(term), depth + 1)),
                );
            }
            Work::Predicate(TypePredicate::Equal(a, b)) => {
                pending.push((Work::Term(a), depth + 1));
                pending.push((Work::Term(b), depth + 1));
            }
            Work::Predicate(TypePredicate::Not(p)) => pending.push((Work::Predicate(p), depth + 1)),
            Work::Predicate(TypePredicate::And(items) | TypePredicate::Or(items)) => {
                if items.len() > fuel {
                    return Err("input budget exhausted".into());
                }
                pending.extend(items.iter().map(|p| (Work::Predicate(p), depth + 1)));
            }
            _ => {}
        }
    }
    Ok(fuel)
}
fn unresolved(formula: Formula, context: &Context, budget: Budget) -> TypeAssessment {
    // NotEntailed supplies one countermodel; only proof of the negation rules
    // the candidate out for every assignment allowed by this caller scope.
    match context.prove(formula.negate(), budget) {
        Outcome::Entailed(proof) => TypeAssessment::Excluded(proof),
        Outcome::IllFormed(reason) => TypeAssessment::Invalid(reason),
        _ => TypeAssessment::Pending("type relation has not been proved or refuted".into()),
    }
}
impl ImplementationHead {
    pub fn match_types(
        &self,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
    ) -> Result<HeadMatch, TypeAssessment> {
        if self.interface.is_none() {
            return Err(TypeAssessment::Pending("open trait head".into()));
        }
        self.match_subject_types(subject, interface_arguments, context, budget)
    }
    pub fn match_types_with_domains(
        &self,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
        domains: &BTreeMap<DeclarationId, Term>,
    ) -> Result<HeadMatch, TypeAssessment> {
        self.match_types_with_layouts(subject, interface_arguments, context, budget, domains, None)
    }

    pub fn match_types_with_layouts(
        &self,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
        domains: &BTreeMap<DeclarationId, Term>,
        layouts: Option<&nir::program::Package>,
    ) -> Result<HeadMatch, TypeAssessment> {
        if self.interface.is_none() {
            return Err(TypeAssessment::Pending("open trait head".into()));
        }
        self.match_subject_types_with_domains(
            subject,
            interface_arguments,
            context,
            budget,
            domains,
            layouts,
        )
    }
    pub fn assess_types(
        &self,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
    ) -> TypeAssessment {
        if self.interface.is_none() {
            return TypeAssessment::Pending("open trait head".into());
        }
        self.assess_subject_types(subject, interface_arguments, context, budget)
    }
    // Called only after the source domain (fixed trait or inherent member)
    // has been selected. An open trait does not gain inherent semantics.
    pub(super) fn match_subject_types(
        &self,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
    ) -> Result<HeadMatch, TypeAssessment> {
        self.match_subject_types_with_domains(
            subject,
            interface_arguments,
            context,
            budget,
            &BTreeMap::new(),
            None,
        )
    }
    pub(super) fn match_subject_types_with_domains(
        &self,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
        domains: &BTreeMap<DeclarationId, Term>,
        layouts: Option<&nir::program::Package>,
    ) -> Result<HeadMatch, TypeAssessment> {
        let remaining = match validate(self, subject, interface_arguments, budget) {
            Ok(remaining) => remaining,
            Err(reason) => return Err(TypeAssessment::Pending(reason)),
        };
        if self.interface_arguments.len() != interface_arguments.len() {
            return Err(TypeAssessment::Invalid(
                "interface argument count mismatch".into(),
            ));
        }
        if self.type_parameters.len() > budget.steps {
            return Err(TypeAssessment::Pending("parameter budget exhausted".into()));
        }
        if self.type_parameters.iter().collect::<BTreeSet<_>>().len() != self.type_parameters.len()
        {
            return Err(TypeAssessment::Invalid(
                "duplicate implementation parameter".into(),
            ));
        }
        if subject.sort() != Sort::Type || self.subject.sort() != Sort::Type {
            return Err(TypeAssessment::Invalid(
                "implementation subject must be a Type".into(),
            ));
        }
        for term in std::iter::once(subject)
            .chain(interface_arguments)
            .chain(std::iter::once(&self.subject))
            .chain(&self.interface_arguments)
        {
            if term.sort() != Sort::Type
                && !term.is_symbolic_index_value()
                && !matches!(
                    term,
                    Term::Rigid {
                        sort: Sort::ComptimeValue,
                        ..
                    } | Term::RecordValue(_)
                )
            {
                return Err(TypeAssessment::Pending(
                    "non-Type interface argument matching is not implemented".into(),
                ));
            }
            if let Err(error) = term.check_scope(&[]) {
                return Err(TypeAssessment::Invalid(error.to_string()));
            }
        }
        let allowance =
            remaining / (8 + self.interface_arguments.len() * 4 + self.requirements.len() * 2);
        if allowance == 0 {
            return Err(TypeAssessment::Pending("candidate budget exhausted".into()));
        }
        let mut budget = Budget {
            steps: allowance,
            max_depth: budget.max_depth,
        };
        // Known outer structure can refute a candidate before all of its
        // parameters are inferred: Tuple versus a known non-tuple constructor, or an explicit error
        // prefix versus an ordinary type. Require a replayable PTC proof;
        // grant neither injectivity nor disjointness to opaque factories.
        let nominal = |ty: &Term| {
            matches!(ty,
            Term::Nominal { declaration, .. } if context.is_nominal_constructor(*declaration))
        };
        let ordinary = |ty: &Term| {
            nominal(ty)
                || context.is_tuple_type(ty)
                || context.is_list_type(ty)
                || matches!(
                    ty,
                    Term::Primitive(_)
                        | Term::Tuple(_)
                        | Term::Record(_)
                        | Term::Optional(_)
                        | Term::Pointer(_)
                        | Term::Function { .. }
                        | Term::ComptimeFunction(_)
                        | Term::NormalFormFunction(..)
                        | Term::UnsafeFunction(_)
                )
        };
        let non_tuple =
            |ty: &Term| ordinary(ty) && !context.is_tuple_type(ty) && !matches!(ty, Term::Tuple(_));
        let non_list = |ty: &Term| ordinary(ty) && !context.is_list_type(ty);
        if (context.is_list_type(subject) && non_list(&self.subject))
            || (context.is_list_type(&self.subject) && non_list(subject))
            || (context.is_tuple_type(subject) && non_tuple(&self.subject))
            || (context.is_tuple_type(&self.subject) && non_tuple(subject))
            || (matches!(&self.subject, Term::ErrorResult { .. }) && ordinary(subject))
            || (matches!(subject, Term::ErrorResult { .. }) && ordinary(&self.subject))
        {
            if let Outcome::Entailed(proof) = context.prove_metered(
                Formula::equal(self.subject.clone(), subject.clone()).negate(),
                &mut budget,
            ) {
                return Err(TypeAssessment::Excluded(proof));
            }
        }
        let (shape, eligibility_proof) = match subject {
            subject if context.is_tuple_type(subject) || context.is_list_type(subject) => {
                (subject.clone(), None)
            }
            Term::Rigid { .. } | Term::NormalForm { .. } | Term::AssociatedType { .. } => {
                match context
                    .structural_view_with_heads_metered(subject, budget, |head| {
                        context.is_tuple_head(head) || context.is_list_inference_head(head)
                    })
                    .0
                {
                    Ok(Some((shape, proof))) => (shape, Some(proof)),
                    _ => match prove_eligible_alternatives(context, subject, &mut budget) {
                        Some(proof) => (subject.clone(), Some(proof)),
                        None => {
                            return Err(TypeAssessment::Pending(
                                "user implementation subject eligibility is unresolved".into(),
                            ));
                        }
                    },
                }
            }
            other => (other.clone(), None),
        };
        if matches!(shape, Term::Closure { .. } | Term::ContextualClosure { .. }) {
            return Err(TypeAssessment::ForbiddenSubject(shape));
        }
        if !eligible_shape(context, &shape)
            && !eligibility_proof
                .as_ref()
                .is_some_and(|proof| eligible_alternatives(context, subject, &proof.goal))
        {
            return Err(TypeAssessment::Pending(
                "user implementation subject eligibility is unresolved".into(),
            ));
        }
        // A symbolic union can exclude structurally incompatible finer heads
        // (for example *U against f16 | bf16) without inferring their inputs.
        if eligibility_proof
            .as_ref()
            .is_some_and(|proof| eligible_alternatives(context, subject, &proof.goal))
            && eligible_shape(context, &self.subject)
        {
            if let Outcome::Entailed(proof) = context.prove_metered(
                Formula::equal(self.subject.clone(), subject.clone()).negate(),
                &mut budget,
            ) {
                return Err(TypeAssessment::Excluded(proof));
            }
        }
        let parameters = self
            .parameter_binders()
            .map_err(|e| TypeAssessment::Invalid(e.to_string()))?;
        let mut root = Engine::with_context(context.fork());
        let types: Vec<_> = parameters
            .iter()
            .map(|parameter| I::Meta(root.fresh(parameter.sort)))
            .collect();
        let instantiate = |term: &Term| -> Result<I, nir::TypeError> {
            Ok(I::Instantiate {
                template: term.abstract_parameters(&parameters)?,
                arguments: types.clone(),
            })
        };
        let mut relations = Vec::new();
        let template = match instantiate(&self.subject) {
            Ok(t) => t,
            Err(e) => return Err(TypeAssessment::Invalid(e.to_string())),
        };
        relations.push((template.clone(), I::Stable(subject.clone())));
        if shape != *subject {
            relations.push((template, I::Stable(shape)));
        }
        // Fixed value conflicts refute the entire head, even if another input
        // has an unresolved symbolic relation. Inspect them before inference
        // can return Pending for the first unknown field or argument.
        let mut value_fuel = budget.steps;
        for (position, (template, actual)) in self
            .interface_arguments
            .iter()
            .zip(interface_arguments)
            .enumerate()
        {
            if template.sort() != Sort::ComptimeValue || actual.sort() != Sort::ComptimeValue {
                continue;
            }
            if template.index_values_definitely_differ(actual, &mut value_fuel) {
                return Err(TypeAssessment::ValueIndexMismatch {
                    position,
                    expected: template.clone(),
                    actual: actual.clone(),
                });
            }
            if value_fuel == 0 {
                return Err(TypeAssessment::Pending(
                    "value conflict inspection budget exhausted".into(),
                ));
            }
        }
        for (position, (template, actual)) in self
            .interface_arguments
            .iter()
            .zip(interface_arguments)
            .enumerate()
        {
            if template.sort() == Sort::ComptimeValue
                && (actual.is_symbolic_index_value()
                    || matches!(actual, Term::Rigid { declaration, sort: Sort::ComptimeValue } if domains.contains_key(declaration)))
            {
                // Value skeletons and binders are matched outside PTC. Only
                // Type-valued leaves create type obligations; replay extracts
                // those same leaves after the whole substitution is complete.
                let mut pending = vec![(template, actual)];
                let mut fuel = budget.steps;
                while let Some((template, actual)) = pending.pop() {
                    fuel = fuel.checked_sub(1).ok_or_else(|| {
                        TypeAssessment::Pending("value index matching budget exhausted".into())
                    })?;
                    if template.sort() == Sort::Type && actual.sort() == Sort::Type {
                        relations.push((
                            instantiate(template)
                                .map_err(|e| TypeAssessment::Invalid(e.to_string()))?,
                            I::Stable(actual.clone()),
                        ));
                        continue;
                    }
                    if let Term::Rigid {
                        declaration,
                        sort: Sort::ComptimeValue,
                    } = template
                    {
                        fuel = fuel.checked_sub(parameters.len()).ok_or_else(|| {
                            TypeAssessment::Pending("value binder lookup budget exhausted".into())
                        })?;
                        if let Some(input) = parameters.iter().position(|p| {
                            p.declaration == *declaration && p.sort == Sort::ComptimeValue
                        }) {
                            let bound = match root.freeze(&types[input]) {
                                Ok(bound) => bound,
                                Err(_) => {
                                    let I::Meta(variable) = types[input] else {
                                        unreachable!()
                                    };
                                    root.assign(variable, I::Stable(actual.clone())).map_err(
                                        |reason| {
                                            TypeAssessment::Pending(format!(
                                                "value index inference: {reason:?}"
                                            ))
                                        },
                                    )?;
                                    actual.clone()
                                }
                            };
                            if bound == *actual
                                && matches!(
                                    actual,
                                    Term::Rigid {
                                        sort: Sort::ComptimeValue,
                                        ..
                                    }
                                )
                            {
                                continue;
                            }
                            // Either side can be the caller's unknown value:
                            // repeated binder matching must not depend on input order.
                            if matches!(
                                actual,
                                Term::Rigid {
                                    sort: Sort::ComptimeValue,
                                    ..
                                }
                            ) || matches!(
                                bound,
                                Term::Rigid {
                                    sort: Sort::ComptimeValue,
                                    ..
                                }
                            ) {
                                return Err(TypeAssessment::Pending(
                                    "rigid value index identity is unresolved".into(),
                                ));
                            }
                            let Some(type_relations) = bound.index_type_relations(actual) else {
                                if (!bound.is_structural_index_value()
                                    || !actual.is_structural_index_value())
                                    && !bound.index_values_definitely_differ(actual, &mut fuel)
                                {
                                    return Err(TypeAssessment::Pending(
                                        "symbolic record index identity is unresolved".into(),
                                    ));
                                }
                                return Err(TypeAssessment::ValueIndexMismatch {
                                    position,
                                    expected: bound,
                                    actual: actual.clone(),
                                });
                            };
                            relations.extend(type_relations.into_iter().map(|(left, right)| {
                                (I::Stable(left.clone()), I::Stable(right.clone()))
                            }));
                            continue;
                        }
                    }
                    if let (
                        Term::NominalValue {
                            ty: left_ty,
                            variant: left_variant,
                            fields: left,
                        },
                        Term::NominalValue {
                            ty: right_ty,
                            variant: right_variant,
                            fields: right,
                        },
                    ) = (template, actual)
                    {
                        if left_variant == right_variant && left.len() == right.len() {
                            if left.len().saturating_add(1) > fuel.saturating_sub(pending.len()) {
                                return Err(TypeAssessment::Pending(
                                    "value index matching budget exhausted".into(),
                                ));
                            }
                            pending.push((left_ty, right_ty));
                            pending.extend(left.iter().zip(right).rev());
                            continue;
                        }
                    }
                    if template.has_pending_error_branch() || actual.has_pending_error_branch() {
                        if template == actual {
                            continue;
                        }
                        return Err(TypeAssessment::Pending(
                            "error value branch awaits a completed inner result".into(),
                        ));
                    }
                    if let (
                        Term::OptionalValue { ty: a, value: av },
                        Term::OptionalValue { ty: b, value: bv },
                    ) = (template, actual)
                    {
                        if av.is_some() == bv.is_some() {
                            pending.push((a, b));
                            if let (Some(a), Some(b)) = (av, bv) {
                                pending.push((a, b));
                            }
                            continue;
                        }
                    }
                    if let (
                        Term::ErrorValue {
                            ty: a,
                            label: al,
                            value: av,
                        },
                        Term::ErrorValue {
                            ty: b,
                            label: bl,
                            value: bv,
                        },
                    ) = (template, actual)
                    {
                        if al.is_some() == bl.is_some() {
                            pending.push((a, b));
                            if let (Some(a), Some(b)) = (al, bl) {
                                pending.push((a, b));
                            }
                            pending.push((av, bv));
                            continue;
                        }
                    }
                    if let (Term::TupleValue(left), Term::TupleValue(right)) = (template, actual) {
                        if left.len() == right.len() {
                            if left.len() > fuel.saturating_sub(pending.len()) {
                                return Err(TypeAssessment::Pending(
                                    "value index matching budget exhausted".into(),
                                ));
                            }
                            pending.extend(left.iter().zip(right).rev());
                            continue;
                        }
                    }
                    if let (Term::RecordValue(left), Term::RecordValue(right)) = (template, actual)
                    {
                        if left.len() == right.len()
                            && left.iter().zip(right).all(|((a, _), (b, _))| a == b)
                        {
                            if left.len() > fuel.saturating_sub(pending.len()) {
                                return Err(TypeAssessment::Pending(
                                    "value index matching budget exhausted".into(),
                                ));
                            }
                            pending.extend(
                                left.iter().zip(right).rev().map(|((_, a), (_, b))| (a, b)),
                            );
                            continue;
                        }
                    }
                    if template.scalar_index_domain().is_some() && template == actual {
                        continue;
                    }
                    if template.sort() == Sort::ComptimeValue
                        && template.scalar_index_domain().is_none()
                        && !matches!(
                            template,
                            Term::OptionalValue { .. }
                                | Term::ErrorValue { .. }
                                | Term::RecordValue(_)
                                | Term::TupleValue(_)
                                | Term::NominalValue { .. }
                        )
                    {
                        return Err(TypeAssessment::Pending(
                            "value index identity is unresolved".into(),
                        ));
                    }
                    if matches!(
                        actual,
                        Term::Rigid {
                            sort: Sort::ComptimeValue,
                            ..
                        }
                    ) {
                        return Err(TypeAssessment::Pending(
                            "rigid value index identity is unresolved".into(),
                        ));
                    }
                    return Err(TypeAssessment::ValueIndexMismatch {
                        position,
                        expected: template.clone(),
                        actual: actual.clone(),
                    });
                }
                continue;
            }
            if template.sort() != Sort::Type || actual.sort() != Sort::Type {
                return Err(TypeAssessment::Invalid(
                    "interface index domain mismatch".into(),
                ));
            }
            let template = match instantiate(template) {
                Ok(t) => t,
                Err(e) => return Err(TypeAssessment::Invalid(e.to_string())),
            };
            relations.push((template, I::Stable(actual.clone())));
        }
        // Pool input observation, alias resolution, matching, dependency scanning
        // and freezing with one proof slot per relation. These slots remain
        // within the existing head partition and do not enlarge its total.
        let mut inference_budget = Budget {
            steps: budget
                .steps
                .saturating_mul(relations.len().saturating_add(5))
                .min(remaining),
            ..budget
        };
        let (mut candidate, ids) = match root.match_many_metered(&relations, &mut inference_budget)
        {
            Ok(candidate) => candidate,
            Err(reason) => {
                return Err(TypeAssessment::Pending(format!(
                    "head inference: {reason:?}"
                )));
            }
        };
        candidate.advance_metered(ids.len(), &mut inference_budget);
        let mut proofs = Vec::new();
        let mut pending = false;
        for id in ids {
            match candidate.state(id) {
                Some(State::Solved(Outcome::Entailed(proof))) => proofs.push(proof.clone()),
                Some(State::Solved(Outcome::NotEntailed(witness))) => {
                    let assessment = unresolved(witness.goal.clone(), context, budget);
                    if matches!(
                        assessment,
                        TypeAssessment::Excluded(_) | TypeAssessment::Invalid(_)
                    ) {
                        return Err(assessment);
                    }
                    pending = true;
                }
                Some(State::Solved(Outcome::Unknown { residual, .. })) => {
                    let assessment = unresolved(residual.clone(), context, budget);
                    if matches!(
                        assessment,
                        TypeAssessment::Excluded(_) | TypeAssessment::Invalid(_)
                    ) {
                        return Err(assessment);
                    }
                    pending = true;
                }
                Some(
                    State::Solved(Outcome::IllFormed(reason)) | State::InvalidInference(reason),
                ) => return Err(TypeAssessment::Invalid(reason.clone())),
                _ => pending = true,
            }
        }
        if pending {
            return Err(TypeAssessment::Pending(
                "head obligations are unsettled".into(),
            ));
        }
        let arguments = match types
            .iter()
            .map(|ty| candidate.freeze(ty))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(args) => args,
            Err(_) => {
                return Err(TypeAssessment::Pending(
                    "implementation parameters remain unresolved".into(),
                ));
            }
        };
        let mut domain_fuel = budget.steps;
        validate_head_arguments_with_domains(
            &parameters,
            &self.parameter_types,
            &arguments,
            domains,
            Some((context, budget.max_depth)),
            layouts,
            &mut domain_fuel,
        )
        .map_err(|error| match error {
            nir::NominalAdmissionError::Budget => TypeAssessment::Pending(error.to_string()),
            nir::NominalAdmissionError::Invalid(message) => TypeAssessment::Invalid(message),
        })?;
        let substitution = nir::DeclarationSubstitution::bounded(
            &self.type_parameters,
            &self.parameter_types,
            &arguments,
            &[],
            &mut domain_fuel,
        )
        .map_err(|error| match error {
            nir::type_computations::Error::Budget => TypeAssessment::Pending(error.to_string()),
            error => TypeAssessment::Invalid(error.to_string()),
        })?;
        let requirements = match self
            .requirements
            .iter()
            .map(|requirement| {
                substitution.instantiate_predicate_bounded(requirement, &mut domain_fuel)
            })
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(requirements) => requirements,
            Err(nir::type_computations::Error::Budget) => {
                return Err(TypeAssessment::Pending(
                    "requirement substitution budget exhausted".into(),
                ));
            }
            Err(error) => return Err(TypeAssessment::Invalid(error.to_string())),
        };
        // Head inference has its own metered partition. Return its unused work
        // to the positive/negative requirement queries instead of discarding it
        // at each recursive implementation boundary. Other head operations keep
        // their reserved slots; the total candidate allowance does not grow.
        let mut requirement_budget = budget;
        let requirement_queries = requirements.len().saturating_mul(2);
        if requirement_queries > 0 {
            requirement_budget.steps = requirement_budget
                .steps
                .saturating_add(inference_budget.steps / requirement_queries);
        }
        Ok(HeadMatch {
            evidence: TypeEvidence {
                implementation: self.declaration,
                type_arguments: arguments,
                head_proofs: proofs,
                requirement_proofs: vec![],
                eligibility_proof,
            },
            requirements,
            requirement_budget,
        })
    }
    pub(super) fn assess_subject_types(
        &self,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
    ) -> TypeAssessment {
        let matched = match self.match_subject_types(subject, interface_arguments, context, budget)
        {
            Ok(matched) => matched,
            Err(assessment) => return assessment,
        };
        matched.discharge_with(|goal, allowance| context.prove(goal, allowance))
    }
}
impl HeadMatch {
    pub(crate) fn discharge_with(
        self,
        mut prove: impl FnMut(Formula, Budget) -> Outcome,
    ) -> TypeAssessment {
        let per_query = self.requirement_budget;
        self.discharge_metered_with(|goal, allowance| {
            let steps = allowance.steps.min(per_query.steps);
            allowance.steps -= steps;
            prove(goal, Budget { steps, ..per_query })
        })
    }

    pub(crate) fn discharge_metered_with(
        self,
        mut prove: impl FnMut(Formula, &mut Budget) -> Outcome,
    ) -> TypeAssessment {
        let HeadMatch {
            evidence,
            requirements: pending_requirements,
            requirement_budget: budget,
        } = self;
        let TypeEvidence {
            implementation,
            type_arguments: arguments,
            head_proofs: proofs,
            eligibility_proof,
            ..
        } = evidence;
        let mut requirements = Vec::new();
        let mut pending = false;
        // The head reserved two query slots per requirement. Positive proofs
        // usually settle the requirement alone; reuse their unspent work rather
        // than stranding it in negative queries that will never execute.
        let Some(mut remaining) = budget
            .steps
            .checked_mul(pending_requirements.len())
            .and_then(|steps| steps.checked_mul(2))
        else {
            return TypeAssessment::Pending("requirement budget overflow".into());
        };
        for (index, requirement) in pending_requirements.iter().enumerate() {
            let requirement = Formula::from_predicate(requirement);
            let reserved = remaining / (pending_requirements.len() - index);
            let mut allowance = Budget {
                steps: reserved,
                ..budget
            };
            let outcome = prove(requirement.clone(), &mut allowance);
            if allowance.steps > reserved {
                return TypeAssessment::Invalid("requirement proof increased its budget".into());
            }
            remaining -= reserved - allowance.steps;
            match outcome {
                Outcome::Entailed(proof) => requirements.push(proof),
                Outcome::IllFormed(reason) => return TypeAssessment::Invalid(reason),
                _ => {
                    let before = allowance.steps;
                    let outcome = prove(requirement.negate(), &mut allowance);
                    if allowance.steps > before {
                        return TypeAssessment::Invalid(
                            "requirement proof increased its budget".into(),
                        );
                    }
                    remaining -= before - allowance.steps;
                    match outcome {
                        Outcome::Entailed(proof) => return TypeAssessment::Excluded(proof),
                        Outcome::IllFormed(reason) => return TypeAssessment::Invalid(reason),
                        _ => pending = true,
                    }
                }
            }
        }
        if pending {
            return TypeAssessment::Pending("implementation requirements are unsettled".into());
        }
        TypeAssessment::Applicable(TypeEvidence {
            implementation,
            type_arguments: arguments,
            head_proofs: proofs,
            requirement_proofs: requirements,
            eligibility_proof,
        })
    }
}
impl Index {
    pub fn type_resolution_is_current(&self, resolution: &TypeResolution) -> bool {
        resolution.index_identity == self.identity && resolution.index_revision == self.revision
    }
    pub fn resolve_types(
        &self,
        interface: DeclarationId,
        arguments: &[Term],
        subject: &Term,
        context: &Context,
        budget: Budget,
    ) -> Result<TypeResolution, Error> {
        self.resolve_types_with(interface, arguments, subject, context, budget, |_, _| {
            super::Applicability::Applicable
        })
    }
    pub(crate) fn resolve_types_with(
        &self,
        interface: DeclarationId,
        arguments: &[Term],
        subject: &Term,
        context: &Context,
        budget: Budget,
        mut qualify: impl FnMut(&TypeEvidence, Budget) -> super::Applicability,
    ) -> Result<TypeResolution, Error> {
        self.resolve_assessed_with(interface, arguments, subject, budget, |head, allowance| {
            let assessment = match head.match_types_with_layouts(
                subject,
                arguments,
                context,
                allowance,
                &self.parameter_domains,
                Some(&self.nominal_layouts),
            ) {
                Ok(matched) => matched.discharge_with(|goal, budget| context.prove(goal, budget)),
                Err(assessment) => assessment,
            };
            let state = match &assessment {
                TypeAssessment::Applicable(evidence) => qualify(evidence, allowance),
                _ => assessment.applicability(),
            };
            (assessment, state)
        })
    }
    pub(crate) fn resolve_assessed_with(
        &self,
        interface: DeclarationId,
        arguments: &[Term],
        subject: &Term,
        budget: Budget,
        assess: impl FnMut(&ImplementationHead, Budget) -> (TypeAssessment, Applicability),
    ) -> Result<TypeResolution, Error> {
        self.resolve_assessed_with_search_budget(
            interface,
            arguments,
            subject,
            &mut { budget },
            assess,
        )
    }
    pub(crate) fn resolve_assessed_with_search_budget(
        &self,
        interface: DeclarationId,
        arguments: &[Term],
        subject: &Term,
        budget: &mut Budget,
        mut assess: impl FnMut(&ImplementationHead, Budget) -> (TypeAssessment, Applicability),
    ) -> Result<TypeResolution, Error> {
        let mut search_budget = Budget {
            steps: budget.steps / 2,
            max_depth: budget.max_depth,
        };
        let reserved = search_budget.steps;
        let searched =
            self.candidates_metered(Some(interface), arguments, subject, &mut search_budget);
        budget.steps = budget.steps.saturating_sub(reserved - search_budget.steps);
        let mut candidates = searched?;
        // Stable rigid arguments identify one symbolic interface instance.
        // Unlike a raw structural search, this path proves each candidate's
        // complete argument relation before permitting selection.
        if arguments
            .iter()
            .all(|argument| argument.check_scope(&[]).is_ok())
        {
            candidates.fixed_interface = true;
        }
        let candidate_budget = Budget {
            // Search has already charged its actual work above. Unused search
            // allowance belongs to the remaining candidate budget.
            steps: budget.steps / candidates.declarations.len().max(1),
            max_depth: budget.max_depth,
        };
        let mut assessments = BTreeMap::new();
        let selection = self.select(&candidates, |head| {
            let (assessment, state) = assess(head, candidate_budget);
            assessments.insert(head.declaration, assessment);
            state
        });
        Ok(TypeResolution {
            index_identity: self.identity,
            index_revision: self.revision,
            selection,
            assessments,
        })
    }
}
impl fmt::Display for TypeEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(implementation-type-evidence (declaration {}) (arguments",
            self.implementation
        )?;
        for argument in &self.type_arguments {
            write!(f, " {argument}")?;
        }
        f.write_str(") (head-proofs")?;
        for proof in &self.head_proofs {
            write!(f, " {proof}")?;
        }
        f.write_str(") (requires-proofs")?;
        for proof in &self.requirement_proofs {
            write!(f, " {proof}")?;
        }
        f.write_str(") (eligibility")?;
        if let Some(proof) = &self.eligibility_proof {
            write!(f, " {proof}")?;
        }
        f.write_str("))")
    }
}
impl fmt::Display for TypeAssessment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Applicable(evidence) => evidence.fmt(f),
            Self::Excluded(proof) => write!(f, "(excluded {proof})"),
            Self::ValueIndexMismatch {
                position,
                expected,
                actual,
            } => write!(
                f,
                "(value-index-mismatch (position {position}) (expected {expected}) (actual {actual}))"
            ),
            Self::ForbiddenSubject(ty) => write!(f, "(forbidden-subject {ty})"),
            Self::Pending(reason) => write!(f, "(pending {reason:?})"),
            Self::Invalid(reason) => write!(f, "(invalid {reason:?})"),
        }
    }
}
impl fmt::Display for TypeResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(implementation-type-resolution (revision {}) (selection {}) (assessments",
            self.index_revision, self.selection
        )?;
        for (id, assessment) in &self.assessments {
            write!(f, " ({id} {assessment})")?;
        }
        f.write_str("))")
    }
}

impl ImplementationHead {
    /// Replay support and bind it to this exact head, substitution and query.
    /// This validates type applicability only, not implementation-world coherence.
    pub fn verify_type_evidence(
        &self,
        evidence: &TypeEvidence,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
    ) -> Result<(), crate::ptc::ProofError> {
        if self.interface.is_none() {
            return Err(crate::ptc::ProofError::Invalid);
        }
        self.verify_subject_type_evidence(evidence, subject, interface_arguments, context, budget)
    }
    pub fn verify_type_evidence_with_domains(
        &self,
        evidence: &TypeEvidence,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
        domains: &BTreeMap<DeclarationId, Term>,
    ) -> Result<(), crate::ptc::ProofError> {
        self.verify_type_evidence_with_layouts(
            evidence,
            subject,
            interface_arguments,
            context,
            budget,
            domains,
            None,
        )
    }

    pub fn verify_type_evidence_with_layouts(
        &self,
        evidence: &TypeEvidence,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
        domains: &BTreeMap<DeclarationId, Term>,
        layouts: Option<&nir::program::Package>,
    ) -> Result<(), crate::ptc::ProofError> {
        if self.interface.is_none() {
            return Err(crate::ptc::ProofError::Invalid);
        }
        self.verify_subject_evidence_with_domains(
            evidence,
            subject,
            interface_arguments,
            context,
            budget,
            domains,
            layouts,
            |proof, budget| context.verify_proof(proof, budget),
        )
    }
    pub(super) fn verify_subject_type_evidence(
        &self,
        evidence: &TypeEvidence,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
    ) -> Result<(), crate::ptc::ProofError> {
        self.verify_subject_evidence_with(
            evidence,
            subject,
            interface_arguments,
            context,
            budget,
            |proof, budget| context.verify_proof(proof, budget),
        )
    }
    pub(super) fn verify_subject_evidence_with(
        &self,
        evidence: &TypeEvidence,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
        verify_requirement: impl FnMut(&crate::ptc::Proof, Budget) -> Result<(), crate::ptc::ProofError>,
    ) -> Result<(), crate::ptc::ProofError> {
        self.verify_subject_evidence_with_domains(
            evidence,
            subject,
            interface_arguments,
            context,
            budget,
            &BTreeMap::new(),
            None,
            verify_requirement,
        )
    }
    pub(super) fn verify_subject_evidence_with_domains(
        &self,
        evidence: &TypeEvidence,
        subject: &Term,
        interface_arguments: &[Term],
        context: &Context,
        budget: Budget,
        domains: &BTreeMap<DeclarationId, Term>,
        layouts: Option<&nir::program::Package>,
        mut verify_requirement: impl FnMut(
            &crate::ptc::Proof,
            Budget,
        ) -> Result<(), crate::ptc::ProofError>,
    ) -> Result<(), crate::ptc::ProofError> {
        use crate::ptc::{Equality, ProofError};
        let remaining = validate(self, subject, interface_arguments, budget)
            .map_err(|_| ProofError::BudgetExhausted)?;
        let remaining = validate(
            self,
            subject,
            &evidence.type_arguments,
            Budget {
                steps: remaining,
                max_depth: budget.max_depth,
            },
        )
        .map_err(|_| ProofError::BudgetExhausted)?;
        if evidence.implementation != self.declaration
            || evidence.type_arguments.len() != self.type_parameters.len()
            || interface_arguments.len() != self.interface_arguments.len()
        {
            return Err(ProofError::Invalid);
        }
        let parameters = self.parameter_binders().map_err(|_| ProofError::Invalid)?;
        let mut domain_fuel = remaining;
        validate_head_arguments_with_domains(
            &parameters,
            &self.parameter_types,
            &evidence.type_arguments,
            domains,
            Some((context, budget.max_depth)),
            layouts,
            &mut domain_fuel,
        )
        .map_err(|_| ProofError::Invalid)?;
        let substitution = nir::DeclarationSubstitution::bounded(
            &self.type_parameters,
            &self.parameter_types,
            &evidence.type_arguments,
            &[],
            &mut domain_fuel,
        )
        .map_err(substitution_proof_error)?;
        let allowance =
            domain_fuel / (4 + evidence.head_proofs.len() + evidence.requirement_proofs.len());
        if allowance == 0 {
            return Err(ProofError::BudgetExhausted);
        }
        let budget = Budget {
            steps: allowance,
            max_depth: budget.max_depth,
        };
        let shape = if let Some(proof) = &evidence.eligibility_proof {
            context.verify_proof(proof, budget)?;
            match &proof.goal {
                Formula::Equal(Equality(original, shape)) if original == subject => shape,
                goal if eligible_alternatives(context, subject, goal) => subject,
                _ => return Err(ProofError::Invalid),
            }
        } else {
            subject
        };
        if !eligible_shape(context, shape)
            && !evidence
                .eligibility_proof
                .as_ref()
                .is_some_and(|proof| eligible_alternatives(context, subject, &proof.goal))
        {
            return Err(ProofError::Invalid);
        }
        let mut instantiate = |ty: &Term| {
            substitution
                .instantiate_bounded(ty, &mut domain_fuel)
                .map_err(substitution_proof_error)
        };
        let instantiated = instantiate(&self.subject)?;
        let mut expected = vec![Formula::equal(instantiated.clone(), subject.clone())];
        if shape != subject {
            expected.push(Formula::equal(instantiated, shape.clone()));
        }
        for (template, actual) in self.interface_arguments.iter().zip(interface_arguments) {
            let template = instantiate(template)?;
            if template == *actual
                && matches!(actual, Term::Rigid { declaration, sort: Sort::ComptimeValue } if domains.contains_key(declaration))
            {
                continue;
            }
            if template.is_symbolic_index_value() && actual.is_symbolic_index_value() {
                let type_relations = template
                    .index_type_relations(actual)
                    .ok_or(ProofError::Invalid)?;
                for (left, right) in type_relations {
                    expected.push(Formula::equal(left.clone(), right.clone()));
                }
                continue;
            }
            if template.sort() != Sort::Type || actual.sort() != Sort::Type {
                return Err(ProofError::Invalid);
            }
            expected.push(Formula::equal(template, actual.clone()));
        }
        if expected.len() != evidence.head_proofs.len()
            || self.requirements.len() != evidence.requirement_proofs.len()
        {
            return Err(ProofError::Invalid);
        }
        for (goal, proof) in expected.iter().zip(&evidence.head_proofs) {
            if goal != &proof.goal {
                return Err(ProofError::Invalid);
            }
            context.verify_proof(proof, budget)?;
        }
        for (requirement, proof) in self.requirements.iter().zip(&evidence.requirement_proofs) {
            let instantiated = substitution
                .instantiate_predicate_bounded(requirement, &mut domain_fuel)
                .map_err(substitution_proof_error)?;
            if Formula::from_predicate(&instantiated) != proof.goal {
                return Err(ProofError::Invalid);
            }
            verify_requirement(proof, budget)?;
        }
        Ok(())
    }
}

fn substitution_proof_error(error: nir::type_computations::Error) -> crate::ptc::ProofError {
    match error {
        nir::type_computations::Error::Budget => crate::ptc::ProofError::BudgetExhausted,
        _ => crate::ptc::ProofError::Invalid,
    }
}

// An implementation can be checked inside its own declaration telescope.
// Reusing that exact immutable binder is domain evidence; an unrelated rigid
// input, or substitution that changes its dependent domain, is not.
#[cfg(test)]
fn validate_head_arguments(
    binders: &[nir::DeclarationParameter],
    domains: &[nir::Term],
    arguments: &[nir::Term],
    fuel: &mut usize,
) -> Result<(), nir::NominalAdmissionError> {
    validate_head_arguments_with_domains(
        binders,
        domains,
        arguments,
        &BTreeMap::new(),
        None,
        None,
        fuel,
    )
}
fn validate_head_arguments_with_domains(
    binders: &[nir::DeclarationParameter],
    domains: &[Term],
    arguments: &[Term],
    evidence: &BTreeMap<DeclarationId, Term>,
    context: Option<(&Context, usize)>,
    layouts: Option<&nir::program::Package>,
    fuel: &mut usize,
) -> Result<(), nir::NominalAdmissionError> {
    if binders.len() != arguments.len() {
        return Err("implementation argument count mismatch".into());
    }
    let ids: Vec<_> = binders.iter().map(|binder| binder.declaration).collect();
    let substitution = nir::DeclarationSubstitution::bounded(&ids, domains, arguments, &[], fuel)?;
    for (index, (binder, argument)) in binders.iter().zip(arguments).enumerate() {
        let declared = domains
            .get(index)
            .unwrap_or(&nir::Term::Primitive(nir::Primitive::Type));
        let domain = substitution.instantiate_bounded(declared, fuel)?;
        nir::type_computations::charge_term(argument, fuel)?;
        if let Term::TypeListValue(elements) = argument {
            let package = layouts.ok_or_else(|| {
                nir::NominalAdmissionError::from(
                    "type-list argument requires normal-form representation evidence",
                )
            })?;
            let Term::NormalForm {
                head: declaration, ..
            } = &domain
            else {
                return Err("type-list argument domain is not a bound meta.List<Type>".into());
            };
            let mut found = None;
            for head in &package.normal_forms {
                *fuel = fuel
                    .checked_sub(1)
                    .ok_or(nir::NominalAdmissionError::Budget)?;
                if head.declaration == *declaration {
                    if found.is_some() {
                        return Err("ambiguous type-list representation evidence".into());
                    }
                    found = Some(head);
                }
            }
            if binder.sort != Sort::ComptimeValue
                || !found.is_some_and(|head| head.is_type_list_instance(&domain))
                || elements.iter().any(|element| element.sort() != Sort::Type)
                || argument.check_scope(&[]).is_err()
            {
                return Err("type-list argument lacks matching meta.List<Type> evidence".into());
            }
            continue;
        }
        if binder.sort == nir::Sort::ComptimeValue
            && *argument
                == (nir::Term::Rigid {
                    declaration: binder.declaration,
                    sort: binder.sort,
                })
            && domain == *declared
        {
            continue;
        }
        if let Term::Rigid {
            declaration,
            sort: Sort::ComptimeValue,
        } = argument
        {
            if let Some(actual) = evidence.get(declaration) {
                nir::type_computations::charge_term(actual, fuel)?;
                if binder.sort == Sort::ComptimeValue
                    && (actual == &domain
                        || context.is_some_and(|(context, max_depth)| {
                            equivalent_domains(context, actual, &domain, max_depth, fuel)
                        }))
                {
                    continue;
                }
                return Err("implementation rigid argument domain mismatch".into());
            }
        }
        if binder.sort == Sort::ComptimeValue && matches!(argument, Term::RecordValue(_)) {
            let mut symbolic = |value: &Term, expected: &Term, fuel: &mut usize| {
                let Term::Rigid {
                    declaration,
                    sort: Sort::ComptimeValue,
                } = value
                else {
                    return false;
                };
                let Some(actual) = evidence.get(declaration) else {
                    return false;
                };
                if nir::type_computations::charge_term(actual, fuel).is_err() {
                    return false;
                }
                actual == expected
                    || context.is_some_and(|(context, depth)| {
                        equivalent_domains(context, actual, expected, depth, fuel)
                    })
            };
            nir::validate_record_index_with_admission_in(
                argument,
                &domain,
                fuel,
                |value, expected, fuel| {
                    if matches!(value, Term::NominalValue { .. }) {
                        nir::validate_declaration_arguments_with_evidence_in(
                            &[binder.declaration],
                            std::slice::from_ref(expected),
                            std::slice::from_ref(value),
                            fuel,
                            |ty, variant, fuel| {
                                let package = layouts.ok_or_else(|| {
                                    nir::NominalAdmissionError::from(
                                        "nominal record index requires declaration layouts",
                                    )
                                })?;
                                nir::normal_forms::nominal_value_fields_budgeted(
                                    package, ty, variant, fuel,
                                )
                                .ok_or_else(|| {
                                    if *fuel == 0 {
                                        nir::NominalAdmissionError::Budget
                                    } else {
                                        nir::NominalAdmissionError::from(
                                            "invalid nominal record index layout",
                                        )
                                    }
                                })
                            },
                            &mut symbolic,
                            layouts.and_then(|package| package.target),
                        )?;
                        Ok(true)
                    } else {
                        Ok(symbolic(value, expected, fuel))
                    }
                },
                layouts.and_then(|package| package.target),
            )?;
            continue;
        }
        nir::validate_declaration_arguments_with_layouts_in(
            &[binder.declaration],
            &[domain],
            std::slice::from_ref(argument),
            fuel,
            |ty, variant, fuel| {
                let package = layouts.ok_or_else(|| {
                    nir::NominalAdmissionError::from(
                        "nominal value admission requires declaration layout evidence",
                    )
                })?;
                nir::normal_forms::nominal_value_fields_budgeted(package, ty, variant, fuel)
                    .ok_or_else(|| {
                        if *fuel == 0 {
                            nir::NominalAdmissionError::Budget
                        } else {
                            nir::NominalAdmissionError::from("invalid nominal value layout")
                        }
                    })
            },
            layouts.and_then(|package| package.target),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod captured_head_domains {
    use super::*;

    #[test]
    fn completed_domains_admit_type_valued_indices_and_later_parameters() {
        let ids: Vec<_> = (1..=3)
            .map(|local| DeclarationId { package: 81, local })
            .collect();
        let type_domain = Term::Primitive(nir::Primitive::Type);
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let domains = [
            type_domain.clone(),
            Term::Rigid {
                declaration: ids[0],
                sort: Sort::Type,
            },
            type_domain.clone(),
        ];
        let binders = nir::declaration_parameters(&ids, &domains).unwrap();
        let arguments = [type_domain.clone(), boolean.clone(), boolean.clone()];
        assert!(validate_head_arguments(&binders, &domains, &arguments, &mut 1000).is_ok());
        for invalid in [Term::Char('x'), Term::Bool(true)] {
            let arguments = [type_domain.clone(), boolean.clone(), invalid];
            assert!(validate_head_arguments(&binders, &domains, &arguments, &mut 1000).is_err());
        }
        let invalid = [type_domain, Term::Bool(true), Term::Bool(true)];
        assert!(validate_head_arguments(&binders, &domains, &invalid, &mut 1000).is_err());
        assert!(matches!(
            validate_head_arguments(&binders, &domains, &arguments, &mut 0),
            Err(nir::NominalAdmissionError::Budget)
        ));
        assert!(validate_head_arguments(&binders, &domains, &arguments, &mut 1000).is_ok());
    }

    #[test]
    fn symbolic_admission_requires_the_same_binder_and_unchanged_domain() {
        let t = nir::DeclarationParameter {
            declaration: DeclarationId {
                package: 81,
                local: 1,
            },
            sort: Sort::Type,
        };
        let v = nir::DeclarationParameter {
            declaration: DeclarationId {
                package: 81,
                local: 2,
            },
            sort: Sort::ComptimeValue,
        };
        let rigid = |parameter: nir::DeclarationParameter| Term::Rigid {
            declaration: parameter.declaration,
            sort: parameter.sort,
        };
        let binders = [t, v];
        let domains = [Term::Primitive(nir::Primitive::Type), rigid(t)];
        let symbolic = [rigid(t), rigid(v)];
        assert!(validate_head_arguments(&binders, &domains, &symbolic, &mut 1000).is_ok());
        assert!(
            validate_head_arguments(
                &binders,
                &domains,
                &[Term::Primitive(nir::Primitive::Bool), rigid(v)],
                &mut 1000
            )
            .is_err()
        );
        assert!(
            validate_head_arguments(
                &binders,
                &domains,
                &[
                    rigid(t),
                    Term::Rigid {
                        declaration: DeclarationId {
                            package: 81,
                            local: 3
                        },
                        sort: Sort::ComptimeValue
                    }
                ],
                &mut 1000
            )
            .is_err()
        );
        assert!(
            validate_head_arguments(
                &binders,
                &domains,
                &[Term::Primitive(nir::Primitive::Bool), Term::Bool(true)],
                &mut 1000
            )
            .is_ok()
        );
        assert!(
            validate_head_arguments(
                &binders,
                &domains,
                &[Term::Primitive(nir::Primitive::Bool), Term::Char('x')],
                &mut 1000
            )
            .is_err()
        );
        assert!(matches!(
            validate_head_arguments(&binders, &domains, &symbolic, &mut 0),
            Err(nir::NominalAdmissionError::Budget)
        ));
        assert!(validate_head_arguments(&binders, &domains, &symbolic, &mut 1000).is_ok());
    }
}

#[cfg(test)]
mod shared_requirement_budget {
    use super::*;

    fn matched(requirements: Vec<TypePredicate>) -> HeadMatch {
        HeadMatch {
            evidence: TypeEvidence {
                implementation: DeclarationId {
                    package: 92,
                    local: 1,
                },
                type_arguments: vec![],
                head_proofs: vec![],
                requirement_proofs: vec![],
                eligibility_proof: None,
            },
            requirements,
            requirement_budget: Budget {
                steps: 10,
                ..Default::default()
            },
        }
    }

    #[test]
    fn unused_negative_and_positive_work_reaches_later_requirements() {
        let context = Context::default();
        let mut allowances = Vec::new();
        let assessment =
            matched(vec![TypePredicate::Bool(true); 2]).discharge_metered_with(|goal, budget| {
                allowances.push(budget.steps);
                let cost = if allowances.len() == 1 { 3 } else { 30 };
                budget.steps = budget
                    .steps
                    .checked_sub(cost)
                    .expect("shared work available");
                context.prove(goal, Budget::default())
            });
        assert!(matches!(assessment, TypeAssessment::Applicable(_)));
        assert_eq!(allowances, [20, 37]);
    }

    #[test]
    fn negative_proof_uses_only_the_unspent_requirement_share() {
        let context = Context::default();
        let mut allowances = Vec::new();
        let assessment =
            matched(vec![TypePredicate::Bool(false)]).discharge_metered_with(|goal, budget| {
                allowances.push(budget.steps);
                budget.steps -= 5;
                context.prove(goal, Budget::default())
            });
        assert!(matches!(assessment, TypeAssessment::Excluded(_)));
        assert_eq!(allowances, [20, 15]);
    }

    #[test]
    fn queries_cannot_mint_work_or_publish_partial_evidence() {
        let context = Context::default();
        let predicates = vec![TypePredicate::Bool(true); 2];
        let inflated = matched(predicates.clone()).discharge_metered_with(|goal, budget| {
            budget.steps += 1;
            context.prove(goal, Budget::default())
        });
        assert!(matches!(inflated, TypeAssessment::Invalid(_)));
        let exhausted = matched(predicates.clone()).discharge_metered_with(|goal, budget| {
            budget.steps = 0;
            Outcome::Unknown {
                reason: crate::ptc::UnknownReason::BudgetExhausted,
                residual: goal,
            }
        });
        assert!(matches!(exhausted, TypeAssessment::Pending(_)));
        let recovered = matched(predicates)
            .discharge_metered_with(|goal, budget| context.prove_metered(goal, budget));
        assert!(matches!(recovered, TypeAssessment::Applicable(_)));
    }
}
