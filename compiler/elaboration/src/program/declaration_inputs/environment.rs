//! Owned lexical inputs for a selected default provider. Retention copies
//! binding tables under the checking budget; immutable terms and PTC context
//! remain shared. Installation publishes only a fully prepared copy.
use super::*;

pub(super) struct Environment {
    substitution: BTreeMap<DeclarationId, std::sync::Arc<Term>>,
    bindings: BTreeMap<DeclarationId, n::LocalId>,
    normal_bindings: BTreeMap<DeclarationId, (n::LocalId, Term)>,
    context: crate::ptc::Context,
}

pub(super) fn charge(fuel: &mut u64, count: usize, span: Span) -> Result<(), Error> {
    *fuel = fuel
        .checked_sub(count as u64)
        .ok_or_else(|| error(span, "input environment retention budget exhausted"))?;
    Ok(())
}

pub(super) fn copy_term(value: &Term, fuel: &mut u64, span: Span) -> Result<Term, Error> {
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

pub(super) fn copy_domains<'a>(
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
    pub(super) fn retain(
        source: &nominal::InputEnvironment<'_>,
        fuel: &mut u64,
        span: Span,
    ) -> Result<Self, Error> {
        let count = source
            .substitution
            .len()
            .saturating_add(source.bindings.len())
            .saturating_add(source.normal_bindings.len());
        charge(fuel, count, span)?;
        let normal_bindings = source
            .normal_bindings
            .iter()
            .map(|(id, (binding, ty))| Ok((*id, (*binding, copy_term(ty, fuel, span)?))))
            .collect::<Result<_, Error>>()?;
        Ok(Self {
            substitution: source.substitution.clone(),
            bindings: source.bindings.clone(),
            normal_bindings,
            context: source.context.clone(),
        })
    }

    fn view(&self) -> nominal::InputEnvironment<'_> {
        nominal::InputEnvironment {
            substitution: &self.substitution,
            bindings: &self.bindings,
            normal_bindings: &self.normal_bindings,
            context: &self.context,
        }
    }

    pub(super) fn install(&self, checker: &mut Checker<'_, '_>, span: Span) -> Result<(), Error> {
        let copy = Self::retain(&self.view(), &mut checker.required_steps, span)?;
        copy.restore(checker);
        Ok(())
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
        let mut types = BTreeMap::from([(id, original.clone())]);
        let mut values = BTreeMap::from([(id, local)]);
        let mut defaults = BTreeMap::from([(id, (local, p(P::Str)))]);
        let mut context = crate::ptc::Context::default();
        let environment = Environment::retain(
            &nominal::InputEnvironment {
                substitution: &types,
                bindings: &values,
                normal_bindings: &defaults,
                context: &context,
            },
            &mut 100,
            Span::default(),
        )
        .unwrap();

        types.insert(id, Arc::new(Term::Bool(false)));
        values.clear();
        defaults.clear();
        context.assume(crate::ptc::Formula::equal(p(P::U8), p(P::Bool)));
        assert!(Arc::ptr_eq(&environment.substitution[&id], &original));
        assert_eq!(environment.bindings[&id], local);
        assert_eq!(environment.normal_bindings[&id], (local, p(P::Str)));
        assert_eq!(environment.context, crate::ptc::Context::default());
        let dump = environment.to_string();
        assert!(dump.contains("parent\\\"\\n"), "{dump}");
        assert!(!dump.contains('\n'), "{dump}");
    }

    #[test]
    fn environment_retention_rejects_exhaustion_without_a_partial_snapshot() {
        let id = DeclarationId {
            package: 1,
            local: 2,
        };
        let local = n::LocalId { owner: 1, index: 3 };
        let types = BTreeMap::new();
        let values = BTreeMap::new();
        let defaults = BTreeMap::from([(id, (local, Term::Tuple(vec![p(P::Str); 32])))]);
        let context = crate::ptc::Context::default();
        let view = nominal::InputEnvironment {
            substitution: &types,
            bindings: &values,
            normal_bindings: &defaults,
            context: &context,
        };
        let mut fuel = 1_000;
        let completed = Environment::retain(&view, &mut fuel, Span::default()).unwrap();
        let cost = 1_000 - fuel;
        assert!(cost > 32);
        for limit in 0..cost {
            let mut remaining = limit;
            let Err(failure) = Environment::retain(&view, &mut remaining, Span::default()) else {
                panic!("retained an environment with insufficient budget {limit}");
            };
            assert!(!failure.needs_context);
            assert!(remaining <= limit);
            assert_eq!(view.normal_bindings, &completed.normal_bindings);
        }
        let mut fuel = cost;
        let recovered = Environment::retain(&view, &mut fuel, Span::default()).unwrap();
        assert_eq!(fuel, 0);
        assert_eq!(completed.to_string(), recovered.to_string());
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
