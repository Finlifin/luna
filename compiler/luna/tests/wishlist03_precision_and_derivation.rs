//! End-to-end Milestone B integration tests against the real built-in packages (`--sysroot library`):
//! - `OQ-03-06`: `(depth, concrete_assigned)` impl index precision coexistence & priority,
//!   high-precision `derive Clone` over lower-precision `impl Clone`, and closed-`quote`
//!   enforcement on derived `Clone` / `Default`.
//! - `OQ-03-01` / `GAP-08-35`: `^attrs.derivation.default` marker export and default strategy
//!   selection.
//! - `WL-03-12` / `WL-03-14`: generic `derive` requests with `derive fn` strategy inputs,
//!   computations, constraints, `quote`-bearing containers, and failure rollback.
//! - `OQ-03-03`: `type_view` (`T.as(View)`) in `impl` subjects, trait arguments, constraints,
//!   fields, type-as-value expressions, and non-bidirectional rejection.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use comptime_interpretor::Value;

fn compile_native_and_run(source: &str) -> Output {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "luna-wishlist03-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let source_path = root.join("main.fl");
    let output_path = root.join("program");
    fs::write(&source_path, source).unwrap();

    let library = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let build = Command::new(env!("CARGO_BIN_EXE_luna"))
        .args([
            "build".into(),
            source_path.into_os_string(),
            "--entry".into(),
            "run".into(),
            "--output".into(),
            output_path.clone().into_os_string(),
            "--opt-level".into(),
            "0".into(),
            "--fuel".into(),
            "10000000".into(),
            "--sysroot".into(),
            library.into_os_string(),
        ])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "native build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(&output_path).output().unwrap();
    let _ = fs::remove_dir_all(&root);
    run
}

#[test]
fn concrete_assigned_precision_coexists_and_prioritizes_closed_allocator_impls() {
    let source = r#"
        use std.collections.Vec;
        use std.mem.{AllocErr, Allocator, DefaultAllocator, Layout, RawBlock, ReallocErr};

        struct LocalAlloc {
            id: u32,
        }

        impl Allocator for LocalAlloc {
            fn allocate(*self, layout: Layout) -> !AllocErr RawBlock {
                DefaultAllocator.GLOBAL.ref.as(Allocator).allocate(layout)
            }

            fn allocate_zeroed(*self, layout: Layout) -> !AllocErr RawBlock {
                DefaultAllocator.GLOBAL.ref.as(Allocator).allocate_zeroed(layout)
            }

            unsafe fn deallocate(*self, block: RawBlock) {
                DefaultAllocator.GLOBAL.ref.as(Allocator).deallocate(block);
            }

            unsafe fn reallocate(*self, block: RawBlock, new_layout: Layout) -> !ReallocErr RawBlock {
                DefaultAllocator.GLOBAL.ref.as(Allocator).reallocate(block, new_layout)
            }
        }

        trait AllocTag {
            fn tag(self) -> u32;
        }

        impl AllocTag for Vec<T, .A = DefaultAllocator, .allocator = DefaultAllocator.GLOBAL.ref>
        where
            T :- Clone,
        {
            fn tag(self) -> u32 {
                1
            }
        }

        impl AllocTag for Vec<T, .A = A, .allocator = alloc>
        where
            T :- Clone,
            A: Type,
            requires A :- Allocator,
            quote alloc: lift *A,
        {
            fn tag(self) -> u32 {
                2
            }
        }

        struct BoxedItem
        where
            T,
            .A: Type = DefaultAllocator,
            requires A :- Allocator,
            quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
        {
            value: u32,
        }

        impl Clone for BoxedItem<T, .A = DefaultAllocator, .allocator = DefaultAllocator.GLOBAL.ref>
        where
            T :- Clone,
        {
            fn clone(*self) -> Self {
                let v = self.value.*;
                let next = if v < 1000 { v + 100 } else { v };
                BoxedItem<T, .A = DefaultAllocator, .allocator = DefaultAllocator.GLOBAL.ref> {
                    value: next,
                }
            }
        }

        impl Clone for BoxedItem<T, .A = A, .allocator = alloc>
        where
            T :- Clone,
            A: Type,
            requires A :- Allocator,
            quote alloc: lift *A,
        {
            fn clone(*self) -> Self {
                let v = self.value.*;
                let next = if v < 1000 { v + 1 } else { v };
                BoxedItem<T, .A = A, .allocator = alloc> {
                    value: next,
                }
            }
        }

        fn run() -> bool {
            let custom = LocalAlloc { id: 9 };
            let default_vec = Vec<i32>.new();
            let custom_vec = Vec<i32, .A = LocalAlloc, .allocator = custom.ref>.new();
            let vec_ok = default_vec.as(AllocTag).tag() == 1
                and custom_vec.as(AllocTag).tag() == 2;

            let b_default = BoxedItem<i32> { value: 7 };
            let c_default = b_default.clone();
            let b_custom = BoxedItem<i32, .A = LocalAlloc, .allocator = custom.ref> { value: 7 };
            let c_custom = b_custom.clone();
            vec_ok and c_default.value == 107 and c_custom.value == 8
        }
    "#;
    let output = compile_native_and_run(source);
    assert!(
        output.status.success(),
        "native run failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn higher_precision_derive_coexists_with_lower_precision_impl_and_enforces_closed_quote() {
    let valid_source = r#"
        use std.mem.{AllocErr, Allocator, DefaultAllocator, Layout, RawBlock, ReallocErr};

        struct LocalAlloc {
            id: u32,
        }

        impl Allocator for LocalAlloc {
            fn allocate(*self, layout: Layout) -> !AllocErr RawBlock {
                DefaultAllocator.GLOBAL.ref.as(Allocator).allocate(layout)
            }

            fn allocate_zeroed(*self, layout: Layout) -> !AllocErr RawBlock {
                DefaultAllocator.GLOBAL.ref.as(Allocator).allocate_zeroed(layout)
            }

            unsafe fn deallocate(*self, block: RawBlock) {
                DefaultAllocator.GLOBAL.ref.as(Allocator).deallocate(block);
            }

            unsafe fn reallocate(*self, block: RawBlock, new_layout: Layout) -> !ReallocErr RawBlock {
                DefaultAllocator.GLOBAL.ref.as(Allocator).reallocate(block, new_layout)
            }
        }

        struct Buffer
        where
            T,
            .A: Type = DefaultAllocator,
            requires A :- Allocator,
            quote .allocator: lift *A = DefaultAllocator.GLOBAL.ref,
        {
            tag: u32,
        }

        impl Clone for Buffer<T, .A = A, .allocator = alloc>
        where
            T :- Clone,
            A: Type,
            requires A :- Allocator,
            quote alloc: lift *A,
        {
            fn clone(*self) -> Self {
                let t = self.tag.*;
                let next = if t < 1000 { t + 1 } else { 0 };
                Buffer<T, .A = A, .allocator = alloc> {
                    tag: next,
                }
            }
        }

        derive fn buffer_default_clone for Clone
        where
            X: Type,
            requires X :- Clone,
        {
            fn clone(*self) -> Self {
                let t = self.tag.*;
                let next = if t < 1000 { t + 50 } else { 0 };
                Buffer<X> {
                    tag: next,
                }
            }
        }

        derive Clone.use(buffer_default_clone<T>) for Buffer<T, .A = DefaultAllocator, .allocator = DefaultAllocator.GLOBAL.ref>
        where
            T :- Clone;

        struct ClosedHolder
        where
            quote alloc: lift *DefaultAllocator,
        {
            count: u32,
        }

        derive fn closed_default for Default {
            fn default() -> Self {
                ClosedHolder<DefaultAllocator.GLOBAL.ref> {
                    count: 42,
                }
            }
        }

        derive Default.use(closed_default) for ClosedHolder<DefaultAllocator.GLOBAL.ref>;

        fn run() -> u8 {
            let custom = LocalAlloc { id: 3 };
            let b_default = Buffer<i32> { tag: 5 };
            let c_default = b_default.clone();
            let b_custom = Buffer<i32, .A = LocalAlloc, .allocator = custom.ref> { tag: 5 };
            let c_custom = b_custom.clone();
            let held = ClosedHolder<DefaultAllocator.GLOBAL.ref>.as(Default).default();
            if c_default.tag == 55 and c_custom.tag == 6 and held.count == 42 {
                0
            } else {
                1
            }
        }
    "#;
    let out = compile_native_and_run(valid_source);
    assert_eq!(out.status.code(), Some(0), "{out:?}");

    let open_quote_clone = r#"
        use std.mem.{Allocator};

        struct OpenBuf
        where
            T,
            A: Type,
            requires A :- Allocator,
            quote alloc: lift *A,
        {
            tag: u32,
        }

        derive fn open_clone for Clone
        where
            X: Type,
            Y: Type,
            requires X :- Clone,
            requires Y :- Allocator,
            quote a: lift *Y,
        {
            fn clone(*self) -> Self {
                OpenBuf<X, Y, a> {
                    tag: self.tag.*,
                }
            }
        }

        derive Clone.use(open_clone<T, A, alloc>) for OpenBuf<T, A, alloc>
        where
            T :- Clone,
            A: Type,
            requires A :- Allocator,
            quote alloc: lift *A;
    "#;
    let clone_err = support::compile(open_quote_clone).unwrap_err();
    assert!(
        clone_err.contains("closed quote arguments"),
        "expected closed-quote rejection for derive Clone, got: {clone_err}"
    );

    let open_quote_default = r#"
        struct Cfg {
            v: u32,
        }

        struct Holder
        where
            quote cfg: lift *Cfg,
        {
            v: u32,
        }

        derive fn open_default for Default
        where
            quote cfg: lift *Cfg,
        {
            fn default() -> Self {
                Holder<cfg> {
                    v: cfg.v.*,
                }
            }
        }

        derive Default.use(open_default<cfg>) for Holder<cfg>
        where
            quote cfg: lift *Cfg;
    "#;
    let default_err = support::compile(open_quote_default).unwrap_err();
    assert!(
        default_err.contains("closed quote arguments"),
        "expected closed-quote rejection for derive Default, got: {default_err}"
    );
}

#[test]
fn attrs_derivation_default_selects_default_strategy_with_sysroot() {
    let source = r#"
        use attrs.derivation.default as auto_derive;

        trait Flag {
            fn is_default(self) -> bool;
        }

        ^attrs.derivation.default
        derive fn flag_default for Flag {
            fn is_default(self) -> bool {
                true
            }
        }

        trait Tag {
            fn tag_code(self) -> u32;
        }

        ^auto_derive
        derive fn tag_default for Tag {
            fn tag_code(self) -> u32 {
                77
            }
        }

        struct Subject {}
        derive Flag for Subject;
        derive Tag for Subject;

        fn run() -> bool {
            let s = Subject {};
            s.as(Flag).is_default() and s.as(Tag).tag_code() == 77
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn generic_derive_supports_strategy_inputs_computations_quote_and_rollback() {
    let source = r#"
        trait Measure {
            fn score(*self) -> u32;
        }

        derive fn configured(base: u32, .bias: u32 = 5) for Measure
        where
            requires base > 0,
            requires base < 1000,
            requires bias < 1000,
        {
            let total: u32 = base + bias;
            fn score(*self) -> u32 {
                total
            }
        }

        trait BoundTag {
            fn bound_ok(*self) -> bool;
        }

        derive fn by_bound(active: bool) for BoundTag
        where
            X: Type,
            requires X :- Clone,
        {
            let flag: bool = not active;
            fn bound_ok(*self) -> bool {
                not flag
            }
        }

        struct GenericBox
        where
            T,
        {
            value: T,
        }

        derive Measure.use(configured(10)) for GenericBox<T> where T;
        derive BoundTag.use(by_bound<T>(true)) for GenericBox<T> where T :- Clone;

        struct Env {
            factor: u32,
        }

        struct QuoteBox
        where
            T,
            quote env: lift *Env,
        {
            value: T,
        }

        derive Measure.use(configured(20, .bias = 3)) for QuoteBox<T, env>
        where
            T,
            quote env: lift *Env;

        fn run() -> u8 {
            let env = Env { factor: 4 };
            let g = GenericBox<i32> { value: 9 };
            let q = QuoteBox<i32, env.ref> { value: 1 };
            if g.as(Measure).score() == 15
                and g.as(BoundTag).bound_ok()
                and q.as(Measure).score() == 23
            {
                0
            } else {
                1
            }
        }
    "#;
    let out = compile_native_and_run(source);
    assert_eq!(out.status.code(), Some(0), "{out:?}");

    // Rollback on failure: failed precondition / missing trait bound / invalid signature
    // must fail compilation without emitting half-built implementations.
    let failing_precondition = r#"
        trait Measure {
            fn score(*self) -> u32;
        }

        derive fn configured(base: u32) for Measure
        where
            requires base > 0,
            requires base < 1000,
        {
            let total: u32 = base + 1;
            fn score(*self) -> u32 {
                total
            }
        }

        struct GenericBox where T {
            value: T,
        }

        derive Measure.use(configured(0)) for GenericBox<T> where T;
    "#;
    assert!(support::compile(failing_precondition).is_err());

    let missing_bound = r#"
        trait BoundTag {
            fn bound_ok(*self) -> bool;
        }

        derive fn by_bound(active: bool) for BoundTag
        where
            X: Type,
            requires X :- Clone,
        {
            let flag: bool = active;
            fn bound_ok(*self) -> bool {
                flag
            }
        }

        struct GenericBox where T {
            value: T,
        }

        derive BoundTag.use(by_bound<T>(true)) for GenericBox<T> where T;
    "#;
    assert!(support::compile(missing_bound).is_err());
}

#[test]
fn type_view_works_across_impl_subjects_clauses_fields_and_type_values() {
    let source = r#"
        newtype LocalWord = u32;

        impl LocalWord {
            pub fn doubled(self) -> u32 {
                let value: u32 = self.as(u32);
                if value < 1000 { value * 2 } else { 0 }
            }

            pub fn ptr_bias(*self) -> u32 {
                1
            }
        }

        trait WordOps {
            fn compute(*self) -> u32;
        }

        impl WordOps for u32.as(LocalWord) {
            fn compute(*self) -> u32 {
                let d: u32 = self.*.doubled();
                let b: u32 = self.*.ptr_bias();
                if d < 10000 and b < 1000 { d + b } else { 0 }
            }
        }

        trait PairTag
        where
            Arg: Type,
        {
            fn pair_tag(*self) -> u32;
        }

        struct WordWrapper
        where
            W: Type,
            requires W == u32.as(LocalWord),
        {
            raw: W,
        }

        impl PairTag<u32.as(LocalWord)> for WordWrapper<u32.as(LocalWord)> {
            fn pair_tag(*self) -> u32 {
                let raw: u32 = self.raw.*;
                if raw < 1000 { raw + 10 } else { 0 }
            }
        }

        comptime fn same_as_u32(comptime T: Type) -> bool {
            T == u32
        }

        comptime fn check_type_as_value() -> bool {
            let viewed: Type = u32.as(LocalWord);
            same_as_u32(u32.as(LocalWord)) and viewed == u32
        }

        const TYPE_VALUE_OK: bool = check_type_as_value();

        fn run() -> u8 {
            let x: u32 = 12;
            let holder = WordWrapper<u32.as(LocalWord)> { raw: 5 };
            if x.as(WordOps).compute() == 25
                and holder.as(PairTag<u32>).pair_tag() == 15
                and TYPE_VALUE_OK
            {
                0
            } else {
                1
            }
        }
    "#;
    let out = compile_native_and_run(source);
    assert_eq!(out.status.code(), Some(0), "{out:?}");

    let invalid_view_on_impl = r#"
        struct NotAView {
            x: u32,
        }

        trait WordOps {
            fn compute(*self) -> u32;
        }

        impl WordOps for u32.as(NotAView) {
            fn compute(*self) -> u32 {
                self.*
            }
        }
    "#;
    let err = support::compile(invalid_view_on_impl).unwrap_err();
    assert!(
        err.contains("bidirectional newtype relationship"),
        "expected bidirectional newtype check on impl subject view, got: {err}"
    );
}
