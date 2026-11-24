#[path = "support/sysroot_execution.rs"]
mod sysroot;

use comptime_interpretor::Value;

#[test]
fn purity_attributes_accept_valid_p0_through_p5_runtime_functions() {
    let source = r#"
        ^attrs.volatile struct VolFlag { value: bool }
        static mut_flag: bool = true;
        static mut_reg: VolFlag = VolFlag { value: false };
        const imm_u32: u32 = 10;

        ^attrs.purity.p0
        fn p0_volatile() -> bool {
            mut_reg.value
        }

        ^attrs.purity.p1
        fn p1_mutate(p: *u32) -> u32 {
            p.* = 7;
            p.*
        }

        ^attrs.purity.p2
        fn p2_read_global_and_ptr(p: *u32) -> bool {
            mut_flag and (p.* == 7)
        }

        ^attrs.purity.p3
        fn p3_stable_read(p: *u32) -> bool {
            p.* == imm_u32
        }

        ^attrs.purity.p4
        fn p4_local_loop(limit: u32) -> u32 {
            var acc: u32 = 0;
            var i: u32 = 0;
            let cap: u32 = if limit < 10 { limit } else { 10 };
            while i < cap {
                if acc < 1000 {
                    acc = acc + i;
                }
                i = i + 1;
            }
            acc
        }

        ^attrs.purity.p5
        fn p5_pick_max(a: u32, b: u32) -> u32 {
            if a < b { b } else { a }
        }

        ^attrs.purity.p5
        fn p5_caller(a: u32, b: u32) -> bool {
            p5_pick_max(a, b) == b
        }

        fn run() -> bool {
            var x: u32 = 10;
            let a = p1_mutate(x.ref);
            let mut_y: u32 = 10;
            let c = p3_stable_read(mut_y.ref);
            let d = p4_local_loop(4) == 6;
            let e = p5_caller(3, 9);
            (a == 7) and c and d and e
        }

        fn static_entry() -> bool {
            var x: u32 = 10;
            let a = p1_mutate(x.ref);
            let b = p2_read_global_and_ptr(x.ref);
            let _ = p0_volatile();
            (a == 7) and b
        }
    "#;
    sysroot::compile(source).unwrap();
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn purity_p5_rejects_loops_recursion_traps_preconditions_and_non_p5_calls() {
    let cases = [
        (
            "loop",
            "^attrs.purity.p5 fn bad(n: u32) -> u32 { var i: u32 = n; while i > 0 { i = i - 1; } i }",
        ),
        (
            "recursion",
            "^attrs.purity.p5 fn bad(n: u32) -> u32 { if n == 0 { 0 } else { bad(n) } }",
        ),
        (
            "trapping add",
            "^attrs.purity.p5 fn bad(a: u32, b: u32) -> u32 { a + b }",
        ),
        (
            "trapping div",
            "^attrs.purity.p5 fn bad(a: u32, b: u32) -> u32 { a / b }",
        ),
        (
            "precondition",
            "^attrs.purity.p5 fn bad(n: u32) -> u32 where requires n < 10 { n }",
        ),
        (
            "non-p5 helper call",
            "^attrs.purity.p4 fn helper(a: u32, b: u32) -> u32 { a + b } ^attrs.purity.p5 fn bad(x: u32) -> u32 { helper(x, 1) }",
        ),
        (
            "external pointer read",
            "^attrs.purity.p5 fn bad(p: *u32) -> u32 { p.* }",
        ),
        (
            "bit writeback assignment",
            "^attrs.purity.p5 fn bad() -> u8 { var x: u8 = 0; x'bitvec[0] = 1; x }",
        ),
    ];
    for (label, source) in cases {
        let err = sysroot::compile(source).expect_err(label);
        assert!(
            err.contains("p5") || err.contains("P5") || err.contains("P4"),
            "{label}: unexpected error: {err}"
        );
    }
}

#[test]
fn purity_p1_through_p4_reject_tier_violations() {
    let cases = [
        (
            "p4 rejects external pointer read",
            "^attrs.purity.p4 fn bad(p: *u32) -> u32 { p.* }",
        ),
        (
            "p4 rejects external pointer write",
            "^attrs.purity.p4 fn bad(p: *u32) -> bool { p.* = 1; true }",
        ),
        (
            "p4 rejects static global",
            "static g: bool = true; ^attrs.purity.p4 fn bad() -> bool { g }",
        ),
        (
            "p4 rejects impure callee",
            "fn impure(p: *u32) -> u32 { p.* } ^attrs.purity.p4 fn bad(p: *u32) -> u32 { impure(p) }",
        ),
        (
            "p3 rejects static global read",
            "static g: bool = true; ^attrs.purity.p3 fn bad() -> bool { g }",
        ),
        (
            "p3 rejects external pointer write",
            "^attrs.purity.p3 fn bad(p: *u32) -> bool { p.* = 1; true }",
        ),
        (
            "p3 rejects pointer-to-pointer indirection bypass",
            "^attrs.purity.p3 fn bad(ext: *u32) -> bool { var local: u32 = 0; var p = local.ref; let pp = p.ref; pp.* = ext; p.* = 1; true }",
        ),
        (
            "p2 rejects external pointer write",
            "^attrs.purity.p2 fn bad(p: *u32) -> bool { p.* = 1; true }",
        ),
        (
            "p2 rejects pointer-to-pointer indirection bypass",
            "^attrs.purity.p2 fn bad(ext: *u32) -> bool { var local: u32 = 0; var p = local.ref; let pp = p.ref; pp.* = ext; p.* = 1; true }",
        ),
        (
            "p2 rejects static global write",
            "static g: bool = false; ^attrs.purity.p2 fn bad() -> bool { g = true; true }",
        ),
        (
            "p1 rejects volatile access",
            "^attrs.volatile struct Reg { value: bool } static r: Reg = Reg { value: false }; ^attrs.purity.p1 fn bad() -> bool { r.value }",
        ),
        (
            "p1 rejects opaque impure indirect call",
            "^attrs.purity.p1 fn bad(cb: fn(u32) -> u32, x: u32) -> u32 { cb(x) }",
        ),
    ];
    for (label, source) in cases {
        let err = sysroot::compile(source).expect_err(label);
        assert!(
            err.contains("purity") || err.contains("P4"),
            "{label}: unexpected error: {err}"
        );
    }
}

#[test]
fn invalid_purity_attribute_targets_domains_and_configuration_are_rejected() {
    let cases = [
        (
            "struct target",
            "^attrs.purity.p4 struct Bad { x: u32 }",
            "__flurry_purity",
        ),
        (
            "comptime fn target",
            "^attrs.purity.p4 comptime fn bad() -> u32 { 1 }",
            "__flurry_purity",
        ),
        (
            "invalid tier symbol",
            "^{ __flurry_purity: .p9 } fn bad() -> u32 { 1 }",
            "__flurry_purity",
        ),
        (
            "invalid domain integer",
            "^{ __flurry_purity: 4 } fn bad() -> u32 { 1 }",
            "__flurry_purity",
        ),
        (
            "definition configuration syntax",
            "struct ConfigPurity { .__flurry_purity .p4, x: u32 }",
            "__flurry_purity",
        ),
    ];
    for (label, source, needle) in cases {
        let err = sysroot::compile(source).expect_err(label);
        assert!(
            err.contains(needle) || err.contains("configuration"),
            "{label}: unexpected error: {err}"
        );
    }
}

#[test]
fn pure_fn_types_work_as_first_class_values_coerce_to_fn_and_allow_pure_calls() {
    let source = r#"
        pure fn pure_inc(x: u32) -> u32 {
            if x < 1000 { x + 1 } else { x }
        }

        pure fn apply_pure(cb: pure fn(u32) -> u32, x: u32) -> u32 {
            cb(x)
        }

        fn apply_ordinary(cb: fn(u32) -> u32, x: u32) -> u32 {
            cb(x)
        }

        fn run() -> bool {
            let f: pure fn(u32) -> u32 = pure_inc;
            let direct = f(41) == 42;
            let via_pure = apply_pure(f, 41) == 42;
            let widened_fn = apply_ordinary(pure_inc, 41) == 42;
            let widened_val = apply_ordinary(f, 41) == 42;
            let closure_pure: pure fn(u32) -> u32 = |x| if x < 1000 { x + 2 } else { x };
            let closure_ok = apply_pure(closure_pure, 40) == 42;
            direct and via_pure and widened_fn and widened_val and closure_ok
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn impure_functions_and_closures_and_pure_unsafe_fn_types_are_rejected() {
    let cases = [
        (
            "assign impure fn to pure fn local",
            "fn impure(x: u32) -> u32 { x } fn run() -> u32 { let f: pure fn(u32) -> u32 = impure; f(1) }",
        ),
        (
            "pass impure fn to pure fn parameter",
            "fn impure(p: *u32) -> u32 { p.* } fn take(f: pure fn(*u32) -> u32, p: *u32) -> u32 { f(p) } fn run(p: *u32) -> u32 { take(impure, p) }",
        ),
        (
            "pass ordinary fn parameter to pure fn parameter",
            "fn take(f: pure fn(u32) -> u32, x: u32) -> u32 { f(x) } fn relay(g: fn(u32) -> u32, x: u32) -> u32 { take(g, x) }",
        ),
        (
            "assign side-effecting closure to pure fn",
            "fn run() -> bool { let f: pure fn(*bool) -> bool = |p| { p.* = true; true }; var b = false; f(b.ref) }",
        ),
        (
            "static pure closure with side effect",
            "static EVIL: pure fn(*bool) -> bool = |p| { p.* = true; true }; fn run() -> bool { true }",
        ),
        (
            "pure unsafe fn type is invalid",
            "fn take(f: pure unsafe fn(u32) -> u32, x: u32) -> u32 { x }",
        ),
    ];
    for (label, source) in cases {
        let _err = sysroot::compile(source).expect_err(label);
    }
}
