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
