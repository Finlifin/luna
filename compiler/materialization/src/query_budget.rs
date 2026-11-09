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
