//! List checks use the declaration and element domain, preserving effect order.
use super::*;
use nir::normal_forms::{Head, Kind, Parameter, Role};
use nir::program::{Literal, Place};
const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn list(element: Primitive) -> Term {
    Term::NormalForm {
        head: id(2),
        sort: nir::Sort::Type,
        arguments: vec![Term::Primitive(element)],
    }
}
fn package() -> Package {
    Package {
        normal_forms: vec![Head {
            collector: None,
            role: Role::TypeList,
            declaration: id(2),
            requirements: vec![],
            parameters: vec![Parameter {
                declaration: id(3),
                binding: LocalId { owner: 2, index: 0 },
                kind: Kind::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
        }],
        ..Default::default()
    }
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty,
        kind,
    }
}
fn run(
    vm: &mut Interpreter<'_>,
    body: &Expr,
    env: &mut Environment,
    compiled: bool,
) -> Result<Flow, Error> {
    if compiled {
        let code = Arc::new(comptime_compiler::compile(body, &mut 10000).unwrap());
        vm.execute_code(&code, env)
    } else {
        vm.eval(body, env)
    }
}

#[test]
fn list_identity_checks_precede_concat_suffix_and_length_reads() {
    let package = package();
    let input = LocalId { owner: 1, index: 0 };
    let trace = LocalId { owner: 1, index: 1 };
    let left = expr(list(Primitive::Bool), K::Local(input));
    let right = expr(
        list(Primitive::Bool),
        K::Block {
            statements: vec![expr(
                Term::Primitive(Primitive::Void),
                K::Assign {
                    place: Place {
                        root: trace,
                        fields: vec![],
                    },
                    value: Box::new(expr(
                        Term::Primitive(Primitive::Bool),
                        K::Literal(Literal::Bool(true)),
                    )),
                },
            )],
            result: Some(Box::new(expr(list(Primitive::Bool), K::TypeList(vec![])))),
        },
    );
    let concat = expr(
        list(Primitive::Bool),
        K::ListConcat(Box::new(left.clone()), Box::new(right)),
    );
    let length = expr(
        Term::Primitive(Primitive::Integer),
        K::ListLength(Box::new(left)),
    );
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for valid in [false, true] {
            let original = if valid {
                Value::List {
                    ty: list(Primitive::Bool),
                    values: vec![Value::Bool(false)],
                }
            } else {
                Value::List {
                    ty: list(Primitive::Char),
                    values: vec![Value::Char('x')],
                }
            };
            let mut env = Environment::new();
            env.insert(input, original.clone());
            env.insert(trace, Value::Bool(false));
            let result = run(&mut vm, &concat, &mut env, compiled);
            if valid {
                assert!(matches!(result.unwrap(), Flow::Value(Value::List { .. })));
            } else {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
            }
            assert_eq!(env.get(&trace), Some(&Value::Bool(valid)));
            assert_eq!(env.get(&input), Some(&original));
            let result = run(&mut vm, &length, &mut env, compiled);
            if valid {
                assert!(matches!(result.unwrap(), Flow::Value(Value::Integer(_))));
            } else {
                assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
            }
        }
    }
}

#[test]
fn construction_keeps_type_conversion_and_aggregate_validation_boundaries() {
    let package = package();
    let input = LocalId { owner: 1, index: 0 };
    let trace = LocalId { owner: 1, index: 1 };
    for compiled in [false, true] {
        for element in [Primitive::Bool, Primitive::Type] {
            let ty = Term::Primitive(element);
            let second = expr(
                ty.clone(),
                K::Block {
                    statements: vec![expr(
                        Term::Primitive(Primitive::Void),
                        K::Assign {
                            place: Place {
                                root: trace,
                                fields: vec![],
                            },
                            value: Box::new(expr(
                                Term::Primitive(Primitive::Bool),
                                K::Literal(Literal::Bool(true)),
                            )),
                        },
                    )],
                    result: Some(Box::new(expr(
                        ty.clone(),
                        if element == Primitive::Type {
                            K::TypeValue(Term::Primitive(Primitive::Bool))
                        } else {
                            K::Literal(Literal::Bool(true))
                        },
                    ))),
                },
            );
            let body = expr(
                list(element),
                K::TypeList(vec![expr(ty, K::Local(input)), second]),
            );
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let mut env = Environment::new();
            env.insert(input, Value::Char('x'));
            env.insert(trace, Value::Bool(false));
            assert_eq!(
                run(&mut vm, &body, &mut env, compiled).unwrap_err().kind,
                ErrorKind::Type
            );
            assert_eq!(
                env.get(&trace),
                Some(&Value::Bool(element != Primitive::Type))
            );
            env.insert(
                input,
                if element == Primitive::Type {
                    Value::Type(Term::Primitive(Primitive::Bool))
                } else {
                    Value::Bool(false)
                },
            );
            assert!(matches!(
                run(&mut vm, &body, &mut env, compiled).unwrap(),
                Flow::Value(_)
            ));
        }
    }
}
