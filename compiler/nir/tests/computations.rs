use nir::effects::Operation;
use nir::{DeclarationId, Parameter, Primitive as P, RuntimeId, Sort, Term, TypeInterner};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn ty(p: P) -> Term {
    Term::Primitive(p)
}
fn op(local: u32, arguments: Vec<Term>) -> Operation {
    Operation {
        declaration: id(local),
        arguments,
    }
}

#[test]
fn computation_prefixes_join_without_erasing_occurrences_or_reordering() {
    let a = op(1, vec![]);
    let b = op(2, vec![]);
    assert_eq!(Term::computation(vec![], ty(P::Bool)), ty(P::Bool));
    let nested = Term::computation(
        vec![a.clone()],
        Term::computation(vec![b.clone(), a.clone()], ty(P::Bool)),
    );
    assert_eq!(
        nested,
        Term::Computation {
            effects: vec![a.clone(), b.clone(), a.clone()],
            result: Box::new(ty(P::Bool))
        }
    );
    assert_ne!(nested, Term::computation(vec![a, b], ty(P::Bool)));
    assert_eq!(nested.sort(), Sort::Type);
    nested.check_scope(&[]).unwrap();
}

#[test]
fn operation_arguments_and_result_substitute_together_without_unfolding_heads() {
    let variable = Term::Rigid {
        declaration: id(9),
        sort: Sort::Type,
    };
    let head = Term::NormalForm {
        head: id(10),
        sort: Sort::Type,
        arguments: vec![variable.clone()],
    };
    let template = Term::computation(vec![op(1, vec![head.clone(), variable.clone()])], variable);
    assert!(template.mentions_rigid(id(9)));
    let result = template
        .instantiate_types(&[id(9)], &[ty(P::Bool)])
        .unwrap();
    assert!(!result.mentions_rigid(id(9)));
    let Term::Computation { effects, result } = result else {
        panic!()
    };
    assert_eq!(*result, ty(P::Bool));
    assert_eq!(
        effects[0].arguments[0],
        Term::NormalForm {
            head: id(10),
            sort: Sort::Type,
            arguments: vec![ty(P::Bool)]
        }
    );
    assert_eq!(effects[0].arguments[1], ty(P::Bool));
}

#[test]
fn computation_labels_preserve_binder_scope_and_reject_non_type_results() {
    let bound = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Runtime,
    };
    let computation = Term::computation(vec![op(1, vec![bound.clone()])], ty(P::Bool));
    assert!(computation.check_scope(&[]).is_err());
    let callable = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: ty(P::Bool),
        }],
        result: Box::new(computation),
    };
    callable.check_scope(&[]).unwrap();
    assert_eq!(
        callable
            .instantiate_function(
                &[Term::Runtime(RuntimeId {
                    instance: 8,
                    local: 0
                })],
                |_, _, _| Ok::<_, ()>(())
            )
            .unwrap(),
        Term::computation(
            vec![op(
                1,
                vec![Term::Runtime(RuntimeId {
                    instance: 8,
                    local: 0
                })]
            )],
            ty(P::Bool)
        )
    );
    assert!(
        Term::computation(vec![op(1, vec![])], Term::Bool(true))
            .check_scope(&[])
            .is_err()
    );
}

#[test]
fn computation_dump_keeps_full_operation_inputs_and_nested_result() {
    let computation = Term::computation(
        vec![op(3, vec![Term::String("x\n\"".into())])],
        Term::Tuple(vec![ty(P::Bool)]),
    );
    assert_eq!(
        computation.to_string(),
        "(computation-type (effects (operation (declaration (declaration (package 1) (local 3))) (arguments (string \"x\\n\\\"\")))) (result (tuple ((primitive bool)))))"
    );
}

#[test]
fn effect_inputs_keep_runtime_dependencies_when_cores_are_shared() {
    let mut interner = TypeInterner::default();
    let first = RuntimeId {
        instance: 1,
        local: 0,
    };
    let second = RuntimeId {
        instance: 2,
        local: 0,
    };
    let make = |runtime| Term::computation(vec![op(1, vec![Term::Runtime(runtime)])], ty(P::Bool));
    let a = interner
        .abstract_runtime(&make(first), &[(first, ty(P::Bool))])
        .unwrap();
    let b = interner
        .abstract_runtime(&make(second), &[(second, ty(P::Bool))])
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(a.core(), b.core()));
    assert_ne!(a.materialize().unwrap(), b.materialize().unwrap());
    assert_eq!(a.runtime_dependencies().unwrap(), [first].into());
    assert_eq!(b.runtime_dependencies().unwrap(), [second].into());
    assert!(a.check_runtime_bindings(&[second].into()).is_err());
    a.check_runtime_bindings(&[first].into()).unwrap();
}
