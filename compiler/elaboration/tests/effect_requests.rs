use elaboration::calls::Argument;
use elaboration::capabilities::World;
use elaboration::effects::{Error, instantiate};
use elaboration::implementation::Index;
use elaboration::ptc::{Budget, Context, Formula};
use nir::effects::{Abi, Declaration, Operation};
use nir::{DeclarationId, Parameter, Primitive as P, RuntimeId, Sort, Term};
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn ty(p: P) -> Term {
    Term::Primitive(p)
}
fn value(local: u32) -> Term {
    Term::Runtime(RuntimeId { instance: 7, local })
}
fn declaration(parameters: Vec<Term>, result: Term) -> Declaration {
    Declaration {
        declaration: id(1),
        group: None,
        name: "read".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        signature: Term::Function {
            parameters: parameters
                .into_iter()
                .map(|ty| Parameter {
                    sort: Sort::Runtime,
                    ty,
                })
                .collect(),
            result: Box::new(result),
        },
        abi: Abi::Direct,
        inline: false,
        requirements: vec![],
        capabilities: vec![],
    }
}
fn world<'a>(index: &'a Index, context: &'a Context) -> World<'a> {
    World {
        index,
        context,
        implementations: &[],
        traits: &[],
        assumptions: &[],
    }
}

#[test]
fn requests_check_arguments_and_select_a_distinct_allowed_layer() {
    let index = Index::default();
    let context = Context::default();
    let world = world(&index, &context);
    let declaration = declaration(vec![ty(P::Bool)], ty(P::Char));
    let instance = instantiate(&declaration, &[], &world, Budget::default()).unwrap();
    let allowed = [
        instance.operation().clone(),
        Operation {
            declaration: id(2),
            arguments: vec![],
        },
        instance.operation().clone(),
    ];
    let arguments = [Argument {
        value: value(0),
        ty: ty(P::Bool),
    }];
    for _ in 0..3 {
        let request = instance
            .request(&arguments, &allowed, 1, Budget::default())
            .unwrap();
        assert_eq!(request.operation(), instance.operation());
        assert_eq!(request.allowed_index(), 2);
        assert_eq!(request.abi(), Abi::Direct);
        assert_eq!(request.result(), &ty(P::Char));
        assert_eq!(request.arguments()[0].value, value(0));
        assert!(
            request
                .to_string()
                .contains("(allowed-index 2) (arguments (argument (value (runtime")
        );
    }
    assert!(matches!(
        instance.request(&[], &allowed, 0, Budget::default()),
        Err(Error::ArgumentCount {
            expected: 1,
            actual: 0
        })
    ));
    let wrong = [Argument {
        value: value(0),
        ty: ty(P::Char),
    }];
    assert!(matches!(
        instance.request(&wrong, &allowed, 0, Budget::default()),
        Err(Error::ArgumentType { index: 0 })
    ));
    assert!(matches!(
        instance.request(&arguments, &[], 0, Budget::default()),
        Err(Error::MissingEntry)
    ));
    assert!(matches!(
        instance.request(&arguments, &allowed, 2, Budget::default()),
        Err(Error::MissingEntry)
    ));
}

#[test]
fn dependent_request_inputs_substitute_left_to_right_and_preserve_runtime_identity() {
    let index = Index::default();
    let context = Context::default();
    let world = world(&index, &context);
    let dependent = |value| Term::NormalForm {
        head: id(4),
        arguments: vec![value],
        sort: Sort::Type,
    };
    let bound = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Runtime,
    };
    let declaration = declaration(
        vec![ty(P::Bool), dependent(bound.clone())],
        dependent(bound),
    );
    let instance = instantiate(&declaration, &[], &world, Budget::default()).unwrap();
    let allowed = [instance.operation().clone()];
    for input in [0, 9] {
        let arguments = [
            Argument {
                value: value(input),
                ty: ty(P::Bool),
            },
            Argument {
                value: value(3),
                ty: dependent(value(input)),
            },
        ];
        let request = instance
            .request(&arguments, &allowed, 0, Budget::default())
            .unwrap();
        assert_eq!(request.result(), &dependent(value(input)));
        let reference = nir::TypeInterner::default()
            .abstract_runtime(
                request.result(),
                &[(
                    RuntimeId {
                        instance: 7,
                        local: input,
                    },
                    ty(P::Bool),
                )],
            )
            .unwrap();
        assert_eq!(
            reference.runtime_dependencies().unwrap(),
            [RuntimeId {
                instance: 7,
                local: input
            }]
            .into_iter()
            .collect()
        );
    }
    let stale = [
        Argument {
            value: value(9),
            ty: ty(P::Bool),
        },
        Argument {
            value: value(3),
            ty: dependent(value(0)),
        },
    ];
    assert!(matches!(
        instance.request(&stale, &allowed, 0, Budget::default()),
        Err(Error::ArgumentType { index: 1 })
    ));
}

#[test]
fn request_typing_uses_the_instance_world_without_publishing_assumptions() {
    let index = Index::default();
    let variable = Term::Rigid {
        declaration: id(7),
        sort: Sort::Type,
    };
    let mut context = Context::default();
    context.assume(Formula::equal(variable.clone(), ty(P::Bool)));
    let before = context.clone();
    let world = world(&index, &context);
    let declaration = declaration(vec![ty(P::Bool)], ty(P::Bool));
    let instance = instantiate(&declaration, &[], &world, Budget::default()).unwrap();
    let allowed = [instance.operation().clone()];
    let arguments = [Argument {
        value: value(0),
        ty: variable,
    }];
    assert!(
        instance
            .request(&arguments, &allowed, 0, Budget::default())
            .is_ok()
    );
    assert_eq!(context, before);
    let empty = Context::default();
    let empty_world = World {
        index: &index,
        context: &empty,
        implementations: &[],
        traits: &[],
        assumptions: &[],
    };
    let other = instantiate(&declaration, &[], &empty_world, Budget::default()).unwrap();
    assert!(matches!(
        other.request(&arguments, &allowed, 0, Budget::default()),
        Err(Error::ArgumentType { index: 0 })
    ));
}

#[test]
fn request_plans_preserve_abi_and_recover_from_invalid_input_or_budget_failure() {
    let index = Index::default();
    let context = Context::default();
    let world = world(&index, &context);
    for abi in [Abi::Direct, Abi::DirectPassing, Abi::Capturing, Abi::Async] {
        let mut declaration = declaration(vec![], ty(P::Bool));
        declaration.abi = abi;
        let instance = instantiate(&declaration, &[], &world, Budget::default()).unwrap();
        let allowed = [instance.operation().clone()];
        assert!(matches!(
            instance.request(
                &[],
                &allowed,
                0,
                Budget {
                    steps: 0,
                    max_depth: 64
                }
            ),
            Err(Error::BudgetExhausted)
        ));
        let request = instance
            .request(&[], &allowed, 0, Budget::default())
            .unwrap();
        assert_eq!(request.abi(), abi);
    }
    let declaration = declaration(vec![ty(P::Bool)], ty(P::Bool));
    let instance = instantiate(&declaration, &[], &world, Budget::default()).unwrap();
    let allowed = [instance.operation().clone()];
    let invalid = [Argument {
        value: ty(P::Bool),
        ty: ty(P::Bool),
    }];
    assert!(matches!(
        instance.request(&invalid, &allowed, 0, Budget::default()),
        Err(Error::InvalidInput)
    ));
    let invalid = [Argument {
        value: Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Runtime,
        },
        ty: ty(P::Bool),
    }];
    assert!(matches!(
        instance.request(&invalid, &allowed, 0, Budget::default()),
        Err(Error::InvalidInput)
    ));
    let arguments = [Argument {
        value: value(0),
        ty: ty(P::Bool),
    }];
    assert!(
        instance
            .request(&arguments, &allowed, 0, Budget::default())
            .is_ok()
    );
}

#[test]
fn request_substitution_expansion_is_bounded_before_copying_result_dependencies() {
    let index = Index::default();
    let context = Context::default();
    let world = world(&index, &context);
    let dependent = Term::NormalForm {
        head: id(9),
        sort: Sort::Type,
        arguments: vec![Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Runtime,
        }],
    };
    let declaration = declaration(vec![ty(P::Bool)], Term::Tuple(vec![dependent; 64]));
    let instance = instantiate(&declaration, &[], &world, Budget::default()).unwrap();
    let allowed = [instance.operation().clone()];
    let arguments = [Argument {
        value: Term::NormalForm {
            head: id(8),
            sort: Sort::Runtime,
            arguments: vec![Term::String("x".repeat(4000).into())],
        },
        ty: ty(P::Bool),
    }];
    assert!(matches!(
        instance.request(&arguments, &allowed, 0, Budget::default()),
        Err(Error::BudgetExhausted)
    ));
    let small = [Argument {
        value: value(1),
        ty: ty(P::Bool),
    }];
    let request = instance
        .request(&small, &allowed, 0, Budget::default())
        .unwrap();
    let Term::Tuple(fields) = request.result() else {
        panic!()
    };
    assert_eq!(fields.len(), 64);
    assert!(
        fields.iter().all(
            |field| matches!(field,Term::NormalForm {arguments,..} if arguments == &[value(1)])
        )
    );
}
