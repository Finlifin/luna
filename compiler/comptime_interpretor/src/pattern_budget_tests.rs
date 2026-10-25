use super::*;
use nir::program::Pattern;

fn check(pattern: &Pattern, value: &Value, steps: u64) -> Result<bool, ErrorKind> {
    let package = Package::default();
    let mut vm = Interpreter::new(&package, Budget { steps, depth: 128 }).unwrap();
    match vm.match_pattern(pattern, value, &mut Environment::new()) {
        Ok(Flow::Value(Value::Bool(result))) => Ok(result),
        Err(error) => Err(error.kind),
        _ => panic!("literal pattern returned a non-Boolean result"),
    }
}

#[test]
fn text_literal_comparisons_charge_work_before_scanning() {
    let text = "雪".repeat(200);
    for symbol in [false, true] {
        let value = if symbol {
            Value::Symbol(text.clone().into())
        } else {
            Value::String(text.clone().into())
        };
        for equal in [false, true] {
            let expected = if equal {
                text.clone()
            } else {
                format!("{}雨", "雪".repeat(199))
            };
            let pattern = Pattern::Literal(if symbol {
                Literal::Symbol(expected)
            } else {
                Literal::String(expected)
            });
            assert_eq!(check(&pattern, &value, 32), Err(ErrorKind::StepsExhausted));
            assert_eq!(check(&pattern, &value, 10_000), Ok(equal));
        }
    }
    assert_eq!(
        check(
            &Pattern::Literal(Literal::String("".into())),
            &Value::String("".into()),
            32
        ),
        Ok(true)
    );
    assert_eq!(
        check(
            &Pattern::Literal(Literal::Symbol(text.clone())),
            &Value::String(text.into()),
            32
        ),
        Ok(false)
    );
}

#[test]
fn bit_literal_comparisons_charge_words_and_preserve_width() {
    let words = vec![0u32; 256];
    let value = Value::BitVec(
        comptime_runtime::bitvec::BitVec::from_words(8192, &words, &mut Default::default())
            .unwrap(),
    );
    for equal in [false, true] {
        let mut expected = words.clone();
        if !equal {
            expected[255] = 1;
        }
        let pattern = Pattern::Literal(Literal::BitVec {
            width: 8192,
            words: expected,
        });
        assert_eq!(check(&pattern, &value, 32), Err(ErrorKind::StepsExhausted));
        assert_eq!(check(&pattern, &value, 10_000), Ok(equal));
    }
    assert_eq!(
        check(
            &Pattern::Literal(Literal::BitVec { width: 8191, words }),
            &value,
            10_000
        ),
        Ok(false)
    );
}

#[test]
fn text_literal_allocation_is_charged_on_every_evaluation() {
    use nir::program::{Expr, ExprKind, SourceRange};
    for symbol in [false, true] {
        let package = Package::default();
        let text = "雪".repeat(200);
        let source = SourceRange { start: 10, end: 20 };
        let expression = Expr {
            source,
            ty: Term::Primitive(if symbol {
                Primitive::Symbol
            } else {
                Primitive::Str
            }),
            kind: ExprKind::Literal(if symbol {
                Literal::Symbol(text.clone())
            } else {
                Literal::String(text.clone())
            }),
        };
        let mut vm = Interpreter::new(
            &package,
            Budget {
                steps: 1000,
                depth: 128,
            },
        )
        .unwrap();
        let expected = if symbol {
            Value::Symbol(text.into())
        } else {
            Value::String(text.into())
        };
        assert!(
            matches!(vm.eval(&expression, &mut Environment::new()).unwrap(), Flow::Value(value) if value == expected)
        );
        // Re-evaluating the same IR node must pay for another allocation;
        // neither identical spelling nor a previous evaluation replenishes fuel.
        let error = match vm.eval(&expression, &mut Environment::new()) {
            Err(error) => error,
            Ok(_) => panic!("second allocation exceeded the shared budget"),
        };
        assert_eq!(error.kind, ErrorKind::StepsExhausted);
        assert_eq!(error.source, Some(source));
    }
}
