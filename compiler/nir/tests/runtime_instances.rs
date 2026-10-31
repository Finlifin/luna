use nir::{DeclarationId, Parameter, Primitive, Sort, Term, program::*};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}

fn function() -> Function {
    let ty = Term::Primitive(Primitive::Bool);
    Function {
        declaration: id(1),
        name: "identity".into(),
        phase: FunctionPhase::Runtime,
        inline: false,
        pure: false,
        is_unsafe: false,
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        capabilities: vec![],
        requirements: vec![],
        type_parameters: vec![],
        signature: Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Runtime,
                ty: ty.clone(),
            }],
            result: Box::new(ty.clone()),
        },
        parameters: vec![LocalId { owner: 1, index: 0 }],
        body: Expr {
            ty,
            source: SourceRange { start: 0, end: 1 },
            kind: ExprKind::Local(LocalId { owner: 1, index: 0 }),
        },
        defaults: vec![],
    }
}

#[test]
fn residual_keys_preserve_identity_but_do_not_reapply_substitutions() {
    let mut source = function();
    source.type_parameters.push(id(2));
    let residual = RuntimeInstance {
        declaration: id(1),
        arguments: vec![Term::Bool(true)],
        function: function(),
        cleanup_accesses: vec![],
    };
    let package = Package {
        functions: vec![source],
        runtime_instances: vec![residual],
        ..Default::default()
    };
    let key = [Term::Bool(true)];
    let (body, arguments) = package.runtime_body(id(1), &key).unwrap();
    assert!(std::ptr::eq(body, &package.runtime_instances[0].function));
    assert!(arguments.is_empty());
    let other = [Term::Bool(false)];
    let (body, arguments) = package.runtime_body(id(1), &other).unwrap();
    assert!(std::ptr::eq(body, &package.functions[0]));
    assert_eq!(arguments, &other);
    assert!(package.runtime_body(id(1), &[]).is_none());
}

#[test]
fn malformed_residual_never_falls_back_to_a_valid_source_body() {
    let instance = RuntimeInstance {
        declaration: id(1),
        arguments: vec![],
        function: function(),
        cleanup_accesses: vec![],
    };
    for mutation in 0..7 {
        let mut package = Package {
            functions: vec![function()],
            runtime_instances: vec![instance.clone()],
            ..Default::default()
        };
        let body = &mut package.runtime_instances[0].function;
        match mutation {
            0 => body.declaration = id(9),
            1 => body.type_parameters.push(id(2)),
            2 => body.phase = FunctionPhase::Comptime,
            3 => body.signature = Term::Primitive(Primitive::Bool),
            4 => body.parameters.clear(),
            5 => {
                let Term::Function { parameters, .. } = &mut body.signature else {
                    unreachable!()
                };
                parameters[0].sort = Sort::ComptimeValue;
            }
            _ => package.runtime_instances.push(instance.clone()),
        }
        assert!(
            package.runtime_body(id(1), &[]).is_none(),
            "mutation {mutation}"
        );
        package.runtime_instances.clear();
        assert!(package.runtime_body(id(1), &[]).is_some());
    }
}

#[test]
fn source_fallback_rejects_ambiguous_declarations_and_wrong_argument_counts() {
    let mut package = Package {
        functions: vec![function()],
        ..Default::default()
    };
    assert!(package.runtime_body(id(1), &[Term::Bool(true)]).is_none());
    assert_eq!(
        package
            .runtime_body_checked(id(1), &[Term::Bool(true)])
            .err(),
        Some("function type arguments have incorrect arity")
    );
    assert!(package.runtime_body(id(9), &[]).is_none());
    assert_eq!(
        package.runtime_body_checked(id(9), &[]).err(),
        Some("unknown function")
    );
    package.quote_templates.push(function());
    assert!(package.runtime_body(id(1), &[]).is_none());
    assert_eq!(
        package.runtime_body_checked(id(1), &[]).err(),
        Some("function declaration is ambiguous")
    );
    package.quote_templates.clear();
    package.functions.push(function());
    assert!(package.runtime_body(id(1), &[]).is_none());
    package.functions.pop();
    assert!(package.runtime_body(id(1), &[]).is_some());
}
