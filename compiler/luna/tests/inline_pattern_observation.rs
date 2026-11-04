//! Required pattern observation preserves resource failures across elaboration.
#[path = "support/primitive_bindings.rs"]
mod primitive_bindings;

use std::collections::BTreeMap;

#[test]
fn inline_pattern_budget_failure_is_diagnostic_and_a_fresh_attempt_recovers() {
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
    std::fs::write(
        &input,
        primitive_bindings::with_bool_binding(
            "fn run()->bool {inline if true is do {false=>false,true=>true,_=>false,}}",
        ),
    )
    .unwrap();
    let packages = [luna::packages::SourcePackage {
        resolution_id: 17,
        identity: 900,
        name: "input".into(),
        entry: input,
    }];
    let identities = BTreeMap::from([(17, 900)]);
    let mut saw_pattern_limit = false;
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
            for allowance in 0..512 {
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
                    if error
                        .to_string()
                        .contains("inline pattern selection budget exhausted")
                    {
                        assert!(steps < allowance);
                        saw_pattern_limit = true;
                        break;
                    }
                }
            }
            assert!(
                saw_pattern_limit,
                "resource failure never reached the pattern diagnostic"
            );
            let checked = elaboration::program::elaborate_packages_with_invocation_budget(
                batch.published(),
                &identities,
                &Default::default(),
                None,
                &mut 10_000,
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
    assert!(saw_pattern_limit);
    let luna::frontend::Artifact::Nir(package) = artifact else {
        panic!("expected checked NIR")
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}
