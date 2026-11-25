//! Integration tests for generalized normal-form variadic constructors,
//! `VariadicCollector`, and `ExtendedVariadicCollector` (runtime + comptime phases).
#[path = "support/execution.rs"]
mod execution_support;

#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot_support;

use comptime_interpretor::Value;
use execution_support::compile_with_source_bindings;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const PRIMITIVES: &str = include_str!("../../../library/builtin/primitives.fl");

const OPERATORS: &str = include!("support/operators_source.rs");

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct WorkDir(PathBuf);

impl WorkDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "luna-variadic-collectors-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn collector_header() -> String {
    format!(
        "{PRIMITIVES}\n{OPERATORS}\n\
        trait Copy {{}}\n\
        trait Drop {{ fn drop(*self); }}\n\
        impl Copy for bool {{}}\n\
        impl Copy for u8 {{}}\n\
        impl Copy for u16 {{}}\n\
        impl Copy for u32 {{}}\n\
        impl Copy for usize {{}}\n\
        impl Copy for void {{}}\n\
        impl Copy for Type {{}}\n\
        pure comptime fn TupleElements<T> -> Type {{ T }}\n\
        pure comptime fn Tuple<...Es: TupleElements<Type>> -> Type {{ bool }}\n"
    )
}

fn collector_selectors() -> luna::protocols::type_constructors::Selectors<'static> {
    luna::protocols::type_constructors::Selectors {
        copy: Some("input.Copy"),
        drop: Some("input.Drop"),
        meta_list: Some("input.TupleElements"),
        tuple: Some("input.Tuple"),
        variadic_collector: Some("input.VariadicCollector"),
        extended_variadic_collector: Some("input.ExtendedVariadicCollector"),
        ..Default::default()
    }
}

#[test]
fn user_variadic_nf_constructors_generalize_without_special_cases() {
    let source = format!(
        "{}
        impl Copy for TupleElements<Type> {{}}
        pure comptime fn Identity<T> -> Type {{ T }}
        pure comptime fn TaggedList<...Xs: TupleElements<Type>> -> Type {{ bool }}

        struct TypeBag {{ count: u32, first: Type }}
        impl Copy for TypeBag {{}}
        comptime impl VariadicCollector for TypeBag {{
            comptime fn comptime_variadic_collect(args: S) -> TypeBag where S {{
                inline if S == Tuple<Type, Type> {{
                    let typed_args: Tuple<Type, Type> = args;
                    let (a, _b) = typed_args;
                    TypeBag {{ count: 2, first: a }}
                }} else {{
                    TypeBag {{ count: 0, first: bool }}
                }}
            }}
        }}
        pure comptime fn CustomBagNF<...Xs: TypeBag> -> Type {{ bool }}

        const DIRECT_LIST = TaggedList<bool, u8>;
        const EMPTY_LIST = TaggedList<>;
        const NAMED_LIST = TaggedList<.Xs = [bool, u8]>;
        const SPREAD_LIST = TaggedList<...[bool, u8]>;
        const CUSTOM_DIRECT = CustomBagNF<bool, u8>;
        const CUSTOM_EMPTY = CustomBagNF<>;
        const CUSTOM_NAMED = CustomBagNF<.Xs = TypeBag {{ count: 2, first: bool }}>;
        const ID_BOOL = Identity<bool>;

        comptime fn check_nf() -> bool {{
            DIRECT_LIST == NAMED_LIST
                and DIRECT_LIST == SPREAD_LIST
                and CUSTOM_DIRECT == CUSTOM_NAMED
                and (ID_BOOL match {{ Identity<bool> => true, _ => false }})
        }}
        fn run() -> bool {{ true }}",
        collector_header()
    );
    let package =
        compile_with_source_bindings(&source, &Default::default(), collector_selectors()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "check_nf", 100_000).unwrap(),
        Value::Bool(true)
    );
    let non_unfolding = format!(
        "{}
        pure comptime fn Identity<T> -> Type {{ T }}
        fn bad(value: Identity<bool>) -> bool {{ value }}",
        collector_header()
    );
    assert!(
        compile_with_source_bindings(&non_unfolding, &Default::default(), collector_selectors())
            .is_err()
    );
}

#[test]
fn runtime_variadic_collector_keeps_tmp_pinned_and_drops_after_call_in_vm_and_llvm() {
    let program = format!(
        "{}
        struct StackBuf {{ first: u32, second: u32, sentinel: u32 }}
        impl Drop for StackBuf {{
            fn drop(*self) {{
                let _ = self.sentinel.*;
            }}
        }}
        struct PinSlice {{ p1: *u32, p2: *u32, len: u32 }}
        impl Copy for PinSlice {{}}

        impl VariadicCollector for PinSlice {{
            assoc Tmp: for<S: Type> Type = |_s: Type| -> Type {{ StackBuf }};

            fn variadic_init() -> StackBuf where _S {{
                StackBuf {{ first: 0, second: 0, sentinel: 99 }}
            }}

            fn variadic_collect(tmp: *StackBuf, args: S) -> PinSlice where S {{
                inline if S == Tuple<u32, u32> {{
                    let typed_args: Tuple<u32, u32> = args;
                    let (x, y) = typed_args;
                    tmp.first.* = x;
                    tmp.second.* = y;
                    PinSlice {{ p1: tmp.*.first.ref, p2: tmp.*.second.ref, len: 2 }}
                }} else {{
                    PinSlice {{ p1: tmp.*.first.ref, p2: tmp.*.second.ref, len: 0 }}
                }}
            }}
        }}

        fn sum_pinned(...xs: PinSlice) -> u32 {{
            let a = xs.p1.*;
            let b = xs.p2.*;
            assumes a < 1000 and b < 1000 and xs.len < 100;
            a + b + xs.len
        }}

        fn run() -> bool {{
            let p1: u32 = 10;
            let p2: u32 = 32;
            sum_pinned(p1, p2) == 44
        }}",
        collector_header()
    );

    let package =
        compile_with_source_bindings(&program, &Default::default(), collector_selectors()).unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
            .unwrap(),
        Value::Bool(true)
    );

    let dangling = format!(
        "{}
        struct StackBuf {{ first: u32, second: u32, sentinel: u32 }}
        impl Drop for StackBuf {{
            fn drop(*self) {{
                let _ = self.sentinel.*;
            }}
        }}
        struct PinSlice {{ p1: *u32, p2: *u32, len: u32 }}
        impl Copy for PinSlice {{}}

        impl VariadicCollector for PinSlice {{
            assoc Tmp: for<S: Type> Type = |_s: Type| -> Type {{ StackBuf }};

            fn variadic_init() -> StackBuf where _S {{
                StackBuf {{ first: 0, second: 0, sentinel: 99 }}
            }}

            fn variadic_collect(tmp: *StackBuf, args: S) -> PinSlice where S {{
                inline if S == Tuple<u32, u32> {{
                    let typed_args: Tuple<u32, u32> = args;
                    let (x, y) = typed_args;
                    tmp.first.* = x;
                    tmp.second.* = y;
                    PinSlice {{ p1: tmp.*.first.ref, p2: tmp.*.second.ref, len: 2 }}
                }} else {{
                    PinSlice {{ p1: tmp.*.first.ref, p2: tmp.*.second.ref, len: 0 }}
                }}
            }}
        }}

        fn escape_pinned(...xs: PinSlice) -> PinSlice {{ xs }}

        fn run() -> u32 {{
            let p1: u32 = 10;
            let p2: u32 = 32;
            let escaped = escape_pinned(p1, p2);
            escaped.p1.*
        }}",
        collector_header()
    );
    let dangling_pkg =
        compile_with_source_bindings(&dangling, &Default::default(), collector_selectors())
            .unwrap();
    assert!(
        luna::execution::interpret_with_protocols(&dangling_pkg, "run", "Copy", "Drop", 1_000_000)
            .is_err(),
        "Tmp dropped at Boundary::ScopeEnd must not allow returned interior pointers to outlive the call"
    );

    let work = WorkDir::new();
    work.write("input.fl", &program);
    let build = work.run(&[
        "input.fl",
        "--build",
        "run",
        "--copy-protocol",
        "Copy",
        "--drop-protocol",
        "Drop",
        "--meta-list-type",
        "input.TupleElements",
        "--tuple-type",
        "input.Tuple",
        "--variadic-collector-protocol",
        "input.VariadicCollector",
        "--extended-variadic-collector-protocol",
        "input.ExtendedVariadicCollector",
        "--output",
        "native_bin",
    ]);
    assert!(
        build.status.success(),
        "LLVM build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let exec = Command::new(work.0.join("native_bin")).output().unwrap();
    assert!(
        exec.status.success(),
        "native binary failed: {}",
        String::from_utf8_lossy(&exec.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&exec.stdout).trim(), "(bool true)");
}

#[test]
fn two_phase_variadic_collector_supports_runtime_comptime_empty_and_rejects_wrong_phase() {
    let source = format!(
        "{}
        struct DualBag {{ total: u32 }}
        impl Copy for DualBag {{}}

        impl VariadicCollector for DualBag {{
            assoc Tmp: for<S: Type> Type = |_s: Type| -> Type {{ void }};

            fn variadic_init() -> void where _S {{}}

            fn variadic_collect(_tmp: *void, args: S) -> DualBag where S {{
                inline if S == Tuple<u32, u32> {{
                    let typed_args: Tuple<u32, u32> = args;
                    let (a, b) = typed_args;
                    assumes a < 1000 and b < 1000;
                    DualBag {{ total: a + b }}
                }} else {{
                    DualBag {{ total: 0 }}
                }}
            }}
        }}

        comptime impl VariadicCollector for DualBag {{
            comptime fn comptime_variadic_collect(args: S) -> DualBag where S {{
                inline if S == Tuple<u32, u32> {{
                    let typed_args: Tuple<u32, u32> = args;
                    let (a, b) = typed_args;
                    assumes a < 1000 and b < 1000;
                    DualBag {{ total: a * 10 + b }}
                }} else {{
                    DualBag {{ total: 0 }}
                }}
            }}
        }}

        struct EmptyBag {{ empty: bool }}
        impl Copy for EmptyBag {{}}
        impl VariadicCollector for EmptyBag {{
            assoc Tmp: for<S: Type> Type = |_s: Type| -> Type {{ void }};
            fn variadic_init() -> void where _S {{}}
            fn variadic_collect(_tmp: *void, _args: S) -> EmptyBag where S {{
                inline if S == Tuple<> {{
                    EmptyBag {{ empty: true }}
                }} else {{
                    EmptyBag {{ empty: false }}
                }}
            }}
        }}

        fn take_rt(...xs: DualBag) -> u32 {{ xs.total }}
        comptime fn take_ct(...xs: DualBag) -> u32 {{ xs.total }}
        fn take_staged(comptime ...xs: DualBag) -> u32 {{ xs.total }}
        fn take_empty(...xs: EmptyBag) -> bool {{ xs.empty }}

        const CT_A: u32 = 3;
        const CT_B: u32 = 4;
        const CT_SUM: u32 = take_ct(CT_A, CT_B);
        comptime fn check_ct() -> bool {{ CT_SUM == 34 }}

        fn run() -> bool {{
            let a: u32 = 3;
            let b: u32 = 4;
            let rt = take_rt(a, b);
            let st = take_staged(CT_A, CT_B);
            rt == 7 and st == 34 and take_empty()
        }}",
        collector_header()
    );
    let package =
        compile_with_source_bindings(&source, &Default::default(), collector_selectors()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "check_ct", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
            .unwrap(),
        Value::Bool(true)
    );

    let rt_only_in_ct = format!(
        "{}
        struct RtOnly {{ v: u32 }}
        impl VariadicCollector for RtOnly {{
            assoc Tmp: for<S: Type> Type = |_s: Type| -> Type {{ void }};
            fn variadic_init() -> void where _S {{}}
            fn variadic_collect(_tmp: *void, _args: _S) -> RtOnly where _S {{ RtOnly {{ v: 1 }} }}
        }}
        comptime fn bad(...xs: RtOnly) -> u32 {{ xs.v }}
        const BAD: u32 = bad(1);
        fn run() -> u32 {{ BAD }}",
        collector_header()
    );
    let err =
        compile_with_source_bindings(&rt_only_in_ct, &Default::default(), collector_selectors())
            .unwrap_err();
    assert!(err.contains("lacks comptime trait phase"), "{err}");

    let ct_only_in_rt = format!(
        "{}
        struct CtOnly {{ v: u32 }}
        comptime impl VariadicCollector for CtOnly {{
            comptime fn comptime_variadic_collect(_args: _S) -> CtOnly where _S {{ CtOnly {{ v: 1 }} }}
        }}
        fn bad(...xs: CtOnly) -> u32 {{ xs.v }}
        fn run() -> u32 {{ bad(1) }}",
        collector_header()
    );
    let err =
        compile_with_source_bindings(&ct_only_in_rt, &Default::default(), collector_selectors())
            .unwrap_err();
    assert!(err.contains("lacks runtime trait phase"), "{err}");

    let non_collector = format!(
        "{}
        struct Plain {{ v: u32 }}
        fn bad(...xs: Plain) -> u32 {{ xs.v }}
        fn run() -> u32 {{ bad(1) }}",
        collector_header()
    );
    let err =
        compile_with_source_bindings(&non_collector, &Default::default(), collector_selectors())
            .unwrap_err();
    assert!(err.contains("VariadicCollector"), "{err}");
}

#[test]
fn extended_variadic_collector_supports_named_properties_tmp_drop_and_comptime_phase() {
    let source = format!(
        "{}
        struct PropBuf {{ port: u16, sentinel: u16 }}
        impl Drop for PropBuf {{
            fn drop(*self) {{
                let _ = self.sentinel.*;
            }}
        }}

        struct PropConfigRt {{ port: u16, enabled: bool, port_ptr: *u16 }}
        impl Copy for PropConfigRt {{}}

        comptime fn prop_shape(...fields: T) -> T where T {{ fields }}
        const ZERO_U16: u16 = 0;
        const PROP_SAMPLE = prop_shape {{ port: ZERO_U16, enabled: false }};
        typealias PropShape = PROP_SAMPLE'type;

        impl ExtendedVariadicCollector for PropConfigRt {{
            assoc Tmp: for<Ps: Type> Type = |_ps: Type| -> Type {{ PropBuf }};

            fn extended_variadic_init() -> PropBuf where _Ps {{
                PropBuf {{ port: 0, sentinel: 77 }}
            }}

            fn extended_variadic_collect(tmp: *PropBuf, properties: Ps) -> PropConfigRt where Ps {{
                inline if Ps == PropShape {{
                    let typed_props: PropShape = properties;
                    tmp.port.* = typed_props.port;
                    PropConfigRt {{
                        port: tmp.port.*,
                        enabled: typed_props.enabled,
                        port_ptr: tmp.*.port.ref,
                    }}
                }} else {{
                    PropConfigRt {{ port: 0, enabled: false, port_ptr: tmp.*.port.ref }}
                }}
            }}
        }}

        struct PropConfigCt {{ port: u16, enabled: bool }}
        impl Copy for PropConfigCt {{}}

        comptime impl ExtendedVariadicCollector for PropConfigCt {{
            comptime fn comptime_extended_variadic_collect(properties: Ps) -> PropConfigCt where Ps {{
                inline if Ps == PropShape {{
                    let typed_props: PropShape = properties;
                    assumes typed_props.port < 65535;
                    PropConfigCt {{ port: typed_props.port + 1, enabled: typed_props.enabled }}
                }} else {{
                    PropConfigCt {{ port: 0, enabled: false }}
                }}
            }}
        }}

        fn configure_rt(...props: PropConfigRt) -> bool {{
            props.port_ptr.* == 8080 and props.port == 8080 and props.enabled
        }}

        comptime fn configure_ct(...props: PropConfigCt) -> bool {{
            props.port == 8081 and props.enabled
        }}

        const CT_PORT: u16 = 8080;
        const CT_OK: bool = configure_ct {{ port: CT_PORT, enabled: true }};
        comptime fn check_ct() -> bool {{ CT_OK }}

        fn run() -> bool {{
            let port: u16 = 8080;
            configure_rt {{ port: port, enabled: true }}
        }}",
        collector_header()
    );
    let package =
        compile_with_source_bindings(&source, &Default::default(), collector_selectors()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "check_ct", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
            .unwrap(),
        Value::Bool(true)
    );

    let bad = format!(
        "{}
        struct NotProps {{ port: u16 }}
        fn take(...props: NotProps) -> u16 {{ props.port }}
        fn run() -> u16 {{ take {{ port: 80 }} }}",
        collector_header()
    );
    let err =
        compile_with_source_bindings(&bad, &Default::default(), collector_selectors()).unwrap_err();
    assert!(err.contains("ExtendedVariadicCollector"), "{err}");
}

#[test]
fn generic_variadic_collector_constraints_work_via_where_clause() {
    let source = format!(
        "{}
        struct BoxedList {{ len: u32 }}
        impl Copy for BoxedList {{}}
        impl VariadicCollector for BoxedList {{
            assoc Tmp: for<S: Type> Type = |_s: Type| -> Type {{ void }};
            fn variadic_init() -> void where _S {{}}
            fn variadic_collect(_tmp: *void, args: S) -> BoxedList where S {{
                inline if S == Tuple<u32, u32> {{
                    let _typed: Tuple<u32, u32> = args;
                    BoxedList {{ len: 2 }}
                }} else {{
                    BoxedList {{ len: 0 }}
                }}
            }}
        }}
        fn check_generic(xs: C) -> bool where C, requires C :- VariadicCollector {{
            true
        }}
        fn collect_generic(...xs: C) -> C where C, requires C :- VariadicCollector {{
            xs
        }}
        fn run() -> bool {{
            let a: u32 = 10;
            let b: u32 = 20;
            let collected = collect_generic<BoxedList>(a, b);
            collected.len == 2 and check_generic(collected)
        }}",
        collector_header()
    );
    let package =
        compile_with_source_bindings(&source, &Default::default(), collector_selectors()).unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn sysroot_prelude_exports_variadic_collectors_and_runs_with_default_bindings() {
    let source = r#"
        struct SysCollector {
            count: u32,
        }

        impl Clone for SysCollector {
            fn clone(*self) -> Self {
                SysCollector { count: self.count.* }
            }
        }

        impl Copy for SysCollector {}

        impl VariadicCollector for SysCollector {
            assoc Tmp: for<S: Type> Type = |_s: Type| -> Type { void };

            fn variadic_init() -> void where _S {}

            fn variadic_collect(_tmp: *void, args: S) -> SysCollector where S {
                var n: u32 = 0;
                inline if Type.predicates.is_tuple(S) {
                    inline for (_i, _item) in args'tuple_indexed {
                        assumes n < 1000;
                        n = n + 1;
                    }
                }
                SysCollector { count: n }
            }
        }

        fn count_args(...xs: SysCollector) -> u32 {
            xs.count
        }

        fn main() -> u32 {
            let a: u32 = 1;
            let b: u32 = 2;
            let c: u32 = 3;
            count_args(a, b, c)
        }
    "#;
    let package = sysroot_support::compile(source).unwrap();
    let result =
        luna::execution::interpret_with_protocols(&package, "main", "Copy", "Drop", 1_000_000);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(result.unwrap(), Value::UnsignedInteger(3));
}
