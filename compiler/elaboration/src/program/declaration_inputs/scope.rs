//! Caller lexical state retained for one declaration-input preparation.
//! This is not a checking transaction: input facts and storage invalidations
//! keep their existing owners and are not restored with lexical bindings.
use super::*;

#[must_use = "leave the input scope in its original checker"]
pub(super) struct Scope {
    locals: BTreeMap<n::LocalId, Term>,
    bindings: BTreeMap<DeclarationId, (n::LocalId, Term)>,
    types: BTreeMap<DeclarationId, std::sync::Arc<Term>>,
    type_values: BTreeMap<DeclarationId, n::LocalId>,
    context: crate::ptc::Context,
    owner: Option<OwnerId>,
    mode: bool,
    input_scope: Option<Vec<(Sort, Option<Term>)>>,
    input_owner: Option<OwnerId>,
}

impl Scope {
    pub(super) fn enter(checker: &mut Checker<'_, '_>) -> Self {
        let locals = checker.locals.clone();
        let bindings = checker.normal_default_bindings.clone();
        let types = checker.alias_substitution.clone();
        let type_values = checker.alias_value_bindings.clone();
        let context = checker.context.clone();
        let owner = checker.signature_owner;
        let mode = checker.in_required_comptime;
        let caller_scope = checker.call_inference_domains();
        let caller_owner = checker.default_input_owner.or(checker.signature_owner);
        let input_owner = std::mem::replace(&mut checker.default_input_owner, caller_owner);
        let input_scope = checker.default_input_scope.replace(caller_scope);
        Self {
            locals,
            bindings,
            types,
            type_values,
            context,
            owner,
            mode,
            input_scope,
            input_owner,
        }
    }

    pub(super) fn leave(self, checker: &mut Checker<'_, '_>) {
        checker.locals = self.locals;
        checker.normal_default_bindings = self.bindings;
        checker.alias_substitution = self.types;
        checker.alias_value_bindings = self.type_values;
        checker.context = self.context;
        checker.signature_owner = self.owner;
        checker.in_required_comptime = self.mode;
        checker.default_input_scope = self.input_scope;
        checker.default_input_owner = self.input_owner;
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-input-scope (comptime {}) (owner",
            self.mode
        )?;
        if let Some(owner) = self.owner {
            write!(f, " {owner}")?;
        }
        f.write_str(") (input-owner")?;
        if let Some(owner) = self.input_owner {
            write!(f, " {owner}")?;
        }
        f.write_str(") (locals")?;
        for (binding, ty) in &self.locals {
            write!(f, " (binding {binding} {ty})")?;
        }
        f.write_str(") (defaults")?;
        for (parameter, (binding, ty)) in &self.bindings {
            write!(f, " (binding {parameter} {binding} {ty})")?;
        }
        f.write_str(") (types")?;
        for (parameter, ty) in &self.types {
            write!(f, " (binding {parameter} {ty})")?;
        }
        f.write_str(") (type-values")?;
        for (parameter, binding) in &self.type_values {
            write!(f, " (binding {parameter} {binding})")?;
        }
        f.write_str(") (input-scope")?;
        if let Some(inputs) = &self.input_scope {
            for (sort, ty) in inputs {
                write!(f, " (input {sort}")?;
                if let Some(ty) = ty {
                    write!(f, " {ty}")?;
                }
                f.write_str(")")?;
            }
        }
        write!(f, ") {})", self.context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_scope_dump_preserves_owner_domains_and_escaped_values() {
        let owner = hir::OwnerId::new(hir::LocalDefId::new(1));
        let id = DeclarationId {
            package: 2,
            local: 3,
        };
        let local = n::LocalId { owner: 1, index: 4 };
        let scope = Scope {
            locals: BTreeMap::from([(local, p(P::Str))]),
            bindings: BTreeMap::from([(id, (local, p(P::Str)))]),
            types: BTreeMap::from([(id, std::sync::Arc::new(Term::String("caller\"\n".into())))]),
            type_values: BTreeMap::from([(id, local)]),
            context: crate::ptc::Context::default(),
            owner: Some(owner),
            mode: true,
            input_scope: Some(vec![
                (Sort::Type, None),
                (Sort::ComptimeValue, Some(p(P::Str))),
            ]),
            input_owner: Some(owner),
        };
        let dump = scope.to_string();
        for fragment in [
            "(declaration-input-scope (comptime true)",
            "(locals (binding ",
            "(defaults (binding ",
            "(type-values (binding ",
            "(input-scope (input ",
            "caller\\\"\\n",
        ] {
            assert!(dump.contains(fragment), "missing {fragment}: {dump}");
        }
        assert!(!dump.contains('\n'), "{dump}");
        assert_eq!(dump, scope.to_string());
    }
}
