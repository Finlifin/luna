use luna::{
    build_context::{Error, Host, INPUT_VERSION, PackageBuild, Query, Snapshot, Target, Value},
    packages::SourcePackage,
};
use nir::{DeclarationId, target::TargetData};

fn sources() -> Vec<SourcePackage> {
    [(3, 100, "app"), (7, 200, "dependency")]
        .into_iter()
        .map(|(resolution_id, identity, name)| SourcePackage {
            resolution_id,
            identity,
            name: name.into(),
            entry: format!("{name}/lib.fl").into(),
        })
        .collect()
}
fn input(identity: u64, mode: &str) -> PackageBuild {
    PackageBuild {
        identity,
        mode: mode.into(),
        execution_host: Host {
            arch: "aarch64".into(),
            os: "linux".into(),
        },
        host_pipeline_target: Target {
            arch: "x86_64".into(),
            os: "windows".into(),
            layout: TargetData::new(64).unwrap(),
            features: [("avx2".into(), false)].into(),
        },
    }
}
fn definition(package: u64) -> DeclarationId {
    DeclarationId { package, local: 42 }
}
fn snapshot(inputs: Vec<PackageBuild>) -> Snapshot {
    Snapshot::new(INPUT_VERSION, &sources(), inputs).unwrap()
}

#[test]
fn source_owner_selects_configuration_and_execution_host_does_not_replace_target() {
    let world = snapshot(vec![input(100, "debug"), input(200, "release")]);
    assert_eq!(
        world.read(definition(100), Query::Mode).unwrap().value(),
        &Value::Symbol("debug".into())
    );
    assert_eq!(
        world.read(definition(200), Query::Mode).unwrap().value(),
        &Value::Symbol("release".into())
    );
    assert_eq!(
        world
            .read(definition(200), Query::TargetArch)
            .unwrap()
            .value(),
        &Value::Symbol("x86_64".into())
    );
    assert_eq!(
        world
            .read(definition(200), Query::ExecutionHostArch)
            .unwrap()
            .value(),
        &Value::Symbol("aarch64".into())
    );
    assert_eq!(
        world
            .read(definition(200), Query::TargetOs)
            .unwrap()
            .value(),
        &Value::Symbol("windows".into())
    );
    assert_eq!(
        world
            .read(definition(200), Query::ExecutionHostOs)
            .unwrap()
            .value(),
        &Value::Symbol("linux".into())
    );
    assert_eq!(
        world
            .read(definition(200), Query::TargetPointerBits)
            .unwrap()
            .value(),
        &Value::PointerBits(64)
    );
    // Resolution ID 7 is not the semantic identity 200.
    assert_eq!(
        world.read(definition(7), Query::Mode),
        Err(Error::MissingPackage(7))
    );
}

#[test]
fn observations_invalidate_only_changed_owner_and_queried_inputs() {
    let original = snapshot(vec![input(100, "debug"), input(200, "release")]);
    let mode = original.read(definition(200), Query::Mode).unwrap();
    let arch = original.read(definition(200), Query::TargetArch).unwrap();
    let feature = original
        .read(definition(200), Query::TargetFeature("avx2".into()))
        .unwrap();
    let missing = original
        .read(definition(200), Query::TargetFeature("sse2".into()))
        .unwrap();
    let mut changed = input(200, "release");
    changed.execution_host.arch = "riscv64".into();
    changed
        .host_pipeline_target
        .features
        .insert("sse2".into(), false);
    let next = snapshot(vec![input(100, "release"), changed.clone()]);
    assert!(next.unchanged(&mode));
    assert!(next.unchanged(&arch));
    assert!(next.unchanged(&feature));
    assert!(!next.unchanged(&missing));
    changed.mode = "debug".into();
    changed
        .host_pipeline_target
        .features
        .insert("avx2".into(), true);
    let final_world = snapshot(vec![input(100, "debug"), changed]);
    assert!(!final_world.unchanged(&mode));
    assert!(!final_world.unchanged(&feature));
    assert!(final_world.unchanged(&arch));
    assert!(original.unchanged(&mode));
    assert_eq!(missing.value(), &Value::Feature(None));
    assert_eq!(feature.value(), &Value::Feature(Some(false)));
    let removed = Snapshot::new(INPUT_VERSION, &sources()[..1], vec![input(100, "debug")]).unwrap();
    assert!(!removed.unchanged(&mode));
}

#[test]
fn snapshot_rejects_ambiguous_incomplete_and_unsupported_inputs() {
    let source = sources();
    assert!(matches!(
        Snapshot::new(2, &source, vec![]),
        Err(Error::Version(2))
    ));
    for (inputs, expected) in [
        (vec![input(100, "debug")], Error::MissingPackage(200)),
        (
            vec![input(100, "debug"), input(100, "release")],
            Error::DuplicateInput(100),
        ),
        (vec![input(7, "debug")], Error::UnknownPackage(7)),
    ] {
        assert_eq!(
            Snapshot::new(INPUT_VERSION, &source, inputs).unwrap_err(),
            expected
        );
    }
    let mut aliases = source.clone();
    aliases[1].identity = 100;
    assert!(matches!(
        Snapshot::new(INPUT_VERSION, &aliases, vec![]),
        Err(Error::DuplicatePackage(100))
    ));
    aliases = source.clone();
    aliases[1].resolution_id = 3;
    assert!(matches!(
        Snapshot::new(INPUT_VERSION, &aliases, vec![]),
        Err(Error::DuplicateResolution(3))
    ));
    let mut empty = input(100, "");
    assert_eq!(
        Snapshot::new(INPUT_VERSION, &source, vec![empty.clone()]).unwrap_err(),
        Error::EmptyName {
            owner: 100,
            field: "mode"
        }
    );
    empty.mode = "debug".into();
    empty.host_pipeline_target.features.insert("".into(), true);
    assert_eq!(
        Snapshot::new(INPUT_VERSION, &source, vec![empty]).unwrap_err(),
        Error::EmptyName {
            owner: 100,
            field: "target.feature"
        }
    );
}

#[test]
fn dumps_are_stable_escaped_and_balanced() {
    let mut first = input(100, "debug\n\"");
    first
        .host_pipeline_target
        .features
        .insert("z\\feature".into(), true);
    let world = snapshot(vec![input(200, "release"), first.clone()]);
    assert_eq!(
        world.to_string(),
        snapshot(vec![first, input(200, "release")]).to_string()
    );
    let text = world.to_string();
    assert!(text.contains("(mode \"debug\\n\\\"\")"), "{text}");
    assert!(text.contains("(\"z\\\\feature\" true)"), "{text}");
    assert_eq!(
        text.chars().filter(|c| *c == '(').count(),
        text.chars().filter(|c| *c == ')').count()
    );
    assert_eq!(
        world
            .read(definition(200), Query::TargetFeature("avx2".into()))
            .unwrap()
            .to_string(),
        "(build-read (owner 200) (query (target-feature \"avx2\")) (value (feature (present false))))"
    );
}

struct SourceFixture(std::path::PathBuf);
impl SourceFixture {
    fn new(app: &str, dependency: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "luna-build-mode-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("app.fl"), app).unwrap();
        std::fs::write(dir.join("dependency.fl"), dependency).unwrap();
        Self(dir)
    }
    fn compile(&self, modes: &[(u64, &str)]) -> Result<luna::frontend::Artifact, String> {
        let packages = vec![
            SourcePackage {
                resolution_id: 3,
                identity: 100,
                name: "app".into(),
                entry: self.0.join("app.fl"),
            },
            SourcePackage {
                resolution_id: 7,
                identity: 200,
                name: "dependency".into(),
                entry: self.0.join("dependency.fl"),
            },
            SourcePackage {
                resolution_id: 9,
                identity: 300,
                name: "build".into(),
                entry: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../library/build/lib.fl"),
            },
        ];
        let mut protocols = elaboration::program::Protocols::default();
        protocols.build_modes = modes
            .iter()
            .map(|(id, mode)| (*id, (*mode).into()))
            .collect();
        let dependencies = vec![
            resolve::DependencyEdge {
                requester: 3,
                alias: "build".into(),
                provider: 9,
            },
            resolve::DependencyEdge {
                requester: 7,
                alias: "build".into(),
                provider: 9,
            },
            resolve::DependencyEdge {
                requester: 3,
                alias: "dependency".into(),
                provider: 7,
            },
        ];
        luna::packages::compile_with_inputs(
            &packages,
            &dependencies,
            3,
            None,
            &protocols,
            Default::default(),
        )
        .map(|(artifact, _)| artifact)
    }
}
impl Drop for SourceFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn source_build_mode_preserves_definition_origin_aliases_and_constants() {
    let fixture = SourceFixture::new(
        "use build.mode as selected; const own=selected;
        comptime fn run()->bool {(own matches .debug) and dependency.release()}",
        "pub const saved=build.mode;
        pub comptime fn release()->bool {(saved matches .release) and (build.mode matches .release)}",
    );
    let luna::frontend::Artifact::Nir(package) = fixture
        .compile(&[(100, "debug"), (200, "release"), (300, "provider")])
        .unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let luna::frontend::Artifact::Nir(changed) = fixture
        .compile(&[(100, "release"), (200, "release"), (300, "debug")])
        .unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&changed, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(false)
    );
}

#[test]
fn source_build_mode_never_falls_back_to_caller_or_provider() {
    let fixture = SourceFixture::new(
        "comptime fn run()->bool {dependency.ready()}",
        "pub comptime fn ready()->bool {build.mode matches .debug}",
    );
    let error = fixture
        .compile(&[(100, "debug"), (300, "debug")])
        .err()
        .unwrap();
    assert!(
        error.contains("build mode is unavailable for source package 200"),
        "{error}"
    );
    assert!(fixture.compile(&[(100, "debug"), (200, "release")]).is_ok());
}

#[test]
fn source_build_mode_keeps_normal_shadowing_and_runtime_stage_checks() {
    let shadow = SourceFixture::new(
        "mod build {pub const mode=.local;} comptime fn run()->bool {build.mode matches .local}",
        "",
    );
    let luna::frontend::Artifact::Nir(package) = shadow.compile(&[]).unwrap() else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let runtime = SourceFixture::new("fn run()->bool {build.mode matches .debug}", "");
    let error = runtime.compile(&[(100, "debug")]).err().unwrap();
    assert!(error.contains("Splice"), "{error}");
}

#[test]
fn inline_configuration_branch_uses_dependency_mode() {
    let fixture = SourceFixture::new(
        "fn run()->bool {dependency.selected()}",
        "pub inline fn selected()->bool {inline if build.mode matches .release {true} else {false}}",
    );
    let luna::frontend::Artifact::Nir(package) = fixture
        .compile(&[(100, "debug"), (200, "release")])
        .unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn installing_snapshot_modes_removes_stale_bindings() {
    let mut protocols = elaboration::program::Protocols::default();
    protocols.build_modes.insert(999, "stale".into());
    let world = snapshot(vec![input(100, "debug"), input(200, "release")]);
    world.configure_modes(&mut protocols);
    assert_eq!(
        protocols.build_modes,
        [(100, "debug".into()), (200, "release".into())].into()
    );
}
