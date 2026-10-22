use elaboration::effects::{Error, route};
use elaboration::ptc::{Budget, Context, Formula};
use nir::effects::{Abi, Entry, Frame, Operation};
use nir::{DeclarationId, Primitive as P, Sort, Term};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn op(local: u32, arguments: Vec<Term>) -> Operation {
    Operation {
        declaration: id(local),
        arguments,
    }
}
fn entry(operation: Operation, total: bool) -> Frame {
    Frame {
        entries: vec![Entry {
            operation,
            total,
            abi: Abi::Direct,
        }],
    }
}
fn ty(value: P) -> Term {
    Term::Primitive(value)
}
fn rigid() -> Term {
    Term::Rigid {
        declaration: id(99),
        sort: Sort::Type,
    }
}

#[test]
fn nearest_total_entry_runs_in_outer_environment_and_can_be_requested_repeatedly() {
    let context = Context::default();
    let request = op(1, vec![]);
    let frames = [entry(request.clone(), true), entry(request.clone(), true)];
    for _ in 0..2 {
        let route = route(&request, Abi::Direct, &frames, &context, Budget::default()).unwrap();
        assert_eq!(route.entries().count(), 1);
        assert_eq!(route.branch_environment(0), Some(&frames[..1]));
        assert_eq!(route.branch_environment(1), None);
        let outer = route.branch_environment(0).unwrap();
        let forwarded =
            elaboration::effects::route(&request, Abi::Direct, outer, &context, Budget::default())
                .unwrap();
        assert_eq!(forwarded.branch_environment(0), Some(&frames[..0]));
        assert_eq!(
            route.to_string(),
            "(effect-route (hops (hop (frame 1) (entry 0) (outer-prefix 1))))"
        );
    }
}
#[test]
fn partial_entries_retain_ordered_fallbacks_and_cannot_claim_missing_supply() {
    let context = Context::default();
    let request = op(1, vec![]);
    let mut frames = vec![
        entry(request.clone(), true),
        entry(request.clone(), false),
        entry(request.clone(), false),
    ];
    let result = route(&request, Abi::Direct, &frames, &context, Budget::default()).unwrap();
    assert_eq!(
        result.entries().map(|e| e.total).collect::<Vec<_>>(),
        [false, false, true]
    );
    frames[0].entries[0].total = false;
    assert_eq!(
        route(&request, Abi::Direct, &frames, &context, Budget::default()).unwrap_err(),
        Error::MissingEntry
    );
    frames[0].entries[0].total = true;
    assert!(route(&request, Abi::Direct, &frames, &context, Budget::default()).is_ok());
}
#[test]
fn labels_use_declaration_full_arguments_and_proved_type_equalities() {
    let context = Context::default();
    let request = op(1, vec![ty(P::Bool)]);
    let mut frames = vec![
        entry(request.clone(), true),
        entry(op(2, vec![ty(P::Bool)]), true),
    ];
    assert_eq!(
        route(&request, Abi::Direct, &frames, &context, Budget::default())
            .unwrap()
            .branch_environment(0)
            .unwrap()
            .len(),
        0
    );
    frames[1] = entry(op(1, vec![rigid()]), true);
    assert_eq!(
        route(&request, Abi::Direct, &frames, &context, Budget::default()).unwrap_err(),
        Error::PendingIdentity
    );
    let mut context = context;
    context.assume(Formula::equal(rigid(), ty(P::Bool)));
    assert_eq!(
        route(&request, Abi::Direct, &frames, &context, Budget::default())
            .unwrap()
            .branch_environment(0)
            .unwrap()
            .len(),
        1
    );
    frames[1] = entry(op(1, vec![ty(P::Char)]), true);
    assert_eq!(
        route(&request, Abi::Direct, &frames, &context, Budget::default())
            .unwrap()
            .branch_environment(0)
            .unwrap()
            .len(),
        0
    );
}
#[test]
fn abi_mismatches_and_duplicate_entries_are_not_hidden_by_outer_routes() {
    let context = Context::default();
    let request = op(1, vec![]);
    let mut frames = vec![entry(request.clone(), true), entry(request.clone(), true)];
    for abi in [Abi::DirectPassing, Abi::Capturing, Abi::Async] {
        frames[1].entries[0].abi = abi;
        assert_eq!(
            route(&request, Abi::Direct, &frames, &context, Budget::default()).unwrap_err(),
            Error::AbiMismatch
        );
        assert!(route(&request, abi, &frames, &context, Budget::default()).is_ok());
    }
    frames[1].entries[0].abi = Abi::Direct;
    let duplicate = frames[0].entries[0].clone();
    frames[1].entries.push(duplicate);
    assert_eq!(
        route(&request, Abi::Direct, &frames, &context, Budget::default()).unwrap_err(),
        Error::AmbiguousInstallation
    );
}
#[test]
fn value_inputs_do_not_enter_ptc_and_opaque_heads_do_not_unfold() {
    let context = Context::default();
    let request = op(1, vec![Term::Bool(true)]);
    let frames = [
        entry(request.clone(), true),
        entry(op(1, vec![Term::Bool(false)]), true),
    ];
    assert_eq!(
        route(&request, Abi::Direct, &frames, &context, Budget::default())
            .unwrap()
            .branch_environment(0)
            .unwrap()
            .len(),
        0
    );
    let request = op(1, vec![ty(P::Bool)]);
    let frames = [entry(
        op(
            1,
            vec![Term::NormalForm {
                head: id(8),
                sort: Sort::Type,
                arguments: vec![ty(P::Bool)],
            }],
        ),
        true,
    )];
    assert_eq!(
        route(&request, Abi::Direct, &frames, &context, Budget::default()).unwrap_err(),
        Error::PendingIdentity
    );
}
#[test]
fn exhausted_width_depth_and_payload_budgets_recover_without_partial_routes() {
    let context = Context::default();
    let request = op(1, vec![ty(P::Bool)]);
    let frames = [entry(request.clone(), true)];
    assert_eq!(
        route(
            &request,
            Abi::Direct,
            &frames,
            &context,
            Budget {
                steps: 1,
                max_depth: 64
            }
        )
        .unwrap_err(),
        Error::BudgetExhausted
    );
    let wide = op(1, vec![Term::Tuple(vec![ty(P::Bool); 100])]);
    assert_eq!(
        route(
            &wide,
            Abi::Direct,
            &[],
            &context,
            Budget {
                steps: 10,
                max_depth: 64
            }
        )
        .unwrap_err(),
        Error::BudgetExhausted
    );
    let large = op(1, vec![Term::String("x".repeat(100).into())]);
    assert_eq!(
        route(
            &large,
            Abi::Direct,
            &[],
            &context,
            Budget {
                steps: 10,
                max_depth: 64
            }
        )
        .unwrap_err(),
        Error::BudgetExhausted
    );
    let deep = op(
        1,
        vec![Term::Pointer(Box::new(Term::Pointer(Box::new(ty(
            P::Bool,
        )))))],
    );
    assert_eq!(
        route(
            &deep,
            Abi::Direct,
            &[],
            &context,
            Budget {
                steps: 100,
                max_depth: 2
            }
        )
        .unwrap_err(),
        Error::BudgetExhausted
    );
    assert!(route(&request, Abi::Direct, &frames, &context, Budget::default()).is_ok());
}

#[test]
fn incomplete_scopes_cannot_supply_equal_looking_operation_instances() {
    let context = Context::default();
    for argument in [
        Term::Hole {
            index: 0,
            sort: Sort::Type,
        },
        Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        },
    ] {
        let request = op(1, vec![argument]);
        let frames = [entry(request.clone(), true)];
        assert_eq!(
            route(&request, Abi::Direct, &frames, &context, Budget::default()).unwrap_err(),
            Error::InvalidInput
        );
    }
}

#[test]
fn handler_branch_excludes_sibling_operations_in_the_same_installation() {
    let context = Context::default();
    let request = op(1, vec![]);
    let sibling = op(2, vec![]);
    let frames = [Frame {
        entries: vec![
            entry(request.clone(), true).entries.remove(0),
            entry(sibling.clone(), true).entries.remove(0),
        ],
    }];
    let routed = route(&request, Abi::Direct, &frames, &context, Budget::default()).unwrap();
    assert_eq!(
        route(
            &sibling,
            Abi::Direct,
            routed.branch_environment(0).unwrap(),
            &context,
            Budget::default()
        )
        .unwrap_err(),
        Error::MissingEntry
    );
    assert!(route(&sibling, Abi::Direct, &frames, &context, Budget::default()).is_ok());
}

fn declared_operation() -> nir::effects::Declaration {
    nir::effects::Declaration {
        declaration: id(1),
        group: None,
        name: "echo".into(),
        type_parameters: vec![id(99)],
        parameter_types: vec![],
        signature: Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::Runtime,
                ty: rigid(),
            }],
            result: Box::new(rigid()),
        },
        abi: Abi::Direct,
        inline: false,
        requirements: vec![],
        capabilities: vec![],
    }
}

#[test]
fn operation_value_domains_check_dependent_and_symbolic_arguments() {
    let context = Context::default();
    let index = elaboration::implementation::Index::default();
    let world = elaboration::capabilities::World {
        index: &index,
        implementations: &[],
        traits: &[],
        assumptions: &[],
        context: &context,
    };
    let mut declaration = declared_operation();
    declaration.type_parameters.push(id(100));
    declaration.parameter_types = vec![ty(P::Type), rigid()];
    let yes = elaboration::effects::instantiate(
        &declaration,
        &[ty(P::Bool), Term::Bool(true)],
        &world,
        Budget::default(),
    )
    .unwrap();
    let no = elaboration::effects::instantiate(
        &declaration,
        &[ty(P::Bool), Term::Bool(false)],
        &world,
        Budget::default(),
    )
    .unwrap();
    assert_ne!(yes.operation(), no.operation());
    assert!(declaration.to_string().contains("(parameter-types"));
    for arguments in [
        vec![ty(P::Bool), Term::Unit],
        vec![ty(P::U8), Term::Bool(true)],
        vec![ty(P::Bool), ty(P::Bool)],
    ] {
        assert!(
            elaboration::effects::instantiate(&declaration, &arguments, &world, Budget::default())
                .is_err()
        );
    }
    let symbolic = Term::Rigid {
        declaration: id(101),
        sort: Sort::ComptimeValue,
    };
    let arguments = [ty(P::Bool), symbolic.clone()];
    assert!(
        elaboration::effects::instantiate(&declaration, &arguments, &world, Budget::default())
            .is_err()
    );
    elaboration::effects::instantiate_with_domains(
        &declaration,
        &arguments,
        &world,
        Budget::default(),
        |value, domain, _| *value == symbolic && *domain == ty(P::Bool),
    )
    .unwrap();
    assert!(matches!(
        elaboration::effects::instantiate(
            &declaration,
            &[ty(P::Bool), Term::Bool(true)],
            &world,
            Budget {
                steps: 0,
                ..Default::default()
            }
        ),
        Err(Error::BudgetExhausted)
    ));
}

#[test]
fn operation_formation_checks_arity_sort_scope_and_abi_before_routing() {
    use elaboration::effects::instantiate;
    let context = Context::default();
    let index = elaboration::implementation::Index::default();
    let world = elaboration::capabilities::World {
        index: &index,
        implementations: &[],
        traits: &[],
        assumptions: &[],
        context: &context,
    };
    let mut declaration = declared_operation();
    for args in [
        vec![],
        vec![Term::Bool(true)],
        vec![Term::Hole {
            index: 0,
            sort: Sort::Type,
        }],
    ] {
        assert!(matches!(
            instantiate(&declaration, &args, &world, Budget::default()),
            Err(Error::InvalidInput)
        ));
    }
    declaration.abi = Abi::DirectPassing;
    assert!(matches!(
        instantiate(&declaration, &[ty(P::Bool)], &world, Budget::default()),
        Err(Error::InvalidInput)
    ));
    declaration.abi = Abi::Direct;
    declaration.type_parameters.push(id(99));
    assert!(matches!(
        instantiate(
            &declaration,
            &[ty(P::Bool), ty(P::Bool)],
            &world,
            Budget::default()
        ),
        Err(Error::InvalidInput)
    ));
    declaration.type_parameters.pop();
    let instance = instantiate(&declaration, &[ty(P::Bool)], &world, Budget::default()).unwrap();
    assert!(instance.to_string().contains("(effect-instance "));
}

#[test]
fn operation_formation_uses_proved_equalities_without_unfolding_or_assuming_requirements() {
    use elaboration::effects::instantiate;
    let mut declaration = declared_operation();
    declaration
        .requirements
        .push(nir::program::TypePredicate::Equal(rigid(), ty(P::Bool)));
    let mut context = Context::default();
    let index = elaboration::implementation::Index::default();
    let argument = Term::Rigid {
        declaration: id(100),
        sort: Sort::Type,
    };
    {
        let world = elaboration::capabilities::World {
            index: &index,
            implementations: &[],
            traits: &[],
            assumptions: &[],
            context: &context,
        };
        assert!(matches!(
            instantiate(&declaration, &[argument.clone()], &world, Budget::default()),
            Err(Error::FormationNotEstablished)
        ));
        let head = Term::NormalForm {
            head: id(200),
            sort: Sort::Type,
            arguments: vec![ty(P::Bool)],
        };
        assert!(matches!(
            instantiate(&declaration, &[head], &world, Budget::default()),
            Err(Error::FormationNotEstablished)
        ));
    }
    context.assume(Formula::equal(argument.clone(), ty(P::Bool)));
    let world = elaboration::capabilities::World {
        index: &index,
        implementations: &[],
        traits: &[],
        assumptions: &[],
        context: &context,
    };
    assert!(instantiate(&declaration, &[argument], &world, Budget::default()).is_ok());
}

#[test]
fn operation_formation_bounds_substitution_growth_and_recovers_after_failure() {
    use elaboration::effects::instantiate;
    let context = Context::default();
    let index = elaboration::implementation::Index::default();
    let world = elaboration::capabilities::World {
        index: &index,
        implementations: &[],
        traits: &[],
        assumptions: &[],
        context: &context,
    };
    let mut declaration = declared_operation();
    declaration.signature = Term::Function {
        parameters: vec![],
        result: Box::new(Term::Tuple(vec![rigid(); 30])),
    };
    let wide = Term::Tuple(vec![ty(P::Bool); 30]);
    assert!(matches!(
        instantiate(
            &declaration,
            &[wide],
            &world,
            Budget {
                steps: 200,
                max_depth: 64
            }
        ),
        Err(Error::BudgetExhausted)
    ));
    assert!(instantiate(&declaration, &[ty(P::Bool)], &world, Budget::default()).is_ok());
}

#[test]
fn operation_formation_checks_depth_after_substitution() {
    use elaboration::effects::instantiate;
    let context = Context::default();
    let index = elaboration::implementation::Index::default();
    let world = elaboration::capabilities::World {
        index: &index,
        implementations: &[],
        traits: &[],
        assumptions: &[],
        context: &context,
    };
    let mut declaration = declared_operation();
    let mut result = rigid();
    let mut argument = ty(P::Bool);
    for _ in 0..5 {
        result = Term::Pointer(Box::new(result));
        argument = Term::Pointer(Box::new(argument));
    }
    declaration.signature = Term::Function {
        parameters: vec![],
        result: Box::new(result),
    };
    assert!(matches!(
        instantiate(
            &declaration,
            &[argument],
            &world,
            Budget {
                steps: 10000,
                max_depth: 8
            }
        ),
        Err(Error::BudgetExhausted)
    ));
}

#[test]
fn routing_matches_computation_type_arguments_by_proved_row_equality() {
    let context = Context::default();
    let a = op(2, vec![]);
    let b = op(3, vec![]);
    let request = op(
        1,
        vec![Term::computation(
            vec![a.clone(), b.clone(), a.clone()],
            ty(P::Bool),
        )],
    );
    let supplied = op(
        1,
        vec![Term::computation(
            vec![b.clone(), a.clone(), a.clone()],
            ty(P::Bool),
        )],
    );
    let frames = [entry(supplied, true)];
    let result = route(&request, Abi::Direct, &frames, &context, Budget::default()).unwrap();
    assert_eq!(result.entries().count(), 1);
    let frames = [entry(
        op(1, vec![Term::computation(vec![a, b], ty(P::Bool))]),
        true,
    )];
    assert!(route(&request, Abi::Direct, &frames, &context, Budget::default()).is_err());
}
