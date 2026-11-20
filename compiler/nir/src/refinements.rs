//! Checked, world-owned requirements carried by gradual Type values.
use crate::described_applications::DescribedApplication;
use crate::program::{DescriptionLiteral, SourceRange, TypePredicate};
use crate::{DeclarationParameter, Sort};
use std::{
    fmt,
    hash::{Hash, Hasher},
    sync::Arc,
};

/// A requirement binds a Type subject. It carries no implementation selection.
#[derive(Debug)]
struct Entry {
    slot: usize,
    binder: DeclarationParameter,
    requirement: TypePredicate,
    origin: SourceRange,
}

/// Clones share one immutable binding. A slot number alone grants no access.
#[derive(Clone, Debug)]
pub struct Refinement(Arc<Entry>);

impl PartialEq for Refinement {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Refinement {}
impl Hash for Refinement {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}
impl fmt::Display for Refinement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(refinement {})", self.0.slot)
    }
}
/// Only a registry that owns the handle can lend a requirement for use.
pub struct BoundRequirement<'a>(&'a Entry);
impl BoundRequirement<'_> {
    pub fn instantiate(
        &self,
        actual: &crate::Term,
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<TypePredicate, crate::type_computations::Error> {
        crate::nominal_parameters::DeclarationSubstitution::bounded(
            &[self.0.binder.declaration],
            &[],
            std::slice::from_ref(actual),
            frames,
            fuel,
        )?
        .instantiate_predicate_bounded(&self.0.requirement, fuel)
    }
}

/// The registry owns the base edge. A literal handle never owns earlier
/// registry entries, even when a field refers to another refined application.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Published {
    requirement: Refinement,
    parent: Option<usize>,
    application: Option<DescribedApplication>,
}

pub struct Subject<'a> {
    pub application: Option<&'a DescribedApplication>,
    requirements: Vec<BoundRequirement<'a>>,
}

impl<'a> Subject<'a> {
    /// The base must hold before predicates on the same subject apply.
    pub fn requirements(self) -> impl DoubleEndedIterator<Item = BoundRequirement<'a>> {
        self.requirements.into_iter().rev()
    }

    /// Choose a rigid `DeclarationId` under `package = u64::MAX` that does not
    /// collide with any binder or free `u64::MAX` declaration in this subject's
    /// requirements.
    pub fn fresh_subject_id(
        &self,
        fuel: &mut usize,
    ) -> Result<crate::DeclarationId, crate::type_computations::Error> {
        let mut used = std::collections::BTreeSet::new();
        let mut predicates = Vec::with_capacity(self.requirements.len());
        let mut terms = Vec::new();
        for req in &self.requirements {
            *fuel = fuel
                .checked_sub(1)
                .ok_or(crate::type_computations::Error::Budget)?;
            if req.0.binder.declaration.package == u64::MAX {
                used.insert(req.0.binder.declaration.local);
            }
            predicates.push(&req.0.requirement);
        }
        while let Some(predicate) = predicates.pop() {
            *fuel = fuel
                .checked_sub(1)
                .ok_or(crate::type_computations::Error::Budget)?;
            match predicate {
                TypePredicate::Bool(_) => {}
                TypePredicate::Trait(req) => {
                    terms.push(&req.subject);
                    terms.extend(&req.interface_arguments);
                }
                TypePredicate::Equal(left, right) => {
                    terms.push(left);
                    terms.push(right);
                }
                TypePredicate::Kind(ty, _) => terms.push(ty),
                TypePredicate::Not(inner) => predicates.push(inner),
                TypePredicate::And(parts) | TypePredicate::Or(parts) => {
                    predicates.extend(parts.iter());
                }
            }
        }
        while let Some(term) = terms.pop() {
            *fuel = fuel
                .checked_sub(1)
                .ok_or(crate::type_computations::Error::Budget)?;
            if let crate::Term::Rigid { declaration, .. } = term
                && declaration.package == u64::MAX
            {
                used.insert(declaration.local);
            }
            term.visit_children::<crate::type_computations::Error>(|child| {
                terms.push(child);
                Ok(())
            })?;
        }
        for local in 0..=u32::MAX {
            *fuel = fuel
                .checked_sub(1)
                .ok_or(crate::type_computations::Error::Budget)?;
            if !used.contains(&local) {
                return Ok(crate::DeclarationId {
                    package: u64::MAX,
                    local,
                });
            }
        }
        Err(crate::type_computations::Error::Budget)
    }
}

/// Immutable snapshots share the published prefix. An independent compilation
/// or a sibling snapshot cannot claim a requirement by reproducing its index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refinements(Arc<Vec<Published>>, Arc<()>);

impl Refinements {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn contains_refinement(&self, r: &Refinement) -> bool {
        self.0
            .get(r.0.slot)
            .is_some_and(|entry| entry.requirement == *r)
    }
    pub fn contains(&self, description: &DescriptionLiteral) -> bool {
        match description {
            DescriptionLiteral::Application(a) => a.belongs_to(&self.1, self),
            DescriptionLiteral::Any => true,
            DescriptionLiteral::Refined(reference) => self.contains_refinement(reference),
        }
    }

    /// Observe the checked base head without opening its inputs or proving
    /// predicates. Publication preserves this observation through refinements.
    pub fn structure<'a>(
        &'a self,
        description: &'a DescriptionLiteral,
    ) -> Result<Option<&'a DescribedApplication>, &'static str> {
        if !self.contains(description) {
            return Err("description belongs to a different declaration world");
        }
        Ok(match description {
            DescriptionLiteral::Any => None,
            DescriptionLiteral::Application(application) => Some(application),
            DescriptionLiteral::Refined(r) => self.0[r.0.slot].application.as_ref(),
        })
    }

    /// Peel predicates on one subject. Each field introduces a new subject.
    pub fn subject<'a>(
        &'a self,
        description: &'a DescriptionLiteral,
        steps: &mut u64,
    ) -> Result<Subject<'a>, &'static str> {
        let application = self.structure(description)?;
        let mut requirements = Vec::new();
        if let DescriptionLiteral::Refined(r) = description {
            let mut slot = Some(r.0.slot);
            while let Some(index) = slot {
                *steps = steps
                    .checked_sub(1)
                    .ok_or("refinement admission budget exhausted")?;
                let published = &self.0[index];
                requirements.push(BoundRequirement(&published.requirement.0));
                slot = published.parent;
            }
        }
        Ok(Subject {
            application,
            requirements,
        })
    }
    pub fn application(
        &self,
        head: crate::DeclarationId,
        inputs: Vec<crate::described_applications::DataArgument>,
        fuel: &mut usize,
    ) -> Result<DescriptionLiteral, &'static str> {
        crate::described_applications::DescribedApplication::new(
            &self.1,
            head,
            inputs,
            self,
            self.0.last().map(|entry| entry.requirement.clone()),
            fuel,
        )
        .map(DescriptionLiteral::Application)
    }
    pub fn publish(
        &mut self,
        base: DescriptionLiteral,
        binder: DeclarationParameter,
        requirement: TypePredicate,
        origin: SourceRange,
        steps: &mut u64,
    ) -> Result<DescriptionLiteral, String> {
        let application = self.structure(&base)?.cloned();
        let parent = match base {
            DescriptionLiteral::Refined(r) => Some(r.0.slot),
            _ => None,
        };
        if binder.sort != Sort::Type
            || !crate::copy_budget::closed_refinement(&requirement, binder, steps)
                .map_err(|e| format!("refinement publication limit: {e:?}"))?
        {
            return Err("refinement requires a captured input outside its subject binder".into());
        }
        // Copying the shared prefix copies handles only.
        let cost = if Arc::strong_count(&self.0) == 1 {
            1
        } else {
            self.0.len() as u64 + 1
        };
        *steps = steps
            .checked_sub(cost)
            .ok_or("refinement publication budget exhausted")?;
        let handle = Refinement(Arc::new(Entry {
            slot: self.0.len(),
            binder,
            requirement,
            origin,
        }));
        Arc::make_mut(&mut self.0).push(Published {
            requirement: handle.clone(),
            parent,
            application,
        });
        Ok(DescriptionLiteral::Refined(handle))
    }
}
impl fmt::Display for Refinements {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(refinements")?;
        for published in self.0.iter() {
            let item = &published.requirement.0;
            write!(
                f,
                "\n (refinement-definition {} (origin {}) (base ",
                item.slot, item.origin
            )?;
            match (published.parent, &published.application) {
                (Some(parent), _) => write!(f, "(refinement {parent})")?,
                (None, Some(application)) => write!(f, "{application}")?,
                (None, None) => f.write_str("any")?,
            }
            write!(
                f,
                ") (subject {}) (requires {}))",
                item.binder, item.requirement
            )?;
        }
        f.write_str(")")
    }
}
