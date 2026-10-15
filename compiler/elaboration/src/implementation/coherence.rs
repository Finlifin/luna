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
                | Term::TypeListValue(arguments)
                | Term::TypeListConcat(arguments)
                | Term::TupleValue(arguments) => terms.extend(arguments),
                Term::NominalValue { ty, fields, .. } => {
                    terms.push(ty);
                    terms.extend(fields);
                }
                Term::Record(fields) | Term::RecordValue(fields) => {
                    terms.extend(fields.iter().map(|(_, ty)| ty))
                }
                Term::Pointer(inner) | Term::ComptimeFunction(inner) => terms.push(inner),
                Term::Function { parameters, result } => {
                    terms.extend(parameters.iter().map(|p| &p.ty));
                    terms.push(result);
                }
                Term::Computation { effects, result } => {
                    terms.push(result);
                    for operation in effects {
                        terms.extend(&operation.arguments);
                    }
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
            for _ in &head.type_parameters {
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
                            sort: Sort::Type,
                        });
                        break;
                    }
                }
            }
            let instantiate = |term: &Term| {
                term.instantiate_types(&head.type_parameters, &replacements)
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
                    p.instantiate_types(&head.type_parameters, &replacements)
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
        match (&a, &b) {
            (Term::Symbol(left), Term::Symbol(right)) => {
                overlap.push(if left == right {
                    Formula::True
                } else {
                    Formula::False
                });
            }
            _ if a.sort() == Sort::Type && b.sort() == Sort::Type => {
                overlap.push(Formula::equal(a, b))
            }
            _ => return Err("interface index overlap requires a declared supported domain".into()),
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
        let count = self.entries.len();
        let Some(pairs) = count.checked_mul(count.saturating_sub(1)).map(|n| n / 2) else {
            return false;
        };
        if report.pairs.len() != pairs || pairs.saturating_add(count) > budget.steps {
            return false;
        }
        let allowance = (budget.steps - count) / pairs.max(1);
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
                            steps: allowance,
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
        let count = self.entries.len();
        let Some(pair_count) = count.checked_mul(count.saturating_sub(1)).map(|n| n / 2) else {
            report.complete = false;
            return report;
        };
        if pair_count.saturating_add(count) > budget.steps {
            report.complete = false;
            return report;
        }
        let allowance = (budget.steps - count) / pair_count.max(1);
        let entries: Vec<_> = self.entries.values().collect();
        for (i, a) in entries.iter().enumerate() {
            if a.head.interface.is_none() {
                report.open_interfaces.push(a.head.declaration);
            }
            for b in &entries[i + 1..] {
                let assessment = a.head.assess_type_overlap(
                    &b.head,
                    context,
                    Budget {
                        steps: allowance,
                        ..budget
                    },
                );
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
