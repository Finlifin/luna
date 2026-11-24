//! End-to-end tests for creation/move-time `Drop` runtime environment capture,
//! non-pointer runtime env handle support, and `env_arguments` across indirect,
//! temporary, closure, and guard calls (`GAP-05-04`, `GAP-05-06`, `DIV-05-01`,
//! `SND-05-01`, `CMP-05-01`).

use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-drop-env-{}-{}",
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

fn assert_rejected(source: &str, label: &str) {
    let case = NativeCase::new(source);
    let build = case.build("0");
    assert!(
        !build.status.success(),
        "expected rejection for `{label}`, but build succeeded:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
}

#[test]
fn captured_drop_env_survives_block_exit_and_moves_at_o0_and_o3() {
    let source = r#"
        struct Cell {
            value: i32,
        }

        fn bump(p: *Cell) {
            if p.value.* >= 0 and p.value.* < 99 {
                p.*.value = p.value.* + 1;
            }
        }

        struct Guard where quote counter: lift *Cell {}

        impl Drop for Guard<counter> where quote counter: lift *Cell {
            fn drop(*self) {
                bump(counter);
            }
        }

        struct Holder where quote counter: lift *Cell {
            inner: Guard<counter>,
        }

        fn run() -> bool {
            var first = Cell { value: 0 };
            var second = Cell { value: 0 };
            let _wrapped = do {
                let p = first.ref;
                let g = Guard<p> {};
                let moved = g;
                Holder<p> { inner: moved }
            };
            let _direct = do {
                let r = second.ref;
                Guard<r> {}
            };
            first.value == 0 and second.value == 0
        }
    "#;
    for opt in ["0", "3"] {
        let case = NativeCase::new(source);
        assert_success(&case.build(opt));
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn captured_drop_env_keeps_creation_target_after_pointer_reassignment() {
    let source = r#"
        struct Cell {
            value: i32,
        }

        fn bump(p: *Cell) {
            if p.value.* >= 0 and p.value.* < 99 {
                p.*.value = p.value.* + 1;
            }
        }

        struct Guard where quote counter: lift *Cell {}

        impl Drop for Guard<counter> where quote counter: lift *Cell {
            fn drop(*self) {
                bump(counter);
            }
        }

        fn run() -> bool {
            var first = Cell { value: 0 };
            var second = Cell { value: 0 };
            var p = first.ref;
            do {
                let g = Guard<p> {};
                p = second.ref;
                let _moved = g;
            };
            first.value == 1 and second.value == 0
        }
    "#;
    for opt in ["0", "3"] {
        let case = NativeCase::new(source);
        assert_success(&case.build(opt));
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn captured_drop_env_rejects_dangling_targets_after_move_or_scope_exit() {
    let moved_after_reassignment = r#"
        struct Cell {
            value: i32,
        }

        fn consume(c: Cell) {}

        struct Guard where quote counter: lift *Cell {}

        impl Drop for Guard<counter> where quote counter: lift *Cell {
            fn drop(*self) {
                let _v = counter.value.*;
            }
        }

        fn run() -> i32 {
            var first = Cell { value: 1 };
            var second = Cell { value: 2 };
            var p = first.ref;
            let g = Guard<p> {};
            p = second.ref;
            consume(first);
            let _moved = g;
            0
        }
    "#;
    assert_rejected(
        moved_after_reassignment,
        "target moved after pointer reassignment",
    );

    let dangling_block_target = r#"
        struct Cell {
            value: i32,
        }

        struct Guard where quote counter: lift *Cell {}

        impl Drop for Guard<counter> where quote counter: lift *Cell {
            fn drop(*self) {
                let _v = counter.value.*;
            }
        }

        fn run() -> i32 {
            let _escaped = do {
                var local_cell = Cell { value: 7 };
                let p = local_cell.ref;
                Guard<p> {}
            };
            0
        }
    "#;
    assert_rejected(dangling_block_target, "target retired with inner block");
}

#[test]
fn non_pointer_copy_handle_drop_runtime_env_executes_and_rejects_non_copy() {
    let valid_handle = r#"
        struct Cell {
            value: i32,
        }

        struct TagHandle {
            weight: i32,
        }

        impl Clone for TagHandle {
            fn clone(*self) -> Self {
                TagHandle { weight: self.weight.* }
            }
        }

        impl Copy for TagHandle {}

        struct TaggedGuard where quote handle: lift TagHandle {
            out: *Cell,
        }

        impl Drop for TaggedGuard<handle> where quote handle: lift TagHandle {
            fn drop(*self) {
                let p = self.out.*;
                if p.value.* >= 0 and p.value.* < 100 and handle.weight >= 0 and handle.weight < 100 {
                    p.*.value = p.value.* + handle.weight;
                }
            }
        }

        fn run() -> bool {
            var acc = Cell { value: 10 };
            let h = TagHandle { weight: 5 };
            do {
                let g = TaggedGuard<h> { out: acc.ref };
                let _moved = g;
            };
            acc.value == 15
        }
    "#;
    for opt in ["0", "3"] {
        let case = NativeCase::new(valid_handle);
        assert_success(&case.build(opt));
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }

    let non_copy_handle = r#"
        struct Cell {
            value: i32,
        }

        struct LinearHandle {
            weight: i32,
        }

        struct LinearGuard where quote handle: lift LinearHandle {
            out: *Cell,
        }

        impl Drop for LinearGuard<handle> where quote handle: lift LinearHandle {
            fn drop(*self) {
                let _w = handle.weight;
            }
        }

        fn run() -> i32 {
            var acc = Cell { value: 0 };
            let h = LinearHandle { weight: 3 };
            let _g = LinearGuard<h> { out: acc.ref };
            0
        }
    "#;
    assert_rejected(non_copy_handle, "non-Copy runtime env handle in Drop");
}

#[test]
fn env_arguments_work_across_indirect_temporary_closure_and_guard_calls() {
    let source = r#"
        struct Bias {
            delta: i32,
        }

        struct Handle
        where quote bias: lift *Bias
        {
            tag: i32,
        }

        impl Clone for Handle<bias> where quote bias: lift *Bias {
            fn clone(*self) -> Self {
                Handle<bias> { tag: self.tag.* }
            }
        }

        impl Copy for Handle<bias> where quote bias: lift *Bias {}

        fn add_bias(h: Handle<bias>, x: i32) -> i32
        where quote bias: lift *Bias
        {
            if h.tag >= -1000 and x >= -1000 and x <= 1000 and bias.delta.* >= -1000 and bias.delta.* <= 1000 {
                x + bias.delta.*
            } else {
                0
            }
        }

        fn sub_bias(h: Handle<bias>, x: i32) -> i32
        where quote bias: lift *Bias
        {
            if h.tag >= -1000 and x >= -1000 and x <= 1000 and bias.delta.* >= -1000 and bias.delta.* <= 1000 {
                x - bias.delta.*
            } else {
                0
            }
        }

        fn make_add(h: Handle<bias>) -> %{ h: Handle<bias> } fn(i32) -> i32
        where quote bias: lift *Bias
        {
            |v: i32| -> i32 { add_bias(h, v) }
        }

        fn make_sub(h: Handle<bias>) -> %{ h: Handle<bias> } fn(i32) -> i32
        where quote bias: lift *Bias
        {
            |v: i32| -> i32 { sub_bias(h, v) }
        }

        fn matches_bias(h: Handle<bias>, x: i32) -> bool
        where quote bias: lift *Bias
        {
            h.tag >= 0 and x == bias.delta.*
        }

        fn check_env(h: Handle<bias>, flag: bool) -> bool
        where quote bias: lift *Bias
        {
            let clos = make_add(h);
            let d = clos(20);

            let clos2 = make_add(h);
            let indirect = clos2;
            let a = indirect(10);

            let candidate = if flag {
                make_add(h)
            } else {
                make_sub(h)
            };
            let b_res = candidate(10);

            let c = (if not flag {
                make_add(h)
            } else {
                make_sub(h)
            })(10);

            let guarded = 5 match {
                v if matches_bias(h, v) => 1,
                _ => 0,
            };

            a == 15 and b_res == 15 and c == 5 and d == 25 and guarded == 1
        }

        fn run() -> bool {
            var bias = Bias { delta: 5 };
            let h = Handle<bias.ref> { tag: 1 };
            check_env(h, true)
        }
    "#;
    for opt in ["0", "3"] {
        let case = NativeCase::new(source);
        assert_success(&case.build(opt));
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn loop_mutation_of_captured_drop_env_converges_and_rejects_mutated_handle() {
    let pointer_loop_source = r#"
        struct Cell {
            value: i32,
        }

        fn bump(p: *Cell) {
            if p.value.* >= 0 and p.value.* < 99 {
                p.*.value = p.value.* + 1;
            }
        }

        struct Guard where quote counter: lift *Cell {}

        impl Drop for Guard<counter> where quote counter: lift *Cell {
            fn drop(*self) {
                bump(counter);
            }
        }

        fn run() -> bool {
            var first = Cell { value: 0 };
            var second = Cell { value: 0 };
            var p = first.ref;
            do {
                let _g = Guard<p> {};
                var i: i32 = 0;
                while i < 2 {
                    p = second.ref;
                    i = i + 1;
                }
            };
            first.value == 1 and second.value == 0
        }
    "#;
    for opt in ["0", "3"] {
        let case = NativeCase::new(pointer_loop_source);
        assert_success(&case.build(opt));
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }

    let handle_loop_mutated = r#"
        struct TagHandle {
            weight: i32,
        }

        impl Clone for TagHandle {
            fn clone(*self) -> Self {
                TagHandle { weight: self.weight.* }
            }
        }

        impl Copy for TagHandle {}

        struct TaggedGuard where quote handle: lift TagHandle {}

        impl Drop for TaggedGuard<handle> where quote handle: lift TagHandle {
            fn drop(*self) {
                let _w = handle.weight;
            }
        }

        fn run() -> i32 {
            var h = TagHandle { weight: 5 };
            let _g = TaggedGuard<h> {};
            var i: i32 = 0;
            while i < 2 {
                h = TagHandle { weight: 99 };
                i = i + 1;
            }
            0
        }
    "#;
    assert_rejected(
        handle_loop_mutated,
        "non-pointer Copy handle mutated in loop",
    );
}

#[test]
fn indirect_pointer_and_call_writes_invalidate_captured_drop_envs() {
    let indirect_pointer_ok = r#"
        struct Cell {
            value: i32,
        }

        fn bump(p: *Cell) {
            if p.value.* >= 0 and p.value.* < 99 {
                p.*.value = p.value.* + 1;
            }
        }

        struct Guard where quote counter: lift *Cell {}

        impl Drop for Guard<counter> where quote counter: lift *Cell {
            fn drop(*self) {
                bump(counter);
            }
        }

        fn run() -> bool {
            var first = Cell { value: 0 };
            var second = Cell { value: 0 };
            var p = first.ref;
            do {
                let _g = Guard<p> {};
                let pp = p.ref;
                pp.* = second.ref;
            };
            first.value == 1 and second.value == 0
        }
    "#;
    for opt in ["0", "3"] {
        let case = NativeCase::new(indirect_pointer_ok);
        assert_success(&case.build(opt));
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }

    let indirect_pointer_dangling = r#"
        struct Cell {
            value: i32,
        }

        fn consume(c: Cell) {}

        struct Guard where quote counter: lift *Cell {}

        impl Drop for Guard<counter> where quote counter: lift *Cell {
            fn drop(*self) {
                let _v = counter.value.*;
            }
        }

        fn run() -> i32 {
            var first = Cell { value: 1 };
            var second = Cell { value: 2 };
            var p = first.ref;
            let _g = Guard<p> {};
            let pp = p.ref;
            pp.* = second.ref;
            consume(first);
            0
        }
    "#;
    assert_rejected(
        indirect_pointer_dangling,
        "indirect pointer write pp.* = second.ref must not rescue consumed first",
    );

    let call_write_dangling = r#"
        struct Cell {
            value: i32,
        }

        fn consume(c: Cell) {}

        fn repoint(pp: **Cell, next: *Cell) {
            pp.* = next;
        }

        struct Guard where quote counter: lift *Cell {}

        impl Drop for Guard<counter> where quote counter: lift *Cell {
            fn drop(*self) {
                let _v = counter.value.*;
            }
        }

        fn run() -> i32 {
            var first = Cell { value: 1 };
            var second = Cell { value: 2 };
            var p = first.ref;
            let _g = Guard<p> {};
            repoint(p.ref, second.ref);
            consume(first);
            0
        }
    "#;
    assert_rejected(
        call_write_dangling,
        "helper call write to captured pointer must not rescue consumed first",
    );

    let indirect_handle_write = r#"
        struct TagHandle {
            weight: i32,
        }

        impl Clone for TagHandle {
            fn clone(*self) -> Self {
                TagHandle { weight: self.weight.* }
            }
        }

        impl Copy for TagHandle {}

        struct TaggedGuard where quote handle: lift TagHandle {}

        impl Drop for TaggedGuard<handle> where quote handle: lift TagHandle {
            fn drop(*self) {
                let _w = handle.weight;
            }
        }

        fn run() -> i32 {
            var h = TagHandle { weight: 5 };
            let _g = TaggedGuard<h> {};
            let hp = h.ref;
            hp.* = TagHandle { weight: 99 };
            0
        }
    "#;
    assert_rejected(
        indirect_handle_write,
        "indirect write to non-pointer Copy handle hp.* must reject",
    );
}

#[test]
fn creation_time_dangling_capture_cannot_be_rescued_by_reassignment() {
    let source = r#"
        struct Cell {
            value: i32,
        }

        fn consume(c: Cell) {}

        struct Guard where quote counter: lift *Cell {}

        impl Drop for Guard<counter> where quote counter: lift *Cell {
            fn drop(*self) {
                let _v = counter.value.*;
            }
        }

        fn run() -> i32 {
            var first = Cell { value: 1 };
            var second = Cell { value: 2 };
            var p = first.ref;
            consume(first);
            let _g = Guard<p> {};
            p = second.ref;
            0
        }
    "#;
    assert_rejected(
        source,
        "creation-time dangling capture rescued by p = second.ref",
    );
}
