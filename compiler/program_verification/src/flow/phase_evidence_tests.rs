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
                type_parameters: vec![],
                defaults: if in_default { vec![(0, body)] } else { vec![] },
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
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
        let checked = validate_definition_region(&dependencies, region);
        if valid {
            checked.unwrap();
        } else {
            assert_eq!(checked.unwrap_err().reason, Reason::MissingMachineProof);
        }
    }
}
