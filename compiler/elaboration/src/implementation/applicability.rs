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
        let expected = self
            .requirements
            .iter()
            .map(|requirement| {
                requirement.instantiate_parameters(
                    &self.parameter_binders()?,
                    &matched.evidence.type_arguments,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ProofError::Invalid)?;
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
        head.verify_type_evidence(
            &matched.evidence,
            subject,
            interface_arguments,
            context,
            Budget {
                steps: remaining,
                ..budget
            },
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
            Work::Term(Term::Pointer(t) | Term::ComptimeFunction(t)) => {
                pending.push((Work::Term(t), depth + 1))
            }
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
                && !term.is_structural_index_value()
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
        let budget = Budget {
            steps: allowance,
            max_depth: budget.max_depth,
        };
        let (shape, eligibility_proof) = match subject {
            subject if context.is_tuple_type(subject) => (subject.clone(), None),
            Term::Rigid { .. } | Term::NormalForm { .. } | Term::AssociatedType { .. } => {
                match context
                    .structural_view_with_heads_metered(subject, budget, |head| {
                        context.is_tuple_head(head)
                    })
                    .0
                {
                    Ok(Some((shape, proof))) => (shape, Some(proof)),
                    _ => {
                        return Err(TypeAssessment::Pending(
                            "user implementation subject eligibility is unresolved".into(),
                        ));
                    }
                }
            }
            other => (other.clone(), None),
        };
        if matches!(shape, Term::Closure { .. }) {
            return Err(TypeAssessment::ForbiddenSubject(shape));
        }
        if !matches!(
            shape,
            Term::Primitive(_)
                | Term::Nominal { .. }
                | Term::Tuple(_)
                | Term::Record(_)
                | Term::Pointer(_)
                | Term::Function { .. }
                | Term::ComptimeFunction(_)
        ) && !context.is_tuple_type(&shape)
        {
            return Err(TypeAssessment::Pending(
                "user implementation subject eligibility is unresolved".into(),
            ));
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
        for (position, (template, actual)) in self
            .interface_arguments
            .iter()
            .zip(interface_arguments)
            .enumerate()
        {
            if template.sort() == Sort::ComptimeValue && actual.is_structural_index_value() {
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
                            let Some(type_relations) = bound.index_type_relations(actual) else {
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
                        && !matches!(template, Term::RecordValue(_))
                    {
                        return Err(TypeAssessment::Pending(
                            "value index identity is unresolved".into(),
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
        let (mut candidate, ids) = match root.match_many(&relations, budget) {
            Ok(candidate) => candidate,
            Err(reason) => {
                return Err(TypeAssessment::Pending(format!(
                    "head inference: {reason:?}"
                )));
            }
        };
        candidate.advance(ids.len(), budget);
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
        nir::validate_declaration_arguments(
            &self.type_parameters,
            &self.parameter_types,
            &arguments,
            &mut domain_fuel,
        )
        .map_err(TypeAssessment::Invalid)?;
        let requirements = match self
            .requirements
            .iter()
            .map(|requirement| requirement.instantiate_parameters(&parameters, &arguments))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(requirements) => requirements,
            Err(error) => return Err(TypeAssessment::Invalid(error.to_string())),
        };
        Ok(HeadMatch {
            evidence: TypeEvidence {
                implementation: self.declaration,
                type_arguments: arguments,
                head_proofs: proofs,
                requirement_proofs: vec![],
                eligibility_proof,
            },
            requirements,
            requirement_budget: budget,
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
        for requirement in &pending_requirements {
            let requirement = Formula::from_predicate(requirement);
            match prove(requirement.clone(), budget) {
                Outcome::Entailed(proof) => requirements.push(proof),
                Outcome::IllFormed(reason) => return TypeAssessment::Invalid(reason),
                _ => {
                    let assessment = match prove(requirement.negate(), budget) {
                        Outcome::Entailed(proof) => TypeAssessment::Excluded(proof),
                        Outcome::IllFormed(reason) => TypeAssessment::Invalid(reason),
                        _ => TypeAssessment::Pending(
                            "type relation has not been proved or refuted".into(),
                        ),
                    };
                    if matches!(
                        assessment,
                        TypeAssessment::Excluded(_) | TypeAssessment::Invalid(_)
                    ) {
                        return assessment;
                    }
                    pending = true;
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
            let assessment = head.assess_types(subject, arguments, context, allowance);
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
        mut assess: impl FnMut(&ImplementationHead, Budget) -> (TypeAssessment, Applicability),
    ) -> Result<TypeResolution, Error> {
        let search_budget = Budget {
            steps: budget.steps / 2,
            max_depth: budget.max_depth,
        };
        let mut candidates = self.candidates(Some(interface), arguments, subject, search_budget)?;
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
            steps: (budget.steps - search_budget.steps) / candidates.declarations.len().max(1),
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
        nir::validate_declaration_arguments(
            &self.type_parameters,
            &self.parameter_types,
            &evidence.type_arguments,
            &mut domain_fuel,
        )
        .map_err(|_| ProofError::Invalid)?;
        for (parameter, arg) in parameters.iter().zip(&evidence.type_arguments) {
            if arg.sort() != parameter.sort || arg.check_scope(&[]).is_err() {
                return Err(ProofError::Invalid);
            }
        }
        let allowance =
            remaining / (4 + evidence.head_proofs.len() + evidence.requirement_proofs.len());
        if allowance == 0 {
            return Err(ProofError::BudgetExhausted);
        }
        let budget = Budget {
            steps: allowance,
            max_depth: budget.max_depth,
        };
        let shape = if let Some(proof) = &evidence.eligibility_proof {
            let Formula::Equal(Equality(original, shape)) = &proof.goal else {
                return Err(ProofError::Invalid);
            };
            if original != subject {
                return Err(ProofError::Invalid);
            }
            context.verify_proof(proof, budget)?;
            shape
        } else {
            subject
        };
        if !matches!(
            shape,
            Term::Primitive(_)
                | Term::Nominal { .. }
                | Term::Tuple(_)
                | Term::Record(_)
                | Term::Pointer(_)
                | Term::Function { .. }
                | Term::ComptimeFunction(_)
        ) && !context.is_tuple_type(&shape)
        {
            return Err(ProofError::Invalid);
        }
        let instantiate = |ty: &Term| {
            ty.instantiate_parameters(&parameters, &evidence.type_arguments)
                .map_err(|_| ProofError::Invalid)
        };
        let instantiated = instantiate(&self.subject)?;
        let mut expected = vec![Formula::equal(instantiated.clone(), subject.clone())];
        if shape != subject {
            expected.push(Formula::equal(instantiated, shape.clone()));
        }
        for (template, actual) in self.interface_arguments.iter().zip(interface_arguments) {
            let template = instantiate(template)?;
            if template.is_structural_index_value() && actual.is_structural_index_value() {
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
            let instantiated = requirement
                .instantiate_parameters(&parameters, &evidence.type_arguments)
                .map_err(|_| ProofError::Invalid)?;
            if Formula::from_predicate(&instantiated) != proof.goal {
                return Err(ProofError::Invalid);
            }
            verify_requirement(proof, budget)?;
        }
        Ok(())
    }
}
