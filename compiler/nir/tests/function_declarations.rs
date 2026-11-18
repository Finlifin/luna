//! A callable declaration can be observed and substituted without any body.
use nir::program::{
    EntrySnapshot, Expr, ExprKind, FunctionDeclaration, FunctionPhase, LocalId, SourceRange,
    TraitRequirement, TypePredicate,
};
use nir::{DeclarationId, DeclarationParameter, Parameter, Primitive as P, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 19, local }
}
fn binding(index: u32) -> LocalId {
    LocalId { owner: 1, index }
}
fn expression(ty: Term, kind: ExprKind) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 10, end: 20 },
    }
}
fn generic() -> Term {
    Term::Rigid {
        declaration: id(2),
        sort: Sort::Type,
    }
}
fn declaration() -> FunctionDeclaration {
    let type_value = expression(Term::Primitive(P::Type), ExprKind::TypeValue(generic()));
    let condition = expression(
        Term::Primitive(P::Bool),
        ExprKind::TypeEqual(Box::new(type_value.clone()), Box::new(type_value)),
    );
    FunctionDeclaration {
        source: nir::program::SourceRange { start: 0, end: 0 },
        declaration: id(1),
        name: "quoted\"name\nnext".into(),
        phase: FunctionPhase::Comptime,
        inline: true,
        pure: true,
        is_unsafe: true,
        preconditions: vec![condition.clone()],
        postconditions: vec![condition],
        result_binding: Some(binding(1)),
        entry_snapshots: vec![EntrySnapshot {
            parameter: binding(0),
            binding: binding(2),
        }],
        capabilities: vec![TraitRequirement {
            interface: id(3),
            interface_arguments: vec![generic()],
            subject: generic(),
        }],
        requirements: vec![TypePredicate::Equal(generic(), generic())],
        type_parameters: vec![id(2)],
        signature: Term::Function {
            parameters: vec![Parameter {
                sort: Sort::ComptimeValue,
                ty: generic(),
            }],
            result: Box::new(generic()),
        },
        parameters: vec![binding(0)],
        defaults: vec![(0, expression(generic(), ExprKind::Local(binding(3))))],
    }
}

#[test]
fn body_free_substitution_keeps_contracts_defaults_bindings_and_responsibility() {
    let original = declaration();
    let binders = [DeclarationParameter {
        declaration: id(2),
        sort: Sort::Type,
    }];
    assert!(original.instantiate_parameters(&binders, &[]).is_err());
    let concrete = Term::Primitive(P::Bool);
    let instantiated = original
        .instantiate_parameters(&binders, &[concrete.clone()])
        .unwrap();
    let callable = instantiated.callable_type();
    assert!(callable.is_comptime_callable() && callable.is_unsafe_callable());
    assert_eq!(
        callable.callable_signature(),
        &Term::Function {
            parameters: vec![Parameter {
                sort: Sort::ComptimeValue,
                ty: concrete.clone()
            }],
            result: Box::new(concrete.clone()),
        }
    );
    for condition in instantiated
        .preconditions
        .iter()
        .chain(&instantiated.postconditions)
    {
        let ExprKind::TypeEqual(left, right) = &condition.kind else {
            panic!("missing contract")
        };
        for operand in [left, right] {
            assert_eq!(operand.kind, ExprKind::TypeValue(concrete.clone()));
        }
    }
    assert_eq!(instantiated.defaults[0].1.ty, concrete);
    assert_eq!(instantiated.defaults[0].1.kind, ExprKind::Local(binding(3)));
    assert_eq!(
        instantiated.requirements,
        vec![TypePredicate::Equal(concrete.clone(), concrete.clone())]
    );
    assert_eq!(instantiated.capabilities[0].subject, concrete);
    assert_eq!(
        instantiated.capabilities[0].interface_arguments,
        vec![concrete]
    );
    assert_eq!(instantiated.parameters, original.parameters);
    assert_eq!(instantiated.entry_snapshots, original.entry_snapshots);
    assert_eq!(instantiated.result_binding, original.result_binding);
    assert_eq!(instantiated.type_parameters, original.type_parameters);
    assert_eq!(
        original,
        declaration(),
        "failed or successful substitution mutated its source"
    );
}

#[test]
fn declaration_display_preserves_contracts_and_escapes_names_without_a_body() {
    let header = declaration();
    let dump = header.to_string();
    assert!(dump.starts_with("(function-declaration "), "{dump}");
    assert!(dump.contains("(name \"quoted\\\"name\\nnext\")"), "{dump}");
    for field in [
        "(program-requires ",
        "(program-ensures ",
        "(defaults ",
        "(entry-snapshots ",
        "(capabilities ",
    ] {
        assert!(dump.contains(field), "{field}: {dump}");
    }
    assert!(!dump.contains("(body "), "{dump}");
    assert_eq!(dump, header.to_string());
}
