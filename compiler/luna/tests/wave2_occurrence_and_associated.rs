//! Wave 2 integration tests covering:
//! - GAP-02-25: post-loop exit-condition occurrence typing refinements
//! - SND-02-03: loop back-edge invalidation barrier for local write epochs & staged facts
//! - GAP-02-20: parameterized & abstract associated values, and cyclic/parameterized parent traits
//! - GAP-02-12: elaboration-side deferred/invocation support for ordinary `comptime fn` inputs
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-wave2-occ-assoc-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn build(&self, opt_level: &str) -> Output {
        ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "build",
                "main.fl",
                "--entry",
                "run",
                "--output",
                "program",
                "--opt-level",
                opt_level,
                "--fuel",
                "10000000",
                "--sysroot",
            ])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library"))
            .output()
            .unwrap()
    }

    fn run(&self) -> Output {
        ProcessCommand::new(self.0.join("program"))
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}

impl Drop for NativeCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn post_loop_exit_condition_refines_type_facts_and_rejects_leaky_breaks() {
    // `while not (T == bool)` proves `T == bool` on normal loop exit.
    let valid_equality = r#"
        comptime fn require_bool_type(comptime T: Type) -> bool where requires T == bool {
            T == bool
        }

        comptime fn ensure_bool(comptime T: Type) -> bool {
            while not (T == bool) {
                return false;
            }
            require_bool_type(T)
        }

        fn run() -> bool {
            const yes = ensure_bool(bool);
            const no = ensure_bool(u8);
            yes and not no
        }
    "#;
    assert_eq!(support::run(valid_equality), Value::Bool(true));

    // Negated disjunction: exiting `while not (A == bool) or not (B == u8)` proves both.
    let valid_disjunction = r#"
        comptime fn require_pair_types(comptime A: Type, comptime B: Type) -> bool
        where requires A == bool and B == u8
        {
            A == bool and B == u8
        }

        comptime fn both_match(comptime A: Type, comptime B: Type) -> bool {
            while not (A == bool) or not (B == u8) {
                return false;
            }
            require_pair_types(A, B)
        }

        fn run() -> bool {
            both_match(bool, u8) and not both_match(bool, u16)
        }
    "#;
    assert_eq!(support::run(valid_disjunction), Value::Bool(true));

    // A runtime `break` inside the body skips the negated exit condition and must NOT prove `T == bool`.
    // Test both `if cond { break; }` and `let ... else { break; }` (`LetPattern.otherwise`).
    for body in [
        "if stop { break; } return false;",
        "let true = stop else { break; }; return false;",
    ] {
        let leaky_break = format!(
            r#"
            comptime fn require_bool_type(comptime T: Type) -> bool where requires T == bool {{
                T == bool
            }}

            comptime fn unsafe_break(comptime T: Type, stop: bool) -> bool {{
                while not (T == bool) {{
                    {body}
                }}
                require_bool_type(T)
            }}

            fn run() -> bool {{
                unsafe_break(u8, true)
            }}
        "#
        );
        let err = support::compile(&leaky_break).unwrap_err();
        assert!(
            err.contains("precondition") || err.contains("established") || err.contains("prove"),
            "unexpected error for leaky break `{body}`: {err}"
        );
    }

    // Native sysroot execution (-O0 and -O3) for post-loop tuple-kind refinement.
    let native_source = r#"
        comptime fn require_tuple_type(comptime T: Type) -> bool
        where requires Type.predicates.is_tuple(T)
        {
            Type.predicates.is_tuple(T)
        }

        comptime fn is_pair(comptime T: Type) -> bool {
            while not Type.predicates.is_tuple(T) {
                return false;
            }
            require_tuple_type(T)
        }

        fn run() -> bool {
            const ok = is_pair(Tuple<u8, bool>);
            const bad = is_pair(u8);
            ok and not bad
        }
    "#;
    for level in ["0", "3"] {
        let case = NativeCase::new(native_source);
        let build = case.build(level);
        assert_success(&build);
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn loop_backedge_barrier_blocks_stale_capture_before_mutation_and_allows_sound_loops() {
    let prefix = r#"
        struct BiasBox
        where quote k: lift u8
        {
            value: u8,
        }

        impl BiasBox<k> where quote k: lift u8 {
            fn total(*self) -> u8 {
                if self.value.* < 100 and k < 100 {
                    self.value.* + k
                } else {
                    0
                }
            }
        }
    "#;

    // Pre-loop capture `b` read before `k = 9` inside a repeating loop is unsound on iteration 2.
    let unsound_direct = format!(
        r#"{prefix}
        fn run() -> u8 {{
            var k: u8 = 1;
            let b = BiasBox<k> {{ value: 10 }};
            var i: u8 = 0;
            var acc: u8 = 0;
            while i < 2 {{
                if acc < 100 {{
                    acc = acc + b.total();
                }}
                k = 9;
                i = i + 1;
            }}
            acc
        }}
        "#
    );
    let err = support::compile(&unsound_direct).unwrap_err();
    assert!(
        err.contains("runtime dependency binding was modified after capture"),
        "unexpected error for loop stale read: {err}"
    );

    // Conditional mutation on one branch still invalidates pre-loop capture across back-edges.
    let unsound_branch = format!(
        r#"{prefix}
        fn run() -> u8 {{
            var k: u8 = 1;
            let b = BiasBox<k> {{ value: 10 }};
            var i: u8 = 0;
            var acc: u8 = 0;
            while i < 2 {{
                if i == 0 {{
                    acc = b.total();
                }} else {{
                    k = 9;
                }}
                if i < 10 {{
                    i = i + 1;
                }}
            }}
            acc
        }}
        "#
    );
    let err = support::compile(&unsound_branch).unwrap_err();
    assert!(
        err.contains("runtime dependency binding was modified after capture"),
        "unexpected error for conditional loop stale read: {err}"
    );

    // Re-creating the dependent value inside the loop before mutating `k` is sound and runs natively.
    let sound_loop_local = format!(
        r#"{prefix}
        fn run() -> bool {{
            var k: u8 = 1;
            var i: u8 = 0;
            var acc: u8 = 0;
            while i < 2 {{
                let b = BiasBox<k> {{ value: 10 }};
                let t = b.total();
                if acc < 100 and t < 100 {{
                    acc = acc + t;
                }}
                k = 9;
                i = i + 1;
            }}
            acc == 30
        }}
        "#
    );
    for level in ["0", "3"] {
        let case = NativeCase::new(&sound_loop_local);
        let build = case.build(level);
        assert_success(&build);
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }

    // Mutating an unrelated binding `i` keeps pre-loop capture `b` valid inside and after the loop.
    let sound_unmutated_capture = format!(
        r#"{prefix}
        fn run() -> bool {{
            var k: u8 = 2;
            let b = BiasBox<k> {{ value: 10 }};
            var i: u8 = 0;
            var acc: u8 = 0;
            while i < 2 {{
                let t = b.total();
                if acc < 100 and t < 100 {{
                    acc = acc + t;
                }}
                i = i + 1;
            }}
            let last = b.total();
            acc < 100 and last < 100 and acc + last == 36
        }}
        "#
    );
    for level in ["0", "3"] {
        let case = NativeCase::new(&sound_unmutated_capture);
        let build = case.build(level);
        assert_success(&build);
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }

    // A non-repeating `while true { ... break; }` (`!loop_may_continue`) allows pre-loop capture
    // before the write inside the single iteration, while still invalidating post-loop uses.
    let single_trip_valid = format!(
        r#"{prefix}
        fn run() -> bool {{
            var k: u8 = 1;
            let b = BiasBox<k> {{ value: 10 }};
            var out: u8 = 0;
            while true {{
                out = b.total();
                k = 9;
                break;
            }}
            out == 11
        }}
        "#
    );
    for level in ["0", "3"] {
        let case = NativeCase::new(&single_trip_valid);
        let build = case.build(level);
        assert_success(&build);
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }

    let single_trip_post_loop_err = format!(
        r#"{prefix}
        fn run() -> u8 {{
            var k: u8 = 1;
            let b = BiasBox<k> {{ value: 10 }};
            while true {{
                k = 9;
                break;
            }}
            b.total()
        }}
        "#
    );
    let err = support::compile(&single_trip_post_loop_err).unwrap_err();
    assert!(
        err.contains("runtime dependency binding was modified after capture"),
        "unexpected post-loop error: {err}"
    );
}

#[test]
fn parameterized_and_abstract_associated_values_and_parent_cycles() {
    // Associated `pure comptime fn` and `for<...>` members, trait defaults, impl overrides,
    // value & angle-bracket invocation, type-returning `for<...> Type`, and abstract trait associated values.
    let valid_assoc = r#"
        trait Scale {
            assoc factor: usize = 3;
            assoc scale: pure comptime fn(usize) -> usize = |n: usize| -> usize {
                if n < 100 and Self.as(Scale).factor < 100 {
                    n * Self.as(Scale).factor
                } else {
                    0
                }
            };
            assoc pick: for<flag: bool> Type = |flag: bool| -> Type { if flag { u8 } else { bool } };
            fn tagged_default(self) -> usize {
                let a = Self.as(Scale).factor;
                let b = factor;
                if a < 100 and b < 100 { a + b } else { 0 }
            }
        }

        trait Tagged where N: usize {
            assoc tag_id: usize = N;
            fn read_tag(self) -> usize {
                let a = tag_id;
                let b = Self.as(Tagged<N>).tag_id;
                if a < 100 and b < 100 { a + b } else { 0 }
            }
        }

        trait DerivedTag :- Tagged<4> {
            fn derived_read(self) -> usize {
                let a = Self.as(Tagged<4>).tag_id;
                if a < 100 { a + 1 } else { 0 }
            }
        }

        struct DefaultUser {}
        impl Scale for DefaultUser {}
        impl Tagged<4> for DefaultUser {}
        impl DerivedTag for DefaultUser {}

        struct TripleUser {}
        impl Scale for TripleUser {
            assoc factor: usize = 5;
            assoc scale: pure comptime fn(usize) -> usize = |m: usize| -> usize {
                if m < 100 { m * 3 } else { 0 }
            };
        }

        fn run() -> bool {
            let u1 = DefaultUser {};
            let u2 = DefaultUser {};
            let u3 = DefaultUser {};
            let a = DefaultUser.as(Scale).scale(4);
            let b = DefaultUser.as(Scale).scale(4);
            let c = TripleUser.as(Scale).scale(4);
            let d = u1.as(Scale).tagged_default();
            let e = u2.as(Tagged<4>).read_tag();
            let f = u3.as(DerivedTag).derived_read();
            const g = DefaultUser.as(Scale).pick<true> == u8
                and DefaultUser.as(Scale).pick<false> == bool;
            a == 12 and b == 12 and c == 12 and d == 6 and e == 8 and f == 5 and g
        }
    "#;
    for level in ["0", "3"] {
        let case = NativeCase::new(valid_assoc);
        let build = case.build(level);
        assert_success(&build);
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }

    // Mismatched `assoc` function signature in an `impl` is rejected.
    let bad_override = r#"
        trait Scale {
            assoc scale: for<n: usize> usize = |n: usize| -> usize { n * 2 };
        }
        struct S {}
        impl Scale for S {
            assoc scale: for<n: bool> usize = |_n: bool| -> usize { 1 };
        }
        fn run() -> usize {
            S.as(Scale).scale<true>
        }
    "#;
    let err = support::compile(bad_override).unwrap_err();
    assert!(
        err.contains("does not match its declared type"),
        "expected mismatched assoc type error, got: {err}"
    );

    // Removed legacy `assoc id<param>: ...` syntax is rejected by parser, and `where` clause params are rejected.
    let bad_legacy =
        support::compile("trait Bad { assoc item<n: usize>: usize = 1; }").unwrap_err();
    assert!(!bad_legacy.is_empty());
    let err = support::compile("trait Bad { assoc item: usize = 1 where T; }").unwrap_err();
    assert!(
        err.contains("where type/clause parameters"),
        "expected explicit clause-param rejection, got: {err}"
    );

    // Genuinely acyclic parameterized parent DAGs (`PSub :- PMid<T> + PBase<u8> where T`, `PMid :- PBase<U> where U`)
    // succeed, while direct and indirect instantiated cycles (`CycleA :- CycleB<U> where U`, `CycleB :- CycleA<u8> where V`,
    // `Chain :- Chain<u8> where T`) and diverging parent chains fail definitively without "not yet implemented".
    let param_parent_ok = r#"
        trait PBase where A {}
        trait PMid :- PBase<U> where U {}
        trait PSub :- PMid<T> + PBase<u8> where T {}
        trait Base {}
        trait Mid :- Base {}
        trait Top :- Mid + Base {}
        fn run() -> bool { true }
    "#;
    assert_eq!(support::run(param_parent_ok), Value::Bool(true));

    for cyclic in [
        "trait A :- B {} trait B :- A {}",
        "trait A :- B<T> where T {} trait B :- A<U> where U {}",
        "trait CycleA :- CycleB<U> where U {} trait CycleB :- CycleA<u8> where V {}",
        "trait Chain :- Chain<u8> where T {}",
        "trait Grow :- Grow<Tuple<T, u8>> where T {}",
    ] {
        let err = support::compile(cyclic).unwrap_err();
        assert!(
            err.contains("cyclic"),
            "expected cyclic parent diagnostic for `{cyclic}`, got: {err}"
        );
        assert!(
            !err.contains("not yet implemented"),
            "diagnostic should no longer say not yet implemented: {err}"
        );
    }
}

#[test]
fn deferred_ordinary_comptime_fn_annotations_elaborate_at_invocation() {
    let source = r#"
        comptime fn Pick(flag: bool) -> Type {
            if flag { u8 } else { bool }
        }

        comptime fn choose_identity(flag: bool, value: defer Pick(flag)) -> defer Pick(flag) {
            value
        }

        fn choose_runtime(comptime flag: bool, value: defer Pick(flag)) -> defer Pick(flag) {
            value
        }

        comptime fn outer(comptime flag: bool) -> bool {
            comptime fn inner() -> bool {
                flag
            }
            inner()
        }

        fn run() -> bool {
            const a: u8 = choose_identity(true, 42);
            const b: bool = choose_identity(false, true);
            let s1: u8 = choose_runtime(true, 9);
            let s2: bool = choose_runtime(false, true);
            const c = outer(true);
            const d = outer(false);
            a == 42 and b and s1 == 9 and s2 and c and not d
        }
    "#;
    for level in ["0", "3"] {
        let case = NativeCase::new(source);
        let build = case.build(level);
        assert_success(&build);
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }

    let wrong_deferred_type = r#"
        comptime fn Pick(flag: bool) -> Type {
            if flag { u8 } else { bool }
        }
        comptime fn choose_identity(flag: bool, value: defer Pick(flag)) -> defer Pick(flag) {
            value
        }
        fn run() -> u8 {
            choose_identity(true, false)
        }
    "#;
    assert!(support::compile(wrong_deferred_type).is_err());
}
