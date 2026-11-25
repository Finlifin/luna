//! Attribute inspection precedes strategy selection and publishes no executable.
#[path = "support/primitive_bindings.rs"]
mod primitive_bindings;

use luna::cli::{Emit, Options};
use luna::frontend::{self, Artifact};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn compile(source: &str, emit: Emit) -> Result<Artifact, String> {
    let path = std::env::temp_dir().join(format!(
        "luna-derive-attributes-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    primitive_bindings::write_fixture(&path, source).unwrap();
    let result = frontend::compile(&Options {
        dispatch_fuel: None,
        comptime_intrinsics: Vec::new(),
        input: path.clone(),
        emit,
        output: None,
        build_modes: Default::default(),
        build_targets: Default::default(),
        sysroot: None,
        splice_protocol: None,
        meta_list_type: None,
        tuple_type: None,
        index_protocol: None,
        ord_protocol: None,
        eq_protocol: None,
        list_pattern_protocol: None,
        comptime_list_pattern_protocol: None,
        stable_list_pattern_protocol: None,
        list_pattern_endless_protocol: None,
        stable_list_pattern_endless_protocol: None,
        range_end_type: None,
        range_pattern_protocol: None,
        stable_range_pattern_protocol: None,
        concat_pattern_protocol: None,
        stable_concat_pattern_protocol: None,
        map_pattern_protocol: None,
        stable_map_pattern_protocol: None,
        struct_pattern_protocol: None,
        stable_struct_pattern_protocol: None,
        type_pattern_protocol: None,
        iterator_protocol: None,
        into_iterator_protocol: None,
        variadic_collector_protocol: None,
        extended_variadic_collector_protocol: None,
        literal_into_protocol: None,
        implies_protocol: None,
        comptime_implies_protocol: None,
        deref_protocol: None,
        deref_mut_protocol: None,
        reference_protocol: None,
        implicit_passing_protocol: None,
        call_context_type: None,
        call_description_type: None,
        arithmetic_protocols: Default::default(),
        incremental: Default::default(),
    });
    std::fs::remove_file(path).unwrap();
    result
}

const MARKER: &str =
    "comptime fn marker(flag:bool)->.Object'builtin {let metadata={selected:flag};metadata}";

#[test]
fn default_request_metadata_is_available_before_selection() {
    let source = format!(
        "{MARKER} trait Flag {{}} ^marker(true) derive fn configured for Flag {{}} struct Subject {{}} derive Flag for Subject;"
    );
    let Artifact::Attributes(attributes) = compile(&source, Emit::Attributes).unwrap() else {
        panic!("not attributes")
    };
    assert_eq!(attributes.len(), 1);
    assert!(attributes[0].to_string().contains("selected"));
    let merged = compile(&source, Emit::MergedAttributes)
        .unwrap()
        .to_string();
    assert!(
        merged.contains("(attribute-inspection") && merged.contains("selected"),
        "{merged}"
    );
    let error = compile(&source, Emit::Nir)
        .err()
        .expect("expected compilation failure");
    assert!(
        error.contains("no applicable default derivation strategy found"),
        "{error}"
    );
}

#[test]
fn unselected_strategy_body_is_not_executed_for_its_metadata() {
    let source = format!(
        "{MARKER} trait Flag {{fn flag()->bool;}} ^marker(true) derive fn configured(value:bool) for Flag {{while true {{}} fn flag()->bool {{value}}}}"
    );
    let Artifact::Attributes(attributes) = compile(&source, Emit::Attributes).unwrap() else {
        panic!("not attributes")
    };
    assert_eq!(attributes.len(), 1);
    assert!(compile(&source, Emit::Nir).is_err());
}

#[test]
fn repeated_explicit_requests_do_not_duplicate_strategy_attributes() {
    let source = format!(
        "{MARKER} trait Flag {{}} ^marker(true) derive fn configured for Flag {{}} struct First {{}} struct Second {{}} derive Flag.use(configured) for First; derive Flag.use(configured) for Second;"
    );
    let Artifact::Attributes(attributes) = compile(&source, Emit::Attributes).unwrap() else {
        panic!("not attributes")
    };
    assert_eq!(attributes.len(), 1);
    let prefix =
        format!("{MARKER} trait Flag {{}} ^marker(true) derive fn configured for Flag {{}}");
    let Artifact::Attributes(original) = compile(&prefix, Emit::Attributes).unwrap() else {
        panic!("not attributes")
    };
    assert_eq!(attributes[0].target, original[0].target);
}

#[test]
fn attribute_computation_contracts_still_reject_invalid_inputs() {
    let source = "comptime fn marker(flag:bool)->.Object'builtin where requires flag {let metadata={selected:flag};metadata} trait Flag {} ^marker(false) derive fn configured for Flag {} struct Subject {} derive Flag for Subject;";
    let error = compile(source, Emit::Attributes)
        .err()
        .expect("expected compilation failure");
    assert!(
        error.contains("contract") || error.contains("precondition"),
        "{error}"
    );
}

#[test]
fn attribute_merging_retains_order_and_original_targets() {
    let source = format!(
        "{MARKER} trait Flag {{}} ^marker(false) ^marker(true) derive fn configured for Flag {{}}"
    );
    let Artifact::Attributes(attributes) = compile(&source, Emit::Attributes).unwrap() else {
        panic!("not attributes")
    };
    assert_eq!(attributes.len(), 2);
    assert_eq!(attributes[0].target, attributes[1].target);
    let merged =
        elaboration::program::attributes::merge(&attributes, 1024, &mut Default::default())
            .unwrap();
    assert_eq!(merged.len(), 1);
    let selected = merged[0].get("selected").unwrap();
    assert!(matches!(
        selected.value.kind,
        nir::program::ExprKind::Literal(nir::program::Literal::Bool(false))
    ));
}

#[test]
fn missing_strategy_metadata_does_not_produce_an_executable_artifact() {
    let source =
        "trait Flag {} derive fn configured for Flag {} struct Subject {} derive Flag for Subject;";
    let Artifact::Attributes(attributes) = compile(source, Emit::Attributes).unwrap() else {
        panic!("not attributes")
    };
    assert!(attributes.is_empty());
    assert!(compile(source, Emit::Nir).is_err());
}

#[test]
fn merged_default_marker_consumes_values_not_helper_names() {
    let source = "const arbitrary_name={__flurry_derivation_default:true};trait Flag {} ^arbitrary_name derive fn configured for Flag {} struct Subject {} derive Flag for Subject;";
    let report = compile(source, Emit::MergedAttributes).unwrap().to_string();
    assert!(
        report.contains("(default-strategy-marker") && report.contains("(enabled true)"),
        "{report}"
    );
    let disabled = source.replace(
        "^arbitrary_name",
        "^{__flurry_derivation_default:false} ^arbitrary_name",
    );
    let report = compile(&disabled, Emit::MergedAttributes)
        .unwrap()
        .to_string();
    assert!(
        report.contains("(enabled false)") && !report.contains("(enabled true)"),
        "{report}"
    );
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("expected NIR artifact");
    };
    assert_eq!(package.implementations.len(), 1);
    let error = compile(&disabled, Emit::Nir)
        .err()
        .expect("disabled marker should leave request without a default strategy");
    assert!(
        error.contains("no applicable default derivation strategy found"),
        "{error}"
    );
}

#[test]
fn default_marker_rejects_wrong_targets_and_non_boolean_values() {
    for source in [
        "^{__flurry_derivation_default:true} fn ordinary() {}",
        "trait Flag {} struct Subject {} ^{__flurry_derivation_default:true} derive Flag for Subject;",
        "trait Flag {} ^{__flurry_derivation_default:7} derive fn configured for Flag {}",
    ] {
        assert!(matches!(
            compile(source, Emit::Attributes).unwrap(),
            Artifact::Attributes(_)
        ));
        let error = compile(source, Emit::MergedAttributes)
            .err()
            .expect("invalid marker");
        assert!(error.contains("derivation_default requires"), "{error}");
    }
}

#[test]
fn selected_default_marker_overrides_a_non_boolean_inner_value() {
    let source = "trait Flag {} ^{__flurry_derivation_default:true} ^{__flurry_derivation_default:7} derive fn configured for Flag {}";
    let report = compile(source, Emit::MergedAttributes).unwrap().to_string();
    assert!(report.contains("(enabled true)"), "{report}");
}

#[test]
fn imported_default_metadata_uses_the_same_multi_package_consumer() {
    let directory = std::env::temp_dir().join(format!(
        "luna-default-marker-packages-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("marks.fl"),
        "pub const marker={__flurry_derivation_default:true};",
    )
    .unwrap();
    std::fs::write(directory.join("app.fl"),"use marks.marker as selected; trait Flag {} ^selected derive fn configured for Flag {} struct Subject {} derive Flag for Subject;").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .current_dir(&directory)
        .args([
            "--packages-v1",
            "--package",
            "1",
            "101",
            "app",
            "app.fl",
            "--package",
            "2",
            "202",
            "marks",
            "marks.fl",
            "--dependency",
            "1",
            "marks",
            "2",
            "--primary",
            "1",
            "--emit",
            "merged-attributes",
        ])
        .output()
        .unwrap();
    std::fs::remove_file(directory.join("app.fl")).unwrap();
    std::fs::remove_file(directory.join("marks.fl")).unwrap();
    std::fs::remove_dir(&directory).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        report.matches("(default-strategy-marker ").count(),
        1,
        "{report}"
    );
    assert!(report.contains("(enabled true)"), "{report}");
}

#[test]
fn default_strategy_matches_unordered_error_result_label_sets() {
    let source = r#"
        typealias u64 = .u64'builtin;
        const default_marker = { __flurry_derivation_default: true };
        enum FirstErr { A, }
        enum SecondErr { B, }
        trait ErrorTag where T {
            fn tag() -> u64;
        }
        ^default_marker
        derive fn handle_closed for ErrorTag<!SecondErr !FirstErr bool> {
            fn tag() -> u64 { 7 }
        }
        struct Subject {}
        derive ErrorTag<!FirstErr !SecondErr bool> for Subject;
        fn run() -> bool { Subject.as(ErrorTag<!FirstErr !SecondErr bool>).tag() == 7 }
    "#;
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("expected NIR artifact");
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );

    let parameterized = r#"
        typealias u64 = .u64'builtin;
        const default_marker = { __flurry_derivation_default: true };
        enum FirstErr { A, }
        enum SecondErr { B, }
        enum ThirdErr { C, }
        trait ErrorTag where T {
            fn tag() -> u64;
        }
        ^default_marker
        derive fn handle_param for ErrorTag<!E !FirstErr bool>
        where E
        {
            fn tag() -> u64 { 11 }
        }
        struct First {}
        struct Second {}
        derive ErrorTag<!FirstErr !SecondErr bool> for First;
        derive ErrorTag<!ThirdErr !FirstErr bool> for Second;
        fn run() -> bool {
            First.as(ErrorTag<!FirstErr !SecondErr bool>).tag() == 11
                and Second.as(ErrorTag<!ThirdErr !FirstErr bool>).tag() == 11
        }
    "#;
    let Artifact::Nir(package) = compile(parameterized, Emit::Nir).unwrap() else {
        panic!("expected NIR artifact");
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );

    let unmatched = r#"
        typealias u64 = .u64'builtin;
        const default_marker = { __flurry_derivation_default: true };
        enum FirstErr { A, }
        enum SecondErr { B, }
        trait ErrorTag where T {
            fn tag() -> u64;
        }
        ^default_marker
        derive fn handle_closed for ErrorTag<!SecondErr !FirstErr bool> {
            fn tag() -> u64 { 7 }
        }
        struct Subject {}
        derive ErrorTag<!FirstErr bool> for Subject;
    "#;
    let error = compile(unmatched, Emit::Nir)
        .err()
        .expect("expected unmatched error-result failure");
    assert!(
        error.contains("no applicable default derivation strategy found"),
        "{error}"
    );
}

#[test]
fn default_strategy_matches_parameterized_computation_effects() {
    let source = r#"
        typealias u32 = .u32'builtin;
        typealias u64 = .u64'builtin;
        const default_marker = { __flurry_derivation_default: true };
        effect op(x: T) -> T where T;
        effect other(x: T) -> T where T;
        trait EffectTag where T {
            fn tag() -> u64;
        }
        ^default_marker
        derive fn handle_eff for EffectTag<#op<T> bool>
        where T
        {
            fn tag() -> u64 { 13 }
        }
        struct Subject {}
        derive EffectTag<#op<u32> bool> for Subject;
        fn run() -> bool { Subject.as(EffectTag<#op<u32> bool>).tag() == 13 }
    "#;
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("expected NIR artifact");
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );

    let wrong_effect = source.replace("op<u32>", "other<u32>");
    let error = compile(&wrong_effect, Emit::Nir)
        .err()
        .expect("expected wrong-effect failure");
    assert!(
        error.contains("no applicable default derivation strategy found"),
        "{error}"
    );
}

#[test]
fn parameterized_default_strategies_enforce_ptc_disjointness_at_registration() {
    // General vs specific default strategies overlap and must be rejected at
    // registration time even when no derive requests the overlapping instance.
    let overlapping_param = r#"
        typealias u32 = .u32'builtin;
        typealias u64 = .u64'builtin;
        const default_marker = { __flurry_derivation_default: true };
        trait Format where T {
            fn tag() -> u64;
        }
        ^default_marker
        derive fn general_fmt for Format<T> where T {
            fn tag() -> u64 { 1 }
        }
        ^default_marker
        derive fn bool_fmt for Format<bool> {
            fn tag() -> u64 { 2 }
        }
        struct Subject {}
        derive Format<u32> for Subject;
    "#;
    let error = compile(overlapping_param, Emit::Nir)
        .err()
        .expect("expected parameterized overlap failure");
    assert!(
        error.contains("ambiguous default derivation strategies")
            && error.contains("not provably disjoint"),
        "{error}"
    );

    // Unordered equivalent ErrorResult default strategies must be caught at registration.
    let overlapping_errors = r#"
        typealias u64 = .u64'builtin;
        const default_marker = { __flurry_derivation_default: true };
        enum FirstErr { A, }
        enum SecondErr { B, }
        trait ErrorTag where T {
            fn tag() -> u64;
        }
        ^default_marker
        derive fn first_err for ErrorTag<!FirstErr !SecondErr bool> {
            fn tag() -> u64 { 1 }
        }
        ^default_marker
        derive fn second_err for ErrorTag<!SecondErr !FirstErr bool> {
            fn tag() -> u64 { 2 }
        }
    "#;
    let error = compile(overlapping_errors, Emit::Nir)
        .err()
        .expect("expected unordered error-result overlap failure");
    assert!(
        error.contains("ambiguous default derivation strategies")
            && error.contains("not provably disjoint"),
        "{error}"
    );

    // Disjoint PTC-constrained parameterized default strategies are accepted
    // and uniquely selected according to their constraints.
    let disjoint_constrained = r#"
        typealias Type = .Type'builtin;
        typealias u32 = .u32'builtin;
        typealias u64 = .u64'builtin;
        const default_marker = { __flurry_derivation_default: true };
        trait Format where T {
            fn tag() -> u64;
        }
        ^default_marker
        derive fn int_fmt for Format<T>
        where T, requires Type.predicates.is_integer(T)
        {
            fn tag() -> u64 { 10 }
        }
        ^default_marker
        derive fn bool_fmt for Format<T>
        where T, requires Type.predicates.is_boolean(T)
        {
            fn tag() -> u64 { 20 }
        }
        struct IntSubj {}
        struct BoolSubj {}
        derive Format<u32> for IntSubj;
        derive Format<bool> for BoolSubj;
        fn run() -> bool {
            IntSubj.as(Format<u32>).tag() == 10 and BoolSubj.as(Format<bool>).tag() == 20
        }
    "#;
    let Artifact::Nir(package) = compile(disjoint_constrained, Emit::Nir).unwrap() else {
        panic!("expected NIR artifact");
    };
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}
