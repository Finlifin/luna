//! Real-sysroot tests for capability traits, HandleError `.use(handler)`,
//! opaque `builtin.Continuation`, `catch` effect signatures, and `Abi::Capturing`.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use nir::{
    Primitive, Term,
    effects::Abi,
    program::{ExprKind, Function},
};

fn find_source_fn<'a>(package: &'a nir::program::Package, name: &str) -> &'a Function {
    package
        .functions
        .iter()
        .find(|f| f.declaration.package == 0 && f.name == name)
        .unwrap_or_else(|| panic!("missing source function `{name}`"))
}

fn contains_expr(expr: &nir::program::Expr, pred: &impl Fn(&nir::program::Expr) -> bool) -> bool {
    let mut found = false;
    let mut fuel = 100_000usize;
    let _ = nir::visit::expression(expr, &mut fuel, |node, _| {
        if pred(node) {
            found = true;
        }
        Ok::<(), String>(())
    });
    found
}

#[test]
fn handle_error_use_executes_and_evaluates_handler_once() {
    let source = "
        enum LimitErr {
            TooLarge(u32),
        }

        struct ClampTo {
            max: u32,
            called: *bool,
        }

        impl HandleError<LimitErr, u32> for ClampTo {
            fn handle_error(*self, value: LimitErr) -> u32 {
                value match {
                    LimitErr.TooLarge(n) => if n > self.max { self.max } else { n },
                }
            }
        }

        fn checked_count(v: u32) -> !LimitErr u32 {
            if v <= 8 { v } else { error LimitErr.TooLarge(v) }
        }

        fn make_handler(max: u32, called: *bool) -> ClampTo {
            if called.* {
                ClampTo { max: 999, called: called }
            } else {
                called.* = true;
                ClampTo { max: max, called: called }
            }
        }

        fn run_clamped() -> u32 {
            var called: bool = false;
            let result = checked_count(12).use(make_handler(8, called.ref));
            if called { result } else { 999 }
        }

        fn run_ok() -> u32 {
            var called: bool = false;
            let result = checked_count(5).use(make_handler(8, called.ref));
            if called { result } else { 999 }
        }
    ";

    assert_eq!(
        support::run_with_inputs(source, "run_clamped", vec![]),
        Value::UnsignedInteger(8)
    );
    assert_eq!(
        support::run_with_inputs(source, "run_ok", vec![]),
        Value::UnsignedInteger(5)
    );
}

#[test]
fn handle_error_use_eliminates_handled_error_and_preserves_residual_errors() {
    let source = "
        enum LimitErr {
            TooLarge(u32),
        }

        enum AuditErr {
            Rejected,
        }

        struct ClampTo {
            max: u32,
        }

        impl HandleError<LimitErr, u32> for ClampTo {
            fn handle_error(*self, value: LimitErr) -> u32 {
                value match {
                    LimitErr.TooLarge(n) => if n > self.max { self.max } else { n },
                }
            }
        }

        fn check_both(v: u32) -> ![LimitErr, AuditErr] u32 {
            if v == 0 {
                error AuditErr.Rejected
            } else if v <= 8 {
                v
            } else {
                error LimitErr.TooLarge(v)
            }
        }

        fn handle_limit(v: u32) -> !AuditErr u32 {
            check_both(v).use(ClampTo { max: 8 })
        }

        fn run_clamped() -> u32 {
            handle_limit(12) match {
                val! => val,
                error AuditErr.Rejected => 0,
            }
        }

        fn run_residual() -> u32 {
            handle_limit(0) match {
                val! => val,
                error AuditErr.Rejected => 77,
            }
        }
    ";

    let pkg = support::compile(source).expect("partial HandleError elimination should compile");
    let handle_limit = find_source_fn(&pkg, "handle_limit");
    let Term::Function { result, .. } = &handle_limit.signature else {
        panic!("expected function signature");
    };
    let Term::ErrorResult { errors, .. } = result.as_ref() else {
        panic!("expected residual ErrorResult, got {:?}", result);
    };
    assert_eq!(
        errors.len(),
        1,
        "LimitErr should be eliminated leaving only AuditErr"
    );

    assert_eq!(
        support::run_with_inputs(source, "run_clamped", vec![]),
        Value::UnsignedInteger(8)
    );
    assert_eq!(
        support::run_with_inputs(source, "run_residual", vec![]),
        Value::UnsignedInteger(77)
    );
}

#[test]
fn handle_error_use_rejects_invalid_targets_and_missing_impls() {
    let non_error = "
        struct ClampTo { max: u32 }
        fn plain() -> u32 { 42 }
        fn bad() -> u32 {
            plain().use(ClampTo { max: 8 })
        }
    ";
    let err = support::compile(non_error).unwrap_err();
    assert!(
        err.contains("requires an error-qualified operand"),
        "unexpected error: {err}"
    );

    let missing_impl = "
        enum LimitErr { TooLarge(u32), }
        struct Unrelated {}
        fn checked(v: u32) -> !LimitErr u32 {
            if v <= 8 { v } else { error LimitErr.TooLarge(v) }
        }
        fn bad() -> u32 {
            checked(12).use(Unrelated {})
        }
    ";
    let err = support::compile(missing_impl).unwrap_err();
    assert!(err.contains("HandleError"), "unexpected error: {err}");

    let wrong_error_impl = "
        enum LimitErr { TooLarge(u32), }
        enum OtherErr { Boom, }
        struct OtherHandler {}
        impl HandleError<OtherErr, u32> for OtherHandler {
            fn handle_error(*self, _v: OtherErr) -> u32 { 0 }
        }
        fn checked(v: u32) -> !LimitErr u32 {
            if v <= 8 { v } else { error LimitErr.TooLarge(v) }
        }
        fn bad() -> u32 {
            checked(12).use(OtherHandler {})
        }
    ";
    let err = support::compile(wrong_error_impl).unwrap_err();
    assert!(err.contains("HandleError"), "unexpected error: {err}");

    let deferred_impl = "
        enum LimitErr { TooLarge(u32), }
        enum OtherErr { Boom, }
        struct Handler where T { tag: T }
        impl HandleError<OtherErr, u32> for Handler<T> where T {
            fn handle_error(*self, _v: OtherErr) -> u32 { 0 }
        }
        impl HandleError<LimitErr, u32> for Handler<bool> {
            fn handle_error(*self, _v: LimitErr) -> u32 { 1 }
        }
        fn checked(v: u32) -> ![LimitErr, OtherErr] u32 {
            if v <= 8 { v } else { error LimitErr.TooLarge(v) }
        }
        fn bad(h: Handler<T>) -> !LimitErr u32 where T {
            checked(12).use(h)
        }
    ";
    let err = support::compile(deferred_impl).unwrap_err();
    assert!(
        err.contains("HandleError") || err.contains("method") || err.contains("implementation"),
        "expected HandleError resolution error instead of swallowing deferred selection: {err}"
    );
}

#[test]
fn capability_traits_are_exported_from_builtin_and_prelude() {
    let source = "
        use builtin.capabilities.{HandleError, HandleEffect, HandleCapturingEffect};

        enum MyErr { Fail, }
        struct DirectHandler {}
        struct CapHandler {}

        impl HandleError<MyErr, u32> for DirectHandler {
            fn handle_error(*self, _e: MyErr) -> u32 { 1 }
        }

        impl HandleEffect<u32, u32> for DirectHandler {
            fn handle_effect(*self, args: u32) -> u32 {
                if args < 100 { args + 1 } else { args }
            }
        }

        impl HandleCapturingEffect<u32, u32> for CapHandler {
            fn handle_capturing_effect(
                *self,
                args: u32,
                _continuation: builtin.Continuation,
            ) -> u32 {
                args
            }
        }

        fn run() -> u32 {
            let d = DirectHandler {};
            d.handle_effect(41)
        }
    ";
    assert_eq!(support::run(source), Value::UnsignedInteger(42));
}

#[test]
fn builtin_continuation_has_zero_type_params_is_once_and_not_copy_or_clone() {
    let once_ok = "
        fn require_once(value: T) -> T
        where
            T :- Once,
        {
            value
        }

        fn forward(k: builtin.Continuation) -> builtin.Continuation {
            require_once(k)
        }
    ";
    support::compile(once_ok).expect("builtin.Continuation should satisfy blanket Once");

    let zero_type_params = "
        fn bad(k: builtin.Continuation<u32>) {}
    ";
    assert!(support::compile(zero_type_params).is_err());

    for bound in ["Copy", "Clone"] {
        let bad_bound = format!(
            "
            fn require(value: T) where T :- {bound} {{}}
            fn bad(k: builtin.Continuation) {{ require(k); }}
            "
        );
        assert!(
            support::compile(&bad_bound).is_err(),
            "builtin.Continuation must not implement {bound}"
        );
    }

    let single_use = "
        enum Holder {
            empty,
            full(builtin.Continuation),
        }
        fn consume(k: builtin.Continuation) {}
        fn run() -> bool {
            let opt = Holder.empty;
            opt match {
                Holder.full(k) => {
                    consume(k);
                    true
                },
                Holder.empty => true,
            }
        }
    ";
    let single_pkg = support::compile(single_use).expect("single-use Continuation compiles");
    assert_eq!(
        luna::execution::interpret_with_protocols(&single_pkg, "run", "Copy", "Drop", 10_000_000)
            .expect("single-use Continuation satisfies move checking"),
        Value::Bool(true)
    );

    let double_use = "
        fn run() {
            let a: ?builtin.Continuation = null;
            let _b = a;
            let _c = a;
        }
    ";
    let pkg = support::compile(double_use).expect("type checking succeeds before move analysis");
    let err = luna::execution::interpret_with_protocols(&pkg, "run", "Copy", "Drop", 10_000_000)
        .unwrap_err();
    assert!(
        err.contains("unavailable value"),
        "expected move-checking rejection for double use of builtin.Continuation, got: {err}"
    );
}

#[test]
fn capturing_effect_declarations_and_handlers_emit_abi_capturing() {
    let source = "
        effect yield_number(v: u32, catch continuation: builtin.Continuation);
        effect ask_step(x: u32, catch continuation: builtin.Continuation) -> u32;

        effect enum Stream {
            emit_item(v: u32, catch continuation: builtin.Continuation),
        }

        fn produce() -> #[yield_number, ask_step, Stream.emit_item] u32 {
            yield_number(10)#;
            let next = ask_step(2)#;
            Stream.emit_item(next)#;
            next
        }

        fn handle_resumed() -> u32 {
            produce()# {
                yield_number(_v, _k) => {
                    resume;
                    0
                },
                ask_step(x, _k) => {
                    resume (if x < 100 { x + 1 } else { x });
                    0
                },
                Stream.emit_item(_v, _k) => {
                    resume;
                    0
                },
            }
        }

        fn handle_abandoned() -> u32 {
            produce()# {
                yield_number(v, _) => v,
                ask_step(x, _) => x,
                Stream.emit_item(v, _) => v,
            }
        }
    ";

    let pkg = support::compile(source).expect("capturing effects and handlers should compile");

    let user_effects: Vec<_> = pkg
        .effects
        .iter()
        .filter(|eff| eff.declaration.package == 0)
        .collect();
    assert_eq!(user_effects.len(), 3);
    for eff in &user_effects {
        assert_eq!(
            eff.abi,
            Abi::Capturing,
            "effect `{}` must emit Abi::Capturing",
            eff.name
        );
        let Term::Function { parameters, .. } = &eff.signature else {
            panic!("expected function signature for `{}`", eff.name);
        };
        assert_eq!(
            parameters.len(),
            1,
            "requester signature for `{}` must exclude trailing catch parameter",
            eff.name
        );
        assert_ne!(
            parameters[0].ty,
            Term::Primitive(Primitive::Continuation),
            "requester parameter must not be Continuation"
        );
    }

    let produce = find_source_fn(&pkg, "produce");
    assert!(contains_expr(&produce.body, &|expr| matches!(
        &expr.kind,
        ExprKind::EffectRequest {
            abi: Abi::Capturing,
            ..
        }
    )));

    let handle_resumed = find_source_fn(&pkg, "handle_resumed");
    assert!(contains_expr(&handle_resumed.body, &|expr| {
        if let ExprKind::EffectHandle { handlers, .. } = &expr.kind {
            handlers.len() == 3 && handlers.iter().all(|h| h.abi == Abi::Capturing)
        } else {
            false
        }
    }));
    assert!(contains_expr(&handle_resumed.body, &|expr| matches!(
        &expr.kind,
        ExprKind::EffectResume(_)
    )));

    let handle_abandoned = find_source_fn(&pkg, "handle_abandoned");
    assert!(contains_expr(&handle_abandoned.body, &|expr| {
        if let ExprKind::EffectHandle { handlers, .. } = &expr.kind {
            handlers.len() == 3 && handlers.iter().all(|h| h.abi == Abi::Capturing)
        } else {
            false
        }
    }));
}

#[test]
fn diverging_body_with_expected_type_uses_capturing_arm_type() {
    let source = "
        effect abort_op(v: u32, catch continuation: builtin.Continuation) -> NoReturn;

        fn fail() -> #abort_op NoReturn {
            abort_op(42)#
        }

        fn handle_with_expected() -> u32 {
            let out: u32 = fail()# {
                abort_op(v, _k) => v,
            };
            if out == 42 { out } else { 0 }
        }
    ";

    let pkg = support::compile(source).expect("diverging body with capturing arm should compile");
    let handle_fn = find_source_fn(&pkg, "handle_with_expected");
    assert!(contains_expr(&handle_fn.body, &|expr| {
        matches!(
            (&expr.kind, &expr.ty),
            (
                ExprKind::EffectHandle { .. },
                Term::Primitive(Primitive::U32)
            )
        )
    }));
}

#[test]
fn catch_effect_and_direct_passing_rejections() {
    let cases = [
        (
            "ordinary fn with catch",
            "fn bad(x: u32, catch k: builtin.Continuation) {}",
            "catch",
        ),
        (
            "direct_passing effect with catch",
            "^attrs.effects.direct_passing effect bad(catch k: builtin.Continuation);",
            "direct passing",
        ),
        (
            "direct_passing effect with parameter",
            "^attrs.effects.direct_passing effect bad(x: u32);",
            "direct passing",
        ),
        (
            "direct_passing effect enum variant with catch",
            "effect enum Bad { ^attrs.effects.direct_passing op(catch k: builtin.Continuation), }",
            "direct passing",
        ),
        (
            "async effect with catch",
            "async effect bad(catch k: builtin.Continuation);",
            "catch",
        ),
        (
            "catch with non-Continuation type",
            "effect bad(v: u32, catch k: u32);",
            "Continuation",
        ),
        (
            "non-trailing catch parameter",
            "effect bad(catch k: builtin.Continuation, v: u32);",
            "catch",
        ),
        (
            "moving continuation and resuming in same path",
            "
            effect yield_number(v: u32, catch continuation: builtin.Continuation);
            fn consume(k: builtin.Continuation) {}
            fn step() -> #yield_number u32 {
                yield_number(1)#;
                42
            }
            fn run() -> u32 {
                step()# {
                    yield_number(v, k) => {
                        consume(k);
                        resume;
                        v
                    },
                }
            }
            ",
            "cannot both move continuation binding and resume it",
        ),
        (
            "moving continuation in pattern guard and resuming in arm body",
            "
            effect yield_number(v: u32, catch continuation: builtin.Continuation);
            fn consume_guard(k: builtin.Continuation) -> bool { true }
            fn step() -> #yield_number u32 {
                yield_number(1)#;
                42
            }
            fn run() -> u32 {
                step()# {
                    yield_number(v, k) if consume_guard(k) => {
                        resume;
                        v
                    },
                    yield_number(v, _k) => v,
                }
            }
            ",
            "cannot both move continuation binding and resume it",
        ),
    ];

    for (label, source, expected_fragment) in cases {
        let err = support::compile(source)
            .err()
            .unwrap_or_else(|| panic!("expected rejection for `{label}`"));
        assert!(
            err.contains(expected_fragment),
            "`{label}` failed with unexpected error (expected `{expected_fragment}`): {err}"
        );
    }
}
