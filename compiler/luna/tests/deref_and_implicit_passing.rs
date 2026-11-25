//! End-to-end tests for GAP-03-38 (Deref / DerefMut / Reference 3-layer receiver adaptation)
//! and GAP-03-15 (ImplicitPassing protocol and 6 built-in implicit sources).

use comptime_interpretor::Value;
use luna::cli::{self, Command};
use luna::frontend;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "support/execution.rs"]
mod execution_support;

fn unique_temp_dir(prefix: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{seq}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn library_dir() -> PathBuf {
    if let Ok(path) = std::env::var("LUNA_TEST_SYSROOT") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
}

fn compile_sysroot_project(entry: &Path) -> Result<luna::frontend::Artifact, String> {
    let Command::Check(options) = cli::parse([
        "check".into(),
        entry.as_os_str().to_os_string(),
        "--sysroot".into(),
        library_dir().into_os_string(),
        "--build-target".into(),
        "x86_64".into(),
        "linux".into(),
        "64".into(),
    ])
    .unwrap() else {
        panic!("expected check options")
    };
    frontend::compile(&options)
}

fn compile_sysroot_source(prefix: &str, source: &str) -> Result<luna::frontend::Artifact, String> {
    let dir = unique_temp_dir(prefix);
    let entry = dir.join("main.fl");
    fs::write(&entry, source).unwrap();
    let res = compile_sysroot_project(&entry);
    let _ = fs::remove_dir_all(&dir);
    res
}

fn run_sysroot_runtime(prefix: &str, source: &str, fn_name: &str) -> Result<Value, String> {
    let artifact = compile_sysroot_source(prefix, source)?;
    let luna::frontend::Artifact::Nir(package) = artifact else {
        panic!("expected NIR artifact");
    };
    luna::execution::interpret_with_protocols(&package, fn_name, "Copy", "Drop", 10_000_000)
        .map_err(|err| format!("interpret failed: {err}"))
}

fn run_sysroot_comptime(prefix: &str, source: &str, fn_name: &str) -> Result<Value, String> {
    let dir = unique_temp_dir(prefix);
    let entry = dir.join("main.fl");
    fs::write(&entry, source).unwrap();
    let Command::Check(options) = cli::parse([
        "check".into(),
        entry.as_os_str().to_os_string(),
        "--sysroot".into(),
        library_dir().into_os_string(),
        "--build-target".into(),
        "x86_64".into(),
        "linux".into(),
        "64".into(),
    ])
    .unwrap() else {
        panic!("expected check options")
    };
    let res = frontend::invoke_comptime(&options, fn_name, 30_000_000, None, None);
    let _ = fs::remove_dir_all(&dir);
    res
}

#[test]
fn deref_and_deref_mut_multi_hop_reads_writes_methods_and_fields() {
    let src = r#"
        struct Point {
            x: u32,
            y: u32,
        }
        impl Clone for Point {
            fn clone(*self) -> Self {
                Point { x: self.x.*, y: self.y.* }
            }
        }
        impl Copy for Point {}
        impl Point {
            fn sum(self) -> u32 {
                if self.x < 1000 and self.y < 1000 {
                    self.x + self.y
                } else {
                    0
                }
            }
            fn shift(*self, dx: u32) {
                let cur = self.x.*;
                if cur < 1000 and dx < 1000 {
                    self.x = cur + dx;
                }
            }
        }

        struct InnerBox {
            ptr: *Point,
        }
        impl Clone for InnerBox {
            fn clone(*self) -> Self {
                InnerBox { ptr: self.ptr.* }
            }
        }
        impl Copy for InnerBox {}
        impl Deref for InnerBox {
            assoc Target: Type = Point;
            fn deref(*self) -> *Point {
                self.ptr
            }
        }
        impl DerefMut for InnerBox {
            fn deref_mut(*self) -> *Point {
                self.ptr
            }
        }

        struct OuterBox {
            ptr: *InnerBox,
        }
        impl Deref for OuterBox {
            assoc Target: Type = InnerBox;
            fn deref(*self) -> *InnerBox {
                self.ptr
            }
        }
        impl DerefMut for OuterBox {
            fn deref_mut(*self) -> *InnerBox {
                self.ptr
            }
        }

        fn run() -> u32 {
            var p = Point { x: 10, y: 20 };
            var inner = InnerBox { ptr: p.ref };
            let outer = OuterBox { ptr: inner.ref };

            -- Field projection and by-value / mut-receiver method through 2-hop Deref/DerefMut.
            let x_val: u32 = outer.x;
            let sum_val = outer.sum();
            let before: u32 = if x_val < 1000 and sum_val < 1000 {
                x_val + sum_val
            } else {
                0
            };
            outer.shift(5);
            let y_val: u32 = outer.y.*;
            if y_val < 1000 {
                outer.y = y_val + 3;
            }

            -- Explicit Deref read and write via `.*`.
            let copy_pt = outer.*.*;
            let cx: u32 = copy_pt.x;
            if cx < 1000 {
                inner.*.x = cx + 1;
            }

            let px: u32 = p.x;
            let py: u32 = p.y;
            if before < 1000 and px < 1000 and py < 1000 {
                let sum1: u32 = before + px;
                if sum1 < 2000 {
                    sum1 + py
                } else {
                    0
                }
            } else {
                0
            }
        }
    "#;
    // before = 10 + 30 = 40; after shift(5) + y+=3 + inner.*.x = 15+1 = 16; p.x=16, p.y=23 -> 79
    let val = run_sysroot_runtime("luna-deref-multihop", src, "run").unwrap();
    assert_eq!(val, Value::UnsignedInteger(79));
}

#[test]
fn deref_without_deref_mut_allows_reads_and_rejects_writes() {
    let read_only = r#"
        struct Cell {
            value: u32,
        }
        impl Clone for Cell {
            fn clone(*self) -> Self {
                Cell { value: self.value.* }
            }
        }
        impl Copy for Cell {}
        struct ReadBox {
            ptr: *Cell,
        }
        impl Deref for ReadBox {
            assoc Target: Type = Cell;
            fn deref(*self) -> *Cell {
                self.ptr
            }
        }

        fn run() -> u32 {
            var c = Cell { value: 42 };
            let b = ReadBox { ptr: c.ref };
            b.value
        }
    "#;
    assert_eq!(
        run_sysroot_runtime("luna-deref-readonly-ok", read_only, "run").unwrap(),
        Value::UnsignedInteger(42)
    );

    let bad_field_write = r#"
        struct Cell {
            value: u32,
        }
        impl Clone for Cell {
            fn clone(*self) -> Self {
                Cell { value: self.value.* }
            }
        }
        impl Copy for Cell {}
        struct ReadBox {
            ptr: *Cell,
        }
        impl Deref for ReadBox {
            assoc Target: Type = Cell;
            fn deref(*self) -> *Cell {
                self.ptr
            }
        }

        fn run() -> u32 {
            var c = Cell { value: 42 };
            var b = ReadBox { ptr: c.ref };
            b.value = 99;
            c.value
        }
    "#;
    assert!(compile_sysroot_source("luna-deref-readonly-field-err", bad_field_write).is_err());

    let bad_star_write = bad_field_write.replace("b.value = 99;", "b.* = Cell { value: 99 };");
    assert!(compile_sysroot_source("luna-deref-readonly-star-err", &bad_star_write).is_err());
}

#[test]
fn layer1_explicit_as_views_resolve_in_reverse_order_and_explicit_trait_skips_auto_deref() {
    let reverse_order_src = r#"
        struct Base {
            v: u32,
        }
        impl Clone for Base {
            fn clone(*self) -> Self {
                Base { v: self.v.* }
            }
        }
        impl Copy for Base {}
        impl Base {
            fn tag(self) -> u32 { 1 }
            fn only_a(self) -> u32 { 10 }
        }

        newtype ViewB = Base;
        impl ViewB {
            fn tag(self) -> u32 { 2 }
            fn only_b(self) -> u32 { 20 }
        }

        newtype ViewC = Base;
        impl ViewC {
            fn tag(self) -> u32 { 3 }
            fn only_c(self) -> u32 { 30 }
        }

        newtype ViewD = Base;
        impl ViewD {
            fn tag(self) -> u32 { 4 }
        }

        fn inspect(x: Base.as(ViewB).as(ViewC).as(ViewD)) -> u32 {
            -- Reverse order: ViewD -> ViewC -> ViewB -> Base
            let t = x.tag();
            let c = x.only_c();
            let b = x.only_b();
            let a = x.only_a();
            assumes t < 1000 and c < 1000 and b < 1000 and a < 1000;
            t + c + b + a
        }

        fn run() -> u32 {
            let base = Base { v: 99 };
            inspect(base)
        }
    "#;
    assert_eq!(
        run_sysroot_runtime("luna-layer1-reverse", reverse_order_src, "run").unwrap(),
        Value::UnsignedInteger(64)
    );

    // Explicit `.as(Trait)` must NOT fall back to auto-`Deref` into `Inner`.
    let no_auto_deref_after_explicit_trait = r#"
        trait Marker {
            fn ping(*self) -> u32;
        }
        struct Inner {
            n: u32,
        }
        impl Clone for Inner {
            fn clone(*self) -> Self {
                Inner { n: self.n.* }
            }
        }
        impl Copy for Inner {}
        impl Inner {
            fn inner_only(self) -> u32 { self.n }
        }
        struct Outer {
            ptr: *Inner,
        }
        impl Deref for Outer {
            assoc Target: Type = Inner;
            fn deref(*self) -> *Inner { self.ptr }
        }
        impl Marker for Outer {
            fn ping(*self) -> u32 { 7 }
        }
        fn run() -> u32 {
            var i = Inner { n: 5 };
            let o = Outer { ptr: i.ref };
            o.as(Marker).inner_only()
        }
    "#;
    assert!(
        compile_sysroot_source("luna-layer1-no-deref", no_auto_deref_after_explicit_trait).is_err()
    );
}

#[test]
fn deref_cycle_and_unresolved_blocking_are_rejected() {
    let self_cycle = r#"
        struct LoopNode {
            next: *LoopNode,
        }
        impl Deref for LoopNode {
            assoc Target: Type = LoopNode;
            fn deref(*self) -> *LoopNode { self.next }
        }
        fn run(node: LoopNode) -> u32 {
            node.missing_method()
        }
    "#;
    assert!(compile_sysroot_source("luna-deref-self-cycle", self_cycle).is_err());

    let mutual_cycle = r#"
        struct Left { ptr: *Right }
        struct Right { ptr: *Left }
        impl Deref for Left {
            assoc Target: Type = Right;
            fn deref(*self) -> *Right { self.ptr }
        }
        impl Deref for Right {
            assoc Target: Type = Left;
            fn deref(*self) -> *Left { self.ptr }
        }
        fn run(l: Left) -> u32 {
            l.nonexistent()
        }
    "#;
    assert!(compile_sysroot_source("luna-deref-mutual-cycle", mutual_cycle).is_err());

    // Unresolved generic type parameter must block rather than silently picking an arbitrary impl.
    let unresolved_generic = r#"
        trait TargetTrait {
            fn ping(self) -> u32;
        }
        struct Holder where T {
            ptr: *T,
        }
        impl Deref for Holder<T> where T :- TargetTrait {
            assoc Target: Type = T;
            fn deref(*self) -> *T { self.ptr }
        }
        fn probe(h: Holder<T>) -> u32 where T {
            h.ping()
        }
    "#;
    assert!(compile_sysroot_source("luna-deref-unresolved-block", unresolved_generic).is_err());
}

#[test]
fn receiver_evaluated_once_and_unselected_hooks_not_called() {
    let src = r#"
        struct Pointee {
            val: u32,
        }
        impl Clone for Pointee {
            fn clone(*self) -> Self {
                Pointee { val: self.val.* }
            }
        }
        impl Copy for Pointee {}
        impl Pointee {
            fn read(self) -> u32 { self.val }
        }

        struct SmartPtr {
            ptr: *Pointee,
            deref_calls: *u32,
            ref_calls: *u32,
        }
        impl Deref for SmartPtr {
            assoc Target: Type = Pointee;
            fn deref(*self) -> *Pointee {
                let cur = self.deref_calls.*.*;
                if cur < 1000 {
                    self.deref_calls.*.* = cur + 1;
                }
                self.ptr
            }
        }
        impl Reference for SmartPtr {
            assoc Target: Type = Pointee;
            fn reference(*self) -> *Pointee {
                let cur = self.ref_calls.*.*;
                if cur < 1000 {
                    self.ref_calls.*.* = cur + 1;
                }
                self.ptr
            }
        }
        impl SmartPtr {
            fn direct_method(self) -> u32 { 100 }
        }

        fn make_smart(
            make_count: *u32,
            deref_calls: *u32,
            ref_calls: *u32,
            target: *Pointee,
        ) -> SmartPtr {
            let cur = make_count.*;
            if cur < 1000 {
                make_count.* = cur + 1;
            }
            SmartPtr {
                ptr: target,
                deref_calls: deref_calls,
                ref_calls: ref_calls,
            }
        }

        fn get_smart_ptr(slot: *SmartPtr, make_count: *u32) -> *SmartPtr {
            let cur = make_count.*;
            if cur < 1000 {
                make_count.* = cur + 1;
            }
            slot
        }

        fn run() -> bool {
            var target = Pointee { val: 7 };
            var make_count: u32 = 0;
            var deref_calls: u32 = 0;
            var ref_calls: u32 = 0;

            -- Direct method on SmartPtr: evaluates receiver once, never triggers Deref or Reference.
            let a = make_smart(make_count.ref, deref_calls.ref, ref_calls.ref, target.ref).direct_method();
            let check1 = a == 100 and make_count == 1 and deref_calls == 0 and ref_calls == 0;

            -- Deref method on *SmartPtr: evaluates receiver once, triggers Deref once, never triggers Reference.
            var smart = SmartPtr {
                ptr: target.ref,
                deref_calls: deref_calls.ref,
                ref_calls: ref_calls.ref,
            };
            let b = get_smart_ptr(smart.ref, make_count.ref).read();
            let check2 = b == 7 and make_count == 2 and deref_calls == 1 and ref_calls == 0;

            -- Field access via Deref on *SmartPtr: evaluates receiver once, triggers Deref once.
            let c: u32 = get_smart_ptr(smart.ref, make_count.ref).val;
            let check3 = c == 7 and make_count == 3 and deref_calls == 2 and ref_calls == 0;

            check1 and check2 and check3
        }
    "#;
    assert_eq!(
        run_sysroot_runtime("luna-deref-single-eval", src, "run").unwrap(),
        Value::Bool(true)
    );

    // Calling `*self` Deref on an rvalue temporary must NOT fabricate temporary storage.
    let bad_rvalue_deref = src.replace(
        "get_smart_ptr(smart.ref, make_count.ref).read()",
        "make_smart(make_count.ref, deref_calls.ref, ref_calls.ref, target.ref).read()",
    );
    let err = compile_sysroot_source("luna-deref-rvalue-addr-err", &bad_rvalue_deref)
        .err()
        .expect("rvalue deref should fail");
    assert!(
        err.contains("address-of requires a local or field place"),
        "{err}"
    );
}

#[test]
fn layer3_reference_protocol_and_copy_vs_non_copy_pointer_read() {
    let ref_and_copy_src = r#"
        struct Payload {
            v: u32,
        }
        impl Clone for Payload {
            fn clone(*self) -> Self {
                Payload { v: self.v.* }
            }
        }
        impl Copy for Payload {}

        struct Handle {
            ptr: *Payload,
        }
        impl Reference for Handle {
            assoc Target: Type = Payload;
            fn reference(*self) -> *Payload {
                self.ptr
            }
        }

        fn take_ptr(p: *Payload) -> u32 {
            p.*.v
        }

        fn run() -> u32 {
            var p = Payload { v: 19 };
            let h = Handle { ptr: p.ref };
            let via_ref_arg = take_ptr(h.ref);
            let via_ref_ptr: *Payload = h.ref;
            let via_ref_deref = via_ref_ptr.*;
            assumes via_ref_arg < 1000 and via_ref_deref.v < 1000;
            via_ref_arg + via_ref_deref.v
        }
    "#;
    assert_eq!(
        run_sysroot_runtime("luna-layer3-ref-copy", ref_and_copy_src, "run").unwrap(),
        Value::UnsignedInteger(38)
    );

    // Non-Copy struct read through pointer dereference `.*` MUST be rejected at resource check,
    // and target-guided auto-read of a non-Copy field MUST be rejected at elaboration.
    let non_copy_read = r#"
        struct NonCopy {
            v: u32,
        }
        fn run() -> u32 {
            var n = NonCopy { v: 1 };
            let p = n.ref;
            let copy = p.*;
            copy.v
        }
    "#;
    assert!(run_sysroot_runtime("luna-layer3-noncopy-err", non_copy_read, "run").is_err());

    let non_copy_field_auto_read = r#"
        struct NonCopy {
            v: u32,
        }
        struct Holder {
            item: NonCopy,
        }
        fn bad(h: *Holder) -> NonCopy {
            let x: NonCopy = h.item;
            x
        }
    "#;
    assert!(
        compile_sysroot_source("luna-layer3-noncopy-field-err", non_copy_field_auto_read).is_err()
    );

    // Non-Copy target read through Deref `.*` MUST also be rejected.
    let non_copy_deref = r#"
        struct NonCopy {
            v: u32,
        }
        struct BoxNC {
            ptr: *NonCopy,
        }
        impl Deref for BoxNC {
            assoc Target: Type = NonCopy;
            fn deref(*self) -> *NonCopy { self.ptr }
        }
        fn bad(b: BoxNC) -> NonCopy {
            b.*
        }
    "#;
    assert!(compile_sysroot_source("luna-layer3-noncopy-deref-err", non_copy_deref).is_err());
}

#[test]
fn comptime_forbids_pointers_deref_and_reference() {
    let ct_ptr_param = r#"
        comptime fn bad(p: *u32) -> u32 { 0 }
    "#;
    assert!(compile_sysroot_source("luna-ct-ptr-param", ct_ptr_param).is_err());

    let ct_ref_expr = r#"
        comptime fn bad() -> u32 {
            var x: u32 = 1;
            let p = x.ref;
            p.*
        }
    "#;
    assert!(compile_sysroot_source("luna-ct-ref-expr", ct_ref_expr).is_err());
}

#[test]
fn implicit_passing_six_builtin_sources_runtime_and_comptime() {
    let src = r#"
        use meta.ast.SourceSpan;

        fn probe_line(implicit __line__: u32) -> u32 { __line__ }
        fn probe_file(implicit __file__: str) -> str { __file__ }
        fn probe_caller(implicit __caller__: str) -> str { __caller__ }
        fn probe_package(implicit __package__: str) -> str { __package__ }
        fn probe_src(implicit __src__: SourceSpan) -> SourceSpan { __src__ }

        pure comptime fn ct_line(implicit __line__: u32) -> u32 { __line__ }
        pure comptime fn ct_caller(implicit __caller__: str) -> str { __caller__ }
        pure comptime fn ct_src(implicit __src__: SourceSpan) -> SourceSpan { __src__ }

        mod sub {
            pub fn probe_module(implicit __module__: str) -> str { __module__ }
            pub fn from_sub() -> str {
                probe_module()
            }
        }

        fn run_line() -> u32 {
            probe_line()
        }

        fn run_caller() -> str {
            probe_caller()
        }

        fn run_package() -> str {
            probe_package()
        }

        fn run_file() -> str {
            probe_file()
        }

        fn run_module_sub() -> str {
            sub.from_sub()
        }

        fn run_src_line_span() -> bool {
            let s = probe_src();
            s.end() > s.start() and s.expansion_id() == -1
        }

        comptime fn run_comptime_checks() -> bool {
            let l = ct_line();
            let c = ct_caller();
            let s = ct_src();
            l > 0 and c != "" and s.end() > s.start() and not s.is_synthetic()
        }
    "#;
    let line_val = run_sysroot_runtime("luna-imp-line", src, "run_line").unwrap();
    let Value::UnsignedInteger(line_num) = line_val else {
        panic!("expected UnsignedInteger, got {line_val:?}");
    };
    assert!(line_num > 15, "unexpected line number {line_num}");

    let caller_val = run_sysroot_runtime("luna-imp-caller", src, "run_caller").unwrap();
    assert_eq!(caller_val, Value::String("input.run_caller".into()));

    let pkg_val = run_sysroot_runtime("luna-imp-pkg", src, "run_package").unwrap();
    assert_eq!(pkg_val, Value::String("input".into()));

    let file_val = run_sysroot_runtime("luna-imp-file", src, "run_file").unwrap();
    let Value::String(file_str) = file_val else {
        panic!("expected String, got {file_val:?}");
    };
    assert!(
        file_str.ends_with("main.fl"),
        "unexpected __file__: {file_str}"
    );

    let mod_val = run_sysroot_runtime("luna-imp-mod", src, "run_module_sub").unwrap();
    assert_eq!(mod_val, Value::String("input.sub".into()));

    let src_ok = run_sysroot_runtime("luna-imp-src", src, "run_src_line_span").unwrap();
    assert_eq!(src_ok, Value::Bool(true));

    let ct_ok = run_sysroot_comptime("luna-imp-ct", src, "run_comptime_checks").unwrap();
    assert_eq!(ct_ok, Value::Bool(true));
}

#[test]
fn implicit_passing_custom_protocol_explicit_bypass_and_no_local_capture() {
    let custom_and_bypass = r#"
        struct Tag {
            code: u32,
        }
        impl Clone for Tag {
            fn clone(*self) -> Self {
                Tag { code: self.code.* }
            }
        }
        impl Copy for Tag {}
        impl Splice for Tag {
            assoc RuntimeTarget: Type = Tag;
            fn splice(comptime data: Tag) -> Tag { data }
        }
        impl ImplicitPassing<.ctx> for Tag {
            assoc ArgType: Type = Tag;
            pure comptime fn pass(
                caller: Context,
                callee: Context,
                calling: CallDescription,
            ) -> < |- Tag > {
                '( Tag { code: 777 } )
            }
        }

        fn read_tag(implicit ctx: Tag) -> u32 {
            ctx.code
        }

        fn run() -> u32 {
            -- Even with a same-named local `ctx`, omitted call uses ImplicitPassing, NOT local capture.
            let ctx = Tag { code: 123 };
            let via_protocol = read_tag();
            -- Explicit `.ctx = ...` bypasses ImplicitPassing.
            let via_explicit = read_tag(.ctx = ctx);
            assumes via_protocol < 100000 and via_explicit < 100000;
            via_protocol + via_explicit
        }
    "#;
    assert_eq!(
        run_sysroot_runtime("luna-imp-custom-bypass", custom_and_bypass, "run").unwrap(),
        Value::UnsignedInteger(900)
    );

    // Without an `ImplicitPassing` impl, a same-named local variable MUST NOT be captured!
    let no_local_capture = r#"
        struct UnboundCtx {
            id: u32,
        }
        fn needs_ctx(implicit ctx: UnboundCtx) -> u32 {
            ctx.id
        }
        fn run() -> u32 {
            let ctx = UnboundCtx { id: 55 };
            needs_ctx()
        }
    "#;
    assert!(compile_sysroot_source("luna-imp-no-local-capture", no_local_capture).is_err());

    // Explicit `.ctx = ctx` succeeds even when `ImplicitPassing` is absent.
    let explicit_without_protocol =
        no_local_capture.replace("needs_ctx()", "needs_ctx(.ctx = ctx)");
    assert_eq!(
        run_sysroot_runtime(
            "luna-imp-explicit-no-protocol",
            &explicit_without_protocol,
            "run"
        )
        .unwrap(),
        Value::UnsignedInteger(55)
    );
}

#[test]
fn implicit_passing_interacts_with_spreads_defaults_generics_and_open_functions() {
    let src = r#"
        fn collect(
            base: u32,
            ...rest: Tuple<u32, u32>,
            implicit __line__: u32,
            .bonus: u32 = __line__,
        ) -> u32 {
            let (r0, r1) = rest;
            assumes base < 1000 and r0 < 1000 and r1 < 1000 and __line__ < 100000 and bonus < 100000;
            base + r0 + r1 + __line__ + bonus
        }

        fn generic_tag(x: T, implicit __line__: u32) -> u32 where T {
            __line__
        }

        fn open_tag(comptime bias: u32, x: u32, implicit __line__: u32) -> u32 {
            assumes bias < 1000 and x < 1000 and __line__ < 100000;
            bias + x + __line__
        }

        fn run() -> bool {
            let spread_args1: Tuple<u32, u32> = (10, 20);
            let spread_args2: Tuple<u32, u32> = (10, 20);
            let a = collect(5, ...spread_args1);
            let b = collect(5, ...spread_args2, .bonus = 1);
            let g = generic_tag(true);
            let o = open_tag(10, 3);
            a > 35 and b > 36 and a > b and g > 0 and o > 13
        }
    "#;
    assert_eq!(
        run_sysroot_runtime("luna-imp-spread-open", src, "run").unwrap(),
        Value::Bool(true)
    );

    // Without bound `implicit-passing-protocol` (`Protocols::default()`), omitted implicit still fails.
    let missing_protocol =
        "fn f(implicit __line__: bool) -> bool { __line__ } fn run() -> bool { f() }";
    assert!(
        execution_support::compile_with_protocols(missing_protocol, &Default::default()).is_err()
    );
}
