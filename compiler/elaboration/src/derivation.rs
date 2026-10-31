//! Default-strategy registration and uniqueness, independent of impl precision.
//! Attribute evaluation, visibility and candidate-local elaboration are supplied
//! by the frontend. Registration alone never publishes an implementation.
use crate::implementation::{self, Error, Ownership};
use crate::ptc::Budget;
use nir::{DeclarationId, Term};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultStrategy {
    pub declaration: DeclarationId,
    pub interface: DeclarationId,
    pub interface_arguments: Vec<Term>,
}
impl fmt::Display for DefaultStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(default-strategy (declaration {}) (interface {}) (arguments",
            self.declaration, self.interface
        )?;
        for argument in &self.interface_arguments {
            write!(f, " {argument}")?;
        }
        f.write_str("))")
    }
}

/// Candidate-local elaboration must settle the complete interface, subject,
/// inputs and requirements before reporting Applicable. Unknown is Pending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applicability {
    Applicable,
    Rejected,
    Pending,
}

impl fmt::Display for Applicability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Applicable => "applicable",
            Self::Rejected => "rejected",
            Self::Pending => "pending",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    Missing,
    Unique(DeclarationId),
    Ambiguous(Vec<DeclarationId>),
    Pending {
        applicable: Vec<DeclarationId>,
        pending: Vec<DeclarationId>,
    },
}
impl fmt::Display for Selection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("(default-strategy-selection missing)"),
            Self::Unique(id) => write!(f, "(default-strategy-selection (unique {id}))"),
            Self::Ambiguous(ids) => {
                f.write_str("(default-strategy-selection (ambiguous")?;
                for id in ids {
                    write!(f, " {id}")?;
                }
                f.write_str("))")
            }
            Self::Pending {
                applicable,
                pending,
            } => {
                f.write_str("(default-strategy-selection (applicable")?;
                for id in applicable {
                    write!(f, " {id}")?;
                }
                f.write_str(") (pending")?;
                for id in pending {
                    write!(f, " {id}")?;
                }
                f.write_str("))")
            }
        }
    }
}

#[derive(Default)]
pub struct Registry {
    declarations: BTreeMap<DeclarationId, DefaultStrategy>,
    interfaces: BTreeMap<DeclarationId, BTreeSet<DeclarationId>>,
}
impl fmt::Display for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(default-strategy-registry")?;
        for strategy in self.declarations.values() {
            write!(f, " {strategy}")?;
        }
        f.write_str(")")
    }
}
impl Registry {
    /// Admission is atomic. Re-importing the same declaration is idempotent;
    /// reusing its identity for different metadata is rejected.
    pub fn register(
        &mut self,
        strategy: DefaultStrategy,
        budget: Budget,
    ) -> Result<Ownership, Error> {
        let mut remaining = budget;
        for argument in &strategy.interface_arguments {
            nir::type_computations::charge_term_with_depth(
                argument,
                &mut remaining.steps,
                remaining.max_depth,
            )
            .map_err(|_| Error::BudgetExhausted)?;
        }
        let ownership = implementation::default_strategy_ownership(
            strategy.declaration,
            strategy.interface,
            &strategy.interface_arguments,
            remaining,
        )?;
        if let Some(previous) = self.declarations.get(&strategy.declaration) {
            return if previous == &strategy {
                Ok(ownership)
            } else {
                Err(Error::DuplicateDeclaration(strategy.declaration))
            };
        }
        self.interfaces
            .entry(strategy.interface)
            .or_default()
            .insert(strategy.declaration);
        self.declarations.insert(strategy.declaration, strategy);
        Ok(ownership)
    }

    /// Assess only visible declarations of the requested interface head. The
    /// assessor receives full arguments and the same decreasing work budget.
    /// No precision, import-order or declaration-order tie-break is permitted.
    pub fn select(
        &self,
        interface: DeclarationId,
        visible: &BTreeSet<DeclarationId>,
        fuel: &mut usize,
        mut assess: impl FnMut(&DefaultStrategy, &mut usize) -> Result<Applicability, Error>,
    ) -> Result<Selection, Error> {
        *fuel = fuel.checked_sub(1).ok_or(Error::BudgetExhausted)?;
        let mut applicable = Vec::new();
        let mut pending = Vec::new();
        for id in self.interfaces.get(&interface).into_iter().flatten() {
            *fuel = fuel.checked_sub(1).ok_or(Error::BudgetExhausted)?;
            if !visible.contains(id) {
                continue;
            }
            match assess(&self.declarations[id], fuel)? {
                Applicability::Applicable => applicable.push(*id),
                Applicability::Pending => pending.push(*id),
                Applicability::Rejected => (),
            }
        }
        Ok(if applicable.len() > 1 {
            Selection::Ambiguous(applicable)
        } else if !pending.is_empty() {
            Selection::Pending {
                applicable,
                pending,
            }
        } else if let Some(id) = applicable.first() {
            Selection::Unique(*id)
        } else {
            Selection::Missing
        })
    }
}
