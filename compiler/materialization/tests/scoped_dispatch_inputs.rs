//! A private dispatch snapshot receives scope authority separately from its
//! declaration catalog and keeps closed dispatch preparation strict.
use materialization::{dispatch_instances, dispatch_instances_in_context};
use nir::{DeclarationId as Id, Primitive, Sort, Term, program as n};
use type_resolution::ptc::{Budget, Context};

fn id(local: u32) -> Id {
    Id { package: 1, local }
}

fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}

fn value(local: u32) -> Term {
    Term::Rigid {
        declaration: id(local),
        sort: Sort::ComptimeValue,
    }
}

fn fixture(domain: Term, argument: Term) -> n::Package {
    let interface = n::Trait {
        comptime_only: false,
        declaration: id(1),
        self_parameter: id(2),
        name: "Accept".into(),
        type_parameters: vec![id(3)],
        parameter_types: vec![domain.clone()],
        associated: vec![],
        requirements: vec![],
        capabilities: vec![],
        parents: vec![],
        methods: vec![],
    };
    let mut unrelated = interface.clone();
    unrelated.declaration = id(70);
    unrelated.self_parameter = id(71);
    unrelated.type_parameters = vec![id(80)];
    unrelated.parameter_types = vec![boolean()];
    unrelated.name = "Unrelated".into();
    n::Package {
        traits: vec![interface, unrelated],
        implementations: vec![n::Implementation {
            declaration: id(10),
            phases: n::ImplementationPhases {
                runtime: true,
                comptime: true,
            },
            type_parameters: vec![id(11)],
            parameter_types: vec![domain],
            interface: Some(id(1)),
            interface_arguments: vec![value(11)],
            subject: boolean(),
            requirements: vec![],
            capabilities: vec![],
            associated: vec![],
            associated_initializers: vec![],
            methods: vec![],
        }],
        functions: vec![n::Function {
            interface: nir::program::FunctionDeclaration {
                source: nir::program::SourceRange { start: 0, end: 0 },
                declaration: id(20),
                name: "entry".into(),
                phase: n::FunctionPhase::Comptime,
                pure: false,
                inline: false,
                is_unsafe: false,
                type_parameters: vec![],
                parameters: vec![],
                env_parameters: vec![],
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(boolean()),
                },
                defaults: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                entry_snapshots: vec![],
                requirements: vec![],
                capabilities: vec![n::TraitRequirement {
                    interface: id(1),
                    subject: boolean(),
                    interface_arguments: vec![argument],
                }],
            },
            body: n::Expr {
                source: n::SourceRange { start: 0, end: 1 },
                ty: boolean(),
                kind: n::ExprKind::Literal(n::Literal::Bool(true)),
            },
            runtime_deliveries: Default::default(),
        }],
        ..Default::default()
    }
}

fn prepare(
    package: &n::Package,
    scope: &[(Id, Term)],
    budget: &mut Budget,
) -> Result<Vec<n::DispatchInstance>, String> {
    let context = Context::from_package(package).unwrap();
    dispatch_instances_in_context(package, &context, &[], scope, &[], budget)
}

#[test]
fn catalog_domains_do_not_authorize_private_dispatch_inputs() {
    let package = fixture(boolean(), value(80));
    for scope in [
        vec![],
        vec![(id(81), boolean())],
        vec![(id(80), Term::Primitive(Primitive::Char))],
    ] {
        let failure = prepare(&package, &scope, &mut Budget::default()).unwrap_err();
        assert!(failure.contains("domain evidence"), "{failure}");
    }
    let mut budget = Budget::default();
    let before = budget.steps;
    let selected = prepare(&package, &[(id(80), boolean())], &mut budget).unwrap();
    assert!(budget.steps < before && budget.steps > 0);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].interface_arguments, vec![value(80)]);
    assert_eq!(selected[0].arguments, vec![value(80)]);
    assert_eq!(selected[0].implementation, id(10));
    // No authority was added to the package or a reusable closed index.
    assert!(dispatch_instances(&package, Budget::default()).is_err());
    assert!(prepare(&package, &[], &mut Budget::default()).is_err());
}

#[test]
fn nested_symbolic_values_use_the_expected_payload_domain() {
    let domain = Term::Optional(Box::new(boolean()));
    let argument = Term::OptionalValue {
        ty: Box::new(domain.clone()),
        value: Some(Box::new(value(80))),
    };
    let package = fixture(domain, argument.clone());
    let selected = prepare(&package, &[(id(80), boolean())], &mut Budget::default()).unwrap();
    assert_eq!(selected[0].interface_arguments, vec![argument]);
    for scope in [vec![], vec![(id(80), Term::Primitive(Primitive::Char))]] {
        assert!(prepare(&package, &scope, &mut Budget::default()).is_err());
    }
    let malformed = fixture(
        Term::Optional(Box::new(boolean())),
        Term::OptionalValue {
            ty: Box::new(Term::Optional(Box::new(boolean()))),
            value: Some(Box::new(Term::Char('x'))),
        },
    );
    assert!(prepare(&malformed, &[], &mut Budget::default()).is_err());
}

#[test]
fn failed_scope_and_budget_checks_publish_no_dispatch_and_allow_retry() {
    let package = fixture(boolean(), value(80));
    for scope in [
        vec![(id(80), Term::Primitive(Primitive::Type))],
        vec![
            (id(80), boolean()),
            (id(80), Term::Primitive(Primitive::Char)),
        ],
    ] {
        let mut budget = Budget::default();
        assert!(prepare(&package, &scope, &mut budget).is_err());
        assert_eq!(budget.steps, 0);
    }
    let mut budget = Budget {
        steps: 1,
        ..Budget::default()
    };
    let scope = [(id(80), boolean())];
    let failure = prepare(&package, &scope, &mut budget).unwrap_err();
    assert!(failure.contains("budget"), "{failure}");
    assert_eq!(budget.steps, 0);
    assert_eq!(
        prepare(&package, &scope, &mut Budget::default())
            .unwrap()
            .len(),
        1
    );
    assert!(package.dispatch_instances.is_empty());
}

#[test]
fn a_trait_premise_does_not_supply_value_scope() {
    let mut package = fixture(boolean(), value(80));
    package.implementations.clear();
    let context = Context::from_package(&package).unwrap();
    let assumptions = &package.functions[0].capabilities;
    let prepare = |scope: &[(Id, Term)]| {
        dispatch_instances_in_context(
            &package,
            &context,
            assumptions,
            scope,
            &[],
            &mut Budget::default(),
        )
    };
    assert!(prepare(&[]).unwrap_err().contains("domain evidence"));
    assert!(prepare(&[(id(80), boolean())]).unwrap().is_empty());
}

#[test]
fn interface_domains_depend_on_preceding_type_inputs() {
    let rigid_type = |local| Term::Rigid {
        declaration: id(local),
        sort: Sort::Type,
    };
    let mut package = fixture(boolean(), value(80));
    let contract = &mut package.traits[0];
    contract.type_parameters = vec![id(3), id(4)];
    contract.parameter_types = vec![Term::Primitive(Primitive::Type), rigid_type(3)];
    let implementation = &mut package.implementations[0];
    implementation.type_parameters = vec![id(11), id(12)];
    implementation.parameter_types = vec![Term::Primitive(Primitive::Type), rigid_type(11)];
    implementation.interface_arguments = vec![rigid_type(11), value(12)];
    package.functions[0].capabilities[0].interface_arguments = vec![boolean(), value(80)];
    let scope = [(id(80), boolean())];
    let selected = prepare(&package, &scope, &mut Budget::default()).unwrap();
    assert_eq!(selected[0].arguments, vec![boolean(), value(80)]);
    package.functions[0].capabilities[0].interface_arguments[0] = Term::Primitive(Primitive::Char);
    let failure = prepare(&package, &scope, &mut Budget::default()).unwrap_err();
    assert!(failure.contains("domain evidence"), "{failure}");
}

#[test]
fn private_vm_installs_scope_before_dispatch_admission() {
    use comptime_interpretor::{Budget as VmBudget, ErrorKind, Interpreter, Value};
    let scope = [(id(80), boolean())];
    let mut package = fixture(boolean(), value(80));
    package.dispatch_instances = prepare(&package, &scope, &mut Budget::default()).unwrap();
    for closed in [
        Interpreter::new(&package, VmBudget::default()),
        Interpreter::for_comptime(&package, VmBudget::default()),
    ] {
        assert_eq!(closed.err().unwrap().kind, ErrorKind::InvalidCallPlan);
    }
    let mut budget = VmBudget::default();
    let before = budget.steps;
    let mut vm =
        Interpreter::for_comptime_in_scope_in_budget(&package, &[], &[], &scope, &mut budget)
            .unwrap();
    assert_eq!(budget.steps, vm.remaining_steps());
    assert!(budget.steps < before && budget.steps > 0);
    assert_eq!(vm.invoke(id(20), vec![]).unwrap(), Value::Bool(true));
    assert_eq!(
        vm.decode_immutable_input(&value(80), &boolean()).unwrap(),
        Value::AbstractValue {
            declaration: id(80),
            ty: boolean()
        },
    );
    // Constructing and executing a private VM grants no permission to the next
    // VM using the same immutable snapshot.
    assert_eq!(
        Interpreter::for_comptime(&package, VmBudget::default())
            .err()
            .unwrap()
            .kind,
        ErrorKind::InvalidCallPlan,
    );
}

#[test]
fn private_vm_rejects_foreign_scope_and_recovers_after_failure() {
    use comptime_interpretor::{Budget as VmBudget, ErrorKind, Interpreter};
    let scope = [(id(80), boolean())];
    let mut package = fixture(boolean(), value(80));
    package.dispatch_instances = prepare(&package, &scope, &mut Budget::default()).unwrap();
    for scope in [
        vec![],
        vec![(id(81), boolean())],
        vec![(id(80), Term::Primitive(Primitive::Char))],
    ] {
        let mut budget = VmBudget::default();
        let failed =
            Interpreter::for_comptime_in_scope_in_budget(&package, &[], &[], &scope, &mut budget);
        assert_eq!(failed.err().unwrap().kind, ErrorKind::InvalidCallPlan);
        assert_eq!(budget.steps, 0);
    }
    let mut budget = VmBudget::default();
    assert_eq!(
        Interpreter::for_comptime_in_scope_in_budget(
            &package, &[], &[id(80)], &scope, &mut budget,
        ).err().unwrap().kind,
        ErrorKind::Type,
    );
    assert_eq!(budget.steps, 0);
    budget.steps = 1;
    assert_eq!(
        Interpreter::for_comptime_in_scope_in_budget(&package, &[], &[], &scope, &mut budget,)
            .err()
            .unwrap()
            .kind,
        ErrorKind::StepsExhausted,
    );
    assert_eq!(budget.steps, 0);
    assert!(
        Interpreter::for_comptime_in_scope_in_budget(
            &package,
            &[],
            &[],
            &scope,
            &mut VmBudget::default(),
        )
        .is_ok()
    );
}

#[test]
fn sharing_private_code_does_not_publish_scoped_source_indexes() {
    use comptime_interpretor::{Budget as VmBudget, CodeSession, ErrorKind, Interpreter, Value};
    let scope = [(id(80), boolean())];
    let mut package = fixture(boolean(), value(80));
    package.dispatch_instances = prepare(&package, &scope, &mut Budget::default()).unwrap();
    let package = std::sync::Arc::new(package);
    let mut session = CodeSession::new(package.clone());
    let mut vm = Interpreter::for_comptime_in_scope_in_budget(
        &package,
        &[],
        &[],
        &scope,
        &mut VmBudget::default(),
    )
    .unwrap();
    assert_eq!(
        vm.with_code_session(&mut session, |vm| vm.invoke(id(20), vec![]))
            .unwrap(),
        Value::Bool(true),
    );
    let mut budget = VmBudget::default();
    assert_eq!(
        Interpreter::for_comptime_in_session(&package, &mut session, &mut budget)
            .err()
            .unwrap()
            .kind,
        ErrorKind::InvalidCallPlan,
    );
    assert_eq!(budget.steps, 0);
    let mut retry = Interpreter::for_comptime_in_scope_in_budget(
        &package,
        &[],
        &[],
        &scope,
        &mut VmBudget::default(),
    )
    .unwrap();
    assert_eq!(
        retry
            .with_code_session(&mut session, |vm| vm.invoke(id(20), vec![]))
            .unwrap(),
        Value::Bool(true),
    );
}

#[test]
fn constructor_scope_does_not_make_malformed_dispatch_payloads_valid() {
    use comptime_interpretor::{Budget as VmBudget, ErrorKind, Interpreter};
    let scope = [(id(80), boolean())];
    let mut package = fixture(boolean(), value(80));
    package.dispatch_instances = prepare(&package, &scope, &mut Budget::default()).unwrap();
    let instance = &mut package.dispatch_instances[0];
    // Preserve the selected implementation's substitution relationship while
    // supplying the wrong ordinary value domain. Structural agreement alone
    // must not let the newly prepared VM escape to its caller.
    instance.arguments = vec![Term::Char('x')];
    instance.interface_arguments = instance.arguments.clone();
    let mut budget = VmBudget::default();
    assert_eq!(
        Interpreter::for_comptime_in_scope_in_budget(&package, &[], &[], &scope, &mut budget,)
            .err()
            .unwrap()
            .kind,
        ErrorKind::InvalidCallPlan,
    );
    assert_eq!(budget.steps, 0);
}

fn staged_associated_fixture(second_associated_on_body: bool) -> n::Package {
    let source = nir::program::SourceRange { start: 0, end: 0 };
    let expr = |kind, ty| n::Expr { source, ty, kind };
    let assoc_term = |interface, member, subject| Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin { interface, member }),
        arguments: vec![],
        subject: Box::new(subject),
    };
    let project_id = id(1);
    let project_self = id(2);
    let project_out = id(3);
    let project_method = id(4);
    let second_id = id(5);
    let second_self = id(6);
    let second_out = id(7);
    let project_contract_sig = Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::ComptimeValue,
            ty: Term::Rigid {
                declaration: project_self,
                sort: Sort::Type,
            },
        }],
        result: Box::new(assoc_term(
            project_id,
            project_out,
            Term::Rigid {
                declaration: project_self,
                sort: Sort::Type,
            },
        )),
    };
    let bool_param_sig = |result: Term| Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::ComptimeValue,
            ty: boolean(),
        }],
        result: Box::new(result),
    };
    let body_result = if second_associated_on_body {
        assoc_term(second_id, second_out, boolean())
    } else {
        boolean()
    };
    let mut package = n::Package {
        traits: vec![n::Trait {
            comptime_only: false,
            declaration: project_id,
            self_parameter: project_self,
            name: "Project".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            associated: vec![n::AssociatedContract {
                phase: Default::default(),
                declaration: project_out,
                name: "Out".into(),
                ty: Term::Primitive(Primitive::Type),
                default: None,
            }],
            requirements: vec![],
            capabilities: vec![],
            parents: vec![],
            methods: vec![n::MethodContract {
                source,
                phase: Default::default(),
                is_quote: false,
                pure: false,
                is_unsafe: false,
                type_arguments: vec![],
                type_parameters: vec![],
                capabilities: vec![],
                program: Default::default(),
                parameter_bindings: vec![n::LocalId { owner: 4, index: 0 }],
                by_value_receiver: false,
                by_reference_receiver: false,
                declaration: project_method,
                name: "project".into(),
                signature: project_contract_sig.into(),
                requirements: vec![],
                default: None,
            }],
        }],
        implementations: vec![n::Implementation {
            declaration: id(10),
            phases: n::ImplementationPhases {
                runtime: true,
                comptime: true,
            },
            type_parameters: vec![],
            parameter_types: vec![],
            interface: Some(project_id),
            interface_arguments: vec![],
            subject: boolean(),
            requirements: vec![],
            capabilities: vec![],
            associated: vec![(
                project_out,
                expr(
                    n::ExprKind::TypeValue(boolean()),
                    Term::Primitive(Primitive::Type),
                ),
            )],
            associated_initializers: vec![],
            methods: vec![("project".into(), id(11))],
        }],
        functions: vec![
            n::Function {
                interface: n::FunctionDeclaration {
                    source,
                    declaration: id(11),
                    name: "project_bool".into(),
                    phase: n::FunctionPhase::Runtime,
                    pure: false,
                    inline: false,
                    is_unsafe: false,
                    type_parameters: vec![],
                    parameters: vec![n::LocalId {
                        owner: 11,
                        index: 0,
                    }],
                    env_parameters: vec![],
                    signature: bool_param_sig(body_result),
                    defaults: vec![],
                    preconditions: vec![],
                    postconditions: vec![],
                    result_binding: None,
                    result_type_binder: None,
                    result_guarantees: vec![],
                    entry_snapshots: vec![],
                    requirements: vec![],
                    capabilities: vec![],
                },
                body: expr(n::ExprKind::Literal(n::Literal::Bool(true)), boolean()),
                runtime_deliveries: Default::default(),
            },
            n::Function {
                interface: n::FunctionDeclaration {
                    source,
                    declaration: id(20),
                    name: "caller".into(),
                    phase: n::FunctionPhase::Runtime,
                    pure: false,
                    inline: false,
                    is_unsafe: false,
                    type_parameters: vec![],
                    parameters: vec![],
                    env_parameters: vec![],
                    signature: Term::Function {
                        parameters: vec![],
                        result: Box::new(boolean()),
                    },
                    defaults: vec![],
                    preconditions: vec![],
                    postconditions: vec![],
                    result_binding: None,
                    result_type_binder: None,
                    result_guarantees: vec![],
                    entry_snapshots: vec![],
                    requirements: vec![],
                    capabilities: vec![],
                },
                body: expr(
                    n::ExprKind::Call {
                        callee: Box::new(expr(
                            n::ExprKind::TraitMethod {
                                method_arguments: vec![],
                                interface_arguments: vec![],
                                interface: project_id,
                                member: project_method,
                                subject: boolean(),
                            },
                            bool_param_sig(assoc_term(project_id, project_out, boolean())),
                        )),
                        arguments: vec![n::CallArgument {
                            passing: Default::default(),
                            parameter: 0,
                            value: expr(n::ExprKind::Literal(n::Literal::Bool(true)), boolean()),
                            is_default: false,
                        }],
                        env_arguments: vec![],
                        parameter_bindings: vec![n::LocalId {
                            owner: 20,
                            index: 0,
                        }],
                    },
                    boolean(),
                ),
                runtime_deliveries: Default::default(),
            },
        ],
        ..Default::default()
    };
    if second_associated_on_body {
        package.traits.push(n::Trait {
            comptime_only: false,
            declaration: second_id,
            self_parameter: second_self,
            name: "Second".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            associated: vec![n::AssociatedContract {
                phase: Default::default(),
                declaration: second_out,
                name: "Out".into(),
                ty: Term::Primitive(Primitive::Type),
                default: None,
            }],
            requirements: vec![],
            capabilities: vec![],
            parents: vec![],
            methods: vec![],
        });
        package.implementations.push(n::Implementation {
            declaration: id(12),
            phases: n::ImplementationPhases {
                runtime: true,
                comptime: true,
            },
            type_parameters: vec![],
            parameter_types: vec![],
            interface: Some(second_id),
            interface_arguments: vec![],
            subject: boolean(),
            requirements: vec![],
            capabilities: vec![],
            associated: vec![(
                second_out,
                expr(
                    n::ExprKind::TypeValue(boolean()),
                    Term::Primitive(Primitive::Type),
                ),
            )],
            associated_initializers: vec![],
            methods: vec![],
        });
    }
    package
}

#[test]
fn staged_trait_method_retains_associated_type_in_contract_signature() {
    use comptime_interpretor::{Budget as VmBudget, Interpreter};
    let mut package = staged_associated_fixture(false);
    package.dispatch_instances = dispatch_instances(&package, Budget::default()).unwrap();
    let project = package
        .dispatch_instances
        .iter()
        .find(|instance| instance.interface == id(1) && instance.subject == boolean())
        .expect("Project dispatch instance");
    assert_eq!(project.methods.len(), 1);
    assert!(
        project
            .associated
            .iter()
            .any(|(member, _)| *member == id(3)),
        "expected Out associated type to be retained, got {:?}",
        project.associated
    );
    Interpreter::for_comptime(&package, VmBudget::default()).unwrap();
}

#[test]
fn staged_trait_method_retains_associated_type_from_implementation_body_signature() {
    use comptime_interpretor::{Budget as VmBudget, Interpreter};
    let mut package = staged_associated_fixture(true);
    package.dispatch_instances = dispatch_instances(&package, Budget::default()).unwrap();
    let project = package
        .dispatch_instances
        .iter()
        .find(|instance| instance.interface == id(1) && instance.subject == boolean())
        .expect("Project dispatch instance");
    let second = package
        .dispatch_instances
        .iter()
        .find(|instance| instance.interface == id(5) && instance.subject == boolean())
        .expect("Second dispatch instance");
    assert!(
        project
            .associated
            .iter()
            .any(|(member, _)| *member == id(3))
    );
    assert!(second.associated.iter().any(|(member, _)| *member == id(7)));
    Interpreter::for_comptime(&package, VmBudget::default()).unwrap();
}

#[test]
fn staged_callable_with_dependent_and_closed_associated_types_retains_only_closed_supply() {
    let source = nir::program::SourceRange { start: 0, end: 0 };
    let expr = |kind, ty| n::Expr { source, ty, kind };
    let project_id = id(1);
    let project_out = id(3);
    let assoc_term = |subject| Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: project_id,
            member: project_out,
        }),
        arguments: vec![],
        subject: Box::new(subject),
    };
    let package = n::Package {
        traits: vec![n::Trait {
            comptime_only: false,
            declaration: project_id,
            self_parameter: id(2),
            name: "Project".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            associated: vec![n::AssociatedContract {
                phase: Default::default(),
                declaration: project_out,
                name: "Out".into(),
                ty: Term::Primitive(Primitive::Type),
                default: None,
            }],
            requirements: vec![],
            capabilities: vec![],
            parents: vec![],
            methods: vec![],
        }],
        implementations: vec![n::Implementation {
            declaration: id(10),
            phases: n::ImplementationPhases {
                runtime: true,
                comptime: true,
            },
            type_parameters: vec![],
            parameter_types: vec![],
            interface: Some(project_id),
            interface_arguments: vec![],
            subject: boolean(),
            requirements: vec![],
            capabilities: vec![],
            associated: vec![(
                project_out,
                expr(
                    n::ExprKind::TypeValue(boolean()),
                    Term::Primitive(Primitive::Type),
                ),
            )],
            associated_initializers: vec![],
            methods: vec![],
        }],
        functions: vec![n::Function {
            interface: n::FunctionDeclaration {
                source,
                declaration: id(21),
                name: "mixed_staged".into(),
                phase: n::FunctionPhase::Runtime,
                pure: false,
                inline: false,
                is_unsafe: false,
                type_parameters: vec![],
                parameters: vec![
                    n::LocalId {
                        owner: 21,
                        index: 0,
                    },
                    n::LocalId {
                        owner: 21,
                        index: 1,
                    },
                ],
                signature: Term::Function {
                    parameters: vec![
                        nir::Parameter {
                            sort: Sort::Type,
                            ty: Term::Primitive(Primitive::Type),
                        },
                        nir::Parameter {
                            sort: Sort::Runtime,
                            ty: assoc_term(Term::Bound {
                                depth: 0,
                                index: 0,
                                sort: Sort::Type,
                            }),
                        },
                    ],
                    result: Box::new(assoc_term(boolean())),
                },
                env_parameters: vec![],
                defaults: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                entry_snapshots: vec![],
                requirements: vec![],
                capabilities: vec![],
            },
            body: expr(n::ExprKind::Literal(n::Literal::Bool(true)), boolean()),
            runtime_deliveries: Default::default(),
        }],
        ..Default::default()
    };
    let dispatch = dispatch_instances(&package, Budget::default()).unwrap();
    let project = dispatch
        .iter()
        .find(|instance| instance.interface == project_id && instance.subject == boolean())
        .expect("closed Project dispatch instance");
    assert!(
        project
            .associated
            .iter()
            .any(|(member, _)| *member == project_out),
        "expected closed Out supply to be retained, got {:?}",
        project.associated
    );
}
