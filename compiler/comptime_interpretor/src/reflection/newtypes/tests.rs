use super::*;
use nir::{Sort, program::TypePredicate};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 92, local }
}
fn nominal(local: u32, arguments: Vec<Term>) -> Term {
    Term::Nominal {
        declaration: id(local),
        arguments,
    }
}
fn definition() -> nir::newtypes::Newtype {
    nir::newtypes::Newtype {
        declaration: id(1),
        name: "Wrapper".into(),
        type_parameters: vec![id(2)],
        parameter_types: vec![],
        capture_count: 0,
        requirements: vec![],
        capabilities: vec![],
        representation: Term::Rigid {
            declaration: id(2),
            sort: Sort::Type,
        },
    }
}
fn package() -> Package {
    Package {
        newtypes: vec![definition()],
        ..Default::default()
    }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}

#[test]
fn reflection_substitutes_one_edge_and_preserves_pointer_type_metadata() {
    let package = package();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for representation in [
        boolean(),
        Term::Pointer(Box::new(boolean())),
        nominal(1, vec![boolean()]),
    ] {
        let subject = nominal(1, vec![representation.clone()]);
        let view = vm.reflect_newtype(&subject).unwrap();
        assert_eq!(view.subject, subject);
        assert_eq!(view.representation, representation);
        assert_eq!(
            view.to_string(),
            format!("(newtype-reflection (subject {subject}) (representation {representation}))")
        );
    }
    for subject in [
        boolean(),
        nominal(1, vec![]),
        nominal(1, vec![Term::Bool(true)]),
        nominal(99, vec![]),
    ] {
        assert!(vm.reflect_newtype(&subject).is_err());
    }
    assert_eq!(
        vm.reflect_newtype(&nominal(1, vec![boolean()]))
            .unwrap()
            .representation,
        boolean()
    );
}

#[test]
fn reflection_checks_formation_and_all_insufficient_budgets_then_recovers() {
    let mut package = package();
    let mut bad = definition();
    bad.declaration = id(3);
    bad.requirements = vec![TypePredicate::Bool(false)];
    package.newtypes.push(bad);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert!(vm.reflect_newtype(&nominal(3, vec![boolean()])).is_err());
    let subject = nominal(1, vec![boolean()]);
    let before = vm.remaining_steps();
    vm.reflect_newtype(&subject).unwrap();
    let cost = before - vm.remaining_steps();
    for steps in 0..cost {
        vm.budget.steps = steps;
        assert_eq!(
            vm.reflect_newtype(&subject).unwrap_err().kind,
            ErrorKind::StepsExhausted,
            "{steps}"
        );
        vm.budget.steps = 100_000;
        assert_eq!(
            vm.reflect_newtype(&subject).unwrap().representation,
            boolean()
        );
    }
}

#[test]
fn concrete_reflection_selects_the_complete_instance_key() {
    let graph = iir::Package {
        nominals: [boolean(), Term::Pointer(Box::new(boolean()))]
            .into_iter()
            .map(|representation| {
                let mut definition = definition();
                definition.type_parameters.clear();
                definition.representation = representation.clone();
                iir::NominalInstance {
                    ty: nominal(1, vec![representation]),
                    definition: iir::NominalDefinition::Newtype(definition),
                }
            })
            .collect(),
        ..Default::default()
    };
    graph.validate(&mut 100_000).unwrap();
    let mut vm = Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap();
    for instance in &graph.nominals {
        let iir::NominalDefinition::Newtype(definition) = &instance.definition else {
            panic!()
        };
        assert_eq!(
            vm.reflect_newtype(&instance.ty).unwrap().representation,
            definition.representation
        );
        let source = SourceRange { start: 1, end: 2 };
        let type_ty = Term::Primitive(Primitive::Type);
        let query = Expr {
            source,
            ty: type_ty.clone(),
            kind: K::NewtypeRepresentation(Box::new(Expr {
                source,
                ty: type_ty,
                kind: K::TypeValue(instance.ty.clone()),
            })),
        };
        let code = Arc::new(comptime_compiler::compile(&query, &mut 100_000).unwrap());
        for compiled in [false, true] {
            let flow = if compiled {
                vm.execute_code(&code, &mut Environment::new())
            } else {
                vm.eval(&query, &mut Environment::new())
            }
            .unwrap();
            let Flow::Value(value) = flow else { panic!() };
            assert_eq!(value, Value::Type(definition.representation.clone()));
        }
    }
}

#[test]
fn observation_agrees_across_carriers_and_rejects_runtime_before_operand() {
    let package = package();
    let source = SourceRange { start: 1, end: 3 };
    let type_ty = Term::Primitive(Primitive::Type);
    let operand = Expr {
        source,
        ty: type_ty.clone(),
        kind: K::TypeValue(nominal(1, vec![boolean()])),
    };
    let query = Expr {
        source,
        ty: type_ty,
        kind: K::NewtypeRepresentation(Box::new(operand)),
    };
    for compiled in [false, true] {
        let execute = |vm: &mut Interpreter<'_>, expr: &Expr| {
            if compiled {
                let code = Arc::new(comptime_compiler::compile(expr, &mut 10000).unwrap());
                vm.execute_code(&code, &mut Environment::new())
            } else {
                vm.eval(expr, &mut Environment::new())
            }
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let Flow::Value(value) = execute(&mut vm, &query).unwrap() else {
            panic!()
        };
        assert_eq!(value, Value::Type(boolean()));
        let mut malformed = query.clone();
        malformed.ty = boolean();
        assert_eq!(
            execute(&mut vm, &malformed).unwrap_err().kind,
            ErrorKind::Type
        );
        // An unbound local would fail if the operand were executed first.
        let mut guarded = query.clone();
        let K::NewtypeRepresentation(operand) = &mut guarded.kind else {
            panic!()
        };
        operand.kind = K::Local(LocalId {
            owner: 55,
            index: 0,
        });
        let mut runtime = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            execute(&mut runtime, &guarded).unwrap_err().kind,
            ErrorKind::PhaseMismatch
        );
    }
}
