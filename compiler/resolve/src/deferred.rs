//! Lexical lookup plans for scopes whose generated exports are not ready yet.
//! A pending scope is not an absent binding. Plans preserve ordinary fallback
//! errors and lexical order without guessing exports from unrelated declarations.
use crate::{
    binding::{BindingKind, Resolution},
    error::ResolveResult,
    ids::AstNodeRef,
};
use std::fmt;
use symbol::Symbol;

#[derive(Clone, Debug)]
pub enum OutputBinding {
    Pending,
    Missing,
    Resolved(Resolution),
}

#[derive(Clone, Debug)]
pub struct NameResolutionPlan {
    pub name: Symbol,
    /// Innermost first, stopping at the first ordinary lexical binding.
    pub pending_outputs: Vec<AstNodeRef>,
    pub fallback: ResolveResult<Resolution>,
}

impl NameResolutionPlan {
    /// None means a nearer scope is still pending: never inspect an outer
    /// binding until that scope has authoritatively reported Missing.
    /// The immutable plan can be retried after pending work or a failed query.
    pub fn resolve_with(
        &self,
        mut lookup: impl FnMut(AstNodeRef, Symbol) -> ResolveResult<OutputBinding>,
    ) -> ResolveResult<Option<Resolution>> {
        for &site in &self.pending_outputs {
            match lookup(site, self.name)? {
                OutputBinding::Pending => return Ok(None),
                OutputBinding::Missing => {}
                OutputBinding::Resolved(binding) => return Ok(Some(binding)),
            }
        }
        self.fallback.clone().map(Some)
    }
}

fn site(f: &mut fmt::Formatter<'_>, value: AstNodeRef) -> fmt::Result {
    write!(f, "(source {} {})", value.file.raw(), value.node)
}
fn binding(f: &mut fmt::Formatter<'_>, value: BindingKind) -> fmt::Result {
    match value {
        BindingKind::Definition(id, scope) => write!(
            f,
            "(definition {} {} (scope {}))",
            id.pkg,
            id.index,
            scope.raw()
        ),
        BindingKind::FnParam(value) => {
            write!(f, "(fn-param ")?;
            site(f, value)?;
            write!(f, ")")
        }
        BindingKind::Local(value) => {
            write!(f, "(local ")?;
            site(f, value)?;
            write!(f, ")")
        }
        BindingKind::ExpandedLocal(value, output) => {
            write!(f, "(expanded-local ")?;
            site(f, value)?;
            write!(f, " (output {output}))")
        }
        BindingKind::ClauseParam(id, value) => {
            write!(f, "(clause-param {} {} ", id.pkg, id.index)?;
            site(f, value)?;
            write!(f, ")")
        }
        BindingKind::LoopLabel(value) => {
            write!(f, "(loop-label ")?;
            site(f, value)?;
            write!(f, ")")
        }
    }
}
impl fmt::Display for NameResolutionPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(name-resolution (name {:?}) (pending",
            self.name.as_str()
        )?;
        for &value in &self.pending_outputs {
            write!(f, " ")?;
            site(f, value)?;
        }
        write!(f, ") (fallback ")?;
        match &self.fallback {
            Ok(value) => binding(f, value.kind)?,
            Err(error) => write!(f, "(error {:?})", error.message())?,
        }
        write!(f, "))")
    }
}

impl fmt::Display for OutputBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(output-binding ")?;
        match self {
            Self::Pending => write!(f, "pending")?,
            Self::Missing => write!(f, "missing")?,
            Self::Resolved(value) => binding(f, value.kind)?,
        }
        write!(f, ")")
    }
}
