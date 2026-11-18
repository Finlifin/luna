//! The same declaration slots can be supplied in different source orders.
//! A checked schedule exposes preparations and inputs in execution order and
//! identifies exactly the locals that belong to this evaluation.
use super::{
    Expr, ExprKind, LocalId, NormalFormArgument, NormalFormInputPlan, NormalFormPreparation,
};
use std::collections::BTreeSet;
use std::fmt;

/// A position in a contextual operand. Branches, match arms, and container
/// fields use their source index. A block result is its only contextual child.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct InputPath(std::sync::Arc<[usize]>);

impl InputPath {
    pub fn from_indices(indices: &[usize]) -> Self {
        Self(indices.into())
    }

    pub fn child(&self, index: usize) -> Self {
        Self(
            self.0
                .iter()
                .copied()
                .chain(std::iter::once(index))
                .collect(),
        )
    }
}

impl fmt::Display for InputPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(input-path")?;
        for index in self.0.iter() {
            write!(f, " {index}")?;
        }
        f.write_str(")")
    }
}

#[derive(Clone, Copy, Default)]
pub struct InputSchedule<'a> {
    pub order: Option<&'a [usize]>,
    pub bindings: Option<&'a [LocalId]>,
    pub preparations: &'a [NormalFormPreparation],
}

pub struct CheckedInputSchedule<'a> {
    schedule: InputSchedule<'a>,
    count: usize,
    pub locals: Vec<LocalId>,
}

pub enum InputStep<E> {
    Prepare(E),
    Supply(usize),
}

impl<'a> InputSchedule<'a> {
    pub fn normal(plan: Option<&'a NormalFormInputPlan>) -> Self {
        plan.map_or_else(Self::default, |plan| Self {
            order: Some(&plan.order),
            bindings: Some(&plan.bindings),
            preparations: &plan.preparations,
        })
    }

    /// Charge the owning operation before traversing or allocating each part.
    /// A malformed plan exports no bindings. An absent order denotes declaration order.
    pub fn check<E>(
        self,
        count: usize,
        mut charge: impl FnMut(usize) -> Result<(), E>,
    ) -> Result<Option<CheckedInputSchedule<'a>>, E> {
        if self.order.is_some_and(|order| order.len() != count)
            || self
                .bindings
                .is_some_and(|bindings| bindings.len() != count)
        {
            return Ok(None);
        }
        if let Some(order) = self.order {
            charge(order.len())?;
            let mut seen = vec![false; count];
            for &slot in order {
                if slot >= count || std::mem::replace(&mut seen[slot], true) {
                    return Ok(None);
                }
            }
        }
        let bindings = self.bindings.unwrap_or_default();
        charge(bindings.len().saturating_add(self.preparations.len()))?;
        let mut locals = bindings.to_vec();
        let mut previous = 0;
        for preparation in self.preparations {
            if preparation.before < previous || preparation.before > count {
                return Ok(None);
            }
            previous = preparation.before;
            if !preparation.visit_bindings(&mut charge, |binding| locals.push(binding))? {
                return Ok(None);
            }
        }
        let mut unique = BTreeSet::new();
        if locals.iter().any(|id| !unique.insert(*id)) {
            return Ok(None);
        }
        Ok(Some(CheckedInputSchedule {
            schedule: self,
            count,
            locals,
        }))
    }
}

impl<E> InputStep<E> {
    /// Interleave an already routed slot sequence with its spread producers.
    /// Elaboration moves owned expressions; inspections borrow the same steps.
    pub fn in_order(
        mut slots: impl Iterator<Item = usize>,
        preparations: impl Iterator<Item = (usize, E)>,
    ) -> impl Iterator<Item = Self> {
        let mut preparations = preparations.peekable();
        let mut position = 0;
        std::iter::from_fn(move || {
            if let Some((_, expr)) = preparations.next_if(|(before, _)| *before == position) {
                return Some(Self::Prepare(expr));
            }
            let slot = slots.next()?;
            position += 1;
            Some(Self::Supply(slot))
        })
    }
}

impl<'a> CheckedInputSchedule<'a> {
    /// Binding for a declaration slot yielded by this schedule.
    pub fn binding(&self, slot: usize) -> Option<LocalId> {
        self.schedule.bindings.map(|ids| ids[slot])
    }

    pub fn steps(&self) -> impl Iterator<Item = InputStep<&'a Expr>> + '_ {
        let slots = (0..self.count).map(|position| {
            self.schedule
                .order
                .map_or(position, |order| order[position])
        });
        InputStep::in_order(
            slots,
            self.schedule
                .preparations
                .iter()
                .map(|p| (p.before, &p.expression)),
        )
    }
}

impl ExprKind {
    pub fn declaration_inputs(&self) -> Option<(&[NormalFormArgument], InputSchedule<'_>)> {
        match self {
            Self::NormalFormType {
                arguments, plan, ..
            }
            | Self::ApplyNormalForm {
                arguments, plan, ..
            } => Some((arguments, InputSchedule::normal(plan.as_ref()))),
            Self::NominalType {
                arguments,
                order,
                bindings,
                preparations,
                ..
            }
            | Self::CheckTypeAlias {
                arguments,
                order,
                bindings,
                preparations,
                ..
            } => Some((
                arguments,
                InputSchedule {
                    order: order.as_deref(),
                    bindings: bindings.as_deref(),
                    preparations,
                },
            )),
            _ => None,
        }
    }
}

impl fmt::Display for InputSchedule<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(input-plan (order")?;
        if let Some(order) = self.order {
            for slot in order {
                write!(f, " {slot}")?;
            }
        } else {
            f.write_str(" declaration")?;
        }
        f.write_str(") (bindings")?;
        for id in self.bindings.unwrap_or_default() {
            write!(f, " {id}")?;
        }
        f.write_str(") (preparations")?;
        for preparation in self.preparations {
            write!(
                f,
                " (before {} {})",
                preparation.before, preparation.expression
            )?;
        }
        f.write_str("))")
    }
}

impl fmt::Display for CheckedInputSchedule<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(checked-input-schedule (arity {}) {} (locals",
            self.count, self.schedule
        )?;
        for id in &self.locals {
            write!(f, " {id}")?;
        }
        f.write_str("))")
    }
}

impl<E: fmt::Display> fmt::Display for InputStep<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prepare(expr) => write!(f, "(prepare {expr})"),
            Self::Supply(slot) => write!(f, "(supply {slot})"),
        }
    }
}
