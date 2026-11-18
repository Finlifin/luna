//! Checked, world-owned requirements carried by gradual Type values.
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
    base: Option<usize>,
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

/// Immutable snapshots share the published prefix. An independent compilation
/// or a sibling snapshot cannot claim a requirement by reproducing its index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refinements(Arc<Vec<Refinement>>);

impl Refinements {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn contains(&self, description: &DescriptionLiteral) -> bool {
        match description {
            DescriptionLiteral::Any => true,
            DescriptionLiteral::Refined(reference) => {
                self.0.get(reference.0.slot) == Some(reference)
            }
        }
    }

    pub fn requirements<'a>(
        &'a self,
        description: &DescriptionLiteral,
    ) -> Option<impl Iterator<Item = BoundRequirement<'a>>> {
        if !self.contains(description) {
            return None;
        }
        let first = match description {
            DescriptionLiteral::Any => None,
            DescriptionLiteral::Refined(reference) => self.0.get(reference.0.slot),
        };
        Some(
            std::iter::successors(first, |item| item.0.base.and_then(|slot| self.0.get(slot)))
                .map(|handle| BoundRequirement(&handle.0)),
        )
    }
    pub fn publish(
        &mut self,
        base: DescriptionLiteral,
        binder: DeclarationParameter,
        requirement: TypePredicate,
        origin: SourceRange,
        steps: &mut u64,
    ) -> Result<DescriptionLiteral, String> {
        if !self.contains(&base) {
            return Err("refinement base belongs to a different declaration world".into());
        }
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
        let base = match base {
            DescriptionLiteral::Any => None,
            DescriptionLiteral::Refined(parent) => Some(parent.0.slot),
        };
        let handle = Refinement(Arc::new(Entry {
            slot: self.0.len(),
            base,
            binder,
            requirement,
            origin,
        }));
        Arc::make_mut(&mut self.0).push(handle.clone());
        Ok(DescriptionLiteral::Refined(handle))
    }
}
impl fmt::Display for Refinements {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(refinements")?;
        for reference in self.0.iter() {
            let item = &reference.0;
            write!(
                f,
                "\n (refinement-definition {} (origin {}) (base ",
                item.slot, item.origin
            )?;
            match item.base {
                Some(slot) => write!(f, "(refinement {slot})")?,
                None => f.write_str("any")?,
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
