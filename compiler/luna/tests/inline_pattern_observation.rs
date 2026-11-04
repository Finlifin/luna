//! Required static observation preserves resource failures across elaboration.
#[path = "support/primitive_bindings.rs"]
mod primitive_bindings;

use std::collections::BTreeMap;

fn check_source_observation(source: &str, expected_limit: &str, max_allowance: u64) {
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "luna-pattern-observation-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    let _cleanup = Cleanup(directory.clone());
    let input = directory.join("lib.fl");
    std::fs::write(&input, primitive_bindings::with_bool_binding(source)).unwrap();
    let packages = [luna::packages::SourcePackage {
        resolution_id: 17,
        identity: 900,
        name: "input".into(),
        entry: input,
    }];
    let identities = BTreeMap::from([(17, 900)]);
    let mut saw_limit = false;
    let (artifact, _) = luna::packages::compile_with_definitions(
        &packages,
        &[],
        17,
        None,
        &Default::default(),
        Default::default(),
        None,
        &mut 100_000,
        &mut |_, batch, _| {
            // Locate the boundary without baking unrelated preparation costs
            // into the test. Every attempt checks the same parsed source HIR.
            let mut diagnostics = std::collections::BTreeSet::new();
            for allowance in 0..max_allowance {
                let mut steps = allowance;
                let result = elaboration::program::elaborate_packages_with_invocation_budget(
                    batch.published(),
                    &identities,
                    &Default::default(),
                    None,
                    &mut steps,
                );
                assert!(steps <= allowance);
                if let Err(error) = result {
                    diagnostics.insert(error.message.clone());
                    if error.to_string().contains(expected_limit) {
                        assert!(steps < allowance);
                        saw_limit = true;
                        break;
                    }
                }
            }
            assert!(
                saw_limit,
                "resource failure never reached the expected diagnostic: {expected_limit}; observed {diagnostics:?}"
            );
            let checked = elaboration::program::elaborate_packages_with_invocation_budget(
                batch.published(),
                &identities,
                &Default::default(),
                None,
                &mut 1_000_000,
            )
            .map_err(|error| error.to_string())?;
            assert_eq!(
                luna::execution::interpret(&checked, "run", 100_000).unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
            Ok(())
        },
    )
    .unwrap();
    assert!(saw_limit);
    let luna::frontend::Artifact::Nir(package) = artifact else {
        panic!("expected checked NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn inline_pattern_budget_failure_is_diagnostic_and_a_fresh_attempt_recovers() {
    check_source_observation(
        "fn run()->bool {inline if true is do {false=>false,true=>true,_=>false,}}",
        "inline pattern selection budget exhausted",
        512,
    );
}

#[test]
fn pending_clause_context_retains_the_failed_type_observation() {
    let source = format!(
        "typealias Type = .Type'builtin; typealias str = .str'builtin;
        pure comptime fn Tagged<label: str> -> Type {{ bool }}
        typealias Marker = Tagged<\"{}\">;
        comptime fn choose() -> bool where T, .value: ?T = null {{ true }}
        fn run() -> bool {{ const answer = choose<.value = null, if true {{ Marker }} else {{ bool }}>(); answer }}",
        "x".repeat(128),
    );
    check_source_observation(&source, "required Type observation failed", 4096);
}

#[test]
fn normal_inputs_report_observation_limits_and_recover_with_fresh_fuel() {
    let label = format!("\"{}\"", "x".repeat(128));
    for argument in [
        label.clone(),
        format!("if true {{ {label} }} else {{ \"short\" }}"),
    ] {
        let source = format!(
            "typealias Type = .Type'builtin; typealias str = .str'builtin;
            pure comptime fn Tagged<label: str> -> Type {{ bool }}
            typealias Marker = Tagged<{argument}>;
            fn run() -> bool {{ true }}",
        );
        check_source_observation(&source, "static value observation failed", 4096);
    }
}

#[test]
fn pending_annotation_dependency_failure_does_not_authorize_the_typed_path() {
    check_source_observation(
        "typealias Type = .Type'builtin; typealias u8 = .u8'builtin;
        pure comptime fn Index<n: u8> -> Type { bool }
        comptime fn Domain(flag: bool) -> Type { if flag { Type } else { bool } }
        comptime fn accept(flag: bool, T: defer Domain(flag), n: u8,
            callback: defer fn(Index<if n < 255 { n + 1 } else { n }>) -> bool) -> bool { true }
        fn run() -> bool { true }",
        "lexical dependency observation budget exhausted",
        4096,
    );
}

#[test]
fn signature_dependency_classification_reports_exhaustion_and_recovers() {
    check_source_observation(
        "typealias Type = .Type'builtin;
        comptime fn Domain(flag: bool) -> Type { if flag { Type } else { bool } }
        fn identity(value: bool) -> bool { value }
        fn accept(comptime flag: bool, comptime T: defer Domain(flag),
            callback: defer fn(T) -> T) -> bool { true }
        fn run() -> bool { accept(true, bool, identity) }",
        "signature input observation budget exhausted",
        4096,
    );
}

#[test]
fn invocation_dependency_selection_exhaustion_preserves_fresh_attempts() {
    check_source_observation(
        "typealias Type = .Type'builtin;
        comptime fn accept(T: Type,
            .U: Type = do { let choose = || T; choose() }, .value: defer U = true) -> bool { true }
        const answer = accept(bool);
        fn run() -> bool { true }",
        "parameter dependency mapping budget exhausted",
        4096,
    );
}
