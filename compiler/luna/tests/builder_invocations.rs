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
fn inherent_builder_can_form_the_subject_of_a_trait_implementation() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        struct Factory {{}}
        impl Factory {{
            comptime fn Make() -> Type {{.build_struct'builtin(.Created, [.flag], [bool])}}
        }}
        typealias Created = Factory.Make();
        trait Read {{fn read(self) -> bool;}}
        impl Read for Created {{fn read(self) -> bool {{self.flag}}}}
        fn run() -> bool {{Created {{flag:true}}.as(Read).read()}}
        comptime fn ready() -> bool {{true}}"
    ));
    let (Artifact::Nir(package), _, _) = fixture.invoke("ready", &mut 100_000, 100_000).unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        package
            .structs
            .iter()
            .filter(|def| def.name == "Created")
            .count(),
        1
    );
}

#[test]
fn early_inherent_builder_cannot_ignore_a_competing_later_candidate() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        struct Factory {{}}
        impl Factory {{comptime fn Make() -> Type {{.build_struct'builtin(.First, [.flag], [bool])}}}}
        typealias Created = Factory.Make();
        trait Read {{fn read(self) -> bool;}}
        impl Read for Created {{fn read(self) -> bool {{self.flag}}}}
        impl Factory {{comptime fn Make() -> Type {{.build_struct'builtin(.Second, [.flag], [bool])}}}}
        comptime fn ready() -> bool {{true}}"
    ));
    let failure = fixture
        .invoke("ready", &mut 100_000, 100_000)
        .err()
        .unwrap();
    assert!(
        failure.contains("overlap") || failure.contains("coherence"),
        "{failure}"
    );
}

#[test]
fn early_inherent_builder_retains_receiver_type_arguments() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        struct Factory where T {{}}
        impl Factory<T> where T {{
            comptime fn Make() -> Type {{.build_struct'builtin(.Created, [.flag], [T])}}
        }}
        typealias Created = Factory<bool>.Make();
        trait Read {{fn read(self) -> bool;}}
        impl Read for Created {{fn read(self) -> bool {{self.flag}}}}
        fn run() -> bool {{Created {{flag:true}}.as(Read).read()}}
        comptime fn ready() -> bool {{true}}"
    ));
    let (Artifact::Nir(package), _, _) = fixture.invoke("ready", &mut 100_000, 100_000).unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let generated = package
        .structs
        .iter()
        .find(|def| def.name == "Created")
        .unwrap();
    assert_eq!(
        generated.fields[0].1,
        nir::Term::Primitive(nir::Primitive::Bool)
    );
}

#[test]
fn builder_loop_retains_a_long_chain_of_generated_layouts() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        comptime fn Build() -> Type {{
            var current: Type = Original;
            var index = 0;
            while index < 256 {{
                current = .build_struct'builtin(.Layer, [.inner], [current]);
                index = index + 1;
            }}
            current
        }}"
    ));
    let (Artifact::Nir(package), Value::Type(mut ty), _) =
        fixture.invoke("Build", &mut 1_000_000, 1_000_000).unwrap()
    else {
        panic!("expected generated Type and NIR")
    };
    assert_eq!(package.structs.len(), 257);
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    let mut visited = std::collections::BTreeSet::new();
    for _ in 0..256 {
        let nir::Term::Nominal { declaration, .. } = &ty else {
            panic!("nominal layer")
        };
        assert!(visited.insert(*declaration));
        let layout = vm.reflect_struct(&ty).unwrap();
        assert_eq!(layout.fields.len(), 1);
        assert_eq!(layout.fields[0].name, "inner");
        ty = layout.fields[0].ty.clone();
    }
    assert_eq!(vm.reflect_struct(&ty).unwrap().fields[0].name, "flag");
}

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

#[test]
fn unexecuted_builder_paths_do_not_fork_hir_or_spend_structural_budget() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        comptime fn ready(.build:bool = false) -> bool {{
            if build {{ .build_struct'builtin(.Skipped, [.flag], [bool]); }}
            true
        }}
    "
    ));
    let mut structural = 0;
    let (Artifact::Nir(package), value, _) =
        fixture.invoke("ready", &mut structural, 1_000_000).unwrap()
    else {
        panic!()
    };
    assert_eq!(value, Value::Bool(true));
    assert_eq!(structural, 0);
    assert_eq!(package.structs.len(), 1);
    let mut structural = 12345;
    assert_eq!(
        fixture
            .invoke("ready", &mut structural, 1_000_000)
            .unwrap()
            .1,
        Value::Bool(true)
    );
    assert_eq!(structural, 12345);
}

#[test]
fn invalid_invocation_resources_do_not_consume_unused_builder_budget() {
    let fixture = Fixture::new(&format!(
        "{PREFIX} trait Duplicate {{}} comptime fn ready()->bool {{true}}"
    ));
    let mut structural = 12345;
    let error = compile_and_invoke_with_struct_builders(
        &fixture.packages(),
        &[],
        17,
        None,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            copy: Some("input.Duplicate"),
            ..Default::default()
        },
        None,
        &[],
        "ready",
        &mut structural,
        1_000_000,
    )
    .err()
    .unwrap();
    assert!(error.contains("missing Drop binding"), "{error}");
    assert_eq!(structural, 12345);
}

#[test]
fn type_computation_session_shares_generated_catalog_and_never_replays_publication() {
    use elaboration::type_computations::{Input, Session, State};
    use elaboration::{InferenceContext, InferenceTerm};
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        comptime fn ready() -> bool {{true}}
        comptime fn Generate() -> Type {{.build_struct'builtin(.Generated, [.flag], [bool])}}
        comptime fn Wrap(ty:Type) -> Type {{.build_struct'builtin(.Wrapped, [.inner], [ty])}}
        comptime fn Field(ty:Type) -> Type {{.struct_field_types'builtin(ty)[0]}}
        "
    ));
    let (Artifact::Nir(package), _, _) = fixture.invoke("ready", &mut 100_000, 100_000).unwrap()
    else {
        panic!("expected NIR")
    };
    let entry = |name: &str| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    let arena = hir::HirArena::new();
    let publisher = elaboration::definitions::StructPublisher::new(
        &arena,
        hir::Package::new(),
        0,
        std::collections::BTreeMap::from([(0, 901)]),
        Default::default(),
        None,
        100_000,
    )
    .unwrap();
    let host = |request: &comptime_interpretor::StructBuildRequest, steps: &mut u64| {
        publisher.prepare(request, steps)
    };
    let mut context = InferenceContext::default();
    let mut session =
        Session::new_with_struct_builder(&package, &context, Default::default(), &host).unwrap();
    let first = session
        .enqueue(&mut context, entry("Generate"), vec![])
        .unwrap();
    let second = session
        .enqueue(
            &mut context,
            entry("Wrap"),
            vec![Input {
                ty: InferenceTerm::Stable(nir::Term::Primitive(nir::Primitive::Type)),
                value: InferenceTerm::Meta(first),
            }],
        )
        .unwrap();
    assert!(matches!(
        session.resume(&mut context, second).unwrap(),
        State::Waiting(_)
    ));
    assert_eq!(publisher.published_definition_count(), 0);
    let first_ty = match session.resume(&mut context, first).unwrap() {
        State::Complete(ty) => ty.clone(),
        state => panic!("first generation failed: {state}"),
    };
    assert_eq!(publisher.published_definition_count(), 1);
    let second_ty = match session.resume(&mut context, second).unwrap() {
        State::Complete(ty) => ty.clone(),
        state => panic!("dependent generation failed: {state}"),
    };
    assert_ne!(first_ty, second_ty);
    assert_eq!(publisher.published_definition_count(), 2);
    let fuel = session.remaining_steps();
    assert_eq!(
        session.resume(&mut context, first).unwrap(),
        &State::Complete(first_ty.clone())
    );
    assert_eq!(
        session.resume(&mut context, second).unwrap(),
        &State::Complete(second_ty)
    );
    assert_eq!(session.remaining_steps(), fuel);
    assert_eq!(publisher.published_definition_count(), 2);
    let reflected = session
        .enqueue(
            &mut context,
            entry("Field"),
            vec![Input {
                ty: InferenceTerm::Stable(nir::Term::Primitive(nir::Primitive::Type)),
                value: InferenceTerm::Meta(second),
            }],
        )
        .unwrap();
    assert_eq!(
        session.resume(&mut context, reflected).unwrap(),
        &State::Complete(first_ty)
    );
}

#[test]
fn failed_type_computation_does_not_replay_builder_or_poison_independent_calls() {
    use elaboration::type_computations::{Input, Session, State};
    use elaboration::{InferenceContext, InferenceTerm};
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        comptime fn ready() -> bool {{true}}
        comptime fn Generate() -> Type {{.build_struct'builtin(.Generated, [.flag], [bool])}}
        comptime fn Wrap(ty:Type) -> Type {{.build_struct'builtin(.Wrapped, [.inner], [ty])}}
        "
    ));
    let (Artifact::Nir(package), _, _) = fixture.invoke("ready", &mut 100_000, 100_000).unwrap()
    else {
        panic!("expected NIR")
    };
    let entry = |name: &str| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    let arena = hir::HirArena::new();
    let publisher = elaboration::definitions::StructPublisher::new(
        &arena,
        hir::Package::new(),
        0,
        std::collections::BTreeMap::from([(0, 901)]),
        Default::default(),
        None,
        100_000,
    )
    .unwrap();
    let attempts = std::cell::Cell::new(0);
    let host = |request: &comptime_interpretor::StructBuildRequest, steps: &mut u64| {
        attempts.set(attempts.get() + 1);
        let mut prepared = publisher.prepare(request, steps)?;
        if attempts.get() == 1 {
            // Fail at the commit boundary, after VM admission. Both catalogs
            // must discard this candidate, and the failed waiter is terminal.
            prepared.commit = Box::new(|| {
                Err(comptime_interpretor::Error {
                    kind: comptime_interpretor::ErrorKind::InvalidCallPlan,
                    source: None,
                })
            });
        }
        Ok(prepared)
    };
    let mut context = InferenceContext::default();
    let mut session =
        Session::new_with_struct_builder(&package, &context, Default::default(), &host).unwrap();
    let first = session
        .enqueue(&mut context, entry("Generate"), vec![])
        .unwrap();
    assert!(matches!(
        session.resume(&mut context, first).unwrap(),
        State::Failed(_)
    ));
    assert_eq!(publisher.published_definition_count(), 0);
    let fuel = session.remaining_steps();
    assert!(matches!(
        session.resume(&mut context, first).unwrap(),
        State::Failed(_)
    ));
    assert_eq!(session.remaining_steps(), fuel);
    assert_eq!(attempts.get(), 1);
    let dependent = session
        .enqueue(
            &mut context,
            entry("Wrap"),
            vec![Input {
                ty: InferenceTerm::Stable(nir::Term::Primitive(nir::Primitive::Type)),
                value: InferenceTerm::Meta(first),
            }],
        )
        .unwrap();
    assert!(
        matches!(session.resume(&mut context, dependent).unwrap(), State::Failed(elaboration::type_computations::Error::DependencyFailed(id)) if *id == first)
    );
    assert_eq!(attempts.get(), 1);
    let repaired = session
        .enqueue(&mut context, entry("Generate"), vec![])
        .unwrap();
    assert!(matches!(
        session.resume(&mut context, repaired).unwrap(),
        State::Complete(_)
    ));
    assert_eq!(attempts.get(), 2);
    assert_eq!(publisher.published_definition_count(), 1);
}

#[test]
fn checked_snapshot_publisher_generates_from_source_types_without_reelaboration() {
    use elaboration::type_computations::{Session, State};
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        comptime fn ready() -> bool {{true}}
        comptime fn Generate() -> Type {{
            let first = .build_struct'builtin(.First, [.source], [Identity<Original>]);
            let second = .build_struct'builtin(.Second, [.nested], [first]);
            .struct_field_types'builtin(second)[0]
        }}"
    ));
    let (Artifact::Nir(package), _, _) = fixture.invoke("ready", &mut 100_000, 100_000).unwrap()
    else {
        panic!("expected NIR")
    };
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "Generate")
        .unwrap()
        .declaration;
    let publisher = elaboration::definitions::CheckedStructPublisher::new(&package);
    // This fixture reserves a separate declaration namespace explicitly. The
    // production caller must reserve identities from its full source world.
    let next = std::cell::Cell::new(0);
    let host = |request: &comptime_interpretor::StructBuildRequest, steps: &mut u64| {
        let local = next.get();
        next.set(local + 1);
        publisher.prepare(
            nir::DeclarationId {
                package: 901,
                local,
            },
            request,
            steps,
        )
    };
    let mut context = elaboration::InferenceContext::default();
    let mut session =
        Session::new_with_struct_builder(&package, &context, Default::default(), &host).unwrap();
    let result = session.enqueue(&mut context, entry, vec![]).unwrap();
    assert_eq!(
        session.resume(&mut context, result).unwrap(),
        &State::Complete(nir::Term::Nominal {
            declaration: nir::DeclarationId {
                package: 901,
                local: 0
            },
            arguments: vec![],
        })
    );
    assert_eq!(publisher.published_definition_count(), 2);
    assert_eq!(next.get(), 2);
    drop(session);
    let definitions = publisher.into_definitions();
    assert!(matches!(
        definitions[0][0].fields[0].1,
        nir::Term::NormalForm { .. }
    ));
    assert_eq!(
        definitions[1][0].fields[0].1,
        nir::Term::Nominal {
            declaration: definitions[0][0].declaration,
            arguments: vec![],
        }
    );
    assert_eq!(package.structs.len(), 1);
}

#[test]
fn required_builder_type_alias_can_be_constructed_and_extended_by_explicit_invocation() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        comptime fn Make() -> Type {{.build_struct'builtin(.Built, [.flag], [bool])}}
        typealias Built = Make();
        fn run() -> bool {{let value = Built {{flag:true}}; value.flag}}
        comptime fn More() -> Type {{.build_struct'builtin(.Outer, [.nested], [Built])}}"
    ));
    let (Artifact::Nir(package), Value::Type(outer), _) =
        fixture.invoke("More", &mut 100_000, 100_000).unwrap()
    else {
        panic!("expected generated Type and NIR")
    };
    assert_eq!(package.structs.len(), 3);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let built = package
        .structs
        .iter()
        .find(|def| def.name == "Built")
        .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.reflect_struct(&outer).unwrap().fields[0].ty,
        nir::Term::Nominal {
            declaration: built.declaration,
            arguments: vec![]
        }
    );
    let ids: std::collections::BTreeSet<_> =
        package.structs.iter().map(|def| def.declaration).collect();
    assert_eq!(ids.len(), 3);
    assert!(
        ids.iter()
            .all(|id| id.local < package.definition_identity_cursor)
    );
    assert!(package.to_string().contains("(definition-identity-cursor "));
}

#[test]
fn required_builder_constants_and_chained_aliases_keep_distinct_identities() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        comptime fn Make() -> Type {{.build_struct'builtin(.Built, [.flag], [bool])}}
        const FIRST = Make();
        typealias First = FIRST;
        typealias Second = Make();
        comptime fn Wrap(ty:Type) -> Type {{.build_struct'builtin(.Wrapped, [.item], [ty])}}
        typealias Wrapped = Wrap(First);
        comptime fn ready() -> bool {{true}}
        fn run() -> bool {{
            var value = Wrapped {{item: First {{flag:false}}}};
            value.item.flag = true;
            value.item.flag
        }}"
    ));
    let (Artifact::Nir(package), _, _) = fixture.invoke("ready", &mut 100_000, 100_000).unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(package.structs.len(), 4);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let definitions: Vec<_> = package
        .structs
        .iter()
        .filter(|def| def.name == "Built")
        .collect();
    assert_eq!(definitions.len(), 2);
    assert_ne!(definitions[0].declaration, definitions[1].declaration);
}

#[test]
fn required_builder_field_errors_are_diagnostics_not_missing_owner_panics() {
    for body in ["Built {}", "Built {flag:'x'}", "Built {unknown:true}"] {
        let fixture = Fixture::new(&format!(
            "{PREFIX}
            comptime fn Make() -> Type {{.build_struct'builtin(.Built, [.flag], [bool])}}
            typealias Built = Make();
            fn bad() {{{body}}}
            comptime fn ready() -> bool {{true}}"
        ));
        let error = fixture
            .invoke("ready", &mut 100_000, 100_000)
            .err()
            .unwrap();
        assert!(
            error.contains("field") || error.contains("type mismatch"),
            "{error}"
        );
    }
}

#[test]
fn public_builder_can_define_a_type_alias_from_reflected_fields() {
    let fixture = Fixture::new(
        "typealias Type=.Type'builtin;
        struct Source {first:bool, second:bool}
        comptime fn Columns() -> Type {
            let names = meta.struct_field_names(Source);
            let types = meta.struct_field_types(Source);
            let count = meta.length(names);
            var index = count - count;
            var builder = meta.StructBuilder.new(.name = .Columns);
            while index < count {
                builder.add_field(names[index], types[index]);
                index = index + 1;
            }
            builder.Build()
        }
        typealias Generated = Columns();
        trait ReadGenerated {fn first(self) -> bool;}
        impl ReadGenerated for Generated {fn first(self) -> bool {self.first}}
        comptime fn inspect() -> bool {
            let names=meta.struct_field_names(Generated);
            meta.length(names)==2 and names[0]==.first and names[1]==.second
        }
        fn run() -> bool {let value=Generated {first:true,second:false};value.as(ReadGenerated).first()}",
    );
    let (Artifact::Nir(package), observed, _) = fixture.invoke_meta("inspect").unwrap() else {
        panic!("expected NIR")
    };
    assert_eq!(observed, Value::Bool(true));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let generated: Vec<_> = package
        .structs
        .iter()
        .filter(|def| def.name == "Columns")
        .collect();
    assert_eq!(generated.len(), 1);
    assert_eq!(generated[0].declaration.package, 900);
}

#[test]
fn public_builder_operations_can_run_while_preparing_impl_subjects() {
    let bodies = [
        "meta.StructBuilder.new().Build()",
        "var builder = meta.StructBuilder.new(); builder.add_field(.flag, bool); builder.Build()",
        "let names = meta.struct_field_names(Source); Source",
        "let types = meta.struct_field_types(Source); Source",
        "var builder = meta.StructBuilder.new(); let names = meta.struct_field_names(Source); builder.add_field(names[0], bool); builder.Build()",
        "var builder = meta.StructBuilder.new(); let types = meta.struct_field_types(Source); builder.add_field(.flag, types[0]); builder.Build()",
    ];
    let mut failures = Vec::new();
    for body in bodies {
        let fixture = Fixture::new(&format!(
            "typealias Type = .Type'builtin;
            struct Source {{flag:bool}}
            comptime fn Generate() -> Type {{{body}}}
            typealias Generated = Generate();
            impl Generated {{fn ping(*self) -> bool {{true}}}}
            comptime fn inspect() -> bool {{true}}"
        ));
        if let Err(failure) = fixture.invoke_meta("inspect") {
            failures.push(format!("{body}: {failure}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn early_public_builder_reflects_nested_and_applied_source_types() {
    for source in ["Source", "Generic<Inner>"] {
        let fixture = Fixture::new(&format!(
            "typealias Type = .Type'builtin;
            struct Inner {{flag:bool}}
            struct Source {{inner:Inner}}
            struct Generic where T {{inner:T}}
            comptime fn Generate() -> Type {{
                let types = meta.struct_field_types({source});
                var builder = meta.StructBuilder.new();
                builder.add_field(.inner, types[0]);
                builder.Build()
            }}
            typealias Generated = Generate();
            impl Generated {{fn read(self) -> bool {{self.inner.flag}}}}
            fn run() -> bool {{Generated {{inner:Inner {{flag:true}}}}.read()}}
            comptime fn inspect() -> bool {{true}}"
        ));
        let (Artifact::Nir(package), _, _) = fixture
            .invoke_meta("inspect")
            .unwrap_or_else(|error| panic!("{source}: {error}"))
        else {
            panic!("expected NIR")
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn early_public_builder_reflects_computed_nominal_applications() {
    for (declaration, application, expected_calls) in [
        ("struct Source where T {inner:Inner}", "Source<ty>", 0),
        (
            "struct Source where T, value:T {inner:Inner}",
            "Source<ty, true>",
            0,
        ),
        (
            "struct Source where T {inner:Inner}",
            "Source<do {calls = calls + 1; ty}>",
            1,
        ),
        (
            "struct Source where T, value:T {inner:Inner}",
            "Source<do {calls = calls * 10 + 1; ty}, do {calls = calls * 10 + 2; true}>",
            12,
        ),
    ] {
        let fixture = Fixture::new(&format!(
            "typealias Type = .Type'builtin;
            struct Inner {{flag:bool}}
            {declaration}
            comptime fn Generate() -> Type {{
                let ty = bool;
                var calls: .u32'builtin = 0;
                let types = meta.struct_field_types({application});
                let builder = meta.fold(types, meta.StructBuilder.new(), |draft, field| {{
                    var draft = draft;
                    draft.add_field(.inner, field);
                    draft
                }});
                if calls == {expected_calls} {{builder.Build()}} else {{bool}}
            }}
            typealias Generated = Generate();
            impl Generated {{fn read(self) -> bool {{self.inner.flag}}}}
            fn run() -> bool {{Generated {{inner:Inner {{flag:true}}}}.read()}}
            comptime fn inspect() -> bool {{true}}"
        ));
        let (Artifact::Nir(package), _, _) = fixture
            .invoke_meta("inspect")
            .unwrap_or_else(|error| panic!("{application}: {error}"))
        else {
            panic!("expected NIR")
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn effectful_reflection_indexing_preserves_order_and_bounds() {
    for position in ["0", "1", "-1"] {
        let fixture = Fixture::new(&format!(
            "typealias Type = .Type'builtin;
            struct Inner {{flag:bool}}
            struct Source where T {{inner:Inner}}
            comptime fn Generate() -> Type {{
                var calls: .u32'builtin = 0;
                let subject = Source<do {{calls = calls + 1; bool}}>;
                let types = meta.struct_field_types(subject);
                let names = meta.struct_field_names(subject);
                var builder = meta.StructBuilder.new();
                builder.add_field(
                    names[do {{calls = calls * 10 + 2; 0}}],
                    types[do {{calls = calls * 10 + 3; {position}}}]);
                if calls == 123 {{builder.Build()}} else {{bool}}
            }}
            typealias Generated = Generate();
            impl Generated {{fn read(self) -> bool {{self.inner.flag}}}}
            fn run() -> bool {{Generated {{inner:Inner {{flag:true}}}}.read()}}
            comptime fn inspect() -> bool {{true}}"
        ));
        let result = fixture.invoke_meta("inspect");
        if position != "0" {
            let failure = result.err().expect("out-of-range index must fail");
            assert!(failure.contains("IndexOutOfBounds"), "{failure}");
            continue;
        }
        let (Artifact::Nir(package), _, _) = result.unwrap() else {
            panic!("expected NIR")
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn comptime_list_index_early_return_preserves_prior_effects() {
    for (domain, value) in [
        ("Type", "bool"),
        (".Symbol'builtin", ".ready"),
        ("bool", "true"),
        (".u8'builtin", "7"),
    ] {
        let fixture = Fixture::new(&format!(
            "typealias Type = .Type'builtin;
            comptime fn inspect() -> bool {{
                let values:meta.List<{domain}> = [{value}];
                var calls: .u8'builtin = 0;
                let ignored = values[do {{calls = calls + 1; return calls == 1;}}];
                var unreachable: .u8'builtin = 255;
                unreachable = unreachable + 1;
                false
            }}"
        ));
        let (_, observed, _) = fixture.invoke_meta("inspect").unwrap();
        assert_eq!(observed, Value::Bool(true));
    }
    for (domain, value) in [("Type", "bool"), (".Symbol'builtin", ".ready")] {
        let fixture = Fixture::new(&format!(
            "typealias Type = .Type'builtin;
            comptime fn inspect() -> bool {{
                let values:meta.List<{domain}> = [{value}];
                var calls: .u8'builtin = 0;
                let ignored = (if calls == 0 {{
                    calls = calls + 1;
                    return calls == 1;
                }} else {{values}})[do {{
                    var unreachable: .u8'builtin = 255;
                    unreachable = unreachable + 1;
                    0
                }}];
                false
            }}"
        ));
        let (_, observed, _) = fixture.invoke_meta("inspect").unwrap();
        assert_eq!(observed, Value::Bool(true));
    }
}

#[test]
fn reflected_type_index_operands_still_require_machine_arithmetic_proofs() {
    for observation in [
        "types[do {calls = calls + 1; 0}]",
        "(do {calls = calls + 1; types})[0]",
    ] {
        let fixture = Fixture::new(&format!(
            "typealias Type = .Type'builtin;
            comptime fn inspect() -> bool {{
                let types:meta.List<Type> = [bool];
                var calls: .u8'builtin = 255;
                let observed = {observation};
                true
            }}"
        ));
        let failure = fixture
            .invoke_meta("inspect")
            .err()
            .expect("overflow must fail");
        assert!(failure.contains("range"), "{failure}");
    }
}

#[test]
fn early_public_builder_preserves_recursive_pointer_field_identity() {
    let fixture = Fixture::new(
        "typealias Type = .Type'builtin;
        struct Node {next:?*Node}
        struct Source {node:*Node}
        comptime fn Generate() -> Type {
            var builder = meta.StructBuilder.new();
            builder.add_field(.node, meta.struct_field_types(Source)[0]);
            builder.Build()
        }
        typealias Generated = Generate();
        impl Generated {fn marker(*self) -> bool {true}}
        comptime fn inspect() -> bool {
            meta.struct_field_types(Generated)[0] == *Node
        }",
    );
    let (_, observed, _) = fixture.invoke_meta("inspect").unwrap();
    assert_eq!(observed, Value::Bool(true));
}

#[test]
fn deferred_annotations_publish_once_per_formed_domain() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        comptime fn Domain(flag:bool)->Type {{
            let generated=.build_struct'builtin(if flag {{.TrueDomain}} else {{.FalseDomain}}, [.flag], [bool]);
            bool
        }}
        fn keep(comptime flag:bool,value:defer Domain(flag))->bool {{value}}
        fn yes()->bool {{keep(true,true)}}
        fn no()->bool {{keep(false,false)}}
        comptime fn ready()->bool {{true}}"
    ));
    let (Artifact::Nir(package), _, _) = fixture.invoke("ready", &mut 100_000, 100_000).unwrap()
    else {
        panic!("expected NIR")
    };
    for (entry, expected) in [("yes", true), ("no", false)] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(expected)
        );
    }
    for name in ["TrueDomain", "FalseDomain"] {
        assert_eq!(
            package
                .structs
                .iter()
                .filter(|def| def.name == name)
                .count(),
            1
        );
    }
}

#[test]
fn deferred_annotation_can_reflect_a_type_generated_after_its_snapshot() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        comptime fn Field(ty:Type)->Type {{.struct_field_types'builtin(ty)[0]}}
        fn keep(comptime T:Type,value:defer Field(T))->bool {{value}}
        comptime fn Make()->Type {{.build_struct'builtin(.Late, [.flag], [bool])}}
        fn run()->bool {{keep(Make(),true)}}
        comptime fn ready()->bool {{true}}"
    ));
    let (Artifact::Nir(package), _, _) = fixture.invoke("ready", &mut 100_000, 100_000).unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        package
            .structs
            .iter()
            .filter(|def| def.name == "Late")
            .count(),
        1
    );
}

#[test]
fn staged_default_generates_fresh_types_and_explicit_input_skips_it() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        comptime fn Make()->Type {{.build_struct'builtin(.DefaultType, [.flag], [bool])}}
        fn create(comptime flag:bool,comptime .T:Type=Make())->bool {{flag}}
        fn generated()->bool {{create(true)}}
        fn generated_again()->bool {{create(false)}}
        fn supplied()->bool {{create(false,.T=Original)}}
        comptime fn ready()->bool {{true}}"
    ));
    let (Artifact::Nir(package), _, _) = fixture.invoke("ready", &mut 100_000, 100_000).unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "generated", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "supplied", 100_000).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        luna::execution::interpret(&package, "generated_again", 100_000).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        package
            .structs
            .iter()
            .filter(|def| def.name == "DefaultType")
            .count(),
        2
    );
}

#[test]
fn computed_constructor_publishes_its_type_once() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        comptime fn Make()->Type {{.build_struct'builtin(.Computed, [.flag], [bool])}}
        fn run()->bool {{let value=Make() {{flag:true}};value.flag}}
        comptime fn ready()->bool {{true}}"
    ));
    let (Artifact::Nir(package), _, _) = fixture.invoke("ready", &mut 100_000, 100_000).unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        package
            .structs
            .iter()
            .filter(|def| def.name == "Computed")
            .count(),
        1
    );
}

#[test]
fn generated_nominals_remain_distinct_in_fresh_type_requirement_contexts() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        comptime fn Make()->Type {{.build_struct'builtin(.Fresh, [.flag], [bool])}}
        typealias First=Make();
        typealias Second=Make();
        fn distinct(comptime A:Type,comptime B:Type)->bool where requires A != B {{true}}
        fn run()->bool {{distinct(First,Original) and distinct(First,Second)}}
        comptime fn ready()->bool {{true}}"
    ));
    let (Artifact::Nir(package), _, _) = fixture.invoke("ready", &mut 100_000, 100_000).unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generated_type_default_does_not_authorize_fields_on_an_unconstrained_type_parameter() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        comptime fn Make()->Type {{.build_struct'builtin(.Default, [.flag], [bool])}}
        fn invalid(comptime .T:Type=Make())->defer T {{T {{flag:true}}}}
        comptime fn ready()->bool {{true}}"
    ));
    let error = fixture
        .invoke("ready", &mut 100_000, 100_000)
        .err()
        .unwrap();
    assert!(
        error.contains("construction requires a supported struct type"),
        "{error}"
    );
}

#[test]
fn explicit_staged_type_input_skips_a_failing_builder_default() {
    let source = format!(
        "{PREFIX}
        comptime fn Bad()->Type {{.build_struct'builtin(.Duplicate, [.flag,.flag], [bool,bool])}}
        fn choose(comptime .T:Type=Bad())->bool {{true}}
        comptime fn ready()->bool {{true}}"
    );
    let explicit = Fixture::new(&format!("{source} fn run()->bool {{choose(.T=Original)}}"));
    let (Artifact::Nir(package), _, _) = explicit.invoke("ready", &mut 100_000, 100_000).unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(package.structs.len(), 1);
    let omitted = Fixture::new(&format!("{source} fn run()->bool {{choose()}}"));
    assert!(omitted.invoke("ready", &mut 100_000, 100_000).is_err());
}

#[test]
fn body_generated_types_are_available_during_quote_purity_validation() {
    let fixture = Fixture::new(&format!(
        "{PREFIX}
        trait Copy {{}} trait Drop {{fn drop(*self);}} impl Copy for bool {{}}
        comptime fn Make()->Type {{.build_struct'builtin(.BodyType, [.flag], [bool])}}
        pure quote fn identity(value:bool)->bool {{value}}
        pure fn run()->bool {{let value=Make() {{flag:true}};identity(value.flag)}}
        comptime fn ready()->bool {{true}}"
    ));
    let (Artifact::Nir(package), _, _) = compile_and_invoke_with_struct_builders(
        &fixture.packages(),
        &[],
        17,
        None,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            copy: Some("input.Copy"),
            drop: Some("input.Drop"),
            ..Default::default()
        },
        None,
        &[],
        "ready",
        &mut 100_000,
        100_000,
    )
    .unwrap() else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        package
            .structs
            .iter()
            .filter(|def| def.name == "BodyType")
            .count(),
        1
    );
}

#[test]
fn public_enum_builder_publishes_payload_shapes_and_runtime_matching() {
    let fixture = Fixture::new("
        comptime fn EventType() -> .Type'builtin {
            var builder = meta.EnumBuilder.new(.name = .GeneratedEvent);
            builder.add_unit(.idle);
            builder.add_tuple(.data, [bool]);
            builder.add_named(.record, [.flag], [bool]);
            builder.Build()
        }
        typealias Event = EventType();
        fn read(value: Event) -> bool {
            value match {.idle => false, .data(flag) => flag, .record {flag: flag} => flag}
        }
        fn run() -> bool {read(Event.data(true)) and read(Event.record {flag: true}) and not read(Event.idle)}
        comptime fn ready() -> bool {true}
    ");
    let (Artifact::Nir(package), _, _) = fixture.invoke_meta("ready").unwrap() else {
        panic!()
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let generated = package
        .enums
        .iter()
        .find(|item| item.name == "GeneratedEvent")
        .unwrap();
    assert_eq!(generated.declaration.package, 900);
    assert_eq!(generated.variants.len(), 3);
    assert_eq!(
        generated.variants[0].shape,
        nir::program::VariantShape::Unit
    );
    assert_eq!(
        generated.variants[1].shape,
        nir::program::VariantShape::Tuple
    );
    assert_eq!(
        generated.variants[2].shape,
        nir::program::VariantShape::Named(vec!["flag".into()])
    );
}

#[test]
fn explicit_invocation_returns_generated_enum_and_mixed_structs() {
    let fixture = Fixture::new(
        "
        comptime fn Generate() -> .Type'builtin {
            var inner = meta.StructBuilder.new(.name = .Inner);
            inner.add_field(.flag, bool);
            let payload = inner.Build();
            var builder = meta.EnumBuilder.new(.name = .GeneratedChoice);
            builder.add_tuple(.some, [payload]);
            builder.add_unit(.none);
            let choice = builder.Build();
            var outer = meta.StructBuilder.new(.name = .Outer);
            outer.add_field(.choice, choice);
            outer.Build()
        }
    ",
    );
    let (Artifact::Nir(package), Value::Type(result), _) = fixture.invoke_meta("Generate").unwrap()
    else {
        panic!()
    };
    let inner = package
        .structs
        .iter()
        .find(|item| item.name == "Inner")
        .unwrap();
    let choice = package
        .enums
        .iter()
        .find(|item| item.name == "GeneratedChoice")
        .unwrap();
    let outer = package
        .structs
        .iter()
        .find(|item| item.name == "Outer")
        .unwrap();
    assert_ne!(inner.declaration, choice.declaration);
    assert_ne!(outer.declaration, choice.declaration);
    assert_eq!(
        choice.variants[0].fields,
        [nir::Term::Nominal {
            declaration: inner.declaration,
            arguments: vec![]
        }]
    );
    assert_eq!(
        outer.fields[0].1,
        nir::Term::Nominal {
            declaration: choice.declaration,
            arguments: vec![]
        }
    );
    assert_eq!(
        result,
        nir::Term::Nominal {
            declaration: outer.declaration,
            arguments: vec![]
        }
    );
}

#[test]
fn enum_builder_rejects_malformed_drafts_and_runtime_use() {
    for body in [
        "builder.add_unit(.same); builder.add_unit(.same);",
        "builder.add_named(.item, [.flag], []);",
        "builder.add_named(.item, [.flag, .flag], [bool, bool]);",
        "builder.add_group(.same); builder.add_unit(.same);",
    ] {
        let fixture = Fixture::new(&format!(
            "comptime fn Generate() -> .Type'builtin {{var builder = meta.EnumBuilder.new(); {body} builder.Build()}}"
        ));
        assert!(fixture.invoke_meta("Generate").is_err(), "accepted {body}");
    }
    let fixture = Fixture::new(
        "fn bad() -> .Type'builtin {meta.EnumBuilder.new().Build()} comptime fn ready() -> bool {true}",
    );
    assert!(fixture.invoke_meta("ready").is_err());
}

#[test]
fn enum_builtin_preserves_early_return_and_requires_comptime() {
    let fixture = Fixture::new(&format!("{PREFIX}
        comptime fn early() -> bool {{.build_enum'builtin(do {{return true;}}, [], [], [], [], []); false}}
    "));
    assert_eq!(
        fixture.invoke("early", &mut 100_000, 100_000).unwrap().1,
        Value::Bool(true)
    );
    for source in [
        "fn bad() -> Type {.build_enum'builtin(.Generated, [], [], [], [], [])}",
        "comptime fn Bad() -> Type {.build_enum'builtin(.Generated, [])}",
        "comptime fn Bad() -> Type {.build_enum'builtin(.Generated, [], [.item], [.unit], [[]], [[true]])}",
    ] {
        let fixture = Fixture::new(&format!(
            "{PREFIX}{source} comptime fn ready() -> bool {{true}}"
        ));
        assert!(
            fixture.invoke("ready", &mut 100_000, 100_000).is_err(),
            "accepted {source}"
        );
    }
}

#[test]
fn generated_enum_missing_and_unknown_fields_are_diagnostics() {
    let prefix = "comptime fn Make() -> .Type'builtin {var builder = meta.EnumBuilder.new(); builder.add_named(.record, [.flag], [bool]); builder.Build()} typealias Event = Make();";
    for body in [
        "Event.record {}",
        "Event.record {other:true}",
        "Event.record {flag:true, flag:false}",
    ] {
        let fixture = Fixture::new(&format!(
            "{prefix} fn bad() -> Event {{{body}}} comptime fn ready() -> bool {{true}}"
        ));
        let failure = fixture.invoke_meta("ready").err().unwrap();
        assert!(failure.contains("field"), "{failure}");
    }
}

#[test]
fn public_enum_builder_preserves_nested_group_paths_and_prefix_boundaries() {
    let fixture = Fixture::new("
        comptime fn EventType() -> .Type'builtin {
            var builder = meta.EnumBuilder.new(.name = .GroupedEvent);
            builder.add_group(.net);
            builder.add_group(.nested, .parent = [.net]);
            builder.add_group(.network);
            builder.add_group(.empty);
            builder.add_unit(.ready, .group = [.net]);
            builder.add_tuple(.data, [bool], .group = [.net, .nested]);
            builder.add_named(.record, [.flag], [bool], .group = [.network]);
            builder.Build()
        }
        typealias Event = EventType();
        fn inside(value: Event) -> bool {value match {.net => true, .network => false, .empty => false}}
        fn read(value: Event) -> bool {value match {.net.ready => false, .net.nested.data(flag) => flag, .network.record {flag: flag} => flag}}
        fn run() -> bool {inside(Event.net.ready) and inside(.net.nested.data(true))
            and not inside(.network.record {flag:true}) and read(.net.nested.data(true)) and read(.network.record {flag:true})}
        comptime fn ready() -> bool {true}
    ");
    let (Artifact::Nir(package), _, _) = fixture.invoke_meta("ready").unwrap() else {
        panic!()
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let generated = package
        .enums
        .iter()
        .find(|item| item.name == "GroupedEvent")
        .unwrap();
    assert_eq!(generated.groups, ["net", "net.nested", "network", "empty"]);
    assert_eq!(
        generated
            .variants
            .iter()
            .map(|variant| variant.name.as_str())
            .collect::<Vec<_>>(),
        ["net.ready", "net.nested.data", "network.record"]
    );
}

#[test]
fn grouped_enum_builder_rejects_missing_parents_and_name_collisions() {
    for body in [
        "builder.add_unit(.leaf, .group = [.missing]);",
        "builder.add_group(.nested, .parent = [.missing]); builder.add_unit(.leaf);",
        "builder.add_group(.group); builder.add_group(.group); builder.add_unit(.leaf);",
        "builder.add_group(.group); builder.add_unit(.group);",
        "builder.add_group(.group); builder.add_unit(.leaf, .group = [.group]); builder.add_tuple(.leaf, [bool], .group = [.group]);",
    ] {
        let fixture = Fixture::new(&format!(
            "comptime fn Generate() -> .Type'builtin {{var builder = meta.EnumBuilder.new(); {body} builder.Build()}}"
        ));
        assert!(fixture.invoke_meta("Generate").is_err(), "accepted {body}");
    }
}

#[test]
fn code_generator_reflects_its_published_struct_to_select_runtime_code() {
    let fixture = Fixture::new(
        "
        comptime fn generate() -> <|-bool> {
            var builder = meta.StructBuilder.new(.name = .QuoteLocal);
            builder.add_field(.flag, bool);
            let T = builder.Build();
            let fields = meta.struct_field_names(T);
            if meta.length(fields) == 1 and fields[0] == .flag {
                return '(true);
            }
            '(false)
        }
        fn run() -> bool {~<generate()>}
        comptime fn ready() -> bool {true}
    ",
    );
    let (Artifact::Nir(package), _, _) = fixture.invoke_meta("ready").unwrap() else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn struct_builder_copies_instantiated_fields_and_extends_the_draft() {
    let fixture = Fixture::new(
        "
        struct Source where T {first:T,second:bool}
        comptime fn Mirror() -> .Type'builtin {
            var builder = meta.StructBuilder.from_type(Source<bool>, .name = .Mirrored);
            builder.add_field(.extra, bool);
            builder.Build()
        }
        typealias Mirrored = Mirror();
        fn run() -> bool {
            let value = Mirrored {first:true,second:false,extra:true};
            value.first and not value.second and value.extra
        }
        comptime fn ready() -> bool {true}
    ",
    );
    let (Artifact::Nir(package), _, _) = fixture.invoke_meta("ready").unwrap() else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let copied = package
        .structs
        .iter()
        .find(|item| item.name == "Mirrored")
        .unwrap();
    assert!(copied.type_parameters.is_empty());
    assert_eq!(copied.fields.len(), 3);
}

#[test]
fn struct_builder_copies_live_types_once_and_keeps_drafts_independent() {
    let fixture = Fixture::new(
        "
        comptime fn Subject(visited:*bool) -> .Type'builtin {
            visited.* = not visited.*;
            var builder = meta.StructBuilder.new(.name = .Source);
            builder.add_field(.first, bool);
            builder.Build()
        }
        comptime fn run() -> bool {
            var visited = false;
            var draft = meta.StructBuilder.from_type(Subject(visited.ref));
            let saved = draft;
            draft.add_field(.second, bool);
            let extended = draft.Build();
            let original = saved.Build();
            let copied = meta.StructBuilder.from_type(original).Build();
            let names = meta.struct_field_names(extended);
            let types = meta.struct_field_types(copied);
            visited and original != copied and original != extended
                and meta.length(meta.struct_field_names(original)) == 1
                and meta.length(names) == 2 and names[0] == .first and names[1] == .second
                and types[0] == bool
        }
    ",
    );
    assert_eq!(fixture.invoke_meta("run").unwrap().1, Value::Bool(true));
}

#[test]
fn struct_builder_copy_rejects_invalid_subjects_and_duplicate_extensions() {
    for source in [
        "comptime fn bad() -> .Type'builtin {meta.StructBuilder.from_type(bool).Build()}",
        "enum E {one} comptime fn bad() -> .Type'builtin {meta.StructBuilder.from_type(E).Build()}",
        "struct S {flag:bool} comptime fn bad() -> .Type'builtin {var b = meta.StructBuilder.from_type(S); b.add_field(.flag,bool); b.Build()}",
        "struct S {} fn bad() -> meta.StructBuilder {meta.StructBuilder.from_type(S)}",
    ] {
        let fixture = Fixture::new(source);
        assert!(fixture.invoke_meta("bad").is_err(), "accepted {source}");
    }
}

#[test]
fn enum_builder_copies_instantiated_source_structure_and_extends_it() {
    let fixture = Fixture::new("
        enum Source where T {net.{idle,data(T),record {value:T}},stopped,empty.{}}
        comptime fn Mirror() -> .Type'builtin {
            var builder = meta.EnumBuilder.from_type(Source<bool>, .name = .Mirrored);
            builder.add_unit(.extra);
            builder.Build()
        }
        typealias Mirrored = Mirror();
        fn read(value:Mirrored) -> bool {value match {
            .net.idle => false, .net.data(value) => value, .net.record {value:value} => value,
            .stopped => false, .extra => true,
        }}
        fn run() -> bool {read(Mirrored.net.data(true)) and read(Mirrored.net.record {value:true}) and read(.extra) and not read(.stopped)}
        comptime fn ready() -> bool {true}
    ");
    let (Artifact::Nir(package), _, _) = fixture.invoke_meta("ready").unwrap() else {
        panic!()
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let copied = package
        .enums
        .iter()
        .find(|item| item.name == "Mirrored")
        .unwrap();
    assert_eq!(copied.groups, ["net", "empty"]);
    assert_eq!(
        copied.variants[1].fields,
        [nir::Term::Primitive(nir::Primitive::Bool)]
    );
    assert!(copied.type_parameters.is_empty());
}

#[test]
fn enum_reflection_observes_live_generated_types_and_preserves_nominal_identity() {
    let fixture = Fixture::new("
        comptime fn run() -> bool {
            var builder = meta.EnumBuilder.new(.name = .First);
            builder.add_group(.net);
            builder.add_named(.data, [.flag], [bool], .group = [.net]);
            let first = builder.Build();
            let paths = meta.enum_variant_paths(first);
            let groups = meta.enum_group_paths(first);
            let shapes = meta.enum_variant_shapes(first);
            let names = meta.enum_variant_field_names(first);
            let types = meta.enum_variant_field_types(first);
            let copy = meta.EnumBuilder.from_type(first, .name = .Copied).Build();
            first != copy and paths[0][0] == .net and paths[0][1] == .data
                and groups[0][0] == .net and shapes[0] == .named and names[0][0] == .flag and types[0][0] == bool
        }
    ");
    assert_eq!(fixture.invoke_meta("run").unwrap().1, Value::Bool(true));
}

#[test]
fn enum_reflection_rejects_wrong_subjects_and_runtime_calls_but_propagates_return() {
    for call in [
        "meta.enum_group_paths",
        "meta.enum_variant_paths",
        "meta.enum_variant_shapes",
        "meta.enum_variant_field_names",
        "meta.enum_variant_field_types",
    ] {
        let fixture = Fixture::new(&format!(
            "comptime fn bad() -> bool {{let ignored = {call}(bool); true}}"
        ));
        assert!(fixture.invoke_meta("bad").is_err(), "accepted {call}(bool)");
        let fixture = Fixture::new(&format!(
            "enum E {{one}} fn bad() -> bool {{let ignored = {call}(E); true}} comptime fn ready() -> bool {{true}}"
        ));
        assert!(fixture.invoke_meta("ready").is_err(), "runtime {call}");
    }
    let fixture = Fixture::new(&format!(
        "{PREFIX} comptime fn early() -> bool {{.enum_group_paths'builtin(do {{return true;}}); false}}"
    ));
    assert_eq!(
        fixture.invoke("early", &mut 100_000, 100_000).unwrap().1,
        Value::Bool(true)
    );
}

#[test]
fn enum_reflection_evaluates_each_subject_once() {
    for call in [
        "meta.enum_group_paths",
        "meta.enum_variant_paths",
        "meta.enum_variant_shapes",
        "meta.enum_variant_field_names",
        "meta.enum_variant_field_types",
    ] {
        let fixture = Fixture::new(&format!("
            enum Source {{one}}
            comptime fn Subject(flag:*bool) -> .Type'builtin {{flag.* = not flag.*; Source}}
            comptime fn run() -> bool {{var visited = false; let observed = {call}(Subject(visited.ref)); visited}}
        "));
        assert_eq!(
            fixture.invoke_meta("run").unwrap().1,
            Value::Bool(true),
            "{call}"
        );
    }
}

#[test]
fn public_builder_uses_invocation_ranges_for_distinct_reflected_subjects() {
    let fixture = Fixture::new(
        r#"
        typealias Type=.Type'builtin;
        struct First {flag:bool,letter:.char'builtin}
        struct Second {letter:.char'builtin}
        comptime fn Columns(subject:Type,name:.Symbol'builtin)->Type {
            let names=meta.struct_field_names(subject);
            let types=meta.struct_field_types(subject);
            var builder=meta.StructBuilder.new(.name=name);
            inline for index in 0..meta.length(names) {
                builder.add_field(names[index],types[index]);
            }
            builder.Build()
        }
        typealias FirstColumns=Columns(First,.FirstColumns);
        typealias SecondColumns=Columns(Second,.SecondColumns);
        comptime fn inspect()->bool {
            let first=meta.struct_field_types(FirstColumns);
            let second=meta.struct_field_types(SecondColumns);
            meta.length(first)==2 and first[0]==bool and first[1]==.char'builtin
                and meta.length(second)==1 and second[0]==.char'builtin
        }
        fn run()->bool {
            let first=FirstColumns {flag:true,letter:'x'};
            let second=SecondColumns {letter:'y'};
            first.flag and first.letter=='x' and second.letter=='y'
        }
    "#,
    );
    let (Artifact::Nir(package), observed, _) = fixture.invoke_meta("inspect").unwrap() else {
        panic!("expected NIR")
    };
    assert_eq!(observed, Value::Bool(true));
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let generated: Vec<_> = package
        .structs
        .iter()
        .filter(|item| item.name == "FirstColumns" || item.name == "SecondColumns")
        .collect();
    assert_eq!(generated.len(), 2);
    assert_ne!(generated[0].declaration, generated[1].declaration);
    assert!(generated.iter().all(|item| item.declaration.package == 900));
}

#[test]
fn struct_builder_edits_preserve_order_snapshots_and_failed_bounds() {
    let fixture = Fixture::new(
        "typealias Type=.Type'builtin;
        struct Source {old:bool,removed:bool}
        comptime fn inspect()->bool {
            var builder=meta.StructBuilder.from_type(Source);
            let snapshot=builder;
            let replaced=builder.replace_field(0,.value,.u8'builtin);
            let removed=builder.remove_field(1);
            let inserted=builder.insert_field(0,.first,bool);
            let appended=builder.insert_field(2,.last,bool);
            let negative=builder.insert_field(-1,.bad,bool);
            let far=builder.remove_field(99999999999999999999999999999999999999999999999);
            let past=builder.replace_field(3,.bad,bool);
            let result=builder.Build();
            let old=snapshot.Build();
            let names=meta.struct_field_names(result);
            let types=meta.struct_field_types(result);
            let old_names=meta.struct_field_names(old);
            replaced and removed and inserted and appended
                and not negative and not far and not past
                and meta.length(names)==3 and names[0]==.first and names[1]==.value
                and names[2]==.last and types[0]==bool and types[1]==.u8'builtin
                and types[2]==bool and meta.length(old_names)==2
                and old_names[0]==.old and old_names[1]==.removed
        }",
    );
    let (_, result, _) = fixture.invoke_meta("inspect").unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn struct_builder_can_edit_empty_drafts_and_remove_every_field() {
    let fixture = Fixture::new(
        "comptime fn inspect()->bool {
            var builder=meta.StructBuilder.new();
            let missing=builder.remove_field(0);
            let replacement=builder.replace_field(0,.bad,bool);
            let inserted=builder.insert_field(0,.only,bool);
            let removed=builder.remove_field(0);
            let empty=builder.Build();
            let reinserted=builder.insert_field(0,.new,bool);
            let filled=builder.Build();
            not missing and not replacement and inserted and removed and reinserted
                and meta.length(meta.struct_field_names(empty))==0
                and meta.struct_field_names(filled)[0]==.new
        }",
    );
    let (_, result, _) = fixture.invoke_meta("inspect").unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn struct_builder_edits_do_not_bypass_duplicate_field_validation() {
    for edit in [
        "builder.insert_field(1,.first,bool)",
        "builder.replace_field(1,.first,bool)",
    ] {
        let fixture = Fixture::new(&format!(
            "struct Source {{first:bool,second:bool}}
            comptime fn draft()->meta.StructBuilder {{
                var builder=meta.StructBuilder.from_type(Source);
                let accepted={edit};
                assumes accepted;
                builder
            }}
            comptime fn Make()->.Type'builtin {{draft().Build()}}
            comptime fn repaired()->bool {{
                var builder=draft();
                let fixed=builder.replace_field(1,.unique,bool);
                let result=builder.Build();
                fixed and meta.struct_field_names(result)[1]==.unique
            }}"
        ));
        let error = fixture.invoke_meta("Make").err().unwrap();
        assert!(
            error.contains("duplicate-field") && error.contains("(name \"first\")"),
            "{error}"
        );
        assert_eq!(
            fixture.invoke_meta("repaired").unwrap().1,
            Value::Bool(true)
        );
    }
}

#[test]
fn builder_diagnostics_identify_malformed_source_drafts() {
    for (body, expected) in [
        (
            ".build_struct'builtin(.Bad,[.first,.second],[bool])",
            "(field-count (names 2) (types 1))",
        ),
        (
            "var b=meta.EnumBuilder.new();b.add_unit(.ready);b.add_unit(.ready);b.Build()",
            "(duplicate-variant (name \"ready\"))",
        ),
        (
            "var b=meta.EnumBuilder.new();b.add_named(.data,[.flag,.flag],[bool,bool]);b.Build()",
            "(duplicate-field (owner \"data\") (name \"flag\"))",
        ),
        (
            "var b=meta.EnumBuilder.new();b.add_named(.data,[.flag],[bool,bool]);b.Build()",
            "(variant-shape (name \"data\") (shape \"named\") (names 1) (types 2))",
        ),
    ] {
        let fixture = Fixture::new(&format!("comptime fn Bad()->.Type'builtin {{{body}}}"));
        let error = fixture.invoke_meta("Bad").err().unwrap();
        assert!(error.contains(expected), "expected {expected}: {error}");
        assert!(error.contains("at (source "), "{error}");
    }
}

#[test]
fn enum_builder_edits_payloads_groups_and_runtime_patterns() {
    let fixture = Fixture::new(&format!(
        "typealias Type=.Type'builtin; typealias u8=.u8'builtin; {}",
        include_str!("../../../development/examples/sysroot-edited-enum.fl")
    ));
    let (Artifact::Nir(package), result, _) = fixture.invoke_meta("inspect_enum").unwrap() else {
        panic!("expected NIR")
    };
    assert_eq!(result, Value::Bool(true));
    assert_eq!(
        luna::execution::interpret(&package, "exercise_enum", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn enum_builder_edits_preserve_snapshots_and_reject_out_of_bounds_indices() {
    let fixture = Fixture::new("enum Source {group.{old(bool)}}
        comptime fn inspect()->bool {
            var builder=meta.EnumBuilder.from_type(Source);
            let snapshot=builder;
            let published=builder.Build();
            let negative=builder.remove_variant(-1);
            let far=builder.insert_unit(99999999999999999999999999999999999999999999999,.bad);
            let past=builder.replace_tuple(1,.bad,[bool]);
            let group_negative=builder.insert_group(-1,.bad);
            let group_past=builder.replace_group(1,.bad);
            let group_far=builder.remove_group(99999999999999999999999999999999999999999999999);
            let changed=builder.replace_named(0,.new,[.flag],[bool],.group=[.group]);
            let newer=builder.Build();
            let saved=snapshot.Build();
            let unit_changed=builder.replace_unit(0,.empty,.group=[.group]);
            let unit=builder.Build();
            not negative and not far and not past and not group_negative and not group_past and not group_far
                and unit_changed and meta.enum_variant_shapes(unit)[0]==.unit
                and meta.length(meta.enum_variant_field_names(unit)[0])==0
                and meta.length(meta.enum_variant_field_types(unit)[0])==0
                and changed and published!=newer and saved!=published and saved!=newer
                and meta.enum_variant_paths(published)[0][1]==.old
                and meta.enum_variant_paths(saved)[0][1]==.old
                and meta.enum_variant_shapes(saved)[0]==.tuple
                and meta.length(meta.enum_variant_field_names(saved)[0])==0
                and meta.length(meta.enum_variant_paths(newer))==1
                and meta.length(meta.enum_group_paths(newer))==1
                and meta.enum_variant_paths(newer)[0][1]==.new
                and meta.enum_variant_shapes(newer)[0]==.named
                and meta.enum_variant_field_names(newer)[0][0]==.flag
        }");
    assert_eq!(fixture.invoke_meta("inspect").unwrap().1, Value::Bool(true));
}

#[test]
fn enum_builder_can_remove_every_variant_and_repopulate_the_draft() {
    let fixture = Fixture::new("comptime fn inspect()->bool {
        var builder=meta.EnumBuilder.new();
        let missing=builder.remove_variant(0);
        let group_missing=builder.replace_group(0,.bad);
        let replacement=builder.replace_unit(0,.bad);
        let first=builder.insert_tuple(0,.only,[bool]);
        let removed=builder.remove_variant(0);
        let empty=builder.Build();
        let added=builder.insert_group(0,.group);
        let inserted=builder.insert_named(0,.value,[.flag],[bool],.group=[.group]);
        let filled=builder.Build();
        not missing and not group_missing and not replacement and first and removed and added and inserted
            and meta.length(meta.enum_variant_paths(empty))==0
            and meta.length(meta.enum_group_paths(empty))==0
            and meta.enum_variant_paths(filled)[0][0]==.group
            and meta.enum_variant_field_types(filled)[0][0]==bool
    }");
    assert_eq!(fixture.invoke_meta("inspect").unwrap().1, Value::Bool(true));
}

#[test]
fn enum_builder_edits_require_valid_payloads_and_explicit_group_repair() {
    for (edit, repair) in [
        (
            "builder.replace_unit(1,.one,.group=[.group])",
            "builder.replace_unit(1,.two)",
        ),
        (
            "builder.replace_named(0,.one,[.flag],[bool,bool],.group=[.group])",
            "builder.replace_named(0,.one,[.flag],[bool],.group=[.group])",
        ),
        (
            "builder.replace_group(0,.renamed)",
            "builder.replace_tuple(0,.one,[bool],.group=[.renamed])",
        ),
        (
            "builder.remove_group(0)",
            "builder.replace_tuple(0,.one,[bool])",
        ),
        ("builder.insert_group(1,.group)", "builder.remove_group(1)"),
    ] {
        let fixture = Fixture::new(&format!("enum Source {{group.{{one(bool)}},two}}
            comptime fn draft()->meta.EnumBuilder {{var builder=meta.EnumBuilder.from_type(Source); let edited={edit}; assumes edited; builder}}
            comptime fn Make()->.Type'builtin {{draft().Build()}}
            comptime fn repaired()->bool {{var builder=draft(); let fixed={repair}; assumes fixed; let result=builder.Build(); meta.length(meta.enum_variant_paths(result))==2}}"));
        assert!(
            fixture.invoke_meta("Make").is_err(),
            "accepted invalid edit {edit}"
        );
        assert_eq!(
            fixture.invoke_meta("repaired").unwrap().1,
            Value::Bool(true),
            "{edit}"
        );
    }
}

#[test]
fn enum_builder_nested_group_edits_preserve_opaque_payload_types() {
    let fixture = Fixture::new(
        "pure comptime fn Identity<T> -> .Type'builtin {T}
        comptime fn inspect()->bool {
            var builder=meta.EnumBuilder.new();
            let root=builder.insert_group(0,.root);
            let child=builder.insert_group(1,.child,.parent=[.root]);
            let inserted=builder.insert_tuple(0,.data,[Identity<bool>],.group=[.root,.child]);
            let old=builder.Build();
            let renamed=builder.replace_group(1,.branch,.parent=[.root]);
            let moved=builder.replace_tuple(0,.data,[Identity<bool>],.group=[.root,.branch]);
            let edited=builder.Build();
            let groups=meta.enum_group_paths(edited);
            let paths=meta.enum_variant_paths(edited);
            let types=meta.enum_variant_field_types(edited);
            root and child and inserted and renamed and moved
                and groups[1][0]==.root and groups[1][1]==.branch
                and paths[0][0]==.root and paths[0][1]==.branch and paths[0][2]==.data
                and types[0][0]==Identity<bool>
                and meta.enum_variant_paths(old)[0][1]==.child
        }",
    );
    let (Artifact::Nir(package), result, _) = fixture.invoke_meta("inspect").unwrap() else {
        panic!("expected NIR")
    };
    assert_eq!(result, Value::Bool(true));
    let generated: Vec<_> = package
        .enums
        .iter()
        .filter(|item| item.name == "Generated")
        .collect();
    assert_eq!(generated.len(), 2);
    for item in generated {
        let nir::Term::NormalForm {
            head, arguments, ..
        } = &item.variants[0].fields[0]
        else {
            panic!("payload head must remain opaque")
        };
        assert!(
            package
                .normal_forms
                .iter()
                .any(|item| item.declaration == *head)
        );
        assert_eq!(arguments, &vec![nir::Term::Primitive(nir::Primitive::Bool)]);
    }
}
