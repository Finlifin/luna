//! End-to-end Stage G1b tests for runtime-dependent types, `quote` clauses,
//! runtime environment delivery across methods/traits/functions, and negative
//! identity/lifetime/clause rejections.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-rdt-{}-{}",
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

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn sensor_reads_distinct_calibrations_with_zero_layout_overhead() {
    let source = r#"
        struct Calibration {
            scale: i32,
            offset: i32,
        }

        struct Sensor
        where quote cal: lift *Calibration
        {
            raw: i32,
        }

        impl Sensor<cal> where quote cal: lift *Calibration {
            fn read(self) -> i32 {
                if self.raw >= -1000
                    and self.raw <= 1000
                    and cal.scale.* >= -100
                    and cal.scale.* <= 100
                    and cal.offset.* >= -1000
                    and cal.offset.* <= 1000
                {
                    self.raw * cal.scale.* + cal.offset.*
                } else {
                    0
                }
            }
        }

        fn forward_sensor(sensor: Sensor<cal>) -> Sensor<cal>
        where quote cal: lift *Calibration
        {
            sensor
        }

        fn run() -> bool {
            var first = Calibration { scale: 2, offset: 3 };
            var second = Calibration { scale: 5, offset: 1 };

            let s1 = forward_sensor(Sensor<first.ref> { raw: 10 });
            let s2 = forward_sensor(Sensor<second.ref> { raw: 10 });

            let size_ok = Type.size_of(Sensor<first.ref>) == 4
                and Type.size_of(Sensor<second.ref>) == 4;
            size_ok and s1.read() == 23 and s2.read() == 51
        }
    "#;

    let case = NativeCase::new(source);
    let build = case.build();
    assert_success(&build);
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn node_index_soa_handle_reads_bound_storage() {
    let source = r#"
        struct NodeStorage {
            a: i32,
            b: i32,
            c: i32,
        }

        struct NodeIndex
        where quote storage: lift *NodeStorage
        {
            index: u32,
        }

        impl NodeIndex<storage> where quote storage: lift *NodeStorage {
            fn get(self) -> i32 {
                if self.index == 0 {
                    storage.a.*
                } else if self.index == 1 {
                    storage.b.*
                } else {
                    storage.c.*
                }
            }
        }

        fn read_sum(x: NodeIndex<s>, y: NodeIndex<s>) -> i32
        where quote s: lift *NodeStorage
        {
            let a = x.get();
            let b = y.get();
            if a >= -10000 and a <= 10000 and b >= -10000 and b <= 10000 {
                a + b
            } else {
                0
            }
        }

        fn run() -> bool {
            var left = NodeStorage { a: 10, b: 20, c: 30 };
            var right = NodeStorage { a: 100, b: 200, c: 300 };

            let l0 = NodeIndex<left.ref> { index: 0 };
            let l2 = NodeIndex<left.ref> { index: 2 };
            let r1 = NodeIndex<right.ref> { index: 1 };

            let size_ok = Type.size_of(NodeIndex<left.ref>) == 4;
            size_ok and read_sum(l0, l2) == 40 and r1.get() == 200
        }
    "#;

    let case = NativeCase::new(source);
    let build = case.build();
    assert_success(&build);
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn trait_impl_and_sensor_pair_deliver_distinct_and_shared_envs() {
    let source = r#"
        struct Calibration {
            scale: i32,
            offset: i32,
        }

        struct Sensor
        where quote cal: lift *Calibration
        {
            raw: i32,
        }

        trait Measure {
            fn measure(self) -> i32;
        }

        impl Measure for Sensor<cal> where quote cal: lift *Calibration {
            fn measure(self) -> i32 {
                if self.raw >= -1000
                    and self.raw <= 1000
                    and cal.scale.* >= -100
                    and cal.scale.* <= 100
                    and cal.offset.* >= -1000
                    and cal.offset.* <= 1000
                {
                    self.raw * cal.scale.* + cal.offset.*
                } else {
                    0
                }
            }
        }

        struct SensorPair
        where quote cal: lift *Calibration
        {
            first: Sensor<cal>,
            second: Sensor<cal>,
        }

        impl SensorPair<cal> where quote cal: lift *Calibration {
            fn total(self) -> i32 {
                let first_val = self.first.as(Measure).measure();
                let second_val = self.second.as(Measure).measure();
                if first_val >= -100000
                    and first_val <= 100000
                    and second_val >= -100000
                    and second_val <= 100000
                {
                    first_val + second_val
                } else {
                    0
                }
            }
        }

        fn measure_twice(item: Sensor<cal>) -> i32
        where quote cal: lift *Calibration
        {
            let v = item.as(Measure).measure();
            if v >= -100000 and v <= 100000 {
                v * 2
            } else {
                0
            }
        }

        fn run() -> bool {
            var c1 = Calibration { scale: 3, offset: 4 };
            var c2 = Calibration { scale: 7, offset: -1 };

            let a1 = Sensor<c1.ref> { raw: 5 };
            let a2 = Sensor<c1.ref> { raw: 5 };
            let a3 = Sensor<c1.ref> { raw: 5 };
            let b = Sensor<c1.ref> { raw: 6 };
            let other1 = Sensor<c2.ref> { raw: 5 };
            let other2 = Sensor<c2.ref> { raw: 5 };

            let pair = SensorPair<c1.ref> { first: a1, second: b };
            let size_ok = Type.size_of(SensorPair<c1.ref>) == 8;

            size_ok
                and a2.as(Measure).measure() == 19
                and other1.as(Measure).measure() == 34
                and pair.total() == (19 + 22)
                and measure_twice(a3) == 38
                and measure_twice(other2) == 68
        }
    "#;

    let case = NativeCase::new(source);
    let build = case.build();
    assert_success(&build);
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn mutable_env_pointer_in_loop_and_optional_quote_defaults() {
    let source = r#"
        struct Counter {
            count: i32,
        }

        struct Ticker
        where quote counter: lift *Counter
        {
            step: i32,
        }

        impl Ticker<counter> where quote counter: lift *Counter {
            fn tick(*self) {
                if counter.count.* >= 0
                    and counter.count.* <= 1000
                    and self.step.* >= 0
                    and self.step.* <= 100
                {
                    counter.*.count = counter.count.* + self.step.*;
                }
            }
        }

        fn add_bias(x: i32) -> i32
        where quote .bias: lift i32 = 10
        {
            if x >= -10000 and x <= 10000 and bias >= -10000 and bias <= 10000 {
                x + bias
            } else {
                0
            }
        }

        struct BiasBox
        where quote .bias: lift i32 = 7
        {
            v: i32,
        }

        impl BiasBox<.bias = bias> where quote bias: lift i32 {
            fn total(self) -> i32 {
                if self.v >= -10000 and self.v <= 10000 and bias >= -10000 and bias <= 10000 {
                    self.v + bias
                } else {
                    0
                }
            }
        }

        fn run() -> bool {
            var c = Counter { count: 0 };
            var ticker = Ticker<c.ref> { step: 3 };
            var i: i32 = 0;
            while i < 4 {
                ticker.ref.tick();
                i = i + 1;
            }

            let default_fn = add_bias(5);
            let override_fn = add_bias<.bias = 25>(5);
            let default_box = BiasBox { v: 3 };
            let override_box = BiasBox<.bias = 20> { v: 3 };

            c.count == 12
                and default_fn == 15
                and override_fn == 30
                and default_box.total() == 10
                and override_box.total() == 23
        }
    "#;

    let case = NativeCase::new(source);
    let build = case.build();
    assert_success(&build);
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

fn assert_elaboration_err(source: &str) {
    let err = support::compile(source).unwrap_err();
    assert!(
        !err.contains("Error: expected") && !err.contains("parse"),
        "expected semantic rejection, got syntax error: {err}"
    );
}

#[test]
fn negative_rejections_enforce_runtime_dependent_safety_and_identity() {
    // 1. Runtime identity mismatch on direct assignment and shared-env pair.
    let identity_mismatch_assign = r#"
        struct Calibration { scale: i32 }
        struct Sensor where quote cal: lift *Calibration { raw: i32 }
        fn run() -> i32 {
            var a = Calibration { scale: 1 };
            var b = Calibration { scale: 2 };
            let s1: Sensor<a.ref> = Sensor<b.ref> { raw: 5 };
            s1.raw
        }
    "#;
    assert_elaboration_err(identity_mismatch_assign);

    let identity_mismatch_pair = r#"
        struct Calibration { scale: i32 }
        struct Sensor where quote cal: lift *Calibration { raw: i32 }
        struct SensorPair where quote cal: lift *Calibration {
            first: Sensor<cal>,
            second: Sensor<cal>,
        }
        fn run() -> i32 {
            var a = Calibration { scale: 1 };
            var b = Calibration { scale: 2 };
            let s1 = Sensor<a.ref> { raw: 5 };
            let s2 = Sensor<b.ref> { raw: 6 };
            let pair = SensorPair<a.ref> { first: s1, second: s2 };
            pair.first.raw
        }
    "#;
    assert_elaboration_err(identity_mismatch_pair);

    // 2. Invalid quote / lift clause combinations.
    let lift_without_quote = r#"
        struct Calibration { scale: i32 }
        struct Bad where cal: lift *Calibration { raw: i32 }
        fn run() -> i32 { 0 }
    "#;
    assert_elaboration_err(lift_without_quote);

    let quote_without_lift = r#"
        struct Bad where quote n: i32 { raw: i32 }
        fn run() -> i32 { 0 }
    "#;
    assert_elaboration_err(quote_without_lift);

    // 3. Quote / lift rejected on comptime fns and effects.
    let comptime_quote_fn = r#"
        comptime fn bad() -> i32 where quote x: lift i32 { x }
        fn run() -> i32 { 0 }
    "#;
    assert_elaboration_err(comptime_quote_fn);

    let effect_quote = r#"
        effect bad() -> i32 where quote x: lift i32;
        fn run() -> i32 { 0 }
    "#;
    assert_elaboration_err(effect_quote);

    // 4. Captured local mutation and dangling scope escape.
    let mutated_captured_local = r#"
        struct BiasBox where quote bias: lift i32 { v: i32 }
        impl BiasBox<bias> where quote bias: lift i32 {
            fn total(self) -> i32 { self.v + bias }
        }
        fn run() -> i32 {
            var k: i32 = 1;
            let b = BiasBox<k> { v: 2 };
            k = 9;
            b.total()
        }
    "#;
    assert_elaboration_err(mutated_captured_local);

    let dangling_pointer_escape = r#"
        struct Calibration { offset: i32 }
        struct Sensor where quote cal: lift *Calibration { raw: i32 }
        impl Sensor<cal> where quote cal: lift *Calibration {
            fn read(self) -> i32 { self.raw + cal.offset.* }
        }
        fn run() -> i32 {
            var outside = Calibration { offset: 1 };
            let escaped: Sensor<outside.ref> = do {
                var local_cal = Calibration { offset: 9 };
                Sensor<local_cal.ref> { raw: 5 }
            };
            escaped.read()
        }
    "#;
    assert_elaboration_err(dangling_pointer_escape);

    // 5. Bare function values with runtime env parameters and entry rejection.
    let bare_function_value = r#"
        fn add_bias(x: i32) -> i32 where quote bias: lift i32 { x + bias }
        fn run() -> i32 {
            let f = add_bias;
            0
        }
    "#;
    assert_elaboration_err(bare_function_value);

    let entry_with_quote = r#"
        fn run() -> bool where quote .flag: lift bool = true {
            flag
        }
    "#;
    let entry_case = NativeCase::new(entry_with_quote);
    let build = entry_case.build();
    assert!(!build.status.success());
}

#[test]
fn forwarded_quote_defaults_preserve_identity_across_generic_calls() {
    let source = r#"
        struct Cell { v: i32 }

        struct View
        where
            quote src: lift *Cell,
            quote .via: lift *Cell = src,
            quote .end: lift *Cell = via,
        {}

        fn read(v: View<src, .via = src, .end = src>) -> i32
        where quote src: lift *Cell
        {
            src.v.*
        }

        fn forward(v: View<src>) -> i32
        where quote src: lift *Cell
        {
            read<src>(v)
        }

        fn inferred(v: View<src>) -> i32
        where quote src: lift *Cell, quote .end: lift *Cell = src
        {
            end.v.*
        }

        fn read_default() -> i32
        where quote src: lift *Cell, quote .end: lift *Cell = src
        {
            end.v.*
        }

        fn run() -> bool {
            var a = Cell { v: 7 };
            var b = Cell { v: 42 };
            let left = View<a.ref> {};
            let right = View<b.ref> {};
            forward<a.ref>(left) == 7 and forward<b.ref>(right) == 42
                and read_default<a.ref>() == 7 and read_default<b.ref>() == 42
                and inferred(View<b.ref> {}) == 42
        }
    "#;
    let case = NativeCase::new(source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");

    let mismatch = source.replace("forward<a.ref>(left)", "forward<b.ref>(left)");
    let error = support::compile(&mismatch).unwrap_err();
    assert!(error.contains("type mismatch"), "{error}");
}

#[test]
fn forwarded_quote_defaults_check_the_instantiated_payload_domain() {
    let source = r#"
        struct View
        where A, B, quote src: lift *A, quote .via: lift *B = src
        {}
        fn run() {
            var a: u8 = 7;
            let same = View<u8, u8, a.ref> {};
        }
    "#;
    support::compile(source).unwrap();
    let error = support::compile(&source.replace("View<u8, u8,", "View<u8, bool,")).unwrap_err();
    assert!(error.contains("type mismatch"), "{error}");
}

#[test]
fn forwarded_and_projected_slots_acquire_one_runtime_producer() {
    let case = NativeCase::new(
        r#"
        struct Cell { value: i32 }
        struct Envelope { inner: Cell }
        fn choose(counter: *Cell, target: *Envelope) -> *Envelope {
            if counter.value.* >= 0 and counter.value.* < 1000 {
                counter.*.value = counter.value.* + 1;
            }
            target
        }
        fn observe() -> i32
        where quote src: lift *Envelope,
            quote .same: lift *Envelope = src,
            quote .inner: lift *Cell = same.inner,
            quote .last: lift *i32 = inner.value,
            quote .also: lift *i32 = src.inner.value
        {
            if last.* == also.* { last.* } else { 0 }
        }
        fn run() -> i32 {
            var calls = Cell { value: 0 };
            var data = Envelope { inner: Cell { value: 34 } };
            let result = observe<choose(calls.ref, data.ref)>();
            if result == 34 { calls.value } else { 0 }
        }
    "#,
    );
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(signed-integer 1)\n");
}

#[test]
fn record_acquires_computed_environment_before_fields_and_preserves_it_for_reads() {
    let case = NativeCase::new(
        r#"
        struct Cell { value: i32 }
        fn choose(calls: *Cell, target: *Cell) -> *Cell {
            if calls.value.* >= 0 and calls.value.* < 1000 {
                calls.*.value = calls.value.* + 1;
            }
            target
        }
        struct View where quote src: lift *Cell, quote .field: lift *i32 = src.value {
            at_creation: i32,
        }
        impl View<src, .field = field> where quote src: lift *Cell, quote field: lift *i32 {
            fn read(*self) -> i32 { field.* }
            fn observed(self) -> i32 { self.at_creation }
        }
        fn consume(before: i32, view: View<src, .field = field>, after: i32) -> bool
        where quote src: lift *Cell, quote field: lift *i32
        {
            before == 2 and view.at_creation == 3 and after == 3 and field.* == 42
        }
        fn run() -> bool {
            var calls = Cell { value: 0 };
            var data = Cell { value: 34 };
            if data.value == 0 {
                let unused = View<choose(calls.ref, data.ref)> { at_creation: calls.value };
            }
            var view = View<choose(calls.ref, data.ref)> { at_creation: calls.value };
            let a = view.ref.read();
            data.value = 42;
            let b = view.ref.read();
            let after_reads = calls.value;
            let immediate = View<choose(calls.ref, data.ref)> { at_creation: calls.value }.observed();
            let ordinary = consume(calls.value, View<choose(calls.ref, data.ref)> {
                at_creation: calls.value,
            }, calls.value);
            view.at_creation == 1 and after_reads == 1 and a == 34 and b == 42
                and immediate == 2 and ordinary and calls.value == 3
        }
    "#,
    );
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn returned_environments_preserve_argument_order_cleanup_and_projected_storage() {
    let case = NativeCase::new(
        r#"
        struct Cell { value: i32 }
        fn mark(t: *Cell, digit: i32) -> i32 {
            if t.value.* >= 0 and t.value.* < 10000 and digit > 0 and digit < 10 {
                t.*.value = t.value.* * 10 + digit;
            }
            digit
        }
        struct Guard where quote trace: lift *Cell {}
        impl Drop for Guard<trace> where quote trace: lift *Cell {
            fn drop(*self) { mark(trace, 4); }
        }
        fn first() -> Guard<trace> where quote trace: lift *Cell {
            mark(trace, 1);
            Guard<trace> {}
        }
        struct View where quote src: lift *Cell, quote .field: lift *i32 = src.value {}
        impl View<src, .field = field> where quote src: lift *Cell, quote field: lift *i32 {
            fn read(*self) -> i32 { field.* }
        }
        fn choose(trace: *Cell, target: *Cell) -> *Cell {
            mark(trace, 2);
            target
        }
        fn forward(guard: Guard<trace>, v: View<src, .field = field>, last: i32)
            -> View<src, .field = field>
        where quote trace: lift *Cell, quote src: lift *Cell, quote field: lift *i32
        {
            v
        }
        fn identity(v: T) -> T where T { v }
        fn stop(trace: *Cell, data: *Cell) -> bool {
            let v = forward(first<trace>(), View<do {
                if trace.value.* == 1 { return true; }
                choose(trace, data)
            }> {}, mark(trace, 3));
            false
        }
        fn run() -> bool {
            var trace = Cell { value: 0 };
            var data = Cell { value: 17 };
            var v = identity(forward(first<trace.ref>(),
                View<choose(trace.ref, data.ref)> {}, mark(trace.ref, 3)));
            let before = v.ref.read();
            data.value = 29;
            var early = Cell { value: 0 };
            let stopped = stop(early.ref, data.ref);
            trace.value == 1234 and before == 17 and v.ref.read() == 29
                and stopped and early.value == 14
        }
    "#,
    );
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn returning_one_argument_keeps_other_acquisitions_available_for_cleanup() {
    let case = NativeCase::new(
        r#"
        struct Cell { value: i32 }
        fn bump(p: *Cell) {
            if p.value.* >= 0 and p.value.* < 99 {
                p.*.value = p.value.* + 1;
            }
        }
        fn choose(calls: *Cell, target: *Cell) -> *Cell {
            bump(calls);
            target
        }
        struct Guard where quote counter: lift *Cell {}
        impl Drop for Guard<counter> where quote counter: lift *Cell {
            fn drop(*self) { bump(counter); }
        }
        struct View where quote src: lift *Cell {}
        impl View<src> where quote src: lift *Cell {
            fn read(*self) -> i32 { src.value.* }
        }
        fn right(g: Guard<counter>, v: View<src>, last: bool) -> View<src>
        where quote counter: lift *Cell, quote src: lift *Cell
        {
            v
        }
        fn identity(t: T) -> T where T { t }
        fn observe(calls: *Cell, drops: *Cell, data: *Cell, stop: bool, late: bool) -> bool {
            var v = identity(right(
                Guard<choose(calls, drops)> {},
                View<do {
                    if stop { return true; }
                    choose(calls, data)
                }> {}, do { if late { return true; } false }
            ));
            v.ref.read() == 17 and drops.value.* == 1
        }
        fn run() -> bool {
            var calls = Cell { value: 0 };
            var drops = Cell { value: 0 };
            var data = Cell { value: 17 };
            let done = observe(calls.ref, drops.ref, data.ref, false, false);
            let once = calls.value == 2 and drops.value == 1;
            let stopped = observe(calls.ref, drops.ref, data.ref, true, false);
            let early = stopped and calls.value == 3 and drops.value == 2;
            let late = observe(calls.ref, drops.ref, data.ref, false, true);
            done and once and early and late and calls.value == 5 and drops.value == 3
        }
        "#,
    );
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn call_scoped_affine_environment_drops_before_the_result_is_used() {
    let case = NativeCase::new(
        r#"
        struct Cell { value: i32 }
        struct Ticket { counter: *Cell }
        impl Drop for Ticket {
            fn drop(*self) {
                let p = self.counter.*;
                if p.value.* >= 0 and p.value.* < 100 {
                    p.*.value = p.value.* + 1;
                }
            }
        }
        fn acquire(p: *Cell) -> Ticket { Ticket { counter: p } }
        struct Tag where quote ticket: lift Ticket {}
        struct View where quote src: lift *Cell {}
        impl View<src> where quote src: lift *Cell {
            fn read(*self) -> i32 { src.value.* }
        }
        fn choose(p: *Cell) -> *Cell { p }
        fn right(a: Tag<ticket>, b: View<src>) -> View<src>
        where quote ticket: lift Ticket, quote src: lift *Cell
        {
            b
        }
        fn observe(drops: *Cell, data: *Cell, stop: bool) -> bool {
            var v = right(Tag<acquire(drops)> {}, View<do {
                if stop { return true; }
                choose(data)
            }> {});
            v.ref.read() == 17 and drops.value.* == 1
        }
        fn run() -> bool {
            var drops = Cell { value: 0 };
            var data = Cell { value: 17 };
            let ok = observe(drops.ref, data.ref, false);
            let early = observe(drops.ref, data.ref, true);
            ok and early and drops.value == 2
        }
        "#,
    );
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn returned_environments_reuse_explicit_runtime_inputs_after_the_call() {
    let case = NativeCase::new(
        r#"
        struct Cell { value: i32 }
        fn choose(calls: *Cell, target: *Cell) -> *Cell {
            if calls.value.* >= 0 and calls.value.* < 100 {
                calls.*.value = calls.value.* + 1;
            }
            target
        }
        struct View where quote src: lift *Cell {}
        impl View<src> where quote src: lift *Cell {
            fn read(self) -> i32 { src.value.* }
        }
        fn make() -> View<src> where quote src: lift *Cell { View<src> {} }
        fn run() -> bool {
            var calls = Cell { value: 0 };
            var data = Cell { value: 17 };
            let v = make<choose(calls.ref, data.ref)>();
            let val = v.read();
            val == 17 and calls.value == 1
        }
    "#,
    );
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn returning_a_dependent_call_does_not_export_a_nested_local() {
    let err = support::compile(
        r#"
        struct Cell { value: i32 }
        struct View where quote src: lift *Cell {}
        impl View<src> where quote src: lift *Cell {
            fn read(self) -> i32 { src.value.* }
        }
        fn choose(src: *Cell) -> *Cell { src }
        fn forward(v: T) -> T where T { v }
        fn run() -> i32 {
            let v = do {
                var inner_cell = Cell { value: 17 };
                forward(View<choose(inner_cell.ref)> {})
            };
            v.read()
        }
    "#,
    )
    .unwrap_err();
    assert!(err.contains("no longer in scope"), "{err}");
}

#[test]
fn named_quote_inputs_run_between_the_receiver_and_ordinary_arguments() {
    let case = NativeCase::new(
        r#"
        struct Trace { value: i32 }
        fn mark(trace: *Trace, digit: i32) -> i32 {
            if trace.value.* >= 0 and trace.value.* < 10000 and digit > 0 and digit < 10 {
                trace.*.value = trace.value.* * 10 + digit;
            }
            digit
        }
        struct Gauge { v: i32 }
        impl Gauge {
            fn observe(self, last: i32) -> bool
            where quote .left: lift i32 = 0, quote .right: lift i32 = 0,
                quote .same: lift i32 = left
            {
                left == 1 and right == 2 and same == 1 and last == 3
            }
        }
        fn receiver(t: *Trace) -> Gauge { Gauge { v: mark(t, 4) } }
        fn run() -> i32 {
            var trace = Trace { value: 0 };
            let ok = receiver(trace.ref).observe<
                .right = mark(trace.ref, 2), .left = mark(trace.ref, 1)
            >(mark(trace.ref, 3));
            if ok { trace.value } else { 0 }
        }
    "#,
    );
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(signed-integer 4213)\n");
}

#[test]
fn quote_acquisition_stops_at_return_and_keeps_distinct_producer_calls() {
    let case = NativeCase::new(
        r#"
        struct Trace { value: i32 }
        fn mark(trace: *Trace, digit: i32) -> i32 {
            if trace.value.* >= 0 and trace.value.* < 10000 and digit > 0 and digit < 10 {
                trace.*.value = trace.value.* * 10 + digit;
            }
            digit
        }
        fn observe(last: i32) -> bool
        where quote .left: lift i32 = 0, quote .right: lift i32 = 0,
            quote .same: lift i32 = left
        {
            left == 1 and right == 2 and same == 1 and last == 3
        }
        fn pair() -> bool
        where quote .left: lift i32 = 0, quote .right: lift i32 = 0,
            quote .again: lift i32 = left
        {
            left == right and again == left
        }
        fn exits(trace: *Trace, stop: bool) -> i32 {
            let unused = observe<
                .right = do { if stop { return 7; }; mark(trace, 2) },
                .left = mark(trace, 1)
            >(mark(trace, 3));
            9
        }
        fn run() -> i32 {
            var trace = Trace { value: 0 };
            let value = exits(trace.ref, true);
            let both = pair<.left = mark(trace.ref, 1), .right = mark(trace.ref, 1)>();
            if value == 7 and both { trace.value } else { 0 }
        }
    "#,
    );
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(signed-integer 11)\n");
}

#[test]
fn computed_quote_defaults_require_finite_observation_at_the_declaration() {
    let source = r#"
        struct Cell { first: i32 }
        fn supply(p: *Cell) -> *i32 { BODY }
        struct View where quote src: lift *Cell, quote .field: lift *i32 = supply(src) {}
        fn run() -> bool { true }
    "#;
    support::compile(&source.replace("BODY", "p.first")).unwrap();
    for body in ["p.*.first = 9; p.first", "supply(p)"] {
        let err = support::compile(&source.replace("BODY", body)).unwrap_err();
        assert!(err.contains("P3 stability/termination"), "{err}");
    }
}

#[test]
fn computed_defaults_observe_acquired_inputs_once_in_source_order() {
    let src = r#"
        struct Cell { val: i32 }
        fn read(p: *Cell) -> i32 { p.val.* }
        fn choose(n: *i32, p: *Cell) -> *Cell {
            if n.* >= 0 and n.* < 100 { n.* = n.* + 1; }
            p
        }
        fn change(p: *Cell) -> bool { p.*.val = 42; true }
        fn keep(x: i32) -> i32 { x }
        struct Sample
        where quote src: lift *Cell, quote .saved: lift i32 = read(src), quote .again: lift i32 = keep(saved)
        {}
        fn stored(v: Sample<src, .saved = a, .again = b>) -> i32
        where quote src: lift *Cell, quote a: lift i32, quote b: lift i32
        {
            if a == b { b } else { 0 }
        }
        fn sample(changed: bool) -> i32
        where quote src: lift *Cell, quote .saved: lift i32 = read(src)
        {
            saved
        }
        fn relay(value: T) -> T where T { value }
        fn default_number() -> i32 { 53 }
        struct Closed where quote .n: lift i32 = default_number() {}
        fn number(v: Closed<.n = n>) -> i32 where quote n: lift i32 { n }
        fn run() -> bool {
            var p = Cell { val: 7 };
            var count: i32 = 0;
            let value = relay(Sample<choose(count.ref, p.ref)> {});
            let after = sample<choose(count.ref, p.ref)>(change(p.ref));
            let overridden = sample<p.ref, .saved = 13>(true);
            stored(value) == 7 and after == 42 and count == 2
                and overridden == 13 and number(Closed<> {}) == 53
        }
    "#;
    let case = NativeCase::new(src);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn variants_acquire_quote_inputs_before_payloads_and_retain_their_values() {
    let source = r#"
        struct Cell { value: i32 }
        fn read(p: *Cell) -> i32 { p.value.* }
        fn select(p: *Cell, times: *i32) -> *Cell {
            if times.* < 10 { times.* = times.* + 1; }
            p
        }
        fn change(p: *Cell, n: i32) -> i32 { p.*.value = n; n }
        enum Sample where quote src: lift *Cell, quote .saved: lift i32 = read(src) {
            empty,
            pair(i32, bool),
            group.{point { x: i32 }, leaf},
        }
        fn saved(v: Sample<src, .saved = n>) -> i32
        where quote src: lift *Cell, quote n: lift i32
        {
            n
        }
        fn relay(v: T) -> T where T { v }
        fn run() -> bool {
            var cell = Cell { value: 7 };
            var times: i32 = 0;
            let a = Sample<select(cell.ref, times.ref)>.pair(change(cell.ref, 11), true);
            let b = relay(Sample<select(cell.ref, times.ref)>.group.point { x: change(cell.ref, 13) });
            let c = Sample<select(cell.ref, times.ref)>.group.leaf;
            change(cell.ref, 17);
            let d = Sample<select(cell.ref, times.ref)>.pair(...(change(cell.ref, 19), false));
            let e = Sample<select(cell.ref, times.ref), .saved = 23>.empty;
            change(cell.ref, 31);
            let f: Sample<select(cell.ref, times.ref)> = .empty;
            let g: Sample<select(cell.ref, times.ref)> = .pair(change(cell.ref, 37), false);
            change(cell.ref, 41);
            times == 7 and saved(f) == 31 and saved(g) == 31 and saved(a) == 7 and saved(b) == 11 and saved(c) == 13
                and saved(d) == 17 and saved(e) == 23
        }
    "#;
    let case = NativeCase::new(source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn nested_type_inputs_acquire_each_application_in_written_order() {
    let source = r#"
        struct Cell { value: i32 }
        fn read(p: *Cell) -> i32 { p.value.* }
        fn select(p: *Cell, times: *i32, n: i32) -> *Cell {
            p.*.value = n;
            if times.* < 10 { times.* = times.* + 1; }
            p
        }
        enum Sample where quote src: lift *Cell, quote .saved: lift i32 = read(src) { empty, }
        enum Outer where .A: Type = bool, .B: Type = bool { both(A, B), }
        fn saved(v: Sample<src, .saved = n>) -> i32
        where quote src: lift *Cell, quote n: lift i32
        { n }
        fn saved_optional(v: ?Sample<src, .saved = n>) -> i32
        where quote src: lift *Cell, quote n: lift i32
        { n }
        fn run() -> bool {
            var cell = Cell { value: 0 };
            var times: i32 = 0;
            let v: Outer<.B = Sample<select(cell.ref, times.ref, 7)>,
                .A = ?Sample<select(cell.ref, times.ref, 13)>> = .both(null, .empty);
            let before: i32 = times;
            let last: i32 = cell.value;
            cell.value = 23;
            v match {
                .both(a, b) => saved(b) == 7 and saved_optional(a) == 13
                    and before == 2 and times == 2 and last == 13,
            }
        }
    "#;
    let case = NativeCase::new(source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn explicit_function_type_inputs_run_between_the_receiver_and_arguments() {
    let case = NativeCase::new(
        r#"
        struct Trace { value: i32 }
        fn mark(t: *Trace, digit: i32) -> i32 {
            if t.value.* >= 0 and t.value.* < 10000 and digit > 0 and digit < 10 {
                t.*.value = t.value.* * 10 + digit;
            }
            digit
        }
        fn select(t: *Trace) -> *Trace { mark(t, 3); t }
        fn read(t: *Trace) -> i32 { t.value.* }
        enum Sample where quote src: lift *Trace, quote .saved: lift i32 = read(src) { empty, tagged(bool), }
        fn saved(v: Sample<src, .saved = n>) -> i32
        where quote src: lift *Trace, quote n: lift i32
        {
            n
        }
        struct Gauge {}
        fn receiver(t: *Trace) -> Gauge { mark(t, 1); Gauge {} }
        impl Gauge {
            fn choose(self, v: T, ignored: i32) -> T
            where .T: Type = bool, quote .left: lift i32 = 0, quote .right: lift i32 = 0
            {
                v
            }
        }

        fn choose(v: T, ignored: U) -> T where T, U { v }
        fn staged(comptime keep: bool, v: T, ignored: i32) -> T where T {
            inline if keep { v } else { v }
        }
        fn pair(v: T, n: i32, flag: bool) -> T where T { v }
        fn packet(t: *Trace) -> Tuple<i32, bool> { (mark(t, 5), true) }
        fn run() -> bool {
            var trace = Trace { value: 0 };
            let v = receiver(trace.ref).choose<
                .right = mark(trace.ref, 2), .T = Sample<select(trace.ref)>, .left = mark(trace.ref, 4)
            >(.tagged(true), mark(trace.ref, 5));
            let interleaved = saved(v) == 123 and trace.value == 12345;
            trace.value = 0;
            let a = choose<Sample<select(trace.ref)>>(.empty, mark(trace.ref, 5));
            let inferred = saved(a) == 3 and trace.value == 35;
            trace.value = 0;
            let b = staged<Sample<select(trace.ref)>>(true, .tagged(false), mark(trace.ref, 5));
            let specialized = saved(b) == 3 and trace.value == 35;
            trace.value = 0;
            let c = pair<Sample<select(trace.ref)>>(.empty, ...packet(trace.ref));
            interleaved and inferred and specialized and saved(c) == 3 and trace.value == 35
        }
    "#,
    );
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn closures_keep_prior_storage_addresses_and_own_their_by_value_captures() {
    let definitions = r#"
        enum View where quote address: lift *i32 { empty, }
        fn read(v: View<p>) -> i32 where quote p: lift *i32 { p.* }
        fn same(a: View<p>, b: View<p>) -> i32 where quote p: lift *i32 { p.* }
    "#;
    let case = NativeCase::new(&format!(
        r#"{definitions}
        fn run() -> bool {{
            var cell: i32 = 7;
            let v = View<cell.ref>.empty;
            let w = View<cell.ref>.empty;
            let observe = || {{
                let snapshot = cell;
                let nested = || read(v);
                snapshot == 7 and nested() == 42 and read(w) == 42
            }};
            cell = 42;
            observe()
        }}
    "#
    ));
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");

    let distinct = format!(
        r#"{definitions}
        fn run() -> i32 {{
            var cell: i32 = 7;
            let outside = View<cell.ref>.empty;
            let f = || same(outside, View<cell.ref>.empty);
            f()
        }}
    "#
    );
    assert!(
        support::compile(&distinct)
            .unwrap_err()
            .contains("not entailed")
    );

    let expired = NativeCase::new(&format!(
        r#"{definitions}
        fn run() -> i32 {{
            let f = do {{
                var cell: i32 = 7;
                let v = View<cell.ref>.empty;
                || read(v)
            }};
            f()
        }}
    "#
    ));
    let output = expired.build();
    assert!(!output.status.success());
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(diagnostic.contains("no longer in scope"), "{diagnostic}");
}

#[test]
fn generic_calls_inside_closures_deliver_the_environment_of_the_inferred_type() {
    let case = NativeCase::new(
        r#"
        struct Cell { value: i32 }
        fn read(p: *Cell) -> i32 { p.value.* }
        enum View where quote src: lift *Cell, quote .snapshot: lift i32 = read(src) { empty, }
        fn relay(v: T) -> T where F, T { v }
        fn check(v: View<p, .snapshot = n>) -> bool
        where quote p: lift *Cell, quote n: lift i32
        {
            p.value.* == 42 and n == 7
        }
        fn run() -> bool {
            var cell = Cell { value: 7 };
            let v = View<cell.ref>.empty;
            let call = || {
                let nested = || relay<fn(comptime U: Type, value: U) -> U>(v);
                nested()
            };
            cell.value = 42;
            check(call())
        }
    "#,
    );
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn computed_default_identities_are_per_acquisition_and_cannot_export_dead_storage() {
    let definitions = r#"
        struct Pair { first: i32 }
        fn first(p: *Pair) -> *i32 { p.first }
        struct View where quote src: lift *Pair, quote .field: lift *i32 = first(src) {}
        fn read(v: View<src, .field = field>) -> i32
        where quote src: lift *Pair, quote field: lift *i32
        {
            field.*
        }
        fn same(a: View<src, .field = field>, b: View<src, .field = field>) -> i32
        where quote src: lift *Pair, quote field: lift *i32
        {
            field.*
        }
    "#;
    for (body, diagnostic) in [
        (
            "var p = Pair { first: 7 }; same(View<p.ref> {}, View<p.ref> {})",
            "not entailed",
        ),
        (
            "let v = do { var p = Pair { first: 7 }; View<p.ref> {} }; read(v)",
            "no longer in scope",
        ),
    ] {
        let source = format!("{definitions} fn run() -> i32 {{ {body} }}");
        let reason = support::compile(&source).unwrap_err();
        assert!(reason.contains(diagnostic), "{reason}");
    }
}

#[test]
fn projected_quote_defaults_keep_the_field_and_storage_identity() {
    let source = r#"
        struct Cell {
            first: i32,
            second: i32,
            pub const GLOBAL: Cell = Cell { first: 97, second: 7 };
        }

        impl Splice for Cell {
            assoc RuntimeTarget: Type = Cell;
            fn splice(comptime data: Cell) -> Cell {
                Cell { first: data.first, second: data.second }
            }
        }

        struct View
        where quote src: lift *Cell, quote .field: lift *i32 = src.first
        {}

        fn read_view(v: View<src, .field = field>) -> i32
        where quote src: lift *Cell, quote field: lift *i32
        {
            field.*
        }

        fn projected() -> i32
        where quote src: lift *Cell, quote .field: lift *i32 = src.first
        {
            field.*
        }

        fn forward() -> i32 where quote src: lift *Cell {
            projected<src>()
        }

        fn forward_view(v: View<src>) -> View<src, .field = src.first> where quote src: lift *Cell {
            v
        }

        fn run() -> bool {
            var a = Cell { first: 17, second: 32 };
            var b = Cell { first: 42, second: 17 };
            let left = View<a.ref> {};
            let right = View<b.ref> {};
            let global = View<Cell.GLOBAL.ref> {};
            read_view<a.ref, a.first.ref>(forward_view<a.ref>(left)) == 17
                and read_view<b.ref, b.first.ref>(forward_view(right)) == 42
                and read_view<Cell.GLOBAL.ref, Cell.GLOBAL.ref.first>(global) == 97
                and forward<a.ref>() == 17 and projected<b.ref>() == 42
        }
    "#;
    let case = NativeCase::new(source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");

    for other in ["a.second.ref", "b.first.ref"] {
        let invalid = source.replace("a.ref, a.first.ref>", &format!("a.ref, {other}>"));
        let error = support::compile(&invalid).unwrap_err();
        assert!(error.contains("type mismatch"), "{error}");
    }
}

#[test]
fn implicit_drop_delivers_the_projected_field_and_preserves_its_sibling() {
    let case = NativeCase::new(
        r#"
        struct Counters { left: i32, right: i32 }
        struct Guard where quote dst: lift *i32 {}
        impl Drop for Guard<dst> where quote dst: lift *i32 {
            fn drop(*self) {
                if dst.* >= 0 and dst.* < 100 { dst.* = dst.* + 1; }
            }
        }
        fn release(g: Guard<dst>) where quote dst: lift *i32 {}
        fn run() -> bool {
            var counters = Counters { left: 6, right: 20 };
            do { let g = Guard<counters.left.ref> {}; };
            release(Guard<counters.right.ref> {});
            counters.left == 7 and counters.right == 21
        }
    "#,
    );
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn projected_pointer_dependency_expires_when_its_binding_changes() {
    let source = r#"
        struct Cell { first: i32 }
        struct Field where quote field: lift *i32 {}
        impl Field<field> where quote field: lift *i32 {
            fn read(self) -> i32 { field.* }
        }

        fn run() -> bool {
            var a = Cell { first: 17 };
            var b = Cell { first: 42 };
            var p = a.ref;
            let captured = Field<p.first> {};
            p = b.ref;
            captured.read() == 17
        }
    "#;
    let error = support::compile(source).unwrap_err();
    assert!(
        error.contains("runtime dependency binding was modified after capture"),
        "{error}"
    );
}

#[test]
fn global_const_address_identity_and_dependent_quote_defaults() {
    let source = r#"
        trait MiniAlloc {
            fn tag(*self) -> i32;
        }

        struct DefaultAlloc {
            pub const GLOBAL: DefaultAlloc = DefaultAlloc {};
        }

        impl MiniAlloc for DefaultAlloc {
            fn tag(*self) -> i32 { 7 }
        }

        impl Splice for DefaultAlloc {
            assoc RuntimeTarget: Type = DefaultAlloc;
            fn splice(comptime data: DefaultAlloc) -> DefaultAlloc {
                DefaultAlloc {}
            }
        }

        struct AltAlloc {
            id: i32,
        }

        impl MiniAlloc for AltAlloc {
            fn tag(*self) -> i32 { self.id.* }
        }

        struct OwnedBox
        where
            T,
            .A: Type = DefaultAlloc,
            requires A :- MiniAlloc,
            quote .allocator: lift *A = DefaultAlloc.GLOBAL.ref,
        {
            value: T,
        }

        impl OwnedBox<T, .A = A, .allocator = a>
        where
            T,
            A: Type,
            requires A :- MiniAlloc,
            quote a: lift *A,
        {
            fn tag(*self) -> i32 {
                a.as(MiniAlloc).tag()
            }
        }

        fn read_box(b: OwnedBox<i32>) -> i32 {
            let t = b.ref.tag();
            if b.value >= -1000 and b.value <= 1000 and t >= -1000 and t <= 1000 {
                b.value + t
            } else {
                0
            }
        }

        fn read_alt(b: OwnedBox<i32, .A = AltAlloc, .allocator = a>) -> i32
        where quote a: lift *AltAlloc
        {
            let t = b.ref.tag();
            if b.value >= -1000 and b.value <= 1000 and t >= -1000 and t <= 1000 {
                b.value + t
            } else {
                0
            }
        }

        fn run() -> bool {
            let implicit_all = OwnedBox<i32> { value: 10 };
            let explicit_alloc = OwnedBox<i32, .A = DefaultAlloc> { value: 20 };
            let explicit_all =
                OwnedBox<i32, .A = DefaultAlloc, .allocator = DefaultAlloc.GLOBAL.ref> {
                    value: 30,
                };

            var alt = AltAlloc { id: 11 };
            let alt_box = OwnedBox<i32, .A = AltAlloc, .allocator = alt.ref> { value: 5 };

            let size_ok = Type.size_of(OwnedBox<i32>) == Type.size_of(i32)
                and Type.size_of(OwnedBox<i32, .A = DefaultAlloc>) == Type.size_of(i32)
                and Type.size_of(
                    OwnedBox<i32, .A = DefaultAlloc, .allocator = DefaultAlloc.GLOBAL.ref>,
                ) == Type.size_of(i32)
                and Type.size_of(OwnedBox<i32, .A = AltAlloc, .allocator = alt.ref>)
                    == Type.size_of(i32);

            size_ok
                and read_box(implicit_all) == 17
                and read_box(explicit_alloc) == 27
                and read_box(explicit_all) == 37
                and read_alt(alt_box) == 16
        }
    "#;

    let case = NativeCase::new(source);
    let build = case.build();
    assert_success(&build);
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn drop_delivers_global_and_local_stateful_allocator_environments() {
    let source = r#"
        trait MiniAlloc {
            fn on_drop(*self, val: i32);
        }

        struct DefaultAlloc {
            pub const GLOBAL: DefaultAlloc = DefaultAlloc {};
        }

        impl MiniAlloc for DefaultAlloc {
            fn on_drop(*self, val: i32) {}
        }

        impl Splice for DefaultAlloc {
            assoc RuntimeTarget: Type = DefaultAlloc;
            fn splice(comptime data: DefaultAlloc) -> DefaultAlloc {
                DefaultAlloc {}
            }
        }

        struct CountingAlloc {
            drops: i32,
            sum: i32,
        }

        impl MiniAlloc for CountingAlloc {
            fn on_drop(*self, val: i32) {
                if self.drops.* >= 0
                    and self.drops.* <= 1000
                    and self.sum.* >= -10000
                    and self.sum.* <= 10000
                    and val >= -1000
                    and val <= 1000
                {
                    self.*.drops = self.drops.* + 1;
                    self.*.sum = self.sum.* + val;
                }
            }
        }

        struct OwnedBox
        where
            T,
            .A: Type = DefaultAlloc,
            requires A :- MiniAlloc,
            quote .allocator: lift *A = DefaultAlloc.GLOBAL.ref,
        {
            value: i32,
        }

        impl Drop for OwnedBox<T, .A = A, .allocator = a>
        where
            T,
            A: Type,
            requires A :- MiniAlloc,
            quote a: lift *A,
        {
            fn drop(*self) {
                a.as(MiniAlloc).on_drop(self.value.*);
            }
        }

        fn make_default_box(v: i32) -> OwnedBox<i32> {
            OwnedBox<i32> { value: v }
        }

        fn consume_box(b: OwnedBox<i32, .A = A, .allocator = a>)
        where
            A: Type,
            requires A :- MiniAlloc,
            quote a: lift *A,
        {}

        fn relay(value: T) -> T where T { value }

        fn select(calls: *i32, allocator: *CountingAlloc) -> *CountingAlloc {
            if calls.* >= 0 and calls.* < 100 { calls.* = calls.* + 1; }
            allocator
        }

        fn run() -> bool {
            let default_v = do {
                let d = make_default_box(99);
                d.value
            };

            var alloc = CountingAlloc { drops: 0, sum: 0 };
            let size_ok = Type.size_of(OwnedBox<i32>) == Type.size_of(i32)
                and Type.size_of(OwnedBox<i32, .A = CountingAlloc, .allocator = alloc.ref>)
                    == Type.size_of(i32);

            do {
                let local_box = OwnedBox<i32, .A = CountingAlloc, .allocator = alloc.ref> {
                    value: 10,
                };
            };
            let after_local_drops = alloc.drops;
            let after_local_sum = alloc.sum;

            consume_box(OwnedBox<i32, .A = CountingAlloc, .allocator = alloc.ref> { value: 25 });

            var selections: i32 = 0;
            do {
                let selected = OwnedBox<i32, .A = CountingAlloc,
                    .allocator = select(selections.ref, alloc.ref)> { value: 5 };
            };

            do {
                let relayed = relay(OwnedBox<i32, .A = CountingAlloc,
                    .allocator = select(selections.ref, alloc.ref)> { value: 7 });
            };
            size_ok
                and selections == 2
                and default_v == 99
                and after_local_drops == 1
                and after_local_sum == 10
                and alloc.drops == 4
                and alloc.sum == 47
        }
    "#;

    let case = NativeCase::new(source);
    let build = case.build();
    assert_success(&build);
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn negative_global_const_and_drop_env_safety_rejections() {
    let assert_rejected = |source: &str, label: &str, needle: &str| {
        let case = NativeCase::new(source);
        let build = case.build();
        assert!(
            !build.status.success(),
            "expected build failure for {label}, but succeeded"
        );
        let stderr = String::from_utf8_lossy(&build.stderr);
        assert!(
            stderr.contains(needle),
            "expected {needle:?} for {label}, got:\n{stderr}"
        );
    };

    // (a) Direct write through a global constant's address.
    let direct_write_global = r#"
        struct Counter {
            count: i32,
            pub const GLOBAL: Counter = Counter { count: 0 };
        }

        impl Splice for Counter {
            assoc RuntimeTarget: Type = Counter;
            fn splice(comptime data: Counter) -> Counter {
                Counter { count: data.count }
            }
        }

        fn run() -> i32 {
            let p = Counter.GLOBAL.ref;
            p.*.count = 1;
            0
        }
    "#;
    assert_rejected(
        direct_write_global,
        "direct write through GLOBAL.ref",
        "constant",
    );

    // (b) Drop mutating through a global constant allocator's address.
    let drop_writes_global = r#"
        trait MiniAlloc {
            fn on_drop(*self, val: i32);
        }

        struct CountingAlloc {
            drops: i32,
            pub const GLOBAL: CountingAlloc = CountingAlloc { drops: 0 };
        }

        impl MiniAlloc for CountingAlloc {
            fn on_drop(*self, val: i32) {
                if self.drops.* >= 0 and self.drops.* <= 1000 {
                    self.*.drops = self.drops.* + 1;
                }
            }
        }

        impl Splice for CountingAlloc {
            assoc RuntimeTarget: Type = CountingAlloc;
            fn splice(comptime data: CountingAlloc) -> CountingAlloc {
                CountingAlloc { drops: data.drops }
            }
        }

        struct OwnedBox
        where
            .A: Type = CountingAlloc,
            requires A :- MiniAlloc,
            quote .allocator: lift *A = CountingAlloc.GLOBAL.ref,
        {
            value: i32,
        }

        impl Drop for OwnedBox<.A = A, .allocator = a>
        where
            A: Type,
            requires A :- MiniAlloc,
            quote a: lift *A,
        {
            fn drop(*self) {
                a.as(MiniAlloc).on_drop(self.value.*);
            }
        }

        fn run() -> i32 {
            do {
                let b = OwnedBox { value: 5 };
            };
            0
        }
    "#;
    assert_rejected(
        drop_writes_global,
        "Drop write through GLOBAL.ref",
        "constant",
    );

    // (c) Moving/retiring local allocator before dependent Drop runs.
    let alloc_moved_before_drop = r#"
        struct CountingAlloc {
            drops: i32,
        }

        fn take_alloc(alloc: CountingAlloc) {}

        struct OwnedBox
        where quote allocator: lift *CountingAlloc
        {
            value: i32,
        }

        impl Drop for OwnedBox<a>
        where quote a: lift *CountingAlloc
        {
            fn drop(*self) {
                let _v = a.drops.*;
            }
        }

        fn run() -> i32 {
            var alloc = CountingAlloc { drops: 0 };
            let b = OwnedBox<alloc.ref> { value: 1 };
            take_alloc(alloc);
            b.value
        }
    "#;
    assert_rejected(
        alloc_moved_before_drop,
        "allocator moved before dependent Drop",
        "Drop",
    );
}

#[test]
fn staged_ordinary_inputs_preserve_runtime_environment_acquisition() {
    let source = r#"
        struct Trace { value: i32 }
        fn mark(trace: *Trace, digit: i32) -> i32 {
            if trace.value.* >= 0 and trace.value.* < 10000 and digit > 0 and digit < 10 {
                trace.*.value = trace.value.* * 10 + digit;
            }
            digit
        }
        fn select(comptime first: bool, fallback: i32) -> i32
        where quote current: lift i32
        {
            if first { current } else { fallback }
        }
        fn relay(trace: *Trace, comptime first: bool) -> i32 {
            select<mark(trace, 2)>(first, mark(trace, 3))
        }
        fn run() -> bool {
            var trace = Trace { value: 0 };
            let a = relay(trace.ref, true);
            let b = relay(trace.ref, false);
            let first_trace = trace.value;
            trace.value = 0;
            let c = select<mark(trace.ref, 4)>(not false, mark(trace.ref, 5));
            a == 2 and b == 3 and c == 4 and first_trace == 2323 and trace.value == 45
        }
    "#;
    let case = NativeCase::new(source);
    assert_success(&case.build());
    let result = case.run();
    assert_success(&result);
    assert_eq!(result.stdout, b"(bool true)\n");

    let invalid = source.replace(">(first, mark", ">(trace.value.* == 0, mark");
    let error = support::compile(&invalid).unwrap_err();
    assert!(
        error.contains("runtime") && error.contains("comptime"),
        "{error}"
    );
}
