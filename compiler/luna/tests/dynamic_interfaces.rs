use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-dyn-{}-{}",
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

    fn check(&self) -> Output {
        ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "check",
                "main.fl",
                "--entry",
                "run",
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
        "command failed ({:?})\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_rejected(source: &str, label: &str) {
    assert_rejected_with(source, label, &[]);
}

fn assert_rejected_with(source: &str, label: &str, expected_substrings: &[&str]) {
    let case = NativeCase::new(source);
    let output = case.build("0");
    assert!(
        !output.status.success(),
        "{label} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    for expected in expected_substrings {
        assert!(
            stderr.contains(expected),
            "{label} failed with unexpected error (expected `{expected}`):\n{stderr}"
        );
    }
    let check = case.check();
    assert!(
        !check.status.success(),
        "{label} check unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn pointer_dynamic_views_and_layout_work_native_o0_and_o3() {
    let source = r#"
        trait Reading {
            fn read(*self) -> i64;
        }

        trait MutableReading {
            fn read(*self) -> i64;
            fn add(*self, delta: i64) -> void;
        }

        struct Gauge {
            value: i64,
        }

        impl Reading for Gauge {
            fn read(*self) -> i64 {
                self.value.*
            }
        }

        impl MutableReading for Gauge {
            fn read(*self) -> i64 {
                self.value.*
            }

            fn add(*self, delta: i64) -> void {
                if self.value.* >= -10000 and self.value.* <= 10000 and delta >= -10000 and delta <= 10000 {
                    self.*.value = self.value.* + delta;
                }
            }
        }

        struct Fixed {
            constant: i64,
        }

        impl Reading for Fixed {
            fn read(*self) -> i64 {
                self.constant.*
            }
        }

        fn sum_twice(a: dyn Reading, b: dyn Reading) -> i64 {
            let x = a.read();
            let y = b.read();
            if x >= -10000 and x <= 10000 and y >= -10000 and y <= 10000 {
                (x + y) * 2
            } else {
                0
            }
        }

        fn run() -> bool {
            let usize_bytes = Type.size_of(usize);
            let layout_ok = if usize_bytes <= 1024 {
                Type.size_of(dyn Reading) == 2 * usize_bytes
            } else {
                false
            };
            var gauge = Gauge { value: 19 };
            var fixed = Fixed { constant: 23 };
            let total = sum_twice(gauge.ref.dyn(Reading), fixed.ref.dyn(Reading));

            let view = gauge.ref.dyn(MutableReading);
            view.add(11);
            let updated = view.read();

            layout_ok and total == 84 and updated == 30 and gauge.value == 30
        }
    "#;

    for opt in ["0", "3"] {
        let case = NativeCase::new(source);
        assert_success(&case.build(opt));
        let result = case.run();
        assert_success(&result);
        assert_eq!(result.stdout, b"(bool true)\n", "failed at -O{opt}");
    }
}

#[test]
fn owned_value_encapsulation_runs_concrete_drop_on_scope_exit() {
    let source = r#"
        trait Reading {
            fn read(*self) -> i64;
        }

        struct Tracker {
            drops: i64,
        }

        struct Meter {
            log: *Tracker,
            value: i64,
        }

        impl Reading for Meter {
            fn read(*self) -> i64 {
                self.value.*
            }
        }

        impl Drop for Meter {
            fn drop(*self) {
                let log = self.*.log;
                if log.drops.* >= 0 and log.drops.* < 100 {
                    log.*.drops = log.drops.* + 1;
                }
            }
        }

        fn read_and_drop(r: dyn Reading) -> i64 {
            r.read()
        }

        fn run() -> bool {
            var tracker = Tracker { drops: 0 };
            let m = Meter { log: tracker.ref, value: 42 };
            let v = read_and_drop(m.dyn(Reading));
            v == 42 and tracker.drops == 1
        }
    "#;

    for opt in ["0", "3"] {
        let case = NativeCase::new(source);
        assert_success(&case.build(opt));
        let result = case.run();
        assert_success(&result);
        assert_eq!(result.stdout, b"(bool true)\n", "failed at -O{opt}");
    }
}

#[test]
fn runtime_env_carrying_dyn_trait_preserves_distinct_envs() {
    let source = r#"
        trait Reading {
            fn read(*self) -> i64;
        }

        struct Scale {
            factor: i64,
        }

        struct Calibrated
        where quote scale: lift *Scale
        {
            raw: i64,
        }

        impl Reading for Calibrated<scale>
        where quote scale: lift *Scale
        {
            fn read(*self) -> i64 {
                if self.raw.* >= -1000 and self.raw.* <= 1000
                    and scale.factor.* >= -1000 and scale.factor.* <= 1000
                {
                    self.raw.* * scale.factor.*
                } else {
                    0
                }
            }
        }

        fn relay_reading(reading: %c dyn Reading) -> %c dyn Reading
        where c: Type.vars.Closure
        {
            reading
        }

        fn run() -> bool {
            var s1 = Scale { factor: 3 };
            var s2 = Scale { factor: 5 };
            let r1 = relay_reading(Calibrated<s1.ref> { raw: 7 }.dyn(Reading));
            let r2 = relay_reading(Calibrated<s2.ref> { raw: 11 }.dyn(Reading));
            r1.read() == 21 and r2.read() == 55
        }
    "#;

    for opt in ["0", "3"] {
        let case = NativeCase::new(source);
        assert_success(&case.build(opt));
        let result = case.run();
        assert_success(&result);
        assert_eq!(result.stdout, b"(bool true)\n", "failed at -O{opt}");
    }
}

#[test]
fn self_return_reencapsulates_with_owned_drop_under_o0_and_o3() {
    let source = r#"
        trait Duplicate {
            fn duplicate(*self) -> Self;
            fn value(*self) -> i64;
        }

        struct Tracker {
            drops: i64,
        }

        struct Token {
            log: *Tracker,
            n: i64,
        }

        impl Duplicate for Token {
            fn duplicate(*self) -> Token {
                Token {
                    log: self.log.*,
                    n: if self.n.* >= -10000 and self.n.* <= 10000 { self.n.* + 1 } else { 0 },
                }
            }

            fn value(*self) -> i64 {
                self.n.*
            }
        }

        impl Drop for Token {
            fn drop(*self) {
                let log = self.*.log;
                if log.drops.* >= 0 and log.drops.* < 100 {
                    log.*.drops = log.drops.* + 1;
                }
            }
        }

        fn clone_value(source: dyn Duplicate) -> i64 {
            let copy = source.duplicate();
            copy.value()
        }

        fn run() -> bool {
            var tracker = Tracker { drops: 0 };
            var token = Token { log: tracker.ref, n: 10 };
            let from_view = clone_value(token.ref.dyn(Duplicate));
            let drops_after_view = tracker.drops;
            let from_owned = clone_value(token.dyn(Duplicate));
            from_view == 11 and drops_after_view == 1 and from_owned == 11 and tracker.drops == 3
        }
    "#;

    for opt in ["0", "3"] {
        let case = NativeCase::new(source);
        assert_success(&case.build(opt));
        let result = case.run();
        assert_success(&result);
        assert_eq!(result.stdout, b"(bool true)\n", "failed at -O{opt}");
    }
}

#[test]
fn dynamic_interface_rejections_enforce_encapsulation_and_lifetimes() {
    // 1. Dangling local pointer escaping as `dyn Reading`.
    let dangling_escape = r#"
        trait Reading {
            fn read(*self) -> i64;
        }

        struct Gauge {
            value: i64,
        }

        impl Reading for Gauge {
            fn read(*self) -> i64 {
                self.value.*
            }
        }

        fn escape() -> dyn Reading {
            var local = Gauge { value: 9 };
            local.ref.dyn(Reading)
        }

        fn run() -> i64 {
            escape().read()
        }
    "#;
    assert_rejected(dangling_escape, "dangling local.ref.dyn(Reading)");

    // 2. Silently dropping non-empty `%c` into bare `dyn Reading`.
    let drop_env_to_bare_dyn = r#"
        trait Reading {
            fn read(*self) -> i64;
        }

        struct Scale {
            factor: i64,
        }

        struct Calibrated {
            raw: i64,
        }

        impl Reading for Calibrated
        where quote scale: lift *Scale
        {
            fn read(*self) -> i64 {
                self.raw.* * scale.factor.*
            }
        }

        fn drop_env(scale: *Scale, sensor: Calibrated) -> dyn Reading {
            sensor.dyn(Reading)
        }

        fn run() -> i64 {
            var s = Scale { factor: 2 };
            drop_env(s.ref, Calibrated { raw: 5 }).read()
        }
    "#;
    assert_rejected(
        drop_env_to_bare_dyn,
        "silent drop of non-empty %c into bare dyn Trait",
    );

    // 3. Method taking non-receiver `Self` parameter cannot be dynamically dispatched.
    let self_param_dispatch = r#"
        trait Equatable {
            fn eq(*self, other: Self) -> bool;
        }

        struct Item {
            id: i64,
        }

        impl Equatable for Item {
            fn eq(*self, other: Item) -> bool {
                self.id.* == other.id
            }
        }

        fn compare(a: dyn Equatable, b: dyn Equatable) -> bool {
            a.eq(b)
        }

        fn run() -> bool {
            var x = Item { id: 1 };
            var y = Item { id: 1 };
            compare(x.ref.dyn(Equatable), y.ref.dyn(Equatable))
        }
    "#;
    assert_rejected(
        self_param_dispatch,
        "non-receiver Self parameter in dynamic dispatch",
    );

    // 4. Comptime-only method cannot be dynamically dispatched.
    let comptime_method_dispatch = r#"
        trait Tagged {
            comptime fn tag(*self) -> i64;
            fn val(*self) -> i64;
        }

        struct Boxed {
            x: i64,
        }

        impl Tagged for Boxed {
            comptime fn tag(*self) -> i64 { 1 }
            fn val(*self) -> i64 { self.x.* }
        }

        fn read_tag(t: dyn Tagged) -> i64 {
            t.tag()
        }

        fn run() -> i64 {
            var b = Boxed { x: 7 };
            read_tag(b.ref.dyn(Tagged))
        }
    "#;
    assert_rejected(
        comptime_method_dispatch,
        "comptime method in dynamic dispatch",
    );

    // 5. Open / unfixed associated type prevents `dyn Trait` formation.
    let open_assoc_dyn = r#"
        trait Container {
            assoc Item: Type;
            fn count(*self) -> i64;
        }

        struct Bag {
            n: i64,
        }

        impl Container for Bag {
            assoc Item: Type = i64;
            fn count(*self) -> i64 { self.n.* }
        }

        fn count_any(c: dyn Container) -> i64 {
            c.count()
        }

        fn run() -> i64 {
            var b = Bag { n: 3 };
            count_any(b.ref.dyn(Container))
        }
    "#;
    assert_rejected_with(
        open_assoc_dyn,
        "open associated type on dyn Trait",
        &["associated"],
    );

    // 6. `pure comptime trait` cannot form runtime `dyn Trait`.
    let comptime_trait_dyn = r#"
        pure comptime trait ComptimeOnly {
            fn val(*self) -> i64;
        }

        struct S {
            x: i64,
        }

        impl ComptimeOnly for S {
            fn val(*self) -> i64 { self.x.* }
        }

        fn f(x: dyn ComptimeOnly) -> i64 {
            x.val()
        }

        fn run() -> i64 {
            var s = S { x: 1 };
            f(s.ref.dyn(ComptimeOnly))
        }
    "#;
    assert_rejected(comptime_trait_dyn, "pure comptime trait dyn Trait");

    // 7. Closure env binding provenance `%{ s: *Scale = s1.ref }` is rejected without dropping identity.
    let unverified_closure_env_source = r#"
        trait Reading {
            fn read(*self) -> i64;
        }

        struct Scale {
            factor: i64,
        }

        struct Calibrated
        where quote scale: lift *Scale
        {
            raw: i64,
        }

        impl Reading for Calibrated<scale>
        where quote scale: lift *Scale
        {
            fn read(*self) -> i64 {
                self.raw.* * scale.factor.*
            }
        }

        fn relay(sensor: Calibrated<scale>) -> %{ _0: *Scale = scale } dyn Reading
        where quote scale: lift *Scale
        {
            sensor.dyn(Reading)
        }

        fn run() -> i64 {
            var s = Scale { factor: 2 };
            relay(Calibrated<s.ref> { raw: 5 }).read()
        }
    "#;
    assert_rejected_with(
        unverified_closure_env_source,
        "closure env binding source value requires provenance checking",
        &["closure environment sources"],
    );

    // 8. Dangling `Self` return escaping a local pointer captured inside `dyn Duplicate`.
    let escape_dup = r#"
        trait Duplicate {
            fn duplicate(*self) -> Self;
            fn value(*self) -> i64;
        }

        struct Tracker {
            drops: i64,
        }

        struct Token {
            log: *Tracker,
            n: i64,
        }

        impl Duplicate for Token {
            fn duplicate(*self) -> Token {
                Token { log: self.log.*, n: self.n.* }
            }
            fn value(*self) -> i64 { self.n.* }
        }

        impl Drop for Token {
            fn drop(*self) {
                let log = self.*.log;
                if log.drops.* >= 0 and log.drops.* < 100 {
                    log.*.drops = log.drops.* + 1;
                }
            }
        }

        fn escape_dup() -> dyn Duplicate {
            var tracker = Tracker { drops: 0 };
            let t = Token { log: tracker.ref, n: 10 };
            t.ref.dyn(Duplicate).duplicate()
        }

        fn run() -> i64 {
            escape_dup().value()
        }
    "#;
    assert_rejected(escape_dup, "dangling Self return escaping local pointer");

    // 9. Dangling pointer returned from a dynamic method call.
    let escape_ptr = r#"
        trait Borrow {
            fn get_ptr(*self) -> *i64;
        }

        struct BoxedInt {
            x: i64,
        }

        impl Borrow for BoxedInt {
            fn get_ptr(*self) -> *i64 {
                self.x
            }
        }

        fn escape_ptr() -> *i64 {
            var b = BoxedInt { x: 99 };
            b.ref.dyn(Borrow).get_ptr()
        }

        fn run() -> i64 {
            escape_ptr().*
        }
    "#;
    assert_rejected(escape_ptr, "dangling pointer return from DynCall");

    // 10. Trait with a defaulted associated item cannot form `dyn Trait`.
    let defaulted_assoc_dyn = r#"
        trait DefaultAssoc {
            assoc Out: Type = i64;
            fn get(*self) -> Self.as(DefaultAssoc).Out;
        }

        struct A {
            v: i64,
        }

        impl DefaultAssoc for A {
            fn get(*self) -> i64 {
                self.v.*
            }
        }

        fn call_default_assoc(x: dyn DefaultAssoc) -> i64 {
            x.get()
        }

        fn run() -> i64 {
            var a = A { v: 4 };
            call_default_assoc(a.ref.dyn(DefaultAssoc))
        }
    "#;
    assert_rejected_with(
        defaulted_assoc_dyn,
        "defaulted associated item on dyn Trait",
        &["associated"],
    );

    // 11. Compound `Self` return method cannot be dispatched on `dyn Trait`.
    let compound_self_dyn = r#"
        trait MaybeClone {
            fn try_clone(*self) -> ?Self;
            fn val(*self) -> i64;
        }

        struct Item {
            n: i64,
        }

        impl MaybeClone for Item {
            fn try_clone(*self) -> ?Item {
                Item { n: self.n.* }
            }
            fn val(*self) -> i64 {
                self.n.*
            }
        }

        fn call_compound(x: dyn MaybeClone) -> i64 {
            x.try_clone() match {
                _? => 1,
                null => 0,
            }
        }

        fn run() -> i64 {
            var i = Item { n: 5 };
            call_compound(i.ref.dyn(MaybeClone))
        }
    "#;
    assert_rejected_with(
        compound_self_dyn,
        "compound Self return in dynamic dispatch",
        &["compound Self"],
    );
}

#[test]
fn uncalled_function_taking_dyn_trait_with_compound_self_sibling_compiles_cleanly() {
    let source = r#"
        trait Service {
            fn ping(*self) -> i64;
            fn maybe(*self) -> ?Self;
        }

        struct Node {
            v: i64,
        }

        impl Service for Node {
            fn ping(*self) -> i64 { self.v.* }
            fn maybe(*self) -> ?Node { null }
        }

        fn unused_caller(s: dyn Service) -> i64 {
            s.ping()
        }

        fn run() -> bool {
            let ok = Type.size_of(dyn Service) == 16;
            var n = Node { v: 7 };
            let used_ok = n.ref.dyn(Service).ping() == 7;
            ok and used_ok
        }
    "#;

    let case = NativeCase::new(source);
    assert_success(&case.build("0"));
    let result = case.run();
    assert_success(&result);
    assert_eq!(result.stdout, b"(bool true)\n");
}
