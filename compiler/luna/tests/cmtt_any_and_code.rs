//! CMTT Any/any code evidence recovery, variadic code collections, and meta code sysroot API.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

use comptime_interpretor::Value;

#[test]
fn any_code_evidence_recovers_expr_and_pattern_contracts() {
    let source = "
        comptime fn keep(value: Any) -> Any { value }
        comptime fn choose(flag: bool, left: Any, right: Any) -> Any {
            if flag { left } else { right }
        }
        fn add_one(value: i32) -> i32 where requires value >= 0 and value <= 100 {
            const one: i32 = 1;
            const code: Any = keep('(value + 1));
            let _ = one;
            ~<code>
        }
        fn gradual_result(value: i32) -> i32 where requires value >= 0 and value <= 100 {
            const code: < value: i32 |- Any > = '(value + 2);
            ~<code>
        }
        fn runtime_choice(flag: bool) -> u32 {
            const left: Any = '(do { let v: u32 = 1; v });
            const right: Any = '(do { let v: u32 = 2; v });
            if flag {
                ~<choose(true, left, right)>
            } else {
                ~<choose(false, left, right)>
            }
        }
        fn pattern_via_any() -> i32 {
            const raw: < i32 -| x: i32 > = '<x>;
            const wrapped: Any = keep(raw);
            let ~[x = x]<wrapped> = 41;
            if x == 41 { x + 1 } else { 0 }
        }
        fn run_all() -> i32 {
            let a = add_one(10);
            let b = gradual_result(20);
            let c: i32 = if runtime_choice(true) == 1 { 1 } else { 0 };
            let d = pattern_via_any();
            if a == 11 and b == 22 and c == 1 and d == 42 {
                76
            } else {
                0
            }
        }
    ";
    assert_eq!(
        sysroot::run_with_inputs(source, "add_one", vec![Value::SignedInteger(4)]),
        Value::SignedInteger(5)
    );
    assert_eq!(
        sysroot::run_with_inputs(source, "gradual_result", vec![Value::SignedInteger(4)]),
        Value::SignedInteger(6)
    );
    assert_eq!(
        sysroot::run_with_inputs(source, "runtime_choice", vec![Value::Bool(true)]),
        Value::UnsignedInteger(1)
    );
    assert_eq!(
        sysroot::run_with_inputs(source, "runtime_choice", vec![Value::Bool(false)]),
        Value::UnsignedInteger(2)
    );
    assert_eq!(
        sysroot::run_with_inputs(source, "pattern_via_any", vec![]),
        Value::SignedInteger(42)
    );
    assert_eq!(
        sysroot::run_with_inputs(source, "run_all", vec![]),
        Value::SignedInteger(11 + 22 + 1 + 42)
    );
}

#[test]
fn any_code_evidence_negative_cases_reject_invalid_result_kind_branch_and_missing_body() {
    let wrong_result = "
        comptime fn keep(value: Any) -> Any { value }
        fn bad(value: i32) -> i32 {
            const code: Any = keep('(true));
            ~<code>
        }
    ";
    assert!(sysroot::compile(wrong_result).is_err());

    let wrong_kind_pattern_in_expr = "
        fn bad() -> i32 {
            const pat: Any = '<value>;
            ~<pat>
        }
    ";
    assert!(sysroot::compile(wrong_kind_pattern_in_expr).is_err());

    let wrong_kind_expr_in_pattern = "
        fn bad() -> i32 {
            const expr_code: Any = '(1);
            let ~<expr_code> = 1;
            1
        }
    ";
    assert!(sysroot::compile(wrong_kind_expr_in_pattern).is_err());

    let missing_program_direct = "
        fn bad() -> i32 {
            ~<.any'builtin>
        }
    ";
    assert!(sysroot::compile(missing_program_direct).is_err());

    let missing_program_bound = "
        fn bad() -> i32 {
            const empty: Any = .any'builtin;
            ~<empty>
        }
    ";
    assert!(sysroot::compile(missing_program_bound).is_err());

    let late_choice = "
        fn bad(flag: bool) -> u32 {
            const code: Any = if flag { '(1) } else { '(2) };
            ~<code>
        }
    ";
    assert!(sysroot::compile(late_choice).is_err());

    let bad_else_branch = "
        comptime fn pick(flag: bool) -> Any {
            if flag {
                '(do { let v: u32 = 1; v })
            } else {
                '(do { let x: u32 = ~<.any'builtin>; x })
            }
        }
        fn bad() -> u32 { ~<pick(true)> }
    ";
    let err = sysroot::compile(bad_else_branch).unwrap_err();
    assert!(!err.contains("parsing failed"), "{err}");

    let bad_when_arm = "
        comptime fn pick(flag: bool) -> Any {
            inline when {
                flag => '(do { let v: u32 = 1; v }),
                else => '(do { let x: u32 = ~<.any'builtin>; x }),
            }
        }
        fn bad() -> u32 { ~<pick(true)> }
    ";
    let err = sysroot::compile(bad_when_arm).unwrap_err();
    assert!(!err.contains("parsing failed"), "{err}");
}

#[test]
fn variadic_code_collection_cannot_leak_as_tuple_data() {
    let copy_in_generator = "
        pure comptime fn bad(...items: <|- i32>) -> <|- i32> {
            let copy = items;
            '(do { let v: i32 = 1; v })
        }
        fn run() -> i32 { ~<bad('(1), '(2))> }
    ";
    let err = sysroot::compile(copy_in_generator).unwrap_err();
    assert!(!err.contains("parsing failed"), "{err}");

    let return_in_consumer = "
        pure comptime fn bad(...items: <|- i32>) -> () {
            items
        }
        fn run() -> i32 {
            const _ = bad('(1), '(2));
            0
        }
    ";
    let err = sysroot::compile(return_in_consumer).unwrap_err();
    assert!(!err.contains("parsing failed"), "{err}");

    let quote_collection = "
        pure comptime fn bad(...items: <|- i32>) -> <|- ()> {
            '(items)
        }
        fn run() -> () { ~<bad('(1), '(2))> }
    ";
    let err = sysroot::compile(quote_collection).unwrap_err();
    assert!(!err.contains("parsing failed"), "{err}");
}

#[test]
fn mutable_any_binding_rechecks_actual_reassigned_element() {
    let valid_reassignment = "
        comptime fn overwrite_with_i32() -> <|- i32> {
            var slot: Any = '(true);
            slot = '(do { let v: i32 = 42; v });
            slot
        }
        fn run() -> i32 { ~<overwrite_with_i32()> }
    ";
    assert_eq!(sysroot::run(valid_reassignment), Value::SignedInteger(42));

    let invalid_reassignment = "
        comptime fn overwrite_with_bool() -> <|- i32> {
            var slot: Any = '(do { let v: i32 = 42; v });
            slot = '(true);
            slot
        }
        fn run() -> i32 { ~<overwrite_with_bool()> }
    ";
    assert!(sysroot::compile(invalid_reassignment).is_err());
}

#[test]
fn variadic_code_collections_work_for_expression_and_pattern_generators() {
    let source = "
        pure comptime fn first_expr(head: <|- T>, ...tail: <|- T>) -> <|- T> where T {
            head
        }
        pure comptime fn last_expr(head: <|- T>, ...tail: <|- T>) -> <|- T> where T {
            var current: <|- T> = head;
            inline for elem in tail'tuple_enumerate {
                current = elem;
            }
            current
        }
        pure comptime fn count_exprs(...items: <|- T>) -> i32 where T {
            var total: i32 = 0;
            inline for _ in items'tuple_enumerate {
                total += 1;
            }
            total
        }
        pure comptime fn pick_first_pat(
            head: < i32 -| out: i32 >,
            ...tail: < i32 -| out: i32 >
        ) -> < i32 -| out: i32 > {
            head
        }
        fn run_first() -> i32 { ~<first_expr('(10), '(20), '(30))> }
        fn run_last() -> i32 { ~<last_expr('(10), '(20), '(30))> }
        fn run_count() -> i32 {
            const total = count_exprs<i32>('(1), '(2), '(3));
            total
        }
        fn run_empty_count() -> i32 {
            const total = count_exprs<i32>();
            total
        }
        fn run_pat() -> i32 {
            const p = pick_first_pat('<out>, '<out>);
            let ~<p> = 99;
            out
        }
    ";
    assert_eq!(
        sysroot::run_with_inputs(source, "run_first", vec![]),
        Value::SignedInteger(10)
    );
    assert_eq!(
        sysroot::run_with_inputs(source, "run_last", vec![]),
        Value::SignedInteger(30)
    );
    assert_eq!(
        sysroot::run_with_inputs(source, "run_count", vec![]),
        Value::SignedInteger(3)
    );
    assert_eq!(
        sysroot::run_with_inputs(source, "run_empty_count", vec![]),
        Value::SignedInteger(0)
    );
    assert_eq!(
        sysroot::run_with_inputs(source, "run_pat", vec![]),
        Value::SignedInteger(99)
    );

    let mismatched_element = "
        pure comptime fn first_expr(head: <|- T>, ...tail: <|- T>) -> <|- T> where T {
            head
        }
        fn bad() -> i32 { ~<first_expr('(1), '(true))> }
    ";
    assert!(sysroot::compile(mismatched_element).is_err());
}

#[test]
fn meta_code_sysroot_api_executes_end_to_end() {
    let source = "
        use meta.{
            CodeContext,
            CodeErr,
            CodeInterfaceSummary,
            CodeKind,
            ContextEntry,
            Substitution,
            count_exprs,
            first_expr,
            keep,
            last_expr,
            select_any,
        };

        pure comptime fn check_context_api() -> !CodeErr bool {
            let entry = ContextEntry.new(.x, i32, .has_default = true);
            let ctx0 = CodeContext.empty();
            let ctx1 = ctx0.add(entry)!;
            let summary = CodeInterfaceSummary.expression(ctx1, i32);
            let sub = Substitution.identity(.x);
            summary.is_expression()
                and ctx1.has_port(.x)
                and entry.name() == .x
                and entry.has_default()
                and sub.port() == .x
                and sub.target() == .x
                and summary.context().len() == 1
                and summary.output_names().length() == 0
                and entry.matches_entry(.x, i32)
        }

        fn add_ten(value: i32) -> i32 where requires value >= 0 and value <= 100 {
            const kept: Any = keep('(value + 10));
            ~<kept>
        }

        fn choose_and_collect() -> i32 {
            const picked: Any = select_any(true, '(do { let v: i32 = 7; v }), '(do { let v: i32 = 99; v }));
            let first: i32 = ~<first_expr('(3), '(4))>;
            let last: i32 = ~<last_expr('(3), '(5))>;
            const count = count_exprs<i32>('(1), '(2), '(3));
            const bonus: i32 = if count == 3 { 3 } else { 0 };
            let p: i32 = ~<picked>;
            if p == 7 and first == 3 and last == 5 and bonus == 3 { 18 } else { 0 }
        }

        fn run() -> i32 {
            const ok = check_context_api() match {
                v! => v,
                error _ => false,
            };
            if ok and add_ten(5) == 15 and choose_and_collect() == 18 {
                33
            } else {
                0
            }
        }
    ";
    assert_eq!(
        sysroot::run(source),
        Value::SignedInteger(15 + 7 + 3 + 5 + 3)
    );
}
