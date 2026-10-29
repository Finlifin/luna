use comptime_interpretor::{Budget, Interpreter, Value};
use nir::normal_forms::{Head, Kind, Parameter as Input};
use nir::program::{
    Expr, ExprKind as K, Function, FunctionPhase, LocalId, NormalFormArgument, Package, SourceRange,
};
use nir::{DeclarationId, Parameter, Primitive as P, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 21, local }
}
fn ty(p: P) -> Term {
    Term::Primitive(p)
}
fn list_type() -> Term {
    Term::NormalForm {
        head: id(1),
        sort: Sort::Type,
        arguments: vec![ty(P::Type)],
    }
}
fn tuple_type(elements: Vec<Term>) -> Term {
    Term::NormalForm {
        head: id(2),
        sort: Sort::Type,
        arguments: vec![Term::TypeListValue(elements)],
    }
}
fn package() -> Package {
    let binding = LocalId { owner: 3, index: 0 };
    let source = SourceRange { start: 0, end: 1 };
    Package {
        normal_forms: vec![
            Head {
                role: nir::normal_forms::Role::TypeList,
                declaration: id(1),
                requirements: vec![],
                parameters: vec![Input {
                    declaration: id(10),
                    binding: LocalId { owner: 1, index: 0 },
                    kind: Kind::Type,
                    ty: ty(P::Type),
                }],
            },
            Head {
                role: nir::normal_forms::Role::Opaque,
                declaration: id(2),
                requirements: vec![],
                parameters: vec![Input {
                    declaration: id(20),
                    binding: LocalId { owner: 2, index: 0 },
                    kind: Kind::Value,
                    ty: list_type(),
                }],
            },
        ],
        functions: vec![Function {
            declaration: id(3),
            name: "BuildTuple".into(),
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: true,
            is_unsafe: false,
            type_parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            parameters: vec![binding],
            defaults: vec![],
            signature: Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::ComptimeValue,
                    ty: list_type(),
                }],
                result: Box::new(ty(P::Type)),
            },
            body: Expr {
                ty: ty(P::Type),
                source,
                kind: K::NormalFormType {
                    head: id(2),
                    plan: None,
                    arguments: vec![NormalFormArgument::Value(Expr {
                        ty: list_type(),
                        source,
                        kind: K::Local(binding),
                    })],
                },
            },
        }],
        ..Default::default()
    }
}

#[test]
fn homogeneous_value_lists_require_the_bound_head_and_validate_every_element() {
    let mut package = package();
    package.normal_forms.truncate(1);
    package.normal_forms[0].role = nir::normal_forms::Role::TypeList;
    let list = Term::NormalForm {
        head: id(1),
        sort: Sort::Type,
        arguments: vec![ty(P::Bool)],
    };
    let function = &mut package.functions[0];
    function.signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::ComptimeValue,
            ty: list.clone(),
        }],
        result: Box::new(list.clone()),
    };
    function.body.ty = list.clone();
    function.body.kind = K::Local(function.parameters[0]);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let good = Value::List {
        ty: list.clone(),
        values: vec![Value::Bool(true), Value::Bool(false)],
    };
    for bad in [
        Value::List {
            ty: list.clone(),
            values: vec![Value::Bool(true), Value::Type(ty(P::Bool))],
        },
        Value::List {
            ty: list_type(),
            values: vec![Value::Bool(true)],
        },
        Value::Tuple(vec![Value::Bool(true)]),
        Value::TypeList(vec![ty(P::Bool)]),
    ] {
        assert!(vm.invoke(id(3), vec![bad]).is_err());
        assert_eq!(vm.invoke(id(3), vec![good.clone()]).unwrap(), good);
    }
    let mut unbound = package.clone();
    unbound.normal_forms[0].role = nir::normal_forms::Role::Opaque;
    let mut vm = Interpreter::for_comptime(&unbound, Budget::default()).unwrap();
    assert!(vm.invoke(id(3), vec![good.clone()]).is_err());
    let wide = Value::List {
        ty: list.clone(),
        values: vec![Value::Bool(true); 100_001],
    };
    assert!(wide.immutable_copy_work().is_none());
    assert!(good.immutable_copy_work().is_some());
    let frozen = good
        .clone()
        .into_normal_term(&mut Default::default())
        .unwrap();
    assert_eq!(
        frozen,
        Term::NominalValue {
            ty: Box::new(list.clone()),
            variant: None,
            fields: vec![Term::Bool(true), Term::Bool(false)],
        }
    );
    assert!(frozen.check_scope(&[]).is_ok());
    assert!(
        vm.invoke_with_normal_arguments(id(3), vec![(list.clone(), frozen.clone())])
            .is_err(),
        "the unbound head must not acquire list representation authority by serialization"
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for malformed in [
        Term::NominalValue {
            ty: Box::new(list_type()),
            variant: None,
            fields: vec![Term::Bool(true)],
        },
        Term::NominalValue {
            ty: Box::new(list.clone()),
            variant: Some(0),
            fields: vec![Term::Bool(true)],
        },
        Term::NominalValue {
            ty: Box::new(list.clone()),
            variant: None,
            fields: vec![Term::Primitive(P::Bool)],
        },
        Term::TupleValue(vec![Term::Bool(true)]),
        Term::TypeListValue(vec![Term::Primitive(P::Bool)]),
    ] {
        assert!(
            vm.invoke_with_normal_arguments(id(3), vec![(list.clone(), malformed)])
                .is_err()
        );
        assert_eq!(
            vm.invoke_with_normal_arguments(id(3), vec![(list.clone(), frozen.clone())])
                .unwrap(),
            good
        );
    }
    assert!(
        good.clone()
            .into_normal_term(&mut comptime_runtime::Budget {
                steps: 1,
                ..Default::default()
            })
            .is_err()
    );
    let empty = Value::List {
        ty: list.clone(),
        values: vec![],
    };
    let empty_term = empty
        .clone()
        .into_normal_term(&mut Default::default())
        .unwrap();
    assert_eq!(
        vm.invoke_with_normal_arguments(id(3), vec![(list, empty_term)])
            .unwrap(),
        empty
    );
}

#[test]
fn collected_type_list_crosses_invocation_and_preserves_tuple_head() {
    let package = package();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    for elements in [
        vec![],
        vec![ty(P::U8)],
        vec![ty(P::U8), ty(P::Bool), ty(P::U8)],
    ] {
        let result = vm
            .invoke(id(3), vec![Value::TypeList(elements.clone())])
            .unwrap();
        assert_eq!(result, Value::Type(tuple_type(elements.clone())));
        assert_ne!(result, Value::Type(Term::Tuple(elements.clone())));
        let thawed = vm
            .invoke_with_normal_arguments(
                id(3),
                vec![(list_type(), Term::TypeListValue(elements.clone()))],
            )
            .unwrap();
        assert_eq!(thawed, Value::Type(tuple_type(elements)));
    }
}

#[test]
fn type_list_constant_nodes_validate_elements_domain_and_substitution() {
    let mut package = package();
    let source = SourceRange { start: 0, end: 1 };
    package.functions[0].parameters.clear();
    package.functions[0].signature = Term::Function {
        parameters: vec![],
        result: Box::new(list_type()),
    };
    for types in [vec![], vec![ty(P::Bool), ty(P::U8)]] {
        package.functions[0].body = Expr {
            source,
            ty: list_type(),
            kind: K::TypeListValue(types.clone()),
        };
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(vm.invoke(id(3), vec![]).unwrap(), Value::TypeList(types));
    }
    for invalid in [
        Term::Bool(true),
        Term::Rigid {
            declaration: id(99),
            sort: Sort::Type,
        },
    ] {
        package.functions[0].body.kind = K::TypeListValue(vec![invalid]);
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert!(vm.invoke(id(3), vec![]).is_err());
    }
    let mut constant = Expr {
        source,
        ty: list_type(),
        kind: K::TypeListValue(vec![Term::Rigid {
            declaration: id(99),
            sort: Sort::Type,
        }]),
    };
    constant
        .rewrite_types(&mut |t| t.instantiate_types(&[id(99)], &[ty(P::Bool)]))
        .unwrap();
    assert_eq!(constant.kind, K::TypeListValue(vec![ty(P::Bool)]));
    assert!(constant.to_string().contains("(type-list-value"));
    package.functions[0].body = constant;
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(3), vec![]).unwrap(),
        Value::TypeList(vec![ty(P::Bool)])
    );
    package.functions[0].body.ty = ty(P::Type);
    assert!(
        Interpreter::new(&package, Budget::default())
            .unwrap()
            .invoke(id(3), vec![])
            .is_err()
    );
    package.functions[0].body.ty = list_type();
    package.normal_forms.clear();
    assert!(
        Interpreter::new(&package, Budget::default())
            .unwrap()
            .invoke(id(3), vec![])
            .is_err()
    );
}

#[test]
fn list_input_contract_rejects_tuple_wrong_elements_and_recovers() {
    let package = package();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    for bad in [
        Value::Tuple(vec![Value::Type(ty(P::U8))]),
        Value::TypeList(vec![Term::Bool(true)]),
        Value::Type(ty(P::U8)),
    ] {
        assert!(vm.invoke(id(3), vec![bad]).is_err());
        assert_eq!(
            vm.invoke(id(3), vec![Value::TypeList(vec![ty(P::Bool)])])
                .unwrap(),
            Value::Type(tuple_type(vec![ty(P::Bool)]))
        );
    }
    assert!(
        vm.invoke_with_normal_arguments(
            id(3),
            vec![(list_type(), Term::TupleValue(vec![ty(P::U8)]))]
        )
        .is_err()
    );
    assert!(
        vm.validate_immutable_input(&Value::TypeList(vec![ty(P::Bool)]), &list_type())
            .is_ok()
    );
}

#[test]
fn list_domain_uses_ordinary_value_metadata_and_registered_layout() {
    let mut package = package();
    assert!(nir::normal_forms::validate(&package).is_ok());
    assert_eq!(package.normal_forms[1].parameters[0].kind, Kind::Value);
    assert!(package.normal_forms[1].to_string().contains("(kind value)"));
    // A parameter cannot grant a made-up declaration the list representation.
    package.normal_forms[0].role = nir::normal_forms::Role::Opaque;
    assert!(nir::normal_forms::validate(&package).is_err());
    package.normal_forms[0].role = nir::normal_forms::Role::TypeList;
    assert!(nir::normal_forms::validate(&package).is_ok());
    package.normal_forms[1].parameters[0].kind = Kind::Type;
    assert!(nir::normal_forms::validate(&package).is_err());
}

#[test]
fn contextual_tuple_types_preserve_the_head_before_and_after_their_context_arrives() {
    use nir::program::{NormalFormInputPlan, TypePredicate};
    let mut package = package();
    package.normal_forms[1].role = nir::normal_forms::Role::Tuple;
    package.normal_forms.push(Head {
        role: nir::normal_forms::Role::Opaque,
        declaration: id(4),
        requirements: vec![],
        parameters: vec![
            Input {
                declaration: id(40),
                binding: LocalId { owner: 4, index: 0 },
                kind: Kind::Type,
                ty: ty(P::Type),
            },
            Input {
                declaration: id(41),
                binding: LocalId { owner: 4, index: 1 },
                kind: Kind::Value,
                ty: Term::Rigid {
                    declaration: id(40),
                    sort: Sort::Type,
                },
            },
        ],
    });
    let source = SourceRange { start: 0, end: 1 };
    let type_input = |value| {
        NormalFormArgument::Value(Expr {
            source,
            ty: ty(P::Type),
            kind: K::TypeValue(value),
        })
    };
    package.functions[0].parameters.clear();
    package.functions[0].signature = Term::Function {
        parameters: vec![],
        result: Box::new(ty(P::Type)),
    };
    for order in [vec![0, 1], vec![1, 0]] {
        package.functions[0].body.kind = K::NormalFormType {
            head: id(4),
            plan: Some(NormalFormInputPlan {
                preparations: vec![],
                order,
                bindings: vec![
                    LocalId { owner: 3, index: 1 },
                    LocalId { owner: 3, index: 2 },
                ],
            }),
            arguments: vec![
                type_input(ty(P::Type)),
                NormalFormArgument::Tuple {
                    source,
                    type_head: Some(id(2)),
                    items: vec![type_input(ty(P::Bool)), type_input(ty(P::U8))],
                },
            ],
        };
        assert!(package.functions[0].body.to_string().contains("(type-head"));
        let expected = Value::Type(Term::NormalForm {
            head: id(4),
            sort: Sort::Type,
            arguments: vec![ty(P::Type), tuple_type(vec![ty(P::Bool), ty(P::U8)])],
        });
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(vm.invoke(id(3), vec![]).unwrap(), expected);

        // A contextual Tuple construction must enforce the inner head's
        // formation contract, not just the surrounding normal-form contract.
        let mut denied = package.clone();
        denied.normal_forms[1]
            .requirements
            .push(TypePredicate::Equal(ty(P::Bool), ty(P::U8)));
        let mut vm = Interpreter::new(&denied, Budget::default()).unwrap();
        assert!(vm.invoke(id(3), vec![]).is_err());
        let mut unbound = package.clone();
        unbound.normal_forms[1].role = nir::normal_forms::Role::Opaque;
        let mut vm = Interpreter::new(&unbound, Budget::default()).unwrap();
        assert!(vm.invoke(id(3), vec![]).is_err());
    }
}

#[test]
fn registered_list_heads_admit_reduced_inputs_and_reject_pending_concatenation() {
    let package = package();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    let parts = vec![
        Term::TypeListValue(vec![ty(P::Bool)]),
        Term::TypeListValue(vec![ty(P::U8)]),
    ];
    let pending = Term::NormalForm {
        head: id(2),
        sort: Sort::Type,
        arguments: vec![Term::TypeListConcat(parts.clone())],
    };
    assert!(
        vm.validate_immutable_input(&Value::Type(pending), &ty(P::Type))
            .is_err()
    );
    let completed = Term::NormalForm {
        head: id(2),
        sort: Sort::Type,
        arguments: vec![Term::type_list_concat(parts)],
    };
    assert!(
        vm.validate_immutable_input(&Value::Type(completed.clone()), &ty(P::Type))
            .is_ok()
    );
    assert_eq!(completed, tuple_type(vec![ty(P::Bool), ty(P::U8)]));
}

#[test]
fn explicit_list_role_does_not_require_a_variadic_consumer() {
    let mut package = package();
    package.normal_forms.truncate(1);
    package.normal_forms[0].role = nir::normal_forms::Role::TypeList;
    let function = &mut package.functions[0];
    let Term::Function { result, .. } = &mut function.signature else {
        unreachable!()
    };
    *result = Box::new(list_type());
    function.body = Expr {
        ty: list_type(),
        source: function.body.source,
        kind: K::Local(function.parameters[0]),
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for types in [vec![], vec![ty(P::Bool), ty(P::U8)]] {
        let value = Value::TypeList(types);
        assert_eq!(vm.invoke(id(3), vec![value.clone()]).unwrap(), value);
    }
    let mut unbound = package.clone();
    unbound.normal_forms[0].role = nir::normal_forms::Role::Opaque;
    let mut vm = Interpreter::for_comptime(&unbound, Budget::default()).unwrap();
    assert!(vm.invoke(id(3), vec![Value::TypeList(vec![])]).is_err());

    let mut invalid = package.clone();
    invalid.normal_forms[0].parameters.clear();
    assert!(Interpreter::for_comptime(&invalid, Budget::default()).is_err());
    let mut impostor = package.clone();
    impostor.normal_forms[0].declaration = id(99);
    let mut vm = Interpreter::for_comptime(&impostor, Budget::default()).unwrap();
    assert!(vm.invoke(id(3), vec![Value::TypeList(vec![])]).is_err());
}

#[test]
fn planned_input_type_rewrite_charges_unused_closure_body() {
    use nir::program::{Literal, NormalFormInputPlan};
    let mut package = package();
    let source = SourceRange { start: 0, end: 1 };
    package.functions[0].parameters.clear();
    package.functions[0].signature = Term::Function {
        parameters: vec![],
        result: Box::new(ty(P::Type)),
    };
    let closure = Expr {
        source,
        ty: Term::Function {
            parameters: vec![],
            result: Box::new(ty(P::Str)),
        }
        .with_comptime_phase(true),
        kind: K::Closure {
            parameters: vec![],
            captures: vec![],
            body: std::sync::Arc::new(Expr {
                source,
                ty: ty(P::Str),
                kind: K::Literal(Literal::String("x".repeat(4096))),
            }),
        },
    };
    package.functions[0].body.kind = K::NormalFormType {
        head: id(2),
        plan: Some(NormalFormInputPlan {
            order: vec![0],
            bindings: vec![LocalId { owner: 3, index: 1 }],
            preparations: vec![],
        }),
        arguments: vec![NormalFormArgument::Block {
            source,
            statements: vec![Expr {
                source,
                ty: ty(P::Void),
                kind: K::Let {
                    binding: LocalId { owner: 3, index: 2 },
                    value: Box::new(closure),
                },
            }],
            result: Some(Box::new(NormalFormArgument::Value(Expr {
                source,
                ty: list_type(),
                kind: K::TypeListValue(vec![]),
            }))),
        }],
    };
    let mut low = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    low.charge_preparation((low.remaining_steps() - 512) as usize)
        .unwrap();
    assert_eq!(
        low.invoke(id(3), vec![]).unwrap_err().kind,
        comptime_interpretor::ErrorKind::StepsExhausted
    );
    let mut enough = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        enough.invoke(id(3), vec![]).unwrap(),
        Value::Type(tuple_type(vec![]))
    );
}

#[test]
fn repeated_fixed_list_indices_charge_selected_elements_not_whole_lists() {
    check_list_read_cost(false);
}

#[test]
fn repeated_local_list_indices_charge_selected_elements_not_whole_lists() {
    check_list_read_cost(true);
}

fn check_list_read_cost(local_index: bool) {
    for element in [P::Bool, P::Type] {
        let run = |reads: usize| {
            let (mut package, list) = fixed_list_read_package(element, reads, "511");
            if local_index {
                use_local_list_index(&mut package);
            }
            let argument = if element == P::Type {
                Value::TypeList(vec![ty(P::Bool); 512])
            } else {
                Value::List {
                    ty: list,
                    values: vec![Value::Bool(true); 512],
                }
            };
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let before = vm.remaining_steps();
            let mut arguments = vec![argument];
            if local_index {
                arguments.push(Value::Integer(comptime_runtime::Integer::from_u128(511)));
            }
            let result = vm.invoke(id(3), arguments).unwrap();
            assert_eq!(
                result,
                if element == P::Type {
                    Value::Type(ty(P::Bool))
                } else {
                    Value::Bool(true)
                }
            );
            before - vm.remaining_steps()
        };
        let single = run(1);
        let repeated = run(64);
        assert!(repeated > single);
        assert!(
            repeated - single < 2_000,
            "{element:?}: {single} -> {repeated}"
        );
    }
}

fn fixed_list_read_package(element: P, reads: usize, spelling: &str) -> (Package, Term) {
    use nir::program::Literal;
    let mut package = package();
    package.normal_forms.truncate(1);
    package.normal_forms[0].role = nir::normal_forms::Role::TypeList;
    let list = Term::NormalForm {
        head: id(1),
        sort: Sort::Type,
        arguments: vec![ty(element)],
    };
    let function = &mut package.functions[0];
    let source = function.body.source;
    function.signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::ComptimeValue,
            ty: list.clone(),
        }],
        result: Box::new(ty(element)),
    };
    let read = Expr {
        source,
        ty: ty(element),
        kind: K::TypeListIndex(
            Box::new(Expr {
                source,
                ty: list.clone(),
                kind: K::Local(function.parameters[0]),
            }),
            Box::new(Expr {
                source,
                ty: ty(P::Integer),
                kind: K::Literal(Literal::Integer(spelling.into())),
            }),
        ),
    };
    function.body = Expr {
        source,
        ty: ty(element),
        kind: K::Block {
            statements: vec![read.clone(); reads - 1],
            result: Some(Box::new(read)),
        },
    };
    (package, list)
}

#[test]
fn fixed_list_index_rejects_bounds_and_recovers_without_mutating_the_list() {
    use comptime_interpretor::ErrorKind;
    for element in [P::Bool, P::Type] {
        for local_index in [false, true] {
            for spelling in ["-1", "1", "999999999999999999999999999999999999"] {
                let (mut package, list) = fixed_list_read_package(element, 1, spelling);
                if local_index {
                    use_local_list_index(&mut package);
                }
                let (good, _) = fixed_list_read_package(element, 1, "0");
                let mut recovery = good.functions[0].clone();
                recovery.declaration = id(4);
                package.functions.push(recovery);
                let argument = if element == P::Type {
                    Value::TypeList(vec![ty(P::Bool)])
                } else {
                    Value::List {
                        ty: list,
                        values: vec![Value::Bool(true)],
                    }
                };
                let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                let mut arguments = vec![argument.clone()];
                if local_index {
                    let mut position = comptime_runtime::Integer::parse_literal(
                        spelling.trim_start_matches('-'),
                        &mut Default::default(),
                    )
                    .unwrap();
                    if spelling.starts_with('-') {
                        position = position.negated();
                    }
                    arguments.push(Value::Integer(position));
                }
                assert_eq!(
                    vm.invoke(id(3), arguments).unwrap_err().kind,
                    ErrorKind::IndexOutOfBounds,
                    "{element:?} [{spelling}]"
                );
                let expected = if element == P::Type {
                    Value::Type(ty(P::Bool))
                } else {
                    Value::Bool(true)
                };
                assert_eq!(vm.invoke(id(4), vec![argument]).unwrap(), expected);
            }
        }
    }
}

fn use_local_list_index(package: &mut Package) {
    let function = &mut package.functions[0];
    let index = LocalId { owner: 3, index: 1 };
    function.parameters.push(index);
    let Term::Function { parameters, .. } = &mut function.signature else {
        unreachable!()
    };
    parameters.push(Parameter {
        sort: Sort::ComptimeValue,
        ty: ty(P::Integer),
    });
    let K::Block { statements, result } = &mut function.body.kind else {
        unreachable!()
    };
    for read in statements
        .iter_mut()
        .chain(result.iter_mut().map(Box::as_mut))
    {
        let K::TypeListIndex(_, operand) = &mut read.kind else {
            unreachable!()
        };
        operand.kind = K::Local(index);
    }
}

fn list_operation_package(element: P, concat: bool) -> (Package, Term) {
    let (mut package, list) = fixed_list_read_package(element, 1, "0");
    let function = &mut package.functions[0];
    let source = function.body.source;
    let operand = Expr {
        source,
        ty: list.clone(),
        kind: K::Local(function.parameters[0]),
    };
    let result = if concat { list.clone() } else { ty(P::Integer) };
    let Term::Function { result: output, .. } = &mut function.signature else {
        unreachable!()
    };
    *output = Box::new(result.clone());
    function.body = Expr {
        source,
        ty: result,
        kind: if concat {
            K::ListConcat(Box::new(operand.clone()), Box::new(operand))
        } else {
            K::ListLength(Box::new(operand))
        },
    };
    (package, list)
}

#[test]
fn immutable_list_operations_preserve_types_and_enforce_combined_size_limit() {
    use comptime_interpretor::ErrorKind;
    for element in [P::Bool, P::Type] {
        let (mut package, list) = list_operation_package(element, true);
        let (length, _) = list_operation_package(element, false);
        let mut length = length.functions[0].clone();
        length.declaration = id(4);
        package.functions.push(length);
        let argument = |count| {
            if element == P::Type {
                Value::TypeList(vec![ty(P::Bool); count])
            } else {
                Value::List {
                    ty: list.clone(),
                    values: vec![Value::Bool(true); count],
                }
            }
        };
        let mut vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 10_000_000,
                ..Default::default()
            },
        )
        .unwrap();
        for count in [0, 1, 20_000] {
            assert_eq!(
                vm.invoke(id(3), vec![argument(count)]).unwrap(),
                argument(count * 2)
            );
            assert_eq!(
                vm.invoke(id(4), vec![argument(count)]).unwrap(),
                Value::Integer(comptime_runtime::Integer::from_u128(count as u128))
            );
        }
        assert_eq!(
            vm.invoke(
                id(3),
                vec![argument(if element == P::Type { 30_000 } else { 50_001 })]
            )
            .unwrap_err()
            .kind,
            ErrorKind::RepresentationSizeLimit
        );
        assert_eq!(
            vm.invoke(id(4), vec![argument(1)]).unwrap(),
            Value::Integer(comptime_runtime::Integer::from_u128(1))
        );
    }
}

#[test]
fn list_operations_reject_forged_output_domains_and_honor_vm_budget() {
    use comptime_interpretor::ErrorKind;
    for concat in [false, true] {
        let (mut package, list) = list_operation_package(P::Bool, concat);
        let argument = Value::List {
            ty: list,
            values: vec![Value::Bool(true); 32],
        };
        let mut vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 32,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            vm.invoke(id(3), vec![argument.clone()]).unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        let mut malformed = package.functions[0].clone();
        malformed.declaration = id(4);
        malformed.body.ty = ty(P::Bool);
        package.functions.push(malformed);
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(4), vec![argument.clone()]).unwrap_err().kind,
            ErrorKind::Type
        );
        assert!(vm.invoke(id(3), vec![argument]).is_ok());
    }
}

fn repeated_list_lengths(
    element: P,
    reads: usize,
    storage: bool,
    general: bool,
) -> (Package, Term) {
    let (mut package, list) = list_operation_package(element, false);
    let function = &mut package.functions[0];
    let source = function.body.source;
    let mut read = function.body.clone();
    if general {
        let K::ListLength(value) = &mut read.kind else {
            unreachable!()
        };
        **value = Expr {
            source,
            ty: list.clone(),
            kind: K::Block {
                statements: vec![],
                result: Some(value.clone()),
            },
        };
    }
    let mut statements = Vec::new();
    if storage {
        statements.push(Expr {
            source,
            ty: Term::Pointer(Box::new(list.clone())),
            kind: K::AddressOf(Box::new(Expr {
                source,
                ty: list.clone(),
                kind: K::Local(function.parameters[0]),
            })),
        });
    }
    statements.extend(vec![read.clone(); reads - 1]);
    function.body = Expr {
        source,
        ty: ty(P::Integer),
        kind: K::Block {
            statements,
            result: Some(Box::new(read)),
        },
    };
    (package, list)
}

#[test]
fn repeated_list_lengths_charge_the_observation_not_every_element() {
    for element in [P::Bool, P::Type] {
        for storage in [false, true] {
            let run = |length, reads, general| {
                let (package, list) = repeated_list_lengths(element, reads, storage, general);
                let argument = if element == P::Type {
                    Value::TypeList(vec![ty(P::Bool); length])
                } else {
                    Value::List {
                        ty: list,
                        values: vec![Value::Bool(true); length],
                    }
                };
                let mut vm = Interpreter::for_comptime(
                    &package,
                    Budget {
                        steps: 10_000_000,
                        ..Default::default()
                    },
                )
                .unwrap();
                let before = vm.remaining_steps();
                assert_eq!(
                    vm.invoke(id(3), vec![argument]).unwrap(),
                    Value::Integer(comptime_runtime::Integer::from_u128(length as u128))
                );
                before - vm.remaining_steps()
            };
            let small = run(1, 64, false) - run(1, 1, false);
            let large = run(4096, 64, false) - run(4096, 1, false);
            assert_eq!(small, large, "{element:?} storage={storage}");
            assert!(large > 0 && large < 1000, "{large}");
            assert!(run(4096, 64, true) > run(4096, 64, false));
            assert!(run(0, 64, false) > 0);
        }
    }
}

#[test]
fn list_length_does_not_accept_a_local_with_a_different_element_domain() {
    use comptime_interpretor::ErrorKind;
    for element in [P::Bool, P::Type] {
        for general in [false, true] {
            let (mut package, list) = list_operation_package(element, false);
            let mut malformed = package.functions[0].clone();
            malformed.declaration = id(4);
            let K::ListLength(operand) = &mut malformed.body.kind else {
                unreachable!()
            };
            let forged = Term::NormalForm {
                head: id(1),
                sort: Sort::Type,
                arguments: vec![ty(P::Char)],
            };
            operand.ty = forged.clone();
            if general {
                **operand = Expr {
                    source: operand.source,
                    ty: forged,
                    kind: K::Block {
                        statements: vec![],
                        result: Some(operand.clone()),
                    },
                };
            }
            package.functions.push(malformed);
            let argument = if element == P::Type {
                Value::TypeList(vec![ty(P::Bool)])
            } else {
                Value::List {
                    ty: list,
                    values: vec![Value::Bool(true)],
                }
            };
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke(id(4), vec![argument.clone()]).unwrap_err().kind,
                ErrorKind::Type
            );
            assert_eq!(
                vm.invoke(id(3), vec![argument]).unwrap(),
                Value::Integer(comptime_runtime::Integer::from_u128(1))
            );
        }
    }
}

#[test]
fn list_length_observes_replacement_and_runs_general_operand_effects() {
    use nir::program::{Literal, Place};
    for element in [P::Bool, P::Type] {
        for storage in [false, true] {
            for general in [false, true] {
                let (mut package, list) = repeated_list_lengths(element, 1, storage, false);
                let function = &mut package.functions[0];
                let source = function.body.source;
                let replacement = Expr {
                    source,
                    ty: list.clone(),
                    kind: if element == P::Type {
                        K::TypeListValue(vec![ty(P::Bool), ty(P::Char)])
                    } else {
                        K::TypeList(vec![
                            Expr {
                                source,
                                ty: ty(P::Bool),
                                kind: K::Literal(Literal::Bool(false))
                            };
                            2
                        ])
                    },
                };
                let assignment = Expr {
                    source,
                    ty: ty(P::Void),
                    kind: K::Assign {
                        place: Place {
                            root: function.parameters[0],
                            fields: vec![],
                        },
                        value: Box::new(replacement),
                    },
                };
                let K::Block {
                    statements,
                    result: Some(result),
                } = &mut function.body.kind
                else {
                    unreachable!()
                };
                if general {
                    let K::ListLength(operand) = &mut result.kind else {
                        unreachable!()
                    };
                    **operand = Expr {
                        source,
                        ty: list.clone(),
                        kind: K::Block {
                            statements: vec![assignment],
                            result: Some(operand.clone()),
                        },
                    };
                } else {
                    statements.push(assignment);
                }
                let argument = if element == P::Type {
                    Value::TypeList(vec![ty(P::Bool)])
                } else {
                    Value::List {
                        ty: list,
                        values: vec![Value::Bool(true)],
                    }
                };
                let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                assert_eq!(
                    vm.invoke(id(3), vec![argument]).unwrap(),
                    Value::Integer(comptime_runtime::Integer::from_u128(2))
                );
            }
        }
    }
}

#[test]
fn local_list_length_still_charges_large_element_type_descriptions() {
    let run = |width, reads| {
        let (mut package, _) = repeated_list_lengths(P::Bool, reads, false, false);
        let element = Term::Tuple(vec![ty(P::Bool); width]);
        let list = Term::NormalForm {
            head: id(1),
            sort: Sort::Type,
            arguments: vec![element],
        };
        let function = &mut package.functions[0];
        let Term::Function { parameters, .. } = &mut function.signature else {
            unreachable!()
        };
        parameters[0].ty = list.clone();
        let K::Block { statements, result } = &mut function.body.kind else {
            unreachable!()
        };
        for read in statements
            .iter_mut()
            .chain(result.iter_mut().map(Box::as_mut))
        {
            let K::ListLength(operand) = &mut read.kind else {
                unreachable!()
            };
            operand.ty = list.clone();
        }
        let mut vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 10_000_000,
                ..Default::default()
            },
        )
        .unwrap();
        let before = vm.remaining_steps();
        assert_eq!(
            vm.invoke(
                id(3),
                vec![Value::List {
                    ty: list,
                    values: vec![]
                }]
            )
            .unwrap(),
            Value::Integer(comptime_runtime::Integer::from_u128(0))
        );
        before - vm.remaining_steps()
    };
    let small = run(1, 64) - run(1, 1);
    let large = run(4096, 64) - run(4096, 1);
    assert!(large > small + 200_000, "{small} -> {large}");
}

fn list_pointer_package(element: P) -> (Package, Term, Term) {
    let (mut package, list) = fixed_list_read_package(element, 1, "0");
    let pointer = Term::Pointer(Box::new(list.clone()));
    let function = &mut package.functions[0];
    let source = function.body.source;
    let input = function.parameters[0];
    let node = |ty, kind| Expr { source, ty, kind };
    function.signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: list.clone(),
        }],
        result: Box::new(pointer.clone()),
    };
    function.body = node(
        pointer.clone(),
        K::AddressOf(Box::new(node(list.clone(), K::Local(input)))),
    );
    let mut reader = function.clone();
    reader.declaration = id(4);
    reader.signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: pointer.clone(),
        }],
        result: Box::new(list.clone()),
    };
    reader.body = node(
        list.clone(),
        K::ReadPointer(Box::new(node(pointer.clone(), K::Local(input)))),
    );
    let mut writer = reader.clone();
    writer.declaration = id(5);
    let replacement = LocalId { owner: 3, index: 1 };
    writer.parameters.push(replacement);
    writer.signature = Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Runtime,
                ty: pointer.clone(),
            },
            Parameter {
                sort: Sort::ComptimeValue,
                ty: list.clone(),
            },
        ],
        result: Box::new(ty(P::Void)),
    };
    writer.body = node(
        ty(P::Void),
        K::WritePointer {
            pointer: Box::new(node(pointer.clone(), K::Local(input))),
            value: Box::new(node(list.clone(), K::Local(replacement))),
        },
    );
    package.functions.extend([reader, writer]);
    (package, list, pointer)
}

#[test]
fn comptime_list_pointer_writes_are_atomic_under_budget_exhaustion() {
    use comptime_interpretor::{ErrorKind, Storage};
    for element in [P::Bool, P::Type] {
        let (package, list, pointer_ty) = list_pointer_package(element);
        let value = |count| {
            if element == P::Type {
                Value::TypeList(vec![ty(P::Bool); count])
            } else {
                Value::List {
                    ty: list.clone(),
                    values: vec![Value::Bool(true); count],
                }
            }
        };
        let mut failures = 0;
        let mut successes = 0;
        for steps in (1..2000).step_by(7) {
            let original = value(2);
            let replacement = value(8);
            let owner = Storage::new(original.clone());
            let mut setup = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let pointer = setup
                .invoke_with_typed_values(
                    id(3),
                    vec![(list.clone(), Value::Storage(owner.clone()))],
                )
                .unwrap();
            let result = Interpreter::for_comptime(
                &package,
                Budget {
                    steps,
                    ..Budget::default()
                },
            )
            .and_then(|mut vm| {
                vm.invoke_with_typed_values(
                    id(5),
                    vec![
                        (pointer_ty.clone(), pointer.clone()),
                        (list.clone(), replacement.clone()),
                    ],
                )
            });
            match result {
                Ok(Value::Unit) => {
                    successes += 1;
                    assert_eq!(owner.read().unwrap(), replacement);
                }
                Err(error) => {
                    failures += 1;
                    assert_eq!(error.kind, ErrorKind::StepsExhausted);
                    assert_eq!(owner.read().unwrap(), original);
                }
                other => panic!("unexpected write result: {other:?}"),
            }
            let snapshot = setup
                .invoke_with_typed_values(id(4), vec![(pointer_ty.clone(), pointer)])
                .unwrap();
            assert_eq!(snapshot, owner.read().unwrap());
        }
        assert!(failures > 0 && successes > 0);
    }
}

#[test]
fn list_pointer_access_does_not_grant_runtime_copy_permission() {
    use comptime_interpretor::{ErrorKind, Storage};
    for element in [P::Bool, P::Type] {
        let (mut package, list, pointer_ty) = list_pointer_package(element);
        let original = if element == P::Type {
            Value::TypeList(vec![])
        } else {
            Value::List {
                ty: list.clone(),
                values: vec![],
            }
        };
        let owner = Storage::new(original.clone());
        let pointer = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .invoke_with_typed_values(id(3), vec![(list.clone(), Value::Storage(owner.clone()))])
            .unwrap();
        for function in &mut package.functions {
            function.phase = FunctionPhase::Runtime;
            if let Term::Function { parameters, .. } = &mut function.signature {
                for parameter in parameters {
                    parameter.sort = Sort::Runtime;
                }
            }
        }
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke_with_typed_values(id(4), vec![(pointer_ty.clone(), pointer.clone())])
                .unwrap_err()
                .kind,
            ErrorKind::Unsupported
        );
        assert_eq!(
            vm.invoke_with_typed_values(
                id(5),
                vec![(pointer_ty, pointer), (list, original.clone())]
            )
            .unwrap_err()
            .kind,
            // Generic lists are rejected at runtime argument admission;
            // the specialized type-list representation reaches the copy gate.
            if element == P::Type {
                ErrorKind::Unsupported
            } else {
                ErrorKind::Type
            }
        );
        assert_eq!(owner.read().unwrap(), original);
    }
}

#[test]
fn list_pointer_rejects_wrong_element_and_identity_without_modifying_storage() {
    use comptime_interpretor::{ErrorKind, Storage};
    let (package, list, pointer_ty) = list_pointer_package(P::Bool);
    let original = Value::List {
        ty: list.clone(),
        values: vec![Value::Bool(true)],
    };
    let owner = Storage::new(original.clone());
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let pointer = vm
        .invoke_with_typed_values(id(3), vec![(list.clone(), Value::Storage(owner.clone()))])
        .unwrap();
    for invalid in [
        Value::TypeList(vec![ty(P::Bool)]),
        Value::List {
            ty: list.clone(),
            values: vec![Value::Type(ty(P::Bool))],
        },
        Value::List {
            ty: list_type(),
            values: vec![Value::Bool(false)],
        },
        Value::List {
            ty: list.clone(),
            values: vec![pointer.clone()],
        },
    ] {
        assert_eq!(
            vm.invoke_with_typed_values(
                id(5),
                vec![
                    (pointer_ty.clone(), pointer.clone()),
                    (list.clone(), invalid)
                ]
            )
            .unwrap_err()
            .kind,
            ErrorKind::Type
        );
        assert_eq!(owner.read().unwrap(), original);
        assert_eq!(
            vm.invoke_with_typed_values(id(4), vec![(pointer_ty.clone(), pointer.clone())])
                .unwrap(),
            original
        );
    }
}

fn list_append_package(element: P, general: bool) -> (Package, Term, Term) {
    let (mut package, list, pointer) = list_pointer_package(element);
    let writer = &mut package.functions[2];
    let source = writer.body.source;
    let node = |ty, kind| Expr { source, ty, kind };
    let Term::Function { parameters, .. } = &mut writer.signature else {
        unreachable!()
    };
    parameters[1].ty = ty(element);
    let local = node(pointer.clone(), K::Local(writer.parameters[0]));
    let suffix = node(
        list.clone(),
        K::TypeList(vec![node(ty(element), K::Local(writer.parameters[1]))]),
    );
    let suffix = if general {
        node(
            list.clone(),
            K::Block {
                statements: vec![],
                result: Some(Box::new(suffix)),
            },
        )
    } else {
        suffix
    };
    writer.body = node(
        ty(P::Void),
        K::WritePointer {
            pointer: Box::new(local.clone()),
            value: Box::new(node(
                list.clone(),
                K::ListConcat(
                    Box::new(node(list.clone(), K::ReadPointer(Box::new(local)))),
                    Box::new(suffix),
                ),
            )),
        },
    );
    (package, list, pointer)
}

#[test]
fn list_append_failure_rolls_back_and_success_preserves_snapshots() {
    use comptime_interpretor::{ErrorKind, Storage};
    for element in [P::Bool, P::Type] {
        for general in [false, true] {
            let (package, list, pointer_ty) = list_append_package(element, general);
            let original = if element == P::Type {
                Value::TypeList(vec![ty(P::Bool); 4])
            } else {
                Value::List {
                    ty: list.clone(),
                    values: vec![Value::Bool(true); 4],
                }
            };
            let item = if element == P::Type {
                Value::Type(ty(P::Char))
            } else {
                Value::Bool(false)
            };
            let expected = if element == P::Type {
                Value::TypeList(vec![
                    ty(P::Bool),
                    ty(P::Bool),
                    ty(P::Bool),
                    ty(P::Bool),
                    ty(P::Char),
                ])
            } else {
                Value::List {
                    ty: list.clone(),
                    values: vec![
                        Value::Bool(true),
                        Value::Bool(true),
                        Value::Bool(true),
                        Value::Bool(true),
                        Value::Bool(false),
                    ],
                }
            };
            let mut successes = 0;
            let mut failures = 0;
            for steps in 1..600 {
                let owner = Storage::new(original.clone());
                let mut setup = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                let pointer = setup
                    .invoke_with_typed_values(
                        id(3),
                        vec![(list.clone(), Value::Storage(owner.clone()))],
                    )
                    .unwrap();
                let snapshot = setup
                    .invoke_with_typed_values(id(4), vec![(pointer_ty.clone(), pointer.clone())])
                    .unwrap();
                let result = Interpreter::for_comptime(
                    &package,
                    Budget {
                        steps,
                        ..Budget::default()
                    },
                )
                .and_then(|mut vm| {
                    vm.invoke_with_typed_values(
                        id(5),
                        vec![
                            (pointer_ty.clone(), pointer.clone()),
                            (ty(element), item.clone()),
                        ],
                    )
                });
                match result {
                    Ok(Value::Unit) => {
                        successes += 1;
                        assert_eq!(owner.read().unwrap(), expected);
                    }
                    Err(error) => {
                        failures += 1;
                        assert_eq!(error.kind, ErrorKind::StepsExhausted);
                        assert_eq!(owner.read().unwrap(), original);
                    }
                    other => panic!("unexpected result {other:?}"),
                }
                assert_eq!(snapshot, original);
            }
            assert!(successes > 0 && failures > 0);
        }
    }
}

#[test]
fn list_append_representation_limit_preserves_old_storage_and_recovers() {
    use comptime_interpretor::{ErrorKind, Storage};
    let (package, list, pointer_ty) = list_append_package(P::Type, false);
    let original = Value::TypeList(vec![ty(P::Bool); 49_999]);
    let owner = Storage::new(original.clone());
    let mut vm = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 10_000_000,
            ..Budget::default()
        },
    )
    .unwrap();
    let pointer = vm
        .invoke_with_typed_values(id(3), vec![(list.clone(), Value::Storage(owner.clone()))])
        .unwrap();
    assert_eq!(
        vm.invoke_with_typed_values(
            id(5),
            vec![
                (pointer_ty.clone(), pointer.clone()),
                (ty(P::Type), Value::Type(ty(P::Char)))
            ]
        )
        .unwrap_err()
        .kind,
        ErrorKind::RepresentationSizeLimit
    );
    assert_eq!(owner.read().unwrap(), original);
    assert_eq!(
        vm.invoke_with_typed_values(id(4), vec![(pointer_ty, pointer)])
            .unwrap(),
        original
    );
}
