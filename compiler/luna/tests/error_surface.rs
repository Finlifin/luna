use luna::{
    cli::{Emit, Options},
    frontend::{self, Artifact},
};
#[path = "support/primitive_bindings.rs"]
mod primitive_bindings;

use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn compile(source: &str, emit: Emit) -> Result<Artifact, String> {
    let path = std::env::temp_dir().join(format!(
        "error-surface-{}-{}.fl",
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
        iterator_protocol: None,
        into_iterator_protocol: None,
        literal_into_protocol: None,
        arithmetic_protocols: Default::default(),
    });
    std::fs::remove_file(path).unwrap();
    result
}

#[test]
fn error_surface_retains_rows_propagation_and_catch_roles() {
    let source = "enum InputErr {Missing,}
        fn read()->![InputErr] bool {error InputErr.Missing}
        fn forward()->!InputErr bool {read()!}
        fn run()->bool {read()! {
            catch before => {before;},
            InputErr.Missing => false,
            catch after => {after;true},
        }}";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        panic!("HIR")
    };
    for marker in [
        "error-qualified-type",
        "(errors ",
        "(result ",
        "error-new",
        "error-propagate",
        "error-handle",
        "(before (",
        "(after (",
    ] {
        assert!(dump.contains(marker), "missing {marker}: {dump}");
    }
    assert!(!dump.contains("invalid"), "{dump}");
}

#[test]
fn single_catch_is_trailing_and_accepts_patterns() {
    for pattern in ["_", "failure", "error payload", "value!"] {
        let source = format!("fn run(input:bool)->bool {{input! {{catch {pattern} => true,}}}}");
        let Artifact::Hir(dump) = compile(&source, Emit::Hir).unwrap() else {
            panic!("HIR")
        };
        assert!(dump.contains("error-handle"), "{dump}");
        assert!(!dump.contains("invalid"), "{dump}");
    }
}

#[test]
fn ordinary_match_preserves_success_and_error_patterns_and_bindings() {
    let source =
        "fn run(input:bool)->bool {input match {value! => value, error failure => failure,}}";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        panic!("HIR")
    };
    assert!(dump.contains("error-ok"), "{dump}");
    assert!(dump.contains("error-err"), "{dump}");
    assert!(!dump.contains("invalid"), "{dump}");
}

#[test]
fn invalid_catch_positions_and_counts_are_diagnosed() {
    for arms in [
        "catch _ => true, _ => false,",
        "_ => false, catch _ => true, _ => false,",
        "catch _ => true, catch _ => true, catch _ => false,",
    ] {
        let source = format!("fn run(input:bool)->bool {{input! {{{arms}}}}}");
        let failure = compile(&source, Emit::Hir)
            .err()
            .expect("expected rejection");
        assert!(failure.contains("catch"), "{failure}");
    }
}

#[test]
fn catch_bindings_do_not_leak_to_other_arms() {
    let failure = compile(
        "fn run(input:bool)->bool {input! {catch first => first, _ => first, catch last => last,}}",
        Emit::Hir,
    )
    .err()
    .expect("expected rejection");
    assert!(failure.contains("first"), "{failure}");
}

#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

#[test]
fn closed_error_types_form_from_source_and_preserve_qualified_identity() {
    let package = support::compile(
        "enum InputErr {Missing,} enum ParseErr {Invalid,}
        typealias Result=![InputErr,ParseErr,InputErr] bool;
        fn pass(value:Result)->Result {value}",
    );
    let function = package.functions.iter().find(|f| f.name == "pass").unwrap();
    let nir::Term::Function { parameters, result } = &function.signature else {
        panic!("signature")
    };
    assert_eq!(&parameters[0].ty, result.as_ref());
    let nir::Term::ErrorResult { errors, result } = result.as_ref() else {
        panic!("error result")
    };
    assert_eq!(errors.len(), 2);
    assert_ne!(errors[0], errors[1]);
    assert_eq!(result.as_ref(), &nir::Term::Primitive(nir::Primitive::Bool));
}

#[test]
fn empty_error_rows_erase_and_nested_rows_merge_in_source_types() {
    let package = support::compile(
        "enum InputErr {Missing,}
        typealias Empty=![] bool;
        typealias Nested=!InputErr !InputErr bool;
        fn plain(value:Empty)->bool {value}
        fn nested(value:Nested)->!InputErr bool {value}",
    );
    let plain = package
        .functions
        .iter()
        .find(|f| f.name == "plain")
        .unwrap();
    assert!(!plain.signature.to_string().contains("error-result"));
    let nested = package
        .functions
        .iter()
        .find(|f| f.name == "nested")
        .unwrap();
    let nir::Term::Function { result, .. } = &nested.signature else {
        panic!("signature")
    };
    assert!(
        matches!(result.as_ref(),nir::Term::ErrorResult {errors,result} if errors.len()==1 && **result==nir::Term::Primitive(nir::Primitive::Bool))
    );
}

#[test]
fn generic_error_type_labels_preserve_parameter_identity() {
    let package = support::compile(
        "struct FailureErr where T {value:T}
        fn pass(value:!FailureErr<T> T)->!FailureErr<T> T where T {value}",
    );
    let function = package.functions.iter().find(|f| f.name == "pass").unwrap();
    assert!(function.signature.to_string().contains("error-result-type"));
    assert!(
        function
            .signature
            .mentions_rigid(function.type_parameters[0])
    );
}

#[test]
fn error_type_rows_reject_non_types() {
    for source in [
        "typealias Bad=![true] bool;",
        "typealias Bad=!true bool;",
        "typealias Bad=![] true;",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn reordered_closed_error_rows_have_the_same_source_type() {
    support::compile(
        "enum InputErr {Missing,} enum ParseErr {Invalid,}
        fn pass(value:![InputErr,ParseErr] bool)->![ParseErr,InputErr] bool {value}",
    );
}

#[test]
fn empty_error_qualification_executes_as_the_success_type() {
    assert_eq!(
        support::run_iir("fn run()->![] bool {true}", "run", vec![]),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn source_error_rows_do_not_erase_missing_labels_or_change_success_type() {
    for source in [
        "enum FirstErr {Bad,} enum SecondErr {Bad,} fn pass(value:!FirstErr bool)->!SecondErr bool {value}",
        "typealias char=.char'builtin; enum FirstErr {Bad,} fn pass(value:!FirstErr bool)->!FirstErr char {value}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn error_result_constructors_execute_through_iir() {
    use comptime_interpretor::Value;
    for (body, branch, payload) in [("true", None, true), ("error false", Some(0), false)] {
        let source = format!("fn run()->!bool bool {{{body}}}");
        let value = support::run_iir(&source, "run", vec![]);
        let Value::ErrorResult { error, value, .. } = value else {
            panic!("expected tagged result");
        };
        assert_eq!(error, branch);
        assert_eq!(*value, Value::Bool(payload));
    }
}

#[test]
fn error_values_preserve_tags_across_reordering_and_widening() {
    use comptime_interpretor::Value;
    let source = "typealias u8=.u8'builtin; enum A {Failed(bool),} enum B {Failed(bool),}
        fn original()->![A,B] bool {error A.Failed(true)}
        fn reordered()->![B,A] bool {original()}
        fn widened()->![u8,B,A] bool {reordered()}
        fn success()->![B,A] bool {true}
        fn pass(value:![B,A] bool)->![A,B] bool {value}
        fn run()->![A,B] bool {pass(success())}";
    let package = support::compile(source);
    let expected_label = nir::Term::Nominal {
        declaration: package
            .enums
            .iter()
            .find(|item| item.name == "A")
            .unwrap()
            .declaration,
        arguments: vec![],
    };
    for (entry, failed) in [
        ("original", true),
        ("reordered", true),
        ("widened", true),
        ("run", false),
    ] {
        let result = support::run_iir(source, entry, vec![]);
        let Value::ErrorResult { ty, error, value } = result else {
            panic!("result")
        };
        assert_eq!(error.is_some(), failed, "{entry}");
        if failed {
            let nir::Term::ErrorResult { errors, .. } = ty else {
                panic!("result type")
            };
            assert_eq!(errors[error.unwrap()], expected_label, "{entry}");
            let Value::Variant { fields, .. } = *value else {
                panic!("payload")
            };
            assert_eq!(fields, vec![Value::Bool(true)]);
        } else {
            assert_eq!(*value, Value::Bool(true));
        }
    }
}

#[test]
fn error_construction_moves_payload_once_and_selects_only_live_branch() {
    use comptime_interpretor::Value;
    let package = support::compile(
        "fn payload()->bool {true}
        fn skipped()->bool {false}
        fn run()->!bool bool {if true {error payload()} else {skipped()}}",
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    let result = vm.invoke_instance(&graph.roots[0], vec![]).unwrap();
    assert!(matches!(result, Value::ErrorResult { error: Some(0), .. }));
    let events = support::calls(&package, &vm);
    assert_eq!(events.iter().filter(|name| *name == "payload").count(), 1);
    assert!(!events.iter().any(|name| name == "skipped"));
}

#[test]
fn generic_error_construction_and_bottom_success_embedding_execute() {
    use comptime_interpretor::Value;
    let source = "typealias u8=.u8'builtin; fn fail(value:T)->!T bool where T {error value}
        fn run()->![u8,bool] bool {fail(true)}
        fn inferred()->!bool bool {let failed=error false; failed}";
    for (entry, payload) in [("run", true), ("inferred", false)] {
        let Value::ErrorResult { ty, error, value } = support::run_iir(source, entry, vec![])
        else {
            panic!("result")
        };
        let nir::Term::ErrorResult { errors, .. } = ty else {
            panic!("result type")
        };
        assert_eq!(
            errors[error.expect("failure")],
            nir::Term::Primitive(nir::Primitive::Bool)
        );
        assert_eq!(*value, Value::Bool(payload));
    }
}

#[test]
fn error_conversions_reject_missing_labels_and_incompatible_success() {
    for source in [
        "fn run()->!u8 bool {error true}",
        "fn run(value:![bool,u8] bool)->!bool bool {value}",
        "fn run(value:!bool u8)->!bool bool {value}",
        "fn run()->bool {error true}",
    ] {
        assert!(
            support::compile_with_protocols(
                &format!("typealias u8=.u8'builtin; {source}"),
                &Default::default()
            )
            .is_err(),
            "{source}"
        );
    }
}

#[test]
fn contextual_error_payload_and_dump_preserve_the_conversion_kind() {
    let source = "enum Failure {Bad(bool),} fn run()->!Failure bool {error .Bad(true)}";
    let package = support::compile(source);
    let dump = package.to_string();
    assert!(
        dump.contains("(error-convert (conversion failure)"),
        "{dump}"
    );
    let value = support::run_iir(source, "run", vec![]);
    assert!(value.to_string().contains("(branch (error 0))"));
}

#[test]
fn host_error_values_reject_bad_tags_and_payloads_then_recover() {
    use comptime_interpretor::Value;
    let package = support::compile("fn run(value:!bool bool)->!bool bool {value}");
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    let ty = nir::Term::error_result(
        vec![nir::Term::Primitive(nir::Primitive::Bool)],
        nir::Term::Primitive(nir::Primitive::Bool),
    );
    for (index, payload) in [
        (Some(1), Value::Bool(true)),
        (None, Value::Unit),
        (Some(0), Value::Unit),
    ] {
        let value = Value::ErrorResult {
            ty: ty.clone(),
            error: index,
            value: Box::new(payload),
        };
        assert!(vm.validate_immutable_input(&value, &ty).is_err());
    }
    let valid = Value::ErrorResult {
        ty: ty.clone(),
        error: Some(0),
        value: Box::new(Value::Bool(true)),
    };
    vm.validate_immutable_input(&valid, &ty).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![valid.clone()])
            .unwrap(),
        valid
    );
}

#[test]
fn propagation_returns_the_original_error_and_skips_later_calls() {
    use comptime_interpretor::Value;
    for fail in [false, true] {
        let source = format!(
            "fn input()->!bool bool {{if {fail} {{error false}} else {{true}}}}
            fn next(value:bool)->bool {{value}}
            fn run()->!bool bool {{let value=input()!;next(value)}}"
        );
        let package = support::compile(&source);
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        let value = vm.invoke_instance(&graph.roots[0], vec![]).unwrap();
        let Value::ErrorResult { error, value, .. } = value else {
            panic!("result")
        };
        assert_eq!(error, fail.then_some(0));
        assert_eq!(*value, Value::Bool(!fail));
        let calls = support::calls(&package, &vm);
        assert_eq!(calls.iter().filter(|n| *n == "input").count(), 1);
        assert_eq!(
            calls.iter().filter(|n| *n == "next").count(),
            usize::from(!fail)
        );
    }
}

#[test]
fn propagation_can_change_success_type_and_reorder_error_labels() {
    let source = "typealias u8=.u8'builtin;
        enum E {Bad(bool),} enum F {Other,}
        fn input()->![E,F] bool {error E.Bad(true)}
        fn run()->![F,E] u8 {let value=input()!;9}";
    let value = support::run_iir(source, "run", vec![]);
    let comptime_interpretor::Value::ErrorResult { ty, error, value } = value else {
        panic!("result")
    };
    let package = support::compile(source);
    let expected_label = nir::Term::Nominal {
        declaration: package
            .enums
            .iter()
            .find(|item| item.name == "E")
            .unwrap()
            .declaration,
        arguments: vec![],
    };
    let nir::Term::ErrorResult { errors, .. } = ty else {
        panic!("result type")
    };
    assert_eq!(errors[error.expect("failure")], expected_label);
    assert!(
        matches!(*value,comptime_interpretor::Value::Variant {ref fields,..} if fields==&vec![comptime_interpretor::Value::Bool(true)])
    );
}

#[test]
fn propagation_rejects_unrepresentable_errors_and_plain_operands() {
    for source in [
        "fn input()->!bool bool {error true} fn run()->bool {input()!}",
        "enum E {Bad,} fn input()->!bool bool {error true} fn run()->!E bool {input()!}",
        "fn run()->!bool bool {true!}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn propagation_in_inline_functions_returns_to_the_inline_call_boundary() {
    use comptime_interpretor::Value;
    let source = "fn input()->!bool bool {error false}
        inline fn helper()->!bool bool {let value=input()!;value}
        fn run()->bool {helper();true}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_propagation_substitutes_pattern_labels_and_payload_bindings() {
    use comptime_interpretor::Value;
    let source = "fn input(value:T)->!T bool where T {error value}
        fn forward(value:T)->!T bool where T {let success=input(value)!;success}
        fn run()->!bool bool {forward(false)}";
    let Value::ErrorResult { error, value, .. } = support::run_iir(source, "run", vec![]) else {
        panic!("result")
    };
    assert_eq!(error, Some(0));
    assert_eq!(*value, Value::Bool(false));
}

#[test]
fn iir_validates_types_embedded_in_error_patterns() {
    use nir::program::{ExprKind as K, Pattern};
    let package =
        support::compile("fn input()->!bool bool {error true} fn run()->!bool bool {input()!}");
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for invalid in [
        nir::Term::Bool(true),
        nir::Term::Rigid {
            declaration: nir::DeclarationId {
                package: 9,
                local: 9,
            },
            sort: nir::Sort::Type,
        },
    ] {
        let mut broken = graph.clone();
        let mut changed = false;
        for instance in &mut broken.functions {
            nir::visit::expression_mut(&mut instance.function.body, &mut 100_000, |expr, _| {
                if let K::Match { arms, .. } = &mut expr.kind {
                    for arm in arms {
                        if let Pattern::ErrorResult {
                            label: Some(label), ..
                        } = &mut arm.pattern
                        {
                            *label = invalid.clone();
                            changed = true;
                        }
                    }
                }
                Ok(())
            })
            .unwrap();
        }
        assert!(changed);
        assert!(broken.validate(&mut 100_000).is_err());
    }
    graph.validate(&mut 100_000).unwrap();
}

#[test]
fn source_match_delivers_success_or_error_payload() {
    use comptime_interpretor::Value;
    for fail in [false, true] {
        let source = format!(
            "fn input()->!bool bool {{if {fail} {{error false}} else {{true}}}}
            fn run()->bool {{input() match {{value! => value, error rejected => rejected,}}}}"
        );
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(!fail));
    }
}

#[test]
fn source_match_distinguishes_multiple_full_error_types() {
    use comptime_interpretor::Value;
    for (constructor, expected) in [
        ("error E.Bad(true)", true),
        ("error F.Bad(false)", false),
        ("true", false),
    ] {
        let source=format!("enum E {{Bad(bool),}} enum F {{Bad(bool),}}
            fn input()->![F,E] bool {{{constructor}}}
            fn run()->bool {{input() match {{_! => false, error E.Bad(value) => value, error F.Bad(value) => value,}}}}");
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(expected)
        );
    }
}

#[test]
fn error_wildcard_covers_errors_without_covering_success() {
    use comptime_interpretor::Value;
    let source = "enum E {Bad,} enum F {Bad,}
        fn input()->![E,F] bool {error F.Bad}
        fn run()->bool {input() match {_! => false,error _ => true,}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    for arms in [
        "error _ => true,",
        "_! => true,",
        "true! => true, error _ => false,",
        "_! => true,error true => false,",
    ] {
        let source = format!("fn run(value:!bool bool)->bool {{value match {{{arms}}}}}");
        let failure = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(failure.contains("exhaustive"), "{failure}");
    }
}

#[test]
fn error_pattern_coverage_checks_each_payload_domain() {
    use comptime_interpretor::Value;
    let source="fn input()->!bool bool {error false}
        fn run()->bool {input() match {true! => true,false! => false,error true => true,error false => false,}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(false));
}

#[test]
fn error_pattern_guards_rollback_and_generic_bindings_are_retained() {
    use comptime_interpretor::Value;
    let source="fn recover(value:!T bool,fallback:T)->T where T {value match {error payload => payload, _! => fallback,}}
        fn input()->!bool bool {error false}
        fn run()->bool {input() match {(error rejected if rejected) => true,error other => recover(input(),other),_! => true,}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(false));
}

#[test]
fn multiple_error_rows_accept_named_variant_payload_patterns() {
    use comptime_interpretor::Value;
    let source="enum E {Bad {value:bool},} enum F {Bad,}
        fn input()->![E,F] bool {error E.Bad{value:true}}
        fn run()->bool {input() match {_! => false,error E.Bad{value} => value,error F.Bad => false,}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn source_error_patterns_reject_wrong_domains_and_missing_labels() {
    for (source, reason) in [
        (
            "fn run(value:bool)->bool {value match {payload! => payload,error _=>false,}}",
            "success pattern requires",
        ),
        (
            "enum E {Bad,} enum F {Bad,} enum G {Bad,} fn run(value:![E,F] bool)->bool {value match {_! =>true,error G.Bad=>false,error _=>false,}}",
            "absent",
        ),
        (
            "enum E {Bad,} enum F {Bad,} fn run(value:![E,F] bool)->bool {value match {_! =>true,error payload=>false,}}",
            "multiple error types",
        ),
    ] {
        let failure = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(failure.contains(reason), "{failure}");
    }
}

#[test]
fn error_patterns_distinguish_instances_of_the_same_generic_error_type() {
    use comptime_interpretor::Value;
    let source="typealias u8=.u8'builtin; enum E where T {Bad(T),}
        fn input()->![E<u8>,E<bool>] bool {error E<bool>.Bad(true)}
        fn run()->bool {input() match {_! => false,error E<bool>.Bad(value) => value,error E<u8>.Bad(_) => false,}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn success_patterns_support_let_else_without_claiming_irrefutability() {
    use comptime_interpretor::Value;
    for fail in [false, true] {
        let source = format!(
            "fn input()->!bool bool {{if {fail} {{error true}} else {{true}}}}
            fn run()->bool {{let payload! = input() else {{return false;}};payload}}"
        );
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(!fail));
    }
    let source =
        "fn input()->!bool bool {error true} fn run()->bool {let payload! = input();payload}";
    assert!(support::compile_with_protocols(source, &Default::default()).is_err());
}

#[test]
fn ordinary_error_handler_selects_payload_and_bypasses_arms_on_success() {
    use comptime_interpretor::Value;
    for fail in [false, true] {
        let source = format!(
            "enum E {{Bad(bool),}}
            fn input()->!E bool {{if {fail} {{error E.Bad(false)}} else {{true}}}}
            fn arm(value:bool)->bool {{value}}
            fn run()->bool {{input()! {{.Bad(value)=>arm(value),}}}}"
        );
        let package = support::compile(&source);
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            Value::Bool(!fail)
        );
        let calls = support::calls(&package, &vm);
        assert_eq!(calls.iter().filter(|n| *n == "input").count(), 1);
        assert_eq!(
            calls.iter().filter(|n| *n == "arm").count(),
            usize::from(fail)
        );
    }
}

#[test]
fn partial_error_handler_preserves_unmatched_payloads_in_its_result() {
    use comptime_interpretor::Value;
    for constructor in ["E.Bad(false)", "E.Other(true)"] {
        let source = format!(
            "enum E {{Bad(bool),Other(bool),}}
            fn input()->!E bool {{error {constructor}}}
            fn run()->!E bool {{input()! {{.Bad(value)=>value,}}}}"
        );
        let Value::ErrorResult { error, value, .. } = support::run_iir(&source, "run", vec![])
        else {
            panic!("result")
        };
        if constructor.starts_with("E.Bad") {
            assert_eq!(error, None);
            assert_eq!(*value, Value::Bool(false));
        } else {
            assert_eq!(error, Some(0));
            assert!(
                matches!(*value,Value::Variant{index:1,ref fields,..} if fields==&vec![Value::Bool(true)])
            );
        }
    }
}

#[test]
fn handler_output_errors_are_independent_of_consumed_input_labels() {
    use comptime_interpretor::Value;
    let source = "enum E {Bad(bool),}
        fn input()->!E bool {error E.Bad(false)}
        fn run()->!E bool {let result=input()! {.Bad(value)=>error E.Bad(not value),};result}";
    let Value::ErrorResult { error, value, .. } = support::run_iir(source, "run", vec![]) else {
        panic!("result")
    };
    assert_eq!(error, Some(0));
    assert!(matches!(*value,Value::Variant{ref fields,..} if fields==&vec![Value::Bool(true)]));
}

#[test]
fn handler_inference_removes_fully_covered_labels_only() {
    use comptime_interpretor::Value;
    let source = "enum E {Bad,} enum F {Bad(bool),}
        fn input()->![E,F] bool {error F.Bad(true)}
        fn run()->!F bool {let result=input()! {E.Bad=>false,};result}";
    let Value::ErrorResult { ty, error, .. } = support::run_iir(source, "run", vec![]) else {
        panic!("result")
    };
    let nir::Term::ErrorResult { errors, .. } = ty else {
        panic!("type")
    };
    assert_eq!(errors.len(), 1);
    assert_eq!(error, Some(0));
}

#[test]
fn guarded_handlers_preserve_errors_when_the_guard_fails() {
    use comptime_interpretor::Value;
    let source = "fn input()->!bool bool {error false}
        fn guard(value:bool)->bool {value}
        fn run()->!bool bool {let result=input()! {(value if guard(value))=>true,};result}";
    let package = support::compile(source);
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    let Value::ErrorResult { error, value, .. } =
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap()
    else {
        panic!("result")
    };
    assert_eq!(error, Some(0));
    assert_eq!(*value, Value::Bool(false));
    assert_eq!(
        support::calls(&package, &vm)
            .iter()
            .filter(|n| *n == "guard")
            .count(),
        1
    );
}

#[test]
fn handler_infers_new_output_labels_and_plain_exhaustive_results() {
    use comptime_interpretor::Value;
    let source = "enum E {Bad,} enum F {Bad(bool),}
        fn input()->!E bool {error E.Bad}
        fn translated()->!F bool {let result=input()! {.Bad=>error F.Bad(true),};result}
        fn recovered()->bool {let result=input()! {_=>true,};result}";
    assert_eq!(
        support::run_iir(source, "recovered", vec![]),
        Value::Bool(true)
    );
    let Value::ErrorResult { ty, error, .. } = support::run_iir(source, "translated", vec![])
    else {
        panic!("result")
    };
    let nir::Term::ErrorResult { errors, .. } = ty else {
        panic!("type")
    };
    assert_eq!(errors.len(), 1);
    assert_eq!(error, Some(0));
}

#[test]
fn handler_explicit_return_preserves_the_enclosing_inline_boundary() {
    use comptime_interpretor::Value;
    let source = "fn input()->!bool bool {error true}
        inline fn inner()->bool {input()! {_=>return false,};true}
        fn run()->bool {inner();true}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn handlers_reject_unrepresented_residual_errors_and_incompatible_success() {
    for source in [
        "enum E {One,Two,} fn input()->!E bool {error E.Two} fn run()->bool {input()! {.One=>false,}}",
        "typealias u8=.u8'builtin; fn input()->!bool bool {error true} fn run()->bool {input()! {_=>9u8,}}",
    ] {
        assert!(
            support::compile_with_protocols(source, &Default::default()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn error_catches_run_in_order_only_on_error_paths() {
    use comptime_interpretor::Value;
    for failed in [true, false] {
        let source="enum E {Bad,} fn before() {} fn arm() {} fn after()->bool {false}
            fn input(failed:bool)->!E bool {if failed {error E.Bad}else{true}}
            fn run(failed:bool)->bool {input(failed)! {catch _ => before(),.Bad=>arm(),catch _=>after(),}}";
        let package = support::compile(source);
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![Value::Bool(failed)])
                .unwrap(),
            Value::Bool(!failed)
        );
        assert_eq!(
            support::calls(&package, &vm),
            if failed {
                vec!["run", "input", "before", "arm", "after"]
            } else {
                vec!["run", "input"]
            }
        );
    }
}

#[test]
fn trailing_catch_receives_original_result_even_without_an_ordinary_match() {
    use comptime_interpretor::Value;
    let source = "enum E {Bad(bool),Other,}
        fn run()->!E bool {(error E.Bad(true))! {.Other=>false,catch original=>original,}}";
    let Value::ErrorResult { error, value, .. } = support::run_iir(source, "run", vec![]) else {
        panic!("error result")
    };
    assert_eq!(error, Some(0));
    assert!(
        matches!(*value,Value::Variant {index:0,ref fields,..} if fields==&vec![Value::Bool(true)])
    );
}

#[test]
fn partial_trailing_catch_preserves_unmatched_errors() {
    use comptime_interpretor::Value;
    for failed in [true, false] {
        let source = "fn run(failed:bool)->!bool bool {(error failed)! {catch error true=>true,}}";
        let Value::ErrorResult { error, value, .. } =
            support::run_iir(source, "run", vec![Value::Bool(failed)])
        else {
            panic!("result")
        };
        assert_eq!(error, if failed { None } else { Some(0) });
        assert_eq!(*value, Value::Bool(failed));
    }
}

#[test]
fn catch_coverage_accounts_for_earlier_explicit_exits_without_ignoring_shadowing() {
    use comptime_interpretor::Value;
    for arms in [
        "catch _=>return true,catch error true=>false",
        "_=>return true,catch error true=>false",
    ] {
        assert_eq!(
            support::run_iir(
                &format!("fn run()->bool {{(error false)! {{{arms}}}}}"),
                "run",
                vec![]
            ),
            Value::Bool(true)
        );
    }
    let failure = compile(
        "fn run()->bool {(error false)! {_=>{},_=>return true,catch error true=>false,}}",
        Emit::Nir,
    )
    .err()
    .expect("unhandled error must be rejected");
    assert!(
        failure.contains("error") || failure.contains("type"),
        "{failure}"
    );
}

#[test]
fn generic_catch_returns_the_original_instantiated_error_type() {
    use comptime_interpretor::Value;
    let source = "enum Failure {Bad(bool),}
        fn preserve(input:!T bool)->!T bool where T {input! {catch original=>original,}}
        fn run()->!Failure bool {preserve<Failure>(error Failure.Bad(true))}";
    let Value::ErrorResult { error, value, .. } = support::run_iir(source, "run", vec![]) else {
        panic!("result")
    };
    assert_eq!(error, Some(0));
    assert!(matches!(*value,Value::Variant {ref fields,..} if fields==&vec![Value::Bool(true)]));
}

#[test]
fn inferred_error_callback_completes_success_and_failure_exits() {
    use comptime_interpretor::Value;
    for flag in ["true", "false"] {
        let source = format!(
            "fn input(flag:bool)->!bool bool {{if flag {{true}} else {{error false}}}}
            fn run()->bool {{let callback=|flag:bool| {{input(flag)!}};
                callback({flag}) match {{value! => value,error failure=>not failure}}}}"
        );
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
    }
}

#[test]
fn inferred_error_callback_unions_multiple_labels_with_tail_row() {
    use comptime_interpretor::Value;
    for (first, second) in [("false", "false"), ("true", "false"), ("true", "true")] {
        let source = format!("enum A {{Bad(bool)}} enum B {{Bad(bool)}} enum C {{Bad(bool)}}
            fn first(flag:bool)->!A bool {{if flag {{true}} else {{error A.Bad(true)}}}}
            fn second(flag:bool)->!B bool {{if flag {{true}} else {{error B.Bad(true)}}}}
            fn run()->bool {{let callback=|a:bool,b:bool| {{first(a)!;second(b)!;error C.Bad(true)}};
                callback({first},{second}) match {{error A.Bad(v)=>v,error B.Bad(v)=>v,error C.Bad(v)=>v}}}}");
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
    }
}

#[test]
fn inferred_error_callback_does_not_leak_nested_return_labels() {
    use comptime_interpretor::Value;
    let source = "enum A {Bad(bool)} enum B {Bad(bool)}
        fn first()->!A bool {error A.Bad(true)} fn second()->!B bool {error B.Bad(true)}
        fn run()->bool {let outer=|| {let inner=|| {second()!};
            let local=inner()! {B.Bad(v)=>v};first()!;local};
            outer()! {A.Bad(v)=>v}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn inferred_error_callback_rejects_non_error_operand_and_wrong_target() {
    for source in [
        "fn run()->bool {let callback=|| {true!};callback()}",
        "fn fail()->!bool bool {error true} fn run()->bool {let callback=|| {fail()!};callback()}",
        "enum A {Bad} enum B {Bad} fn fail()->!A bool {error A.Bad}
            fn run()->!B bool {let callback=|| {fail()!};callback()}",
    ] {
        assert!(compile(source, Emit::Nir).is_err(), "{source}");
    }
}

#[test]
fn inferred_error_callback_adapts_explicit_success_returns() {
    use comptime_interpretor::Value;
    for flag in ["true", "false"] {
        let source = format!(
            "fn fail()->!bool bool {{error true}}
            fn run()->bool {{let callback=|flag:bool| {{if flag {{return true;}}fail()!}};
                callback({flag}) match {{value! => value,error failure=>failure}}}}"
        );
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
    }
    assert!(
        compile(
            "fn fail()->!bool bool {error true}
        fn run()->bool {let callback=|flag:bool| {if flag {return;}fail()!};true}",
            Emit::Nir
        )
        .is_err()
    );
}

#[test]
fn inferred_error_callback_is_a_generic_call_result() {
    use comptime_interpretor::Value;
    for modifier in ["", "inline "] {
        let source = format!(
            "fn fail()->!bool bool {{error true}}
            {modifier}fn apply(action:fn()->T)->T where T {{action()}}
            fn run()->bool {{apply(|| {{fail()!}})! {{failure=>failure}}}}"
        );
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
    }
}

#[test]
fn inferred_error_callback_keeps_contextual_effect_row() {
    use comptime_interpretor::Value;
    for flag in ["true", "false"] {
        for modifier in ["", "inline "] {
            let source = format!("effect read()->bool;
                fn checked(flag:bool)->!bool bool {{if flag {{true}} else {{error true}}}}
                {modifier}fn apply(action:fn()->#read T)->#read T where T {{action()}}
                fn run()->bool {{let result=apply(|| {{let flag=read()#;checked(flag)!}})# {{read()=>{flag}}};
                    result! {{failure=>failure}}}}");
            assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
        }
    }
}

#[test]
fn inferred_error_callback_effect_row_preserves_explicit_return() {
    use comptime_interpretor::Value;
    let source = "effect read()->bool;
        fn fail()->!bool bool {error false}
        fn apply(action:fn()->#read T)->#read T where T {action()}
        fn run()->bool {let result=apply(|| {let flag=read()#;if flag {return true;}fail()!})# {read()=>true};
            result! {failure=>failure}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn inferred_error_callback_effect_context_does_not_authorize_other_effects() {
    let source = "effect read()->bool;effect write()->bool;
        fn fail()->!bool bool {error true}
        fn apply(action:fn()->#read T)->#read T where T {action()}
        fn run()->bool {let result=apply(|| {write()#;fail()!})# {read()=>true};result! {failure=>failure}}";
    let failure = compile(source, Emit::Nir)
        .err()
        .expect("reject unlisted effect");
    assert!(!failure.contains("parsing failed"), "{failure}");
}

#[test]
fn inferred_error_effect_context_crosses_generic_callable_forwarding() {
    use comptime_interpretor::Value;
    let source = "effect read()->bool;
        fn checked(flag:bool)->!bool bool {if flag {true} else {error true}}
        fn relay(value:T)->T where T {value}
        fn apply(action:fn()->#read T)->#read T where T {action()}
        fn run()->bool {let result=apply(relay(|| {let flag=read()#;checked(flag)!}))# {read()=>false};
            result! {failure=>failure}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn partial_error_callback_context_injects_plain_success() {
    use comptime_interpretor::Value;
    let source = "enum Failure {Bad(bool)}
        fn apply(action:fn()->!Failure T)->!Failure T where T {action()}
        fn run()->bool {apply(|| true)! {Failure.Bad(value)=>value}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn partial_error_callback_context_checks_both_branch_orders() {
    use comptime_interpretor::Value;
    for body in [
        "if flag {error .Bad(true)} else {true}",
        "if flag {true} else {error .Bad(true)}",
    ] {
        for flag in ["true", "false"] {
            let source = format!(
                "enum Failure {{Bad(bool)}}
                fn apply(action:fn(bool)->!Failure T,flag:bool)->!Failure T where T {{action(flag)}}
                fn run()->bool {{apply(|flag| {{{body}}},{flag})! {{Failure.Bad(value)=>value}}}}"
            );
            assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
        }
    }
}

#[test]
fn partial_error_callback_context_completes_explicit_failure_returns() {
    use comptime_interpretor::Value;
    for body in [
        "if flag {return error .Bad(true);}true",
        "if flag {return error .Bad(true);}return true;",
    ] {
        for flag in ["true", "false"] {
            let source = format!(
                "enum Failure {{Bad(bool)}}
                fn apply(action:fn(bool)->!Failure T,flag:bool)->!Failure T where T {{action(flag)}}
                fn run()->bool {{apply(|flag| {{{body}}},{flag})! {{Failure.Bad(value)=>value}}}}"
            );
            assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
        }
    }
}

#[test]
fn partial_error_callback_context_composes_with_effect_row() {
    use comptime_interpretor::Value;
    let source="enum Failure {Bad(bool)} effect read()->bool;
        fn apply(action:fn()->#read !Failure T)->#read !Failure T where T {action()}
        fn run()->bool {let result=apply(|| {if (read()#) {error .Bad(true)} else {true}})# {read()=>true};
            result! {Failure.Bad(value)=>value}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn partial_error_callback_context_rejects_labels_outside_declared_row() {
    for body in [
        "error Other.Bad",
        "fail()!",
        "if true {return error Other.Bad;}true",
    ] {
        let source = format!(
            "enum Allowed {{Bad}} enum Other {{Bad}}
            fn fail()->!Other bool {{error Other.Bad}}
            fn apply(action:fn()->!Allowed T)->!Allowed T where T {{action()}}
            fn run()->bool {{apply(|| {{{body}}})! {{Allowed.Bad=>true}}}}"
        );
        let failure = compile(&source, Emit::Nir)
            .err()
            .expect("reject extra label");
        assert!(!failure.contains("parsing failed"), "{failure}");
    }
}

#[test]
fn partial_error_callback_context_reaches_match_when_and_generic_forwarding() {
    use comptime_interpretor::Value;
    for body in [
        "flag match {true=>error .Bad(true),false=>true}",
        "when {flag=>error .Bad(true),else=>true}",
        "if flag {true} else {error .Bad(true)}",
    ] {
        for flag in ["true", "false"] {
            let source=format!("enum Failure {{Bad(bool)}}
                fn relay(value:T)->T where T {{value}}
                fn apply(action:fn(bool)->!Failure T,flag:bool)->!Failure T where T {{action(flag)}}
                fn run()->bool {{apply(relay(|flag| {{{body}}}),{flag})! {{Failure.Bad(value)=>value}}}}");
            assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
        }
    }
}

#[test]
fn generic_error_forwarding_infers_singleton_label_and_payload() {
    use comptime_interpretor::Value;
    for flag in ["true", "false"] {
        let source = format!(
            "enum Failure {{Bad(bool)}}
            fn sample()->!Failure bool {{if {flag} {{true}} else {{error Failure.Bad(true)}}}}
            fn forward(value:!E T)->!E T where E,T {{value}}
            fn run()->bool {{forward(sample())! {{Failure.Bad(value)=>value}}}}"
        );
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
    }
}

#[test]
fn generic_error_labels_match_nominal_heads_across_row_permutations() {
    use comptime_interpretor::Value;
    for row in ["Failure<bool>,Other", "Other,Failure<bool>"] {
        let source=format!("enum Failure where T {{Bad(T)}} enum Other {{Bad}}
            fn sample()->![{row}] bool {{error Failure<bool>.Bad(true)}}
            fn forward(value:![Failure<T>,Other] bool)->![Other,Failure<T>] bool where T {{value}}
            fn run()->bool {{forward(sample())! {{Failure<bool>.Bad(value)=>value,Other.Bad=>false}}}}");
        assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
    }
}

#[test]
fn generic_error_labels_do_not_choose_ambiguous_permutations_or_unfold_heads() {
    for row in ["bool,.char'builtin", ".char'builtin,bool"] {
        let source = format!(
            "fn sample()->![{row}] bool {{true}}
            fn forward(value:![E,F] bool)->![E,F] bool where E,F {{value}}
            fn run()->bool {{let result=forward(sample());true}}"
        );
        let failure = compile(&source, Emit::Nir).err().expect("ambiguous labels");
        assert!(!failure.contains("parsing failed"), "{failure}");
    }
    let source = "pure comptime fn Wrap<T>->.Type'builtin {T}
        fn forward(value:!Wrap<T> bool)->!Wrap<T> bool where T {value}
        fn run(value:!Wrap<bool> bool)->!Wrap<bool> bool {forward(value)}";
    let failure = compile(source, Emit::Nir)
        .err()
        .expect("opaque head cannot infer its argument");
    assert!(!failure.contains("parsing failed"), "{failure}");
}

#[test]
fn generic_error_row_inference_uses_later_argument_evidence() {
    use comptime_interpretor::Value;
    let source = "fn sample()->![.char'builtin,bool] bool {error 'x'}
        fn forward(value:![E,T] bool,seed:T)->![E,T] bool where E,T {value}
        fn run()->bool {forward(sample(),true)! {_=>true}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn inferred_error_rows_cross_spread_and_method_call_boundaries() {
    use comptime_interpretor::Value;
    let prefix="enum Failure where T {Bad(T)} enum Other {Bad}
        fn sample()->![Other,Failure<bool>] bool {error Failure<bool>.Bad(true)}
        fn forward(value:![Failure<T>,Other] bool)->![Other,Failure<T>] bool where T {value}
        struct Worker {} impl Worker {
            fn forward(value:![Failure<T>,Other] bool)->![Other,Failure<T>] bool where T {value}
        }
        trait Forward {fn forward(value:![Failure<T>,Other] bool)->![Other,Failure<T>] bool where T;}
        impl Forward for Worker {
            fn forward(value:![Failure<T>,Other] bool)->![Other,Failure<T>] bool where T {value}
        }";
    for call in [
        "forward(...(sample(),))",
        "Worker.forward(sample())",
        "Worker.as(Forward).forward(sample())",
    ] {
        let source = format!(
            "{prefix} fn run()->bool {{{call}! {{Failure<bool>.Bad(value)=>value,Other.Bad=>false}}}}"
        );
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(true),
            "{call}"
        );
    }
}

#[test]
fn inferred_error_rows_inside_tuple_arguments_remap_each_payload() {
    use comptime_interpretor::Value;
    let source = "enum Failure where T {Bad(T)} enum Other {Bad}
        fn sample()->![Other,Failure<bool>] bool {error Failure<bool>.Bad(true)}
        fn packet()->Tuple<![Other,Failure<bool>] bool,bool> {(sample(),true)}
        fn forward(value:Tuple<![Failure<T>,Other] bool,bool>)->bool where T {
            let (result,flag)=value;
            result! {Failure<T>.Bad(value)=>flag,Other.Bad=>false}
        }
        fn run()->bool {forward(packet())}";
    assert_eq!(
        support::run_iir_with_tuple(source, "run", vec![]),
        Value::Bool(true)
    );
}

#[test]
fn inferred_error_rows_inside_nested_products_preserve_argument_evaluation() {
    use comptime_interpretor::{Interpreter, Value};
    let source = "enum Failure where T {Bad(T)} enum Other {Bad}
        fn sample()->![Other,Failure<bool>] bool {error Failure<bool>.Bad(true)}
        fn packet()->Tuple<Tuple<![Other,Failure<bool>] bool,bool>,bool> {((sample(),true),true)}
        fn forward(value:Tuple<Tuple<![Failure<T>,Other] bool,bool>,bool>)->bool where T {
            let ((result,first),second)=value;
            result! {Failure<T>.Bad(_)=>first and second,Other.Bad=>false}
        }
        fn run()->bool {forward(packet())}";
    let package = support::compile_with_tuple(source);
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let calls = support::calls(&package, &vm);
    assert_eq!(
        calls
            .iter()
            .filter(|name| name.as_str() == "sample")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|name| name.as_str() == "packet")
            .count(),
        1
    );
}

#[test]
fn inferred_error_rows_inside_record_arguments_remap_payloads() {
    use comptime_interpretor::Value;
    let source = "fn record(...fields:T)->T where T {fields}
        enum Failure where T {Bad(T)} enum Other {Bad}
        fn sample()->![Other,Failure<bool>] bool {error Failure<bool>.Bad(true)}
        fn expected()->![Failure<T>,Other] bool where T {true}
        fn forward(value:(record {result:expected<T>(),flag:true})'type)->bool where T {
            value.result! {Failure<T>.Bad(_)=>value.flag,Other.Bad=>false}
        }
        fn run()->bool {forward(record {result:sample(),flag:true})}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn inferred_error_product_conversion_preserves_bound_tuple_head() {
    use comptime_interpretor::{Interpreter, Value};
    let source = "typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        enum Failure where T {Bad(T)} enum Other {Bad}
        fn sample()->![Other,Failure<bool>] bool {error Failure<bool>.Bad(true)}
        fn packet()->Tuple<![Other,Failure<bool>] bool,bool> {(sample(),true)}
        fn forward(value:Tuple<![Failure<T>,Other] bool,bool>)->bool where T {
            let (result,flag)=value;result! {Failure<T>.Bad(_)=>flag,Other.Bad=>false}
        }
        fn run()->bool {forward(packet())}";
    let package = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            tuple: Some("input.Tuple"),
            ..Default::default()
        },
    )
    .unwrap();
    let forward = package
        .functions
        .iter()
        .find(|f| f.name == "forward")
        .unwrap();
    let nir::Term::Function { parameters, .. } = &forward.signature else {
        panic!("function")
    };
    assert!(matches!(parameters[0].ty, nir::Term::NormalForm { .. }));
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn inferred_error_product_conversion_rejects_incompatible_nested_rows() {
    let source = "enum Failure where T {Bad(T)} enum Other {Bad}
        fn sample()->![Other,Failure<bool>] bool {error Failure<bool>.Bad(true)}
        fn packet()->Tuple<![Other,Failure<bool>] bool,bool> {(sample(),true)}
        fn forward(value:Tuple<!Failure<T> bool,bool>)->bool where T {true}
        fn run()->bool {forward(packet())}";
    let failure = compile(source, Emit::Nir)
        .err()
        .expect("reject incompatible nested error row");
    assert!(!failure.contains("parsing failed"), "{failure}");
}

#[test]
fn normal_form_inputs_freeze_error_values_with_full_branch_identity() {
    let package = support::compile(
        "typealias Type=.Type'builtin; pure comptime fn ResultKey<value: !bool bool> -> Type {bool}
         pure comptime fn success() -> !bool bool {true}
         pure comptime fn failure() -> !bool bool {error false}
         typealias Success = ResultKey<success()>;
         typealias Failure = ResultKey<failure()>;
         fn success_input(value: Success) -> Success {value}
         fn failure_input(value: Failure) -> Failure {value}",
    );
    let dump = package.to_string();
    assert!(dump.contains("(error-value"), "{dump}");
    assert!(dump.contains("(branch success)"), "{dump}");
    assert!(dump.contains("(branch (error (primitive bool)))"), "{dump}");
}

#[test]
fn frozen_error_input_retains_nominal_payload_and_opaque_head() {
    let source = "typealias Type=.Type'builtin;
        enum Failure where T {Bad(T)}
        pure comptime fn Key<value: !Failure<bool> bool>->Type {bool}
        pure comptime fn sample()->!Failure<bool> bool {error Failure<bool>.Bad(true)}
        fn pass(value:Key<sample()>)->Key<sample()> {value}";
    let package = support::compile(source);
    let dump = package.to_string();
    assert!(dump.contains("(error-value"), "{dump}");
    assert!(dump.contains("(nominal-value"), "{dump}");
    let invalid = format!("{source} fn wrong(value:Key<sample()>)->bool {{value}}");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn staged_error_inputs_reconstruct_success_and_failure_branches() {
    use comptime_interpretor::Value;
    for (body, expected) in [("true", true), ("error false", false)] {
        let source = format!(
            "pure comptime fn sample()->!bool bool {{{body}}}
            fn inspect(comptime value:!bool bool)->bool {{value! {{bad=>bad}}}}
            fn run()->bool {{inspect(sample())}}"
        );
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(expected)
        );
    }
}

#[test]
fn frozen_success_and_failure_with_equal_payloads_remain_distinct_inputs() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn Key<value:!bool bool>->Type {bool}
        pure comptime fn good()->!bool bool {true}
        pure comptime fn bad()->!bool bool {error true}
        fn wrong(value:Key<good()>)->Key<bad()> {value}";
    assert!(support::compile_with_protocols(source, &Default::default()).is_err());
}

#[test]
fn staged_generic_product_arguments_transport_nested_error_rows() {
    use comptime_interpretor::Value;
    for body in ["error Failure<bool>.Bad(true)", "true"] {
        let source = format!("enum Failure where T {{Bad(T)}} enum Other {{Bad}}
            pure comptime fn sample()->![Other,Failure<bool>] bool {{{body}}}
            pure comptime fn packet()->Tuple<Tuple<![Other,Failure<bool>] bool,bool>,bool> {{((sample(),true),true)}}
            fn inspect(comptime value:Tuple<Tuple<![Failure<T>,Other] bool,bool>,bool>)->bool where T {{
                let ((result,first),second)=value;
                result! {{Failure<T>.Bad(_)=>true,Other.Bad=>false}} and first and second
            }}
            fn run()->bool {{inspect(packet())}}");
        assert_eq!(
            support::run_iir_with_tuple(&source, "run", vec![]),
            Value::Bool(true)
        );
    }
}

#[test]
fn error_value_trait_indices_reconstruct_both_branches_in_selected_bodies() {
    use comptime_interpretor::Value;
    let source = "pure comptime fn good()->!bool bool {true}
        pure comptime fn bad()->!bool bool {error false}
        trait Read where tag:!bool bool {fn read()->bool;}
        impl Read<tag> for bool where tag:!bool bool {fn read()->bool {tag! {bad=>bad}}}
        fn run()->bool {bool.as(Read<good()>).read() and not bool.as(Read<bad()>).read()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn record_trait_indices_preserve_nested_error_branches() {
    use comptime_interpretor::Value;
    let source = "pure comptime fn record(...fields:T)->T where T {fields}
        pure comptime fn good()->!bool bool {true}
        pure comptime fn bad()->!bool bool {error false}
        typealias Config=(record {result:good(),flag:true})'type;
        pure comptime fn first()->Config {record {result:good(),flag:true}}
        pure comptime fn second()->Config {record {result:bad(),flag:true}}
        trait Read where config:Config {fn read()->bool;}
        impl Read<config> for bool where config:Config {
            fn read()->bool {config.result! {bad=>bad} and config.flag}
        }
        fn run()->bool {bool.as(Read<first()>).read() and not bool.as(Read<second()>).read()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn concrete_error_indices_select_disjoint_trait_implementations() {
    use comptime_interpretor::Value;
    let prefix = "pure comptime fn good()->!bool bool {true}
        pure comptime fn bad()->!bool bool {error true}
        trait Read where tag:!bool bool {fn read()->bool;}
        impl Read<good()> for bool {fn read()->bool {true}}
        impl Read<bad()> for bool {fn read()->bool {false}}";
    let source = format!(
        "{prefix} fn run()->bool {{bool.as(Read<good()>).read() and not bool.as(Read<bad()>).read()}}"
    );
    assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
    let duplicate = format!("{prefix} impl Read<good()> for bool {{fn read()->bool {{false}}}}");
    assert!(support::compile_with_protocols(&duplicate, &Default::default()).is_err());
}

#[test]
fn record_error_trait_indices_admit_nominal_payloads_with_layout_evidence() {
    use comptime_interpretor::Value;
    let source = "enum Failure where T {Bad(T)}
        pure comptime fn record(...fields:T)->T where T {fields}
        pure comptime fn sample()->!Failure<bool> bool {error Failure<bool>.Bad(true)}
        typealias Config=(record {result:sample(),flag:true})'type;
        pure comptime fn config()->Config {record {result:sample(),flag:true}}
        trait Read where data:Config {fn read()->bool;}
        impl Read<data> for bool where data:Config {
            fn read()->bool {data.result! {Failure<bool>.Bad(payload)=>payload} and data.flag}
        }
        fn run()->bool {bool.as(Read<config()>).read()}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn associated_error_labels_survive_generic_trait_calls() {
    use comptime_interpretor::Value;
    let source = "trait Source {assoc Failure:.Type'builtin;fn sample()->!Failure bool;}
        impl Source for bool {assoc Failure:.Type'builtin=bool;fn sample()->!bool bool {error false}}
        fn forward()->!T.as(Source).Failure bool where T:-Source {T.as(Source).sample()}
        fn run()->bool {not forward<bool>()! {bad=>bad}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    let mismatch = source.replace(
        "Failure:.Type'builtin=bool",
        "Failure:.Type'builtin='x''type",
    );
    assert!(support::compile_with_protocols(&mismatch, &Default::default()).is_err());
}

#[test]
fn generic_comptime_error_inputs_resume_with_completed_success_domains() {
    let source = "typealias Type=.Type'builtin; typealias Char='x''type;
        pure comptime fn Key<T,value:!Char T>->Type {bool}
        pure comptime fn Wrap(value:T)->Type where T {Key<T,value>}
        pure comptime fn good()->!bool bool {true}
        pure comptime fn bad()->!bool bool {error false}
        typealias Good=Wrap(good()); typealias Bad=Wrap(bad());
        typealias DirectGood=Key<!bool bool,good()>;
        typealias DirectBad=Key<!bool bool,bad()>;
        fn keep_good(value:Good)->DirectGood {value}
        fn keep_bad(value:Bad)->DirectBad {value}";
    let package = support::compile(source);
    let dump = package.to_string();
    assert!(dump.contains("(error-value"), "{dump}");
    let invalid = format!("{source} fn wrong(value:Good)->Bad {{value}}");
    assert!(support::compile_with_protocols(&invalid, &Default::default()).is_err());
}

#[test]
fn generic_success_injection_becomes_error_embedding_at_runtime() {
    use comptime_interpretor::Value;
    for (body, expected) in [("true", true), ("error Failure.Bad", false)] {
        let source = format!(
            "enum Failure {{Bad}} enum Other {{Bad}}
            fn sample()->!Failure bool {{{body}}}
            fn wrap(value:T)->!Other T where T {{value}}
            fn run()->bool {{wrap(sample())! {{Failure.Bad=>false,Other.Bad=>false}}}}"
        );
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(expected)
        );
    }
}

#[test]
fn associated_success_domain_specialization_preserves_error_branches() {
    use comptime_interpretor::Value;
    for (body, expected) in [("true", true), ("error Failure.Bad", false)] {
        let source = format!(
            "enum Failure {{Bad}} enum Other {{Bad}}
            trait Source {{assoc Item:.Type'builtin;}}
            impl Source for bool {{assoc Item:.Type'builtin=!Failure bool;}}
            fn sample()->!Failure bool {{{body}}}
            fn wrap(value:T.as(Source).Item)->!Other T.as(Source).Item where T:-Source {{value}}
            fn run()->bool {{wrap<bool>(sample())! {{Failure.Bad=>false,Other.Bad=>false}}}}"
        );
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(expected)
        );
    }
}

#[test]
fn generic_propagation_preserves_a_newly_qualified_success_domain() {
    use comptime_interpretor::Value;
    for (body, expected) in [("true", true), ("error Failure.Bad", false)] {
        let source = format!(
            "enum Failure {{Bad}} enum Other {{Bad}}
            fn sample()->!Failure bool {{{body}}}
            fn forward(value:!Other T)->!Other T where T {{let result=value!;result}}
            fn run()->bool {{forward<!Failure bool>(sample())! {{Failure.Bad=>false,Other.Bad=>false}}}}"
        );
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(expected)
        );
    }
}

#[test]
fn generic_propagation_keeps_definition_time_exit_boundaries() {
    use comptime_interpretor::{Interpreter, Value};
    for modifier in ["", "inline "] {
        for (body, expected, continues) in [
            ("true", true, true),
            ("error Failure.Bad", false, true),
            ("error Other.Bad", false, false),
        ] {
            let source = format!(
                "enum Failure {{Bad}} enum Other {{Bad}}
                fn sample()->![Other,Failure] bool {{{body}}}
                fn after(value:T)->T where T {{value}}
                {modifier}fn forward(value:!Other T)->!Other T where T {{let result=value!;after(result)}}
                fn run()->bool {{forward<!Failure bool>(sample())! {{Failure.Bad=>false,Other.Bad=>false}}}}"
            );
            let package = support::compile(&source);
            let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
            let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
            assert_eq!(
                vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
                Value::Bool(expected)
            );
            let calls = support::calls(&package, &vm);
            assert_eq!(calls.iter().filter(|name| *name == "sample").count(), 1);
            assert_eq!(
                calls.iter().filter(|name| *name == "after").count(),
                usize::from(continues)
            );
        }
    }
}

#[test]
fn generic_propagation_completes_before_required_comptime_invocation() {
    for (body, expected) in [("true", true), ("error Failure.Bad", false)] {
        let source = format!(
            "typealias Type=.Type'builtin;
            enum Failure {{Bad}} enum Other {{Bad}}
            pure comptime fn Key<value:bool>->Type {{bool}}
            pure comptime fn sample()->!Failure bool {{{body}}}
            pure comptime fn forward(value:!Other T)->!Other T where T {{let result=value!;result}}
            pure comptime fn compute()->bool {{forward<!Failure bool>(sample())! {{Failure.Bad=>false,Other.Bad=>false}}}}
            typealias Got=Key<compute()>;
            fn check(value:Got)->Key<{expected}> {{value}}"
        );
        support::compile(&source);
    }
}

#[test]
fn generic_handlers_preserve_new_errors_and_existing_residuals() {
    use comptime_interpretor::Value;
    for (body, expected) in [
        ("true", true),
        ("error Failure.Bad", false),
        ("error Other.Bad", true),
        ("error Third.Bad", false),
    ] {
        let source = format!(
            "enum Failure {{Bad}} enum Other {{Bad}} enum Third {{Bad}}
            fn sample()->![Other,Third,Failure] bool {{{body}}}
            fn fallback()->!Failure bool {{true}}
            fn forward(value:![Other,Third] T, fallback:T)->!Third T where T {{value! {{Other.Bad=>fallback}}}}
            fn run()->bool {{forward<!Failure bool>(sample(),fallback())! {{Failure.Bad=>false,Third.Bad=>false}}}}"
        );
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(expected)
        );
    }
}

#[test]
fn generic_guarded_handler_keeps_rejected_and_new_error_branches() {
    use comptime_interpretor::Value;
    for guard in [false, true] {
        for (body, expected) in [
            ("true", true),
            ("error Failure.Bad", false),
            ("error Other.Bad", guard),
        ] {
            let source = format!(
                "enum Failure {{Bad}} enum Other {{Bad}}
                fn sample()->![Other,Failure] bool {{{body}}}
                fn fallback()->!Failure bool {{true}}
                fn forward(value:!Other T, fallback:T)->!Other T where T {{value! {{(Other.Bad if {guard})=>fallback}}}}
                fn run()->bool {{forward<!Failure bool>(sample(),fallback())! {{Failure.Bad=>false,Other.Bad=>false}}}}"
            );
            assert_eq!(
                support::run_iir(&source, "run", vec![]),
                Value::Bool(expected)
            );
        }
    }
}

#[test]
fn generic_catch_phases_preserve_the_definition_time_error_boundary() {
    use comptime_interpretor::{Interpreter, Value};
    for modifier in ["", "inline "] {
        for (body, expected, catches) in [
            ("true", true, 0),
            ("error Inner.Bad", false, 0),
            ("error Outer.Bad", false, 1),
        ] {
            let source = format!(
                "enum Inner {{Bad}} enum Outer {{Bad}}
                fn sample()->![Outer,Inner] bool {{{body}}}
                fn before()->bool {{true}}
                fn ordinary()->bool {{true}}
                fn after(value:T)->T where T {{value}}
                {modifier}fn forward(value:!Outer T)->!Outer T where T {{value! {{
                    catch _=>before(),Outer.Bad=>ordinary(),catch original=>after(original),
                }}}}
                fn run()->bool {{forward<!Inner bool>(sample())! {{Inner.Bad=>false,Outer.Bad=>false}}}}"
            );
            let package = support::compile(&source);
            let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
            let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
            assert_eq!(
                vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
                Value::Bool(expected)
            );
            let calls = support::calls(&package, &vm);
            assert_eq!(calls.iter().filter(|name| *name == "sample").count(), 1);
            for phase in ["before", "ordinary", "after"] {
                assert_eq!(
                    calls.iter().filter(|name| *name == phase).count(),
                    catches,
                    "{body}: {calls:?}"
                );
            }
            let phases = calls
                .iter()
                .filter(|name| matches!(name.as_str(), "before" | "ordinary" | "after"))
                .map(String::as_str)
                .collect::<Vec<_>>();
            assert_eq!(
                phases,
                if catches == 1 {
                    vec!["before", "ordinary", "after"]
                } else {
                    vec![]
                }
            );
        }
    }
}

#[test]
fn generic_error_match_projects_success_after_explicit_error_arms() {
    use comptime_interpretor::Value;
    for (body, expected) in [
        ("true", true),
        ("error Inner.Bad", false),
        ("error Outer.Bad", true),
    ] {
        let source = format!(
            "enum Inner {{Bad}} enum Outer {{Bad}}
            fn sample()->![Outer,Inner] bool {{{body}}}
            fn forward(value:!Outer T)->!Outer T where T {{value match {{
                error Outer.Bad=>error Outer.Bad,
                result! => result,
                _ => error Outer.Bad,
            }}}}
            fn run()->bool {{forward<!Inner bool>(sample())! {{Inner.Bad=>false,Outer.Bad=>true}}}}"
        );
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(expected)
        );
    }
}

#[test]
fn generic_success_completion_does_not_bypass_an_earlier_wildcard() {
    use comptime_interpretor::Value;
    let source = "enum Inner {Bad} enum Outer {Bad}
        fn sample()->!Inner bool {error Inner.Bad}
        fn fallback()->!Inner bool {true}
        fn forward(value:!Outer T, fallback:T)->!Outer T where T {value match {
            _ => fallback,
            result! => result,
            error Outer.Bad => error Outer.Bad,
        }}
        fn run()->bool {forward<!Inner bool>(sample(),fallback())! {Inner.Bad=>false,Outer.Bad=>false}}";
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn generic_success_match_body_can_call_and_bind_the_abstract_payload() {
    use comptime_interpretor::{Interpreter, Value};
    for branch in [
        "after(result)",
        "{let retained=after(result);retained}",
        "{let callback=|| {result};after(callback())}",
        "{return after(result);}",
    ] {
        for (body, expected) in [
            ("true", true),
            ("error Inner.Bad", false),
            ("error Outer.Bad", false),
        ] {
            let source = format!(
            "enum Inner {{Bad}} enum Outer {{Bad}}
            fn sample()->![Outer,Inner] bool {{{body}}}
            fn after(value:T)->T where T {{value}}
            fn forward(value:!Outer T)->!Outer T where T {{value match {{
                result! => {branch},
                error Outer.Bad => error Outer.Bad,
            }}}}
            fn run()->bool {{forward<!Inner bool>(sample())! {{Inner.Bad=>false,Outer.Bad=>false}}}}"
        );
            let package = support::compile(&source);
            let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
            let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
            assert_eq!(
                vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
                Value::Bool(expected)
            );
            assert_eq!(
                support::calls(&package, &vm)
                    .iter()
                    .filter(|name| *name == "after")
                    .count(),
                usize::from(body != "error Outer.Bad")
            );
        }
    }
}

#[test]
fn generic_success_guard_observes_the_complete_payload_and_preserves_fallback() {
    use comptime_interpretor::{Interpreter, Value};
    for modifier in ["", "inline "] {
        for flag in [false, true] {
            for (body, expected) in [
                ("true", true),
                ("error Inner.Bad", false),
                ("error Outer.Bad", false),
            ] {
                let source = format!(
                "enum Inner {{Bad}} enum Outer {{Bad}}
                fn sample()->![Outer,Inner] bool {{{body}}}
                fn allowed(value:*T,flag:bool)->bool where T {{flag}}
                fn first(value:*T)->bool where T {{false}}
                fn unexpected(value:T)->T where T {{value}}
                fn accepted(value:T)->T where T {{value}}
                fn rejected(value:T)->T where T {{value}}
                {modifier}fn forward(value:!Outer T,flag:bool)->!Outer T where T {{value match {{
                    (result! if first(result.ref)) => unexpected(result),
                    (result! if allowed(result.ref,flag)) => accepted(result),
                    result! => rejected(result),
                    error Outer.Bad => error Outer.Bad,
                }}}}
                fn run()->bool {{forward<!Inner bool>(sample(),{flag})! {{Inner.Bad=>false,Outer.Bad=>false}}}}"
            );
                let package = support::compile(&source);
                let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
                let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
                assert_eq!(
                    vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
                    Value::Bool(expected)
                );
                let calls = support::calls(&package, &vm);
                let success = body != "error Outer.Bad";
                for (name, count) in [
                    ("sample", 1),
                    ("first", usize::from(success)),
                    ("unexpected", 0),
                    ("allowed", usize::from(success)),
                    ("accepted", usize::from(success && flag)),
                    ("rejected", usize::from(success && !flag)),
                ] {
                    assert_eq!(
                        calls.iter().filter(|actual| *actual == name).count(),
                        count,
                        "{calls:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn generic_success_guards_complete_during_required_comptime_invocation() {
    for pattern in [
        "(result! if flag)",
        "(result if flag)!",
        "(result and flag is true)!",
        "(result! if false) or (result! if flag)",
    ] {
        for flag in [false, true] {
            for (body, expected) in [("true", true), ("error Inner.Bad", false)] {
                let source = format!(
                "typealias Type=.Type'builtin;
                enum Inner {{Bad}} enum Outer {{Bad}}
                pure comptime fn Key<value:bool>->Type {{bool}}
                pure comptime fn sample()->!Inner bool {{{body}}}
                pure comptime fn forward(value:!Outer T,flag:bool)->!Outer T where T {{value match {{
                    {pattern} => result,
                    result! => result,
                    error Outer.Bad => error Outer.Bad,
                }}}}
                pure comptime fn compute()->bool {{forward<!Inner bool>(sample(),{flag})! {{Inner.Bad=>false,Outer.Bad=>false}}}}
                typealias Actual=Key<compute()>;
                fn check(value:Actual)->Key<{expected}> {{value}}"
            );
                support::compile(&source);
            }
        }
    }
}

#[test]
fn generic_success_guard_trait_method_receives_the_restored_error_reference() {
    use comptime_interpretor::Value;
    for (body, expected) in [
        ("true", true),
        ("false", true),
        ("error Inner.Bad", true),
        ("error Outer.Bad", false),
    ] {
        let source = format!(
            "enum Inner {{Bad}} enum Outer {{Bad}}
            typealias Wrapped=!Inner bool;
            trait Probe {{assoc Input:.Type'builtin;fn enabled(value:*Input,flag:bool)->bool;}}
            impl Probe for bool {{assoc Input:.Type'builtin=Wrapped;
                fn enabled(value:*Wrapped,flag:bool)->bool {{flag}}}}
            fn sample()->![Outer,Inner] bool {{{body}}}
            fn inspect(value:!Outer T.as(Probe).Input)->bool where T:-Probe {{value match {{
                (result! if T.as(Probe).enabled(result.ref,true)) => true,
                _! => false,
                error Outer.Bad => false,
            }}}}
            fn run()->bool {{inspect<bool>(sample())}}"
        );
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(expected)
        );
    }
}

#[test]
fn borrowing_an_error_result_does_not_implicitly_authorize_a_copy_read() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);}
        enum Inner {Bad} typealias Wrapped=!Inner bool;
        fn enabled(value:*Wrapped)->bool {value.*! {Inner.Bad=>false}}
        fn run()->bool {let value:Wrapped=true;enabled(value.ref)}";
    // Storage formation is valid; ownership admission must still demand Copy.
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    let failure = luna::execution::interpret_with_protocols(
        &package,
        "run",
        "Duplicate",
        "Retire",
        1_000_000,
    )
    .unwrap_err();
    assert!(failure.contains("Copy"), "{failure}");
}

#[test]
fn generic_success_pattern_combinators_restore_the_whole_payload() {
    use comptime_interpretor::{Interpreter, Value};
    for pattern in [
        "(result if allowed(result.ref,flag))!",
        "(result and allowed(result.ref,flag) is true)!",
        "(result! and allowed(result.ref,flag) is true)",
        "((result! if true) if allowed(result.ref,flag))",
        "((result if first(result.ref)) or (result if allowed(result.ref,flag)))!",
        "(result! if first(result.ref)) or (result! if allowed(result.ref,flag))",
    ] {
        for flag in [false, true] {
            for (body, expected) in [
                ("true", true),
                ("error Inner.Bad", false),
                ("error Outer.Bad", false),
            ] {
                let source = format!(
                    "enum Inner {{Bad}} enum Outer {{Bad}}
                    fn sample()->![Outer,Inner] bool {{{body}}}
                    fn first(value:*T)->bool where T {{false}}
                    fn allowed(value:*T,flag:bool)->bool where T {{flag}}
                    fn accepted(value:T)->T where T {{value}}
                    fn rejected(value:T)->T where T {{value}}
                    fn forward(value:!Outer T,flag:bool)->!Outer T where T {{value match {{
                        {pattern} => accepted(result),result! => rejected(result),error Outer.Bad => error Outer.Bad,
                    }}}}
                    fn run()->bool {{forward<!Inner bool>(sample(),{flag})! {{Inner.Bad=>false,Outer.Bad=>false}}}}"
                );
                let package = support::compile(&source);
                let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
                let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
                assert_eq!(
                    vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
                    Value::Bool(expected)
                );
                let calls = support::calls(&package, &vm);
                let success = body != "error Outer.Bad";
                for (name, count) in [
                    ("sample", 1),
                    ("first", usize::from(success && pattern.contains("first"))),
                    ("allowed", usize::from(success)),
                    ("accepted", usize::from(success && flag)),
                    ("rejected", usize::from(success && !flag)),
                ] {
                    assert_eq!(
                        calls.iter().filter(|actual| *actual == name).count(),
                        count,
                        "{pattern}, {body}, {flag}: {calls:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn generic_mixed_error_alternatives_keep_the_whole_alias_and_final_guard() {
    use comptime_interpretor::{Interpreter, Value};
    for pattern in [
        "(_! or _! or error Outer.Bad)",
        "((_ if flag)! or _! or error Outer.Bad)",
        "((_ and flag is true)! or _! or error Outer.Bad)",
    ] {
        for flag in [false, true] {
            for body in [
                "true",
                "error Inner.Bad",
                "error Outer.Bad",
                "error Other.Bad",
            ] {
                let source = format!(
                    "enum Inner {{Bad}} enum Outer {{Bad}} enum Other {{Bad}}
                fn sample()->![Outer,Other,Inner] bool {{{body}}}
                fn allowed(value:*T,flag:bool)->bool where T {{flag}}
                fn inspect(value:![Outer,Other] T,flag:bool)->bool where T {{value match {{
                    (({pattern} as whole) if allowed(whole.ref,flag)) => true,
                    _ => false,
                }}}}
                fn run()->bool {{inspect<!Inner bool>(sample(),{flag})}}"
                );
                let package = support::compile(&source);
                let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
                let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
                let matches = body != "error Other.Bad";
                assert_eq!(
                    vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
                    Value::Bool(flag && matches)
                );
                let calls = support::calls(&package, &vm);
                assert_eq!(
                    calls.iter().filter(|name| *name == "allowed").count(),
                    usize::from(matches),
                    "{body}, {flag}: {calls:?}"
                );
                assert_eq!(calls.iter().filter(|name| *name == "sample").count(), 1);
            }
        }
    }
}

#[test]
fn generic_mixed_error_patterns_complete_for_required_comptime_invocation() {
    for flag in [false, true] {
        for body in ["true", "error Inner.Bad", "error Outer.Bad"] {
            let source = format!(
                "typealias Type=.Type'builtin;
                enum Inner {{Bad}} enum Outer {{Bad}}
                pure comptime fn Key<value:bool>->Type {{bool}}
                pure comptime fn sample()->![Outer,Inner] bool {{{body}}}
                pure comptime fn inspect(value:!Outer T,flag:bool)->bool where T {{value match {{
                    (((_! or error Outer.Bad) as whole) if flag) => true,
                    _ => false,
                }}}}
                pure comptime fn compute()->bool {{inspect<!Inner bool>(sample(),{flag})}}
                typealias Actual=Key<compute()>;
                fn check(value:Actual)->Key<{flag}> {{value}}"
            );
            support::compile(&source);
        }
    }
}

#[test]
fn generic_error_wildcards_in_tuple_and_let_patterns_keep_their_subject_domain() {
    use comptime_interpretor::Value;
    for (body, expected) in [
        ("true", true),
        ("error Inner.Bad", true),
        ("error Outer.Bad", false),
    ] {
        for (signature, inspect, argument) in [
            (
                "value:Tuple<!Outer T,bool>",
                "value match {(_!,flag)=>flag,_=>false}",
                "(sample(),true)",
            ),
            (
                "value:!Outer T",
                "let _! = value else {return false;};true",
                "sample()",
            ),
        ] {
            let source = format!(
                "enum Inner {{Bad}} enum Outer {{Bad}}
                fn sample()->![Outer,Inner] bool {{{body}}}
                fn inspect({signature})->bool where T {{{inspect}}}
                fn run()->bool {{inspect<!Inner bool>({argument})}}"
            );
            assert_eq!(
                support::run_iir_with_tuple(&source, "run", vec![]),
                Value::Bool(expected),
                "{signature}: {body}"
            );
        }
    }
}

#[test]
fn generic_error_record_patterns_use_each_named_fields_domain() {
    use comptime_interpretor::Value;
    for (body, expected) in [
        ("true", true),
        ("error Inner.Bad", true),
        ("error Outer.Bad", false),
    ] {
        let source = format!(
            "fn record(...fields:T)->T where T {{fields}}
            enum Inner {{Bad}} enum Outer {{Bad}}
            fn sample()->![Outer,Inner] bool {{{body}}}
            fn expected()->!Outer T where T {{error Outer.Bad}}
            fn inspect(value:(record {{result:expected<T>(),flag:true}})'type)->bool where T {{
                value match {{{{flag:flag,result:_!}}=>flag,_=>false}}
            }}
            fn run()->bool {{inspect<!Inner bool>(record {{result:sample(),flag:true}})}}"
        );
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(expected)
        );
    }
}

#[test]
fn generic_propagation_does_not_duplicate_a_shared_error_identity() {
    use comptime_interpretor::{Interpreter, Value};
    let source = "enum Failure {Bad}
        fn after(value:T)->T where T {value}
        fn sample()->!Failure bool {error Failure.Bad}
        fn forward(value:!Failure T)->!Failure T where T {let result=value!;after(result)}
        fn run()->bool {forward<!Failure bool>(sample())! {Failure.Bad=>true}}";
    let package = support::compile(source);
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert!(
        !support::calls(&package, &vm)
            .iter()
            .any(|name| name == "after")
    );
}

#[test]
fn generic_nested_success_bindings_restore_the_complete_payload() {
    use comptime_interpretor::Value;
    for (body, expected) in [
        ("true", true),
        ("error Inner.Bad", true),
        ("error Outer.Bad", false),
    ] {
        for (signature, pattern, argument) in [
            ("Tuple<!Outer T,bool>", "(result!,flag)", "(sample(),true)"),
            (
                "Tuple<Tuple<!Outer T,bool>,bool>",
                "((result!,_),flag)",
                "((sample(),true),true)",
            ),
            (
                "(record {result:expected<T>(),flag:true})'type",
                "{flag:flag,result:result!}",
                "record {result:sample(),flag:true}",
            ),
        ] {
            let source = format!(
                "enum Inner {{Bad}} enum Outer {{Bad}}
                fn record(...fields:T)->T where T {{fields}}
                fn expected()->!Outer T where T {{error Outer.Bad}}
                fn sample()->![Outer,Inner] bool {{{body}}}
                fn after(value:T)->bool where T {{true}}
                fn inspect(value:{signature})->bool where T {{
                    value match {{{pattern}=>after(result) and flag,_=>false}}
                }}
                fn run()->bool {{inspect<!Inner bool>({argument})}}"
            );
            assert_eq!(
                support::run_iir_with_tuple(&source, "run", vec![]),
                Value::Bool(expected),
                "{signature}: {body}"
            );
        }
    }
}

#[test]
fn generic_let_success_bindings_restore_the_complete_payload() {
    use comptime_interpretor::Value;
    for (body, expected) in [
        ("true", true),
        ("error Inner.Bad", true),
        ("error Outer.Bad", false),
    ] {
        for (signature, pattern, argument) in [
            ("!Outer T", "result!", "sample()"),
            ("Tuple<!Outer T,bool>", "(result!,_)", "(sample(),true)"),
            (
                "(record {result:expected<T>(),flag:true})'type",
                "{flag:_,result:result!}",
                "record {result:sample(),flag:true}",
            ),
        ] {
            let source = format!(
                "enum Inner {{Bad}} enum Outer {{Bad}}
                fn record(...fields:T)->T where T {{fields}}
                fn expected()->!Outer T where T {{error Outer.Bad}}
                fn sample()->![Outer,Inner] bool {{{body}}}
                fn after(value:T)->bool where T {{true}}
                fn inspect(value:{signature})->bool where T {{
                    let {pattern} = value else {{return false;}};
                    after(result)
                }}
                fn run()->bool {{inspect<!Inner bool>({argument})}}"
            );
            assert_eq!(
                support::run_iir_with_tuple(&source, "run", vec![]),
                Value::Bool(expected),
                "{signature}: {body}"
            );
        }
    }
}

#[test]
fn generic_let_success_bindings_complete_during_required_comptime_invocation() {
    for (body, expected) in [
        ("true", true),
        ("error Inner.Bad", true),
        ("error Outer.Bad", false),
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
            enum Inner {{Bad}} enum Outer {{Bad}}
            pure comptime fn Key<value:bool>->Type {{bool}}
            pure comptime fn sample()->![Outer,Inner] bool {{{body}}}
            pure comptime fn after(value:T)->bool where T {{true}}
            pure comptime fn inspect(value:!Outer T)->bool where T {{
                let result! = value else {{return false;}};
                after(result)
            }}
            pure comptime fn compute()->bool {{inspect<!Inner bool>(sample())}}
            typealias Actual=Key<compute()>;
            fn check(value:Actual)->Key<{expected}> {{value}}"
        );
        support::compile(&source);
    }
}

#[test]
fn generic_guarded_let_restores_the_payload_before_observation() {
    use comptime_interpretor::Value;
    for flag in [false, true] {
        for body in ["true", "error Inner.Bad", "error Outer.Bad"] {
            for pattern in [
                "(result! if allowed(result.ref,flag))",
                "(result if allowed(result.ref,flag))!",
            ] {
                let source = format!(
                    "enum Inner {{Bad}} enum Outer {{Bad}}
                    fn sample()->![Outer,Inner] bool {{{body}}}
                    fn allowed(value:*T,flag:bool)->bool where T {{flag}}
                    fn after(value:T)->bool where T {{true}}
                    fn inspect(value:!Outer T,flag:bool)->bool where T {{
                        let {pattern} = value else {{return false;}};
                        after(result)
                    }}
                    fn run()->bool {{inspect<!Inner bool>(sample(),{flag})}}"
                );
                assert_eq!(
                    support::run_iir(&source, "run", vec![]),
                    Value::Bool(flag && !body.contains("Outer")),
                    "{pattern}: {body}, {flag}"
                );
            }
        }
    }
}

#[test]
fn generic_guarded_let_completes_during_required_comptime_invocation() {
    for flag in [false, true] {
        for body in ["true", "error Inner.Bad", "error Outer.Bad"] {
            let expected = flag && !body.contains("Outer");
            for pattern in [
                "(result! if flag)",
                "((result and flag is extra) if extra)!",
            ] {
                let source = format!(
                    "typealias Type=.Type'builtin;
                enum Inner {{Bad}} enum Outer {{Bad}}
                pure comptime fn Key<value:bool>->Type {{bool}}
                pure comptime fn sample()->![Outer,Inner] bool {{{body}}}
                pure comptime fn after(value:T)->bool where T {{true}}
                pure comptime fn inspect(value:!Outer T,flag:bool)->bool where T {{
                    let {pattern} = value else {{return false;}};
                    after(result)
                }}
                pure comptime fn compute()->bool {{inspect<!Inner bool>(sample(),{flag})}}
                typealias Actual=Key<compute()>;
                fn check(value:Actual)->Key<{expected}> {{value}}"
                );
                support::compile(&source);
            }
        }
    }
}

#[test]
fn generic_computed_let_exports_payload_and_computed_bindings() {
    use comptime_interpretor::Value;
    for flag in [false, true] {
        for body in ["true", "error Inner.Bad", "error Outer.Bad"] {
            for pattern in [
                "(result and measure(result.ref,flag) is extra)!",
                "((result and measure(result.ref,flag) is extra) if extra)!",
            ] {
                let source = format!(
                    "enum Inner {{Bad}} enum Outer {{Bad}}
                    fn sample()->![Outer,Inner] bool {{{body}}}
                    fn measure(value:*T,flag:bool)->bool where T {{flag}}
                    fn after(value:T)->bool where T {{true}}
                    fn inspect(value:!Outer T,flag:bool)->bool where T {{
                        let {pattern} = value else {{return false;}};
                        after(result) and extra
                    }}
                    fn run()->bool {{inspect<!Inner bool>(sample(),{flag})}}"
                );
                assert_eq!(
                    support::run_iir(&source, "run", vec![]),
                    Value::Bool(flag && !body.contains("Outer")),
                    "{pattern}: {body}, {flag}"
                );
            }
        }
    }
}

#[test]
fn computed_scalar_export_packets_are_available_to_required_invocation() {
    for flag in [false, true] {
        let source = format!(
            "typealias Type=.Type'builtin;
            struct Reading {{flag:bool}}
            pure comptime fn Key<value:bool>->Type {{bool}}
            pure comptime fn make()->Reading {{Reading {{flag:{flag}}}}}
            pure comptime fn compute()->bool {{
                let (_ and make() is {{flag:found}}) = true;
                found
            }}
            typealias Actual=Key<compute()>;
            fn check(value:Actual)->Key<{flag}> {{value}}"
        );
        support::compile(&source);
    }
}

#[test]
fn generic_computed_export_packets_are_instantiated_during_required_invocation() {
    for flag in [false, true] {
        let source = format!(
            "typealias Type=.Type'builtin;
            struct Reading where T {{flag:T}}
            pure comptime fn Key<value:bool>->Type {{bool}}
            pure comptime fn make(value:T)->Reading<T> where T {{Reading<T> {{flag:value}}}}
            pure comptime fn inspect(value:T)->bool where T, requires T == bool {{
                let (_ and make(value) is {{flag:found}})=true;
                found
            }}
            pure comptime fn compute()->bool {{inspect({flag})}}
            typealias Actual=Key<compute()>;
            fn check(value:Actual)->Key<{flag}> {{value}}"
        );
        support::compile(&source);
    }
}

#[test]
fn error_result_subjects_dispatch_explicit_and_generic_user_implementations() {
    use comptime_interpretor::Value;
    for subject in ["Outcome", "!Failure T where T, requires T == bool"] {
        for (body, expected) in [
            ("true", true),
            ("false", false),
            ("error Failure.Bad", false),
        ] {
            let source = format!(
                "enum Failure {{Bad}} typealias Outcome=!Failure bool;
                trait Read {{fn read(self)->bool;}}
                impl Read for {subject} {{fn read(self)->bool {{self! {{Failure.Bad=>false}}}}}}
                fn make()->Outcome {{{body}}}
                fn run()->bool {{make().as(Read).read()}}"
            );
            assert_eq!(
                support::run_iir(&source, "run", vec![]),
                Value::Bool(expected),
                "{subject}: {body}"
            );
        }
    }
}

#[test]
fn generic_frozen_error_inputs_keep_labels_when_substitution_reorders_rows() {
    for (types, value) in [
        ("char,bool", "'x'"),
        ("bool,char", "false"),
        ("bool,bool", "false"),
    ] {
        for failed in [false, true] {
            let expected = if failed {
                format!("error {value}")
            } else {
                "true".into()
            };
            let source = format!(
                "typealias Type=.Type'builtin; typealias char=.char'builtin;
                pure comptime fn Key<value:![bool,char] bool>->Type {{bool}}
                pure comptime fn choose(value:T,failed:bool)->![T,U] bool where T,U {{
                    if failed {{error value}} else {{true}}
                }}
                pure comptime fn expected()->![bool,char] bool {{{expected}}}
                typealias Actual=Key<choose<{types}>({value},{failed})>;
                fn check(value:Actual)->Key<expected()> {{value}}"
            );
            support::compile(&source);
            let wrong = if !failed {
                "false"
            } else if types == "char,bool" {
                "error true"
            } else {
                "error 'y'"
            };
            let mismatched = source.replace(
                &format!("fn expected()->![bool,char] bool {{{expected}}}"),
                &format!("fn expected()->![bool,char] bool {{{wrong}}}"),
            );
            assert_ne!(source, mismatched);
            assert!(
                support::compile_with_protocols(&mismatched, &Default::default()).is_err(),
                "frozen branch and payload identity must survive substitution: {types}, {failed}"
            );
        }
    }
}

#[test]
fn associated_error_values_materialize_completed_success_and_failure_payloads() {
    for (value, expected) in [("true", true), ("error false", false)] {
        for (declaration, subject) in [
            (
                format!("impl Source for bool {{assoc result:!bool bool={value};}}"),
                "bool",
            ),
            (
                format!(
                    "struct Holder where T {{}} impl Source for Holder<T> where T {{assoc result:!bool bool={value};}}"
                ),
                "Holder<bool>",
            ),
        ] {
            let source = format!(
                "trait Source {{assoc result:!bool bool;}}{declaration}fn run()->bool {{{subject}.as(Source).result match {{value! => value,error failure=>failure}}}}"
            );
            assert_eq!(
                support::run_iir(&source, "run", vec![]),
                comptime_interpretor::Value::Bool(expected)
            );
        }
    }
}

#[test]
fn error_result_storage_cannot_hide_comptime_only_payloads() {
    for source in [
        "typealias MetaType=.Type'builtin; enum Fault {bad} fn invalid(value:!Fault MetaType) {}",
        "enum Fault {bad(.Type'builtin)} fn invalid(value:!Fault bool) {}",
        "typealias Integer=.Integer'builtin; enum Fault {bad} fn invalid(value:*!Fault Integer) {}",
    ] {
        let error = compile(source, Emit::Nir)
            .err()
            .expect("comptime payload must be rejected");
        assert!(error.contains("comptime-only"), "{source}: {error}");
        compile(
            &source.replace("fn invalid", "comptime fn valid"),
            Emit::Nir,
        )
        .unwrap();
    }
    compile("enum Fault {bad} fn valid(value:!Fault bool) {}", Emit::Nir).unwrap();
}

#[test]
fn void_success_injection_covers_implicit_and_explicit_delivery() {
    for body in [
        "",
        "do {}",
        "return;",
        "if flag {return;}",
        "if flag {do {}} else {do {}}",
        "if flag {error true} else {}",
    ] {
        let source = format!(
            "typealias Void=.void'builtin;
            fn attempt(flag:bool)->!bool Void {{{body}}}
            fn run()->bool {{attempt(false) match {{_! =>true,error _=>false,}}}}"
        );
        let Artifact::Nir(package) = compile(&source, Emit::Nir).unwrap() else {
            panic!()
        };
        assert_eq!(
            luna::execution::interpret(&package, "run", 100000)
                .unwrap()
                .to_string(),
            "(bool true)"
        );
    }
    for target in ["bool", ".NoReturn'builtin"] {
        for body in ["", "do {}", "return;"] {
            let source = format!("fn run()->!bool {target} {{{body}}}");
            assert!(compile(&source, Emit::Nir).is_err(), "{source}");
        }
    }
}
