use nir::{ContractError, Parameter, Primitive, Sort, Term, TypeError, TypeInterner, TypeRef};
use std::sync::Arc;

fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn type_parameter() -> Parameter {
    Parameter {
        sort: Sort::Type,
        ty: Term::Primitive(Primitive::Type),
    }
}
fn hole() -> Term {
    Term::Hole {
        index: 0,
        sort: Sort::Type,
    }
}
fn bound(depth: u32) -> Term {
    Term::Bound {
        depth,
        index: 0,
        sort: Sort::Type,
    }
}
fn payload() -> Term {
    Term::Record(vec![("x".repeat(1000), boolean())])
}

#[test]
fn repeated_holes_charge_expanded_payload_and_retry_without_mutation() {
    let mut pool = TypeInterner::default();
    let core = pool
        .intern(
            vec![type_parameter()],
            Term::Record((0..10).map(|i| (format!("f{i}"), hole())).collect()),
        )
        .unwrap();
    let reference = TypeRef::bind(core.clone(), vec![payload()]).unwrap();
    assert_eq!(
        reference.materialize_with_budget(&mut 2000),
        Err(TypeError::ConstructionBudget)
    );
    let expanded = reference.materialize_with_budget(&mut 20_000).unwrap();
    assert_eq!(
        expanded,
        Term::Record((0..10).map(|i| (format!("f{i}"), payload())).collect())
    );
    assert!(Arc::ptr_eq(reference.core(), &core));
    assert_eq!(reference.arguments(), &[payload()]);
}

#[test]
fn combined_depth_and_binding_inputs_are_bounded() {
    let mut pool = TypeInterner::default();
    let wrap = |value| (0..70).fold(value, |ty, _| Term::Pointer(Box::new(ty)));
    let core = pool.intern(vec![type_parameter()], wrap(hole())).unwrap();
    let reference = TypeRef::bind(core.clone(), vec![wrap(boolean())]).unwrap();
    assert_eq!(
        reference.materialize_with_budget(&mut 10_000),
        Err(TypeError::ConstructionBudget)
    );
    let deep = (0..140).fold(boolean(), |ty, _| Term::Pointer(Box::new(ty)));
    assert_eq!(
        TypeRef::bind(core.clone(), vec![deep]),
        Err(TypeError::ConstructionBudget)
    );
    assert_eq!(
        TypeRef::bind_with_budget(core.clone(), vec![boolean()], &mut 0),
        Err(TypeError::ConstructionBudget)
    );
    TypeRef::bind_with_budget(core, vec![boolean()], &mut 100).unwrap();
}

#[test]
fn composition_accounts_for_repetition_and_preserves_nested_binders() {
    let mut pool = TypeInterner::default();
    let core = pool.intern(vec![type_parameter()], hole()).unwrap();
    let nested = Term::Function {
        parameters: vec![type_parameter()],
        result: Box::new(Term::Record(vec![
            ("local".into(), bound(0)),
            ("outer".into(), bound(1)),
        ])),
    };
    let reference = TypeRef::bind(core.clone(), vec![nested]).unwrap();
    let composed = reference
        .substitute_with_budget(&[bound(0)], &mut 1000)
        .unwrap();
    assert!(Arc::ptr_eq(reference.core(), composed.core()));
    assert_eq!(composed.arguments(), reference.arguments());
    let repeated = TypeRef::bind(
        core,
        vec![Term::Record(
            (0..10).map(|i| (format!("f{i}"), bound(0))).collect(),
        )],
    )
    .unwrap();
    assert_eq!(
        repeated.substitute_with_budget(&[payload()], &mut 2000),
        Err(TypeError::ConstructionBudget)
    );
    let result = repeated
        .substitute_with_budget(&[payload()], &mut 20_000)
        .unwrap();
    assert_eq!(
        result.materialize().unwrap(),
        Term::Record((0..10).map(|i| (format!("f{i}"), payload())).collect())
    );
}

#[test]
fn contract_budget_failure_precedes_callbacks_but_semantic_checks_still_run() {
    let mut pool = TypeInterner::default();
    let core = pool
        .intern(
            vec![
                type_parameter(),
                Parameter {
                    sort: Sort::Runtime,
                    ty: hole(),
                },
            ],
            boolean(),
        )
        .unwrap();
    let reference = TypeRef::bind(
        core,
        vec![
            payload(),
            Term::Runtime(nir::RuntimeId {
                instance: 1,
                local: 1,
            }),
        ],
    )
    .unwrap();
    let mut seen = 0;
    let result = reference.check_with_budget(&mut 100, |_, _, _| {
        seen += 1;
        Ok::<_, ()>(())
    });
    assert_eq!(
        result,
        Err(ContractError::Shape(TypeError::ConstructionBudget))
    );
    assert_eq!(seen, 0);
    let result = reference.check_with_budget(&mut 2000, |index, _, ty| {
        seen += 1;
        if index == 1 {
            assert_eq!(ty, &payload());
            Err("contract")
        } else {
            Ok(())
        }
    });
    assert_eq!(result, Err(ContractError::Contract("contract")));
    assert_eq!(seen, 2);
}
