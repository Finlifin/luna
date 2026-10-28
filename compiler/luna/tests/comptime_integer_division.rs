#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "typealias Integer=.Integer'builtin;mod meta {{{}}}{source}",
            include!("support/meta_source.rs")
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )
}

#[test]
fn explicit_integer_division_truncates_toward_zero_for_all_signs() {
    for (a, b, q, r) in [
        (7, 3, 2, 1),
        (-7, 3, -2, -1),
        (7, -3, -2, 1),
        (-7, -3, 2, -1),
        (0, 3, 0, 0),
        (2, 3, 0, 2),
        (-2, 3, 0, -2),
        (-6, 3, -2, 0),
        (6, -3, -2, 0),
    ] {
        let package=compile(&format!(
            "comptime fn run()->bool {{meta.div_trunc({a},{b}) == {q} and meta.rem_trunc({a},{b}) == {r}}}"
        )).unwrap();
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn arbitrary_precision_quotients_retain_every_bit() {
    let package = compile(
        "comptime fn run()->bool {
            let q:Integer=1606938044258990275541962092341162602522202993782792835301376;
            let a=q * 17 + 9;
            meta.div_trunc(a,17) == q and meta.rem_trunc(a,17) == 9
                and meta.div_trunc(-a,17) == -q and meta.rem_trunc(-a,17) == -9
                and meta.div_trunc(a,-17) == -q and meta.rem_trunc(a,-17) == 9
                and meta.div_trunc(-a,-17) == q and meta.rem_trunc(-a,-17) == -9
        }",
    )
    .unwrap();
    assert!(luna::invocation::invoke(&package, "run", 10).is_err());
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn explicit_division_evaluates_operands_once_and_preserves_early_returns() {
    for operation in ["div_trunc", "rem_trunc"] {
        let expected = if operation == "div_trunc" { 2 } else { 1 };
        let package = compile(&format!(
            "comptime fn left(trace:*Integer)->Integer {{trace.* = trace.* * 10 + 1;7}}
            comptime fn right(trace:*Integer)->Integer {{trace.* = trace.* * 10 + 2;3}}
            comptime fn run()->bool {{
                var trace:Integer=0;
                let result=meta.{operation}(left(trace.ref),right(trace.ref));
                result == {expected} and trace == 12
            }}
            comptime fn early()->bool {{
                .comptime_integer_{operation}'builtin(do {{return true;}},do {{return false;}});
                false
            }}"
        ))
        .unwrap();
        for entry in ["run", "early"] {
            assert_eq!(
                luna::invocation::invoke(&package, entry, 100_000).unwrap(),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn zero_divisors_fail_with_source_and_leave_the_vm_usable() {
    for operation in ["div_trunc", "rem_trunc"] {
        let package = compile(&format!(
            "comptime fn fail()->Integer {{meta.{operation}(7,0)}}
            comptime fn recover()->bool {{meta.div_trunc(7,3) == 2 and meta.rem_trunc(7,3) == 1}}"
        ))
        .unwrap();
        let mut vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 100_000,
                ..Default::default()
            },
        )
        .unwrap();
        let error = vm
            .invoke(support::id(&package, "fail"), vec![])
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::DivisionByZero);
        assert!(error.source.is_some());
        assert_eq!(
            vm.invoke(support::id(&package, "recover"), vec![]).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn explicit_division_rejects_invalid_domains_arity_and_runtime_use() {
    for operation in ["div_trunc", "rem_trunc"] {
        for body in [
            format!("fn bad() {{meta.{operation}(7,3);}}"),
            format!("comptime fn bad() {{meta.{operation}(true,3);}}"),
            format!("comptime fn bad() {{meta.{operation}(7,false);}}"),
            format!("comptime fn bad() {{.comptime_integer_{operation}'builtin(7);}}"),
            format!("comptime fn bad() {{.comptime_integer_{operation}'builtin(7,3,2);}}"),
            format!("fn bad() {{.comptime_integer_{operation}'builtin(7,3);}}"),
        ] {
            assert!(compile(&body).is_err(), "accepted {body}");
        }
    }
    // Explicit library operations must not install an implicit rounding policy.
    for op in ["/", "%"] {
        assert!(
            compile(&format!(
                "comptime fn bad(a:Integer,b:Integer)->Integer {{a {op} b}}"
            ))
            .is_err()
        );
    }
}
