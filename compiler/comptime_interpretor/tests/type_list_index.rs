//! Type-list leaf reads retain evaluation order while charging only selected copies.
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
fn package() -> Package {
    let binding = LocalId { owner: 3, index: 0 };
    let source = SourceRange { start: 0, end: 1 };
    Package {
        normal_forms: vec![
            Head {
                collector: None,
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
                collector: None,
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

fn reads_package(reads: usize, local_index: bool, index: &str, storage: bool) -> Package {
    let mut package = package();
    let f = &mut package.functions[0];
    let source = f.body.source;
    let position = LocalId { owner: 3, index: 1 };
    if local_index {
        f.parameters.push(position);
        let Term::Function { parameters, .. } = &mut f.signature else {
            unreachable!()
        };
        parameters.push(Parameter {
            sort: Sort::ComptimeValue,
            ty: ty(P::Integer),
        });
    }
    let read = Expr {
        source,
        ty: ty(P::Type),
        kind: K::TypeListIndex(
            Box::new(Expr {
                source,
                ty: list_type(),
                kind: K::Local(f.parameters[0]),
            }),
            Box::new(Expr {
                source,
                ty: ty(P::Integer),
                kind: if local_index {
                    K::Local(position)
                } else {
                    K::Literal(nir::program::Literal::Integer(index.into()))
                },
            }),
        ),
    };
    let mut statements = vec![];
    if storage {
        statements.push(Expr {
            source,
            ty: ty(P::Void),
            kind: K::Assign {
                place: nir::program::Place {
                    root: f.parameters[0],
                    fields: vec![],
                },
                value: Box::new(Expr {
                    source,
                    ty: list_type(),
                    kind: K::Local(f.parameters[0]),
                }),
            },
        });
    }
    statements.extend(vec![read.clone(); reads - 1]);
    f.body = Expr {
        source,
        ty: ty(P::Type),
        kind: K::Block {
            statements,
            result: Some(Box::new(read)),
        },
    };
    package
}

#[test]
fn repeated_leaf_reads_do_not_recopy_unrelated_types() {
    for local in [false, true] {
        for storage in [false, true] {
            let run = |reads| {
                let package = reads_package(reads, local, "4095", storage);
                let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                let mut arguments = vec![Value::TypeList(vec![ty(P::Bool); 4096])];
                if local {
                    arguments.push(Value::Integer(comptime_runtime::Integer::from_u128(4095)));
                }
                let before = vm.remaining_steps();
                let compilation_before = vm.bytecode_compilation_steps();
                assert_eq!(
                    vm.invoke(id(3), arguments).unwrap(),
                    Value::Type(ty(P::Bool))
                );
                let compilation = vm.bytecode_compilation_steps() - compilation_before;
                assert!(compilation > 0);
                assert_eq!(vm.bytecode_invocations(), 1);
                before - vm.remaining_steps() - compilation
            };
            let single = run(1);
            let repeated = run(64);
            assert!(
                repeated > single && repeated - single < 2000,
                "local={local}, storage={storage}: {single} -> {repeated}"
            );
        }
    }
}

#[test]
fn bounds_failures_leave_the_vm_reusable() {
    for local in [false, true] {
        for index in ["-1", "1", "999999999999999999999999999999999999"] {
            let mut package = reads_package(1, local, index, true);
            let mut good = reads_package(1, false, "0", true).functions.remove(0);
            good.declaration = id(4);
            package.functions.push(good);
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut arguments = vec![Value::TypeList(vec![ty(P::Bool)])];
            if local {
                let mut index_value = comptime_runtime::Integer::parse_literal(
                    index.trim_start_matches('-'),
                    &mut Default::default(),
                )
                .unwrap();
                if index.starts_with('-') {
                    index_value = index_value.negated();
                }
                arguments.push(Value::Integer(index_value));
            }
            assert_eq!(
                vm.invoke(id(3), arguments).unwrap_err().kind,
                comptime_interpretor::ErrorKind::IndexOutOfBounds
            );
            assert_eq!(
                vm.invoke(id(4), vec![Value::TypeList(vec![ty(P::Bool)])])
                    .unwrap(),
                Value::Type(ty(P::Bool))
            );
        }
    }
}

#[test]
fn effectful_index_keeps_the_list_snapshot_from_before_index_evaluation() {
    let mut package = reads_package(1, false, "0", false);
    let f = &mut package.functions[0];
    let source = f.body.source;
    let K::Block {
        result: Some(read), ..
    } = &mut f.body.kind
    else {
        unreachable!()
    };
    let K::TypeListIndex(_, index) = &mut read.kind else {
        unreachable!()
    };
    let position = index.clone();
    index.kind = K::Block {
        statements: vec![Expr {
            source,
            ty: ty(P::Void),
            kind: K::Assign {
                place: nir::program::Place {
                    root: f.parameters[0],
                    fields: vec![],
                },
                value: Box::new(Expr {
                    source,
                    ty: list_type(),
                    kind: K::TypeListValue(vec![ty(P::U8)]),
                }),
            },
        }],
        result: Some(position),
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(3), vec![Value::TypeList(vec![ty(P::Bool)])])
            .unwrap(),
        Value::Type(ty(P::Bool))
    );
}
