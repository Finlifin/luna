//! Tests for `where` partial named & positional clause parameter inference across
//! `comptime fn`, static/receiver inherent & trait methods, optional type clauses,
//! and variadic clause collectors (`GAP-02-24` / `CMP-02-02`).
#[path = "support/primitive_bindings.rs"]
mod primitive_bindings;

#[path = "support/execution.rs"]
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
            "luna-partial-clause-{}-{}",
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
fn comptime_fn_supports_partial_named_and_positional_clause_inference() {
    let source = r#"
        typealias Type = .Type'builtin;
        typealias u32 = .u32'builtin;

        comptime fn tag_value(x: T) -> bool
        where
            T,
            .flag: bool = false,
        {
            flag
        }

        comptime fn choose_pair(x: T, y: U) -> bool
        where
            T,
            U,
            .expect_same: bool = false,
        {
            (T == U) == expect_same
        }

        comptime fn with_dependent_optional(x: T) -> T
        where
            T,
            .override: ?T = null,
        {
            override match {
                v? => v,
                null => x,
            }
        }

        comptime fn run() -> bool {
            let n: u32 = 42;
            let one: u32 = 1;
            let two: u32 = 2;
            let a = tag_value<.flag = true>(n);
            let b = not tag_value(n);
            let c = choose_pair<u32>(one, true);
            let d = choose_pair<u32, .expect_same = true>(one, two);
            let e = with_dependent_optional<.override = 99>(n) == 99;
            a and b and c and d and e
        }
    "#;
    let package = support::compile(source);
    nir::phases::validate(&package).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn comptime_fn_partial_named_rejects_unknown_name_and_standalone_omission() {
    let unknown_named = r#"
        typealias u32 = .u32'builtin;
        comptime fn tag_value(x: T) -> bool
        where
            T,
            .flag: bool = false,
        {
            flag
        }
        comptime fn run() -> bool {
            let n: u32 = 42;
            tag_value<.missing = true>(n)
        }
    "#;
    let err = support::compile_with_protocols(unknown_named, &Default::default()).unwrap_err();
    assert!(err.contains("unknown named"), "{err}");

    let standalone = r#"
        typealias u32 = .u32'builtin;
        comptime fn pair(x: T, y: U) -> bool
        where
            T,
            U,
        {
            true
        }
        comptime fn run() -> bool {
            let one: u32 = 1;
            let f = pair<u32>;
            f(one, true)
        }
    "#;
    let err = support::compile_with_protocols(standalone, &Default::default()).unwrap_err();
    assert!(
        err.contains("count mismatch") || err.contains("missing"),
        "standalone partial instantiation must fail cleanly: {err}"
    );
}

#[test]
fn inherent_and_trait_methods_support_partial_named_and_positional_inference() {
    let source = r#"
        typealias Type = .Type'builtin;
        typealias u32 = .u32'builtin;

        struct DefaultTag {}
        struct CustomTag {}

        struct Boxed where E {
            head: E,
        }

        impl Boxed<E> where E {
            fn combine(self, extra: U) -> bool
            where
                U,
                .Tag: Type = DefaultTag,
            {
                Tag == CustomTag and E == u32 and U == bool
            }

            fn pick_second(self, a: T, b: U) -> U
            where
                T,
                U,
                .Tag: Type = DefaultTag,
            {
                b
            }

            fn static_check(value: T) -> bool
            where
                T,
                .Tag: Type = DefaultTag,
            {
                Tag == CustomTag and T == u32
            }
        }

        trait Inspect {
            fn check_with(self, extra: U) -> bool
            where
                U,
                .Tag: Type = DefaultTag;

            fn static_inspect(value: T) -> bool
            where
                T,
                .Tag: Type = DefaultTag;
        }

        impl Inspect for Boxed<E> where E {
            fn check_with(self, extra: U) -> bool
            where
                U,
                .Tag: Type = DefaultTag,
            {
                Tag == CustomTag and E == u32 and U == bool
            }

            fn static_inspect(value: T) -> bool
            where
                T,
                .Tag: Type = DefaultTag,
            {
                Tag == CustomTag and T == bool
            }
        }

        comptime impl Boxed<E> where E {
            comptime fn ct_combine(self, extra: U) -> bool
            where
                U,
                .flag: bool = false,
            {
                flag and E == u32 and U == bool
            }
        }

        fn runtime_check() -> bool {
            let one: u32 = 1;
            let nine: u32 = 9;
            let b = Boxed<u32> { head: 7 };
            let r1 = b.combine<.Tag = CustomTag>(true);
            let r2 = b.pick_second<u32>(one, true);
            let r3 = Boxed<u32>.static_check<.Tag = CustomTag>(nine);
            let r4 = b.as(Inspect).check_with<.Tag = CustomTag>(true);
            let r5 = b.check_with<.Tag = CustomTag>(true);
            let r6 = Boxed<u32>.as(Inspect).static_inspect<.Tag = CustomTag>(true);
            r1 and r2 and r3 and r4 and r5 and r6
        }

        comptime fn run() -> bool {
            let b = Boxed<u32> { head: 7 };
            b.ct_combine<.flag = true>(true)
        }
    "#;
    let package = support::compile(source);
    nir::phases::validate(&package).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn optional_clauses_before_and_between_mandatory_clauses_infer_cleanly() {
    let source = r#"
        typealias Type = .Type'builtin;
        typealias u32 = .u32'builtin;

        struct DefaultTag {}
        struct CustomTag {}

        comptime fn leading_opt(x: T, y: U) -> bool
        where
            .Tag: Type = DefaultTag,
            T,
            U,
        {
            T == u32 and U == bool and Tag == DefaultTag
        }

        comptime fn leading_opt_custom(x: T, y: U) -> bool
        where
            .Tag: Type = DefaultTag,
            T,
            U,
        {
            T == u32 and U == bool and Tag == CustomTag
        }

        comptime fn middle_dep_opt(x: T, y: U) -> bool
        where
            T,
            .Mirror: Type = T,
            U,
        {
            Mirror == u32 and U == bool
        }

        comptime fn run() -> bool {
            let one: u32 = 1;
            leading_opt<u32>(one, true)
                and leading_opt_custom<u32, .Tag = CustomTag>(one, true)
                and middle_dep_opt<u32>(one, true)
        }
    "#;
    let package = support::compile(source);
    nir::phases::validate(&package).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn variadic_clause_collectors_support_partial_named_and_positional_inference() {
    let source = r#"
        use std.prelude.*;

        struct DefaultTag {}
        struct CustomTag {}

        comptime fn same_type(left: Type, right: Type) -> bool {
            left == right
        }

        comptime fn list_has_len(items: meta.List<Type>, expected: Integer) -> bool {
            items.length() == expected
        }

        fn pack(head: T, rest: Tuple<.Es = Es>) -> bool
        where
            T,
            .Tag: Type = DefaultTag,
            ...Es: meta.List<Type>,
        {
            same_type(Tag, CustomTag) and same_type(T, u32) and list_has_len(Es, 2)
        }

        fn pack_two(a: T, b: U, rest: Tuple<.Es = Es>) -> bool
        where
            T,
            U,
            ...Es: meta.List<Type>,
        {
            same_type(T, u32) and same_type(U, bool) and list_has_len(Es, 2)
        }

        fn empty_pack(head: T) -> bool
        where
            T,
            .Tag: Type = DefaultTag,
            ...Es: meta.List<Type>,
        {
            same_type(Tag, DefaultTag) and same_type(T, u32) and list_has_len(Es, 0)
        }

        fn run() -> bool {
            let seven: u32 = 7;
            let inferred_variadic = pack<.Tag = CustomTag>(seven, (true, 'a'));
            let named_variadic = pack<.Tag = CustomTag, .Es = [bool, char]>(seven, (true, 'a'));
            let partial_pos = pack_two<u32>(seven, true, ('x', false));
            let complete_empty = empty_pack<u32>(seven);
            let complete_positional = pack<u32, bool, char, .Tag = CustomTag>(seven, (true, 'a'));
            inferred_variadic and named_variadic and partial_pos and complete_empty and complete_positional
        }
    "#;
    let case = NativeCase::new(source);
    assert_success(&case.build("0"));
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn variadic_clause_collector_rejects_unconstrained_omitted_variadic_when_mandatory_omitted() {
    let source = r#"
        use std.prelude.*;

        struct DefaultTag {}
        struct CustomTag {}

        comptime fn list_has_len(items: meta.List<Type>, expected: Integer) -> bool {
            items.length() == expected
        }

        fn unconstrained(head: T) -> bool
        where
            T,
            .Tag: Type = DefaultTag,
            ...Es: meta.List<Type>,
        {
            list_has_len(Es, 0)
        }

        fn run() -> bool {
            let seven: u32 = 7;
            unconstrained<.Tag = CustomTag>(seven)
        }
    "#;
    let case = NativeCase::new(source);
    let output = case.build("0");
    assert!(
        !output.status.success(),
        "unconstrained omitted variadic collector must not silently default to empty"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Unresolved")
            || stderr.contains("unresolved")
            || stderr.contains("context"),
        "stderr: {stderr}"
    );
}

#[test]
fn method_partial_named_rejects_unknown_name_and_type_mismatch() {
    let unknown_named = r#"
        typealias Type = .Type'builtin;
        struct DefaultTag {}
        struct Boxed {}
        impl Boxed {
            fn combine(self, extra: U) -> bool
            where
                U,
                .Tag: Type = DefaultTag,
            {
                true
            }
        }
        fn run() -> bool {
            let b = Boxed {};
            b.combine<.NoSuch = DefaultTag>(true)
        }
    "#;
    let err = support::compile_with_protocols(unknown_named, &Default::default()).unwrap_err();
    assert!(err.contains("unknown named"), "{err}");

    let mismatch = r#"
        typealias Type = .Type'builtin;
        typealias u32 = .u32'builtin;
        struct DefaultTag {}
        struct Boxed {}
        impl Boxed {
            fn pick_second(self, a: T, b: U) -> U
            where
                T,
                U,
                .Tag: Type = DefaultTag,
            {
                b
            }
        }
        fn run() -> bool {
            let b = Boxed {};
            b.pick_second<u32>(true, true)
        }
    "#;
    let err = support::compile_with_protocols(mismatch, &Default::default()).unwrap_err();
    assert!(
        err.contains("mismatch") || err.contains("expected") || err.contains("type"),
        "{err}"
    );
}

#[test]
fn native_sysroot_receiver_and_static_methods_partial_named_allocator_o0_and_o3() {
    let source = r#"
        use std.prelude.*;
        use std.mem.{
            AllocErr, Allocator, DefaultAllocator, FixedBufferAllocator, Layout, SystemAllocator,
        };

        struct Formatter {
            prefix: str,
        }

        impl Formatter {
            fn render_debug(
                *self,
                value: *T,
            ) -> !AllocErr String<.A = A, .allocator = allocator>
            where
                T :- Debug,
                .A: Type = DefaultAllocator,
                requires A :- Allocator,
                quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
            {
                var out = String<.A = A, .allocator = allocator>.from_str(self.prefix.*)!;
                let rendered = format_debug<.A = A, .allocator = allocator>(value)!;
                out.ref.push_string(rendered.ref)!;
                out
            }

            fn make_debug(
                value: *T,
            ) -> !AllocErr String<.A = A, .allocator = allocator>
            where
                T :- Debug,
                .A: Type = DefaultAllocator,
                requires A :- Allocator,
                quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
            {
                format_debug<.A = A, .allocator = allocator>(value)
            }
        }

        trait Render {
            fn render_item(
                *self,
                value: *T,
            ) -> !AllocErr String<.A = A, .allocator = allocator>
            where
                T :- Debug,
                .A: Type = DefaultAllocator,
                requires A :- Allocator,
                quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref;
        }

        impl Render for Formatter {
            fn render_item(
                *self,
                value: *T,
            ) -> !AllocErr String<.A = A, .allocator = allocator>
            where
                T :- Debug,
                .A: Type = DefaultAllocator,
                requires A :- Allocator,
                quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
            {
                self.render_debug<.A = A, .allocator = allocator>(value)
            }
        }

        fn exercise(fba: *FixedBufferAllocator) -> !AllocErr bool {
            let fmt = Formatter { prefix: "val=" };
            let n: i32 = -42;
            let s1 = fmt.ref.render_debug<.A = FixedBufferAllocator, .allocator = fba>(n.ref)!;
            let s2 = Formatter.make_debug<.A = FixedBufferAllocator, .allocator = fba>(n.ref)!;
            let s3 = fmt.ref.as(Render).render_item<.A = FixedBufferAllocator, .allocator = fba>(n.ref)!;
            let s4 = fmt.ref.render_item<.A = FixedBufferAllocator, .allocator = fba>(n.ref)!;
            s1.ref.equals_str("val=-42")
                and s2.ref.equals_str("-42")
                and s3.ref.equals_str("val=-42")
                and s4.ref.equals_str("val=-42")
        }

        fn run() -> bool {
            let layout = Layout.new(16384, 16) match {
                found? => found,
                null => return false,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                block! => block,
                error _ => return false,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let ok = exercise(fba.ref) match {
                passed! => passed,
                error _ => false,
            };
            let clean = fba.ref.live_allocations() == 0;
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(backing);
            }
            ok and clean
        }
    "#;

    for opt in ["0", "3"] {
        let case = NativeCase::new(source);
        assert_success(&case.build(opt));
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.stdout, b"(bool true)\n", "opt={opt}");
    }
}
