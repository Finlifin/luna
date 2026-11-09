use super::*;
use nir::program::Place;

const SOURCE: SourceRange = SourceRange { start: 3, end: 11 };
fn flag(index: usize) -> LocalId {
    LocalId {
        owner: 1,
        index: index as u32,
    }
}
fn expr(ty: Primitive, kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty: Term::Primitive(ty),
        kind,
    }
}
fn bits() -> Expr {
    expr(
        Primitive::BitVec(8),
        K::Literal(Literal::BitVec {
            width: 8,
            words: vec![0xff],
        }),
    )
}
fn position(value: u128) -> Expr {
    expr(Primitive::U64, K::Literal(Literal::UnsignedInteger(value)))
}
fn marked(index: usize, value: Expr) -> Expr {
    Expr {
        source: SOURCE,
        ty: value.ty.clone(),
        kind: K::Block {
            statements: vec![expr(
                Primitive::Void,
                K::Assign {
                    place: Place {
                        root: flag(index),
                        fields: vec![],
                    },
                    value: Box::new(expr(Primitive::Bool, K::Literal(Literal::Bool(true)))),
                },
            )],
            result: Some(Box::new(value)),
        },
    }
}
fn operands(arity: usize, return_at: Option<usize>, wrong_first: bool) -> Expr {
    let mut inputs = match arity {
        1 => vec![bits()],
        2 => vec![bits(), bits()],
        3 => vec![bits(), position(1), position(6)],
        _ => unreachable!(),
    };
    if wrong_first {
        inputs[0] = expr(Primitive::Bool, K::Literal(Literal::Bool(false)));
    }
    if let Some(index) = return_at {
        inputs[index] = expr(Primitive::Never, K::Return(Some(Box::new(bits()))));
    }
    let mut inputs = inputs
        .into_iter()
        .enumerate()
        .map(|(index, value)| Box::new(marked(index, value)));
    let value = inputs.next().unwrap();
    let (width, kind) = match arity {
        1 => (
            8,
            K::BitSlice {
                value,
                start: 0,
                end: 8,
            },
        ),
        2 => (16, K::BitConcat(value, inputs.next().unwrap())),
        3 => (
            5,
            K::BitWindow {
                value,
                start: inputs.next().unwrap(),
                end: inputs.next().unwrap(),
            },
        ),
        _ => unreachable!(),
    };
    expr(Primitive::BitVec(width), kind)
}
fn execute(
    vm: &mut Interpreter<'_>,
    body: &Expr,
    environment: &mut Environment,
    compiled: bool,
) -> Result<Flow, Error> {
    if compiled {
        let code = Arc::new(comptime_compiler::compile(body, &mut 10000).unwrap());
        vm.execute_code(&code, environment)
    } else {
        vm.eval(body, environment)
    }
}
fn fresh_environment() -> Environment {
    (0..3)
        .map(|index| (flag(index), Value::Bool(false)))
        .collect()
}
fn check_flags(environment: &Environment, count: usize) {
    for index in 0..3 {
        assert_eq!(environment[&flag(index)], Value::Bool(index < count));
    }
}

#[test]
fn fixed_arity_operands_preserve_order_early_returns_and_eager_rejection() {
    let package = Package::default();
    for compiled in [false, true] {
        for arity in 1..=3 {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            for stop in std::iter::once(None).chain((0..arity).map(Some)) {
                let mut environment = fresh_environment();
                let result = execute(
                    &mut vm,
                    &operands(arity, stop, false),
                    &mut environment,
                    compiled,
                )
                .unwrap();
                let bits = match (stop, result) {
                    (Some(_), Flow::Return(Value::BitVec(bits))) => {
                        assert_eq!(bits.width(), 8);
                        bits
                    }
                    (None, Flow::Value(Value::BitVec(bits))) => {
                        assert_eq!(bits.width(), [0, 8, 16, 5][arity]);
                        bits
                    }
                    _ => panic!("wrong control-flow result"),
                };
                assert_eq!(bits.words(), &[(1u32 << bits.width()) - 1]);
                check_flags(&environment, stop.map_or(arity, |index| index + 1));
            }
            let mut environment = fresh_environment();
            assert_eq!(
                execute(
                    &mut vm,
                    &operands(arity, None, true),
                    &mut environment,
                    compiled
                )
                .unwrap_err(),
                Error {
                    kind: ErrorKind::Type,
                    source: Some(SOURCE)
                }
            );
            check_flags(&environment, 1);
            // Failure must not leave a borrowed operand buffer or execution state
            // that prevents a later independent call in the same VM.
            let mut fresh = fresh_environment();
            assert!(matches!(
                execute(&mut vm, &operands(arity, None, false), &mut fresh, compiled).unwrap(),
                Flow::Value(Value::BitVec(_))
            ));
            check_flags(&fresh, arity);
        }
    }
}

#[test]
fn window_validation_follows_both_positions_and_preserves_all_effects() {
    let package = Package::default();
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for (start, end, expected) in [
            (position(7), position(6), ErrorKind::IndexOutOfBounds),
            (position(1), position(9), ErrorKind::IndexOutOfBounds),
            (
                expr(Primitive::Bool, K::Literal(Literal::Bool(true))),
                position(6),
                ErrorKind::IndexOutOfBounds,
            ),
            (position(1), position(7), ErrorKind::Type),
        ] {
            let body = expr(
                Primitive::BitVec(5),
                K::BitWindow {
                    value: Box::new(marked(0, bits())),
                    start: Box::new(marked(1, start)),
                    end: Box::new(marked(2, end)),
                },
            );
            let mut environment = fresh_environment();
            assert_eq!(
                execute(&mut vm, &body, &mut environment, compiled)
                    .unwrap_err()
                    .kind,
                expected
            );
            check_flags(&environment, 3);
        }
    }
}

#[test]
fn operand_buffers_preserve_arity_checks_without_consuming_invalid_packets() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let boolean = Term::Primitive(Primitive::Bool);
    for operation in [
        B::Take,
        B::Convert,
        B::Index,
        B::Concat,
        B::Equal,
        B::And,
        B::Or,
        B::Window,
        B::Slice { start: 0, end: 1 },
    ] {
        for count in 0..=4 {
            let expected = match operation {
                B::Take | B::Convert | B::Slice { .. } => 1,
                B::Window => 3,
                _ => 2,
            };
            if count == expected {
                continue;
            }
            let mut values = vec![Value::Bool(true); count];
            let before = values.clone();
            assert_eq!(
                vm.bit_values(
                    operation,
                    &vec![&boolean; count],
                    &boolean,
                    &mut values,
                    SOURCE
                )
                .unwrap_err()
                .kind,
                ErrorKind::InvalidCallPlan
            );
            assert_eq!(values, before);
        }
    }
}

#[test]
fn nested_bit_operations_fit_a_normal_native_stack_in_both_engines() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let package = Package::default();
            let mut body = bits();
            for _ in 0..32 {
                body = expr(
                    Primitive::BitVec(8),
                    K::BitSlice {
                        value: Box::new(body),
                        start: 0,
                        end: 8,
                    },
                );
            }
            for compiled in [false, true] {
                let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
                let Flow::Value(Value::BitVec(value)) =
                    execute(&mut vm, &body, &mut Environment::new(), compiled).unwrap()
                else {
                    panic!("bits expected")
                };
                assert_eq!(value.words(), &[0xff]);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
