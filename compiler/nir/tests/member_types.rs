use nir::code_interfaces::MethodSignature;
use nir::program::{MethodContract, Trait};
use nir::type_computations::Error;
use nir::{DeclarationId, Primitive, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}

fn contract() -> Trait {
    Trait {
        declaration: id(1),
        name: "Factory".into(),
        self_parameter: id(2),
        type_parameters: vec![id(3)],
        parameter_types: vec![Term::Primitive(Primitive::Type)],
        comptime_only: false,
        parents: vec![],
        requirements: vec![],
        capabilities: vec![],
        associated: vec![],
        methods: vec![MethodContract {
            declaration: id(4),
            name: "make".into(),
            phase: Default::default(),
            is_quote: false,
            pure: false,
            is_unsafe: false,
            type_arguments: vec![],
            type_parameters: vec![id(5)],
            capabilities: vec![],
            requirements: vec![],
            program: Default::default(),
            parameter_bindings: vec![],
            by_value_receiver: false,
            by_reference_receiver: false,
            default: None,
            signature: signature(Term::NormalForm {
                head: id(9),
                sort: Sort::Type,
                arguments: vec![
                    Term::Rigid {
                        declaration: id(3),
                        sort: Sort::Type,
                    },
                    Term::Rigid {
                        declaration: id(5),
                        sort: Sort::ComptimeValue,
                    },
                ],
            })
            .into(),
        }],
    }
}

fn signature(result: Term) -> Term {
    Term::Function {
        parameters: vec![],
        result: Box::new(result),
    }
}

#[test]
fn public_method_observation_preserves_domains_and_failure_boundaries() {
    let contract = contract();
    let before = contract.to_string();
    let boolean = Term::Primitive(Primitive::Bool);
    let mut fuel = 1000;
    let failure = nir::member_types::method(
        &contract,
        &contract.methods[0],
        &boolean,
        &[boolean.clone()],
        &[Term::Bool(true)],
        |_, _| Err(Error::Structure("domain query unavailable".into())),
        &[],
        &mut fuel,
    )
    .unwrap_err();
    assert_eq!(failure, Error::Structure("domain query unavailable".into()));
    assert!(fuel < 1000);
    let observe = |interfaces: &[Term], methods: &[Term], fuel: &mut usize| {
        nir::member_types::method(
            &contract,
            &contract.methods[0],
            &boolean,
            interfaces,
            methods,
            |parameter, _| Ok((parameter == id(5)).then_some(&boolean)),
            &[],
            fuel,
        )
    };
    assert_eq!(
        observe(&[boolean.clone()], &[Term::Bool(true)], &mut 0),
        Err(Error::Budget)
    );
    // Equal total length cannot move a trait input into the method telescope.
    assert!(observe(&[], &[boolean.clone(), Term::Bool(true)], &mut 1000).is_err());
    assert!(observe(&[boolean.clone()], &[boolean.clone()], &mut 1000).is_err());
    assert_eq!(
        observe(&[boolean.clone()], &[Term::Bool(true)], &mut 1000).unwrap(),
        MethodSignature::Callable(signature(Term::NormalForm {
            head: id(9),
            sort: Sort::Type,
            arguments: vec![boolean, Term::Bool(true)],
        }))
    );
    assert_eq!(contract.to_string(), before);
}
