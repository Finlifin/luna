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
        let mut protocols = elaboration::program::Protocols::default();
        protocols.build_modes = modes
            .iter()
            .map(|(id, mode)| (*id, (*mode).into()))
            .collect();
        self.compile_configured(&protocols, None)
    }
    fn compile_configured(
        &self,
        protocols: &elaboration::program::Protocols,
        target: Option<TargetData>,
    ) -> Result<luna::frontend::Artifact, String> {
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
        luna::packages::compile_with_definitions(
            &packages,
            &dependencies,
            3,
            None,
            protocols,
            Default::default(),
            target,
            &mut 100_000,
            &mut |_, _, _| Ok(()),
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

#[test]
fn cli_mode_inputs_are_explicit_and_reject_ambiguous_forms() {
    let parse = |args: &[&str]| luna::cli::parse(args.iter().map(std::ffi::OsString::from));
    let luna::cli::Command::Compile(options) = parse(&[
        "input.fl",
        "--emit",
        "nir",
        "--build-mode",
        "debug",
        "--package-build-mode",
        "200=release",
    ])
    .unwrap() else {
        panic!("expected compile")
    };
    assert_eq!(options.build_modes.primary.as_deref(), Some("debug"));
    assert_eq!(
        options.build_modes.packages.get(&200).map(String::as_str),
        Some("release")
    );
    for args in [
        vec!["input.fl", "--emit", "nir", "--build-mode", ""],
        vec![
            "input.fl",
            "--emit",
            "nir",
            "--build-mode",
            "debug",
            "--build-mode",
            "release",
        ],
        vec![
            "input.fl",
            "--emit",
            "nir",
            "--package-build-mode",
            "-1=debug",
        ],
        vec!["input.fl", "--emit", "nir", "--package-build-mode", "1="],
        vec![
            "input.fl",
            "--emit",
            "nir",
            "--package-build-mode",
            "1=debug",
            "--package-build-mode",
            "01=release",
        ],
        vec!["input.fl", "--emit", "hir", "--build-mode", "debug"],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
    let mut modes = luna::build_context::ModeOptions::default();
    modes.set_primary("debug".into()).unwrap();
    modes.add_package("100=release").unwrap();
    assert!(
        modes
            .bind([100], 100, &Default::default())
            .unwrap_err()
            .contains("conflicts")
    );
}

#[test]
fn package_cli_and_stream_transport_mode_and_invalidate_reuse() {
    let fixture = SourceFixture::new(
        "fn run()->bool {inline if build.mode matches .debug {true} else {false}}",
        "",
    );
    let build = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/build/lib.fl");
    let base = vec![
        "--package".to_string(),
        "3".into(),
        "100".into(),
        "app".into(),
        fixture.0.join("app.fl").to_str().unwrap().into(),
        "--package".into(),
        "9".into(),
        "300".into(),
        "build".into(),
        build.to_str().unwrap().into(),
        "--dependency".into(),
        "3".into(),
        "build".into(),
        "9".into(),
        "--primary".into(),
        "3".into(),
        "--emit".into(),
        "nir".into(),
    ];
    let mut frames = String::new();
    for mode in [
        Some("debug"),
        Some("debug"),
        Some("release"),
        None,
        Some("debug"),
    ] {
        let mut args = base.clone();
        if let Some(mode) = mode {
            args.extend(["--build-mode".into(), mode.into()]);
        }
        frames.push_str(
            &args
                .iter()
                .map(|arg| {
                    arg.as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join(" "),
        );
        frames.push('\n');
    }
    let mut output = Vec::new();
    luna::package_stream::serve(&mut std::io::Cursor::new(frames), &mut output, 4).unwrap();
    let output = String::from_utf8(output).unwrap();
    let replies = output.lines().collect::<Vec<_>>();
    assert_eq!(replies.len(), 5);
    for index in [0, 1, 2, 4] {
        assert!(
            replies[index].contains("(exit-code 0)"),
            "{}",
            replies[index]
        );
    }
    assert!(replies[0].contains("(reused false)"));
    assert!(replies[1].contains("(reused true)"));
    assert!(replies[2].contains("(reused false)"));
    assert!(
        replies[3].contains("(exit-code 1)") && replies[3].contains("source package 100"),
        "{}",
        replies[3]
    );
    assert!(replies[4].contains("(reused true)"));
}

fn target_protocols() -> elaboration::program::Protocols {
    let mut app = input(100, "debug");
    app.host_pipeline_target
        .features
        .insert("sse2".into(), true);
    let mut dependency = input(200, "release");
    dependency.host_pipeline_target.arch = "wasm32".into();
    dependency.host_pipeline_target.os = "wasi".into();
    dependency.host_pipeline_target.layout = TargetData::new(32).unwrap();
    let mut protocols = elaboration::program::Protocols::default();
    snapshot(vec![app, dependency]).configure(&mut protocols);
    protocols
}

#[test]
fn target_values_preserve_origin_aliases_and_optional_feature_states() {
    let fixture = SourceFixture::new(
        "use build.target as selected;
        const saved = selected;
        comptime fn run()->bool {
            let target = saved;
            (target.arch matches .x86_64) and (target.os matches .windows)
            and target.pointer_bits == 64
            and (target.features.get(.sse2) matches true?)
            and (target.features.get(.avx2) matches false?)
            and (target.features.get(.unknown) matches null)
            and dependency.ready()
        }",
        "pub const saved = build.target;
        pub comptime fn ready()->bool {
            (saved.arch matches .wasm32) and (build.target.os matches .wasi)
            and saved.pointer_bits == 32
        }",
    );
    let luna::frontend::Artifact::Nir(package) = fixture
        .compile_configured(&target_protocols(), None)
        .unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn target_reads_reject_missing_origin_and_conflicting_layout() {
    let fixture = SourceFixture::new(
        "comptime fn run()->bool { dependency.ready() }",
        "pub comptime fn ready()->bool {build.target.arch matches .wasm32}",
    );
    let mut protocols = target_protocols();
    protocols.build_targets.remove(&200);
    let error = fixture.compile_configured(&protocols, None).err().unwrap();
    assert!(
        error.contains("build target is unavailable for source package 200"),
        "{error}"
    );
    protocols = target_protocols();
    let error = fixture
        .compile_configured(&protocols, Some(TargetData::new(64).unwrap()))
        .err()
        .unwrap();
    assert!(error.contains("build target layout conflicts"), "{error}");
    assert!(
        fixture
            .compile_configured(&protocols, Some(TargetData::new(32).unwrap()))
            .is_ok()
    );
}

#[test]
fn target_values_keep_shadowing_and_runtime_splice_checks() {
    let fixture = SourceFixture::new(
        "mod build {pub mod target {pub const arch = .local;}}
        comptime fn run()->bool {build.target.arch matches .local}",
        "",
    );
    assert!(
        fixture
            .compile_configured(&Default::default(), None)
            .is_ok()
    );
    let fixture = SourceFixture::new("fn run()->bool {build.target.arch matches .x86_64}", "");
    let error = fixture
        .compile_configured(&target_protocols(), None)
        .err()
        .unwrap();
    assert!(error.contains("Splice"), "{error}");
}

#[test]
fn installing_snapshot_replaces_targets_and_preserves_stable_dump() {
    let mut protocols = target_protocols();
    let old = protocols.build_targets[&100].clone();
    protocols.build_targets.insert(999, old.clone());
    snapshot(vec![input(100, "release"), input(200, "debug")]).configure(&mut protocols);
    assert_eq!(protocols.build_targets.len(), 2);
    assert!(!protocols.build_targets[&100].features.contains_key("sse2"));
    let mut escaped = old;
    escaped.arch = "a\"\n".into();
    let dump = escaped.to_string();
    assert!(dump.contains("a\\\"\\n"), "{dump}");
    assert!(dump.find("avx2").unwrap() < dump.find("sse2").unwrap());
}

#[test]
fn real_sysroot_target_branches_execute_at_o0_and_o3() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let luna::cli::Command::Compile(options) = luna::cli::parse([
        root.join("development/examples/sysroot-build-target.fl")
            .into_os_string(),
        "--sysroot".into(),
        root.join("library").into_os_string(),
        "--emit".into(),
        "nir".into(),
    ])
    .unwrap() else {
        panic!("expected compilation")
    };
    let mut protocols = elaboration::program::Protocols::default();
    let target = luna::frontend::native_test_target().expect("native target available");
    assert_eq!(
        target.pointer_bits(),
        64,
        "fixture expects a 64-bit native target"
    );
    protocols.build_targets.insert(
        100,
        elaboration::program::build_context::Target {
            arch: "configured_arch".into(),
            os: "configured_os".into(),
            layout: target,
            features: [("vector".into(), true), ("disabled".into(), false)].into(),
        },
    );
    let (luna::frontend::Artifact::Nir(package), bindings) =
        luna::frontend::compile_bound_for_target(
            &options,
            100,
            &protocols,
            Default::default(),
            Some(target),
        )
        .unwrap()
    else {
        panic!("expected NIR")
    };
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        let report = luna::testing::run_native_report_filtered(
            &package,
            100,
            100_000,
            Some(&bindings),
            None,
            &luna::host::Toolchain {
                optimization,
                ..Default::default()
            },
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(report.results.len(), 2, "{report}");
        assert_eq!(report.failures(), 0, "{report}");
    }
}

#[test]
fn inline_target_branches_keep_dependency_configuration() {
    let fixture = SourceFixture::new(
        "fn run()->bool {dependency.ready()}",
        "pub inline fn ready()->bool {
            inline if (build.target.arch matches .wasm32)
                and (build.target.features.get(.avx2) matches false?) {true} else {false}
        }",
    );
    let luna::frontend::Artifact::Nir(package) = fixture
        .compile_configured(&target_protocols(), None)
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
fn source_target_rejects_unvalidated_empty_driver_names() {
    let fixture = SourceFixture::new(
        "comptime fn run()->bool {build.target.arch matches .x86_64}",
        "",
    );
    for field in ["arch", "os", "feature"] {
        let mut protocols = target_protocols();
        let target = protocols.build_targets.get_mut(&100).unwrap();
        match field {
            "arch" => target.arch.clear(),
            "os" => target.os.clear(),
            _ => {
                target.features.insert(String::new(), true);
            }
        }
        let error = fixture.compile_configured(&protocols, None).err().unwrap();
        assert!(error.contains("empty configuration name"), "{error}");
    }
}

#[test]
fn cli_targets_validate_identity_features_and_stage_without_order_dependence() {
    let parse = |extra: &[&str]| {
        luna::cli::parse(
            ["input.fl", "--emit", "nir"]
                .into_iter()
                .chain(extra.iter().copied())
                .map(std::ffi::OsString::from),
        )
    };
    let luna::cli::Command::Compile(options) = parse(&[
        "--build-feature",
        "avx2=false",
        "--build-target",
        "x86_64",
        "linux",
        "64",
        "--package-build-feature",
        "200",
        "simd=true",
        "--package-build-target",
        "200",
        "aarch64",
        "other",
        "64",
    ])
    .unwrap() else {
        panic!("expected compile")
    };
    let bindings = options
        .build_targets
        .bind([100, 200], 100, &Default::default())
        .unwrap();
    assert_eq!(
        bindings.build_targets[&100].features.get("avx2"),
        Some(&false)
    );
    assert_eq!(
        bindings.build_targets[&200].features.get("simd"),
        Some(&true)
    );
    assert_eq!(bindings.build_targets[&200].arch, "aarch64");
    assert_eq!(
        options.build_targets.resolve_layout(100, None).unwrap(),
        Some(TargetData::new(64).unwrap())
    );
    assert!(
        options
            .build_targets
            .resolve_layout(100, Some(TargetData::new(32).unwrap()))
            .is_err()
    );
    // Reapplying an identical snapshot is permitted, conflicting data is not.
    assert!(
        options
            .build_targets
            .bind([100, 200], 100, &bindings)
            .is_ok()
    );
    let mut conflicting = bindings.clone();
    conflicting
        .build_targets
        .get_mut(&100)
        .unwrap()
        .features
        .clear();
    assert!(
        options
            .build_targets
            .bind([100, 200], 100, &conflicting)
            .unwrap_err()
            .contains("conflicts with driver snapshot")
    );
    assert!(
        options
            .build_targets
            .bind([100], 100, &Default::default())
            .unwrap_err()
            .contains("unknown semantic package identity 200")
    );
    for invalid in [
        vec!["--build-target", "x86_64", "linux"],
        vec!["--build-target", "", "linux", "64"],
        vec!["--build-target", "x86_64", "", "64"],
        vec!["--build-target", "x86_64", "linux", "0"],
        vec!["--build-target", "x86_64", "linux", "65"],
        vec!["--build-target", "x86_64", "linux", "256"],
        vec!["--build-target", "x86_64", "linux", "+64"],
        vec!["--build-feature", "simd=true"],
        vec!["--build-feature", "=true"],
        vec!["--build-feature", "simd=yes"],
        vec![
            "--build-target",
            "x",
            "y",
            "64",
            "--build-feature",
            "simd=true",
            "--build-feature",
            "simd=false",
        ],
        vec![
            "--build-target",
            "x",
            "y",
            "64",
            "--build-target",
            "x",
            "y",
            "64",
        ],
        vec!["--package-build-target", "-1", "x", "y", "64"],
        vec![
            "--package-build-target",
            "18446744073709551616",
            "x",
            "y",
            "64",
        ],
        vec![
            "--package-build-target",
            "1",
            "x",
            "y",
            "64",
            "--package-build-target",
            "01",
            "x",
            "y",
            "64",
        ],
        vec![
            "--build-target",
            "x",
            "y",
            "64",
            "--package-build-feature",
            "200",
            "simd=true",
        ],
    ] {
        assert!(parse(&invalid).is_err(), "{invalid:?}");
    }
    assert!(
        luna::cli::parse(
            ["input.fl", "--build-target", "x", "y", "64"].map(std::ffi::OsString::from)
        )
        .is_err()
    );
    let luna::cli::Command::Compile(dual) = parse(&[
        "--build-target",
        "x",
        "y",
        "64",
        "--package-build-target",
        "100",
        "x",
        "y",
        "64",
    ])
    .unwrap() else {
        panic!("expected compile")
    };
    assert!(
        dual.build_targets
            .bind([100], 100, &Default::default())
            .is_err()
    );
}

#[test]
fn resident_target_inputs_recompile_changes_and_recover_after_missing_config() {
    let fixture = SourceFixture::new(
        "fn run()->bool {inline if build.target.features.get(.simd) matches true? {true}else{false}}",
        "",
    );
    let build = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/build/lib.fl");
    let base = vec![
        "--package".to_string(),
        "3".into(),
        "100".into(),
        "app".into(),
        fixture.0.join("app.fl").to_str().unwrap().into(),
        "--package".into(),
        "9".into(),
        "300".into(),
        "build".into(),
        build.to_str().unwrap().into(),
        "--dependency".into(),
        "3".into(),
        "build".into(),
        "9".into(),
        "--primary".into(),
        "3".into(),
        "--emit".into(),
        "nir".into(),
    ];
    let mut frames = String::new();
    for feature in [
        Some("true"),
        Some("true"),
        Some("false"),
        None,
        Some("true"),
    ] {
        let mut args = base.clone();
        if let Some(feature) = feature {
            args.extend([
                "--package-build-feature".into(),
                "100".into(),
                format!("simd={feature}"),
                "--package-build-target".into(),
                "100".into(),
                "arch".into(),
                "os".into(),
                "64".into(),
            ]);
        }
        frames.push_str(
            &args
                .iter()
                .map(|arg| {
                    arg.as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join(" "),
        );
        frames.push('\n');
    }
    let mut output = Vec::new();
    luna::package_stream::serve(&mut std::io::Cursor::new(frames), &mut output, 4).unwrap();
    let output = String::from_utf8(output).unwrap();
    let replies: Vec<_> = output.lines().collect();
    assert_eq!(replies.len(), 5);
    for index in [0, 1, 2, 4] {
        assert!(
            replies[index].contains("(exit-code 0)"),
            "{}",
            replies[index]
        );
    }
    assert!(replies[0].contains("(reused false)"));
    assert!(replies[1].contains("(reused true)"));
    assert!(replies[2].contains("(reused false)"));
    assert!(
        replies[3].contains("(exit-code 1)") && replies[3].contains("source package 100"),
        "{}",
        replies[3]
    );
    assert!(replies[4].contains("(reused true)"));
}

#[test]
fn template_defaults_keep_provider_context_and_instance_bodies_keep_consumer_context() {
    for category in ["fn", "inline fn", "quote fn"] {
        let fixture = SourceFixture::new(
            &format!(
                "{category} selected use dependency.signature {{
                value and (inline if build.mode matches .debug {{true}} else {{false}})
            }}
            fn run()->bool {{ selected() }}"
            ),
            "pub template fn signature(.value:bool=inline if build.mode matches .release {true}else{false})->bool;",
        );
        let luna::frontend::Artifact::Nir(package) = fixture
            .compile(&[(100, "debug"), (200, "release")])
            .unwrap_or_else(|error| panic!("{category}: {error}"))
        else {
            panic!("expected NIR")
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true),
            "{category}"
        );
        let error = fixture
            .compile(&[(100, "debug"), (300, "release")])
            .err()
            .unwrap();
        assert!(error.contains("source package 200"), "{category}: {error}");
    }
}

#[test]
fn template_target_defaults_keep_source_configuration() {
    let fixture = SourceFixture::new(
        "fn selected use dependency.signature {value} fn run()->bool {selected()}",
        "pub template fn signature(.value:bool=inline if build.target.arch matches .wasm32 {true}else{false})->bool;",
    );
    let luna::frontend::Artifact::Nir(package) = fixture
        .compile_configured(&target_protocols(), None)
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
fn quote_functions_and_generated_code_keep_provider_configuration() {
    for (app, dependency) in [
        (
            "fn run()->bool {dependency.selected()}",
            "pub quote fn selected()->bool {inline if build.mode matches .release {true}else{false}}",
        ),
        (
            "fn run()->bool {const generated=dependency.code(); ~<generated>}",
            "pub comptime fn code()-><|-bool> {'(inline if build.mode matches .release {true}else{false})}",
        ),
        (
            "fn run()->bool {dependency.selected(inline if build.mode matches .debug {true}else{false})}",
            "pub quote fn selected(value:bool)->bool {inline if build.mode matches .release {value}else{false}}",
        ),
        (
            "fn run()->bool {const generated=dependency.code('(inline if build.mode matches .debug {true}else{false})); ~<generated>}",
            "pub comptime fn code(input:<|-bool>)-><|-bool> {'(~<input> and (inline if build.mode matches .release {true}else{false}))}",
        ),
    ] {
        let fixture = SourceFixture::new(app, dependency);
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
        let error = fixture.compile(&[(100, "debug")]).err().unwrap();
        assert!(error.contains("source package 200"), "{error}");
    }
}

#[test]
fn derived_members_keep_generator_source_configuration() {
    let fixture = SourceFixture::new(
        "use dependency.Flag; use dependency.enabled;
        struct Subject {} derive Flag.use(enabled) for Subject;
        fn run()->bool {Subject.as(Flag).flag()}",
        "pub trait Flag {fn flag()->bool;}
        pub derive fn enabled for Flag {
            fn flag()->bool {
                inline if (build.mode matches .release) and (build.target.arch matches .wasm32) {true}else{false}
            }
        }");
    let protocols = target_protocols();
    let luna::frontend::Artifact::Nir(package) =
        fixture.compile_configured(&protocols, None).unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let mut missing = protocols;
    missing.build_modes.remove(&200);
    let error = fixture.compile_configured(&missing, None).err().unwrap();
    assert!(error.contains("source package 200"), "{error}");
}

fn metadata_protocols() -> elaboration::program::Protocols {
    use luna::build_context::metadata::{Entry, Input, Snapshot, VERSION};
    let mut packages = sources();
    packages.push(SourcePackage {
        resolution_id: 9,
        identity: 300,
        name: "build".into(),
        entry: "unused.fl".into(),
    });
    let dependencies = vec![
        resolve::DependencyEdge {
            requester: 3,
            alias: "dependency".into(),
            provider: 7,
        },
        resolve::DependencyEdge {
            requester: 7,
            alias: "build".into(),
            provider: 9,
        },
    ];
    let inputs = [(200, true), (300, false)]
        .into_iter()
        .map(|(identity, value)| Input {
            identity,
            fields: vec![(
                "enabled".into(),
                Entry::new(
                    nir::Term::Primitive(nir::Primitive::Bool),
                    nir::Term::Bool(value),
                    &mut 1000,
                )
                .unwrap(),
            )],
        })
        .collect();
    let snapshot = Snapshot::new(VERSION, &packages, &dependencies, inputs, &mut 10000).unwrap();
    let mut protocols = elaboration::program::Protocols::default();
    snapshot.configure(&mut protocols);
    protocols
}

#[test]
fn source_package_metadata_uses_public_build_library_and_saved_origin() {
    let fixture = SourceFixture::new(
        r#"use build.package_metadata as lookup;
        comptime fn run()->bool {
            let own = lookup("dependency").features;
            let foreign = dependency.saved("build").features;
            (own.get(.enabled, bool) matches true?) and
            (foreign.get(.enabled, bool) matches false?) and
            (own.get(.absent, bool) matches null) and
            (own.get(.enabled, .Symbol'builtin) matches null)
        }"#,
        "pub const saved = build.package_metadata;",
    );
    let luna::frontend::Artifact::Nir(package) = fixture
        .compile_configured(&metadata_protocols(), None)
        .unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn source_package_metadata_reports_unknown_alias_and_missing_input() {
    for alias in ["unknown", "dependency"] {
        let fixture = SourceFixture::new(
            &format!(
                "comptime fn run()->bool {{ build.package_metadata(\"{alias}\").features.get(.enabled, bool) matches true? }}"
            ),
            "",
        );
        let mut protocols = metadata_protocols();
        std::sync::Arc::make_mut(protocols.build_metadata.as_mut().unwrap())
            .inputs
            .clear();
        let luna::frontend::Artifact::Nir(package) =
            fixture.compile_configured(&protocols, None).unwrap()
        else {
            panic!("expected NIR")
        };
        let error = luna::invocation::invoke(&package, "run", 100_000).unwrap_err();
        assert!(
            error.to_string().contains("BuildMetadataUnavailable"),
            "{error}"
        );
    }
}

#[test]
fn source_package_metadata_retains_dependency_enum_identity() {
    let fixture = SourceFixture::new(
        r#"comptime fn run()->bool {
            build.package_metadata("dependency").features.get(.arch, dependency.TargetArch) matches .gpu?
        }"#,
        "pub enum TargetArch { cpu, gpu }",
    );
    let mut protocols = metadata_protocols();
    let luna::frontend::Artifact::Nir(package) =
        fixture.compile_configured(&protocols, None).unwrap()
    else {
        panic!("expected NIR")
    };
    let declaration = package
        .enums
        .iter()
        .find(|item| item.name == "TargetArch")
        .unwrap()
        .declaration;
    let ty = nir::Term::Nominal {
        declaration,
        arguments: vec![],
    };
    let value = nir::Term::NominalValue {
        ty: Box::new(ty.clone()),
        variant: Some(1),
        fields: vec![],
    };
    std::sync::Arc::make_mut(protocols.build_metadata.as_mut().unwrap())
        .inputs
        .get_mut(&200)
        .unwrap()
        .insert("arch".into(), (ty, value));
    let luna::frontend::Artifact::Nir(package) =
        fixture.compile_configured(&protocols, None).unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn real_sysroot_metadata_materializes_at_o0_and_o3() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let luna::cli::Command::Compile(options) = luna::cli::parse([
        root.join("development/examples/sysroot-build-metadata.fl")
            .into_os_string(),
        "--sysroot".into(),
        root.join("library").into_os_string(),
        "--emit".into(),
        "nir".into(),
    ])
    .unwrap() else {
        panic!("expected compilation")
    };
    let mut protocols = elaboration::program::Protocols::default();
    let target = luna::frontend::native_test_target().expect("native target available");
    assert_eq!(
        target.pointer_bits(),
        64,
        "fixture expects a 64-bit native target"
    );
    protocols.build_metadata = Some(std::sync::Arc::new(
        elaboration::program::build_context::Metadata {
            aliases: [(100, [("build".into(), 4)].into())].into(),
            inputs: [(
                4,
                [(
                    "enabled".into(),
                    (
                        nir::Term::Primitive(nir::Primitive::Bool),
                        nir::Term::Bool(true),
                    ),
                )]
                .into(),
            )]
            .into(),
        },
    ));
    let (luna::frontend::Artifact::Nir(package), bindings) =
        luna::frontend::compile_bound_for_target(
            &options,
            100,
            &protocols,
            Default::default(),
            Some(target),
        )
        .unwrap()
    else {
        panic!("expected NIR")
    };
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        let report = luna::testing::run_native_report_filtered(
            &package,
            100,
            100_000,
            Some(&bindings),
            None,
            &luna::host::Toolchain {
                optimization,
                ..Default::default()
            },
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(report.results.len(), 2, "{report}");
        assert_eq!(report.failures(), 0, "{report}");
    }
}

#[test]
fn forwarded_metadata_callable_keeps_the_defining_packages_aliases() {
    let fixture = SourceFixture::new(
        "comptime fn run()->bool { dependency.read(build.package_metadata) }",
        r#"pub comptime fn read(query: comptime fn(.str'builtin) -> build.PackageMetadata)->bool {
            query("dependency").features.get(.enabled, bool) matches true?
        }"#,
    );
    let luna::frontend::Artifact::Nir(package) = fixture
        .compile_configured(&metadata_protocols(), None)
        .unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn source_metadata_admission_rejects_bad_payload_and_recovers() {
    let fixture = SourceFixture::new(
        r#"comptime fn run()->bool {
            build.package_metadata("dependency").features.get(.enabled, bool) matches true?
        }"#,
        "",
    );
    let mut protocols = metadata_protocols();
    std::sync::Arc::make_mut(protocols.build_metadata.as_mut().unwrap())
        .inputs
        .get_mut(&200)
        .unwrap()
        .insert(
            "enabled".into(),
            (
                nir::Term::Primitive(nir::Primitive::Bool),
                nir::Term::Symbol("invalid".into()),
            ),
        );
    let luna::frontend::Artifact::Nir(package) =
        fixture.compile_configured(&protocols, None).unwrap()
    else {
        panic!("expected NIR")
    };
    assert!(luna::invocation::invoke(&package, "run", 100_000).is_err());
    let luna::frontend::Artifact::Nir(recovered) = fixture
        .compile_configured(&metadata_protocols(), None)
        .unwrap()
    else {
        panic!("expected NIR")
    };
    assert_eq!(
        luna::invocation::invoke(&recovered, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn source_metadata_requires_driver_configuration_only_when_read() {
    let unused = SourceFixture::new("comptime fn run()->bool {true}", "");
    assert!(unused.compile_configured(&Default::default(), None).is_ok());
    let used = SourceFixture::new("const query=build.package_metadata;", "");
    let error = used
        .compile_configured(&Default::default(), None)
        .err()
        .expect("missing configuration must fail");
    assert!(
        error.contains("package metadata input is unavailable"),
        "{error}"
    );
    let mut protocols = metadata_protocols();
    std::sync::Arc::make_mut(protocols.build_metadata.as_mut().unwrap())
        .aliases
        .remove(&100);
    let error = used
        .compile_configured(&protocols, None)
        .err()
        .expect("missing owner must fail");
    assert!(
        error.contains("package metadata source owner is unavailable"),
        "{error}"
    );
}
