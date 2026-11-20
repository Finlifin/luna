//! Adversarial torture tests for Elaboration Phase 2A P0 soundness fixes.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot_support;

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
            "luna-elab-p0a-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn build(&self) -> Output {
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
                "0",
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

fn compile_with_resources(body: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod resources {{ {} }} use resources.*; {body}",
            include!("support/resources_source.rs")
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input.resources.Copy"),
            drop: Some("input.resources.Drop"),
            ..Default::default()
        },
    )
}

#[test]
fn p0_2_inline_if_is_enforces_once_and_inline_for_delivers_materialized_bindings() {
    let double_use = r#"
        struct Token { flag: bool }
        fn take(t: Token) -> bool { t.flag }
        const TOKEN: Token = Token { flag: true };
        fn bad_inline_if_is() -> bool {
            inline if TOKEN is do {
                tok => take(tok) and take(tok),
            }
        }
    "#;
    let package = compile_with_resources(double_use);
    match package {
        Err(err) => {
            assert!(
                err.contains("move") || err.contains("Copy") || err.contains("unavailable"),
                "{err}"
            );
        }
        Ok(package) => {
            let err = luna::execution::interpret_with_protocols(
                &package,
                "bad_inline_if_is",
                "Copy",
                "Drop",
                100_000,
            )
            .unwrap_err();
            assert!(
                err.contains("unavailable value") || err.contains("Copy"),
                "{err}"
            );
        }
    }

    let valid_inline_if_is_and_for = r#"
        struct Token { flag: bool }
        fn take(t: Token) -> bool { t.flag }
        const TOKEN: Token = Token { flag: true };
        fn good_inline_if_is() -> bool {
            inline if TOKEN is do {
                tok => take(tok),
            }
        }
        fn good_for() -> bool {
            var seen = false;
            inline for (0 and Token { flag: true } is tok) in 0..1 {
                seen = take(tok);
            }
            seen
        }
    "#;
    let valid_pkg = compile_with_resources(valid_inline_if_is_and_for).unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &valid_pkg,
            "good_inline_if_is",
            "Copy",
            "Drop",
            100_000
        )
        .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&valid_pkg, "good_for", "Copy", "Drop", 100_000)
            .unwrap(),
        Value::Bool(true)
    );

    let bad_for = r#"
        struct Token { flag: bool }
        fn take(t: Token) -> bool { t.flag }
        fn bad_for() -> bool {
            var ok = true;
            inline for (0 and Token { flag: true } is tok) in 0..1 {
                ok = take(tok) and take(tok);
            }
            ok
        }
    "#;
    let bad_for_pkg = compile_with_resources(bad_for).unwrap();
    let err =
        luna::execution::interpret_with_protocols(&bad_for_pkg, "bad_for", "Copy", "Drop", 100_000)
            .unwrap_err();
    assert!(
        err.contains("unavailable value") || err.contains("Copy"),
        "{err}"
    );
}

#[test]
fn p0_3_ptc_ensures_type_guarantees_reject_mutated_locals_and_preserve_immutable() {
    let forged_cases = [
        r#"
            comptime fn forge() -> R: Type
            where ensures Type.predicates.is_tuple(R)
            {
                var T: Type = Tuple<bool>;
                T = i32;
                T
            }
            fn run() {}
        "#,
        r#"
            comptime fn forge_alias() -> R: Type
            where ensures Type.predicates.is_tuple(R)
            {
                var T: Type = Tuple<bool>;
                let alias = T;
                T = i32;
                alias
            }
            fn run() {}
        "#,
        r#"
            comptime fn forge_branch(flag: bool) -> R: Type
            where ensures Type.predicates.is_tuple(R)
            {
                var T: Type = Tuple<bool>;
                if flag {
                    T = i32;
                }
                T
            }
            fn run() {}
        "#,
        r#"
            comptime fn forge_loop() -> R: Type
            where ensures Type.predicates.is_tuple(R)
            {
                var T: Type = Tuple<bool>;
                var i: i32 = 0;
                while true {
                    if i == 1 { break; }
                    T = i32;
                    i = 1;
                }
                T
            }
            fn run() {}
        "#,
        r#"
            comptime fn overwrite(p: *Type) { p.* = i32; }
            comptime fn forge_ref() -> R: Type
            where ensures Type.predicates.is_tuple(R)
            {
                var T: Type = Tuple<bool>;
                overwrite(T.ref);
                T
            }
            fn run() {}
        "#,
    ];
    for source in forged_cases {
        let error = sysroot_support::compile(source).expect_err(source);
        assert!(
            !error.contains("parsing failed"),
            "syntax regression in test fixture: {error}"
        );
        assert!(
            error.contains("result guarantee contract is not established"),
            "unexpected error for `{source}`: {error}"
        );
    }

    let valid = r#"
        comptime fn sound_tuple() -> R: Type
        where ensures Type.predicates.is_tuple(R)
        {
            let T: Type = Tuple<bool>;
            T
        }
        comptime fn run() -> bool {
            let T = sound_tuple();
            Type.predicates.is_tuple(T)
        }
    "#;
    assert_eq!(
        sysroot_support::invoke(valid, "run").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn type_result_guarantees_follow_writes_in_ordered_match_tests() {
    let producer = r#"
        comptime fn Choose(flag: bool) -> R: Type
        where ensures Type.predicates.is_tuple(R)
        {
            var chosen: Type = Tuple<bool>;
            MATCH
        }
    "#;
    for selection in [
        "flag match { _ if do { chosen = bool; false } => Tuple<bool>, _ => chosen, }",
        "(flag, true) match { (_ if do { chosen = bool; flag }, _) => chosen, _ => Tuple<bool>, }",
        "(flag, true) match { (_ if do { chosen = bool; flag }, _) => Tuple<bool>, _ => chosen, }",
        "flag match { first and (do { chosen = bool; first }) is true => Tuple<bool>, _ => chosen, }",
        "(true, flag) match { (_ if do { chosen = bool; true }, true) => Tuple<bool>, _ => chosen, }",
    ] {
        let error = sysroot_support::compile(&producer.replace("MATCH", selection)).unwrap_err();
        assert!(
            error.contains("result guarantee contract is not established"),
            "{selection}: {error}"
        );
    }

    // An earlier arm's body cannot run on the fallthrough path. Its guard
    // writes a different local, so both outcomes still establish the contract.
    let active = producer.replace(
        "MATCH",
        r#"
        var other = true;
        flag match {
            _ if do { other = flag; other } => do { chosen = bool; Tuple<u8> },
            _ => chosen,
        }
    "#,
    );
    // Here the pattern can fail before writing. Once the guard executes it
    // always succeeds, so a later arm still sees the original tuple.
    let guaranteed = producer.replace("Choose", "Guarded").replace(
        "MATCH",
        "flag match { true if do { chosen = bool; true } => Tuple<u8>, _ => chosen, }",
    );
    let program = active + &guaranteed + r#"
        comptime fn run() -> bool {
            Choose(true) == Tuple<u8> and Choose(false) == Tuple<bool>
                and Guarded(true) == Tuple<u8> and Guarded(false) == Tuple<bool>
        }
    "#;
    assert_eq!(
        sysroot_support::invoke(&program, "run").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn p0_4_open_function_defer_annotation_stays_isolated_under_caller_binder_premises() {
    let source = r#"
        typealias Type = .Type'builtin;
        fn apply_id(comptime T: Type, value: T) -> defer T {
            value
        }
        fn outer(f: fn(comptime A: Type, x: A) -> A) -> bool
        where B: Type, requires B == bool
        {
            let _: fn(comptime C: Type, x: C) -> C = apply_id;
            apply_id(bool, true) and f(bool, true)
        }
        fn run() -> bool { outer<bool>(apply_id) }
    "#;
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn p0_5_failed_speculative_attempt_does_not_leak_write_versions_or_refinements() {
    let source = r#"
        typealias i32 = .i32'builtin;
        fn helper(x: i32) -> i32
        where
            requires x >= 0 and x < 1000,
        {
            x + 1
        }
        fn pick(a: ?i32) -> i32 {
            var count: i32 = 10;
            let v: i32 = a match {
                n? => if n >= 0 and n < 500 { n } else { 0 },
                null => {
                    count = 20;
                    count
                },
            };
            helper(v + count - 10)
        }
        fn run() -> bool {
            var flag: bool = false;
            let a = (if flag { 1 } else { 2 });
            pick(null) == 31 and a == 2
        }
    "#;
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn p0_6_deferred_runtime_quote_arg_resolves_caller_local_when_earlier_type_defaults() {
    let self_contained = r#"
        struct Alloc {
            pub const GLOBAL: Alloc = Alloc { tag: 7 };
            tag: i32,
        }
        impl Splice for Alloc {
            assoc RuntimeTarget: Type = Alloc;
            fn splice(comptime data: Alloc) -> Alloc { data }
        }
        struct Handle
        where
            T,
            .A: Type = Alloc,
            quote .alloc: lift *A = Alloc.GLOBAL.ref,
        {
            raw: T,
        }
        impl Handle<T, .A = A, .alloc = alloc>
        where
            T,
            A: Type,
            quote alloc: lift *A,
        {
            fn read(self, reader: fn(*A) -> i32) -> i32 {
                reader(alloc)
            }
        }
        fn read_alloc(a: *Alloc) -> i32 { a.tag.* }
        fn make_default_A(x: u32) -> i32 {
            var local_alloc = Alloc { tag: 42 };
            let h = Handle<u32, .alloc = local_alloc.ref> { raw: x };
            h.read(read_alloc)
        }
        fn make_forwarded(x: u32, a: *Alloc) -> i32 {
            let h = Handle<u32, .A = Alloc, .alloc = a> { raw: x };
            h.read(read_alloc)
        }
        fn run() -> bool {
            var second = Alloc { tag: 99 };
            make_default_A(1) == 42 and make_forwarded(2, second.ref) == 99
        }
    "#;
    let self_case = NativeCase::new(self_contained);
    let self_build = self_case.build();
    assert!(
        self_build.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&self_build.stdout),
        String::from_utf8_lossy(&self_build.stderr)
    );
    let self_output = self_case.run();
    assert!(
        self_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&self_output.stdout),
        String::from_utf8_lossy(&self_output.stderr)
    );
    assert_eq!(self_output.stdout, b"(bool true)\n");

    let sysroot_box = r#"
        use std.mem.{Box, DefaultAllocator};
        fn check_box() -> bool {
            var a = DefaultAllocator.GLOBAL;
            let b = Box<u32, .allocator = a.ref>.new(123) match {
                value! => value,
                error _ => return false,
            };
            b.ref.get() == 123
        }
        fn run() -> bool {
            check_box()
        }
    "#;
    let case = NativeCase::new(sysroot_box);
    let build = case.build();
    assert!(
        build.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let output = case.run();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn p0_7_quote_blocks_pure_comptime_trait_and_selects_runtime_counterpart() {
    let leak_into_quote = r#"
        pure comptime trait OnlyComptime {
            comptime fn flag(self) -> bool;
        }
        struct Item { v: bool }
        comptime impl OnlyComptime for Item {
            comptime fn flag(self) -> bool { self.v }
        }
        quote fn bad(item: Item) -> bool {
            item.flag()
        }
    "#;
    assert!(support::compile_with_protocols(leak_into_quote, &Default::default()).is_err());

    let leak_into_comptime_quote_expr = r#"
        pure comptime trait OnlyComptime {
            comptime fn flag(self) -> bool;
        }
        struct Item { v: bool }
        comptime impl OnlyComptime for Item {
            comptime fn flag(self) -> bool { self.v }
        }
        comptime fn build_bad() -> <|- bool> {
            '(Item { v: true }.flag())
        }
        fn run() -> bool { ~<build_bad()> }
    "#;
    assert!(
        support::compile_with_protocols(leak_into_comptime_quote_expr, &Default::default())
            .is_err()
    );

    let dual_phase = r#"
        trait Eval {
            fn eval(self) -> i32;
        }
        pure comptime trait ComptimeEval {
            comptime fn eval(self) -> i32;
        }
        struct Num { v: i32 }
        impl Eval for Num {
            fn eval(self) -> i32 {
                if self.v >= 0 and self.v < 1000 { self.v + 1 } else { 0 }
            }
        }
        comptime impl ComptimeEval for Num {
            comptime fn eval(self) -> i32 {
                if self.v >= 0 and self.v < 1000 { self.v + 100 } else { 0 }
            }
        }
        quote fn runtime_eval(n: Num) -> i32 {
            n.eval()
        }
        comptime fn gen_quoted(n: Num) -> <|- i32> {
            '(Num { v: n.v }.eval())
        }
        fn run() -> bool {
            let ct = inline if (Num { v: 5 }.eval() == 105) { true } else { false };
            let rt = runtime_eval(Num { v: 5 }) == 6;
            let sp = ~<gen_quoted(Num { v: 5 })> == 6;
            ct and rt and sp
        }
    "#;
    assert_eq!(sysroot_support::run(dual_phase), Value::Bool(true));
}

#[test]
fn p0_8_closure_snapshot_preserves_contract_and_guards_body_scope() {
    let valid_contract = r#"
        typealias Type = .Type'builtin;
        trait Tag {
            comptime fn code() -> bool;
        }
        impl Tag for bool {
            comptime fn code() -> bool { true }
        }
        comptime fn check_tag() -> bool where T, requires T :- Tag {
            true
        }
        pure comptime fn Holder<action: %{} comptime fn() -> bool> -> Type { bool }
        fn apply(
            comptime action: %{} comptime fn() -> bool,
            _tag: ?Holder<action>,
        ) -> bool {
            const answer = action();
            answer
        }
        fn forward() -> bool where T, requires T :- Tag {
            apply(|| check_tag<T>(), null)
        }
        fn run() -> bool {
            forward<bool>()
        }
    "#;
    let package = support::compile_with_protocols(valid_contract, &Default::default())
        .expect("closure snapshot with valid trait contract should compile");
    let snapshot_fn = package
        .functions
        .iter()
        .find(|f| f.interface.name == "<closure-snapshot>")
        .expect("expected generated <closure-snapshot> factory");
    assert!(
        !snapshot_fn.interface.capabilities.is_empty()
            || !snapshot_fn.interface.requirements.is_empty(),
        "closure snapshot erased its enclosing trait contract: {}",
        snapshot_fn.interface
    );
    let specialized_apply = package
        .functions
        .iter()
        .find(|f| f.interface.name == "apply" && !f.interface.type_parameters.is_empty())
        .expect("expected specialized open apply instance");
    assert!(
        !specialized_apply.interface.capabilities.is_empty()
            || !specialized_apply.interface.requirements.is_empty(),
        "specialized open function erased captured caller trait contract: {}",
        specialized_apply.interface
    );
    let bad_caller = valid_contract.replace("forward<bool>()", "forward<?bool>()");
    assert!(
        support::compile_with_protocols(&bad_caller, &Default::default()).is_err(),
        "specialized closure snapshot and open apply must reject callers without Tag"
    );

    let out_of_scope_rigid = r#"
        typealias i32 = .i32'builtin;
        typealias Type = .Type'builtin;
        pure comptime fn Id<X: i32> -> Type { bool }
        pure comptime fn Holder<action: %{} comptime fn() -> bool> -> Type { bool }
        fn apply(
            comptime action: %{} comptime fn() -> bool,
            _tag: ?Holder<action>,
        ) -> bool {
            const answer = action();
            answer
        }
        fn forward() -> bool where M: i32 {
            apply(|| { let _: ?Id<M> = null; true }, null)
        }
        fn run() -> bool {
            forward<7>()
        }
    "#;
    let err = support::compile_with_protocols(out_of_scope_rigid, &Default::default()).unwrap_err();
    assert!(
        err.contains("out-of-scope") || err.contains("closure snapshot"),
        "{err}"
    );
}
