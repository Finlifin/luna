use comptime_interpretor::{Interpreter, Value};
use luna::{
    frontend::Artifact,
    packages::{SourcePackage, compile_and_invoke_with_struct_builders},
};
use std::path::PathBuf;

static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(source: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "luna-builder-invocation-{}-{nonce}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("lib.fl"), source).unwrap();
        Self(path)
    }
    fn packages(&self) -> Vec<SourcePackage> {
        vec![SourcePackage {
            resolution_id: 17,
            identity: 900,
            name: "input".into(),
            entry: self.0.join("lib.fl"),
        }]
    }
    fn invoke(
        &self,
        entry: &str,
        structural: &mut usize,
        steps: u64,
    ) -> Result<(Artifact, Value, elaboration::program::Protocols), String> {
        compile_and_invoke_with_struct_builders(
            &self.packages(),
            &[],
            17,
            None,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input.List"),
                ..Default::default()
            },
            None,
            &[],
            entry,
            structural,
            steps,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
const PREFIX: &str = "
    typealias Type = .Type'builtin;
    pub pure comptime fn List<T> -> Type {List<T>}
    pure comptime fn Identity<T> -> Type {T}
    struct Original {flag:bool}
";

#[test]
fn package_driver_returns_generated_types_and_their_checked_world() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        comptime fn Build(.enabled:bool = true) -> Type {{
            let first = .build_struct'builtin(.First, [.wrapped], [Identity<Original>]);
            if enabled {{ .build_struct'builtin(.Second, [.nested], [first]) }} else {{ first }}
        }}
    "
    ));
    let mut structural = 1_000_000;
    let (Artifact::Nir(package), Value::Type(generated), protocols) =
        fixture.invoke("Build", &mut structural, 1_000_000).unwrap()
    else {
        panic!("expected checked NIR and Type")
    };
    assert!(structural < 1_000_000);
    assert!(protocols.meta_list.is_some());
    assert_eq!(package.structs.len(), 3);
    let nir::Term::Nominal { declaration, .. } = &generated else {
        panic!("nominal result")
    };
    assert_eq!(declaration.package, 900);
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    let second = vm.reflect_struct(&generated).unwrap();
    assert_eq!(second.fields[0].name, "nested");
    let first = vm.reflect_struct(&second.fields[0].ty).unwrap();
    assert_eq!(first.fields[0].name, "wrapped");
    let nir::Term::NormalForm {
        head, arguments, ..
    } = &first.fields[0].ty
    else {
        panic!("Identity head must remain opaque")
    };
    assert!(
        package
            .normal_forms
            .iter()
            .any(|item| item.declaration == *head)
    );
    assert!(matches!(arguments.as_slice(), [nir::Term::Nominal { .. }]));
}

#[test]
fn package_driver_preserves_ordinary_results_and_reports_builder_failures() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        comptime fn ready() -> bool {{true}}
        fn runtime() -> bool {{true}}
        comptime fn Bad() -> Type {{.build_struct'builtin(.Bad, [.x, .x], [bool, bool])}}
        comptime fn Build() -> Type {{.build_struct'builtin(.Generated, [.flag], [bool])}}
    "
    ));
    let (Artifact::Nir(package), value, _) =
        fixture.invoke("ready", &mut 1_000_000, 1_000_000).unwrap()
    else {
        panic!()
    };
    assert_eq!(value, Value::Bool(true));
    assert_eq!(package.structs.len(), 1);
    for entry in ["missing", "runtime", "Bad"] {
        assert!(
            fixture.invoke(entry, &mut 1_000_000, 1_000_000).is_err(),
            "{entry}"
        );
    }
    assert!(
        fixture
            .invoke("Build", &mut 0, 1_000_000)
            .err()
            .unwrap()
            .contains("preparation")
    );
    assert!(
        fixture
            .invoke("Build", &mut 1_000_000, 0)
            .err()
            .unwrap()
            .contains("positive")
    );
    assert!(fixture.invoke("Build", &mut 1_000_000, 1).is_err());
    assert!(fixture.invoke("Build", &mut 1_000_000, 1_000_000).is_ok());
}

#[test]
fn builder_driver_preserves_dependency_types_preludes_and_primary_entry_identity() {
    let fixture = Fixture::new(&format!("{PREFIX}
        use dependency.Shared;
        comptime fn Build() -> Type {{
            if enabled() {{ .build_struct'builtin(.Generated, [.shared], [Shared]) }} else {{ Original }}
        }}
    "));
    let dependency = fixture.0.join("dep.fl");
    std::fs::write(
        &dependency,
        "
        pub struct Shared {flag:bool}
        pub comptime fn Build() -> .Type'builtin {Shared}
        pub mod prelude {pub pure comptime fn enabled()->bool {true}}
    ",
    )
    .unwrap();
    let mut packages = fixture.packages();
    packages.push(SourcePackage {
        resolution_id: 42,
        identity: 901,
        name: "dependency".into(),
        entry: dependency,
    });
    let dependencies = [resolve::DependencyEdge {
        requester: 17,
        alias: "dependency".into(),
        provider: 42,
    }];
    let (Artifact::Nir(package), Value::Type(generated), _) =
        compile_and_invoke_with_struct_builders(
            &packages,
            &dependencies,
            17,
            None,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input.List"),
                ..Default::default()
            },
            None,
            &[42],
            "Build",
            &mut 1_000_000,
            1_000_000,
        )
        .unwrap()
    else {
        panic!()
    };
    let nir::Term::Nominal { declaration, .. } = generated.clone() else {
        panic!()
    };
    assert_eq!(declaration.package, 900);
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    let fields = vm.reflect_struct(&generated).unwrap().fields;
    assert_eq!(fields[0].name, "shared");
    let nir::Term::Nominal { declaration, .. } = fields[0].ty else {
        panic!()
    };
    assert_eq!(declaration.package, 901);
}

impl Fixture {
    fn invoke_meta(
        &self,
        entry: &str,
    ) -> Result<(Artifact, Value, elaboration::program::Protocols), String> {
        let mut packages = self.packages();
        packages.push(SourcePackage {
            resolution_id: 42,
            identity: 901,
            name: "meta".into(),
            entry: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library/meta/lib.fl"),
        });
        compile_and_invoke_with_struct_builders(
            &packages,
            &[resolve::DependencyEdge {
                requester: 17,
                alias: "meta".into(),
                provider: 42,
            }],
            17,
            None,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("meta.List"),
                ..Default::default()
            },
            None,
            &[42],
            entry,
            &mut 1_000_000,
            1_000_000,
        )
    }
}

#[test]
fn public_struct_builder_copies_reflected_fields_through_mutating_methods() {
    let fixture = Fixture::new(
        "
        struct Source {first:bool, second:bool}
        comptime fn run() -> bool {
            let names = meta.struct_field_names(Source);
            let types = meta.struct_field_types(Source);
            let count = meta.length(names);
            var index = count - count;
            var builder = meta.StructBuilder.new(.name = .Columns);
            while index < count {
                builder.add_field(names[index], types[index]);
                index = index + 1;
            }
            let generated = builder.Build();
            let observed = meta.struct_field_names(generated);
            let fields = meta.struct_field_types(generated);
            meta.length(observed) == 2 and observed[0] == .first and observed[1] == .second
                and fields[0] == bool and fields[1] == bool
        }
    ",
    );
    let (Artifact::Nir(package), value, _) = fixture.invoke_meta("run").unwrap() else {
        panic!()
    };
    assert_eq!(value, Value::Bool(true));
    let generated = package
        .structs
        .iter()
        .find(|item| item.name == "Columns")
        .unwrap();
    assert_eq!(generated.declaration.package, 900);
    assert_eq!(generated.fields.len(), 2);
}

#[test]
fn public_struct_builder_handles_empty_defaults_and_rejects_duplicate_fields() {
    let fixture = Fixture::new(
        "
        comptime fn empty() -> bool {
            let first = meta.StructBuilder.new().Build();
            let second = meta.StructBuilder.new().Build();
            meta.length(meta.struct_field_names(first)) == 0
                and meta.length(meta.struct_field_types(second)) == 0 and not (first == second)
        }
        comptime fn duplicate() -> bool {
            var builder = meta.StructBuilder.new();
            builder.add_field(.flag, bool);
            builder.add_field(.flag, bool);
            builder.Build(); true
        }
        fn runtime() -> bool {true}
    ",
    );
    assert_eq!(fixture.invoke_meta("empty").unwrap().1, Value::Bool(true));
    assert!(fixture.invoke_meta("duplicate").is_err());
    assert!(fixture.invoke_meta("runtime").is_err());
}

#[test]
fn public_struct_builder_cannot_escape_into_a_runtime_interface() {
    for source in [
        "fn bad() -> meta.StructBuilder {meta.StructBuilder.new()}",
        "fn bad(value: meta.StructBuilder) -> meta.StructBuilder {value}",
    ] {
        let fixture = Fixture::new(source);
        let error = fixture.invoke_meta("bad").err().unwrap();
        assert!(error.contains("elaboration failed"), "{error}");
    }
}

#[test]
fn package_cli_invokes_public_builders_with_observed_inputs() {
    let fixture = Fixture::new(
        "
        comptime fn run() -> bool {
            var builder = meta.StructBuilder.new(.name = .Visible);
            builder.add_field(.flag, bool);
            let generated = builder.Build();
            meta.struct_field_names(generated)[0] == .flag
                and meta.struct_field_types(generated)[0] == bool
        }
    ",
    );
    let meta = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library/meta/lib.fl");
    for observed in [false, true] {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_luna"));
        command
            .args(["--packages-v1", "--package", "17", "900", "app"])
            .arg(fixture.0.join("lib.fl"))
            .args(["--package", "42", "901", "meta"])
            .arg(&meta)
            .args([
                "--dependency",
                "17",
                "meta",
                "42",
                "--primary",
                "17",
                "--meta-list-type",
                "meta.List",
                "--invoke-comptime",
                "run",
            ]);
        if observed {
            command.arg("--with-inputs");
        }
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8(result.stdout).unwrap();
        if observed {
            assert!(stdout.contains("(result (bool true))"), "{stdout}");
            assert!(
                stdout.contains("(source (resolution-id 42) (identity 901)"),
                "{stdout}"
            );
            assert!(stdout.contains("pub struct StructBuilder"), "{stdout}");
        } else {
            assert_eq!(stdout.trim(), "(bool true)");
        }
    }
}

#[test]
fn standalone_cli_executes_builders_and_preserves_source_output_protection() {
    let fixture = Fixture::new(&format!(
        "mod meta {{ {} }}
        comptime fn Generate() -> bool {{
            let generated = meta.StructBuilder.new().Build();
            meta.length(meta.struct_field_types(generated)) == 0
        }}",
        include_str!("../../../library/meta/lib.fl")
    ));
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .arg(fixture.0.join("lib.fl"))
        .args([
            "--meta-list-type",
            "input.meta.List",
            "--invoke-comptime",
            "Generate",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout).trim(),
        "(bool true)"
    );
    let source = fixture.0.join("lib.fl");
    let before = std::fs::read(&source).unwrap();
    let rejected = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .arg(&source)
        .args([
            "--meta-list-type",
            "input.meta.List",
            "--invoke-comptime",
            "Generate",
            "-o",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert_eq!(std::fs::read(&source).unwrap(), before);
}

#[test]
fn sysroot_cli_installs_builder_service_after_loading_meta_bindings() {
    let fixture = Fixture::new(
        "
        comptime fn run() -> bool {
            var builder = meta.StructBuilder.new();
            builder.add_field(.flag, bool);
            meta.struct_field_types(builder.Build())[0] == bool
        }
    ",
    );
    let root = fixture.0.join("library");
    for name in ["builtin", "std", "meta"] {
        std::fs::create_dir_all(root.join(name)).unwrap();
    }
    std::fs::write(root.join("builtin/lib.fl"), "pub mod prelude {}").unwrap();
    std::fs::write(root.join("std/lib.fl"), "pub mod prelude {}").unwrap();
    std::fs::write(
        root.join("meta/lib.fl"),
        include_str!("../../../library/meta/lib.fl"),
    )
    .unwrap();
    std::fs::write(
        root.join("language-items.conf"),
        "version = 1\nmeta-list-type = meta.List\n",
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .arg(fixture.0.join("lib.fl"))
        .arg("--sysroot")
        .arg(&root)
        .args(["--invoke-comptime", "run"])
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
