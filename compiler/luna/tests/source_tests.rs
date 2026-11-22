#[path = "support/execution.rs"]
mod support;

use luna::testing::run_report;

#[test]
fn structured_test_report_preserves_mixed_results_and_source_labels() {
    let package = support::compile("test failing {assumes false;} test passing {} test passing {}");
    let report = run_report(&package, 0, 10_000, false, None).unwrap();
    assert!(!report.is_success());
    assert_eq!(report.failures(), 1);
    assert_eq!(report.results.len(), 3);
    assert_eq!(
        report
            .results
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["failing", "passing", "passing"]
    );
    assert!(report.results[0].failure.is_some());
    assert!(report.results[1..].iter().all(|r| r.failure.is_none()));
    assert_ne!(report.results[1].declaration, report.results[2].declaration);
    let other = run_report(&package, 999, 10_000, false, None).unwrap();
    assert!(other.is_success());
    assert!(other.results.is_empty());
}

#[test]
fn test_driver_rejects_inconsistent_metadata_and_configuration() {
    let mut package = support::compile("test example {}");
    package.tests.push(package.tests[0]);
    assert!(
        run_report(&package, 0, 10_000, false, None)
            .unwrap_err()
            .contains("duplicate")
    );
    package.tests.pop();
    let function = package.functions.pop().unwrap();
    assert!(
        run_report(&package, 0, 10_000, false, None)
            .unwrap_err()
            .contains("no executable body")
    );
    package.functions.extend([function.clone(), function]);
    assert!(
        run_report(&package, 0, 10_000, false, None)
            .unwrap_err()
            .contains("ambiguous")
    );
    let empty = nir::program::Package::default();
    assert!(run_report(&empty, 0, 0, false, None).is_err());
    assert!(run_report(&empty, 0, 10_000, true, None).is_err());
    assert!(run_report(&empty, 0, 10_000, false, Some(&Default::default())).is_err());
}

#[test]
fn checked_test_session_isolates_failures_budgets_and_cleanup() {
    let package = support::compile(
        r#"
        trait Duplicate {}
        trait Retire { fn drop(*self); }
        struct Ticket { closed: *bool }
        impl Retire for Ticket { fn drop(*self) { self.closed.*.* = true; } }
        inline fn own(closed: *bool) { let ticket = Ticket { closed: closed }; }
        test shared {
            let closed = false;
            own(closed.ref);
            assumes closed;
        }
        test rejected { assumes false; }
        test exhausted { while true {} }
        test shared {
            let closed = false;
            assumes not closed;
            own(closed.ref);
            assumes closed;
        }
    "#,
    );
    let protocols = elaboration::program::Protocols {
        copy: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Duplicate")
                .unwrap()
                .declaration,
        ),
        drop: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Retire")
                .unwrap()
                .declaration,
        ),
        ..Default::default()
    };
    let before = package.to_string();
    for inline in [false, true] {
        for _ in 0..2 {
            let report = run_report(&package, 0, 10_000, inline, Some(&protocols)).unwrap();
            assert_eq!(report.results.len(), 4);
            assert_eq!(report.failures(), 2, "{report}");
            assert!(report.results[0].failure.is_none(), "{report}");
            assert!(report.results[1].failure.is_some(), "{report}");
            assert!(report.results[2].failure.is_some(), "{report}");
            assert!(report.results[3].failure.is_none(), "{report}");
        }
    }
    assert_eq!(
        before,
        package.to_string(),
        "shared preparation mutated source IR"
    );
}

#[test]
fn test_metadata_validation_precedes_filtering_and_ignores_unselected_packages() {
    use luna::testing::run_report_filtered;
    let mut package = support::compile("test selected {} test other {}");
    package.functions[1].phase = nir::program::FunctionPhase::Comptime;
    assert!(
        run_report_filtered(&package, 0, 1000, false, None, Some("selected"))
            .unwrap_err()
            .contains("runtime")
    );
    package.functions[1].phase = nir::program::FunctionPhase::Runtime;
    let mut unrelated = package.functions[1].clone();
    unrelated.declaration.package = 999;
    package.tests.push(unrelated.declaration);
    package.functions.extend([unrelated.clone(), unrelated]);
    let report = run_report_filtered(&package, 0, 1000, false, None, Some("absent")).unwrap();
    assert_eq!(report.results.len(), 0);
    assert_eq!(report.filtered_out, 2);
    assert!(report.is_success());
    assert!(
        run_report(&package, 999, 1000, false, None)
            .unwrap_err()
            .contains("ambiguous")
    );
}

#[test]
fn source_tests_invoke_declaration_identities_in_both_execution_modes() {
    let mut package = support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);} \
         test first {assumes true;} test second {assumes false;} test third {assumes true;}",
    );
    // NIR identities, not human-facing names, are the test entry contract.
    // Imported/generated entries may use the same diagnostic spelling.
    for function in &mut package.functions {
        if package.tests.contains(&function.declaration) {
            function.name = "same".into();
        }
    }
    let protocols = elaboration::program::Protocols {
        copy: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Duplicate")
                .unwrap()
                .declaration,
        ),
        drop: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Retire")
                .unwrap()
                .declaration,
        ),
        ..Default::default()
    };
    for (inline, bindings) in [
        (false, None),
        (false, Some(&protocols)),
        (true, Some(&protocols)),
    ] {
        for _ in 0..2 {
            let report = run_report(&package, 0, 10_000, inline, bindings).unwrap();
            assert_eq!(report.results.len(), 3);
            assert_eq!(report.failures(), 1, "{report}");
            assert!(report.results[0].failure.is_none(), "{report}");
            assert!(report.results[1].failure.is_some(), "{report}");
            assert!(report.results[2].failure.is_none(), "{report}");
            assert!(report.results.iter().all(|r| r.name == "same"));
            assert_eq!(
                report
                    .results
                    .iter()
                    .map(|r| r.declaration)
                    .collect::<Vec<_>>(),
                package.tests
            );
        }
    }
}

#[test]
fn discovery_preserves_order_and_validates_metadata_without_execution() {
    let mut package = support::compile("test fail {assumes false;} test {} test fail {}");
    let inventory = luna::testing::discover(&package, 0, None).unwrap();
    assert_eq!(
        inventory
            .entries
            .iter()
            .map(|e| e.declaration)
            .collect::<Vec<_>>(),
        package.tests
    );
    assert_eq!(
        inventory
            .entries
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        ["fail", "anonymous", "fail"]
    );
    assert_eq!(
        luna::testing::discover(&package, 999, None)
            .unwrap()
            .entries
            .len(),
        0
    );
    package.tests.push(package.tests[0]);
    assert!(
        luna::testing::discover(&package, 0, Some("absent"))
            .unwrap_err()
            .contains("duplicate")
    );
}

#[test]
fn native_source_tests_keep_identity_order_and_continue_after_traps() {
    let package = support::compile("test same {} test same {assumes false;} test same {}");
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        let toolchain = luna::host::Toolchain {
            optimization,
            ..Default::default()
        };
        let report = luna::testing::run_native_report_filtered(
            &package,
            0,
            100_000,
            None,
            None,
            &toolchain,
            std::time::Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(report.results.len(), 3);
        assert_eq!(report.failures(), 1, "{report}");
        assert!(report.results[0].failure.is_none());
        assert!(
            report.results[1]
                .failure
                .as_ref()
                .unwrap()
                .contains("native test failed")
        );
        assert!(report.results[2].failure.is_none());
        assert_eq!(
            report
                .results
                .iter()
                .map(|r| r.declaration)
                .collect::<Vec<_>>(),
            package.tests
        );
        assert!(report.results.iter().all(|r| r.name == "same"));
    }
}

#[test]
fn native_source_test_deadline_does_not_block_later_tests() {
    let package = support::compile("test endless {while true {}} test following {}");
    let report = luna::testing::run_native_report_filtered(
        &package,
        0,
        100_000,
        None,
        None,
        &Default::default(),
        std::time::Duration::from_millis(100),
    )
    .unwrap();
    assert_eq!(report.failures(), 1, "{report}");
    assert!(
        report.results[0]
            .failure
            .as_ref()
            .unwrap()
            .contains("timed out")
    );
    assert!(report.results[1].failure.is_none());
}

#[test]
fn native_source_test_filter_skips_toolchain_and_validates_limits() {
    let package = support::compile("test example {}");
    let toolchain = luna::host::Toolchain {
        llvm_config: "/nonexistent/flurry-llvm".into(),
        ..Default::default()
    };
    let timeout = std::time::Duration::from_secs(1);
    let report = luna::testing::run_native_report_filtered(
        &package,
        0,
        100_000,
        None,
        Some("absent"),
        &toolchain,
        timeout,
    )
    .unwrap();
    assert!(report.is_success());
    assert_eq!(report.filtered_out, 1);
    assert!(
        luna::testing::run_native_report_filtered(&package, 0, 0, None, None, &toolchain, timeout)
            .is_err()
    );
    assert!(
        luna::testing::run_native_report_filtered(
            &package,
            0,
            100_000,
            None,
            None,
            &toolchain,
            std::time::Duration::ZERO
        )
        .is_err()
    );
    let report = luna::testing::run_native_report_filtered(
        &package, 0, 100_000, None, None, &toolchain, timeout,
    )
    .unwrap();
    assert_eq!(report.failures(), 1);
    assert!(
        report.results[0]
            .failure
            .as_ref()
            .unwrap()
            .contains("cannot start")
    );
}

#[test]
fn native_source_test_lowering_failure_keeps_later_tests_independent() {
    let package = support::compile(
        "test unsupported {let flag=true;let pointer=flag.ref;pointer.*;}
         test following {} test trapped {assumes false;} test last {}",
    );
    let report = luna::testing::run_native_report_filtered(
        &package,
        0,
        100_000,
        None,
        None,
        &Default::default(),
        std::time::Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(report.results.len(), 4);
    assert_eq!(report.failures(), 2, "{report}");
    assert!(
        report.results[0].failure.as_ref().unwrap().contains("host"),
        "{report}"
    );
    assert!(report.results[1].failure.is_none(), "{report}");
    assert!(
        report.results[2]
            .failure
            .as_ref()
            .unwrap()
            .contains("native test failed"),
        "{report}"
    );
    assert!(report.results[3].failure.is_none(), "{report}");
    let filtered = luna::testing::run_native_report_filtered(
        &package,
        0,
        100_000,
        None,
        Some("following"),
        &Default::default(),
        std::time::Duration::from_secs(2),
    )
    .unwrap();
    assert!(filtered.is_success(), "{filtered}");
    assert_eq!(filtered.filtered_out, 3);
}

#[test]
fn native_source_test_invalid_result_does_not_prevent_valid_identity_execution() {
    let mut package = support::compile("test invalid {} test valid {}");
    let first = package.tests[0];
    let function = package
        .functions
        .iter_mut()
        .find(|f| f.declaration == first)
        .unwrap();
    function.signature = nir::Term::Function {
        parameters: vec![],
        result: Box::new(nir::Term::Primitive(nir::Primitive::Bool)),
    };
    function.body.ty = nir::Term::Primitive(nir::Primitive::Bool);
    function.body.kind = nir::program::ExprKind::Literal(nir::program::Literal::Bool(true));
    let report = luna::testing::run_native_report_filtered(
        &package,
        0,
        100_000,
        None,
        None,
        &Default::default(),
        std::time::Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(
        report.results[0].failure.as_deref(),
        Some("test must return void or !E void")
    );
    assert!(report.results[1].failure.is_none(), "{report}");

    let mut error_bool = support::compile("test invalid {} test valid {}");
    let first = error_bool.tests[0];
    let function = error_bool
        .functions
        .iter_mut()
        .find(|f| f.declaration == first)
        .unwrap();
    function.signature = nir::Term::Function {
        parameters: vec![],
        result: Box::new(nir::Term::ErrorResult {
            errors: vec![nir::Term::Primitive(nir::Primitive::Bool)],
            result: Box::new(nir::Term::Primitive(nir::Primitive::Bool)),
        }),
    };
    function.body.ty = nir::Term::ErrorResult {
        errors: vec![nir::Term::Primitive(nir::Primitive::Bool)],
        result: Box::new(nir::Term::Primitive(nir::Primitive::Bool)),
    };
    function.body.kind = nir::program::ExprKind::Literal(nir::program::Literal::Bool(true));
    let interp_report = run_report(&error_bool, 0, 10_000, false, None).unwrap();
    assert_eq!(
        interp_report.results[0].failure.as_deref(),
        Some("test must return void or !E void")
    );
    assert!(
        interp_report.results[1].failure.is_none(),
        "{interp_report}"
    );
    let native_report = luna::testing::run_native_report_filtered(
        &error_bool,
        0,
        100_000,
        None,
        None,
        &Default::default(),
        std::time::Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(
        native_report.results[0].failure.as_deref(),
        Some("test must return void or !E void")
    );
    assert!(
        native_report.results[1].failure.is_none(),
        "{native_report}"
    );
}

#[test]
fn native_source_test_linker_failure_is_reported_per_identity_and_recovers() {
    let package = support::compile("test same {} test same {}");
    let broken = luna::host::Toolchain {
        linker: "/nonexistent/flurry-linker".into(),
        ..Default::default()
    };
    let timeout = std::time::Duration::from_secs(2);
    let failed = luna::testing::run_native_report_filtered(
        &package, 0, 100_000, None, None, &broken, timeout,
    )
    .unwrap();
    assert_eq!(failed.results.len(), 2);
    assert_eq!(failed.failures(), 2);
    assert!(failed.results.iter().all(|result| {
        result
            .failure
            .as_ref()
            .unwrap()
            .contains("cannot start /nonexistent/flurry-linker")
    }));
    let recovered = luna::testing::run_native_report_filtered(
        &package,
        0,
        100_000,
        None,
        None,
        &Default::default(),
        timeout,
    )
    .unwrap();
    assert!(recovered.is_success(), "{recovered}");
    assert_eq!(
        recovered
            .results
            .iter()
            .map(|r| r.declaration)
            .collect::<Vec<_>>(),
        package.tests
    );
}

#[test]
fn native_inline_preparation_failure_preserves_source_and_recovers() {
    let package = support::compile(
        "trait Duplicate {} trait Retire {fn drop(*self);}impl Duplicate for bool {}
        inline fn identity(value:bool)->bool {value}
        test same {let value=identity(true);assumes value;}
        test same {let value=identity(false);assumes not value;}",
    );
    let protocols = elaboration::program::Protocols {
        copy: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Duplicate")
                .unwrap()
                .declaration,
        ),
        drop: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Retire")
                .unwrap()
                .declaration,
        ),
        ..Default::default()
    };
    let original = package.to_string();
    let missing = luna::host::Toolchain {
        llvm_config: "/nonexistent/flurry-llvm".into(),
        ..Default::default()
    };
    let timeout = std::time::Duration::from_secs(5);
    let filtered = luna::testing::run_native_report_filtered_with_inlining(
        &package,
        0,
        1,
        Some(&protocols),
        Some("absent"),
        &missing,
        timeout,
        true,
    )
    .unwrap();
    assert!(filtered.is_success());
    assert_eq!(filtered.filtered_out, 2);
    let rejected = luna::testing::run_native_report_filtered_with_inlining(
        &package,
        0,
        1,
        Some(&protocols),
        None,
        &missing,
        timeout,
        true,
    )
    .unwrap();
    assert_eq!(rejected.failures(), 2, "{rejected}");
    assert!(
        rejected.results.iter().all(|r| r
            .failure
            .as_ref()
            .unwrap()
            .contains("cannot prepare optional host inlining")),
        "{rejected}"
    );
    assert_eq!(package.to_string(), original);
    let recovered = luna::testing::run_native_report_filtered_with_inlining(
        &package,
        0,
        100_000,
        Some(&protocols),
        None,
        &Default::default(),
        timeout,
        true,
    )
    .unwrap();
    assert!(recovered.is_success(), "{recovered}");
    assert_eq!(
        recovered
            .results
            .iter()
            .map(|r| r.declaration)
            .collect::<Vec<_>>(),
        package.tests
    );
    assert_eq!(package.to_string(), original);
    assert!(
        luna::testing::run_native_report_filtered_with_inlining(
            &package, 0, 100_000, None, None, &missing, timeout, true
        )
        .unwrap_err()
        .contains("Copy and Drop")
    );
}

#[test]
fn error_result_tests_execute_across_interpreter_and_native_modes() {
    let mut package = support::compile(
        r#"
        typealias void = .void'builtin;
        trait Duplicate {}
        trait Retire { fn drop(*self); }
        enum IoErr { NotFound, Denied, }
        enum ParseErr { Invalid, }

        fn check_ok() -> !IoErr void {}
        fn check_single_err() -> !IoErr void { error IoErr.Denied }
        fn check_empty_ok() -> ![] void {}
        fn helper() -> !IoErr void { error IoErr.NotFound }
        fn check_propagated() -> ![IoErr, ParseErr] void {
            helper()!;
        }
        fn check_second_err() -> ![IoErr, ParseErr] void {
            if false { error IoErr.NotFound } else { error ParseErr.Invalid }
        }
    "#,
    );
    let protocols = elaboration::program::Protocols {
        copy: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Duplicate")
                .unwrap()
                .declaration,
        ),
        drop: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Retire")
                .unwrap()
                .declaration,
        ),
        ..Default::default()
    };
    let names = [
        "check_ok",
        "check_single_err",
        "check_empty_ok",
        "check_propagated",
        "check_second_err",
    ];
    package.tests.clear();
    for name in names {
        let decl = package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration;
        package.tests.push(decl);
    }

    for (inline, bindings) in [
        (false, None),
        (false, Some(&protocols)),
        (true, Some(&protocols)),
    ] {
        let report = run_report(&package, 0, 100_000, inline, bindings).unwrap();
        assert_eq!(report.results.len(), 5, "{report}");
        assert_eq!(report.failures(), 3, "{report}");
        assert!(report.results[0].failure.is_none(), "{report}");
        let single_err = report.results[1].failure.as_ref().unwrap();
        assert!(
            single_err.contains("(branch (error 0))")
                && single_err.contains("(error-type (string \"IoErr\"))"),
            "{single_err}"
        );
        assert!(report.results[2].failure.is_none(), "{report}");
        let prop_err = report.results[3].failure.as_ref().unwrap();
        assert!(
            prop_err.contains("(branch (error 0))")
                && prop_err.contains("(error-type (string \"IoErr\"))"),
            "{prop_err}"
        );
        let second_err = report.results[4].failure.as_ref().unwrap();
        assert!(
            second_err.contains("(branch (error 1))")
                && second_err.contains("(error-type (string \"ParseErr\"))"),
            "{second_err}"
        );
    }

    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        let toolchain = luna::host::Toolchain {
            optimization,
            ..Default::default()
        };
        // Unchecked native lowering rejects nominal error storage cleanly per
        // identity without preventing independent non-nominal `![] void` tests.
        let unchecked = luna::testing::run_native_report_filtered(
            &package,
            0,
            100_000,
            None,
            None,
            &toolchain,
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(unchecked.results.len(), 5, "{unchecked}");
        assert!(unchecked.results[2].failure.is_none(), "{unchecked}");
        for nominal_index in [0usize, 1, 3, 4] {
            let reason = unchecked.results[nominal_index].failure.as_deref().unwrap();
            assert!(
                reason.contains("Copy/Drop resource verification"),
                "{reason}"
            );
        }

        for inline in [false, true] {
            let report = luna::testing::run_native_report_filtered_with_inlining(
                &package,
                0,
                100_000,
                Some(&protocols),
                None,
                &toolchain,
                std::time::Duration::from_secs(5),
                inline,
            )
            .unwrap();
            assert_eq!(report.results.len(), 5, "{report}");
            assert_eq!(report.failures(), 3, "{report}");
            assert!(report.results[0].failure.is_none(), "{report}");
            let single_err = report.results[1].failure.as_ref().unwrap();
            assert!(
                single_err.contains("exit status: 104")
                    && single_err.contains(
                        r#"(test-failure (reason error-result) (branch (error 0)) (error-type (string \"IoErr\"))"#,
                    ),
                "{single_err}"
            );
            assert!(report.results[2].failure.is_none(), "{report}");
            let prop_err = report.results[3].failure.as_ref().unwrap();
            assert!(
                prop_err.contains("exit status: 104")
                    && prop_err.contains(
                        r#"(test-failure (reason error-result) (branch (error 0)) (error-type (string \"IoErr\"))"#,
                    ),
                "{prop_err}"
            );
            let second_err = report.results[4].failure.as_ref().unwrap();
            assert!(
                second_err.contains("exit status: 104")
                    && second_err.contains(
                        r#"(test-failure (reason error-result) (branch (error 1)) (error-type (string \"ParseErr\"))"#,
                    ),
                "{second_err}"
            );
        }
    }

    // Structural primitive `!bool void` tests work in both unchecked and checked
    // native modes without requiring nominal Copy/Drop bindings.
    let mut primitive_pkg = support::compile(
        r#"
        typealias void = .void'builtin;
        fn bool_ok() -> !bool void {}
        fn bool_err() -> !bool void { error true }
    "#,
    );
    primitive_pkg.tests = ["bool_ok", "bool_err"]
        .into_iter()
        .map(|name| {
            primitive_pkg
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .declaration
        })
        .collect();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        let toolchain = luna::host::Toolchain {
            optimization,
            ..Default::default()
        };
        let report = luna::testing::run_native_report_filtered(
            &primitive_pkg,
            0,
            100_000,
            None,
            None,
            &toolchain,
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(report.failures(), 1, "{report}");
        assert!(report.results[0].failure.is_none(), "{report}");
        let err = report.results[1].failure.as_ref().unwrap();
        assert!(
            err.contains("exit status: 104")
                && err.contains("(test-failure (reason error-result) (branch (error 0)) (type (primitive bool)))"),
            "{err}"
        );
    }
}

#[test]
fn error_result_tests_run_drop_cleanup_on_returned_error_payloads() {
    let mut package = support::compile(
        r#"
        typealias void = .void'builtin;
        trait Duplicate {}
        trait Retire { fn drop(*self); }
        impl Duplicate for bool {}
        struct OwnedErr { valid: bool }
        impl Retire for OwnedErr {
            fn drop(*self) { assumes self.valid.*; }
        }

        fn owned_ok() -> !OwnedErr void {}
        fn owned_err_valid() -> !OwnedErr void {
            error OwnedErr { valid: true }
        }
        fn owned_err_invalid_drop() -> !OwnedErr void {
            error OwnedErr { valid: false }
        }
    "#,
    );
    let protocols = elaboration::program::Protocols {
        copy: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Duplicate")
                .unwrap()
                .declaration,
        ),
        drop: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Retire")
                .unwrap()
                .declaration,
        ),
        ..Default::default()
    };
    package.tests.clear();
    for name in ["owned_ok", "owned_err_valid", "owned_err_invalid_drop"] {
        let decl = package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration;
        package.tests.push(decl);
    }

    for inline in [false, true] {
        let report = run_report(&package, 0, 100_000, inline, Some(&protocols)).unwrap();
        assert_eq!(report.failures(), 2, "{report}");
        assert!(report.results[0].failure.is_none(), "{report}");
        let valid_err = report.results[1].failure.as_ref().unwrap();
        assert!(
            valid_err.contains("(branch (error 0))")
                && valid_err.contains("(error-type (string \"OwnedErr\"))"),
            "{valid_err}"
        );
        let invalid_drop = report.results[2].failure.as_ref().unwrap();
        assert!(
            !invalid_drop.contains("(branch (error 0))")
                && invalid_drop.contains("ProgramContractRejected"),
            "{invalid_drop}"
        );
    }

    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        let toolchain = luna::host::Toolchain {
            optimization,
            ..Default::default()
        };
        for inline in [false, true] {
            let report = luna::testing::run_native_report_filtered_with_inlining(
                &package,
                0,
                100_000,
                Some(&protocols),
                None,
                &toolchain,
                std::time::Duration::from_secs(5),
                inline,
            )
            .unwrap();
            assert_eq!(report.failures(), 2, "{report}");
            assert!(report.results[0].failure.is_none(), "{report}");
            let valid_err = report.results[1].failure.as_ref().unwrap();
            assert!(
                valid_err.contains("exit status: 104")
                    && valid_err.contains(
                        r#"(test-failure (reason error-result) (branch (error 0)) (error-type (string \"OwnedErr\"))"#,
                    ),
                "{valid_err}"
            );
            let invalid_drop = report.results[2].failure.as_ref().unwrap();
            // A predicate failure inside Drop traps rather than returning control
            // to the interrupted cleanup caller (exit status 104 / 101).
            assert!(
                invalid_drop.contains("native test failed")
                    && invalid_drop.contains("(runtime-check-failure")
                    && !invalid_drop.contains("error-result")
                    && !invalid_drop.contains("exit status: 104"),
                "{invalid_drop}"
            );
        }
    }
}

#[test]
fn checked_native_void_test_reports_structured_failure_and_runs_drop_cleanup() {
    let package = support::compile(
        r#"
        trait Duplicate {}
        trait Retire { fn drop(*self); }
        impl Duplicate for bool {}
        struct Ticket { ready: bool }
        impl Retire for Ticket {
            fn drop(*self) { assumes self.ready.*; }
        }

        test void_ok {
            let ticket = Ticket { ready: true };
        }
        test void_body_fail {
            let ticket = Ticket { ready: true };
            assumes false;
        }
        test void_drop_fail {
            let ticket = Ticket { ready: false };
            return;
        }
    "#,
    );
    let protocols = elaboration::program::Protocols {
        copy: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Duplicate")
                .unwrap()
                .declaration,
        ),
        drop: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Retire")
                .unwrap()
                .declaration,
        ),
        ..Default::default()
    };

    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        let toolchain = luna::host::Toolchain {
            optimization,
            ..Default::default()
        };
        for inline in [false, true] {
            let report = luna::testing::run_native_report_filtered_with_inlining(
                &package,
                0,
                100_000,
                Some(&protocols),
                None,
                &toolchain,
                std::time::Duration::from_secs(5),
                inline,
            )
            .unwrap();
            assert_eq!(report.failures(), 2, "{report}");
            assert!(report.results[0].failure.is_none(), "{report}");
            let body_fail = report.results[1].failure.as_ref().unwrap();
            assert!(
                body_fail.contains("exit status: 101")
                    && body_fail.contains("(runtime-check-failure")
                    && body_fail.contains("(test-failure (reason runtime-failure))"),
                "void_body_fail: {body_fail}"
            );
            let drop_fail = report.results[2].failure.as_ref().unwrap();
            assert!(
                drop_fail.contains("native test failed")
                    && drop_fail.contains("(runtime-check-failure"),
                "void_drop_fail: {drop_fail}"
            );
        }
    }
}

#[test]
fn test_template_instances_execute_natively_with_cleanup_contracts_and_filtering() {
    let package = support::compile(
        r#"
        typealias void = .void'builtin;
        trait Duplicate {}
        trait Retire { fn drop(*self); }
        impl Duplicate for bool {}
        impl Duplicate for *Ticket {}
        struct Ticket { ready: bool }
        impl Retire for Ticket {
            fn drop(*self) { assumes self.ready.*; }
        }

        template test with_ticket
        where body: < ticket: *Ticket |- void >,
            .expected_ready: bool = true,
            requires expected_ready
        {
            var guard = Ticket { ready: false };
            let ticket = guard.ref;
            ~<body>;
        }

        test templ_ok use with_ticket {
            ticket.ready.* = true;
        }
        test templ_early_return_drop_fail use with_ticket {
            return;
        }
        test templ_body_fail use with_ticket {
            ticket.ready.* = true;
            assumes false;
        }
    "#,
    );
    let protocols = elaboration::program::Protocols {
        copy: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Duplicate")
                .unwrap()
                .declaration,
        ),
        drop: Some(
            package
                .traits
                .iter()
                .find(|t| t.name == "Retire")
                .unwrap()
                .declaration,
        ),
        ..Default::default()
    };

    let inventory = luna::testing::discover(&package, 0, None).unwrap();
    assert_eq!(
        inventory
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        [
            "templ_ok",
            "templ_early_return_drop_fail",
            "templ_body_fail"
        ]
    );

    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        let toolchain = luna::host::Toolchain {
            optimization,
            ..Default::default()
        };
        for inline in [false, true] {
            let filtered = luna::testing::run_native_report_filtered_with_inlining(
                &package,
                0,
                100_000,
                Some(&protocols),
                Some("templ_ok"),
                &toolchain,
                std::time::Duration::from_secs(5),
                inline,
            )
            .unwrap();
            assert!(filtered.is_success(), "{filtered}");
            assert_eq!(filtered.filtered_out, 2);

            let report = luna::testing::run_native_report_filtered_with_inlining(
                &package,
                0,
                100_000,
                Some(&protocols),
                None,
                &toolchain,
                std::time::Duration::from_secs(5),
                inline,
            )
            .unwrap();
            assert_eq!(report.results.len(), 3, "{report}");
            assert_eq!(report.failures(), 2, "{report}");
            assert!(report.results[0].failure.is_none(), "{report}");
            let drop_fail = report.results[1].failure.as_ref().unwrap();
            assert!(
                drop_fail.contains("native test failed")
                    && drop_fail.contains("(runtime-check-failure"),
                "templ_early_return_drop_fail: {drop_fail}"
            );
            let body_fail = report.results[2].failure.as_ref().unwrap();
            assert!(
                body_fail.contains("exit status: 101")
                    && body_fail.contains("(runtime-check-failure")
                    && body_fail.contains("(test-failure (reason runtime-failure))"),
                "templ_body_fail: {body_fail}"
            );
        }
    }
}
