//! Region-owned return and loop state, and conservative completion checks.
use super::*;

/// Active source control state. Independent callable, declaration and staging
/// boundaries move this entire context out; speculative checks clone it. This
/// owns no VM execution, capture storage or inference transaction authority.
#[derive(Clone, Default)]
pub(super) struct Context {
    pub(super) result: Option<Term>,
    pub(super) result_guarantees: Option<(DeclarationId, Vec<n::TypePredicate>)>,
    pub(super) partial_return_context: Option<crate::calls::InputContext>,
    // Missing returns completed before exporting the enclosing callback.
    pub(super) pending_optional_returns: BTreeMap<(u32, u32), n::SourceRange>,
    // Propagation sites completed once the callback result has been inferred.
    pub(super) pending_error_returns: BTreeMap<(u32, u32), Term>,
    pub(super) inferred_return_sites: BTreeSet<(u32, u32)>,
    pub(super) return_observations: Option<ReturnObservations>,
    pub(super) loops: Vec<n::LocalId>,
}

/// A return scope can infer a closure's result, or merely check an open
/// definition whose result domain is still suspended. The latter must not
/// unify distinct source exits before the invocation inputs are available.
#[derive(Clone)]
pub(super) enum ReturnObservations {
    Infer(Vec<Term>),
    Deferred,
}
impl ReturnObservations {
    pub(super) fn iter(&self) -> std::slice::Iter<'_, Term> {
        match self {
            Self::Infer(types) => types.iter(),
            Self::Deferred => [].iter(),
        }
    }
    pub(super) fn push(&mut self, ty: Term) {
        if let Self::Infer(types) = self {
            types.push(ty);
        }
    }
}

/// Definition-side counterpart of `boolean`: inspect only literal Boolean
/// trees, without interpreting staged inputs or invoking source computations.
pub(super) fn source_boolean(expr: &Expr<'_>, fuel: &mut u64, depth: usize) -> Option<bool> {
    *fuel = fuel.checked_sub(1)?;
    if depth >= 64 {
        return None;
    }
    match &expr.kind {
        H::Lit(literal) => match literal.kind {
            hir::common::LitKind::Bool(value) => Some(value),
            _ => None,
        },
        H::Unary(UnOp::Not, value) => Some(!source_boolean(value, fuel, depth + 1)?),
        H::Binary(BinOp::And, left, right) => {
            Some(source_boolean(left, fuel, depth + 1)? && source_boolean(right, fuel, depth + 1)?)
        }
        H::Binary(BinOp::Or, left, right) => {
            Some(source_boolean(left, fuel, depth + 1)? || source_boolean(right, fuel, depth + 1)?)
        }
        _ => None,
    }
}

pub(super) use crate::verification::control::{
    boolean, conditional_never, freshen_loop_sites, loop_may_complete, loop_may_continue,
    pattern_matches,
};

impl std::fmt::Display for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(control-context (result")?;
        if let Some(result) = &self.result {
            write!(f, " {result}")?;
        }
        f.write_str(") (partial-return")?;
        if let Some(context) = &self.partial_return_context {
            write!(f, " {context}")?;
        }
        f.write_str(") (pending-optional")?;
        for ((start, end), source) in &self.pending_optional_returns {
            write!(f, " (return (source {start} {end}) (absence {source}))")?;
        }
        f.write_str(") (pending-errors")?;
        for ((start, end), ty) in &self.pending_error_returns {
            write!(f, " (return (source {start} {end}) (type {ty}))")?;
        }
        f.write_str(") (inferred-returns")?;
        for (start, end) in &self.inferred_return_sites {
            write!(f, " (source {start} {end})")?;
        }
        f.write_str(") (observations")?;
        if let Some(observations) = &self.return_observations {
            write!(f, " {observations}")?;
        }
        f.write_str(") (loops")?;
        for target in &self.loops {
            write!(f, " {target}")?;
        }
        f.write_str("))")
    }
}

impl std::fmt::Display for ReturnObservations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Deferred => f.write_str("(deferred)"),
            Self::Infer(types) => {
                f.write_str("(infer")?;
                for ty in types {
                    write!(f, " {ty}")?;
                }
                f.write_str(")")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_dump_keeps_pending_sites_and_partial_context_structure() {
        let mut context = Context {
            partial_return_context: Some(crate::calls::InputContext::Optional(Box::new(
                crate::calls::InputContext::Record(vec![(
                    "name\"(\n".into(),
                    crate::calls::InputContext::UnknownDependentValue,
                )]),
            ))),
            return_observations: Some(ReturnObservations::Deferred),
            ..Context::default()
        };
        context
            .pending_optional_returns
            .insert((8, 12), n::SourceRange { start: 10, end: 12 });
        context
            .pending_optional_returns
            .insert((1, 4), n::SourceRange { start: 2, end: 4 });
        context.pending_error_returns.insert((20, 24), p(P::Bool));
        context.inferred_return_sites.insert((30, 35));
        let dump = context.to_string();
        assert_eq!(
            dump,
            format!(
                "(control-context (result) (partial-return (optional (record (field \"name\\\"(\\n\" (unknown-dependent-value))))) \
            (pending-optional (return (source 1 4) (absence (source 2 4))) (return (source 8 12) (absence (source 10 12)))) \
            (pending-errors (return (source 20 24) (type {}))) (inferred-returns (source 30 35)) \
            (observations (deferred)) (loops))",
                p(P::Bool)
            )
        );
        assert_eq!(dump, context.to_string());
    }
}
