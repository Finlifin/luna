//! Resolve phase availability only after the common implementation relationship.
use super::*;
use hir::ImplementationPhase;
use nir::program::TypePredicate;

fn slot(phase: ImplementationPhase) -> usize {
    match phase {
        ImplementationPhase::Runtime => 0,
        ImplementationPhase::Comptime => 1,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhaseSelection {
    Selected {
        relationship: DeclarationId,
        contribution: DeclarationId,
        phase: ImplementationPhase,
    },
    Missing {
        relationship: DeclarationId,
        phase: ImplementationPhase,
    },
    Unselected(Selection),
}
impl fmt::Display for PhaseSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Selected {
                relationship,
                contribution,
                phase,
            } => write!(
                f,
                "(phase-selection (relationship {relationship}) (phase {phase}) (contribution {contribution}))"
            ),
            Self::Missing {
                relationship,
                phase,
            } => write!(
                f,
                "(missing-phase (relationship {relationship}) (phase {phase}))"
            ),
            Self::Unselected(selection) => write!(f, "(phase-unselected {selection})"),
        }
    }
}

/// Alpha-bound head identity. Normal-form bodies are never unfolded and free
/// declaration identities are retained. Only applicability conditions use
/// logical equivalence; heads themselves retain structural identity.
#[derive(PartialEq, Eq)]
struct Relationship {
    interface: Option<DeclarationId>,
    domains: Vec<Term>,
    subject: Term,
    arguments: Vec<Term>,
    requirements: Vec<TypePredicate>,
    sorts: Vec<Sort>,
}

fn predicate_scope(predicate: &TypePredicate) -> Result<(), nir::TypeError> {
    match predicate {
        TypePredicate::Bool(_) => Ok(()),
        TypePredicate::Trait(requirement) => {
            requirement.subject.check_scope(&[])?;
            for argument in &requirement.interface_arguments {
                argument.check_scope(&[])?;
            }
            Ok(())
        }
        TypePredicate::Equal(left, right) => {
            left.check_scope(&[])?;
            right.check_scope(&[])
        }
        TypePredicate::Not(inner) => predicate_scope(inner),
        TypePredicate::And(parts) | TypePredicate::Or(parts) => {
            for part in parts {
                predicate_scope(part)?;
            }
            Ok(())
        }
    }
}

impl Relationship {
    fn new(head: &ImplementationHead, budget: &mut Budget) -> Result<Self, Error> {
        budget.steps = super::applicability::validate(head, &head.subject, &[], *budget)
            .map_err(|_| Error::BudgetExhausted)?;
        budget.steps = budget
            .steps
            .checked_sub(head.type_parameters.len())
            .ok_or(Error::BudgetExhausted)?;
        let parameters = head
            .parameter_binders()
            .map_err(|e| Error::InvalidPhaseHead(e.to_string()))?;
        let bind = |term: &Term| {
            term.check_scope(&[])
                .map_err(|e| Error::InvalidPhaseHead(e.to_string()))?;
            term.abstract_parameters(&parameters)
                .map_err(|e| Error::InvalidPhaseHead(e.to_string()))
        };
        let arguments: Vec<_> = parameters
            .iter()
            .enumerate()
            .map(|(index, p)| Term::Bound {
                depth: 0,
                index,
                sort: p.sort,
            })
            .collect();
        let frames = vec![parameters.iter().map(|p| p.sort).collect()];
        let requirements = head
            .requirements
            .iter()
            .map(|p| {
                predicate_scope(p).map_err(|e| Error::InvalidPhaseHead(e.to_string()))?;
                p.instantiate_parameters_in_scope(&parameters, &arguments, &frames)
                    .map_err(|e| Error::InvalidPhaseHead(e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            interface: head.interface,
            domains: if head.parameter_types.is_empty() {
                vec![Term::Primitive(nir::Primitive::Type); parameters.len()]
            } else {
                head.parameter_types
                    .iter()
                    .map(bind)
                    .collect::<Result<_, _>>()?
            },
            subject: bind(&head.subject)?,
            arguments: head
                .interface_arguments
                .iter()
                .map(bind)
                .collect::<Result<_, _>>()?,
            requirements,
            sorts: parameters.iter().map(|parameter| parameter.sort).collect(),
        })
    }

    fn equivalent(&self, other: &Self, budget: &mut Budget) -> Result<bool, Error> {
        if self.interface != other.interface
            || self.domains != other.domains
            || self.subject != other.subject
            || self.arguments != other.arguments
            || self.sorts != other.sorts
        {
            return Ok(false);
        }
        if self.requirements == other.requirements {
            return Ok(true);
        }
        // Only the applicability conditions enter PTC. Head identity remains
        // structural, and no caller premises may make two declarations appear
        // equivalent in just one use site.
        use crate::ptc::{Context, Formula, Outcome, UnknownReason};
        let context = Context::with_scope(vec![self.sorts.clone()]);
        for (premise, conclusion) in [
            (&self.requirements, &other.requirements),
            (&other.requirements, &self.requirements),
        ] {
            for predicate in premise.iter().chain(conclusion) {
                nir::copy_budget::charge_predicate_substitution(
                    predicate,
                    &[],
                    &[],
                    &mut budget.steps,
                )
                .map_err(|_| Error::BudgetExhausted)?;
            }
            let premise = Formula::And(premise.iter().map(Formula::from_predicate).collect());
            let conclusion = Formula::And(conclusion.iter().map(Formula::from_predicate).collect());
            match context.prove_metered(premise.implies(conclusion), budget) {
                Outcome::Entailed(_) => {}
                Outcome::NotEntailed(_) => return Ok(false),
                Outcome::Unknown {
                    reason: UnknownReason::BudgetExhausted,
                    ..
                } => {
                    return Err(Error::BudgetExhausted);
                }
                Outcome::Unknown { reason, .. } => {
                    return Err(Error::UndeterminedPhaseRelationship(format!("{reason:?}")));
                }
                Outcome::IllFormed(message) => return Err(Error::InvalidPhaseHead(message)),
            }
        }
        Ok(true)
    }
}

impl Index {
    /// Admit a contribution to a common relationship. Equivalent normalized
    /// heads with complementary phases share one selection/coherence entry.
    /// Same-phase duplicates remain separate entries for ordinary coherence to
    /// reject. The returned identity names the relationship, not necessarily
    /// this contribution. All validation precedes publication.
    pub fn insert_phase(
        &mut self,
        head: ImplementationHead,
        phase: ImplementationPhase,
        mut budget: Budget,
    ) -> Result<DeclarationId, Error> {
        spend(&mut budget.steps)?;
        if self.contributions.contains(&head.declaration) {
            return Err(Error::DuplicateDeclaration(head.declaration));
        }
        self.check_ownership(&head, &mut budget.steps, budget.max_depth)?;
        let identity = Relationship::new(&head, &mut budget)?;
        let wanted = slot(phase);
        let mut complement = None;
        for (id, entry) in &self.entries {
            spend(&mut budget.steps)?;
            if head.interface.is_none()
                || entry.head.interface != head.interface
                || entry.phases[wanted].is_some()
            {
                continue;
            }
            let other = Relationship::new(&entry.head, &mut budget)?;
            if identity.equivalent(&other, &mut budget)? {
                if complement.replace(*id).is_some() {
                    return Err(Error::AmbiguousPhaseRelationship);
                }
            }
        }
        if let Some(id) = complement {
            self.contributions.insert(head.declaration);
            self.entries.get_mut(&id).unwrap().phases[wanted] = Some(head.declaration);
            self.revision += 1;
            return Ok(id);
        }
        let declaration = head.declaration;
        self.insert(head, budget)?;
        let entry = self.entries.get_mut(&declaration).unwrap();
        entry.phases = [None, None];
        entry.phases[wanted] = Some(declaration);
        Ok(declaration)
    }

    /// The type resolver selects across both phases. Never filter candidates
    /// by phase: a missing contribution on that winner is a terminal result.
    /// Returned type arguments belong positionally to either alpha-bound head;
    /// method/associated contracts and phase membership still need checking.
    pub fn select_phase(
        &self,
        resolution: &TypeResolution,
        phase: ImplementationPhase,
    ) -> PhaseSelection {
        if !self.type_resolution_is_current(resolution) {
            return PhaseSelection::Unselected(Selection::StaleIndex);
        }
        let Selection::Selected(relationship) = resolution.selection else {
            return PhaseSelection::Unselected(resolution.selection.clone());
        };
        let Some(entry) = self.entries.get(&relationship) else {
            return PhaseSelection::Unselected(Selection::InvalidCandidate(relationship));
        };
        match entry.phases[slot(phase)] {
            Some(contribution) => PhaseSelection::Selected {
                relationship,
                contribution,
                phase,
            },
            None => PhaseSelection::Missing {
                relationship,
                phase,
            },
        }
    }
}
