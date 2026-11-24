//! Function safety tiers, loop/assertion predicate roles, and termination
//! decreases clauses stored alongside NIR declarations.
use crate::program::{Expr, SourceRange};
use crate::{DeclarationId, Term};
use std::collections::BTreeMap;
use std::fmt;

/// Four-tier safety classification of a Flurry function declaration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SafetyLevel {
    #[default]
    Safe,
    Spec,
    Verified,
    Unsafe,
}

impl fmt::Display for SafetyLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Safe => "safe",
            Self::Spec => "spec",
            Self::Verified => "verified",
            Self::Unsafe => "unsafe",
        })
    }
}

/// Role of a lowered `RequirePredicate` / `EnsurePredicate` obligation in NIR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PredicateRole {
    Assert,
    Invariant,
    LoopDecreases,
}

impl fmt::Display for PredicateRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Assert => "assert",
            Self::Invariant => "invariant",
            Self::LoopDecreases => "loop-decreases",
        })
    }
}

/// Package-wide safety tiers, `decreases` clauses, and statement-level
/// verification predicate roles (`asserts`, `invariant`, `decreases`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SafetyContracts {
    pub levels: BTreeMap<DeclarationId, SafetyLevel>,
    pub decreases: BTreeMap<DeclarationId, Vec<Expr>>,
    pub predicate_roles: BTreeMap<SourceRange, PredicateRole>,
}

impl SafetyContracts {
    pub fn is_empty(&self) -> bool {
        self.levels.is_empty() && self.decreases.is_empty() && self.predicate_roles.is_empty()
    }

    pub fn entry_count(&self) -> usize {
        self.levels
            .len()
            .saturating_add(self.decreases.len())
            .saturating_add(self.decreases.values().map(Vec::len).sum::<usize>())
            .saturating_add(self.predicate_roles.len())
    }

    pub fn level(&self, declaration: DeclarationId) -> SafetyLevel {
        self.levels
            .get(&declaration)
            .copied()
            .unwrap_or(SafetyLevel::Safe)
    }

    pub fn is_spec(&self, declaration: DeclarationId) -> bool {
        self.level(declaration) == SafetyLevel::Spec
    }

    pub fn is_verified(&self, declaration: DeclarationId) -> bool {
        self.level(declaration) == SafetyLevel::Verified
    }

    pub fn decreases_for(&self, declaration: DeclarationId) -> &[Expr] {
        self.decreases.get(&declaration).map_or(&[], Vec::as_slice)
    }

    pub fn role(&self, source: SourceRange) -> Option<PredicateRole> {
        self.predicate_roles.get(&source).copied()
    }

    pub fn rewrite_decreases(
        &mut self,
        declaration: DeclarationId,
        mut transform: impl FnMut(&Term) -> Result<Term, crate::TypeError>,
    ) -> Result<(), crate::TypeError> {
        if let Some(exprs) = self.decreases.get_mut(&declaration) {
            for expr in exprs {
                expr.rewrite_types(&mut transform)?;
            }
        }
        Ok(())
    }
}

impl fmt::Display for SafetyContracts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(safety-contracts (levels")?;
        for (declaration, level) in &self.levels {
            write!(f, " (function {declaration} {level})")?;
        }
        f.write_str(") (decreases")?;
        for (declaration, exprs) in &self.decreases {
            write!(f, " (function {declaration}")?;
            for expr in exprs {
                write!(f, " {expr}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(") (predicate-roles")?;
        for (source, role) in &self.predicate_roles {
            write!(f, " (role {source} {role})")?;
        }
        f.write_str("))")
    }
}
