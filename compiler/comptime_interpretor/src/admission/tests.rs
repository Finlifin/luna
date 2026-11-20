use super::*;
use nir::program::{Struct, TraitRequirement, TypePredicate};
use std::cell::Cell;

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 83, local }
}

fn ty(primitive: Primitive) -> Term {
    Term::Primitive(primitive)
}

#[test]
fn nominal_contract_substitution_charges_payloads_before_copying() {
    let wide = Term::Record(vec![("field".repeat(4096), ty(Primitive::Bool))]);
    let package = Package {
        structs: vec![Struct {
            declaration: id(1),
            name: "Guarded".into(),
            type_parameters: vec![],
            parameter_types: vec![],
            fields: vec![],
            capabilities: vec![],
            requirements: vec![TypePredicate::Equal(wide.clone(), wide)],
            packed: false,
        }],
        ..Default::default()
    };
    let nominal = Term::Nominal {
        declaration: id(1),
        arguments: vec![],
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    vm.budget.steps = 200;
    assert_eq!(
        vm.admit_type(&nominal, 0).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert!(vm.remaining_steps() < 200);
    assert!(vm.events.is_empty());
    vm.budget.steps = 100_000;
    vm.admit_type(&nominal, 0).unwrap();
    assert!(vm.remaining_steps() < 100_000 - 20_000);
}

#[test]
fn alias_capability_failure_prevents_body_effects_and_can_retry() {
    let capability = TraitRequirement {
        interface: id(2),
        subject: ty(Primitive::Bool),
        interface_arguments: vec![],
    };
    let package = Package {
        alias_contracts: vec![nir::type_aliases::Contract {
            declaration: id(1),
            parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![capability.clone()],
        }],
        ..Default::default()
    };
    let source = SourceRange { start: 1, end: 7 };
    let marker = LocalId {
        owner: 1,
        index: 99,
    };
    let body = Expr {
        source,
        ty: ty(Primitive::Type),
        kind: K::Block {
            statements: vec![Expr {
                source,
                ty: ty(Primitive::Void),
                kind: K::Assign {
                    place: nir::program::Place {
                        root: marker,
                        fields: vec![],
                    },
                    value: Box::new(Expr {
                        source,
                        ty: ty(Primitive::Bool),
                        kind: K::Literal(Literal::Bool(true)),
                    }),
                },
            }],
            result: Some(Box::new(Expr {
                source,
                ty: ty(Primitive::Type),
                kind: K::TypeValue(ty(Primitive::Bool)),
            })),
        },
    };
    let expression = Expr {
        source,
        ty: ty(Primitive::Type),
        kind: K::CheckTypeAlias {
            declaration: id(1),
            preparations: vec![],
            bindings: None,
            order: None,
            arguments: vec![],
            body: Some(Box::new(body)),
        },
    };
    let mode = Cell::new(0);
    let solver = |predicate: &TypePredicate, fuel: &mut u64| {
        assert_eq!(predicate, &TypePredicate::Trait(capability.clone()));
        *fuel = fuel.saturating_sub(9);
        Ok(match mode.get() {
            0 => ContractOutcome::Unknown,
            1 => ContractOutcome::Rejected,
            _ => ContractOutcome::Satisfied,
        })
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_budgeted_type_solver(&solver);
    let mut environment = Environment::new();
    environment.insert(marker, Value::Bool(false));
    for (state, expected) in [
        (0, ErrorKind::TypeContractUnresolved),
        (1, ErrorKind::TypeContractRejected),
    ] {
        mode.set(state);
        let before = vm.remaining_steps();
        assert_eq!(
            vm.eval(&expression, &mut environment).unwrap_err().kind,
            expected
        );
        assert_eq!(environment.get(&marker), Some(&Value::Bool(false)));
        assert_eq!(environment.len(), 1);
        assert!(vm.remaining_steps() < before - 8);
    }
    mode.set(2);
    assert!(
        matches!(vm.eval(&expression, &mut environment), Ok(Flow::Value(Value::Type(value))) if value == ty(Primitive::Bool))
    );
    assert_eq!(environment.get(&marker), Some(&Value::Bool(true)));
}

#[test]
fn nested_value_domains_use_the_telescope_that_declared_the_index() {
    use nir::{Parameter, Sort};
    let type_parameter = id(10);
    let value_parameter = id(11);
    let formal = Term::Rigid {
        declaration: type_parameter,
        sort: Sort::Type,
    };
    let structure = Struct {
        declaration: id(12),
        name: "Tagged".into(),
        type_parameters: vec![type_parameter, value_parameter],
        parameter_types: vec![ty(Primitive::Type), formal.clone()],
        fields: vec![],
        requirements: vec![TypePredicate::Equal(formal.clone(), formal.clone())],
        capabilities: vec![],
        packed: false,
    };
    let operation = nir::effects::Declaration {
        declaration: id(13),
        group: None,
        name: "observe".into(),
        type_parameters: structure.type_parameters.clone(),
        parameter_types: structure.parameter_types.clone(),
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(formal.clone()),
        },
        abi: nir::effects::Abi::Direct,
        inline: false,
        requirements: vec![],
        capabilities: vec![],
    };
    let package = Package {
        structs: vec![structure],
        effects: vec![operation],
        ..Default::default()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let universe = Parameter {
        sort: Sort::Type,
        ty: ty(Primitive::Type),
    };
    let bound = |depth, index, sort| Term::Bound { depth, index, sort };
    let outer = |inner| Term::Function {
        parameters: vec![
            universe.clone(),
            Parameter {
                sort: Sort::ComptimeValue,
                ty: bound(0, 0, Sort::Type),
            },
        ],
        result: Box::new(Term::Function {
            parameters: vec![universe.clone()],
            result: Box::new(inner),
        }),
    };
    // Both T and U use slot zero. The outer tag's domain must be lifted
    // across the inner binder before comparison with either type.
    for effect in [false, true] {
        let application = |depth| {
            let arguments = vec![
                bound(depth, 0, Sort::Type),
                bound(1, 1, Sort::ComptimeValue),
            ];
            if effect {
                Term::computation(
                    vec![nir::effects::Operation {
                        declaration: id(13),
                        arguments,
                    }],
                    ty(Primitive::Bool),
                )
            } else {
                Term::Nominal {
                    declaration: id(12),
                    arguments,
                }
            }
        };
        assert_eq!(
            vm.admit_type(&outer(application(0)), 0).unwrap_err().kind,
            ErrorKind::Type
        );
        vm.admit_type(&outer(application(1)), 0).unwrap();
    }
    let forward = Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Runtime,
                ty: bound(0, 1, Sort::Type),
            },
            universe,
        ],
        result: Box::new(ty(Primitive::Bool)),
    };
    assert_eq!(
        vm.admit_type(&forward, 0).unwrap_err().kind,
        ErrorKind::Type
    );
    assert!(vm.events.is_empty());
}

#[test]
fn callable_formation_keeps_inner_binders_out_of_closed_proof_queries() {
    let subject = Term::Rigid {
        declaration: id(2),
        sort: nir::Sort::Type,
    };
    let package = Package {
        structs: vec![Struct {
            declaration: id(1),
            name: "Guarded".into(),
            type_parameters: vec![id(2)],
            parameter_types: vec![ty(Primitive::Type)],
            fields: vec![],
            requirements: vec![TypePredicate::Equal(subject, ty(Primitive::Bool))],
            capabilities: vec![],
            packed: false,
        }],
        ..Default::default()
    };
    let queried = Cell::new(0);
    let solver = |_: &TypePredicate, _: &mut u64| {
        queried.set(queried.get() + 1);
        Ok(ContractOutcome::Rejected)
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_budgeted_type_solver(&solver);
    let signature = |argument| Term::Function {
        parameters: vec![nir::Parameter {
            sort: nir::Sort::Type,
            ty: ty(Primitive::Type),
        }],
        result: Box::new(Term::Nominal {
            declaration: id(1),
            arguments: vec![argument],
        }),
    };
    let open = signature(Term::Bound {
        depth: 0,
        index: 0,
        sort: nir::Sort::Type,
    });
    assert_eq!(
        vm.admit_type(&open, 0).unwrap_err().kind,
        ErrorKind::TypeContractUnresolved
    );
    assert_eq!(queried.get(), 0);
    assert_eq!(
        vm.admit_type(&signature(ty(Primitive::Char)), 0)
            .unwrap_err()
            .kind,
        ErrorKind::TypeContractRejected
    );
    assert_eq!(queried.get(), 1);
    vm.admit_type(&signature(ty(Primitive::Bool)), 0).unwrap();
}
