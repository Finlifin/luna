//! Lexical roots for a selected default provider. A child shares the caller's
//! immutable bindings and proof context until it shadows an entry.
use super::*;

#[derive(Clone)]
pub(in crate::program) struct Environment {
    substitution: DeclarationBindings<Term>,
    bindings: DeclarationBindings<n::LocalId>,
    normal_bindings: DeclarationBindings<(n::LocalId, Term)>,
    context: crate::ptc::Context,
}

pub(super) fn charge(fuel: &mut u64, count: usize, span: Span) -> Result<(), Error> {
    *fuel = fuel
        .checked_sub(count as u64)
        .ok_or_else(|| error(span, "input environment retention budget exhausted"))?;
    Ok(())
}

pub(in crate::program) fn copy_term(
    value: &Term,
    fuel: &mut u64,
    span: Span,
) -> Result<Term, Error> {
    let before = usize::try_from(*fuel).unwrap_or(usize::MAX);
    let mut remaining = before;
    let result = nir::copy_budget::clone_term(value, &mut remaining);
    *fuel -= (before - remaining) as u64;
    result.map_err(|failure| {
        error(
            span,
            format!("input environment type retention budget exhausted: {failure:?}"),
        )
    })
}

pub(in crate::program) fn copy_domains<'a>(
    domains: impl IntoIterator<Item = (Sort, Option<&'a Term>)>,
    fuel: &mut u64,
    span: Span,
) -> Result<Vec<(Sort, Option<Term>)>, Error> {
    // Do not reserve an iterator's full lower size hint before its entries
    // have consumed budget. A large telescope can fail after a small prefix.
    let mut retained = Vec::new();
    for (sort, ty) in domains {
        charge(fuel, 1, span)?;
        let ty = ty.map(|ty| copy_term(ty, fuel, span)).transpose()?;
        retained.push((sort, ty));
    }
    Ok(retained)
}

impl Environment {
    pub(in crate::program) fn retain_prefix(
        source: &nominal::InputEnvironment<'_>,
        parameters: &[DeclarationId],
        prefix: &[Term],
        fuel: &mut u64,
        span: Span,
    ) -> Result<Self, Error> {
        if prefix.len() > parameters.len() {
            return Err(error(span, "clause input prefix arity mismatch"));
        }
        let mut retained = Self::retain(source);
        for (parameter, value) in parameters.iter().zip(prefix) {
            charge(fuel, 1, span)?;
            let value = copy_term(value, fuel, span)?;
            retained
                .substitution
                .insert(*parameter, std::sync::Arc::new(value));
            retained.bindings.remove(parameter);
            retained.normal_bindings.remove(parameter);
        }
        Ok(retained)
    }

    pub(super) fn retain(source: &nominal::InputEnvironment<'_>) -> Self {
        Self {
            substitution: source.substitution.clone(),
            bindings: source.bindings.clone(),
            normal_bindings: source.normal_bindings.clone(),
            context: source.context.clone(),
        }
    }

    pub(super) fn install(&self, checker: &mut Checker<'_, '_>) {
        self.clone().restore(checker);
    }

    pub(super) fn restore(self, checker: &mut Checker<'_, '_>) {
        checker.alias_substitution = self.substitution;
        checker.alias_value_bindings = self.bindings;
        checker.normal_default_bindings = self.normal_bindings;
        checker.context = self.context;
    }
}

impl std::fmt::Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(input-environment (types")?;
        for (parameter, ty) in &self.substitution {
            write!(f, " (binding {parameter} {ty})")?;
        }
        f.write_str(") (type-values")?;
        for (parameter, binding) in &self.bindings {
            write!(f, " (binding {parameter} {binding})")?;
        }
        f.write_str(") (defaults")?;
        for (parameter, (binding, ty)) in &self.normal_bindings {
            write!(f, " (binding {parameter} {binding} {ty})")?;
        }
        write!(f, ") {})", self.context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn retained_environment_owns_bindings_and_shares_only_immutable_values() {
        let id = DeclarationId {
            package: 1,
            local: 2,
        };
        let local = n::LocalId { owner: 1, index: 3 };
        let original = Arc::new(Term::String("parent\"\n".into()));
        let mut types = DeclarationBindings::default();
        types.insert(id, original.clone());
        let mut values = DeclarationBindings::from([(id, local)]);
        let mut defaults = DeclarationBindings::from([(id, (local, p(P::Str)))]);
        let mut context = crate::ptc::Context::default();
        let environment = Environment::retain(&nominal::InputEnvironment {
            substitution: &types,
            bindings: &values,
            normal_bindings: &defaults,
            context: &context,
        });

        types.insert(id, Arc::new(Term::Bool(false)));
        values.clear();
        defaults.clear();
        context.assume(crate::ptc::Formula::equal(p(P::U8), p(P::Bool)));
        assert!(Arc::ptr_eq(
            environment.substitution.shared(&id).unwrap(),
            &original
        ));
        assert_eq!(environment.bindings[&id], local);
        assert_eq!(environment.normal_bindings[&id], (local, p(P::Str)));
        assert_eq!(environment.context, crate::ptc::Context::default());
        let dump = environment.to_string();
        assert!(dump.contains("parent\\\"\\n"), "{dump}");
        assert!(!dump.contains('\n'), "{dump}");
    }

    #[test]
    fn selected_prefix_is_budgeted_and_does_not_mutate_caller_bindings() {
        let first = DeclarationId {
            package: 1,
            local: 2,
        };
        let next = DeclarationId {
            package: 1,
            local: 3,
        };
        let local = n::LocalId { owner: 1, index: 4 };
        let types = DeclarationBindings::from([(first, Term::Bool(false))]);
        let values = DeclarationBindings::from([(first, local), (next, local)]);
        let defaults =
            DeclarationBindings::from([(first, (local, p(P::Bool))), (next, (local, p(P::Bool)))]);
        let context = crate::ptc::Context::default();
        let view = nominal::InputEnvironment {
            substitution: &types,
            bindings: &values,
            normal_bindings: &defaults,
            context: &context,
        };
        let prefix = [Term::Tuple(vec![Term::Bool(true); 16])];
        let mut fuel = 1_000;
        let retained =
            Environment::retain_prefix(&view, &[first, next], &prefix, &mut fuel, Span::default())
                .unwrap();
        assert_eq!(retained.substitution[&first], prefix[0]);
        assert!(!retained.bindings.contains_key(&first));
        assert!(!retained.normal_bindings.contains_key(&first));
        assert_eq!(retained.bindings[&next], local);
        assert_eq!(retained.normal_bindings[&next], (local, p(P::Bool)));
        let cost = 1_000 - fuel;
        for limit in 0..cost {
            let mut remaining = limit;
            let failure = Environment::retain_prefix(
                &view,
                &[first, next],
                &prefix,
                &mut remaining,
                Span::default(),
            );
            assert!(failure.is_err(), "budget {limit}");
            assert_eq!(types[&first], Term::Bool(false));
            assert_eq!(values[&first], local);
            assert_eq!(defaults[&first], (local, p(P::Bool)));
        }
        let mut remaining = cost;
        let recovered = Environment::retain_prefix(
            &view,
            &[first, next],
            &prefix,
            &mut remaining,
            Span::default(),
        )
        .unwrap();
        assert_eq!(remaining, 0);
        assert_eq!(retained.to_string(), recovered.to_string());
        assert!(
            Environment::retain_prefix(&view, &[], &prefix, &mut 1_000, Span::default()).is_err()
        );
    }

    #[test]
    fn snapshots_share_unrelated_default_domains_across_shadowing() {
        let id = |local| DeclarationId {
            package: u64::MAX,
            local,
        };
        let local = n::LocalId { owner: 1, index: 3 };
        let mut defaults = DeclarationBindings::default();
        for index in 0..128 {
            defaults.insert(id(index), (local, Term::Tuple(vec![p(P::Str); 32])));
        }
        let context = crate::ptc::Context::default();
        let retained = Environment::retain(&nominal::InputEnvironment {
            substitution: &Default::default(),
            bindings: &Default::default(),
            normal_bindings: &defaults,
            context: &context,
        });
        defaults.insert(id(0), (local, p(P::Bool)));
        assert_ne!(defaults[&id(0)], retained.normal_bindings[&id(0)]);
        assert!(Arc::ptr_eq(
            defaults.shared(&id(127)).unwrap(),
            retained.normal_bindings.shared(&id(127)).unwrap()
        ));
    }

    #[test]
    fn domain_retention_preserves_unknown_positions_and_budgets_known_types() {
        let known = Term::Tuple(vec![p(P::U8), p(P::Bool)]);
        let domains = [
            (Sort::Type, None),
            (Sort::ComptimeValue, Some(&known)),
            (Sort::ComptimeValue, None),
        ];
        let mut fuel = 100;
        let copied = copy_domains(domains, &mut fuel, Span::default()).unwrap();
        assert_eq!(
            copied,
            vec![
                (Sort::Type, None),
                (Sort::ComptimeValue, Some(known.clone())),
                (Sort::ComptimeValue, None)
            ]
        );
        let cost = 100 - fuel;
        for limit in 0..cost {
            let mut remaining = limit;
            assert!(copy_domains(domains, &mut remaining, Span::default()).is_err());
        }
    }

    #[test]
    fn domain_retention_keeps_depth_exhaustion_distinct_from_remaining_steps() {
        let mut ty = p(P::Str);
        for _ in 0..129 {
            ty = Term::Tuple(vec![ty]);
        }
        let mut fuel = 10_000;
        let failure = copy_domains(
            [(Sort::ComptimeValue, Some(&ty))],
            &mut fuel,
            Span::default(),
        )
        .unwrap_err();
        assert!(fuel > 0 && fuel < 10_000);
        assert!(failure.message.contains("Depth"), "{}", failure.message);
        assert!(!failure.needs_context);
    }

    #[test]
    fn domain_retention_does_not_allocate_an_unaffordable_iterator_size_hint() {
        let domains = std::iter::repeat((Sort::Type, None)).take(usize::MAX);
        let mut fuel = 1;
        let failure = copy_domains(domains, &mut fuel, Span::default()).unwrap_err();
        assert_eq!(fuel, 0);
        assert!(failure.message.contains("budget exhausted"));
    }
}
