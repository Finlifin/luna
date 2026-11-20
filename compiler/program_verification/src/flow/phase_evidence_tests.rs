//! Completed phase evidence cannot discharge program arithmetic obligations.
use super::*;
use nir::{DeclarationId, Parameter, Primitive, Sort, program::*};

fn integer(value: u128) -> Expr {
    Expr {
        source: SourceRange { start: 10, end: 20 },
        ty: Term::Primitive(Primitive::U8),
        kind: K::Literal(Literal::UnsignedInteger(value)),
    }
}

fn package(left: u128, in_default: bool) -> Package {
    let body = Expr {
        kind: K::IntegerBinary {
            op: IntegerOp::Add,
            left: Box::new(integer(left)),
            right: Box::new(integer(1)),
        },
        ..integer(0)
    };
    let local = LocalId { owner: 1, index: 0 };
    Package {
        functions: vec![Function {
            body: if in_default { integer(0) } else { body.clone() },
            interface: nir::program::FunctionDeclaration {
                source: nir::program::SourceRange { start: 0, end: 0 },
                declaration: DeclarationId {
                    package: 1,
                    local: 1,
                },
                name: "arithmetic".into(),
                phase: FunctionPhase::Comptime,
                inline: false,
                pure: true,
                is_unsafe: false,
                signature: Term::Function {
                    parameters: if in_default {
                        vec![Parameter {
                            sort: Sort::Runtime,
                            ty: integer(0).ty,
                        }]
                    } else {
                        vec![]
                    },
                    result: Box::new(integer(0).ty),
                },
                parameters: if in_default { vec![local] } else { vec![] },
                env_parameters: vec![],
                type_parameters: vec![],
                defaults: if in_default { vec![(0, body)] } else { vec![] },
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                result_type_binder: None,
                result_guarantees: vec![],
                entry_snapshots: vec![],
                requirements: vec![],
                capabilities: vec![],
            },
        }],
        ..Default::default()
    }
}

#[test]
fn phase_success_still_requires_body_and_default_arithmetic_proofs() {
    for in_default in [false, true] {
        for left in [254, 255] {
            let package = package(left, in_default);
            let phase = nir::phases::Inspection::new(&package)
                .check_package_with_evidence(&mut 100_000)
                .unwrap();
            assert!(
                phase
                    .machine_arithmetic_functions()
                    .contains(&package.functions[0].declaration)
            );
            let checked = validate_machine_operations_checked(&phase);
            let ordinary = validate_machine_operations(&package);
            if left == 254 {
                checked.unwrap();
                ordinary.unwrap();
            } else {
                assert_eq!(checked.unwrap_err().reason, Reason::MissingMachineProof);
                assert_eq!(ordinary.unwrap_err().reason, Reason::MissingMachineProof);
            }
        }
    }
}

#[test]
fn region_and_dependency_with_one_lexical_owner_require_separate_proofs() {
    for (dependency_left, region_left, valid) in
        [(254, 254, true), (254, 255, false), (255, 254, false)]
    {
        let dependencies = package(dependency_left, true);
        let source = package(region_left, false).functions.pop().unwrap();
        let region = DefinitionRegion {
            owner: dependencies.functions[0].declaration,
            signature: source.interface.signature,
            phase: FunctionPhase::Comptime,
            type_parameters: vec![],
            parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
            body: source.body,
        };
        let checked = validate_definition_region(&dependencies, &region);
        if valid {
            checked.unwrap();
        } else {
            assert_eq!(checked.unwrap_err().reason, Reason::MissingMachineProof);
        }
    }
}

#[test]
fn region_calls_use_the_selected_residual_contract_or_the_imported_declaration() {
    let mut callee = package(0, false).functions.pop().unwrap();
    let id = callee.declaration;
    callee.phase = FunctionPhase::Runtime;
    let condition = |value| Expr {
        source: callee.source,
        ty: Term::Primitive(Primitive::Bool),
        kind: ExprKind::Literal(Literal::Bool(value)),
    };
    let denied = condition(false);
    let allowed = condition(true);
    callee.preconditions = vec![allowed];
    let imports = RegionDeclarations::from([(id, callee.interface.clone())]);
    let region = DefinitionRegion {
        owner: id,
        phase: FunctionPhase::Runtime,
        parameters: vec![],
        signature: callee.signature.clone(),
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
        body: Expr {
            kind: ExprKind::Call {
                callee: Box::new(Expr {
                    source: callee.source,
                    ty: callee.callable_type(),
                    kind: ExprKind::Function(id),
                }),
                arguments: vec![],
                env_arguments: vec![],
                parameter_bindings: vec![],
            },
            ..integer(0)
        },
    };
    let mut world = Package::default();
    validate_definition_region_with_declarations(&world, &imports, &region).unwrap();
    callee.preconditions = vec![denied];
    world.runtime_instances.push(RuntimeInstance {
        declaration: id,
        arguments: vec![],
        function: callee,
        cleanup_accesses: vec![],
    });
    let failure =
        validate_definition_region_with_declarations(&world, &imports, &region).unwrap_err();
    assert_eq!(failure.reason, Reason::MissingPreconditionProof);
    world
        .runtime_instances
        .push(world.runtime_instances[0].clone());
    let failure =
        validate_definition_region_with_declarations(&world, &imports, &region).unwrap_err();
    assert_eq!(failure.reason, Reason::InvalidSignature);
}

#[test]
fn projected_integer_domains_prove_every_admitted_width_before_execution() {
    let package = Package::default();
    let binder = DeclarationId {
        package: 1,
        local: 2,
    };
    let ty = Term::Rigid {
        declaration: binder,
        sort: Sort::Type,
    };
    let literal = |spelling: &str| Expr {
        source: SourceRange { start: 8, end: 12 },
        ty: ty.clone(),
        kind: K::Literal(Literal::PendingInteger {
            spelling: spelling.into(),
            negative: false,
        }),
    };
    let domain = |p| TypePredicate::Equal(ty.clone(), Term::Primitive(p));
    let mut region = DefinitionRegion {
        owner: DeclarationId {
            package: 1,
            local: 1,
        },
        type_parameters: vec![binder],
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(ty.clone()),
        },
        parameters: vec![],
        phase: FunctionPhase::Comptime,
        requirements: vec![domain(Primitive::U16)],
        capabilities: vec![],
        body: Expr {
            source: SourceRange { start: 8, end: 16 },
            ty: ty.clone(),
            kind: K::IntegerBinary {
                op: IntegerOp::Add,
                left: Box::new(literal("255")),
                right: Box::new(literal("1")),
            },
        },
    };
    validate_definition_region(&package, &region).unwrap();
    region.requirements = vec![TypePredicate::Or(vec![
        domain(Primitive::U16),
        domain(Primitive::U8),
    ])];
    let failure = validate_definition_region(&package, &region).unwrap_err();
    assert_eq!(failure.reason, Reason::MissingMachineProof);
    assert_eq!(failure.source, region.body.source);

    region.requirements = vec![domain(Primitive::Usize)];
    assert_eq!(
        validate_definition_region(&package, &region)
            .unwrap_err()
            .reason,
        Reason::MissingMachineProof
    );
    let known = Package {
        target: Some(nir::target::TargetData::new(16).unwrap()),
        ..Package::default()
    };
    validate_definition_region(&known, &region).unwrap();
    assert_eq!(region.body.ty, ty);
}

#[test]
fn a_default_cannot_use_the_entry_assumption_that_proves_the_body() {
    let mut package = package(0, true);
    let function = &mut package.functions[0];
    let input = Expr {
        kind: K::Local(function.parameters[0]),
        ..integer(0)
    };
    let successor = Expr {
        kind: K::IntegerBinary {
            op: IntegerOp::Add,
            left: Box::new(input.clone()),
            right: Box::new(integer(1)),
        },
        ..integer(0)
    };
    function.interface.defaults.clear();
    function.body = successor.clone();
    function.interface.preconditions.push(Expr {
        source: input.source,
        ty: Term::Primitive(Primitive::Bool),
        kind: K::ScalarLess(Box::new(input), Box::new(integer(255))),
    });
    validate_machine_operations(&package).unwrap();

    let function = &mut package.functions[0];
    let Term::Function { parameters, .. } = &mut function.interface.signature else {
        unreachable!()
    };
    parameters.push(parameters[0].clone());
    function
        .interface
        .parameters
        .push(LocalId { owner: 1, index: 1 });
    function.interface.defaults.push((1, successor));
    assert_eq!(
        validate_machine_operations(&package).unwrap_err().reason,
        Reason::MissingMachineProof
    );
}

#[test]
fn source_owned_member_programs_keep_their_machine_proofs_in_the_original_world() {
    use type_resolution::dispatch::DeclarationSnapshot;
    let id = |local| DeclarationId { package: 1, local };
    for left in [254, 255] {
        let body = package(left, false).functions.pop().unwrap().body;
        let template =
            nir::associated_initializers::AssociatedInitializer::new(body, &mut 1000).unwrap();
        let source = Package {
            implementations: vec![Implementation {
                declaration: id(1),
                phases: Default::default(),
                interface: None,
                interface_arguments: vec![],
                subject: Term::Primitive(Primitive::U8),
                type_parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
                associated: vec![],
                associated_initializers: vec![(id(2), template)],
                methods: vec![],
            }],
            ..Default::default()
        };
        let snapshot = DeclarationSnapshot::capture_member_programs(source, &mut 1000).unwrap();
        let phase = type_resolution::dispatch::DeclarationSource::Snapshot(&snapshot)
            .phase_inspection()
            .check_package_with_evidence(&mut 1000)
            .unwrap();
        assert!(
            phase
                .arithmetic_obligations()
                .initializers()
                .contains(&(id(1), id(2)))
        );
        let checked = validate_machine_operations_checked(&phase);
        if left == 254 {
            checked.unwrap();
        } else {
            assert_eq!(checked.unwrap_err().reason, Reason::MissingMachineProof);
        }

        let bare = nir::phases::Inspection::new(snapshot.package())
            .check_package_with_evidence(&mut 1000)
            .unwrap();
        assert!(bare.arithmetic_obligations().initializers().is_empty());
        validate_machine_operations_checked(&bare).unwrap();
    }
}
