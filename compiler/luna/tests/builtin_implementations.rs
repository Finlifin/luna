#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

fn compile(source: &str, allowed: bool) -> Result<nir::program::Package, String> {
    support::compile_with_protocols(
        source,
        &elaboration::program::Protocols {
            builtin_implementation_packages: if allowed {
                [0].into()
            } else {
                Default::default()
            },
            ..Default::default()
        },
    )
}

#[test]
fn explicit_driver_grant_admits_inherent_builtin_methods() {
    let source = "typealias Object=.Object'builtin;
        impl Object {pub comptime fn enabled(self)->bool {true}}
        comptime fn run()->bool {let value:Object={flag:true};value.enabled()}";
    assert!(compile(source, false).is_err());
    let package = compile(source, true).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(package.builtin_implementation_packages, [0].into());
    assert!(
        package
            .to_string()
            .contains("(builtin-implementation-packages 0)")
    );
}

#[test]
fn grants_survive_early_method_selection_and_required_invocation() {
    let source = "typealias Type=.Type'builtin;typealias Object=.Object'builtin;
        impl Object {pub comptime fn Flag()->Type {bool}}
        typealias Flag=Object.Flag();
        comptime fn run()->bool {let value:Flag=true;value}";
    let package = compile(source, true).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn grants_do_not_skip_inherent_conflicts_or_authorize_source_names() {
    for source in [
        "mod meta {typealias Object=.Object'builtin;impl Object {comptime fn flag(self)->bool {true}}}",
        "typealias Object=.Object'builtin;impl Object {comptime fn flag(self)->bool {true}}",
    ] {
        assert!(compile(source, false).is_err());
    }
    assert!(
        compile(
            "typealias Object=.Object'builtin;
        impl Object {pub comptime fn flag(self)->bool {true}}
        impl Object {pub comptime fn flag(self)->bool {false}}",
            true
        )
        .is_err()
    );
}

#[test]
fn sysroot_grants_all_six_packages_but_not_the_application() {
    let root = std::env::temp_dir().join(format!(
        "luna-builtin-impls-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let library = root.join("library");
    std::fs::create_dir(&library).unwrap();
    let names = ["builtin", "std", "meta", "build", "attrs", "verification"];
    for name in names {
        let directory = library.join(name);
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("lib.fl"),
            format!(
                "pub typealias Object=.Object'builtin;pub mod prelude {{}}
            impl Object {{pub comptime fn ready_{name}(self)->bool {{true}}}}"
            ),
        )
        .unwrap();
    }
    let input = root.join("input.fl");
    let checks = names
        .iter()
        .map(|name| format!("value.ready_{name}()"))
        .collect::<Vec<_>>()
        .join(" and ");
    std::fs::write(
        &input,
        format!("use builtin.Object;comptime fn run()->bool {{let value:Object={{}};{checks}}}"),
    )
    .unwrap();
    let invoke = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
            .arg(&input)
            .arg("--sysroot")
            .arg(&library)
            .args(["--invoke-comptime", "run"])
            .output()
            .unwrap()
    };
    let output = invoke();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "(bool true)"
    );
    std::fs::write(&input,"use builtin.Object;mod meta {impl Object {pub comptime fn injected(self)->bool {true}}} comptime fn run()->bool {true}").unwrap();
    let output = invoke();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("inherent implementation admission failed")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn builtin_constructor_methods_keep_generic_subjects() {
    let source="impl ?T where T {pub comptime fn available(self)->bool {self matches _?}}
        comptime fn run()->bool {let present:?bool=true;let absent:?bool=null;present.available() and not absent.available()}";
    assert!(compile(source, false).is_err());
    let package = compile(source, true).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn designated_builtin_package_can_implement_a_foreign_trait() {
    use luna::packages::SourcePackage;
    let root = std::env::temp_dir().join(format!(
        "luna-builtin-trait-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let api = root.join("api.fl");
    let implementation = root.join("implementation.fl");
    std::fs::write(&api, "pub trait Marker {fn marker(self)->bool;}").unwrap();
    std::fs::write(
        &implementation,
        "pub mod prelude {} typealias Byte=.u8'builtin;use api.Marker;
        impl Marker for Byte {fn marker(self)->bool {true}}
        fn run(value:Byte)->bool {value.as(Marker).marker()}",
    )
    .unwrap();
    let packages = [
        SourcePackage {
            resolution_id: 1,
            identity: 10,
            name: "api".into(),
            entry: api,
        },
        SourcePackage {
            resolution_id: 2,
            identity: 20,
            name: "library".into(),
            entry: implementation,
        },
    ];
    let dependencies = [resolve::DependencyEdge {
        requester: 2,
        alias: "api".into(),
        provider: 1,
    }];
    assert!(luna::packages::compile_with_builtin(&packages, &dependencies, 2, None).is_err());
    let artifact =
        luna::packages::compile_with_builtin(&packages, &dependencies, 2, Some(2)).unwrap();
    let luna::frontend::Artifact::Nir(package) = artifact else {
        panic!("expected NIR")
    };
    assert_eq!(package.builtin_implementation_packages, [20].into());
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn package_cli_grants_are_explicit_validated_and_part_of_request_identity() {
    let path = std::env::temp_dir().join(format!("luna-impl-provider-{}.fl", std::process::id()));
    std::fs::write(
        &path,
        "typealias Object=.Object'builtin;impl Object {pub comptime fn ready(self)->bool {true}}
        comptime fn run()->bool {let value:Object={};value.ready()}",
    )
    .unwrap();
    let args: Vec<std::ffi::OsString> = vec![
        "--package".into(),
        "7".into(),
        "99".into(),
        "provider".into(),
        path.clone().into(),
        "--primary".into(),
        "7".into(),
        "--invoke-comptime".into(),
        "run".into(),
    ];
    let plain = luna::package_cli::parse(args.clone()).unwrap();
    assert!(plain.run().is_err());
    let mut granted_args = args.clone();
    granted_args.extend(["--builtin-impl-package".into(), "7".into()]);
    let granted = luna::package_cli::parse(granted_args.clone()).unwrap();
    assert_ne!(plain, granted);
    assert_eq!(granted.run().unwrap().trim(), "(bool true)");
    granted_args.extend(["--builtin-impl-package".into(), "7".into()]);
    assert!(luna::package_cli::parse(granted_args).is_err());
    let mut missing = args;
    missing.extend(["--builtin-impl-package".into(), "8".into()]);
    assert!(
        luna::package_cli::parse(missing)
            .unwrap()
            .run()
            .unwrap_err()
            .contains("is absent")
    );
    std::fs::remove_file(path).unwrap();
}
