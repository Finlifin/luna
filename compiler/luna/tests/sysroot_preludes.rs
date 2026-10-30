//! Exercise driver-selected built-in preludes independently of unfinished APIs.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Inputs(PathBuf);
impl Inputs {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "luna-six-preludes-{}-{nonce}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, path: &str, source: &str) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    fn run(&self, source: &str) -> std::process::Output {
        self.write("input.fl", source);
        std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
            .arg(self.0.join("input.fl"))
            .arg("--sysroot")
            .arg(self.0.join("library"))
            .args(["--interpret", "run"])
            .output()
            .unwrap()
    }
}
impl Drop for Inputs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn assert_true(output: std::process::Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn source_configured_iterator_runs_ordinary_for_from_prelude() {
    let input = Inputs::new();
    input.write(
        "library/builtin/lib.fl",
        "pub use primitives.*; pub use iterator.*; pub use resources.*; pub mod prelude {pub use primitives.*; pub use iterator.Iterator;}",
    );
    input.write(
        "library/builtin/primitives.fl",
        include_str!("../../../library/builtin/primitives.fl"),
    );
    input.write(
        "library/builtin/iterator.fl",
        include_str!("../../../library/builtin/protocols/iterator.fl"),
    );
    input.write(
        "library/builtin/resources.fl",
        include!("support/resources_source.rs"),
    );
    input.write("library/language-items.conf", "version = 1\niterator-protocol = builtin.iterator.Iterator\ncopy-protocol = builtin.resources.Copy\ndrop-protocol = builtin.resources.Drop\n");
    for package in ["meta", "build", "attrs", "verification", "std"] {
        input.write(&format!("library/{package}/lib.fl"), "pub mod prelude {}");
    }
    input.write(
        "input.fl",
        include_str!("../../../development/repros/ordinary-for-inline-effect.fl"),
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .arg(input.0.join("input.fl"))
        .arg("--sysroot")
        .arg(input.0.join("library"))
        .args([
            "--test-native",
            "--inline-functions",
            "--copy-protocol",
            "builtin.resources.Copy",
            "--drop-protocol",
            "builtin.resources.Drop",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(report.contains("(count 1) (failures 0)"), "{report}");
}

#[test]
fn complete_shipped_sysroot_runs_tuple_optional_math_and_iteration() {
    let input = Inputs::new();
    input.write(
        "input.fl",
        "
        struct Container {value:?bool}
        impl IntoIterator for Container {
            assoc Iter:Type=?bool;
            fn into_iter(self)->?bool {self.value}
        }
        fn run()->bool {
            let pair:Tuple<bool,bool> = (true,false);
            let (yes,no)=pair;
            let query:?bool=yes;
            let present=query.as(OptionalOps<bool>).is_some();
            let value:?bool=yes;
            let number:f64=4.0;
            let root=math.sqrt(number);
            for item in (Container {value:value}) {return item and not no and present and root == 2.0;}
            false
        }
        test {let passed=run();assumes passed;}
    ",
    );
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .arg(input.0.join("input.fl"))
        .arg("--sysroot")
        .arg(library)
        .args([
            "--test-native",
            "--inline-functions",
            "--copy-protocol",
            "builtin.protocols.resources.Copy",
            "--drop-protocol",
            "builtin.protocols.resources.Drop",
            "--fuel",
            "4000000",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(report.contains("(count 1) (failures 0)"), "{report}");
}

#[test]
fn complete_shipped_sysroot_runs_qualified_type_traits() {
    let input = Inputs::new();
    input.write(
        "input.fl",
        include_str!("../../../development/examples/sysroot-qualified-methods.fl"),
    );
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .arg(input.0.join("input.fl"))
        .arg("--sysroot")
        .arg(library)
        .args([
            "--test-native",
            "--inline-functions",
            "--copy-protocol",
            "builtin.protocols.resources.Copy",
            "--drop-protocol",
            "builtin.protocols.resources.Drop",
            "--fuel",
            "4000000",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(report.contains("(count 1) (failures 0)"), "{report}");
}

#[test]
fn shipped_text_module_exposes_checked_utf8_byte_reads() {
    let input = Inputs::new();
    input.write(
        "library/builtin/lib.fl",
        "pub use primitives.*; pub mod text {pub use text_bytes.*;} pub mod prelude {pub use primitives.*;}",
    );
    input.write(
        "library/builtin/primitives.fl",
        include_str!("../../../library/builtin/primitives.fl"),
    );
    input.write(
        "library/builtin/text_bytes.fl",
        include_str!("../../../library/builtin/text_impl/bytes.fl"),
    );
    for package in ["meta", "build", "attrs", "verification", "std"] {
        input.write(&format!("library/{package}/lib.fl"), "pub mod prelude {}");
    }
    assert_true(input.run("use builtin.text; fn run()->bool {let length=text.byte_length(\"é\");let byte=text.byte(\"é\",1);let valid=byte match {value?=>value == 169,null=>false};length == 2 and valid}"));
}

#[test]
fn five_default_preludes_deduplicate_identity_and_std_stays_explicit() {
    let input = Inputs::new();
    input.write(
        "library/builtin/lib.fl",
        "
        pub typealias bool = .bool'builtin;
        pub mod math {pub fn answer()->bool {true}}
        pub mod prelude {pub use .bool; pub fn base()->bool {true} pub use .math;}
    ",
    );
    input.write(
        "library/meta/lib.fl",
        "
        pub mod prelude {pub use builtin.prelude.base; pub fn metadata()->bool {true}}
    ",
    );
    for (package, marker) in [
        ("build", "configuration"),
        ("attrs", "attribute"),
        ("verification", "specification"),
        ("std", "standard"),
    ] {
        input.write(
            &format!("library/{package}/lib.fl"),
            &format!("pub mod prelude {{pub fn {marker}()->bool {{true}}}}"),
        );
    }
    // math is a builtin module. A stale standalone directory must not be loaded.
    input.write(
        "library/math/lib.fl",
        "this is intentionally not a package;",
    );
    assert_true(input.run("fn run()->bool {base() and metadata() and configuration() and attribute() and specification() and math.answer()}"));
    let missing = input.run("fn run()->bool {standard()}");
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("standard"));
    assert_true(
        input.run("use std.prelude.*; fn run()->bool {standard() and base() and metadata()}"),
    );

    // Automatic globs supply local lookup, not a new public export route.
    let private = input.run("use build.base; fn run()->bool {base()}");
    assert!(!private.status.success());
    assert!(String::from_utf8_lossy(&private.stderr).contains("base"));
    assert_true(input.run("fn run()->bool {base() and specification()}"));
}

#[test]
fn optional_provider_absence_is_distinct_from_a_broken_present_provider() {
    let input = Inputs::new();
    input.write(
        "library/builtin/lib.fl",
        "pub typealias bool = .bool'builtin; pub mod prelude {pub use .bool; pub fn base()->bool {true}}",
    );
    input.write("library/std/lib.fl", "pub mod prelude {}");
    assert_true(input.run("fn run()->bool {base()}"));
    std::fs::create_dir(input.0.join("library/meta")).unwrap();
    let missing_entry = input.run("fn run()->bool {base()}");
    assert!(!missing_entry.status.success());
    assert!(
        String::from_utf8_lossy(&missing_entry.stderr).contains("sysroot package 'meta' requires")
    );
    input.write(
        "library/meta/lib.fl",
        "pub mod prelude {pub fn metadata()->bool {true}}",
    );
    assert_true(input.run("fn run()->bool {base() and metadata()}"));
}

#[test]
fn complete_shipped_sysroot_invokes_comptime_qualified_type_traits() {
    let input = Inputs::new();
    input.write(
        "input.fl",
        include_str!("../../../development/examples/sysroot-qualified-phases.fl"),
    );
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .arg(input.0.join("input.fl"))
        .arg("--sysroot")
        .arg(library)
        .args([
            "--build-target",
            "x86_64",
            "linux",
            "64",
            "--invoke-comptime",
            "inspect",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "(bool true)"
    );
}

#[test]
fn shipped_meta_object_preserves_type_valued_indices_in_normal_inputs() {
    let input = Inputs::new();
    input.write(
        "input.fl",
        "
        struct Key where T,value:T {payload:bool}
        pure comptime fn Tag<T,value:T,key:Key<T,value>> -> Type
        where requires Key<T,value> == Key<T,value> {T}
        comptime fn Build()->Type {
            let object=Object.append({},Key<Type,bool>{payload:true});
            let key=Object.value<Key<Type,bool>>(object,0) match {
                value?=>value,
                null=>Key<Type,bool>{payload:false},
            };
            Tag<Type,bool,key>
        }
        const STAGED=Build();
        fn run()->bool {
            const equal=STAGED == Tag<Type,bool,Key<Type,bool>{payload:true}>;
            equal
        }
    ",
    );
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .arg(input.0.join("input.fl"))
        .arg("--sysroot")
        .arg(library)
        .args([
            "--build-target",
            "x86_64",
            "linux",
            "64",
            "--interpret",
            "run",
            "--fuel",
            "4000000",
        ])
        .output()
        .unwrap();
    assert_true(output);
}

#[test]
fn shipped_meta_object_values_dispatch_type_indexed_traits() {
    let input = Inputs::new();
    input.write(
        "input.fl",
        "
        struct Key where T,value:T {payload:bool}
        trait Read where T,value:T {fn read(self)->bool;}
        impl Read<T,value> for Key<T,value> where T,value:T {
            fn read(self)->bool {self.payload}
        }
        comptime fn get_key()->Key<Type,bool> {
            let object=Object.append({},Key<Type,bool>{payload:true});
            Object.value<Key<Type,bool>>(object,0) match {
                value?=>value,
                null=>Key<Type,bool>{payload:false},
            }
        }
        fn run()->bool {
            const payload=get_key().payload;
            Key<Type,bool>{payload:payload}.as(Read<Type,bool>).read()
        }
    ",
    );
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .arg(input.0.join("input.fl"))
        .arg("--sysroot")
        .arg(library)
        .args([
            "--build-target",
            "x86_64",
            "linux",
            "64",
            "--interpret",
            "run",
            "--fuel",
            "4000000",
        ])
        .output()
        .unwrap();
    assert_true(output);
}

#[test]
fn shipped_type_factories_keep_identity_through_comptime_parameters() {
    let input = Inputs::new();
    input.write(
        "input.fl",
        "
        comptime fn Apply(factory:pure comptime fn<Type> -> Type, input:Type) -> Type {
            factory<input>
        }
        comptime fn Collect(factory:pure comptime fn<...Es:List<Type>> -> Type) -> Type {
            factory<bool, u8>
        }
        const SEQUENCE=Apply(List, bool);
        const PRODUCT=Collect(Tuple);
        fn run()->bool {
            const same = SEQUENCE == List<bool> and PRODUCT == Tuple<bool,u8>;
            same
        }
    ",
    );
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .arg(input.0.join("input.fl"))
        .arg("--sysroot")
        .arg(library)
        .args([
            "--build-target",
            "x86_64",
            "linux",
            "64",
            "--interpret",
            "run",
            "--fuel",
            "4000000",
        ])
        .output()
        .unwrap();
    assert_true(output);
}

#[test]
fn shipped_list_fold_retains_comptime_callback_state() {
    let input = Inputs::new();
    input.write(
        "input.fl",
        "
        comptime fn folded()->bool {
            var seen:Integer=0;
            let values: List<bool> = [true,false,true];
            let result=values.fold(0,|total:Integer,enabled:bool| {
                seen = seen + 1;
                total + if enabled {seen} else {0}
            });
            result == 4 and seen == 0
        }
    ",
    );
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .arg(input.0.join("input.fl"))
        .arg("--sysroot")
        .arg(library)
        .args([
            "--build-target",
            "x86_64",
            "linux",
            "64",
            "--invoke-comptime",
            "folded",
            "--fuel",
            "4000000",
        ])
        .output()
        .unwrap();
    assert_true(output);
}
