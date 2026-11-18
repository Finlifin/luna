//! Caller lexical state retained for one declaration-input preparation.
//! This is not a checking transaction: input facts and storage invalidations
//! keep their existing owners and are not restored with lexical bindings.
use super::*;

#[must_use = "leave the input scope in its original checker"]
pub(super) struct Scope {
    locals: LocalBindings<Term>,
    environment: environment::Environment,
    owner: Option<OwnerId>,
    mode: bool,
    input_scope: Option<lexical_domains::RetainedDomains>,
    input_owner: Option<OwnerId>,
}

impl Scope {
    pub(super) fn enter(checker: &mut Checker<'_, '_>, span: Span) -> Result<Self, Error> {
        // No lexical state changes until every retained component is ready.
        // Failure still consumes its work budget, as ordinary checking does.
        let caller_scope = checker.retain_input_domains(span)?;
        let environment = environment::Environment::retain(&nominal::InputEnvironment {
            substitution: &checker.alias_substitution,
            bindings: &checker.alias_value_bindings,
            normal_bindings: &checker.normal_default_bindings,
            context: &checker.context,
        });
        let caller_owner = checker.default_input_owner.or(checker.signature_owner);
        let input_owner = std::mem::replace(&mut checker.default_input_owner, caller_owner);
        let input_scope = checker.default_input_scope.replace(caller_scope);
        Ok(Self {
            locals: checker.locals.clone(),
            environment,
            owner: checker.signature_owner,
            mode: checker.in_required_comptime,
            input_scope,
            input_owner,
        })
    }

    pub(super) fn leave(self, checker: &mut Checker<'_, '_>) {
        checker.locals = self.locals;
        self.environment.restore(checker);
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
        f.write_str(") (input-scope")?;
        if let Some(inputs) = &self.input_scope {
            for (sort, ty) in inputs.iter() {
                write!(f, " (input {sort}")?;
                if let Some(ty) = ty {
                    write!(f, " {ty}")?;
                }
                f.write_str(")")?;
            }
        }
        write!(f, ") {})", self.environment)
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
        let types = DeclarationBindings::from([(id, Term::String("caller\"\n".into()))]);
        let values = DeclarationBindings::from([(id, local)]);
        let defaults = DeclarationBindings::from([(id, (local, p(P::Str)))]);
        let context = crate::ptc::Context::default();
        let environment = environment::Environment::retain(&nominal::InputEnvironment {
            substitution: &types,
            bindings: &values,
            normal_bindings: &defaults,
            context: &context,
        });
        let scope = Scope {
            locals: LocalBindings::from([(local, p(P::Str))]),
            environment,
            owner: Some(owner),
            mode: true,
            input_scope: Some([(Sort::Type, None), (Sort::ComptimeValue, Some(p(P::Str)))].into()),
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
