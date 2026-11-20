use comptime_interpretor::{
    Budget, ErrorKind, Interpreter, Object, ObjectEntry, ObjectValue, Storage, Value,
};
use nir::intrinsics::{Operation as IntrinsicOp, Registry, Tag};
use nir::program::{
    Capture, DescriptionLiteral, Enum, Expr, ExprKind as K, Function, FunctionDeclaration,
    FunctionPhase, InterfaceInstance, Literal, LocalId, MatchArm, ObjectEditKind, ObjectQueryKind,
    Package, Pattern, SourceRange, TypePredicate, Variant, VariantShape,
};
use nir::{
    DeclarationId, DeclarationParameter, Parameter, Primitive as P, Sort, Term,
    TraitMethodIdentity, TypeKind,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const SRC: SourceRange = SourceRange { start: 1, end: 2 };

fn decl(local: u32) -> DeclarationId {
    DeclarationId { package: 77, local }
}

fn local(index: u32) -> LocalId {
    LocalId { owner: 1, index }
}

fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SRC,
    }
}

fn p(prim: P) -> Term {
    Term::Primitive(prim)
}

fn comptime_fn(
    declaration: DeclarationId,
    parameters: Vec<(LocalId, Parameter)>,
    result: Term,
    body: Expr,
) -> Function {
    let param_ids = parameters.iter().map(|(id, _)| *id).collect();
    let param_terms = parameters.into_iter().map(|(_, p)| p).collect();
    Function {
        interface: FunctionDeclaration {
            source: SRC,
            declaration,
            name: format!("fn_{}", declaration.local),
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: true,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            result_type_binder: None,
            result_guarantees: vec![],
            entry_snapshots: vec![],
            type_parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
            signature: Term::Function {
                parameters: param_terms,
                result: Box::new(result),
            },
            parameters: param_ids,
            env_parameters: vec![],
            defaults: vec![],
        },
        body,
        runtime_deliveries: Default::default(),
    }
}

fn run_differential(
    package: &Package,
    entry: DeclarationId,
    args: Vec<Value>,
) -> (Result<Value, ErrorKind>, Result<Value, ErrorKind>) {
    let mut byte_vm = Interpreter::for_comptime(package, Budget::default()).unwrap();
    let byte_res = byte_vm.invoke(entry, args.clone()).map_err(|e| e.kind);
    let mut tree_vm = Interpreter::for_comptime_tree(package, Budget::default()).unwrap();
    let tree_res = tree_vm.invoke(entry, args).map_err(|e| e.kind);
    (byte_res, tree_res)
}

#[test]
fn type_kind_over_description_literal_matches_between_bytecode_and_tree() {
    let mut package = Package::default();
    let binder = DeclarationParameter {
        declaration: DeclarationId {
            package: u64::MAX,
            local: 0,
        },
        sort: Sort::Type,
    };
    let subject = Term::Rigid {
        declaration: binder.declaration,
        sort: Sort::Type,
    };
    let mut steps = 10_000;
    let signed_and_not_tuple = package
        .refinements
        .publish(
            DescriptionLiteral::Any,
            binder,
            TypePredicate::And(vec![
                TypePredicate::Kind(subject.clone(), TypeKind::SignedInteger),
                TypePredicate::Not(Box::new(TypePredicate::Kind(
                    subject.clone(),
                    TypeKind::Tuple,
                ))),
            ]),
            SRC,
            &mut steps,
        )
        .unwrap();
    let base_integer = package
        .refinements
        .publish(
            DescriptionLiteral::Any,
            binder,
            TypePredicate::Kind(subject, TypeKind::Integer),
            SRC,
            &mut steps,
        )
        .unwrap();

    let arg = local(0);
    let check_kind = |fn_local, kind| {
        comptime_fn(
            decl(fn_local),
            vec![(
                arg,
                Parameter {
                    sort: Sort::Runtime,
                    ty: p(P::Type),
                },
            )],
            p(P::Bool),
            expr(
                p(P::Bool),
                K::TypeKind {
                    kind,
                    operand: Box::new(expr(p(P::Type), K::Local(arg))),
                },
            ),
        )
    };
    package.functions.push(check_kind(1, TypeKind::Integer));
    package.functions.push(check_kind(2, TypeKind::Boolean));
    package
        .functions
        .push(check_kind(3, TypeKind::SignedInteger));

    for make in [Interpreter::for_comptime, Interpreter::for_comptime_tree] {
        let mut vm = make(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(
                decl(1),
                vec![Value::Description(signed_and_not_tuple.clone())]
            )
            .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            vm.invoke(
                decl(2),
                vec![Value::Description(signed_and_not_tuple.clone())]
            )
            .unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            vm.invoke(decl(3), vec![Value::Description(base_integer.clone())])
                .unwrap_err()
                .kind,
            ErrorKind::TypeContractUnresolved
        );
    }
}

#[test]
fn returned_descriptions_prove_only_their_entailed_kind_guarantees() {
    let mut package = Package::default();
    let result_id = decl(8);
    let result = Term::Rigid {
        declaration: result_id,
        sort: Sort::Type,
    };
    let binder = DeclarationParameter {
        declaration: decl(9),
        sort: Sort::Type,
    };
    let itself = Term::Rigid {
        declaration: binder.declaration,
        sort: Sort::Type,
    };
    let mut steps = 10_000;
    let mut description = |requirement| {
        package
            .refinements
            .publish(
                DescriptionLiteral::Any,
                binder,
                requirement,
                SRC,
                &mut steps,
            )
            .unwrap()
    };
    let boolean = TypePredicate::Kind(itself.clone(), TypeKind::Boolean);
    let integer = TypePredicate::Kind(itself, TypeKind::SignedInteger);
    let unrelated = description(boolean.clone());
    let either = description(TypePredicate::Or(vec![integer.clone(), boolean]));
    let signed = description(TypePredicate::And(vec![TypePredicate::Bool(true), integer]));
    let arg = local(0);
    let mut producer = comptime_fn(
        decl(1),
        vec![(
            arg,
            Parameter {
                sort: Sort::Runtime,
                ty: p(P::Type),
            },
        )],
        p(P::Type),
        expr(p(P::Type), K::Local(arg)),
    );
    producer.result_type_binder = Some(result_id);
    producer.result_guarantees = vec![
        TypePredicate::Kind(result.clone(), TypeKind::Integer),
        TypePredicate::Not(Box::new(TypePredicate::Kind(result, TypeKind::Boolean))),
    ];
    package.functions.push(producer);
    for make in [Interpreter::for_comptime, Interpreter::for_comptime_tree] {
        let mut vm = make(&package, Budget::default()).unwrap();
        for rejected in [&unrelated, &either, &DescriptionLiteral::Any] {
            let actual = vm.invoke(decl(1), vec![Value::Description(rejected.clone())]);
            assert_eq!(
                actual.map_err(|error| error.kind),
                Err(ErrorKind::ProgramContractRejected)
            );
        }
        let actual = vm
            .invoke(decl(1), vec![Value::Description(signed.clone())])
            .unwrap();
        assert_eq!(actual, Value::Description(signed.clone()));
    }
}

#[test]
fn logical_and_or_rejects_bad_rhs_when_not_short_circuited_and_handles_never_lhs() {
    let failing_rhs_fn = comptime_fn(
        decl(4),
        vec![],
        p(P::Bool),
        expr(
            p(P::Bool),
            K::TypeEqual(
                Box::new(expr(p(P::Type), K::TypeValue(p(P::Bool)))),
                Box::new(expr(p(P::Type), K::TypeValue(p(P::U8)))),
            ),
        ),
    );
    let call_failing_rhs = expr(
        p(P::Bool),
        K::Call {
            callee: Box::new(expr(
                Term::ComptimeFunction(Box::new(Term::Function {
                    parameters: vec![],
                    result: Box::new(p(P::Bool)),
                })),
                K::Function(decl(4)),
            )),
            arguments: vec![],
            parameter_bindings: vec![],
            env_arguments: vec![],
        },
    );
    let lhs_param = local(0);
    let make_logic_fn = |id, is_and| {
        let node = if is_and {
            K::And(
                Box::new(expr(p(P::Bool), K::Local(lhs_param))),
                Box::new(call_failing_rhs.clone()),
            )
        } else {
            K::Or(
                Box::new(expr(p(P::Bool), K::Local(lhs_param))),
                Box::new(call_failing_rhs.clone()),
            )
        };
        comptime_fn(
            decl(id),
            vec![(
                lhs_param,
                Parameter {
                    sort: Sort::Runtime,
                    ty: p(P::Bool),
                },
            )],
            p(P::Bool),
            expr(p(P::Bool), node),
        )
    };
    // Never LHS: `return true` before RHS.
    let never_lhs = expr(
        p(P::Never),
        K::Return(Some(Box::new(expr(
            p(P::Bool),
            K::Literal(Literal::Bool(true)),
        )))),
    );
    let never_fn = comptime_fn(
        decl(3),
        vec![],
        p(P::Bool),
        expr(
            p(P::Bool),
            K::And(
                Box::new(never_lhs),
                Box::new(expr(p(P::Bool), K::Literal(Literal::Bool(false)))),
            ),
        ),
    );
    let package = Package {
        functions: vec![
            make_logic_fn(1, true),
            make_logic_fn(2, false),
            never_fn,
            failing_rhs_fn,
        ],
        ..Default::default()
    };

    // Short-circuited cases pass.
    assert_eq!(
        run_differential(&package, decl(1), vec![Value::Bool(false)]),
        (Ok(Value::Bool(false)), Ok(Value::Bool(false)))
    );
    assert_eq!(
        run_differential(&package, decl(2), vec![Value::Bool(true)]),
        (Ok(Value::Bool(true)), Ok(Value::Bool(true)))
    );
    // Evaluated RHS executes identically in bytecode and tree!
    assert_eq!(
        run_differential(&package, decl(1), vec![Value::Bool(true)]),
        (
            Err(ErrorKind::TypeContractUnresolved),
            Err(ErrorKind::TypeContractUnresolved)
        )
    );
    assert_eq!(
        run_differential(&package, decl(2), vec![Value::Bool(false)]),
        (
            Err(ErrorKind::TypeContractUnresolved),
            Err(ErrorKind::TypeContractUnresolved)
        )
    );
    // Never LHS compiles and short-circuits via early return identically.
    assert_eq!(
        run_differential(&package, decl(3), vec![]),
        (Ok(Value::Bool(true)), Ok(Value::Bool(true)))
    );
}

#[test]
fn pattern_remap_preserves_shadowed_outer_binding_in_bytecode_and_tree() {
    let x = local(0);
    let y = local(1);
    let outer_let = expr(
        p(P::Unit),
        K::Let {
            binding: x,
            value: Box::new(expr(p(P::Bool), K::Literal(Literal::Bool(true)))),
        },
    );
    let scrutinee = expr(
        Term::Tuple(vec![p(P::Bool), p(P::Char)]),
        K::Tuple(vec![
            expr(p(P::Bool), K::Literal(Literal::Bool(false))),
            expr(p(P::Char), K::Literal(Literal::Char('z'))),
        ]),
    );
    let remap_pat = Pattern::Remap {
        pattern: Box::new(Pattern::Tuple(vec![
            Pattern::Wild,
            Pattern::Binding {
                binding: x,
                inner: Box::new(Pattern::Wild),
            },
        ])),
        bindings: vec![(x, y)],
    };
    let arm_body = expr(
        Term::Tuple(vec![p(P::Bool), p(P::Char)]),
        K::Tuple(vec![
            expr(p(P::Bool), K::Local(x)),
            expr(p(P::Char), K::Local(y)),
        ]),
    );
    let match_expr = expr(
        Term::Tuple(vec![p(P::Bool), p(P::Char)]),
        K::Match {
            value: Box::new(scrutinee),
            arms: vec![MatchArm {
                pattern: remap_pat,
                guard: None,
                body: arm_body,
            }],
        },
    );
    let a = local(2);
    let b = local(3);
    let unpack = expr(
        p(P::Unit),
        K::LetPattern {
            pattern: Pattern::Tuple(vec![
                Pattern::Binding {
                    binding: a,
                    inner: Box::new(Pattern::Wild),
                },
                Pattern::Binding {
                    binding: b,
                    inner: Box::new(Pattern::Wild),
                },
            ]),
            value: Box::new(match_expr),
            guard: None,
            otherwise: None,
        },
    );
    let body = expr(
        Term::Tuple(vec![p(P::Bool), p(P::Char), p(P::Bool)]),
        K::Block {
            statements: vec![outer_let, unpack],
            result: Some(Box::new(expr(
                Term::Tuple(vec![p(P::Bool), p(P::Char), p(P::Bool)]),
                K::Tuple(vec![
                    expr(p(P::Bool), K::Local(a)),
                    expr(p(P::Char), K::Local(b)),
                    expr(p(P::Bool), K::Local(x)),
                ]),
            ))),
        },
    );
    let package = Package {
        functions: vec![comptime_fn(
            decl(1),
            vec![],
            Term::Tuple(vec![p(P::Bool), p(P::Char), p(P::Bool)]),
            body,
        )],
        ..Default::default()
    };
    let expected = Value::Tuple(vec![Value::Bool(true), Value::Char('z'), Value::Bool(true)]);
    assert_eq!(
        run_differential(&package, decl(1), vec![]),
        (Ok(expected.clone()), Ok(expected))
    );
}

#[test]
fn bit_write_pointer_rejected_at_comptime_and_object_query_releases_lock_before_closure_walk() {
    // 1. BitWritePointer in comptime body must never execute/succeed in either engine.
    let bv4 = p(P::BitVec(4));
    let bv2 = p(P::BitVec(2));
    let ptr_bv4 = Term::Pointer(Box::new(bv4.clone()));
    let bit_write_fn = comptime_fn(
        decl(1),
        vec![],
        p(P::Void),
        expr(
            p(P::Void),
            K::BitWritePointer {
                pointer: Box::new(expr(
                    ptr_bv4,
                    K::Return(Some(Box::new(expr(p(P::Void), K::Unit)))),
                )),
                selections: vec![nir::program::BitSelection::Static { start: 0, end: 2 }],
                value: Box::new(expr(
                    bv2,
                    K::Literal(Literal::BitVec {
                        width: 2,
                        words: vec![1],
                    }),
                )),
            },
        ),
    );
    let pkg = Package {
        functions: vec![bit_write_fn],
        ..Default::default()
    };
    for make in [Interpreter::for_comptime, Interpreter::for_comptime_tree] {
        let err = make(&pkg, Budget::default())
            .err()
            .expect("comptime package with BitWritePointer must fail phase validation");
        assert!(matches!(err.kind, ErrorKind::InvalidPhase(_)), "{err:?}");
    }

    // 2. Object stored in a Storage containing a Closure (capturing another Storage)
    //    and List stored in a Storage: querying/indexing them succeeds identically in
    //    bytecode and tree without lock hazards.
    let other_param = local(0);
    let closure_sig = Term::Closure {
        environment: vec![p(P::Bool)],
        signature: Box::new(Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![],
            result: Box::new(p(P::Bool)),
        }))),
    };
    let make_closure_fn = comptime_fn(
        decl(3),
        vec![(
            other_param,
            Parameter {
                sort: Sort::Runtime,
                ty: p(P::Bool),
            },
        )],
        closure_sig.clone(),
        expr(
            closure_sig.clone(),
            K::Closure {
                parameters: vec![],
                captures: vec![Capture {
                    binding: other_param,
                    ty: p(P::Bool),
                    value: None,
                }],
                body: Arc::new(expr(p(P::Bool), K::Local(other_param))),
            },
        ),
    );
    let obj_param = local(0);
    let obj_query_fn = comptime_fn(
        decl(2),
        vec![(
            obj_param,
            Parameter {
                sort: Sort::Runtime,
                ty: p(P::Object),
            },
        )],
        Term::Optional(Box::new(p(P::Bool))),
        expr(
            Term::Optional(Box::new(p(P::Bool))),
            K::ObjectQuery {
                query: ObjectQueryKind::NamedValue,
                subject: Box::new(expr(
                    Term::Tuple(vec![p(P::Object), p(P::Symbol)]),
                    K::Tuple(vec![
                        expr(p(P::Object), K::Local(obj_param)),
                        expr(p(P::Symbol), K::Literal(Literal::Symbol("flag".into()))),
                    ]),
                )),
            },
        ),
    );
    let list_head = DeclarationId {
        package: 77,
        local: 60,
    };
    let list_param_id = DeclarationId {
        package: 77,
        local: 61,
    };
    let list_ty = Term::NormalForm {
        head: list_head,
        sort: Sort::Type,
        arguments: vec![p(P::Bool)],
    };
    let list_local = local(0);
    let list_index_fn = comptime_fn(
        decl(4),
        vec![(
            list_local,
            Parameter {
                sort: Sort::Runtime,
                ty: list_ty.clone(),
            },
        )],
        p(P::Bool),
        expr(
            p(P::Bool),
            K::TypeListIndex(
                Box::new(expr(list_ty.clone(), K::Local(list_local))),
                Box::new(expr(
                    p(P::Integer),
                    K::Literal(Literal::Integer("1".into())),
                )),
            ),
        ),
    );
    let pkg2 = Package {
        normal_forms: vec![nir::normal_forms::Head {
            collector: None,
            role: nir::normal_forms::Role::TypeList,
            declaration: list_head,
            requirements: vec![],
            parameters: vec![nir::normal_forms::Parameter {
                declaration: list_param_id,
                binding: LocalId { owner: 9, index: 0 },
                kind: nir::normal_forms::Kind::Type,
                ty: p(P::Type),
            }],
        }],
        functions: vec![obj_query_fn, make_closure_fn, list_index_fn],
        ..Default::default()
    };
    let mut make_vm = Interpreter::for_comptime(&pkg2, Budget::default()).unwrap();
    let other_cell = Storage::new(Value::Bool(true));
    let closure_val = make_vm
        .invoke(decl(3), vec![Value::Storage(other_cell)])
        .unwrap();
    let obj_with_closure = Value::Object(Object::from_entries(vec![
        ObjectEntry::Property {
            key: "flag".into(),
            value: ObjectValue {
                ty: p(P::Bool),
                value: Value::Bool(true),
            },
        },
        ObjectEntry::Property {
            key: "cb".into(),
            value: ObjectValue {
                ty: closure_sig,
                value: closure_val,
            },
        },
    ]));
    let expected = Value::Optional {
        ty: Term::Optional(Box::new(p(P::Bool))),
        value: Some(Box::new(Value::Bool(true))),
    };
    let obj_storage = Value::Storage(Storage::new(obj_with_closure));
    assert_eq!(
        run_differential(&pkg2, decl(2), vec![obj_storage]),
        (Ok(expected.clone()), Ok(expected))
    );
    let list_storage = Value::Storage(Storage::new(Value::List {
        ty: list_ty,
        values: vec![Value::Bool(false), Value::Bool(true)].into(),
    }));
    assert_eq!(
        run_differential(&pkg2, decl(4), vec![list_storage]),
        (Ok(Value::Bool(true)), Ok(Value::Bool(true)))
    );
}

#[test]
fn object_literal_and_edit_enforce_phase_size_limit_and_abstract_keys() {
    let ptr_ty = Term::Pointer(Box::new(p(P::Bool)));
    // Object Append with pointer domain / value is rejected with PhaseMismatch.
    let base_obj = expr(
        p(P::Object),
        K::Object {
            names: vec![],
            fields: vec![],
        },
    );
    let append_bad = comptime_fn(
        decl(1),
        vec![],
        p(P::Object),
        expr(
            p(P::Object),
            K::ObjectEdit {
                operation: ObjectEditKind::Append,
                subject: Box::new(expr(
                    Term::Tuple(vec![
                        p(P::Object),
                        Term::Optional(Box::new(p(P::Symbol))),
                        Term::Optional(Box::new(ptr_ty.clone())),
                    ]),
                    K::Tuple(vec![
                        base_obj.clone(),
                        expr(Term::Optional(Box::new(p(P::Symbol))), K::OptionalAbsent),
                        expr(Term::Optional(Box::new(ptr_ty)), K::OptionalAbsent),
                    ]),
                )),
            },
        ),
    );
    // Object Concat exceeding 100_000 entries fails with RepresentationSizeLimit.
    let big_obj = Value::Object(Object::from_entries(vec![
        ObjectEntry::Value(ObjectValue {
            ty: p(P::Bool),
            value: Value::Bool(true),
        });
        60_000
    ]));
    let param_a = local(0);
    let param_b = local(1);
    let concat_fn = comptime_fn(
        decl(2),
        vec![
            (
                param_a,
                Parameter {
                    sort: Sort::Runtime,
                    ty: p(P::Object),
                },
            ),
            (
                param_b,
                Parameter {
                    sort: Sort::Runtime,
                    ty: p(P::Object),
                },
            ),
        ],
        p(P::Object),
        expr(
            p(P::Object),
            K::ObjectEdit {
                operation: ObjectEditKind::Concat,
                subject: Box::new(expr(
                    Term::Tuple(vec![p(P::Object), p(P::Object)]),
                    K::Tuple(vec![
                        expr(p(P::Object), K::Local(param_a)),
                        expr(p(P::Object), K::Local(param_b)),
                    ]),
                )),
            },
        ),
    );
    let pkg = Package {
        functions: vec![append_bad, concat_fn],
        ..Default::default()
    };
    let big_budget = Budget {
        steps: 10_000_000,
        depth: 128,
    };
    let mut byte_vm = Interpreter::for_comptime(&pkg, big_budget).unwrap();
    let mut tree_vm = Interpreter::for_comptime_tree(&pkg, big_budget).unwrap();
    assert!(matches!(
        byte_vm.invoke(decl(1), vec![]).unwrap_err().kind,
        ErrorKind::PhaseMismatch | ErrorKind::InvalidPhase(_)
    ));
    assert!(matches!(
        tree_vm.invoke(decl(1), vec![]).unwrap_err().kind,
        ErrorKind::PhaseMismatch | ErrorKind::InvalidPhase(_)
    ));
    assert_eq!(
        byte_vm
            .invoke(decl(2), vec![big_obj.clone(), big_obj.clone()])
            .unwrap_err()
            .kind,
        ErrorKind::RepresentationSizeLimit
    );
    assert_eq!(
        tree_vm
            .invoke(decl(2), vec![big_obj.clone(), big_obj])
            .unwrap_err()
            .kind,
        ErrorKind::RepresentationSizeLimit
    );

    // Abstract selector in ObjectQuery and ObjectEdit reports AbstractValueUnresolved.
    let key_param_id = DeclarationId {
        package: 77,
        local: 99,
    };
    let mut open_pkg = Package::default();
    let key_param = local(0);
    open_pkg.functions.push(comptime_fn(
        decl(3),
        vec![(
            key_param,
            Parameter {
                sort: Sort::ComptimeValue,
                ty: p(P::Symbol),
            },
        )],
        Term::Optional(Box::new(p(P::Bool))),
        expr(
            Term::Optional(Box::new(p(P::Bool))),
            K::ObjectQuery {
                query: ObjectQueryKind::NamedValue,
                subject: Box::new(expr(
                    Term::Tuple(vec![p(P::Object), p(P::Symbol)]),
                    K::Tuple(vec![base_obj, expr(p(P::Symbol), K::Local(key_param))]),
                )),
            },
        ),
    ));
    let mut budget = Budget::default();
    let mut open_vm = Interpreter::for_comptime_in_scope_in_budget(
        &open_pkg,
        &[],
        &[],
        &[(key_param_id, p(P::Symbol))],
        &mut budget,
    )
    .unwrap();
    assert_eq!(
        open_vm
            .invoke(
                decl(3),
                vec![Value::AbstractValue {
                    declaration: key_param_id,
                    ty: p(P::Symbol),
                }]
            )
            .unwrap_err()
            .kind,
        ErrorKind::AbstractValueUnresolved
    );
}

#[test]
fn scalar_math_single_rounds_ldexp_validates_inputs_and_supports_half_pending_floats() {
    let enum_id = DeclarationId {
        package: 77,
        local: 50,
    };
    let intrinsic_enum = Enum {
        is_intrinsic: true,
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        type_parameters: vec![],
        declaration: enum_id,
        name: "MathOp".into(),
        variants: vec![
            Variant {
                name: "ldexp".into(),
                shape: VariantShape::Unit,
                fields: vec![],
            },
            Variant {
                name: "iabs".into(),
                shape: VariantShape::Unit,
                fields: vec![],
            },
            Variant {
                name: "fma".into(),
                shape: VariantShape::Unit,
                fields: vec![],
            },
        ],
        groups: vec![],
        encoding: None,
        symbol_encoding: None,
    };
    let ldexp_tag = Tag {
        family: enum_id,
        variant: 0,
    };
    let iabs_tag = Tag {
        family: enum_id,
        variant: 1,
    };
    let fma_tag = Tag {
        family: enum_id,
        variant: 2,
    };
    let mut tmp_pkg = Package::default();
    tmp_pkg.enums.push(intrinsic_enum.clone());
    let bindings = Registry::new(
        &tmp_pkg,
        &[
            (ldexp_tag, IntrinsicOp::ScaleByPowerOfTwo),
            (iabs_tag, IntrinsicOp::WrappingAbsoluteValue),
            (fma_tag, IntrinsicOp::FusedMultiplyAdd),
        ],
    )
    .unwrap();
    let ldexp_checked = bindings
        .check(ldexp_tag, &[p(P::F32), p(P::I32)], &p(P::F32))
        .unwrap()
        .for_comptime();
    let ldexp_f64 = bindings
        .check(ldexp_tag, &[p(P::F64), p(P::I32)], &p(P::F64))
        .unwrap()
        .for_comptime();
    let iabs_i8 = bindings
        .check(iabs_tag, &[p(P::I8)], &p(P::I8))
        .unwrap()
        .for_comptime();
    let fma_f32 = bindings
        .check(fma_tag, &[p(P::F32), p(P::F32), p(P::F32)], &p(P::F32))
        .unwrap()
        .for_comptime();

    let x = local(0);
    let n = local(1);
    let ldexp_fn = comptime_fn(
        decl(1),
        vec![
            (
                x,
                Parameter {
                    sort: Sort::Runtime,
                    ty: p(P::F32),
                },
            ),
            (
                n,
                Parameter {
                    sort: Sort::Runtime,
                    ty: p(P::I32),
                },
            ),
        ],
        p(P::F32),
        expr(
            p(P::F32),
            K::UnsafeScope(Box::new(expr(
                p(P::F32),
                K::Intrinsic {
                    tag: ldexp_tag,
                    binding: Some(ldexp_checked),
                    inputs: vec![expr(p(P::F32), K::Local(x)), expr(p(P::I32), K::Local(n))],
                },
            ))),
        ),
    );
    let ldexp_f64_fn = comptime_fn(
        decl(2),
        vec![
            (
                x,
                Parameter {
                    sort: Sort::Runtime,
                    ty: p(P::F64),
                },
            ),
            (
                n,
                Parameter {
                    sort: Sort::Runtime,
                    ty: p(P::I32),
                },
            ),
        ],
        p(P::F64),
        expr(
            p(P::F64),
            K::UnsafeScope(Box::new(expr(
                p(P::F64),
                K::Intrinsic {
                    tag: ldexp_tag,
                    binding: Some(ldexp_f64),
                    inputs: vec![expr(p(P::F64), K::Local(x)), expr(p(P::I32), K::Local(n))],
                },
            ))),
        ),
    );
    let f16_pending_fn = comptime_fn(
        decl(3),
        vec![],
        p(P::F16),
        expr(
            p(P::F16),
            K::Literal(Literal::PendingFloat {
                spelling: "1.5".into(),
                negative: true,
            }),
        ),
    );
    let bf16_pending_fn = comptime_fn(
        decl(4),
        vec![],
        p(P::Bf16),
        expr(
            p(P::Bf16),
            K::Literal(Literal::PendingFloat {
                spelling: "1.5".into(),
                negative: true,
            }),
        ),
    );
    let pkg = Package {
        enums: vec![intrinsic_enum.clone()],
        functions: vec![ldexp_fn, ldexp_f64_fn, f16_pending_fn, bf16_pending_fn],
        ..Default::default()
    };

    // Single-rounding exactness: f32::from_bits(0x3300_000b) * 2^-104 = 0x0010_0001
    // (two-step multiply double-rounds to 0x0010_0002!).
    assert_eq!(
        run_differential(
            &pkg,
            decl(1),
            vec![Value::Float32(0x3300_000b), Value::SignedInteger(-104)]
        ),
        (
            Ok(Value::Float32(0x0010_0001)),
            Ok(Value::Float32(0x0010_0001))
        )
    );
    // Extreme exponents, -0.0, sNaN quietening.
    assert_eq!(
        run_differential(
            &pkg,
            decl(1),
            vec![
                Value::Float32(0x3f80_0000),
                Value::SignedInteger(i128::from(i32::MIN))
            ]
        ),
        (
            Ok(Value::Float32(0x0000_0000)),
            Ok(Value::Float32(0x0000_0000))
        )
    );
    assert_eq!(
        run_differential(
            &pkg,
            decl(1),
            vec![
                Value::Float32(0x8000_0000),
                Value::SignedInteger(i128::from(i32::MAX))
            ]
        ),
        (
            Ok(Value::Float32(0x8000_0000)),
            Ok(Value::Float32(0x8000_0000))
        )
    );
    assert_eq!(
        run_differential(
            &pkg,
            decl(1),
            vec![Value::Float32(0x7f80_0001), Value::SignedInteger(5)]
        ),
        (
            Ok(Value::Float32(0x7fc0_0001)),
            Ok(Value::Float32(0x7fc0_0001))
        )
    );
    // f64 tie-to-even subnormal rounding & PendingFloat F16/Bf16.
    // Exact halfway subnormal for f64: 1.5 * 2^-1074 rounds ties-to-even to 2 * 2^-1074 = 0x2.
    assert_eq!(
        run_differential(
            &pkg,
            decl(2),
            vec![
                Value::Float64(0x3ff8_0000_0000_0000), // 1.5
                Value::SignedInteger(-1074),
            ]
        ),
        (
            Ok(Value::Float64(0x0000_0000_0000_0002)),
            Ok(Value::Float64(0x0000_0000_0000_0002))
        )
    );
    // Exact halfway with even target: 2.5 * 2^-1074 -> 2 * 2^-1074 = 0x2.
    assert_eq!(
        run_differential(
            &pkg,
            decl(2),
            vec![
                Value::Float64(0x4004_0000_0000_0000), // 2.5
                Value::SignedInteger(-1074),
            ]
        ),
        (
            Ok(Value::Float64(0x0000_0000_0000_0002)),
            Ok(Value::Float64(0x0000_0000_0000_0002))
        )
    );
    assert_eq!(
        run_differential(&pkg, decl(3), vec![]),
        (Ok(Value::Float16(0xbe00)), Ok(Value::Float16(0xbe00)))
    );
    assert_eq!(
        run_differential(&pkg, decl(4), vec![]),
        (Ok(Value::BFloat16(0xbfc0)), Ok(Value::BFloat16(0xbfc0)))
    );

    // Out-of-range signed integer input for i8 is rejected cleanly; wrong FMA arity rejected;
    // AbstractValue in scalar math operand returns AbstractValueUnresolved.
    let i8_param = local(0);
    let iabs_fn = comptime_fn(
        decl(5),
        vec![(
            i8_param,
            Parameter {
                sort: Sort::Runtime,
                ty: p(P::I8),
            },
        )],
        p(P::I8),
        expr(
            p(P::I8),
            K::UnsafeScope(Box::new(expr(
                p(P::I8),
                K::Intrinsic {
                    tag: iabs_tag,
                    binding: Some(iabs_i8),
                    inputs: vec![expr(p(P::I8), K::Local(i8_param))],
                },
            ))),
        ),
    );
    let mut bad_fma_pkg = Package {
        enums: vec![intrinsic_enum.clone()],
        functions: vec![iabs_fn],
        ..Default::default()
    };
    let mut vm_i8 = Interpreter::for_comptime_tree(&bad_fma_pkg, Budget::default()).unwrap();
    assert_eq!(
        vm_i8
            .invoke(decl(5), vec![Value::SignedInteger(200)])
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );

    let abs_decl = DeclarationId {
        package: 77,
        local: 88,
    };
    bad_fma_pkg.functions[0].interface.parameters = vec![local(0)];
    let abs_fma = comptime_fn(
        decl(6),
        vec![(
            local(0),
            Parameter {
                sort: Sort::ComptimeValue,
                ty: p(P::F32),
            },
        )],
        p(P::F32),
        expr(
            p(P::F32),
            K::UnsafeScope(Box::new(expr(
                p(P::F32),
                K::Intrinsic {
                    tag: fma_tag,
                    binding: Some(fma_f32),
                    inputs: vec![
                        expr(p(P::F32), K::Literal(Literal::Float32(1.0f32.to_bits()))),
                        expr(p(P::F32), K::Literal(Literal::Float32(2.0f32.to_bits()))),
                        expr(p(P::F32), K::Local(local(0))),
                    ],
                },
            ))),
        ),
    );
    bad_fma_pkg.functions.push(abs_fma);
    let mut b = Budget::default();
    let mut open_vm = Interpreter::for_comptime_in_scope_in_budget(
        &bad_fma_pkg,
        &[],
        &[],
        &[(abs_decl, p(P::F32))],
        &mut b,
    )
    .unwrap();
    assert_eq!(
        open_vm
            .invoke(
                decl(6),
                vec![Value::AbstractValue {
                    declaration: abs_decl,
                    ty: p(P::F32),
                }]
            )
            .unwrap_err()
            .kind,
        ErrorKind::AbstractValueUnresolved
    );
}

#[test]
fn scoped_restore_observations_type_constructors_and_depth_exhausted() {
    // 1. scoped_restore: Lift -> Sort::Runtime; Value::Description rejected.
    let binder_id = DeclarationId {
        package: 77,
        local: 11,
    };
    let lifted = Term::Lift(Box::new(p(P::Bool)));
    let lifted_val = Value::AbstractValue {
        declaration: binder_id,
        ty: lifted.clone(),
    };
    let outer_id = DeclarationId {
        package: 77,
        local: 12,
    };
    let mut steps = 10_000;
    let restored = lifted_val
        .restore_scoped_types(
            &[DeclarationParameter {
                declaration: binder_id,
                sort: Sort::Runtime,
            }],
            &[Term::Rigid {
                declaration: outer_id,
                sort: Sort::Runtime,
            }],
            &[],
            &mut steps,
        )
        .unwrap();
    assert_eq!(
        restored,
        Value::AbstractValue {
            declaration: outer_id,
            ty: lifted.clone(),
        }
    );
    let exported = restored
        .into_abstract_invocation_term(
            &mut comptime_runtime::Budget::default(),
            &BTreeSet::new(),
            &BTreeMap::from([(outer_id, lifted)]),
        )
        .unwrap();
    assert_eq!(
        exported,
        Term::Rigid {
            declaration: outer_id,
            sort: Sort::Runtime,
        }
    );
    assert_eq!(
        Value::Description(DescriptionLiteral::Any)
            .restore_scoped_types(&[], &[], &[], &mut steps)
            .unwrap_err(),
        ErrorKind::TypeContractUnresolved
    );

    let vp_id = DeclarationId {
        package: 77,
        local: 13,
    };
    let value_params = BTreeMap::from([(vp_id, p(P::Bool))]);
    let rigid_comptime = Term::Rigid {
        declaration: vp_id,
        sort: Sort::ComptimeValue,
    };
    let fn_inst = Value::FunctionInstance {
        declaration: decl(1),
        arguments: vec![rigid_comptime.clone()],
    };
    let trait_method = Value::RuntimeTraitMethod(Box::new(TraitMethodIdentity {
        interface: InterfaceInstance {
            declaration: decl(2),
            arguments: vec![],
        },
        subject: rigid_comptime,
        member: decl(3),
        method_arguments: vec![],
    }));
    for val in [fn_inst, trait_method] {
        assert!(
            val.clone()
                .into_abstract_invocation_term(
                    &mut comptime_runtime::Budget::default(),
                    &BTreeSet::new(),
                    &value_params,
                )
                .is_ok()
        );
        assert_eq!(
            val.into_abstract_invocation_term(
                &mut comptime_runtime::Budget::default(),
                &BTreeSet::new(),
                &BTreeMap::new(),
            )
            .unwrap_err(),
            ErrorKind::Type
        );
    }

    // 2. DefinitionAttributes on Value::Description -> TypeContractUnresolved.
    let arg = local(0);
    let def_attrs_fn = comptime_fn(
        decl(1),
        vec![(
            arg,
            Parameter {
                sort: Sort::Runtime,
                ty: p(P::Type),
            },
        )],
        p(P::Object),
        expr(
            p(P::Object),
            K::DefinitionAttributes(Box::new(expr(p(P::Type), K::Local(arg)))),
        ),
    );
    // 3. PointerType rejects comptime-only pointees and out-of-scope Rigids.
    let ptr_ctor_fn = comptime_fn(
        decl(2),
        vec![(
            arg,
            Parameter {
                sort: Sort::Runtime,
                ty: p(P::Type),
            },
        )],
        p(P::Type),
        expr(
            p(P::Type),
            K::PointerType(Box::new(expr(p(P::Type), K::Local(arg)))),
        ),
    );
    let bad_ptr_rigid_fn = comptime_fn(
        decl(3),
        vec![],
        p(P::Type),
        expr(
            p(P::Type),
            K::PointerType(Box::new(expr(
                p(P::Type),
                K::TypeValue(Term::Rigid {
                    declaration: DeclarationId {
                        package: 77,
                        local: 999,
                    },
                    sort: Sort::Type,
                }),
            ))),
        ),
    );
    let pkg = Package {
        functions: vec![def_attrs_fn, ptr_ctor_fn, bad_ptr_rigid_fn],
        ..Default::default()
    };
    assert_eq!(
        run_differential(
            &pkg,
            decl(1),
            vec![Value::Description(DescriptionLiteral::Any)]
        ),
        (
            Err(ErrorKind::TypeContractUnresolved),
            Err(ErrorKind::TypeContractUnresolved)
        )
    );
    for invalid_pointee in [
        p(P::Type),
        p(P::Object),
        p(P::Integer),
        p(P::Float),
        p(P::Unit),
        Term::Lift(Box::new(p(P::Bool))),
        Term::ComptimeFunction(Box::new(Term::Function {
            parameters: vec![],
            result: Box::new(p(P::Bool)),
        })),
        Term::NormalFormFunction(
            Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(p(P::Type)),
            }),
            None,
        ),
        Term::ClosureContextType,
    ] {
        assert_eq!(
            run_differential(&pkg, decl(2), vec![Value::Type(invalid_pointee)]),
            (Err(ErrorKind::Type), Err(ErrorKind::Type))
        );
    }
    assert_eq!(
        run_differential(&pkg, decl(3), vec![]),
        (Err(ErrorKind::Type), Err(ErrorKind::Type))
    );
    assert_eq!(
        run_differential(&pkg, decl(2), vec![Value::Type(p(P::Bool))]),
        (
            Ok(Value::Type(Term::Pointer(Box::new(p(P::Bool))))),
            Ok(Value::Type(Term::Pointer(Box::new(p(P::Bool)))))
        )
    );

    // 4. DepthExhausted is preserved across deep signatures and checkpoints.
    let mut deep_ty = p(P::Bool);
    for _ in 0..140 {
        deep_ty = Term::Optional(Box::new(deep_ty));
    }
    let deep_pkg = Package {
        functions: vec![comptime_fn(
            decl(4),
            vec![(
                local(0),
                Parameter {
                    sort: Sort::Runtime,
                    ty: deep_ty.clone(),
                },
            )],
            p(P::Bool),
            expr(p(P::Bool), K::Literal(Literal::Bool(true))),
        )],
        ..Default::default()
    };
    assert_eq!(
        Interpreter::for_comptime(&deep_pkg, Budget::default())
            .err()
            .expect("deep signature must return DepthExhausted")
            .kind,
        ErrorKind::DepthExhausted
    );
    let closed_pkg = Package::default();
    let mut vm = Interpreter::for_comptime(&closed_pkg, Budget::default()).unwrap();
    let cell = Storage::new(Value::Type(deep_ty));
    assert_eq!(
        vm.checkpoint_owned_graph(&[Value::Storage(cell)])
            .err()
            .expect("deep term in storage should hit DepthExhausted")
            .kind,
        ErrorKind::DepthExhausted
    );
}
