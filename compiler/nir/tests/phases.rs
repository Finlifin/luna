use nir::program::{
    CallArgument, Expr, ExprKind as K, Function, FunctionPhase as Phase, Literal, LocalId, Package,
    SourceRange,
};
use nir::{DeclarationId, Parameter, Primitive, Sort, Term};

fn flag(kind: K) -> Expr {
    Expr {
        kind,
        ty: Term::Primitive(Primitive::Bool),
        source: SourceRange { start: 1, end: 2 },
    }
}
fn function(local: u32, phase: Phase, inputs: &[Sort]) -> Function {
    Function {
        declaration: DeclarationId { package: 1, local },
        name: format!("function{local}"),
        phase,
        inline: false,
        pure: false,
        is_unsafe: false,
        signature: Term::Function {
            parameters: inputs
                .iter()
                .map(|sort| Parameter {
                    sort: *sort,
                    ty: Term::Primitive(Primitive::Bool),
                })
                .collect(),
            result: Box::new(Term::Primitive(Primitive::Bool)),
        },
        parameters: inputs
            .iter()
            .enumerate()
            .map(|(index, _)| LocalId {
                owner: local,
                index: index as u32,
            })
            .collect(),
        body: flag(K::Literal(Literal::Bool(true))),
        type_parameters: vec![],
        defaults: vec![],
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        capabilities: vec![],
        requirements: vec![],
    }
}

#[test]
fn normal_form_values_require_matching_metadata_and_comptime_storage() {
    let head = nir::normal_forms::Head {
        collector: None,
        role: nir::normal_forms::Role::Opaque,
        declaration: DeclarationId {
            package: 1,
            local: 50,
        },
        parameters: vec![],
        requirements: vec![],
    };
    let ty = head.callable_type().unwrap();
    let mut f = function(1, Phase::Comptime, &[]);
    f.signature = Term::Function {
        parameters: vec![],
        result: Box::new(ty.clone()),
    };
    f.body.ty = ty;
    f.body.kind = K::Function(head.declaration);
    let package = Package {
        functions: vec![f],
        normal_forms: vec![head],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    let mut bad = package.clone();
    bad.normal_forms.clear();
    assert!(nir::phases::validate(&bad).is_err());
    let mut bad = package.clone();
    bad.functions[0].phase = Phase::Runtime;
    assert!(nir::phases::validate(&bad).is_err());
    let mut bad = package.clone();
    bad.normal_forms[0]
        .parameters
        .push(nir::normal_forms::Parameter {
            declaration: DeclarationId {
                package: 1,
                local: 51,
            },
            binding: LocalId {
                owner: 50,
                index: 0,
            },
            kind: nir::normal_forms::Kind::Type,
            ty: Term::Primitive(Primitive::Type),
        });
    assert!(nir::phases::validate(&bad).is_err());
    nir::phases::validate(&package).unwrap();
}

#[test]
fn comptime_integer_conversion_checks_phase_and_both_domains() {
    let mut f = function(1, Phase::Comptime, &[]);
    f.signature = Term::Function {
        parameters: vec![],
        result: Box::new(Term::Primitive(Primitive::U8)),
    };
    f.body = Expr {
        source: f.body.source,
        ty: Term::Primitive(Primitive::U8),
        kind: K::ComptimeIntegerConvert(Box::new(Expr {
            source: f.body.source,
            ty: Term::Primitive(Primitive::Integer),
            kind: K::Literal(Literal::Integer("255".into())),
        })),
    };
    let package = Package {
        functions: vec![f],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    let mut runtime = package.clone();
    runtime.functions[0].phase = Phase::Runtime;
    let K::ComptimeIntegerConvert(value) = &mut runtime.functions[0].body.kind else {
        unreachable!()
    };
    value.ty = Term::Primitive(Primitive::U8);
    value.kind = K::Literal(Literal::UnsignedInteger(255));
    let failure = nir::phases::validate(&runtime).unwrap_err();
    assert!(
        failure
            .message
            .contains("requires comptime integer domains"),
        "{failure:?}"
    );
    let mut bad = package.clone();
    let K::ComptimeIntegerConvert(value) = &mut bad.functions[0].body.kind else {
        unreachable!()
    };
    *value = Box::new(flag(K::Literal(Literal::Bool(true))));
    assert!(nir::phases::validate(&bad).is_err());
    let mut bad = package;
    bad.functions[0].body.ty = Term::Primitive(Primitive::Bool);
    bad.functions[0].signature = Term::Function {
        parameters: vec![],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    assert!(nir::phases::validate(&bad).is_err());
}

#[test]
fn text_queries_validate_packet_domains_in_both_phases() {
    use nir::program::TextQueryKind as Q;
    for phase in [Phase::Runtime, Phase::Comptime] {
        for query in [Q::ByteLength, Q::Byte] {
            let mut f = function(1, phase, &[]);
            let mut values = vec![Expr {
                source: f.body.source,
                ty: Term::Primitive(Primitive::Str),
                kind: K::Literal(Literal::String("🦀".into())),
            }];
            if query == Q::Byte {
                values.push(Expr {
                    source: f.body.source,
                    ty: Term::Primitive(Primitive::U64),
                    kind: K::Literal(Literal::UnsignedInteger(0)),
                });
            }
            let result = if query == Q::ByteLength {
                Term::Primitive(Primitive::U64)
            } else {
                Term::Optional(Box::new(Term::Primitive(Primitive::U8)))
            };
            f.signature = Term::Function {
                parameters: vec![],
                result: Box::new(result.clone()),
            };
            f.body = Expr {
                source: f.body.source,
                ty: result,
                kind: K::TextQuery {
                    query,
                    subject: Box::new(Expr {
                        source: f.body.source,
                        ty: Term::Tuple(values.iter().map(|v| v.ty.clone()).collect()),
                        kind: K::Tuple(values),
                    }),
                },
            };
            let package = Package {
                functions: vec![f],
                ..Default::default()
            };
            nir::phases::validate(&package).unwrap();
            for damage in 0..3 {
                let mut invalid = package.clone();
                let expr = &mut invalid.functions[0].body;
                let K::TextQuery { subject, .. } = &mut expr.kind else {
                    unreachable!()
                };
                match damage {
                    0 => expr.ty = Term::Primitive(Primitive::Bool),
                    1 => subject.ty = Term::Tuple(vec![]),
                    _ => {
                        subject.ty = Term::Tuple(vec![
                            Term::Primitive(Primitive::Bool);
                            if query == Q::Byte { 2 } else { 1 }
                        ])
                    }
                }
                assert!(nir::phases::validate(&invalid).is_err());
            }
        }
    }
}
fn call(target: &Function, arguments: Vec<(usize, Expr)>) -> Expr {
    flag(K::Call {
        callee: Box::new(Expr {
            kind: K::Function(target.declaration),
            ty: target.callable_type(),
            source: target.body.source,
        }),
        arguments: arguments
            .into_iter()
            .map(|(parameter, value)| CallArgument {
                passing: Default::default(),
                parameter,
                value,
                is_default: false,
            })
            .collect(),
        parameter_bindings: target.parameters.clone(),
    })
}

#[test]
fn generic_integer_operations_defer_only_owned_matching_domains() {
    let parameter = DeclarationId {
        package: 1,
        local: 501,
    };
    let open = Term::Rigid {
        declaration: parameter,
        sort: Sort::Type,
    };
    let make = |operand: Term, result: Term, owned: bool| {
        let mut target = function(500, Phase::Runtime, &[Sort::Runtime]);
        target.type_parameters = if owned { vec![parameter] } else { vec![] };
        target.signature = Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Runtime,
                ty: operand.clone(),
            }],
            result: Box::new(result.clone()),
        };
        target.body = Expr {
            ty: result,
            source: target.body.source,
            kind: K::IntegerNegate(Box::new(Expr {
                ty: operand,
                source: target.body.source,
                kind: K::Local(target.parameters[0]),
            })),
        };
        Package {
            functions: vec![target],
            ..Default::default()
        }
    };
    let package = make(open.clone(), open.clone(), true);
    let obligations = nir::phases::machine_arithmetic_functions(&package).unwrap();
    assert!(obligations.contains(&package.functions[0].declaration));
    let mut diverging = package.clone();
    let function = &mut diverging.functions[0];
    let K::IntegerNegate(value) = function.body.kind.clone() else {
        unreachable!()
    };
    let right = Expr {
        ty: Term::Primitive(Primitive::Never),
        source: function.body.source,
        kind: K::Return(Some(value.clone())),
    };
    function.body.ty = Term::Primitive(Primitive::Never);
    function.body.kind = K::IntegerBinary {
        op: nir::program::IntegerOp::Add,
        left: value.clone(),
        right: Box::new(right),
    };
    assert!(nir::phases::validate(&diverging).is_ok());
    let K::IntegerBinary { right, .. } = &mut diverging.functions[0].body.kind else {
        unreachable!()
    };
    *right = value;
    assert!(
        nir::phases::validate(&diverging).is_err(),
        "Never cannot hide a returning generic operation"
    );
    assert!(
        nir::phases::validate(&make(open.clone(), Term::Primitive(Primitive::Never), true))
            .is_err()
    );
    assert!(nir::phases::validate(&make(open.clone(), open.clone(), false)).is_err());
    assert!(nir::phases::validate(&make(Term::Primitive(Primitive::Bool), open, true)).is_err());
    for primitive in [Primitive::Bool, Primitive::Integer] {
        let ty = Term::Primitive(primitive);
        assert!(nir::phases::validate(&make(ty.clone(), ty, false)).is_err());
    }
    let machine = Term::Primitive(Primitive::I8);
    assert!(nir::phases::validate(&make(machine.clone(), machine, false)).is_ok());
}

#[test]
fn default_expressions_only_read_available_parameter_bindings() {
    for quote in [false, true] {
        let mut target = function(90, Phase::Runtime, &[Sort::Runtime; 3]);
        target.defaults = vec![(1, flag(K::Local(target.parameters[0])))];
        let package = |target: Function| {
            if quote {
                Package {
                    quote_templates: vec![target],
                    ..Default::default()
                }
            } else {
                Package {
                    functions: vec![target],
                    ..Default::default()
                }
            }
        };
        nir::phases::validate(&package(target.clone())).unwrap();
        let temporary = LocalId {
            owner: 90,
            index: 99,
        };
        target.defaults[0].1 = flag(K::Block {
            statements: vec![Expr {
                kind: K::Let {
                    binding: temporary,
                    value: Box::new(flag(K::Local(target.parameters[0]))),
                },
                ty: Term::Primitive(Primitive::Void),
                source: target.body.source,
            }],
            result: Some(Box::new(flag(K::Local(temporary)))),
        });
        nir::phases::validate(&package(target.clone())).unwrap();
        for unavailable in [
            target.parameters[1],
            target.parameters[2],
            LocalId {
                owner: 999,
                index: 0,
            },
        ] {
            let mut invalid = target.clone();
            invalid.defaults[0].1 = flag(K::Local(unavailable));
            let error = nir::phases::validate(&package(invalid)).unwrap_err();
            assert!(error.message.contains("default"), "{error:?}");
            nir::phases::validate(&package(target.clone())).unwrap();
        }
    }
}

#[test]
fn default_closure_captures_require_explicit_available_bindings() {
    use nir::program::Capture;
    let mut target = function(91, Phase::Runtime, &[Sort::Runtime; 2]);
    let captured = target.parameters[0];
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    let ty = Term::Closure {
        environment: vec![Term::Primitive(Primitive::Bool)],
        signature: Box::new(signature),
    };
    let Term::Function { parameters, .. } = &mut target.signature else {
        unreachable!()
    };
    parameters[1].ty = ty.clone();
    target.defaults = vec![(
        1,
        Expr {
            ty,
            source: target.body.source,
            kind: K::Closure {
                captures: vec![Capture {
                    binding: captured,
                    ty: Term::Primitive(Primitive::Bool),
                    value: None,
                }],
                parameters: vec![],
                body: std::sync::Arc::new(flag(K::Local(captured))),
            },
        },
    )];
    let mut package = Package {
        functions: vec![target],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    let original = package.functions[0].defaults[0].1.clone();
    let K::Closure { body, .. } = &mut package.functions[0].defaults[0].1.kind else {
        unreachable!()
    };
    *body = std::sync::Arc::new(flag(K::Local(LocalId {
        owner: 999,
        index: 0,
    })));
    let error = nir::phases::validate(&package).unwrap_err();
    assert_eq!(error.message, "invalid function default binding scope");
    package.functions[0].defaults[0].1 = original;
    nir::phases::validate(&package).unwrap();
}

#[test]
fn call_plans_reject_duplicate_missing_and_misordered_slots() {
    let target = function(92, Phase::Runtime, &[Sort::Runtime; 3]);
    let valid = call(
        &target,
        vec![
            (2, flag(K::Literal(Literal::Bool(true)))),
            (0, flag(K::Literal(Literal::Bool(false)))),
            (1, flag(K::Literal(Literal::Bool(true)))),
        ],
    );
    let mut root = function(93, Phase::Runtime, &[]);
    root.body = valid.clone();
    let mut package = Package {
        functions: vec![target, root],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    for mutation in 0..9 {
        let K::Call {
            arguments,
            parameter_bindings,
            ..
        } = &mut package.functions[1].body.kind
        else {
            unreachable!()
        };
        match mutation {
            0 => arguments[1].parameter = arguments[0].parameter,
            1 => {
                arguments.pop();
            }
            2 => {
                parameter_bindings.pop();
            }
            3 => parameter_bindings[1] = parameter_bindings[0],
            4 => arguments[0].is_default = true,
            5 => {
                arguments[1].is_default = true;
                arguments[2].is_default = true;
                arguments.swap(1, 2);
            }
            6 => {
                arguments[2].is_default = true;
                parameter_bindings.clear();
            }
            7 => parameter_bindings.swap(0, 1),
            8 => {
                arguments[2].is_default = true;
                arguments[2].value = flag(K::Local(parameter_bindings[1]));
            }
            _ => unreachable!(),
        }
        let error = nir::phases::validate(&package).unwrap_err();
        assert!(error.message.contains("call"), "{error:?}");
        package.functions[1].body = valid.clone();
        nir::phases::validate(&package).unwrap();
    }
    let K::Call {
        arguments,
        parameter_bindings,
        ..
    } = &mut package.functions[1].body.kind
    else {
        unreachable!()
    };
    arguments[2].is_default = true;
    arguments[2].value = flag(K::Local(parameter_bindings[0]));
    nir::phases::validate(&package).unwrap();
    package.functions[1].body = valid;
    let K::Call {
        parameter_bindings, ..
    } = &mut package.functions[1].body.kind
    else {
        unreachable!()
    };
    parameter_bindings.clear();
    nir::phases::validate(&package).unwrap();
}

#[test]
fn closure_frames_reject_ambiguous_bindings_and_environment_arity() {
    for phase in [Phase::Runtime, Phase::Comptime] {
        let first = LocalId {
            owner: 95,
            index: 0,
        };
        let second = LocalId {
            owner: 95,
            index: 1,
        };
        let captured = LocalId {
            owner: 95,
            index: 2,
        };
        let signature = function(95, phase, &[Sort::Runtime; 2])
            .signature
            .with_comptime_phase(phase == Phase::Comptime);
        let body = Expr {
            source: SourceRange { start: 10, end: 20 },
            ty: Term::Closure {
                environment: vec![Term::Primitive(Primitive::Bool)],
                signature: Box::new(signature),
            },
            kind: K::Closure {
                captures: vec![nir::program::Capture {
                    binding: captured,
                    ty: Term::Primitive(Primitive::Bool),
                    value: Some(Box::new(flag(K::Literal(Literal::Bool(true))))),
                }],
                parameters: vec![first, second],
                body: std::sync::Arc::new(flag(K::Local(first))),
            },
        };
        let mut root = function(94, phase, &[]);
        let Term::Function { result, .. } = &mut root.signature else {
            unreachable!()
        };
        *result = Box::new(body.ty.clone());
        root.body = body.clone();
        let mut package = Package {
            functions: vec![root],
            ..Default::default()
        };
        nir::phases::validate(&package).unwrap();
        for mutation in 0..7 {
            let expression = &mut package.functions[0].body;
            let K::Closure {
                captures,
                parameters,
                ..
            } = &mut expression.kind
            else {
                unreachable!()
            };
            match mutation {
                0 => {
                    parameters.pop();
                }
                1 => parameters[1] = parameters[0],
                2 => captures[0].binding = parameters[0],
                3 => captures.push(captures[0].clone()),
                4 => expression.ty = Term::Primitive(Primitive::Bool),
                5 => captures.clear(),
                6 => {
                    let Term::Closure { signature, .. } = &expression.ty else {
                        unreachable!()
                    };
                    expression.ty = *signature.clone();
                }
                _ => unreachable!(),
            }
            let error = nir::phases::validate(&package).unwrap_err();
            assert!(error.message.contains("closure"), "{error:?}");
            package.functions[0].body = body.clone();
            nir::phases::validate(&package).unwrap();
        }
    }
}

#[test]
fn staged_argument_phase_follows_parameter_slot_not_source_order() {
    let compute = function(1, Phase::Comptime, &[]);
    let runtime = function(2, Phase::Runtime, &[]);
    let target = function(3, Phase::Runtime, &[Sort::Runtime, Sort::ComptimeValue]);
    let mut root = function(4, Phase::Runtime, &[]);
    root.body = call(
        &target,
        vec![(1, call(&compute, vec![])), (0, call(&runtime, vec![]))],
    );
    let mut package = Package {
        functions: vec![compute.clone(), runtime.clone(), target.clone(), root],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    // Reversing which slots get the calls violates both execution boundaries.
    package.functions[3].body = call(
        &target,
        vec![(1, call(&runtime, vec![])), (0, call(&compute, vec![]))],
    );
    assert!(nir::phases::validate(&package).is_err());
    package.functions[3].body = call(
        &target,
        vec![
            (1, call(&runtime, vec![])),
            (0, flag(K::Literal(Literal::Bool(true)))),
        ],
    );
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("runtime function is unavailable")
    );
    package.functions[3].body = call(
        &target,
        vec![(1, call(&compute, vec![])), (0, call(&compute, vec![]))],
    );
    assert!(nir::phases::validate(&package).is_err());
}

#[test]
fn staged_context_propagates_through_blocks_and_nested_calls() {
    let compute = function(1, Phase::Comptime, &[]);
    let combine = function(2, Phase::Comptime, &[Sort::Runtime]);
    let target = function(3, Phase::Runtime, &[Sort::ComptimeValue]);
    let mut root = function(4, Phase::Runtime, &[]);
    let nested = flag(K::Block {
        statements: vec![],
        result: Some(Box::new(call(&combine, vec![(0, call(&compute, vec![]))]))),
    });
    root.body = call(&target, vec![(0, nested)]);
    let mut package = Package {
        functions: vec![compute, combine, target.clone(), root],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    package.functions[3].body = call(&target, vec![(7, flag(K::Literal(Literal::Bool(true))))]);
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("index out of range")
    );
}

#[test]
fn defaults_are_checked_in_their_parameter_phase_without_a_call_site() {
    let compute = function(1, Phase::Comptime, &[]);
    let runtime = function(2, Phase::Runtime, &[]);
    let mut target = function(3, Phase::Runtime, &[Sort::ComptimeValue, Sort::Runtime]);
    target.defaults = vec![(0, call(&compute, vec![])), (1, call(&runtime, vec![]))];
    let mut package = Package {
        functions: vec![compute.clone(), runtime.clone(), target],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    package.functions[2].defaults[0].1 = call(&runtime, vec![]);
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("runtime function is unavailable")
    );
    package.functions[2].defaults[0].1 = call(&compute, vec![]);
    package.functions[2].defaults[1].1 = call(&compute, vec![]);
    assert!(nir::phases::validate(&package).is_err());
}

#[test]
fn nominal_callback_storage_preserves_ordinary_input_frames() {
    let nominal = DeclarationId {
        package: 1,
        local: 50,
    };
    let parameter = DeclarationId {
        package: 1,
        local: 51,
    };
    let definition = nir::program::Struct {
        parameter_types: vec![],
        declaration: nominal,
        name: "Box".into(),
        type_parameters: vec![parameter],
        requirements: vec![],
        capabilities: vec![],
        fields: vec![(
            "value".into(),
            Term::Rigid {
                declaration: parameter,
                sort: Sort::Type,
            },
        )],
    };
    let mut root = function(1, Phase::Runtime, &[Sort::Type, Sort::Runtime]);
    let callback = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: Term::Bound {
                depth: 1,
                index: 0,
                sort: Sort::Type,
            },
        }],
        result: Box::new(Term::Bound {
            depth: 1,
            index: 0,
            sort: Sort::Type,
        }),
    };
    let Term::Function { parameters, .. } = &mut root.signature else {
        panic!()
    };
    parameters[0].ty = Term::Primitive(Primitive::Type);
    parameters[1].ty = Term::Nominal {
        declaration: nominal,
        arguments: vec![callback.clone()],
    };
    let mut package = Package {
        structs: vec![definition],
        functions: vec![root],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    let Term::Function { parameters, .. } = &mut package.functions[0].signature else {
        panic!()
    };
    parameters[1].ty = Term::Nominal {
        declaration: nominal,
        arguments: vec![Term::ComptimeFunction(Box::new(callback))],
    };
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("comptime-only")
    );
    let Term::Function { parameters, .. } = &mut package.functions[0].signature else {
        panic!()
    };
    parameters[1].ty = Term::Nominal {
        declaration: nominal,
        arguments: vec![Term::Bound {
            depth: 3,
            index: 0,
            sort: Sort::Type,
        }],
    };
    assert!(nir::phases::validate(&package).is_err());
}

#[test]
fn recursive_runtime_callback_storage_terminates_across_function_frames() {
    let nominal = DeclarationId {
        package: 1,
        local: 80,
    };
    let node = Term::Nominal {
        declaration: nominal,
        arguments: vec![],
    };
    let callback = Term::Function {
        parameters: vec![],
        result: Box::new(node.clone()),
    };
    let definition = nir::program::Struct {
        parameter_types: vec![],
        declaration: nominal,
        name: "Node".into(),
        type_parameters: vec![],
        requirements: vec![],
        capabilities: vec![],
        fields: vec![("next".into(), callback)],
    };
    let mut root = function(1, Phase::Runtime, &[Sort::Runtime]);
    let Term::Function { parameters, .. } = &mut root.signature else {
        panic!()
    };
    parameters[0].ty = node;
    let mut package = Package {
        structs: vec![definition],
        functions: vec![root],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    package.structs[0]
        .fields
        .push(("forbidden".into(), Term::Primitive(Primitive::Type)));
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("comptime-only")
    );
}

#[test]
fn standalone_runtime_domains_preserve_stage_and_scope_checks() {
    let package = Package::default();
    let check =
        |ty: &Term, scope: &[Vec<Sort>]| nir::phases::validate_runtime_storage(ty, &package, scope);
    let input = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Type,
    };
    assert!(check(&input, &[vec![Sort::Type]]).is_ok());
    assert!(check(&input, &[]).is_err());
    assert!(check(&Term::Bool(true), &[]).is_err());
    assert!(check(&Term::Primitive(Primitive::Type), &[]).is_err());
    assert!(check(&Term::Primitive(Primitive::Float), &[]).is_err());
    let callable = |sort| Term::Function {
        parameters: vec![Parameter {
            sort,
            ty: Term::Primitive(Primitive::Type),
        }],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    assert!(check(&callable(Sort::Type), &[]).is_ok());
    assert!(check(&callable(Sort::Runtime), &[]).is_err());
}

#[test]
fn immutable_type_observations_do_not_authorize_runtime_type_storage() {
    let type_value = Expr {
        source: SourceRange { start: 1, end: 2 },
        ty: Term::Primitive(Primitive::Type),
        kind: K::TypeValue(Term::Primitive(Primitive::Bool)),
    };
    let mut root = function(1, Phase::Runtime, &[]);
    root.body = flag(K::TypeEqual(
        Box::new(type_value.clone()),
        Box::new(type_value.clone()),
    ));
    let mut package = Package {
        functions: vec![root],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    package.functions[0].body = type_value.clone();
    assert!(nir::phases::validate(&package).is_err());
    let indirect = Expr {
        kind: K::Block {
            statements: vec![],
            result: Some(Box::new(type_value.clone())),
        },
        ..type_value.clone()
    };
    package.functions[0].body = flag(K::TypeEqual(Box::new(indirect), Box::new(type_value)));
    assert!(nir::phases::validate(&package).is_err());
}

#[test]
fn retained_quote_templates_are_checked_without_becoming_runtime_entries() {
    let inner = function(70, Phase::Runtime, &[]);
    let mut outer = function(71, Phase::Runtime, &[]);
    outer.body = call(&inner, vec![]);
    let mut package = Package {
        quote_templates: vec![inner.clone(), outer.clone()],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    let dump = package.to_string();
    assert_eq!(dump.matches("(quote-template ").count(), 2);
    assert!(dump.contains("function70"));
    let mut root = function(72, Phase::Runtime, &[]);
    root.body = call(&outer, vec![]);
    package.functions.push(root);
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("requires deterministic expansion")
    );
    package.functions.clear();
    let K::Call { callee, .. } = &outer.body.kind else {
        unreachable!()
    };
    package.quote_templates[1].body = (**callee).clone();
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("cannot escape")
    );
}

#[test]
fn ordinary_function_frames_reject_inconsistent_bindings_and_defaults() {
    for phase in [Phase::Runtime, Phase::Comptime] {
        let mut original = function(90, phase, &[Sort::Runtime, Sort::Runtime]);
        original
            .defaults
            .push((1, flag(K::Literal(Literal::Bool(true)))));
        let mut package = Package {
            functions: vec![original.clone()],
            ..Default::default()
        };
        nir::phases::validate(&package).unwrap();
        for mutation in 0..5 {
            let mut invalid = original.clone();
            match mutation {
                0 => {
                    invalid.parameters.pop();
                }
                1 => invalid.parameters[1] = invalid.parameters[0],
                2 => invalid.signature = Term::Primitive(Primitive::Bool),
                3 => invalid.defaults.push(invalid.defaults[0].clone()),
                4 => invalid.defaults[0].0 = 2,
                _ => unreachable!(),
            }
            package.functions[0] = invalid;
            let error = nir::phases::validate(&package).expect_err("invalid callable frame");
            assert!(error.message.contains("function parameter"), "{error:?}");
            package.functions[0] = original.clone();
            nir::phases::validate(&package).unwrap();
        }
    }
}

#[test]
fn retained_quote_templates_validate_parameter_frame_identity() {
    let mut template = function(80, Phase::Runtime, &[Sort::Runtime, Sort::Runtime]);
    template
        .defaults
        .push((1, flag(K::Literal(Literal::Bool(true)))));
    let mut package = Package {
        quote_templates: vec![template.clone()],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    for mutation in 0..5 {
        let mut invalid = template.clone();
        match mutation {
            0 => {
                invalid.parameters.pop();
            }
            1 => invalid.parameters[1] = invalid.parameters[0],
            2 => invalid.signature = Term::Primitive(Primitive::Bool),
            3 => invalid.defaults.push(invalid.defaults[0].clone()),
            4 => invalid.defaults[0].0 = 2,
            _ => unreachable!(),
        }
        package.quote_templates[0] = invalid;
        let error = nir::phases::validate(&package).expect_err("malformed quote parameter frame");
        assert!(
            error.message.contains("quote template parameter"),
            "{error:?}"
        );
        package.quote_templates[0] = template.clone();
        nir::phases::validate(&package).unwrap();
    }
}

#[test]
fn retained_quote_contract_bindings_cannot_alias_inputs_or_each_other() {
    let mut template = function(81, Phase::Runtime, &[Sort::Runtime]);
    let result = LocalId {
        owner: 81,
        index: 1,
    };
    let frozen = LocalId {
        owner: 81,
        index: 2,
    };
    template.result_binding = Some(result);
    template.entry_snapshots.push(nir::program::EntrySnapshot {
        parameter: template.parameters[0],
        binding: frozen,
    });
    let mut package = Package {
        quote_templates: vec![template.clone()],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    for mutation in 0..5 {
        let mut invalid = template.clone();
        match mutation {
            0 => invalid.result_binding = Some(invalid.parameters[0]),
            1 => invalid.entry_snapshots[0].parameter = result,
            2 => invalid.entry_snapshots[0].binding = invalid.parameters[0],
            3 => invalid.entry_snapshots[0].binding = result,
            4 => invalid
                .entry_snapshots
                .push(invalid.entry_snapshots[0].clone()),
            _ => unreachable!(),
        }
        package.quote_templates[0] = invalid;
        let error = nir::phases::validate(&package).expect_err("invalid quote contract identity");
        assert!(
            matches!(
                error.message,
                "result binding aliases an input parameter" | "invalid entry snapshot binding"
            ),
            "{error:?}"
        );
        assert_eq!(error.source, template.body.source);
        package.quote_templates[0] = template.clone();
        nir::phases::validate(&package).unwrap();
    }
}

#[test]
fn retained_quote_templates_reject_identity_collisions_and_wrong_phase() {
    let template = function(73, Phase::Runtime, &[]);
    let mut package = Package {
        quote_templates: vec![template.clone()],
        ..Default::default()
    };
    package.functions.push(template.clone());
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("quote template identity")
    );
    package.functions.clear();
    package.quote_templates.push(template.clone());
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("quote template identity")
    );
    package.quote_templates.pop();
    package
        .runtime_function_identities
        .push(nir::program::RuntimeFunctionIdentity {
            declaration: template.declaration,
            type_parameters: vec![],
            signature: template.signature,
        });
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("quote template identity")
    );
    package.runtime_function_identities.clear();
    package.quote_templates[0].phase = Phase::Comptime;
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("quote template identity")
    );
}

#[test]
fn retained_quote_templates_do_not_hide_invalid_runtime_storage() {
    let mut template = function(74, Phase::Runtime, &[]);
    template.body = Expr {
        source: SourceRange { start: 1, end: 2 },
        ty: Term::Primitive(Primitive::Type),
        kind: K::TypeValue(Term::Primitive(Primitive::Bool)),
    };
    let package = Package {
        quote_templates: vec![template],
        ..Default::default()
    };
    assert!(nir::phases::validate(&package).is_err());
}

#[test]
fn nested_preparation_stages_the_whole_producer_but_not_runtime_siblings() {
    use nir::program::Pattern;
    let compute = function(1, Phase::Comptime, &[]);
    let runtime = function(2, Phase::Runtime, &[]);
    let target = function(3, Phase::Runtime, &[Sort::ComptimeValue, Sort::Runtime]);
    let mut root = function(4, Phase::Runtime, &[]);
    let id = |index| LocalId { owner: 4, index };
    let tuple = |values: Vec<Expr>| Expr {
        ty: Term::Tuple(values.iter().map(|v| v.ty.clone()).collect()),
        kind: K::Tuple(values),
        source: SourceRange { start: 1, end: 2 },
    };
    let packet = tuple(vec![
        call(&compute, vec![]),
        tuple(vec![
            flag(K::Literal(Literal::Bool(false))),
            flag(K::Literal(Literal::Bool(true))),
        ]),
    ]);
    let unpack = Expr {
        source: packet.source,
        ty: Term::Primitive(Primitive::Void),
        kind: K::LetPattern {
            pattern: Pattern::Tuple(vec![
                Pattern::Binding {
                    binding: id(1),
                    inner: Box::new(Pattern::Wild),
                },
                Pattern::Tuple(vec![
                    Pattern::Wild,
                    Pattern::Binding {
                        binding: id(2),
                        inner: Box::new(Pattern::Wild),
                    },
                ]),
            ]),
            value: Box::new(packet),
            guard: None,
            otherwise: None,
        },
    };
    root.body = flag(K::CallPreparation {
        statements: vec![
            unpack,
            Expr {
                source: runtime.body.source,
                ty: Term::Primitive(Primitive::Void),
                kind: K::Let {
                    binding: id(3),
                    value: Box::new(call(&runtime, vec![])),
                },
            },
        ],
        result: Some(Box::new(call(
            &target,
            vec![(0, flag(K::Local(id(1)))), (1, flag(K::Local(id(2))))],
        ))),
    });
    let mut package = Package {
        functions: vec![compute, runtime.clone(), target, root],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    let K::CallPreparation { statements, .. } = &mut package.functions[3].body.kind else {
        unreachable!()
    };
    let K::LetPattern { value, .. } = &mut statements[0].kind else {
        unreachable!()
    };
    let K::Tuple(fields) = &mut value.kind else {
        unreachable!()
    };
    let K::Tuple(inner) = &mut fields[1].kind else {
        unreachable!()
    };
    inner[0] = call(&runtime, vec![]);
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("runtime function is unavailable")
    );
}

#[test]
fn diverging_preparation_checks_patterns_without_requiring_tuple_layouts() {
    use nir::program::Pattern;
    let target = function(1, Phase::Runtime, &[]);
    let mut root = function(2, Phase::Runtime, &[]);
    let binding = LocalId { owner: 2, index: 1 };
    let bind = Pattern::Binding {
        binding,
        inner: Box::new(Pattern::Wild),
    };
    root.body = Expr {
        ty: Term::Primitive(Primitive::Never),
        source: root.body.source,
        kind: K::CallPreparation {
            statements: vec![Expr {
                ty: Term::Primitive(Primitive::Never),
                source: root.body.source,
                kind: K::LetPattern {
                    pattern: Pattern::Tuple(vec![
                        Pattern::Tuple(vec![bind.clone()]),
                        Pattern::Wild,
                    ]),
                    value: Box::new(Expr {
                        ty: Term::Primitive(Primitive::Never),
                        source: root.body.source,
                        kind: K::Return(Some(Box::new(flag(K::Literal(Literal::Bool(true)))))),
                    }),
                    guard: None,
                    otherwise: None,
                },
            }],
            result: Some(Box::new(call(&target, vec![]))),
        },
    };
    let mut package = Package {
        functions: vec![target, root],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    let K::CallPreparation { statements, .. } = &mut package.functions[1].body.kind else {
        unreachable!()
    };
    let K::LetPattern { pattern, .. } = &mut statements[0].kind else {
        unreachable!()
    };
    *pattern = Pattern::Tuple(vec![bind.clone(), Pattern::Tuple(vec![bind])]);
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("distinct")
    );
}

#[test]
fn nominal_contextual_match_patterns_cannot_hide_invalid_function_references() {
    use nir::program::{NormalFormArgument as A, NormalMatchArm, Pattern};
    let mut root = function(80, Phase::Comptime, &[]);
    root.body = Expr {
        source: root.body.source,
        ty: Term::Primitive(Primitive::Type),
        kind: K::NominalType {
            declaration: DeclarationId {
                package: 1,
                local: 81,
            },
            bindings: None,
            order: None,
            preparations: vec![],
            arguments: vec![A::Match {
                source: root.body.source,
                value: flag(K::Literal(Literal::Bool(true))),
                arms: vec![NormalMatchArm {
                    pattern: Pattern::Guard {
                        pattern: Box::new(Pattern::Wild),
                        condition: Box::new(flag(K::Function(DeclarationId {
                            package: 1,
                            local: 999,
                        }))),
                    },
                    guard: None,
                    body: A::Value(flag(K::Literal(Literal::Bool(true)))),
                }],
            }],
        },
    };
    let package = Package {
        functions: vec![root],
        ..Default::default()
    };
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("function phase declaration is missing")
    );
}

#[test]
fn alias_body_cannot_hide_missing_function_references_or_runtime_execution() {
    let declaration = DeclarationId {
        package: 1,
        local: 20,
    };
    let mut entry = function(1, Phase::Comptime, &[]);
    entry.signature = Term::Function {
        parameters: vec![],
        result: Box::new(Term::Primitive(Primitive::Type)),
    };
    let mut alias_body = flag(K::TypeValue(Term::Primitive(Primitive::Bool)));
    alias_body.ty = Term::Primitive(Primitive::Type);
    entry.body = flag(K::CheckTypeAlias {
        preparations: vec![],
        bindings: None,
        order: None,
        declaration,
        arguments: vec![]
            .into_iter()
            .map(nir::program::NormalFormArgument::Value)
            .collect(),
        body: Some(Box::new(alias_body)),
    });
    entry.body.ty = Term::Primitive(Primitive::Type);
    let mut package = Package {
        alias_contracts: vec![nir::type_aliases::Contract {
            declaration,
            parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
        }],
        functions: vec![entry],
        ..Default::default()
    };
    assert!(nir::phases::validate(&package).is_ok());
    package.functions[0].phase = Phase::Runtime;
    assert!(nir::phases::validate(&package).is_err());
    package.functions[0].phase = Phase::Comptime;
    let K::CheckTypeAlias {
        body: Some(body), ..
    } = &mut package.functions[0].body.kind
    else {
        panic!()
    };
    body.kind = K::Function(DeclarationId {
        package: 1,
        local: 999,
    });
    assert!(nir::phases::validate(&package).is_err());
}

#[test]
fn captured_comptime_value_nodes_cannot_encode_types_or_runtime_slots() {
    let mut owner = function(70, Phase::Comptime, &[]);
    let mut package = Package::default();
    for (domain, value) in [
        (
            Term::Primitive(Primitive::Type),
            Term::Primitive(Primitive::Bool),
        ),
        (
            Term::Primitive(Primitive::Bool),
            Term::Primitive(Primitive::Bool),
        ),
        (Term::Primitive(Primitive::Bool), Term::Char('x')),
        (
            Term::Primitive(Primitive::U8),
            Term::Integer(comptime_runtime::Integer::from_i128(256)),
        ),
        (
            Term::Primitive(Primitive::Bool),
            Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::ComptimeValue,
            },
        ),
    ] {
        owner.body = Expr {
            source: owner.body.source,
            ty: domain.clone(),
            kind: K::ComptimeValue(value),
        };
        owner.signature = Term::Function {
            parameters: vec![],
            result: Box::new(domain),
        };
        package.functions = vec![owner.clone()];
        assert!(nir::phases::validate(&package).is_err(), "{}", owner.body);
    }
    owner.body = flag(K::ComptimeValue(Term::Bool(true)));
    owner.signature = Term::Function {
        parameters: vec![],
        result: Box::new(owner.body.ty.clone()),
    };
    package.functions = vec![owner];
    nir::phases::validate(&package).unwrap();
}

#[test]
fn unresolved_pattern_code_evidence_never_enters_executable_packages() {
    for phase in [Phase::Runtime, Phase::Comptime] {
        let mut root = function(1, phase, &[]);
        root.body = flag(K::Match {
            value: Box::new(flag(K::Literal(Literal::Bool(true)))),
            arms: vec![nir::program::MatchArm {
                pattern: nir::program::Pattern::CodeEvidence { outputs: vec![] },
                guard: None,
                body: flag(K::Literal(Literal::Bool(true))),
            }],
        });
        let package = Package {
            functions: vec![root],
            ..Default::default()
        };
        let error = nir::phases::validate(&package).unwrap_err();
        assert_eq!(error.message, "unresolved pattern-code evidence");
    }
}

#[test]
fn computation_result_storage_retains_its_execution_phase() {
    for (result, accepted) in [
        (Primitive::Bool, true),
        (Primitive::Type, false),
        (Primitive::Integer, false),
    ] {
        let mut root = function(1, Phase::Runtime, &[Sort::Runtime]);
        let Term::Function { parameters, .. } = &mut root.signature else {
            unreachable!()
        };
        parameters[0].ty = Term::Computation {
            effects: vec![],
            result: Box::new(Term::Primitive(result)),
        };
        let package = Package {
            functions: vec![root],
            ..Default::default()
        };
        let checked = nir::phases::validate(&package);
        if accepted {
            checked.unwrap();
        } else {
            assert!(checked.unwrap_err().message.contains("comptime-only"));
        }
    }
}

#[test]
fn concrete_storage_layout_inspection_spends_shared_budget_and_recovers() {
    let nominal = Term::Nominal {
        declaration: DeclarationId {
            package: 9,
            local: 1,
        },
        arguments: vec![],
    };
    let metadata = Package::default();
    let resolve = |ty: &Term, fuel: &mut usize| -> Option<Vec<Term>> {
        if ty != &nominal {
            return None;
        }
        (0..256)
            .map(|_| {
                let field = Term::Primitive(Primitive::Bool);
                nir::type_computations::charge_term(&field, fuel).ok()?;
                Some(field)
            })
            .collect()
    };
    let mut small = 16;
    assert!(
        nir::phases::validate_runtime_storage_with_nominals(
            &nominal,
            &metadata,
            &[],
            &resolve,
            &mut small
        )
        .is_err()
    );
    assert!(small < 16);
    let mut enough = 4096;
    nir::phases::validate_runtime_storage_with_nominals(
        &nominal,
        &metadata,
        &[],
        &resolve,
        &mut enough,
    )
    .unwrap();
    assert!(enough < 4096 - 256);
    let mut empty = 0;
    assert!(
        nir::phases::validate_runtime_storage_with_nominals(
            &nominal,
            &metadata,
            &[],
            &resolve,
            &mut empty
        )
        .is_err()
    );
    nir::phases::validate_runtime_storage_with_nominals(
        &nominal,
        &metadata,
        &[],
        &resolve,
        &mut enough,
    )
    .unwrap();
    assert!(
        nir::phases::validate_runtime_storage_with_nominals(
            &nominal,
            &metadata,
            &[],
            &|_, _| None,
            &mut enough
        )
        .is_err()
    );
}

#[test]
fn unsafe_callable_calls_are_checked_against_lexical_scope_and_target_identity() {
    let mut target = function(901, Phase::Runtime, &[]);
    target.is_unsafe = true;
    let mut caller = function(902, Phase::Runtime, &[]);
    caller.body = call(&target, vec![]);
    let package = |caller| Package {
        functions: vec![caller, target.clone()],
        ..Default::default()
    };
    let error = nir::phases::validate(&package(caller.clone())).unwrap_err();
    assert!(error.message.contains("unsafe boundary"), "{error:?}");
    caller.body = flag(K::UnsafeScope(Box::new(caller.body)));
    nir::phases::validate(&package(caller.clone())).unwrap();
    nir::visit::expression_mut(&mut caller.body, &mut 1000, |expr, _| {
        if let K::Function(_) = expr.kind {
            expr.ty = target.signature.clone();
        }
        Ok(())
    })
    .unwrap();
    let error = nir::phases::validate(&package(caller)).unwrap_err();
    assert!(
        error.message.contains("declaration responsibility"),
        "{error:?}"
    );
}

#[test]
fn declaration_responsibility_survives_specialization_without_wrapping_its_telescope() {
    for phase in [Phase::Runtime, Phase::Comptime] {
        let mut original = function(903, phase, &[Sort::Runtime]);
        original.is_unsafe = true;
        let specialized = original.instantiate_parameters(&[], &[]).unwrap();
        assert!(specialized.is_unsafe);
        assert_eq!(specialized.phase, phase);
        assert_eq!(specialized.signature, original.signature);
        assert!(matches!(specialized.signature, Term::Function { .. }));
        let value_type = specialized.callable_type();
        value_type.check_scope(&[]).unwrap();
        assert!(value_type.is_unsafe_callable());
        assert_eq!(value_type.is_comptime_callable(), phase == Phase::Comptime);
        assert_eq!(value_type.callable_signature(), &original.signature);
        assert!(specialized.to_string().contains("(unsafe true)"));
        nir::phases::validate(&Package {
            functions: vec![specialized],
            ..Default::default()
        })
        .unwrap();
    }
}

#[test]
fn struct_reflection_is_not_a_runtime_operation_even_with_a_runtime_result_annotation() {
    for operation in 0..18 {
        let mut f = function(1, Phase::Runtime, &[]);
        let subject = Box::new(Expr {
            source: f.body.source,
            ty: Term::Primitive(Primitive::Type),
            kind: K::TypeValue(Term::Primitive(Primitive::Bool)),
        });
        f.body.kind = match operation {
            0 => K::StructFieldNames(subject),
            1 => K::StructFieldTypes(subject),
            2 => K::BuildStruct(subject),
            3 => K::BuildEnum(subject),
            4..=8 => K::EnumStructure {
                subject,
                query: [
                    nir::program::EnumStructureQuery::GroupPaths,
                    nir::program::EnumStructureQuery::VariantPaths,
                    nir::program::EnumStructureQuery::Shapes,
                    nir::program::EnumStructureQuery::FieldNames,
                    nir::program::EnumStructureQuery::FieldTypes,
                ][operation - 4],
            },
            9..=13 => K::ObjectQuery {
                subject,
                query: [
                    nir::program::ObjectQueryKind::Length,
                    nir::program::ObjectQueryKind::Key,
                    nir::program::ObjectQueryKind::EntryType,
                    nir::program::ObjectQueryKind::Value,
                    nir::program::ObjectQueryKind::NamedValue,
                ][operation - 9],
            },
            _ => K::ObjectEdit {
                subject,
                operation: [
                    nir::program::ObjectEditKind::Concat,
                    nir::program::ObjectEditKind::Append,
                    nir::program::ObjectEditKind::Replace,
                    nir::program::ObjectEditKind::Remove,
                ][operation - 14],
            },
        };
        let mut package = Package {
            functions: vec![f],
            ..Default::default()
        };
        let error = nir::phases::validate(&package).unwrap_err();
        assert!(
            error.message.contains("requires comptime invocation"),
            "{error:?}"
        );
        package.functions[0].phase = Phase::Comptime;
        // Phase admission is separate from the VM's result-domain validation.
        nir::phases::validate(&package).unwrap();
    }
}

#[test]
fn dynamic_comptime_instances_preserve_phase_and_responsibility() {
    let target = function(2, Phase::Comptime, &[]);
    let mut caller = function(1, Phase::Comptime, &[]);
    let mut expression = call(&target, vec![]);
    let K::Call { callee, .. } = &mut expression.kind else {
        panic!()
    };
    callee.kind = K::ComptimeFunctionInstance {
        declaration: target.declaration,
        subject: Box::new(Expr {
            source: callee.source,
            ty: Term::Tuple(vec![]),
            kind: K::Tuple(vec![]),
        }),
    };
    caller.body = expression;
    let mut package = Package {
        functions: vec![caller, target],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    package.functions[0].phase = Phase::Runtime;
    assert!(nir::phases::validate(&package).is_err());
    package.functions[0].phase = Phase::Comptime;
    package.functions[1].is_unsafe = true;
    assert!(nir::phases::validate(&package).is_err());
}

#[test]
fn integer_representation_checks_domains_without_resolving_target_width() {
    for phase in [Phase::Runtime, Phase::Comptime] {
        for input in [
            Primitive::I8,
            Primitive::U128,
            Primitive::Isize,
            Primitive::Usize,
        ] {
            let mut f = function(1, phase, &[]);
            f.signature = Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(Primitive::U128)),
            };
            f.body = Expr {
                source: f.body.source,
                ty: Term::Primitive(Primitive::U128),
                kind: K::IntegerRepresentation(Box::new(Expr {
                    source: f.body.source,
                    ty: Term::Primitive(input),
                    kind: K::Literal(if matches!(input, Primitive::I8 | Primitive::Isize) {
                        Literal::SignedInteger(0)
                    } else {
                        Literal::UnsignedInteger(0)
                    }),
                })),
            };
            let package = Package {
                functions: vec![f],
                ..Default::default()
            };
            nir::phases::validate(&package).unwrap();
            for wrong in [
                Primitive::Bool,
                Primitive::Char,
                Primitive::Str,
                Primitive::F64,
                Primitive::BitVec(64),
            ] {
                let mut invalid = package.clone();
                let K::IntegerRepresentation(value) = &mut invalid.functions[0].body.kind else {
                    unreachable!()
                };
                value.ty = Term::Primitive(wrong);
                assert!(
                    nir::phases::validate(&invalid)
                        .unwrap_err()
                        .message
                        .contains("integer representation domain mismatch")
                );
            }
            let mut invalid = package;
            invalid.functions[0].body.ty = Term::Primitive(Primitive::U64);
            assert!(nir::phases::validate(&invalid).is_err());
        }
    }
}

#[test]
fn value_reference_transport_preserves_phase_and_receiver_identity() {
    use nir::program::ArgumentPassing;
    let target = function(2, Phase::Comptime, &[Sort::Runtime]);
    let mut caller = function(1, Phase::Comptime, &[]);
    caller.body = call(&target, vec![(0, flag(K::Literal(Literal::Bool(false))))]);
    let K::Call { arguments, .. } = &mut caller.body.kind else {
        unreachable!()
    };
    arguments[0].passing = ArgumentPassing::ComptimeReference;
    let package = Package {
        functions: vec![caller.clone(), target.clone()],
        ..Default::default()
    };
    nir::phases::validate(&package).unwrap();
    assert!(
        caller
            .body
            .to_string()
            .contains("(passing comptime-reference)")
    );
    let instantiated = caller.body.instantiate_parameters(&[], &[]).unwrap();
    let K::Call { arguments, .. } = instantiated.kind else {
        unreachable!()
    };
    assert_eq!(arguments[0].passing, ArgumentPassing::ComptimeReference);
    for damage in 0..3 {
        let mut invalid = package.clone();
        if damage == 0 {
            invalid.functions[0].phase = Phase::Runtime;
        }
        let K::Call {
            callee, arguments, ..
        } = &mut invalid.functions[0].body.kind
        else {
            unreachable!()
        };
        match damage {
            1 => callee.ty = target.signature.clone(),
            2 => arguments[0].is_default = true,
            _ => {}
        }
        assert!(nir::phases::validate(&invalid).is_err());
    }
}

#[test]
fn optional_storage_checks_payload_phase_at_every_layer() {
    let package = Package::default();
    let forbidden = [
        Term::Primitive(Primitive::Type),
        Term::Primitive(Primitive::Integer),
        Term::Primitive(Primitive::Float),
        Term::Primitive(Primitive::Object),
        Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(Primitive::Bool)),
        })),
    ];
    for mut ty in forbidden {
        for _ in 0..4 {
            ty = Term::Optional(Box::new(ty));
            assert!(nir::phases::validate_runtime_storage(&ty, &package, &[]).is_err());
            let mut f = function(1, Phase::Runtime, &[Sort::Runtime]);
            let Term::Function { parameters, .. } = &mut f.signature else {
                unreachable!()
            };
            parameters[0].ty = ty.clone();
            let package = Package {
                functions: vec![f],
                ..Default::default()
            };
            assert!(nir::phases::validate(&package).is_err());
            let mut comptime = package;
            comptime.functions[0].phase = Phase::Comptime;
            nir::phases::validate(&comptime).unwrap();
        }
    }
    let mut legal = Term::Primitive(Primitive::Bool);
    for _ in 0..4 {
        legal = Term::Optional(Box::new(legal));
        nir::phases::validate_runtime_storage(&legal, &package, &[]).unwrap();
    }
}
