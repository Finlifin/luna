use super::*;
use nir::program::{
    AssociatedContract, Function, FunctionPhase, Implementation, MethodContract, Trait,
};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 97, local }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn signature() -> Term {
    Term::Function {
        parameters: vec![],
        result: Box::new(boolean()),
    }
}
fn interface(base: u32) -> Trait {
    Trait {
        declaration: id(base),
        self_parameter: id(base + 1),
        name: format!("Protocol{base}"),
        comptime_only: false,
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
        parents: vec![],
        associated: vec![AssociatedContract {
            declaration: id(base + 3),
            name: "flag".into(),
            phase: Default::default(),
            ty: boolean(),
            default: None,
        }],
        methods: vec![MethodContract {
            declaration: id(base + 2),
            name: "read".into(),
            phase: Default::default(),
            is_quote: false,
            pure: false,
            is_unsafe: false,
            type_arguments: vec![],
            type_parameters: vec![],
            capabilities: vec![],
            program: Default::default(),
            parameter_bindings: vec![],
            by_value_receiver: false,
            by_reference_receiver: false,
            signature: signature().into(),
            requirements: vec![],
            default: None,
        }],
    }
}
fn source() -> Arc<Package> {
    Arc::new(Package {
        traits: (0..64).map(|index| interface(100 + index * 10)).collect(),
        implementations: vec![Implementation {
            declaration: id(1),
            phases: Default::default(),
            type_parameters: vec![],
            parameter_types: vec![],
            associated: vec![],
            associated_initializers: vec![],
            interface: None,
            interface_arguments: vec![],
            capabilities: vec![],
            requirements: vec![],
            subject: boolean(),
            methods: vec![],
        }],
        function_parameters: vec![nir::function_parameters::Declaration {
            declaration: id(2),
            parameters: vec![],
            domains: vec![],
        }],
        quote_templates: vec![Function {
            declaration: id(3),
            name: "quoted".into(),
            phase: FunctionPhase::Runtime,
            inline: false,
            pure: false,
            is_unsafe: false,
            type_parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            parameters: vec![],
            defaults: vec![],
            signature: signature(),
            body: Expr {
                source: SourceRange { start: 0, end: 1 },
                ty: boolean(),
                kind: K::Literal(nir::program::Literal::Bool(true)),
            },
        }],
        ..Default::default()
    })
}
fn structure(identity: DeclarationId) -> nir::program::Struct {
    nir::program::Struct {
        declaration: identity,
        name: "Generated".into(),
        type_parameters: vec![],
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        fields: vec![],
    }
}

#[test]
fn failed_identity_index_never_caches_absence_and_warm_queries_do_not_scan() {
    let source = source();
    let mut vm = Interpreter::for_comptime(&source, Budget::default()).unwrap();
    for _ in 0..2 {
        vm.budget.steps = 8;
        assert_eq!(
            vm.known_definition(id(9000)).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        assert!(
            vm.reserved_source_declarations
                .as_ref()
                .unwrap()
                .identities
                .get()
                .is_none()
        );
    }
    vm.budget = Budget::default();
    assert!(!vm.known_definition(id(9000)).unwrap());
    for identity in [1, 2, 3, 100, 101, 102, 103, 730, 731, 732, 733, 9000] {
        vm.budget.steps = 1;
        assert_eq!(vm.known_definition(id(identity)).unwrap(), identity != 9000);
        assert_eq!(vm.budget.steps, 0);
    }
    // Every metadata category still blocks publication, not just lookup.
    for identity in [1, 2, 3, 100, 101, 102, 103] {
        vm.budget = Budget::default();
        let result = vm
            .install_owned_generated_structs_with(Arc::from([structure(id(identity))]), || {
                Ok::<_, ()>(())
            });
        assert!(matches!(
            result,
            Err(StructPublicationError::Admission(Error {
                kind: ErrorKind::InvalidCallPlan,
                ..
            }))
        ));
    }
}

#[test]
fn identity_index_is_snapshot_scoped_and_never_shares_generated_entries() {
    let source = source();
    let mut session = CodeSession::new(source.clone());
    let mut budget = Budget::default();
    let mut first =
        Interpreter::for_comptime_in_session(&source, &mut session, &mut budget).unwrap();
    assert!(!first.known_definition(id(9000)).unwrap());
    let reserved = first.reserved_source_declarations.as_ref().unwrap().clone();
    let failed = first
        .install_owned_generated_structs_with(Arc::from([structure(id(9000))]), || {
            Err::<(), _>("publisher")
        });
    assert!(matches!(
        failed,
        Err(StructPublicationError::Commit("publisher"))
    ));
    assert!(!first.known_definition(id(9000)).unwrap());
    first
        .install_owned_generated_structs_with(Arc::from([structure(id(9000))]), || Ok::<_, ()>(()))
        .unwrap();
    assert!(first.known_definition(id(9000)).unwrap());

    let mut budget = Budget::default();
    let mut second =
        Interpreter::for_comptime_in_session(&source, &mut session, &mut budget).unwrap();
    assert!(Arc::ptr_eq(
        &reserved,
        second.reserved_source_declarations.as_ref().unwrap()
    ));
    second.budget.steps = 1;
    assert!(!second.known_definition(id(9000)).unwrap());
    second.budget = Budget::default();
    assert!(
        second.invoke(id(3), vec![]).is_err(),
        "a reserved quote identity is not executable"
    );

    let mut changed = (*source).clone();
    changed.traits.push(interface(9000));
    let mut budget = Budget::default();
    let mut foreign =
        Interpreter::for_comptime_in_session(&changed, &mut session, &mut budget).unwrap();
    assert!(!Arc::ptr_eq(
        &reserved,
        foreign.reserved_source_declarations.as_ref().unwrap()
    ));
    assert!(foreign.known_definition(id(9000)).unwrap());
    assert!(foreign.known_definition(id(9002)).unwrap());
    assert!(!second.known_definition(id(9000)).unwrap());
}
