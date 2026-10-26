//! Type-only coherence of declaration heads, independently of caller premises.
//! A countermodel establishes possible type overlap, not phase/environment or
//! final user-subject eligibility. Only disjointness permits same-rank pairs.
use super::*;
use crate::ptc::{Context, Formula, Outcome, Proof, Witness};
use nir::program::TypePredicate;

#[derive(Clone, Debug)]
pub enum PairAssessment {
    DifferentInterfaces,
    OrderedPrecision { left: usize, right: usize },
    Disjoint(Proof),
    NotDisjoint(Witness),
    Unresolved(String),
}
impl PairAssessment {
    pub fn is_proved(&self) -> bool {
        matches!(
            self,
            Self::DifferentInterfaces | Self::OrderedPrecision { .. } | Self::Disjoint(_)
        )
    }
}

// An overapproximation of value-index overlap. Known record/scalar
// structure may refute overlap; unknown computations and values remain free.
// Only Type leaves are emitted as propositions.
fn index_overlap(
    left: &Term,
    right: &Term,
    fuel: &mut usize,
    max_depth: usize,
) -> Result<Vec<Formula>, String> {
    let mut pending = vec![(left, right, 0usize)];
    let mut relations = Vec::new();
    while let Some((left, right, depth)) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("index overlap budget exhausted")?;
        if depth >= max_depth.min(128) {
            return Err("index overlap depth exhausted".into());
        }
        if left.has_pending_error_branch() || right.has_pending_error_branch() {
            continue;
        }
        if left.sort() == Sort::Type && right.sort() == Sort::Type {
            relations.push(Formula::equal(left.clone(), right.clone()));
            continue;
        }
        if let (
            Term::NominalValue {
                ty: a,
                variant: av,
                fields: af,
            },
            Term::NominalValue {
                ty: b,
                variant: bv,
                fields: bf,
            },
        ) = (left, right)
        {
            if av != bv || af.len() != bf.len() {
                return Ok(vec![Formula::False]);
            }
            if af.len().saturating_add(1) > fuel.saturating_sub(pending.len()) {
                return Err("index overlap budget exhausted".into());
            }
            pending.push((a, b, depth + 1));
            pending.extend(af.iter().zip(bf).rev().map(|(a, b)| (a, b, depth + 1)));
            continue;
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
        ) = (left, right)
        {
            if al.is_some() != bl.is_some() {
                return Ok(vec![Formula::False]);
            }
            pending.push((a, b, depth + 1));
            if let (Some(a), Some(b)) = (al, bl) {
                pending.push((a, b, depth + 1));
            }
            pending.push((av, bv, depth + 1));
            continue;
        }
        if let (Term::TupleValue(a), Term::TupleValue(b)) = (left, right) {
            if a.len() != b.len() {
                return Ok(vec![Formula::False]);
            }
            if a.len() > fuel.saturating_sub(pending.len()) {
                return Err("index overlap budget exhausted".into());
            }
            pending.extend(a.iter().zip(b).rev().map(|(a, b)| (a, b, depth + 1)));
            continue;
        }
        if let (Term::RecordValue(a), Term::RecordValue(b)) = (left, right) {
            if a.len() != b.len() || a.iter().zip(b).any(|((a, _), (b, _))| a != b) {
                return Ok(vec![Formula::False]);
            }
            if a.len() > fuel.saturating_sub(pending.len()) {
                return Err("index overlap budget exhausted".into());
            }
            pending.extend(
                a.iter()
                    .zip(b)
                    .rev()
                    .map(|((_, a), (_, b))| (a, b, depth + 1)),
            );
            continue;
        }
        let known = |term: &Term| {
            term.sort() == Sort::Type
                || term.scalar_index_domain().is_some()
                || matches!(
                    term,
                    Term::ErrorValue { .. }
                        | Term::RecordValue(_)
                        | Term::TupleValue(_)
                        | Term::NominalValue { .. }
                )
        };
        if known(left) && known(right) && left != right {
            return Ok(vec![Formula::False]);
        }
        // A rigid or opaque value can match any supported value of its domain.
        // Do not unfold it, invert it, or manufacture a PTC value equation.
    }
    Ok(relations)
}

// Input is fully bounded before cloning or invoking recursive substitutions.
// Fresh rigid IDs are chosen deterministically outside all input rigid IDs.
fn separation_goal(
    a: &ImplementationHead,
    b: &ImplementationHead,
    budget: Budget,
) -> Result<(Formula, Budget), String> {
    let mut fuel = super::applicability::validate(a, &b.subject, &b.interface_arguments, budget)?;
    fuel = super::applicability::validate(
        b,
        &a.subject,
        &a.interface_arguments,
        Budget {
            steps: fuel,
            ..budget
        },
    )?;
    // Substitution visits terms once per parameter in the current NIR helper.
    let parameter_count = a
        .type_parameters
        .len()
        .saturating_add(b.type_parameters.len());
    let substitution_work = parameter_count.saturating_mul(budget.steps - fuel);
    fuel = fuel
        .checked_sub(substitution_work)
        .ok_or("coherence substitution budget exhausted")?;
    let mut used = BTreeSet::new();
    for head in [a, b] {
        if head.type_parameters.len() > fuel {
            return Err("coherence input budget exhausted".into());
        }
        fuel -= head.type_parameters.len();
        let parameters: BTreeSet<_> = head.type_parameters.iter().copied().collect();
        if parameters.len() != head.type_parameters.len() {
            return Err("duplicate implementation type parameter".into());
        }
        used.extend(parameters);
        let mut predicates: Vec<_> = head.requirements.iter().collect();
        let mut terms = vec![&head.subject];
        terms.extend(&head.interface_arguments);
        while let Some(predicate) = predicates.pop() {
            match predicate {
                TypePredicate::Trait(requirement) => {
                    terms.push(&requirement.subject);
                    terms.extend(&requirement.interface_arguments);
                }
                TypePredicate::Equal(a, b) => {
                    terms.push(a);
                    terms.push(b);
                }
                TypePredicate::Not(p) => predicates.push(p),
                TypePredicate::And(items) | TypePredicate::Or(items) => predicates.extend(items),
                TypePredicate::Bool(_) => {}
            }
        }
        while let Some(term) = terms.pop() {
            match term {
                Term::AssociatedType {
                    arguments, subject, ..
                } => {
                    terms.extend(arguments);
                    terms.push(subject);
                }
                Term::Rigid { declaration, .. } => {
                    used.insert(*declaration);
                }
                Term::Nominal { arguments, .. }
                | Term::NormalForm { arguments, .. }
                | Term::Tuple(arguments)
                | Term::ClosureContextValue(arguments)
                | Term::TypeListValue(arguments)
                | Term::TypeListConcat(arguments)
                | Term::TupleValue(arguments) => terms.extend(arguments),
                Term::ErrorValue { ty, label, value } => {
                    terms.push(ty);
                    if let Some(label) = label {
                        terms.push(label);
                    }
                    terms.push(value);
                }
                Term::NominalValue { ty, fields, .. } => {
                    terms.push(ty);
                    terms.extend(fields);
                }
                Term::Record(fields) | Term::RecordValue(fields) => {
                    terms.extend(fields.iter().map(|(_, ty)| ty))
                }
                Term::Pointer(inner)
                | Term::ComptimeFunction(inner)
                | Term::UnsafeFunction(inner) => terms.push(inner),
                Term::Function { parameters, result } => {
                    terms.extend(parameters.iter().map(|p| &p.ty));
                    terms.push(result);
                }
                Term::ErrorResult { errors, result } => {
                    terms.push(result);
                    terms.extend(errors);
                }
                Term::Computation { effects, result } => {
                    terms.push(result);
                    for operation in effects {
                        terms.extend(&operation.arguments);
                    }
                }
                Term::ContextualClosure { context, signature } => {
                    terms.push(context);
                    terms.push(signature);
                }
                Term::Closure {
                    environment,
                    signature,
                } => {
                    terms.extend(environment);
                    terms.push(signature);
                }
                _ => {}
            }
        }
    }
    let mut next = u64::MAX;
    let mut rename =
        |head: &ImplementationHead| -> Result<(Term, Vec<Term>, Vec<Formula>), String> {
            let mut replacements = Vec::new();
            let parameters = head.parameter_binders().map_err(|e| e.to_string())?;
            for parameter in &parameters {
                loop {
                    fuel = fuel
                        .checked_sub(1)
                        .ok_or("coherence renaming budget exhausted")?;
                    let id = DeclarationId {
                        package: next,
                        local: 0,
                    };
                    next = next
                        .checked_sub(1)
                        .ok_or("coherence identity space exhausted")?;
                    if used.insert(id) {
                        replacements.push(Term::Rigid {
                            declaration: id,
                            sort: parameter.sort,
                        });
                        break;
                    }
                }
            }
            let instantiate = |term: &Term| {
                term.instantiate_parameters(&parameters, &replacements)
                    .map_err(|e| e.to_string())
            };
            let subject = instantiate(&head.subject)?;
            let args = head
                .interface_arguments
                .iter()
                .map(instantiate)
                .collect::<Result<Vec<_>, _>>()?;
            let requires = head
                .requirements
                .iter()
                .map(|p| {
                    p.instantiate_parameters(&parameters, &replacements)
                        .map(|p| Formula::from_predicate(&p))
                        .map_err(|e| e.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((subject, args, requires))
        };
    let (a, aa, ar) = rename(a)?;
    let (b, ba, br) = rename(b)?;
    if aa.len() != ba.len() {
        return Err("inconsistent interface arity".into());
    }
    let mut overlap = vec![Formula::equal(a, b)];
    for (a, b) in aa.into_iter().zip(ba) {
        if a.sort() == Sort::Type && b.sort() == Sort::Type {
            overlap.push(Formula::equal(a, b));
        } else if a.sort() == Sort::ComptimeValue && b.sort() == Sort::ComptimeValue {
            overlap.extend(index_overlap(&a, &b, &mut fuel, budget.max_depth)?);
        } else {
            return Err("interface index overlap requires a declared supported domain".into());
        }
    }
    overlap.extend(ar);
    overlap.extend(br);
    Ok((
        Formula::And(overlap).negate(),
        Budget {
            steps: fuel,
            ..budget
        },
    ))
}
impl ImplementationHead {
    pub fn assess_type_overlap(
        &self,
        other: &Self,
        context: &Context,
        budget: Budget,
    ) -> PairAssessment {
        let (Some(a), Some(b)) = (self.interface, other.interface) else {
            return PairAssessment::Unresolved("open interface head".into());
        };
        if a != b {
            return PairAssessment::DifferentInterfaces;
        }
        self.assess_subject_overlap(other, context, budget)
    }
    pub(super) fn assess_subject_overlap(
        &self,
        other: &Self,
        context: &Context,
        budget: Budget,
    ) -> PairAssessment {
        let mut fuel = budget.steps;
        let precisions = (|| {
            let a = Key::with_fuel(&self.subject, &mut fuel, budget.max_depth)?.precision();
            let b = Key::with_fuel(&other.subject, &mut fuel, budget.max_depth)?.precision();
            Ok::<_, Error>((a, b))
        })();
        let (left, right) = match precisions {
            Ok(pair) => pair,
            Err(e) => return PairAssessment::Unresolved(format!("{e:?}")),
        };
        if left != right {
            return PairAssessment::OrderedPrecision { left, right };
        }
        let (goal, budget) = match separation_goal(
            self,
            other,
            Budget {
                steps: fuel,
                ..budget
            },
        ) {
            Ok(query) => query,
            Err(e) => return PairAssessment::Unresolved(e),
        };
        match context.theory_context().prove(goal, budget) {
            Outcome::Entailed(proof) => PairAssessment::Disjoint(proof),
            Outcome::NotEntailed(witness) => PairAssessment::NotDisjoint(witness),
            Outcome::Unknown { reason, .. } => PairAssessment::Unresolved(format!("{reason:?}")),
            Outcome::IllFormed(reason) => PairAssessment::Unresolved(reason),
        }
    }

    /// Rebuild the exact alpha-renamed obligation and replay the supplied tree.
    pub fn verify_disjointness(
        &self,
        other: &Self,
        proof: &Proof,
        context: &Context,
        budget: Budget,
    ) -> bool {
        if self.interface.is_none() || self.interface != other.interface {
            return false;
        }
        self.verify_subject_disjointness(other, proof, context, budget)
    }
    pub(super) fn verify_subject_disjointness(
        &self,
        other: &Self,
        proof: &Proof,
        context: &Context,
        budget: Budget,
    ) -> bool {
        let Ok((goal, budget)) = separation_goal(self, other, budget) else {
            return false;
        };
        proof.goal == goal && context.theory_context().verify_proof(proof, budget).is_ok()
    }
}

#[derive(Clone, Debug)]
pub struct CoherenceReport {
    identity: u64,
    revision: u64,
    pub pairs: Vec<(DeclarationId, DeclarationId, PairAssessment)>,
    pub open_interfaces: Vec<DeclarationId>,
    pub complete: bool,
}
impl CoherenceReport {
    /// Only type-head pair obligations, not phase or declaration well-formedness.
    pub fn is_proved(&self) -> bool {
        self.complete
            && self.open_interfaces.is_empty()
            && self.pairs.iter().all(|(_, _, p)| p.is_proved())
    }
}
impl Index {
    // Reserve traversal/report work for every pair, but distribute proof work
    // only among same-interface, same-precision obligations. Unrelated traits
    // must not starve a small number of actual solver queries.
    fn coherence_allowance(
        &self,
        budget: Budget,
    ) -> Option<(
        usize,
        std::collections::BTreeMap<(DeclarationId, usize), usize>,
    )> {
        let count = self.entries.len();
        let pairs = count.checked_mul(count.saturating_sub(1))? / 2;
        let remaining = budget
            .steps
            .checked_sub(count.checked_mul(2)?.checked_add(pairs)?)?;
        let mut groups = std::collections::BTreeMap::new();
        for entry in self.entries.values() {
            if let Some(interface) = entry.head.interface {
                let size = groups.entry((interface, entry.precision)).or_insert(0usize);
                *size += 1;
            }
        }
        // Share proof work across independent interface/precision groups first.
        // A large scalar protocol must not starve an unrelated generic pair.
        let active = groups.values().filter(|size| **size > 1).count();
        let group_budget = remaining / active.max(1);
        let allowances = groups
            .into_iter()
            .filter_map(|(key, size)| {
                let obligations = size * size.saturating_sub(1) / 2;
                (obligations > 0).then(|| (key, group_budget / obligations))
            })
            .collect();
        Some((pairs, allowances))
    }

    /// Verify coverage, ordering, world freshness and every disjointness tree;
    /// editing the report's public diagnostic entries cannot forge acceptance.
    pub fn verify_type_coherence(
        &self,
        report: &CoherenceReport,
        context: &Context,
        budget: Budget,
    ) -> bool {
        if !self.type_coherence_is_current(report) || !report.is_proved() {
            return false;
        }
        let Some((pairs, allowances)) = self.coherence_allowance(budget) else {
            return false;
        };
        if report.pairs.len() != pairs {
            return false;
        }
        let entries: Vec<_> = self.entries.values().collect();
        let mut reported = report.pairs.iter();
        for (i, a) in entries.iter().enumerate() {
            if a.head.interface.is_none() {
                return false;
            }
            for b in &entries[i + 1..] {
                let Some((left, right, assessment)) = reported.next() else {
                    return false;
                };
                if *left != a.head.declaration || *right != b.head.declaration {
                    return false;
                }
                let valid = if a.head.interface != b.head.interface {
                    matches!(assessment, PairAssessment::DifferentInterfaces)
                } else if a.precision != b.precision {
                    matches!(assessment, PairAssessment::OrderedPrecision { left,right } if *left == a.precision && *right == b.precision)
                } else if let PairAssessment::Disjoint(proof) = assessment {
                    a.head.verify_disjointness(
                        &b.head,
                        proof,
                        context,
                        Budget {
                            steps: allowances[&(a.head.interface.unwrap(), a.precision)],
                            ..budget
                        },
                    )
                } else {
                    false
                };
                if !valid {
                    return false;
                }
            }
        }
        true
    }
    pub fn type_coherence_is_current(&self, report: &CoherenceReport) -> bool {
        report.identity == self.identity && report.revision == self.revision
    }
    pub fn check_type_coherence(&self, context: &Context, budget: Budget) -> CoherenceReport {
        let mut report = CoherenceReport {
            identity: self.identity,
            revision: self.revision,
            pairs: Vec::new(),
            open_interfaces: Vec::new(),
            complete: true,
        };
        let Some((_, allowances)) = self.coherence_allowance(budget) else {
            report.complete = false;
            return report;
        };
        let entries: Vec<_> = self.entries.values().collect();
        for (i, a) in entries.iter().enumerate() {
            if a.head.interface.is_none() {
                report.open_interfaces.push(a.head.declaration);
            }
            for b in &entries[i + 1..] {
                let assessment = if a.head.interface.is_none() || b.head.interface.is_none() {
                    PairAssessment::Unresolved("open interface head".into())
                } else if a.head.interface != b.head.interface {
                    PairAssessment::DifferentInterfaces
                } else if a.precision != b.precision {
                    PairAssessment::OrderedPrecision {
                        left: a.precision,
                        right: b.precision,
                    }
                } else {
                    a.head.assess_type_overlap(
                        &b.head,
                        context,
                        Budget {
                            steps: allowances[&(a.head.interface.unwrap(), a.precision)],
                            ..budget
                        },
                    )
                };
                report
                    .pairs
                    .push((a.head.declaration, b.head.declaration, assessment));
            }
        }
        report
    }
}
impl fmt::Display for PairAssessment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DifferentInterfaces => write!(f, "(different-interfaces)"),
            Self::OrderedPrecision { left, right } => {
                write!(f, "(ordered-precision {left} {right})")
            }
            Self::Disjoint(proof) => write!(f, "(disjoint {proof})"),
            Self::NotDisjoint(witness) => write!(
                f,
                "(not-disjoint {})",
                Outcome::NotEntailed(witness.clone())
            ),
            Self::Unresolved(reason) => write!(f, "(unresolved {reason:?})"),
        }
    }
}
impl fmt::Display for CoherenceReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(type-coherence (revision {}) (complete {}) (open-interfaces",
            self.revision, self.complete
        )?;
        for id in &self.open_interfaces {
            write!(f, " {id}")?;
        }
        write!(f, ") (pairs")?;
        for (a, b, result) in &self.pairs {
            write!(f, " (pair {a} {b} {result})")?;
        }
        write!(f, "))")
    }
}

#[cfg(test)]
mod pending_error_indices {
    use super::*;
    #[test]
    fn incomplete_inner_branch_does_not_prove_implementation_disjointness() {
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let inner = Term::error_result(vec![boolean.clone()], boolean.clone());
        let pending = Term::error_value(
            vec![Term::Primitive(nir::Primitive::Char)],
            inner.clone(),
            None,
            Term::Rigid {
                declaration: DeclarationId {
                    package: 1,
                    local: 100,
                },
                sort: Sort::ComptimeValue,
            },
        )
        .unwrap();
        for label in [None, Some(Box::new(boolean.clone()))] {
            let completed = Term::error_value(
                vec![Term::Primitive(nir::Primitive::Char)],
                inner.clone(),
                None,
                Term::ErrorValue {
                    ty: Box::new(inner.clone()),
                    label,
                    value: Box::new(Term::Bool(true)),
                },
            )
            .unwrap();
            assert!(
                index_overlap(&pending, &completed, &mut 10000, 64)
                    .unwrap()
                    .is_empty()
            );
        }
    }
}
