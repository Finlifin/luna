//! End-to-end Stage E1 tests against the real built-in packages (`--sysroot library`):
//! - default derivation selection, visibility, parametric matching, and orphan coherence
//! - packed struct layout consumer (`attrs.packed.byte` / `__flurry_packed_mode`), LLVM `<{...}>` / align 1,
//!   native execution at O0 and O3 (including `Vec<Packed>`), and aligned field pointer rejection
//! - `attrs.effects.direct_passing` ABI lowering, aliases/overrides, and invalid target checks
//! - `attrs.metadata`, `target'attribute`, `Type.is_packed`, cycle rejection, and foreign ABI conventions
//! - `attrs.stable_since` SemVer/target checks and `attrs.macros.*` target/combination checks
//! - `meta.Object.{is_empty, has_key, keys, merge}` and `Symbol.{byte_length, byte}`
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

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
            "luna-stage-e1-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn library_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
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
            .arg(Self::library_dir())
            .output()
            .unwrap()
    }

    fn emit_llvm(&self) -> Result<String, String> {
        let out_file = self.0.join("out.ll");
        let output = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "main.fl",
                "--emit-llvm",
                "run",
                "--output",
                "out.ll",
                "--fuel",
                "10000000",
                "--sysroot",
            ])
            .arg(Self::library_dir())
            .output()
            .unwrap();
        if !output.status.success() {
            return Err(format!(
                "stdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        fs::read_to_string(out_file).map_err(|e| e.to_string())
    }

    fn run_binary(&self) -> Output {
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
fn default_derivation_selection_visibility_parametric_and_coherence() {
    let source = r#"
        const default_marker = { __flurry_derivation_default: true }

        trait Flag {
            fn value() -> u64;
        }

        ^default_marker
        derive fn flag_default for Flag {
            fn value() -> u64 {
                41
            }
        }

        struct Subject {}
        derive Flag for Subject;

        trait FormatLabel where format: Symbol {
            fn label() -> u64;
        }

        ^default_marker
        derive fn label_text for FormatLabel<.text> {
            fn label() -> u64 {
                10
            }
        }

        ^default_marker
        derive fn label_json for FormatLabel<.json> {
            fn label() -> u64 {
                20
            }
        }

        struct Tagged {}
        derive FormatLabel<.text> for Tagged;
        derive FormatLabel<.json> for Tagged;

        fn check_all() -> bool {
            Subject.as(Flag).value() == 41
                and Tagged.as(FormatLabel<.text>).label() == 10
                and Tagged.as(FormatLabel<.json>).label() == 20
        }
    "#;
    let package =
        support::compile(&format!("{source}\nfn run() -> bool {{ check_all() }}")).unwrap();
    let derived_names: Vec<_> = package
        .functions
        .iter()
        .map(|f| f.interface.name.as_str())
        .filter(|n| *n == "check_all")
        .collect();
    assert_eq!(derived_names.len(), 1);
    let subject_id = package
        .structs
        .iter()
        .find(|s| s.name == "Subject")
        .unwrap()
        .declaration;
    let tagged_id = package
        .structs
        .iter()
        .find(|s| s.name == "Tagged")
        .unwrap()
        .declaration;
    let derived_count = package
        .implementations
        .iter()
        .filter(|i| {
            matches!(
                &i.subject,
                nir::Term::Nominal { declaration, .. }
                    if *declaration == subject_id || *declaration == tagged_id
            )
        })
        .count();
    assert_eq!(derived_count, 3);

    let case = NativeCase::new(&format!("{source}\nfn run() -> bool {{ check_all() }}"));
    assert_success(&case.build("0"));
    let out = case.run_binary();
    assert_success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");

    let unmatched_parametric = r#"
        const default_marker = { __flurry_derivation_default: true }
        trait FormatLabel where format: Symbol {
            fn label() -> u64;
        }
        ^default_marker
        derive fn label_text for FormatLabel<.text> {
            fn label() -> u64 { 10 }
        }
        struct Tagged {}
        derive FormatLabel<.xml> for Tagged;
    "#;
    let err = support::compile(unmatched_parametric).unwrap_err();
    assert!(
        err.contains("no applicable default derivation strategy found"),
        "{err}"
    );

    let disabled = r#"
        const default_marker = { __flurry_derivation_default: true }
        trait Flag {
            fn value() -> u64;
        }
        ^{ __flurry_derivation_default: false }
        ^default_marker
        derive fn flag_default for Flag {
            fn value() -> u64 { 1 }
        }
        struct Subject {}
        derive Flag for Subject;
    "#;
    let err = support::compile(disabled).unwrap_err();
    assert!(
        err.contains("no applicable default derivation strategy found"),
        "{err}"
    );

    for hidden in [
        r#"
            mod service {
                private mod hidden {
                    use ..Flag;
                    const secret = { __flurry_derivation_default: true }
                    ^secret
                    derive fn hidden_default for Flag {
                        fn value() -> u64 { 2 }
                    }
                }
            }
            trait Flag {
                fn value() -> u64;
            }
            struct Subject {}
            derive Flag for Subject;
        "#,
        r#"
            mod inner {
                use .Flag;
                const secret = { __flurry_derivation_default: true }
                ^secret
                private derive fn hidden_default for Flag {
                    fn value() -> u64 { 2 }
                }
            }
            trait Flag {
                fn value() -> u64;
            }
            struct Subject {}
            derive Flag for Subject;
        "#,
    ] {
        let err = support::compile(hidden).unwrap_err();
        assert!(
            err.contains("no applicable default derivation strategy found"),
            "{hidden}: {err}"
        );
    }

    let ambiguous = r#"
        const default_marker = { __flurry_derivation_default: true }
        trait Flag {
            fn value() -> u64;
        }
        ^default_marker
        derive fn first for Flag {
            fn value() -> u64 { 1 }
        }
        ^default_marker
        derive fn second for Flag {
            fn value() -> u64 { 2 }
        }
        struct Subject {}
        derive Flag for Subject;
    "#;
    let err = support::compile(ambiguous).unwrap_err();
    assert!(
        err.contains("ambiguous default derivation strategies"),
        "{err}"
    );

    // Cross-package orphan default strategy must fail coherence check.
    let dir = std::env::temp_dir().join(format!(
        "luna-default-orphan-{}-{}",
        std::process::id(),
        file!().len()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("base.fl"), "pub trait ForeignFlag {}\n").unwrap();
    fs::write(
        dir.join("app.fl"),
        r#"
            use base.ForeignFlag;
            const marker = { __flurry_derivation_default: true }
            ^marker
            derive fn bad_orphan for ForeignFlag {}
            struct Subject {}
            derive ForeignFlag for Subject;
        "#,
    )
    .unwrap();
    let out = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
        .current_dir(&dir)
        .args([
            "--packages-v1",
            "--package",
            "1",
            "101",
            "app",
            "app.fl",
            "--package",
            "2",
            "202",
            "base",
            "base.fl",
            "--dependency",
            "1",
            "base",
            "2",
            "--primary",
            "1",
            "--emit",
            "nir",
        ])
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&dir);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("default derivation strategy requires a local trait or trait argument"),
        "{stderr}"
    );
}

#[test]
fn packed_structs_layout_llvm_and_native_execution_o0_and_o3() {
    let source = r#"
        use std.collections.Vec;

        ^attrs.packed.byte
        struct PackedPair {
            a: u8,
            b: u64,
        }

        impl Clone for PackedPair {
            fn clone(*self) -> Self {
                PackedPair { a: self.a, b: self.b }
            }
        }

        impl Copy for PackedPair {}

        struct PlainPair {
            a: u8,
            b: u64,
        }

        ^attrs.packed.byte
        ^attrs.packed.bit
        struct ByteOverBit {
            tag: u8,
            wide: u64,
        }

        ^attrs.packed.byte
        struct NestedPacked {
            lead: u8,
            inner: PackedPair,
        }

        fn check_packed() -> bool {
            var x = PackedPair { a: 7, b: 0x1122334455667788 };
            x.b = x.b + 1;
            let p: *PackedPair = x.ref;
            let direct_read: u64 = p.b;
            let deref_read = p.b.*;
            p.b.* = deref_read + 2;
            p.*.b = p.b.* + 3;
            let c = ByteOverBit { tag: 5, wide: 99 };

            var nested = NestedPacked {
                lead: 3,
                inner: PackedPair { a: 9, b: 1000 },
            };
            let pn = nested.ref;
            pn.inner.b.* = pn.inner.b.* + 77;
            let nested_ok = nested.lead == 3 and pn.inner.b.* == 1077;

            var items = Vec<PackedPair>.new();
            let push1 = items.ref.push(PackedPair { a: 1, b: 1234567890123 }) match {
                _! => true,
                error _ => false,
            };
            let push2 = items.ref.push(PackedPair { a: 2, b: 9876543210987 }) match {
                _! => true,
                error _ => false,
            };
            let vec_ok = items.ref.len() == 2
                and (items.ref.get(0) match {
                    item? => item.a == 1 and item.b == 1234567890123,
                    null => false,
                })
                and (items.ref.get(1) match {
                    item? => item.a == 2 and item.b == 9876543210987,
                    null => false,
                })
                and (items.ref.pop() match {
                    item? => item.a == 2 and item.b == 9876543210987,
                    null => false,
                })
                and items.ref.len() == 1;

            let layout_ok = Type.size_of(PackedPair) == 9
                and Type.align_of(PackedPair) == 1
                and Type.size_of(ByteOverBit) == 9
                and Type.align_of(ByteOverBit) == 1
                and Type.size_of(PlainPair) == 16
                and Type.align_of(PlainPair) == 8
                and Type.size_of(NestedPacked) == 10
                and Type.align_of(NestedPacked) == 1;

            layout_ok
                and x.a == 7
                and direct_read == 0x1122334455667789
                and deref_read == 0x1122334455667789
                and x.b == 0x112233445566778E
                and c.tag == 5
                and c.wide == 99
                and nested_ok
                and push1
                and push2
                and vec_ok
        }

        fn run() -> bool {
            check_packed()
        }
    "#;

    let case = NativeCase::new(source);
    let llvm = case.emit_llvm().unwrap();
    assert!(llvm.contains("<{ i8, i64 }>"), "{llvm}");
    assert!(llvm.contains("alloca <{ i8, i64 }>, align 1"), "{llvm}");
    assert!(llvm.contains("align 1"), "{llvm}");

    for opt in ["0", "3"] {
        assert_success(&case.build(opt));
        let out = case.run_binary();
        assert_success(&out);
        assert_eq!(out.stdout, b"(bool true)\n");
    }
}

#[test]
fn packed_structs_reject_aligned_field_pointers_and_wrong_targets() {
    let header = r#"
        ^attrs.packed.byte
        struct PackedPair {
            a: u8,
            b: u64,
        }

        impl Clone for PackedPair {
            fn clone(*self) -> Self {
                PackedPair { a: self.a, b: self.b }
            }
        }

        impl Copy for PackedPair {}
    "#;
    for bad in [
        "fn bad() { var x = PackedPair { a: 1, b: 2 }; let q = x.b.ref; }",
        "fn bad() { var x = PackedPair { a: 1, b: 2 }; let p = x.ref; let q = p.b.*.ref; }",
        "fn bad() { var x = PackedPair { a: 1, b: 2 }; let p = x.ref; let q = p.b; }",
        "fn bad(p: *PackedPair) -> *u64 { p.b }",
    ] {
        let err =
            support::compile(&format!("{header}\n{bad}\nfn run() -> bool {{ true }}")).unwrap_err();
        assert!(
            err.contains("cannot take an aligned pointer to a field of a packed struct"),
            "{bad}: {err}"
        );
    }

    let wrong_target = r#"
        ^attrs.packed.byte
        fn bad() -> bool { true }
        fn run() -> bool { bad() }
    "#;
    let err = support::compile(wrong_target).unwrap_err();
    assert!(
        err.contains("__flurry_packed_mode requires a struct definition"),
        "{err}"
    );
    assert!(
        !err.starts_with("0..0:"),
        "diagnostic lost source span: {err}"
    );

    for legacy in [
        "^{ packed: true } struct LegacyBare { a: u8 }",
        "struct LegacyConfig { .packed true, a: u8 }",
    ] {
        let err = support::compile(legacy).unwrap_err();
        assert!(
            err.contains(
                "elaboration is not implemented for definition attribute or modifier `packed`"
            ),
            "{legacy}: {err}"
        );
        assert!(
            !err.starts_with("0..0:"),
            "diagnostic lost source span: {err}"
        );
    }
}

#[test]
fn direct_passing_effect_attribute_aliases_overrides_and_validation() {
    let valid = r#"
        use attrs.effects.direct_passing as dp;

        ^attrs.effects.direct_passing
        effect tick() -> u64;

        ^dp
        effect tock() -> u64;

        ^{ __flurry_direct_passing: false }
        ^attrs.effects.direct_passing
        effect overridden() -> u64;

        fn probe() -> #[tick, tock] u64 {
            let a = tick()#;
            let b = tock()#;
            assumes a <= 1000;
            assumes b <= 1000;
            a + b
        }

        fn probe_overridden() -> #overridden u64 {
            overridden()#
        }

        fn run() -> bool {
            let _ = probe_overridden;
            let total = do { probe() }# {
                tick() => 30,
                tock() => 12,
            };
            total == 42
        }
    "#;
    let nir = support::compile(valid).unwrap();
    let tick = nir
        .effects
        .iter()
        .find(|e| e.name == "tick")
        .expect("tick effect");
    let tock = nir
        .effects
        .iter()
        .find(|e| e.name == "tock")
        .expect("tock effect");
    let overridden = nir
        .effects
        .iter()
        .find(|e| e.name == "overridden")
        .expect("overridden effect");
    assert_eq!(tick.abi, nir::effects::Abi::DirectPassing);
    assert_eq!(tock.abi, nir::effects::Abi::DirectPassing);
    assert_eq!(overridden.abi, nir::effects::Abi::Direct);

    let case = NativeCase::new(valid);
    let llvm = case.emit_llvm().unwrap();
    assert!(
        llvm.contains("(i64 %effect0, i64 %effect1, ptr %task)")
            && llvm.contains("(ptr %effect0_entry, ptr %effect0_env, ptr %task)"),
        "{llvm}"
    );
    assert_success(&case.build("0"));
    let out = case.run_binary();
    assert_success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");

    for invalid in [
        "^attrs.effects.direct_passing async effect bad() -> u64; fn run() -> bool { true }",
        "^attrs.effects.direct_passing effect bad(x: u64) -> u64; fn run() -> bool { true }",
    ] {
        let err = support::compile(invalid).unwrap_err();
        assert!(
            err.contains("direct passing effects must be synchronous and have no parameters"),
            "{invalid}: {err}"
        );
    }

    let wrong_target = r#"
        use attrs.effects.direct_passing as dp;
        ^dp
        fn bad() {}
        fn run() -> bool { true }
    "#;
    let err = support::compile(wrong_target).unwrap_err();
    assert!(
        err.contains("direct_passing requires an effect declaration"),
        "{err}"
    );
}

#[test]
fn metadata_attribute_view_reflection_cycle_and_c_abi() {
    let source = r#"
        ^attrs.metadata({ version: 2, author: .luna })
        ^attrs.packed.byte
        struct Record {
            tag: u8,
            value: u64,
        }

        ^attrs.metadata({ role: .entry })
        fn annotated_fn() -> u64 {
            11
        }

        typealias RecordAlias = Record;
        struct Plain {}

        comptime fn run() -> bool {
            let record_attrs = Record'attribute;
            let direct_attrs = RecordAlias'attribute;
            let fn_attrs = annotated_fn'attribute;
            let plain_attrs = Plain'attribute;

            let packed_ok = Record.is_packed()
                and not Plain.is_packed()
                and (Record.packed_mode() matches .byte?)
                and (Plain.packed_mode() matches null);
            let struct_meta_ok = record_attrs.get<meta.Object>(.__flurry_metadata) match {
                m? => (m.get<Integer>(.version) matches 2?)
                    and (m.get<Symbol>(.author) matches .luna?),
                null => false,
            };
            let direct_ok = direct_attrs.get<Symbol>(.__flurry_packed_mode) matches .byte?;
            let fn_meta_ok = fn_attrs.get<meta.Object>(.__flurry_metadata) match {
                m? => m.get<Symbol>(.role) matches .entry?,
                null => false,
            };
            packed_ok and struct_meta_ok and direct_ok and fn_meta_ok and plain_attrs.is_empty()
        }
    "#;
    assert_eq!(support::invoke(source, "run").unwrap(), Value::Bool(true));

    let cycle = r#"
        ^{ tag: SelfReferential'attribute }
        struct SelfReferential {}

        comptime fn run() -> bool {
            SelfReferential'attribute.is_empty()
        }
    "#;
    assert!(
        support::compile(cycle).is_err(),
        "attribute reflection cycle must fail rather than return an empty object"
    );

    let abi_nir = r#"
        ^attrs.abi(.c)
        fn foreign_c() -> u64 { 1 }
        fn run() -> bool { true }
    "#;
    support::compile(abi_nir).unwrap();
}

#[test]
fn meta_object_and_symbol_helpers() {
    let source = r#"
        comptime fn run() -> bool {
            let empty: meta.Object = {};
            let b42: u8 = 42;
            let left: meta.Object = { a: 1, 'x', b: "old", a: 2 };
            let right: meta.Object = { b: true, b42, c: .done };
            let merged = left.merge(right);

            let keys = left.keys();
            let keys_ok = keys.length() == 3
                and (keys.get(0) matches .a?)
                and (keys.get(1) matches .b?)
                and (keys.get(2) matches .a?);

            let has_ok = left.has_key(.a)
                and left.has_key(.b)
                and not left.has_key(.missing)
                and not empty.has_key(.a);

            -- merged keeps unnamed positional entries ('x', 42u8) in order and keeps
            -- only the last value for each named key (a -> 2, b -> true, c -> .done).
            let merged_ok = merged.length() == 5
                and (merged.value<char>(0) matches 'x'?)
                and (merged.value<u8>(3) match {
                    v? => v == 42,
                    null => false,
                })
                and (merged.get<Integer>(.a) matches 2?)
                and (merged.get<bool>(.b) matches true?)
                and (merged.get<Symbol>(.c) matches .done?);

            let sym: Symbol = .hello;
            let utf8_sym: Symbol = .中;
            let sym_ok = sym.byte_length() == 5
                and (sym.byte(0) match {
                    v? => v == 104,
                    null => false,
                })
                and (sym.byte(1) match {
                    v? => v == 101,
                    null => false,
                })
                and (sym.byte(4) match {
                    v? => v == 111,
                    null => false,
                })
                and (sym.byte(5) matches null)
                and utf8_sym.byte_length() == 3
                and (utf8_sym.byte(0) match {
                    v? => v == 228,
                    null => false,
                });

            empty.is_empty() and not left.is_empty() and keys_ok and has_ok and merged_ok and sym_ok
        }
    "#;
    assert_eq!(support::invoke(source, "run").unwrap(), Value::Bool(true));
}

#[test]
fn attribute_suppliers_can_inspect_independent_unannotated_declarations() {
    let source = r#"
        ^attrs.metadata({
            plain_type: Plain'attribute.is_empty(),
            plain_fn: plain_fn'attribute.is_empty(),
        })
        struct Decorated {}
        struct Plain {}
        fn plain_fn() -> bool { true }
        comptime fn run() -> bool {
            Decorated'attribute.get<meta.Object>(.__flurry_metadata) match {
                data? => (data.get<bool>(.plain_type) matches true?)
                    and (data.get<bool>(.plain_fn) matches true?),
                null => false,
            }
        }
    "#;
    assert_eq!(support::invoke(source, "run").unwrap(), Value::Bool(true));
}

#[test]
fn attribute_suppliers_follow_demand_and_reject_cycles() {
    let source = r#"
        ^attrs.metadata({ left: has_packed(middle), right: has_packed(middle) })
        struct First {}
        ^attrs.metadata({ inherited: Last'attribute.has_key(.__flurry_packed_mode) })
        fn middle() -> bool { true }
        ^attrs.packed.byte
        struct Last { tag: u8 }
        comptime fn has_packed(f: fn() -> bool) -> bool {
            f'attribute.get<meta.Object>(.__flurry_metadata) match {
                data? => data.get<bool>(.inherited) matches true?,
                null => false,
            }
        }
        comptime fn run() -> bool {
            First'attribute.get<meta.Object>(.__flurry_metadata) match {
                data? => (data.get<bool>(.left) matches true?)
                    and (data.get<bool>(.right) matches true?),
                null => false,
            }
        }
    "#;
    assert_eq!(support::invoke(source, "run").unwrap(), Value::Bool(true));
    for (source, cycle) in [
        ("^attrs.metadata(A'attribute) struct A {}", "A -> A"),
        (
            "^attrs.metadata(B'attribute) struct A {}
          ^attrs.metadata(A'attribute) struct B {}",
            "A -> B -> A",
        ),
    ] {
        let failure = support::compile(source).unwrap_err();
        assert!(
            failure.contains(&format!("cyclic definition attribute dependency: {cycle}")),
            "{failure}"
        );
    }
}

#[test]
fn early_and_published_attribute_reads_agree_on_overrides_and_empty_objects() {
    let source = r#"
        ^attrs.metadata({ overridden: Target'attribute.get<Symbol>(.__flurry_packed_mode) matches .byte?, empty: Empty'attribute.is_empty() })
        struct Reader {}

        ^attrs.packed.byte
        ^attrs.packed.bit
        struct Target {
            tag: u8,
        }

        const empty_attrs: meta.Object = {};
        ^empty_attrs
        struct Empty {}

        comptime fn run() -> bool {
            Reader'attribute.get<meta.Object>(.__flurry_metadata) match {
                data? => (data.get<bool>(.overridden) matches true?)
                    and (Target'attribute.get<Symbol>(.__flurry_packed_mode) matches .byte?)
                    and (data.get<bool>(.empty) matches true?)
                    and Empty'attribute.is_empty(),
                null => false,
            }
        }
    "#;
    assert_eq!(support::invoke(source, "run").unwrap(), Value::Bool(true));
}

#[test]
fn early_attribute_reads_still_require_the_final_consumer_to_accept_the_target() {
    let source = r#"
        ^attrs.metadata(late'attribute)
        struct Reader {}
        ^attrs.packed.byte
        fn late() -> bool { true }
    "#;
    let failure = support::compile(source).unwrap_err();
    assert!(
        failure.contains("__flurry_packed_mode requires a struct definition"),
        "{failure}"
    );

    let source = r#"
        ^attrs.metadata(Late'attribute)
        struct Reader {}
        ^{ __flurry_metadata: false }
        struct Late {}
    "#;
    let failure = support::compile(source).unwrap_err();
    assert!(
        failure.contains("metadata requires the Object domain"),
        "{failure}"
    );
}

#[test]
fn attribute_supplier_depth_exhaustion_is_distinct_from_a_cycle() {
    let chain = |edges: usize| {
        let mut source = String::new();
        for i in 0..edges {
            source.push_str(&format!(
                "^attrs.metadata({{ has_next: D{}'attribute.has_key(.__flurry_metadata) }}) struct D{i} {{}}\n", i + 1
            ));
        }
        source.push_str(&format!(
            "^attrs.metadata({{ ready: true }}) struct D{edges} {{}}"
        ));
        source
    };
    support::compile(&chain(7)).unwrap();
    let failure = support::compile(&chain(8)).unwrap_err();
    assert!(
        failure.contains("attribute supplier depth exhausted"),
        "{failure}"
    );
}

#[test]
fn attribute_suppliers_build_each_generated_declaration_once() {
    let package = support::compile(
        r#"
        comptime fn outer() -> meta.Object {
            let witness = meta.StructBuilder.new(.name = .BeforeAttributeRead).Build();
            let left = Inner'attribute;
            let right = Inner'attribute;
            let data: meta.Object = { generated: witness == witness, left: left.has_key(.__flurry_metadata), right: right.has_key(.__flurry_metadata) };
            data
        }
        ^attrs.metadata(outer())
        struct Outer {}
        comptime fn inner() -> meta.Object {
            let witness = meta.StructBuilder.new(.name = .DependencyWitness).Build();
            let data: meta.Object = { generated: witness == witness };
            data
        }
        ^attrs.metadata(inner())
        struct Inner {}
    "#,
    )
    .unwrap();
    for name in ["BeforeAttributeRead", "DependencyWitness"] {
        assert_eq!(
            package
                .structs
                .iter()
                .filter(|definition| definition.name == name)
                .count(),
            1,
            "{name}"
        );
    }
}

#[test]
fn nested_attribute_values_deliver_generated_layouts_to_reflection_and_builders() {
    let source = r#"
        comptime fn Generated(subject: Type) -> Type {
            subject'attribute.get<meta.Object>(.__flurry_metadata) match {
                data? => data.get<Type>(.generated) match { ty? => ty, null => bool },
                null => bool,
            }
        }
        comptime fn outer() -> meta.Object {
            let choice = Generated(Inner);
            let payload = choice.enum_variant_field_types()[0][0];
            let flag = payload.struct_field_types()[0];
            let record = meta.StructBuilder.new(.name = .Envelope);
            record.add_field(.value, choice);
            let data = {
                generated: record.Build(),
                correct: flag.NewtypeRepresentation() == bool and Generated(Inner) == choice,
            };
            data
        }
        ^attrs.metadata(outer())
        struct Outer {}
        comptime fn inner() -> meta.Object {
            let flag = meta.NewtypeBuilder.new(bool, .name = .Flag).Build();
            let record = meta.StructBuilder.new(.name = .Payload);
            record.add_field(.flag, flag);
            let choice = meta.EnumBuilder.new(.name = .Choice);
            choice.add_tuple(.some, [record.Build()]);
            let data = { generated: choice.Build() };
            data
        }
        ^attrs.metadata(inner())
        struct Inner {}
        comptime fn run() -> bool {
            let correct = Outer'attribute.get<meta.Object>(.__flurry_metadata) match {
                data? => data.get<bool>(.correct) matches true?, null => false,
            };
            correct and Generated(Outer).struct_field_types()[0] == Generated(Inner)
        }
    "#;
    assert_eq!(support::invoke(source, "run").unwrap(), Value::Bool(true));
}

#[test]
fn normal_form_instances_read_the_head_version_through_aliases() {
    let source = r#"
        ^attrs.stable_since("1.2.3")
        pure comptime fn Tagged<T> -> Type { T }
        typealias Wrapped = Tagged<u32>;
        comptime fn check() -> bool {
            let version = Wrapped'attribute.get<str>(.__flurry_stable_since);
            (version matches "1.2.3"?) and not u32'attribute.has_key(.__flurry_stable_since)
        }
        const result = check();
        fn run() -> bool { result }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn stable_since_and_macro_attributes_enforce_targets_semver_and_combinations() {
    let valid_stable = r#"
        ^attrs.stable_since("0.1.0-alpha.1+build.42")
        struct AnchoredStruct { value: u32 }

        ^attrs.stable_since("2.0.0")
        enum AnchoredEnum { a, b(u32) }

        ^attrs.stable_since("1.0.0")
        newtype AnchoredNewtype = u32;

        ^attrs.stable_since("1.2.3-rc.1")
        pure comptime fn AnchoredNf<T> -> Type { T }

        comptime fn check() -> bool {
            (AnchoredStruct'attribute.get<str>(.__flurry_stable_since) matches "0.1.0-alpha.1+build.42"?)
                and (AnchoredEnum'attribute.get<str>(.__flurry_stable_since) matches "2.0.0"?)
                and (AnchoredNewtype'attribute.get<str>(.__flurry_stable_since) matches "1.0.0"?)
                and (AnchoredNf<u32>'attribute.get<str>(.__flurry_stable_since) matches "1.2.3-rc.1"?)
        }
        const result = check();
        fn run() -> bool { result }
    "#;
    assert_eq!(support::run(valid_stable), Value::Bool(true));

    for bad_semver in [
        "\"1.0\"",
        "\"v1.0.0\"",
        "\"01.0.0\"",
        "\"1.0.0-01\"",
        "\"1.0.0-\"",
        "\"\"",
    ] {
        let src = format!("^attrs.stable_since({bad_semver}) struct S {{}}");
        let err = support::compile(&src).unwrap_err();
        assert!(err.contains("SemVer 2.0.0"), "{bad_semver}: {err}");
        assert!(
            !err.starts_with("0..0:"),
            "zero span on {bad_semver}: {err}"
        );
    }

    for bad_stable_target in [
        "^attrs.stable_since(\"1.0.0\") fn ordinary() -> bool { true }",
        "^attrs.stable_since(\"1.0.0\") comptime fn helper() -> bool { true }",
        "^attrs.stable_since(\"1.0.0\") typealias Alias = u32;",
        "^attrs.stable_since(\"1.0.0\") const C: u32 = 1;",
        "struct Holder { ^attrs.stable_since(\"1.0.0\") field: u32 }",
        "struct ConfigS { .__flurry_stable_since \"1.0.0\", field: u32 }",
    ] {
        let err = support::compile(bad_stable_target).unwrap_err();
        assert!(
            err.contains("__flurry_stable_since"),
            "{bad_stable_target}: {err}"
        );
        assert!(
            !err.starts_with("0..0:"),
            "zero span on {bad_stable_target}: {err}"
        );
    }

    let valid_macros = r#"
        use meta.ast.TokenBuffer;

        ^attrs.macros.macro3
        comptime fn m3(tb: TokenBuffer) -> TokenBuffer { tb }

        ^attrs.macros.macro4
        comptime fn m4(src: str) -> TokenBuffer { TokenBuffer.empty() }

        ^attrs.macros.ast(false)
        comptime fn m1(tb: TokenBuffer) -> TokenBuffer { tb }

        const VARIADIC_FLAG: bool = true;
        ^attrs.macros.ast(VARIADIC_FLAG)
        comptime fn m1_const(tb: TokenBuffer) -> TokenBuffer { tb }

        ^attrs.macros.peg("Entry -> \"x\";")
        comptime fn m2(tb: TokenBuffer) -> TokenBuffer { tb }

        comptime fn check() -> bool {
            (m3'attribute.get<Symbol>(.__flurry_macro_kind) matches .tokens?)
                and (m4'attribute.get<Symbol>(.__flurry_macro_kind) matches .source?)
                and (m1'attribute.get<Symbol>(.__flurry_macro_kind) matches .ast?)
                and (m1'attribute.get<bool>(.__flurry_macro_variadic) matches false?)
                and (m1_const'attribute.get<Symbol>(.__flurry_macro_kind) matches .ast?)
                and (m1_const'attribute.get<bool>(.__flurry_macro_variadic) matches true?)
                and (m2'attribute.get<Symbol>(.__flurry_macro_kind) matches .peg?)
        }
        const result = check();
        fn run() -> bool { result }
    "#;
    assert_eq!(support::run(valid_macros), Value::Bool(true));

    for bad_macro in [
        "^attrs.macros.macro3 fn runtime_fn() -> bool { true }",
        "^attrs.macros.macro3 struct NotMacro {}",
        "^attrs.macros.macro3 pure comptime fn Nf<T> -> Type { T }",
        "struct Holder { ^attrs.macros.macro3 field: u32 }",
        "^{ __flurry_macro_kind: .unknown } comptime fn bad() -> bool { true }",
        "^{ __flurry_macro_kind: .peg } comptime fn missing_grammar() -> bool { true }",
        "^{ __flurry_macro_kind: .tokens, __flurry_macro_variadic: true } comptime fn bad_var() -> bool { true }",
        "^{ __flurry_macro_kind: .ast, __flurry_macro_grammar: \"Entry -> \\\"x\\\";\" } comptime fn bad_gram() -> bool { true }",
        "^{ __flurry_macro_variadic: true } comptime fn orphan_var() -> bool { true }",
        "struct ConfigM { .__flurry_macro_kind .tokens, field: u32 }",
    ] {
        let err = support::compile(bad_macro).unwrap_err();
        assert!(err.contains("__flurry_macro_"), "{bad_macro}: {err}");
        assert!(!err.starts_with("0..0:"), "zero span on {bad_macro}: {err}");
    }
}

#[test]
fn foreign_abi_conventions_emit_llvm_cc_and_execute_natively() {
    let source = r#"
        ^attrs.abi(.c)
        fn c_add(a: u32, b: u32) -> u32 { assumes a <= 1000 and b <= 1000; a + b }

        ^attrs.abi(.system)
        fn sys_add(a: u32, b: u32) -> u32 { assumes a <= 1000 and b <= 1000; a + b }

        ^attrs.abi(.stdcall)
        fn std_add(a: u32, b: u32) -> u32 { assumes a <= 1000 and b <= 1000; a + b }

        ^attrs.abi(.win64)
        fn win_add(a: u32, b: u32) -> u32 { assumes a <= 1000 and b <= 1000; a + b }

        ^attrs.abi(.sysv64)
        fn sysv_add(a: u32, b: u32) -> u32 { assumes a <= 1000 and b <= 1000; a + b }

        fn run() -> bool {
            c_add(1, 2) == 3
                and sys_add(3, 4) == 7
                and std_add(5, 6) == 11
                and win_add(7, 8) == 15
                and sysv_add(9, 10) == 19
        }
    "#;
    let case = NativeCase::new(source);
    let ir = case.emit_llvm().unwrap();
    assert!(ir.contains("define i32 @\"\\63\\5F\\61\\64\\64\"("), "{ir}");
    assert!(
        ir.contains("define i32 @\"\\73\\79\\73\\5F\\61\\64\\64\"("),
        "{ir}"
    );
    assert!(
        ir.contains("define x86_stdcallcc i32 @\"\\73\\74\\64\\5F\\61\\64\\64\"("),
        "{ir}"
    );
    assert!(
        ir.contains("define win64cc i32 @\"\\77\\69\\6E\\5F\\61\\64\\64\"("),
        "{ir}"
    );
    assert!(
        ir.contains("define x86_64_sysvcc i32 @\"\\73\\79\\73\\76\\5F\\61\\64\\64\"("),
        "{ir}"
    );

    if cfg!(target_arch = "x86_64") {
        let abi_case = NativeCase::new(
            "
            ^attrs.abi(.c)
            fn abi_c(a: u32, b: u32) -> u32 { assumes a <= 1000 and b <= 1000; a + b }
            ^attrs.abi(.system)
            fn abi_sys(a: u32, b: u32) -> u32 { assumes a <= 1000 and b <= 1000; a + b }
            ^attrs.abi(.win64)
            fn abi_win(a: u32, b: u32) -> u32 { assumes a <= 1000 and b <= 1000; a + b }
            ^attrs.abi(.sysv64)
            fn abi_sysv(a: u32, b: u32) -> u32 { assumes a <= 1000 and b <= 1000; a + b }
            fn run() -> bool {
                abi_c(1, 2) == 3
                    and abi_sys(3, 4) == 7
                    and abi_win(5, 6) == 11
                    and abi_sysv(7, 8) == 15
            }
            ",
        );
        let ir = abi_case
            .emit_llvm()
            .unwrap()
            .replace("define i32 @main()", "define i32 @unused_main()");
        let harness = format!(
            "{ir}
            define i32 @main() {{
                %a = call i32 @abi_c(i32 10, i32 1)
                %b = call i32 @abi_sys(i32 20, i32 2)
                %c = call win64cc i32 @abi_win(i32 30, i32 3)
                %d = call x86_64_sysvcc i32 @abi_sysv(i32 40, i32 4)
                %s1 = add i32 %a, %b
                %s2 = add i32 %c, %d
                %sum = add i32 %s1, %s2
                %status = sub i32 %sum, 110
                ret i32 %status
            }}"
        );
        for optimization in [
            luna::host::OptimizationLevel::O0,
            luna::host::OptimizationLevel::O3,
        ] {
            luna::host::Toolchain {
                optimization,
                ..Default::default()
            }
            .execute_test(&harness, std::time::Duration::from_secs(5))
            .unwrap();
        }
    }
}
