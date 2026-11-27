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

    fn builder(package: &n::Package) -> Builder<'_, '_> {
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
            functions: BTreeSet::new(),
            staged_functions: BTreeMap::new(),
            generated_functions: BTreeMap::new(),
            recursive_factories: BTreeMap::new(),
            invocation_snapshot: None,
            types: BTreeSet::new(),
            staging_views: HashMap::new(),
            interfaces: BTreeMap::new(),
            associated_demands: BTreeSet::new(),
            output: vec![],
            fuel,
            budget,
            instances: None,
            runtime_bindings: BTreeSet::new(),
            runtime_domains: BTreeMap::new(),
            global_runtime_deliveries: BTreeMap::new(),
            nominals: vec![],
            interface_contracts: vec![],
            dyn_interfaces: vec![],
            dyn_vtables: vec![],
            effect_contracts: vec![],
            effect_instances: BTreeSet::new(),
            statics: vec![],
            static_declarations: BTreeSet::new(),
            frozen_methods: vec![],
            callable_identities: vec![],
            type_cores: Default::default(),
            expression_owner: None,
            active_caller_env: None,
            active_closure_scopes: Vec::new(),
        }
    }

    #[test]
    fn staged_type_construction_uses_source_invocation() {
        let source = n::SourceRange { start: 10, end: 20 };
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let type_domain = Term::Primitive(nir::Primitive::Type);
        let declaration = Id {
            package: 1,
            local: 1,
        };
        let binding = n::LocalId { owner: 1, index: 0 };
        let function = n::Function {
            interface: n::FunctionDeclaration {
                source: nir::program::SourceRange { start: 0, end: 0 },
                declaration,
                name: "consume".into(),
                phase: n::FunctionPhase::Runtime,
                inline: false,
                pure: false,
                is_unsafe: false,
                type_parameters: vec![],
                parameters: vec![binding],
                env_parameters: vec![],
                defaults: vec![],
                signature: Term::Function {
                    parameters: vec![nir::Parameter {
                        sort: nir::Sort::Type,
                        ty: type_domain.clone(),
                    }],
                    result: Box::new(boolean.clone()),
                },
                requirements: vec![],
                capabilities: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                entry_snapshots: vec![],
            },
            body: n::Expr {
                source,
                ty: boolean.clone(),
                kind: K::Literal(n::Literal::Bool(true)),
            },
            runtime_deliveries: Default::default(),
        };
        let callee = n::Expr {
            source,
            ty: function.signature.clone(),
            kind: K::Function(declaration),
        };
        let input = n::Expr {
            source,
            ty: type_domain.clone(),
            kind: K::PointerType(Box::new(n::Expr {
                source,
                ty: type_domain,
                kind: K::TypeValue(boolean.clone()),
            })),
        };
        let mut call = n::Expr {
            source,
            ty: boolean.clone(),
            kind: K::Call {
                callee: Box::new(callee),
                parameter_bindings: vec![binding],
                env_arguments: vec![],
                arguments: vec![n::CallArgument {
                    passing: n::ArgumentPassing::Value,
                    parameter: 0,
                    value: input,
                    is_default: false,
                }],
            },
        };
        let package = n::Package {
            functions: vec![function],
            ..Default::default()
        };
        let mut builder = builder(&package);
        builder.stage_direct_call(&mut call).unwrap();
        assert!(
            builder
                .staged_functions
                .contains_key(&OrderedFunctionKey::new(
                    declaration,
                    &[Term::Pointer(Box::new(boolean))]
                ))
        );
        assert!(builder.invocation_snapshot.is_some());
    }

    #[test]
    fn staged_data_copy_does_not_invoke_or_retry_invalid_data() {
        let package = n::Package::default();
        let mut builder = builder(&package);
        let flag = n::Expr {
            source: n::SourceRange { start: 10, end: 20 },
            ty: Term::Primitive(nir::Primitive::Bool),
            kind: K::Literal(n::Literal::Bool(true)),
        };
        let data = n::Expr {
            source: flag.source,
            ty: Term::Optional(Box::new(flag.ty.clone())),
            kind: K::OptionalPresent(Box::new(flag.clone())),
        };
        assert_eq!(builder.complete_staged_input(&data).unwrap(), data);
        assert!(builder.invocation_snapshot.is_none());

        let invalid = n::Expr {
            ty: Term::Primitive(nir::Primitive::Char),
            ..flag.clone()
        };
        assert!(builder.complete_staged_input(&invalid).is_err());
        assert!(builder.invocation_snapshot.is_none());

        let wide = n::Expr {
            source: flag.source,
            ty: Term::Primitive(nir::Primitive::Symbol),
            kind: K::Literal(n::Literal::Symbol("x".repeat(10_000))),
        };
        builder.fuel = 32;
        let failure = builder.complete_staged_input(&wide).unwrap_err();
        assert!(failure.contains("budget"), "{failure}");
        assert!(builder.invocation_snapshot.is_none());
        builder.fuel = 100_000;
        assert_eq!(builder.complete_staged_input(&wide).unwrap(), wide);
        assert!(builder.invocation_snapshot.is_none());

        let computation = n::Expr {
            kind: K::Not(Box::new(flag.clone())),
            ..flag.clone()
        };
        let result = builder.complete_staged_input(&computation).unwrap();
        assert_eq!(result.ty, flag.ty);
        assert_eq!(result.kind, K::Literal(n::Literal::Bool(false)));
        assert!(builder.invocation_snapshot.is_some());
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
