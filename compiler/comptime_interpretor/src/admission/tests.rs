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
        match mode.get() {
            0 => ContractOutcome::Unknown,
            1 => ContractOutcome::Rejected,
            _ => ContractOutcome::Satisfied,
        }
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
