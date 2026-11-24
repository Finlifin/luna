//! End-to-end 128-bit ABI `Type.type_id` Merkle fingerprint tests against `--sysroot library`.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use luna::cli::{self, Command, Emit};
use luna::frontend::{self, Artifact};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

struct TempWorkspace(PathBuf);

impl TempWorkspace {
    fn new(prefix: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "luna-type-id-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, rel: &str, content: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn library_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library")
}

fn compile_with_version_and_target(
    source: &str,
    package_version: &str,
    pointer_bits: u32,
) -> Result<nir::program::Package, String> {
    let ws = TempWorkspace::new("single");
    let main_path = ws.write("main.fl", source);
    let Command::Check(mut options) = cli::parse([
        "check".into(),
        main_path.into_os_string(),
        "--sysroot".into(),
        library_dir().into_os_string(),
        "--build-target".into(),
        if pointer_bits == 32 { "i686" } else { "x86_64" }.into(),
        "linux".into(),
        pointer_bits.to_string().into(),
    ])
    .unwrap() else {
        panic!("expected check command");
    };
    options.emit = Emit::Nir;

    let pkg_id = 42u64;
    let mut protocols = elaboration::program::Protocols::default();
    protocols
        .package_versions
        .insert(pkg_id, package_version.to_string());
    let target = nir::target::TargetData::new(pointer_bits).unwrap();
    let (artifact, _) = frontend::compile_bound_for_target(
        &options,
        pkg_id,
        &protocols,
        Default::default(),
        Some(target),
    )?;
    let Artifact::Nir(nir) = artifact else {
        panic!("expected NIR artifact");
    };
    Ok(nir)
}

fn eval_u128(package: &nir::program::Package, entry: &str) -> u128 {
    let graph = luna::instantiation::instantiate(package, entry, 1_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    match vm.invoke_instance(&graph.roots[0], vec![]).unwrap() {
        Value::UnsignedInteger(v) => v,
        other => panic!("expected UnsignedInteger, got {other:?}"),
    }
}

fn compile_sysroot_multi(
    extra_packages: &[(&str, u64, &str, &Path)],
    primary_entry: &Path,
    primary_identity: u64,
    primary_version: &str,
    lib_versions: &[(u64, &str)],
    pointer_bits: u32,
) -> Result<nir::program::Package, String> {
    let lib = library_dir();
    let Command::Check(mut options) = cli::parse([
        "check".into(),
        primary_entry.to_path_buf().into_os_string(),
        "--sysroot".into(),
        lib.clone().into_os_string(),
        "--build-target".into(),
        if pointer_bits == 32 { "i686" } else { "x86_64" }.into(),
        "linux".into(),
        pointer_bits.to_string().into(),
    ])
    .unwrap() else {
        panic!("expected check command");
    };
    options.emit = Emit::Nir;

    let mut protocols = elaboration::program::Protocols::default();
    protocols
        .package_versions
        .insert(primary_identity, primary_version.into());
    for &(id, ver) in lib_versions {
        protocols.package_versions.insert(id, ver.into());
    }
    let extra: Vec<(&str, u64, &Path)> = extra_packages
        .iter()
        .map(|&(name, id, _, path)| (name, id, path))
        .collect();
    let target = nir::target::TargetData::new(pointer_bits).unwrap();
    let (Artifact::Nir(nir), _) = frontend::compile_sysroot_with_extra_packages(
        &options,
        primary_identity,
        &lib,
        &extra,
        &protocols,
        Default::default(),
        Some(target),
    )?
    else {
        panic!("expected NIR");
    };
    Ok(nir)
}

#[test]
fn meta_type_id_returns_nonzero_and_matches_across_comptime_modes() {
    let source = r#"
        struct Pair { first: u32, second: u64 }

        const METHOD_ID: u128 = Pair.type_id();
        const FREE_ID: u128 = meta.type_id(Pair);
        const U32_ID: u128 = u32.type_id();

        comptime fn compute_in_tree() -> bool {
            let a = Pair.type_id();
            let b = meta.type_id(Pair);
            a != 0 and a == b and a == METHOD_ID and a != U32_ID
        }

        const RESULT: bool = METHOD_ID != 0 and METHOD_ID == FREE_ID and METHOD_ID != U32_ID and compute_in_tree();
        fn run() -> bool { RESULT }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));

    // Verify comptime VM execution via frontend::invoke_comptime matches elaboration `const`.
    assert_eq!(
        support::invoke(source, "compute_in_tree").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn cross_version_stable_since_preserves_type_id_even_with_shifted_decls() {
    let ws1 = TempWorkspace::new("v1");
    let lib1 = ws1.write(
        "collections.fl",
        r#"
        ^attrs.stable_since("1.0.0")
        pub struct StableBox where T {
            len: usize,
            data: *T,
        }

        pub struct UnanchoredBox where T {
            len: usize,
            data: *T,
        }
        "#,
    );
    let app1 = ws1.write(
        "main.fl",
        r#"
        use collections.{StableBox, UnanchoredBox};
        const STABLE_ID: u128 = StableBox<u32>.type_id();
        const UNANCHORED_ID: u128 = UnanchoredBox<u32>.type_id();
        const STABLE_I32_ID: u128 = StableBox<i32>.type_id();
        fn stable_id() -> u128 { STABLE_ID }
        fn unanchored_id() -> u128 { UNANCHORED_ID }
        fn stable_i32_id() -> u128 { STABLE_I32_ID }
        "#,
    );

    let ws2 = TempWorkspace::new("v2");
    let lib2 = ws2.write(
        "collections.fl",
        r#"
        -- Added unrelated helpers/types above to shift declaration indices!
        pub fn helper_v12() -> u32 { 7 }
        pub struct ExtraPrecursor { tag: u8, count: u64 }

        ^attrs.stable_since("1.0.0")
        pub struct StableBox where T {
            len: usize,
            data: *T,
        }

        pub struct UnanchoredBox where T {
            len: usize,
            data: *T,
        }
        "#,
    );
    let app2 = ws2.write(
        "main.fl",
        r#"
        use collections.{StableBox, UnanchoredBox};
        const STABLE_ID: u128 = StableBox<u32>.type_id();
        const UNANCHORED_ID: u128 = UnanchoredBox<u32>.type_id();
        const STABLE_I32_ID: u128 = StableBox<i32>.type_id();
        fn stable_id() -> u128 { STABLE_ID }
        fn unanchored_id() -> u128 { UNANCHORED_ID }
        fn stable_i32_id() -> u128 { STABLE_I32_ID }
        "#,
    );

    let nir1 = compile_sysroot_multi(
        &[("collections", 77, "1.0.0", &lib1)],
        &app1,
        10,
        "1.0.0",
        &[(77, "1.0.0")],
        64,
    )
    .unwrap();
    let nir2 = compile_sysroot_multi(
        &[("collections", 77, "1.2.0", &lib2)],
        &app2,
        10,
        "1.2.0",
        &[(77, "1.2.0")],
        64,
    )
    .unwrap();

    let s1 = eval_u128(&nir1, "stable_id");
    let s2 = eval_u128(&nir2, "stable_id");
    let s1_i32 = eval_u128(&nir1, "stable_i32_id");
    let u1 = eval_u128(&nir1, "unanchored_id");
    let u2 = eval_u128(&nir2, "unanchored_id");

    assert_ne!(s1, 0);
    assert_eq!(
        s1, s2,
        "anchored StableBox<u32> must match across 1.0.0 vs 1.2.0"
    );
    assert_ne!(s1, s1_i32, "distinct generic arg must differ");
    assert_ne!(
        u1, u2,
        "unanchored type without stable_since must change across versions"
    );
}

#[test]
fn four_part_safety_contract_breaks_type_id_on_abi_or_dependency_changes() {
    let base = r#"
        ^attrs.stable_since("1.0.0")
        struct Inner { x: u32 }

        ^attrs.stable_since("1.0.0")
        struct Record { a: u32, b: u64, inner: Inner }

        ^attrs.stable_since("1.0.0")
        enum Choice { empty, some(u32) }

        fn record_id() -> u128 { const V: u128 = Record.type_id(); V }
        fn choice_id() -> u128 { const V: u128 = Choice.type_id(); V }
    "#;
    let added_field = r#"
        ^attrs.stable_since("1.0.0")
        struct Inner { x: u32 }

        ^attrs.stable_since("1.0.0")
        struct Record { a: u32, b: u64, inner: Inner, c: u8 }

        ^attrs.stable_since("1.0.0")
        enum Choice { empty, some(u32) }

        fn record_id() -> u128 { const V: u128 = Record.type_id(); V }
        fn choice_id() -> u128 { const V: u128 = Choice.type_id(); V }
    "#;
    let reordered_fields = r#"
        ^attrs.stable_since("1.0.0")
        struct Inner { x: u32 }

        ^attrs.stable_since("1.0.0")
        struct Record { b: u64, a: u32, inner: Inner }

        ^attrs.stable_since("1.0.0")
        enum Choice { empty, some(u32) }

        fn record_id() -> u128 { const V: u128 = Record.type_id(); V }
        fn choice_id() -> u128 { const V: u128 = Choice.type_id(); V }
    "#;
    let packed_repr = r#"
        ^attrs.stable_since("1.0.0")
        struct Inner { x: u32 }

        ^attrs.stable_since("1.0.0")
        ^attrs.packed.byte
        struct Record { a: u32, b: u64, inner: Inner }

        ^attrs.stable_since("1.0.0")
        enum Choice { empty, other(u32) }

        fn record_id() -> u128 { const V: u128 = Record.type_id(); V }
        fn choice_id() -> u128 { const V: u128 = Choice.type_id(); V }
    "#;
    let changed_nested_dep = r#"
        ^attrs.stable_since("1.0.0")
        struct Inner { x: u64 }

        ^attrs.stable_since("1.0.0")
        struct Record { a: u32, b: u64, inner: Inner }

        ^attrs.stable_since("1.0.0")
        enum Choice { empty, some(u64) }

        fn record_id() -> u128 { const V: u128 = Record.type_id(); V }
        fn choice_id() -> u128 { const V: u128 = Choice.type_id(); V }
    "#;

    let nir_base = compile_with_version_and_target(base, "1.0.0", 64).unwrap();
    let nir_add = compile_with_version_and_target(added_field, "1.0.0", 64).unwrap();
    let nir_reord = compile_with_version_and_target(reordered_fields, "1.0.0", 64).unwrap();
    let nir_packed = compile_with_version_and_target(packed_repr, "1.0.0", 64).unwrap();
    let nir_dep = compile_with_version_and_target(changed_nested_dep, "1.0.0", 64).unwrap();

    let id_base = eval_u128(&nir_base, "record_id");
    assert_ne!(id_base, eval_u128(&nir_add, "record_id"));
    assert_ne!(id_base, eval_u128(&nir_reord, "record_id"));
    assert_ne!(id_base, eval_u128(&nir_packed, "record_id"));
    assert_ne!(id_base, eval_u128(&nir_dep, "record_id"));

    let ch_base = eval_u128(&nir_base, "choice_id");
    assert_ne!(ch_base, eval_u128(&nir_packed, "choice_id"));
    assert_ne!(ch_base, eval_u128(&nir_dep, "choice_id"));
}

#[test]
fn typealias_is_transparent_and_newtype_is_distinct() {
    let source = r#"
        struct Pair { a: u32, b: u64 }
        typealias PairAlias = Pair;
        newtype WrappedPair = Pair;
        newtype OtherWrapped = Pair;

        const A: u128 = Pair.type_id();
        const B: u128 = PairAlias.type_id();
        const C: u128 = WrappedPair.type_id();
        const D: u128 = OtherWrapped.type_id();

        const OK: bool = A != 0 and A == B and C != A and C != D;
        fn run() -> bool { OK }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn recursive_pointer_struct_terminates_deterministically() {
    let source = r#"
        struct Node {
            value: u32,
            next: ?*Node,
        }
        struct Node2 {
            value: u32,
            next: ?*Node2,
        }
        comptime fn check() -> bool {
            let a = Node.type_id();
            let b = Node.type_id();
            let c = Node2.type_id();
            a != 0 and a == b and a != c
        }
        const OK: bool = check();
        fn run() -> bool { OK }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn normal_form_constructors_propagate_stable_since_to_instances() {
    let ws1 = TempWorkspace::new("nf_v1");
    let mod1 = ws1.write(
        "nf.fl",
        r#"
        ^attrs.stable_since("1.0.0")
        pub struct Boxed where T { value: T }

        ^attrs.stable_since("1.0.0")
        pub enum OptLike where T { none, some(T) }

        ^attrs.stable_since("1.0.0")
        pub newtype Id<T> = T;

        ^attrs.stable_since("1.0.0")
        pub pure comptime fn Tagged<T> -> Type { T }
        "#,
    );
    let app1 = ws1.write(
        "main.fl",
        r#"
        use nf.{Boxed, OptLike, Id, Tagged};
        const ATTR_OK: bool = Tagged<u32>'attribute.get<str>(.__flurry_stable_since) matches "1.0.0"?;
        const BOX_ID: u128 = Boxed<u32>.type_id();
        const ENUM_ID: u128 = OptLike<u32>.type_id();
        const NEWTYPE_ID: u128 = Id<u32>.type_id();
        fn attr_ok() -> bool { ATTR_OK }
        fn box_id() -> u128 { BOX_ID }
        fn enum_id() -> u128 { ENUM_ID }
        fn newtype_id() -> u128 { NEWTYPE_ID }
        "#,
    );
    let nir1 = compile_sysroot_multi(
        &[("nf", 88, "1.0.0", &mod1)],
        &app1,
        10,
        "1.0.0",
        &[(88, "1.0.0")],
        64,
    )
    .unwrap();
    let nir2 = compile_sysroot_multi(
        &[("nf", 88, "1.5.0", &mod1)],
        &app1,
        10,
        "1.5.0",
        &[(88, "1.5.0")],
        64,
    )
    .unwrap();

    assert_eq!(eval_u128(&nir1, "box_id"), eval_u128(&nir2, "box_id"));
    assert_eq!(eval_u128(&nir1, "enum_id"), eval_u128(&nir2, "enum_id"));
    assert_eq!(
        eval_u128(&nir1, "newtype_id"),
        eval_u128(&nir2, "newtype_id")
    );
    let graph = luna::instantiation::instantiate(&nir1, "attr_ok", 1_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );

    // Opaque non-Tuple normal form has no target layout and must be rejected cleanly.
    let opaque_err = support::compile(
        r#"
        ^attrs.stable_since("1.0.0")
        pure comptime fn Tagged<T> -> Type { T }
        const BAD: u128 = Tagged<u32>.type_id();
        "#,
    )
    .unwrap_err();
    assert!(!opaque_err.is_empty());
}

#[test]
fn target_layout_sensitivity_and_error_optional_tuple_types() {
    let source = r#"
        struct PtrPair { count: usize, ptr: *u8 }
        enum IoErr { NotFound, PermissionDenied }
        typealias ErrTy = !IoErr u32;
        typealias OptTy = ?*u8;
        typealias TupTy = Tuple<u32, u64>;
        const P_ID: u128 = PtrPair.type_id();
        const ERR_ID: u128 = ErrTy.type_id();
        const OPT_ID: u128 = OptTy.type_id();
        const TUP_ID: u128 = TupTy.type_id();
        fn p_id() -> u128 { P_ID }
        fn err_id() -> u128 { ERR_ID }
        fn opt_id() -> u128 { OPT_ID }
        fn tup_id() -> u128 { TUP_ID }
    "#;
    let nir64 = compile_with_version_and_target(source, "1.0.0", 64).unwrap();
    let nir32 = compile_with_version_and_target(source, "1.0.0", 32).unwrap();

    let p64 = eval_u128(&nir64, "p_id");
    let p32 = eval_u128(&nir32, "p_id");
    assert_ne!(p64, 0);
    assert_ne!(p32, 0);
    assert_ne!(
        p64, p32,
        "32-bit and 64-bit targets must produce distinct ABI type_ids"
    );

    let e64 = eval_u128(&nir64, "err_id");
    let o64 = eval_u128(&nir64, "opt_id");
    let t64 = eval_u128(&nir64, "tup_id");
    assert_ne!(e64, 0);
    assert_ne!(o64, 0);
    assert_ne!(t64, 0);
    assert_ne!(e64, o64);
    assert_ne!(o64, t64);
}

#[test]
fn rejects_open_generics_comptime_only_and_local_runtime_origins() {
    for bad in [
        r#"const BAD: u128 = Type.type_id();"#,
        r#"const BAD: u128 = Object.type_id();"#,
        r#"
            pure comptime fn id_of() -> u128 where T {
                T.type_id()
            }
            const BAD: u128 = id_of<u32>();
        "#,
        r#"
            struct LocalHolder where quote ptr: lift *u32 { val: u32 }
            fn bad() -> u128 {
                var x: u32 = 1;
                LocalHolder<x.ref>.type_id()
            }
        "#,
    ] {
        let err = support::compile(bad).unwrap_err();
        assert!(
            !err.contains("error[E2") && !err.contains("ComptimeBlock"),
            "unexpected syntax/unimplemented error in negative test:\n{err}"
        );
    }

    // Global lifted constant runtime origin MUST succeed inside a runtime body
    // and distinguish distinct global origins, whereas local `var` fails.
    let global_src = r#"
        const A: u32 = 1;
        const B: u32 = 2;
        struct GlobalHolder where quote p: lift *u32 { dummy: u8 }
        fn run() -> bool {
            let id_a = GlobalHolder<A.ref>.type_id();
            let id_b = GlobalHolder<B.ref>.type_id();
            id_a != 0 and id_b != 0 and id_a != id_b
        }
    "#;
    assert_eq!(support::run(global_src), Value::Bool(true));
}
