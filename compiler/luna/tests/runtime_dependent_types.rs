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
