//! Type-world queries consume the materializer's current allowance.
use super::*;

pub(super) fn run<T>(fuel: &mut usize, limits: Budget, query: impl FnOnce(&mut Budget) -> T) -> T {
    let granted = std::mem::take(fuel);
    let mut budget = Budget {
        steps: granted,
        ..limits
    };
    let result = query(&mut budget);
    *fuel = granted.min(budget.steps);
    result
}

/// Reserve before entering host code, including lazy metadata construction.
/// Failed queries settle actual work; unwinding cannot restore the reservation.
/// A per-query step limit does not discard the caller's unspent outer allowance.
pub(super) fn invocation<T>(
    remaining: &mut u64,
    limits: Budget,
    query: impl FnOnce(&mut Budget) -> T,
) -> T {
    let reserved = std::mem::take(remaining);
    let granted = limits
        .steps
        .min(usize::try_from(reserved).unwrap_or(usize::MAX));
    let mut budget = Budget {
        steps: granted,
        ..limits
    };
    let result = query(&mut budget);
    *remaining = reserved - (granted - granted.min(budget.steps)) as u64;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builder(package: &n::Package) -> Builder<'_> {
        let budget = Budget::default();
        let (world, fuel) = DeclarationWorld::new(package, budget, None).unwrap();
        Builder {
            expansion: None,
            quote_cursor: 0,
            inline_effects: BTreeSet::new(),
            active_staged_inlines: BTreeSet::new(),
            quote_sources: vec![],
            retain_templates: false,
            package,
            source: DeclarationSource::Borrowed(package),
            world,
            value_scope: Default::default(),
            queue: VecDeque::new(),
            functions: HashSet::new(),
            staged_functions: HashMap::new(),
            generated_functions: HashMap::new(),
            recursive_factories: HashMap::new(),
            invocation_snapshot: None,
            types: HashSet::new(),
            staging_views: HashMap::new(),
            interfaces: HashMap::new(),
            output: vec![],
            fuel,
            budget,
            instances: None,
            runtime_bindings: BTreeSet::new(),
            nominals: vec![],
            interface_contracts: vec![],
            effect_contracts: vec![],
            effect_instances: HashSet::new(),
            frozen_methods: vec![],
            callable_identities: vec![],
            type_cores: Default::default(),
        }
    }

    #[test]
    fn staged_snapshot_reuses_declarations_without_retaining_entry_state() {
        let package = n::Package {
            normal_forms: vec![nir::normal_forms::Head {
                declaration: Id {
                    package: 91,
                    local: 1,
                },
                role: Default::default(),
                collector: None,
                parameters: vec![],
                requirements: vec![n::TypePredicate::Bool(true); 1000],
            }],
            ..Default::default()
        };
        let mut builder = builder(&package);
        let flag = |value| n::Expr {
            source: n::SourceRange { start: 10, end: 20 },
            ty: Term::Primitive(nir::Primitive::Bool),
            kind: K::Literal(n::Literal::Bool(value)),
        };
        let binding = n::LocalId { owner: 1, index: 0 };
        let mut cold_cost = None;
        for value in [true, false, true] {
            let body = n::Expr {
                kind: K::Block {
                    statements: vec![n::Expr {
                        ty: Term::Primitive(nir::Primitive::Void),
                        kind: K::Let {
                            binding,
                            value: Box::new(flag(value)),
                        },
                        ..flag(true)
                    }],
                    result: Some(Box::new(n::Expr {
                        kind: K::Local(binding),
                        ..flag(true)
                    })),
                },
                ..flag(value)
            };
            let before = builder.fuel;
            assert_eq!(builder.invoke_staged_input(&body).unwrap(), flag(value));
            assert!(builder.fuel < before);
            let cost = before - builder.fuel;
            match cold_cost {
                None => cold_cost = Some(cost),
                Some(cold) => assert!(cost < cold, "staged preparation must reuse source metadata"),
            }
            assert!(builder.invocation_snapshot.is_some());
            let unbound = n::Expr {
                kind: K::Local(binding),
                ..flag(true)
            };
            assert!(
                builder
                    .invoke_staged_input(&unbound)
                    .unwrap_err()
                    .contains("MissingBinding")
            );
            assert!(builder.invocation_snapshot.is_some());
        }
        builder.fuel = 1;
        assert!(builder.invoke_staged_input(&flag(false)).is_err());
        builder.fuel = 100_000;
        assert_eq!(
            builder.invoke_staged_input(&flag(false)).unwrap(),
            flag(false)
        );
        let description = n::Expr {
            ty: Term::Primitive(nir::Primitive::Type),
            kind: K::TypeValue(Term::Primitive(nir::Primitive::Char)),
            ..flag(true)
        };
        assert_eq!(
            builder.invoke_staged_input(&description).unwrap(),
            description
        );
        assert!(package.functions.is_empty());
    }

    #[test]
    fn repeated_formation_queries_share_the_remaining_allowance() {
        let package = n::Package::default();
        let mut builder = builder(&package);
        let predicate = n::TypePredicate::Equal(
            Term::Primitive(nir::Primitive::Bool),
            Term::Primitive(nir::Primitive::Bool),
        );
        builder.fuel = 32;
        let before = builder.fuel;
        assert!(matches!(
            builder.prove_requirement(&predicate).unwrap(),
            ptc::Outcome::Entailed(_)
        ));
        assert!(
            builder.fuel < before,
            "type proof must debit the materializer"
        );
        loop {
            let before = builder.fuel;
            let outcome = builder.prove_requirement(&predicate);
            assert!(builder.fuel <= before);
            if matches!(outcome, Ok(ptc::Outcome::Entailed(_))) {
                assert!(builder.fuel < before);
            } else {
                break;
            }
        }
        builder.fuel = 0;
        assert!(!matches!(
            builder.prove_requirement(&predicate),
            Ok(ptc::Outcome::Entailed(_))
        ));
        assert_eq!(builder.fuel, 0);
        builder.fuel = 32;
        let rejected = n::TypePredicate::Equal(
            Term::Primitive(nir::Primitive::Bool),
            Term::Primitive(nir::Primitive::Char),
        );
        assert!(!matches!(
            builder.prove_requirement(&rejected),
            Ok(ptc::Outcome::Entailed(_))
        ));
        assert!(builder.fuel < 32, "unsuccessful queries also consume work");
        builder.fuel = 32;
        assert!(matches!(
            builder.prove_requirement(&predicate).unwrap(),
            ptc::Outcome::Entailed(_)
        ));
    }
}
