//! Contextual code witnesses shared by elaboration and comptime consumers.
//!
//! These are comptime descriptions, not runtime values or ordinary Type terms.
//! A missing body represents definition-side interface evidence only. It must
//! never be published as an executable quote. Context identities, fixed
//! dependencies, explicit ports and pattern success identities travel with
//! the body; copying a witness does not copy any runtime storage.
//!
//! Public construction is untrusted. Interface validation checks runtime
//! representation domains, not body typing, resource legality or substitution.
//! Those obligations remain with quotation and splice checking.
use crate::{
    Sort, Term, phases,
    program::{Expr, LocalId, Package, Pattern},
};
use std::{collections::BTreeMap, fmt, sync::Arc};

#[derive(Debug)]
pub struct ExpressionCode {
    pub body: Option<Arc<Expr>>,
    pub result: Term,
    /// All free runtime identities, including fixed dependencies.
    pub context: BTreeMap<LocalId, Term>,
    /// Only explicitly exposed ports may be redirected.
    pub ports: BTreeMap<String, LocalId>,
    pub defaults: BTreeMap<LocalId, LocalId>,
}

impl fmt::Display for ExpressionCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(expression-code (context")?;
        for (binding, ty) in &self.context {
            write!(f, " (binding {binding} {ty})")?;
        }
        f.write_str(") (ports")?;
        for (name, binding) in &self.ports {
            write!(f, " (port {name:?} {binding})")?;
        }
        f.write_str(") (defaults")?;
        for (port, actual) in &self.defaults {
            write!(f, " (connect {port} {actual})")?;
        }
        match &self.body {
            Some(body) => write!(f, ") (result {}) (body {}))", self.result, body),
            None => write!(
                f,
                ") (result {}) (body pending-code-parameter))",
                self.result
            ),
        }
    }
}

#[derive(Debug)]
pub struct PatternCode {
    pub subject: Term,
    pub outputs: BTreeMap<String, (LocalId, Term)>,
    /// Definition-side data inputs may be unavailable. Such evidence carries
    /// the checked interface, never an executable pattern with live captures.
    pub body: Option<Pattern>,
}

impl fmt::Display for PatternCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(pattern-code (subject {}) (outputs", self.subject)?;
        for (name, (binding, ty)) in &self.outputs {
            write!(f, " (output {name:?} {binding} {ty})")?;
        }
        match &self.body {
            Some(body) => write!(f, ") (body {body}))"),
            None => f.write_str(") (body pending-pattern-code))"),
        }
    }
}

impl ExpressionCode {
    /// Validate the transported interface without executing or expanding code.
    /// Defaults may refer to external lexical identities; only their source
    /// identities must belong to this context.
    pub fn validate_runtime_interface(
        &self,
        package: &Package,
        frames: &[Vec<Sort>],
    ) -> Result<(), &'static str> {
        if self.ports.values().any(|id| !self.context.contains_key(id)) {
            return Err("code port is absent from its runtime context");
        }
        if self
            .defaults
            .keys()
            .any(|id| !self.context.contains_key(id))
        {
            return Err("code default is absent from its runtime context");
        }
        for ty in self.context.values().chain(std::iter::once(&self.result)) {
            phases::validate_runtime_storage(ty, package, frames)?;
        }
        Ok(())
    }
}

impl PatternCode {
    /// Success outputs retain both their names and their binding identities.
    /// This checks phase admissibility, not whether a body binds those names.
    pub fn validate_runtime_interface(
        &self,
        package: &Package,
        frames: &[Vec<Sort>],
    ) -> Result<(), &'static str> {
        for ty in std::iter::once(&self.subject).chain(self.outputs.values().map(|(_, ty)| ty)) {
            phases::validate_runtime_storage(ty, package, frames)?;
        }
        Ok(())
    }
}
