use super::*;

fn id(local: u32) -> nir::DeclarationId {
    nir::DeclarationId { package: 1, local }
}

fn boolean(value: bool) -> Expr {
    Expr {
        source: SourceRange { start: 10, end: 20 },
        ty: Term::Primitive(Primitive::Bool),
        kind: ExprKind::Literal(Literal::Bool(value)),
    }
}

fn package() -> Package {
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(boolean(true).ty),
    };
    let method = |index| MethodContract {
        source: nir::program::SourceRange { start: 0, end: 0 },
        phase: TraitPhase::Runtime,
        is_quote: false,
        is_optional: false,
        pure: true,
        is_unsafe: false,
        type_arguments: vec![],
        type_parameters: vec![],
        capabilities: vec![],
        program: nir::contracts::ProgramContract {
            preconditions: vec![boolean(true)],
            postconditions: vec![Expr {
                kind: ExprKind::Local(LocalId {
                    owner: index,
                    index: 0,
                }),
                ..boolean(true)
            }],
            result_binding: Some(LocalId {
                owner: index,
                index: 0,
            }),
            entry_snapshots: vec![],
        },
        parameter_bindings: vec![],
        by_value_receiver: false,
        by_reference_receiver: false,
        declaration: id(index),
        name: format!("method_{index}"),
        signature: signature.clone().into(),
        requirements: vec![],
        default: None,
    };
    let methods = vec![method(2), method(3)];
    let functions = methods
        .iter()
        .map(|method| Function {
            interface: nir::program::FunctionDeclaration {
                source: nir::program::SourceRange { start: 0, end: 0 },
                declaration: id(method.declaration.local + 10),
                name: method.name.clone(),
                phase: FunctionPhase::Runtime,
                inline: false,
                pure: false,
                is_unsafe: false,
                type_parameters: vec![],
                parameters: vec![],
                env_parameters: vec![],
                defaults: vec![],
                requirements: vec![],
                capabilities: vec![],
                preconditions: vec![],
                postconditions: vec![],
                entry_snapshots: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                signature: signature.clone(),
            },
            body: boolean(true),
            runtime_deliveries: Default::default(),
        })
        .collect();
    Package {
        traits: vec![Trait {
            comptime_only: false,
            parameter_types: vec![],
            associated: vec![],
            requirements: vec![],
            capabilities: vec![],
            type_parameters: vec![],
            declaration: id(0),
            parents: vec![],
            self_parameter: id(1),
            name: "Contract".into(),
            methods,
        }],
        implementations: vec![Implementation {
            phases: Default::default(),
            parameter_types: vec![],
            associated: vec![],
            associated_initializers: vec![],
            interface_arguments: vec![],
            capabilities: vec![],
            type_parameters: vec![],
            requirements: vec![],
            declaration: id(4),
            subject: boolean(true).ty,
            interface: Some(id(0)),
            methods: vec![("method_2".into(), id(12)), ("method_3".into(), id(13))],
        }],
        functions,
        ..Default::default()
    }
}

#[test]
fn late_rejection_preserves_package_and_binding_cursor() {
    let mut invalid = package();
    invalid.functions[1].preconditions = vec![boolean(false)];
    let before = invalid.clone();
    let mut next = 100;
    let error = inherit(&mut invalid, &mut next).unwrap_err();
    assert!(error.to_string().contains("strengthens"), "{error}");
    assert_eq!(next, 100, "rejected inheritance consumed result bindings");
    assert_eq!(
        invalid, before,
        "rejected inheritance published earlier methods"
    );

    invalid.functions[1].preconditions.clear();
    inherit(&mut invalid, &mut next).unwrap();
    assert_eq!(next, 102);
    for function in &invalid.functions {
        assert!(function.pure);
        assert_eq!(function.preconditions.len(), 1);
        assert_eq!(function.postconditions.len(), 1);
        assert!(function.result_binding.is_some());
    }
}

#[test]
fn shared_budget_failure_never_publishes_and_can_retry() {
    let mut original = package();
    original.functions[1].preconditions = vec![boolean(true)];
    let mut successful = original.clone();
    let mut next = 100;
    let mut full = Limits::default();
    let initial = full.arithmetic.steps;
    inherit_with_limits(&mut successful, &mut next, &mut full).unwrap();
    let spent = initial - full.arithmetic.steps;
    for allowance in [0, spent / 2, spent - 1] {
        let mut candidate = original.clone();
        let mut next = 100;
        let mut limits = Limits::default();
        limits.arithmetic.steps = allowance;
        let failure = inherit_with_limits(&mut candidate, &mut next, &mut limits).unwrap_err();
        assert!(
            matches!(failure.reason, flow::Reason::Unknown(_)),
            "{failure}"
        );
        assert_eq!(candidate, original);
        assert_eq!(next, 100);
        inherit_with_limits(&mut candidate, &mut next, &mut Limits::default()).unwrap();
        assert_eq!(candidate, successful);
        assert_eq!(next, 102);
    }
}

#[test]
fn weakening_observes_all_effective_callee_contracts_in_either_method_order() {
    let mut original = package();
    original.traits[0].methods[1].program.preconditions = vec![boolean(false)];
    let result = LocalId {
        owner: 13,
        index: 0,
    };
    original.functions[1].result_binding = Some(result);
    original.functions[1].postconditions = vec![Expr {
        kind: ExprKind::Local(result),
        ..boolean(true)
    }];
    // Before inheritance this callee appears callable without a precondition.
    // Its effective interface domain is false, so this cannot prove weakening.
    original.functions[0].preconditions = vec![Expr {
        kind: ExprKind::Call {
            callee: Box::new(Expr {
                source: boolean(true).source,
                ty: original.functions[1].signature.clone(),
                kind: ExprKind::Function(id(13)),
            }),
            arguments: vec![],
            env_arguments: vec![],
            parameter_bindings: vec![],
        },
        ..boolean(true)
    }];
    for reverse in [false, true] {
        let mut candidate = original.clone();
        if reverse {
            candidate.implementations[0].methods.reverse();
        }
        let before = candidate.clone();
        let mut next = 100;
        let failure = inherit(&mut candidate, &mut next).unwrap_err();
        assert!(failure.to_string().contains("strengthens"), "{failure}");
        assert_eq!(candidate, before);
        assert_eq!(next, 100);
    }
}

#[test]
fn inheritance_cost_and_body_storage_do_not_depend_on_body_size() {
    let mut small = package();
    small.functions[1].preconditions = vec![boolean(true)];
    let mut large = small.clone();
    large.functions[1].body.kind = ExprKind::Block {
        statements: vec![boolean(true); 10_000],
        result: Some(Box::new(boolean(true))),
    };
    let body_storage = |package: &Package| match &package.functions[1].body.kind {
        ExprKind::Block { statements, .. } => statements.as_ptr(),
        _ => panic!("expected block body"),
    };
    let storage = body_storage(&large);
    let mut small_limits = Limits::default();
    let initial = small_limits.arithmetic.steps;
    inherit_with_limits(&mut small, &mut 100, &mut small_limits).unwrap();
    let spent = initial - small_limits.arithmetic.steps;
    let mut large_limits = Limits::default();
    large_limits.arithmetic.steps = spent;
    inherit_with_limits(&mut large, &mut 100, &mut large_limits).unwrap();
    assert_eq!(large_limits.arithmetic.steps, 0);
    assert_eq!(
        body_storage(&large),
        storage,
        "inheritance replaced executable storage"
    );
    assert_eq!(large.functions[1].interface, small.functions[1].interface);

    // Late proof failure rolls back headers without replacing body storage.
    let mut rejected = package();
    rejected.functions[1].body = large.functions[1].body.clone();
    rejected.functions[1].preconditions = vec![boolean(false)];
    let storage = body_storage(&rejected);
    let before = rejected.clone();
    let mut next = 100;
    let error = inherit(&mut rejected, &mut next).unwrap_err();
    assert!(error.to_string().contains("strengthens"), "{error}");
    assert_eq!(next, 100);
    assert_eq!(rejected, before);
    assert_eq!(body_storage(&rejected), storage);
    rejected.functions[1].preconditions = vec![boolean(true)];
    inherit(&mut rejected, &mut next).unwrap();
    assert_eq!(next, 102);
    assert_eq!(body_storage(&rejected), storage);
}

#[test]
fn implication_proof_needs_no_body_and_preserves_budget_failure() {
    let mut declaration = package().functions.remove(0).interface;
    declaration.preconditions = vec![boolean(true)];
    declaration.postconditions = vec![boolean(true)];
    let mut full = Limits::default();
    let initial = full.arithmetic.steps;
    flow::prove_implication(&declaration, None, &mut full).unwrap();
    let spent = initial - full.arithmetic.steps;
    assert!(spent > 0);
    for allowance in 0..spent {
        let mut limits = Limits::default();
        limits.arithmetic.steps = allowance;
        let error = flow::prove_implication(&declaration, None, &mut limits)
            .err()
            .expect("implication must be rejected");
        assert!(
            matches!(
                error.reason,
                flow::Reason::Unknown(super::super::Unknown::Arithmetic(
                    comptime_runtime::Error::BudgetExhausted
                ))
            ),
            "allowance {allowance}: {error}"
        );
    }
    flow::prove_implication(&declaration, None, &mut Limits::default()).unwrap();
    declaration.postconditions = vec![boolean(false)];
    let error = flow::prove_implication(&declaration, None, &mut Limits::default())
        .err()
        .expect("implication must be rejected");
    assert_eq!(error.reason, flow::Reason::MissingPostconditionProof);
}

#[test]
fn implication_rejects_execution_only_bindings_and_defaults() {
    let declaration = package().functions.remove(0).interface;
    let binding = LocalId { owner: 1, index: 0 };
    for case in 0..3 {
        let mut invalid = declaration.clone();
        match case {
            0 => invalid.result_binding = Some(binding),
            1 => invalid.entry_snapshots.push(EntrySnapshot {
                parameter: binding,
                binding,
            }),
            _ => invalid.defaults.push((0, boolean(true))),
        }
        let error = flow::prove_implication(&invalid, None, &mut Limits::default())
            .err()
            .expect("implication must be rejected");
        assert_eq!(error.reason, flow::Reason::InvalidSignature);
    }
}

#[test]
fn inheritance_scales_linearly_across_large_packages() {
    let mut candidate = package();
    let template_func = candidate.functions[0].clone();
    let template_impl = candidate.implementations[0].clone();
    for offset in 0..400u32 {
        let fn_a = id(1000 + offset * 2);
        let fn_b = id(1001 + offset * 2);
        let mut func_a = template_func.clone();
        func_a.interface.declaration = fn_a;
        func_a.interface.name = format!("extra_method_2_{offset}");
        let mut func_b = template_func.clone();
        func_b.interface.declaration = fn_b;
        func_b.interface.name = format!("extra_method_3_{offset}");
        candidate.functions.push(func_a);
        candidate.functions.push(func_b);

        let mut extra_impl = template_impl.clone();
        extra_impl.declaration = id(5000 + offset);
        extra_impl.methods = vec![("method_2".into(), fn_a), ("method_3".into(), fn_b)];
        candidate.implementations.push(extra_impl);
    }

    let mut next = 100;
    let mut limits = Limits::default();
    limits.arithmetic.steps = 100_000;
    inherit_with_limits(&mut candidate, &mut next, &mut limits).unwrap();
    assert!(next > 100);
    assert_eq!(candidate.functions.last().unwrap().preconditions.len(), 1);
    assert_eq!(candidate.functions.last().unwrap().postconditions.len(), 1);
}

#[test]
fn incremental_inheritance_skips_verified_upstream_while_checking_downstream() {
    let mut candidate = package();
    // Upstream impl id(4) has a method whose precondition would strengthen the trait;
    // when id(4) is in VerifiedUpstream, inheritance skips re-verifying it without mutating it.
    candidate.functions[0].preconditions = vec![boolean(false)];
    candidate.implementations[0].declaration = nir::DeclarationId {
        package: 1,
        local: 4,
    };

    // Downstream impl id(20) in package 2 uses upstream trait id(0) and a downstream body id(30).
    let downstream_fn_id = nir::DeclarationId {
        package: 2,
        local: 30,
    };
    let mut downstream_fn = candidate.functions[1].clone();
    downstream_fn.interface.declaration = downstream_fn_id;
    downstream_fn.interface.name = "downstream_method_3".into();
    downstream_fn.interface.preconditions = vec![];
    downstream_fn.interface.postconditions = vec![];
    candidate.functions.push(downstream_fn);
    candidate.implementations.push(Implementation {
        declaration: nir::DeclarationId {
            package: 2,
            local: 20,
        },
        methods: vec![
            ("method_2".into(), id(12)),
            ("method_3".into(), downstream_fn_id),
        ],
        ..candidate.implementations[0].clone()
    });
    // Ensure upstream method_2 body also isn't rejected due to id(12)'s own preconditions.
    let mut verified = flow::VerifiedUpstream::default();
    verified
        .implementations
        .insert(candidate.implementations[0].declaration);
    // For method_2 inside downstream impl id(20), mark id(12) empty preconds as default-compatible;
    // test clean inheritance first:
    candidate.functions[0].preconditions = vec![];
    let mut upstream_only = candidate.clone();
    upstream_only.functions[0].preconditions = vec![boolean(false)];
    upstream_only.implementations.pop();

    let mut next = 100;
    inherit_incremental(&mut upstream_only, &mut next, &verified).unwrap();
    assert_eq!(next, 100);
    assert!(upstream_only.functions[0].postconditions.is_empty());

    // Now test downstream impl id(20) inheriting upstream trait id(0) contracts:
    let mut next = 100;
    inherit_incremental(&mut candidate, &mut next, &verified).unwrap();
    let down = candidate
        .functions
        .iter()
        .find(|f| f.declaration == downstream_fn_id)
        .unwrap();
    assert_eq!(down.preconditions.len(), 1);
    assert_eq!(down.postconditions.len(), 1);

    // Strengthening trait precondition in downstream impl id(20) must still be rejected!
    candidate
        .functions
        .iter_mut()
        .find(|f| f.declaration == downstream_fn_id)
        .unwrap()
        .preconditions = vec![boolean(false)];
    let error = inherit_incremental(&mut candidate, &mut next, &verified).unwrap_err();
    assert!(error.to_string().contains("strengthens"), "{error}");
}

#[test]
fn inherit_incremental_skip_proofs_installs_drafts_without_reproving_upstream() {
    let mut candidate = package();
    // Upstream impl id(4) has a stronger explicit precondition on method_3 (id(13))
    // than the trait, which would fail proof if re-checked; `inherit_incremental_skip_proofs`
    // still copies inherited `pure` and postconditions onto its freshly lowered interface
    // while skipping the upstream weakening proof.
    candidate.implementations[0].declaration = nir::DeclarationId {
        package: 1,
        local: 4,
    };
    candidate.traits[0].methods[1].pure = true;

    let downstream_fn_id = nir::DeclarationId {
        package: 2,
        local: 30,
    };
    let mut downstream_fn = candidate.functions[1].clone();
    downstream_fn.interface.declaration = downstream_fn_id;
    downstream_fn.interface.name = "downstream_method_3".into();
    downstream_fn.interface.preconditions = vec![];
    downstream_fn.interface.postconditions = vec![];
    candidate.functions.push(downstream_fn);

    candidate.functions[1].preconditions = vec![boolean(false)];
    candidate.functions[1].interface.pure = false;
    candidate.functions[1].postconditions = vec![];

    candidate.implementations.push(Implementation {
        declaration: nir::DeclarationId {
            package: 2,
            local: 20,
        },
        methods: vec![
            ("method_2".into(), id(12)),
            ("method_3".into(), downstream_fn_id),
        ],
        ..candidate.implementations[0].clone()
    });

    let mut verified = flow::VerifiedUpstream::default();
    verified
        .implementations
        .insert(candidate.implementations[0].declaration);

    let mut next = 100;
    inherit_incremental_skip_proofs(&mut candidate, &mut next, &verified).unwrap();
    let up = candidate
        .functions
        .iter()
        .find(|f| f.declaration == id(13))
        .unwrap();
    assert!(up.interface.pure, "upstream method must still inherit pure");
    assert_eq!(
        up.postconditions.len(),
        1,
        "upstream method must still receive guarded trait postcondition"
    );
    let down = candidate
        .functions
        .iter()
        .find(|f| f.declaration == downstream_fn_id)
        .unwrap();
    assert_eq!(down.preconditions.len(), 1);
    assert_eq!(down.postconditions.len(), 1);

    // Strengthening trait precondition in downstream impl id(20) must still be rejected.
    candidate
        .functions
        .iter_mut()
        .find(|f| f.declaration == downstream_fn_id)
        .unwrap()
        .preconditions = vec![boolean(false)];
    let error = inherit_incremental_skip_proofs(&mut candidate, &mut next, &verified).unwrap_err();
    assert!(error.to_string().contains("strengthens"), "{error}");
}
