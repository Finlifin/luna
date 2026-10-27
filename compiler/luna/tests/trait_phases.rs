//! Source membership phases remain independent of function execution stages.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use luna::cli::{Emit, Options};
use luna::frontend::{self, Artifact};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn compile(source: &str, emit: Emit) -> Result<Artifact, String> {
    let path = std::env::temp_dir().join(format!(
        "luna-impl-phase-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let result = frontend::compile(&Options {
        input: path.clone(),
        emit,
        output: None,
        sysroot: None,
        splice_protocol: None,
        meta_list_type: None,
        tuple_type: None,
        index_protocol: None,
        ord_protocol: None,
        eq_protocol: None,
        list_pattern_protocol: None,
        literal_into_protocol: None,
        arithmetic_protocols: Default::default(),
    });
    std::fs::remove_file(path).unwrap();
    result
}

#[test]
fn complementary_implementation_phases_survive_hir_lowering() {
    let Artifact::Hir(text)=compile("trait Mark {} struct Subject {} impl Mark for Subject {} comptime impl Mark for Subject {}",Emit::Hir).unwrap() else {panic!()};
    assert_eq!(
        text.matches("(impl-definition (phase runtime)").count(),
        1,
        "{text}"
    );
    assert_eq!(
        text.matches("(impl-definition (phase comptime)").count(),
        1,
        "{text}"
    );
    assert!(!text.contains("__flurry_kw_comptime"), "{text}");
}

#[test]
fn inherent_implementation_phase_is_retained_independently_of_members() {
    let Artifact::Hir(text) = compile(
        "struct Subject {} comptime impl Subject {fn value()->bool {true}}",
        Emit::Hir,
    )
    .unwrap() else {
        panic!()
    };
    assert!(text.contains("(impl-definition (phase comptime)"), "{text}");
    assert!(text.contains("(comptime false)"), "{text}");
}

#[test]
fn staged_implementations_cannot_silently_enter_runtime_selection() {
    for source in ["struct Subject {} comptime impl Subject {fn value()->bool {true}}"] {
        let error = compile(source, Emit::Nir)
            .err()
            .expect("phase selection remains unfinished");
        assert!(
            error.contains("comptime implementation phase selection is not yet implemented"),
            "{error}"
        );
    }
}

#[test]
fn ordinary_implementations_continue_to_elaborate() {
    assert!(
        compile(
            "trait Mark {} struct Subject {} impl Mark for Subject {} fn run()->bool {true}",
            Emit::Nir
        )
        .is_ok()
    );
}

fn phase_source(body: &str) -> String {
    format!(
        "mod attrs {{{}}} use attrs.comptime_phase as staged; {body}",
        include_str!("../../../library/attrs/lib.fl")
    )
}

#[test]
fn source_comptime_contribution_executes_and_retains_phase_metadata() {
    let source = phase_source(
        "
        trait Code { ^staged comptime fn value()->bool; }
        struct Subject {}
        comptime impl Code for Subject { comptime fn value()->bool {true} }
        comptime fn run()->bool {Subject.as(Code).value()}
    ",
    );
    let package = support::compile(&source);
    assert_eq!(
        package.traits[0].methods[0].phase,
        nir::program::TraitPhase::Comptime
    );
    assert!(package.implementations[0].phases.comptime);
    assert!(!package.implementations[0].phases.runtime);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn phase_attributes_use_ordinary_comptime_evaluation_and_override_order() {
    let source = "
        comptime fn metadata() -> .Object'builtin {let value={comptime_phase: false}; value}
        const staged={comptime_phase:true};
        trait Code { ^metadata() ^staged comptime fn value()->bool; }
        struct Subject {}
        impl Code for Subject {comptime fn value()->bool {true}}
        comptime fn run()->bool {Subject.as(Code).value()}
    ";
    let package = support::compile(source);
    assert_eq!(
        package.traits[0].methods[0].phase,
        nir::program::TraitPhase::Runtime
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn runtime_contribution_does_not_need_comptime_members() {
    let source = phase_source(
        "
        trait Code { fn runtime_value()->bool; ^staged comptime fn static_value()->bool; }
        struct Subject {}
        impl Code for Subject {fn runtime_value()->bool {true}}
        fn run()->bool {Subject.as(Code).runtime_value()}
    ",
    );
    support::compile(&source);
    let missing = source.replace(
        "fn run()->bool {Subject.as(Code).runtime_value()}",
        "comptime fn run()->bool {Subject.as(Code).static_value()}",
    );
    let diagnostic = support::compile_with_protocols(&missing, &Default::default()).unwrap_err();
    assert!(
        diagnostic.contains("lacks comptime trait phase"),
        "{diagnostic}"
    );
}

#[test]
fn source_phase_metadata_rejects_wrong_targets_domains_and_unknown_keys() {
    for (source, expected) in [
        (
            "const staged={comptime_phase:true}; ^staged fn run()->bool {true}",
            "requires a trait method",
        ),
        (
            "const staged={comptime_phase:9}; trait Code {^staged fn value()->bool;}",
            "requires a completed Bool",
        ),
        (
            "const staged={other:true}; trait Code {^staged fn value()->bool;}",
            "not implemented for definition attribute",
        ),
    ] {
        let diagnostic = support::compile_with_protocols(source, &Default::default()).unwrap_err();
        assert!(diagnostic.contains(expected), "{diagnostic}");
    }
}

#[test]
fn wrong_phase_contribution_cannot_supply_or_omit_required_members() {
    for (implementation, expected) in [
        (
            "impl Code for Subject {comptime fn value()->bool {true}}",
            "belongs to the comptime trait phase",
        ),
        (
            "comptime impl Code for Subject {}",
            "missing required trait method",
        ),
    ] {
        let source = phase_source(&format!(
            "trait Code {{^staged comptime fn value()->bool;}} struct Subject {{}} {implementation}"
        ));
        let diagnostic = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(diagnostic.contains(expected), "{diagnostic}");
    }
}

#[test]
fn generic_comptime_contribution_specializes_its_owner_inputs() {
    let source = phase_source(
        "\n        trait Code { ^staged comptime fn value()->bool; }\n        struct Subject where T {}\n        comptime impl Code for Subject<T> where T {comptime fn value()->bool {true}}\n        comptime fn run()->bool {Subject<bool>.as(Code).value()}\n    ",
    );
    let package = support::compile(&source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn available_comptime_phase_can_use_a_trait_default() {
    let source = phase_source(
        "\n        trait Code { ^staged comptime fn value()->bool {true} }\n        struct Subject {}\n        comptime impl Code for Subject {}\n        comptime fn run()->bool {Subject.as(Code).value()}\n    ",
    );
    let package = support::compile(&source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn metadata_inspection_accepts_the_same_comptime_contribution_as_execution() {
    let source = phase_source(
        "\n        trait Code { ^staged comptime fn value()->bool; }\n        struct Subject {}\n        comptime impl Code for Subject {comptime fn value()->bool {true}}\n    ",
    );
    compile(&source, Emit::Attributes).unwrap();
}

#[test]
fn explicitly_opposite_member_phase_is_rejected() {
    let source = phase_source(
        "\n        const ordinary={comptime_phase:false};\n        trait Code { ^staged comptime fn value()->bool; }\n        struct Subject {}\n        comptime impl Code for Subject { ^ordinary comptime fn value()->bool {true} }\n    ",
    );
    let diagnostic = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
    assert!(
        diagnostic.contains("member phase does not match"),
        "{diagnostic}"
    );
}

#[test]
fn source_phase_selection_never_falls_back_from_a_more_precise_relationship() {
    let source = phase_source(
        "\n        trait Code { ^staged comptime fn value()->bool; }\n        struct Subject where T {}\n        comptime impl Code for Subject<T> where T {comptime fn value()->bool {true}}\n        impl Code for Subject<bool> {}\n        comptime fn run()->bool {Subject<bool>.as(Code).value()}\n    ",
    );
    let diagnostic = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
    assert!(
        diagnostic.contains("lacks comptime trait phase"),
        "{diagnostic}"
    );
    let other = source.replace("Subject<bool>.as(Code)", "Subject<.char'builtin>.as(Code)");
    let package = support::compile(&other);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn complementary_source_contributions_merge_in_either_order() {
    for reverse in [false, true] {
        let runtime = "impl Code for Subject {comptime fn ordinary()->bool {true}}";
        let comptime = "comptime impl Code for Subject {comptime fn staged_value()->bool {false}}";
        let implementations = if reverse {
            format!("{comptime} {runtime}")
        } else {
            format!("{runtime} {comptime}")
        };
        let source = phase_source(&format!(
            "\n            trait Code {{comptime fn ordinary()->bool; ^staged comptime fn staged_value()->bool;}}\n            struct Subject {{}}\n            {implementations}\n            comptime fn run()->bool {{Subject.as(Code).ordinary() and not Subject.as(Code).staged_value()}}\n        "
        ));
        let package = support::compile(&source);
        assert_eq!(package.implementations.len(), 1);
        let implementation = &package.implementations[0];
        assert!(implementation.phases.runtime && implementation.phases.comptime);
        assert_eq!(
            implementation
                .methods
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            vec!["ordinary", "staged_value"]
        );
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn complementary_generic_contributions_keep_each_method_input_telescope() {
    for reverse in [false, true] {
        let runtime =
            "impl Code<T> for Subject<T> where T {comptime fn ordinary(value:T)->T {value}}";
        let comptime = "comptime impl Code<U> for Subject<U> where U {comptime fn staged_value(value:U)->U {value}}";
        let implementations = if reverse {
            format!("{comptime} {runtime}")
        } else {
            format!("{runtime} {comptime}")
        };
        let source = phase_source(&format!(
            "\n            trait Code where E {{comptime fn ordinary(value:E)->E; ^staged comptime fn staged_value(value:E)->E;}}\n            struct Subject where T {{}}\n            {implementations}\n            comptime fn run()->bool {{Subject<bool>.as(Code<bool>).ordinary(true) and not Subject<bool>.as(Code<bool>).staged_value(false)}}\n        "
        ));
        let package = support::compile(&source);
        assert_eq!(package.implementations.len(), 1);
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn complementary_contributions_reject_different_capability_conditions() {
    let source = phase_source(
        "\n        trait Bound {}\n        trait Code {comptime fn ordinary()->bool; ^staged comptime fn staged_value()->bool;}\n        struct Subject where T {}\n        impl Code for Subject<T> where T:-Bound {comptime fn ordinary()->bool {true}}\n        comptime impl Code for Subject<U> where U {comptime fn staged_value()->bool {true}}\n    ",
    );
    assert!(support::compile_with_protocols(&source, &Default::default()).is_err());
}

#[test]
fn complementary_value_input_contributions_transport_the_selected_value() {
    let source = phase_source(
        "\n        trait Code {comptime fn ordinary()->bool; ^staged comptime fn staged_value()->bool;}\n        struct Subject where flag:bool {}\n        impl Code for Subject<flag> where flag:bool {comptime fn ordinary()->bool {flag}}\n        comptime impl Code for Subject<other> where other:bool {comptime fn staged_value()->bool {not other}}\n        comptime fn run()->bool {Subject<true>.as(Code).ordinary() and not Subject<true>.as(Code).staged_value()}\n    ",
    );
    let package = support::compile(&source);
    assert_eq!(package.implementations.len(), 1);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn complementary_capability_conditions_accept_alpha_renaming() {
    let source = phase_source(
        r#"
        trait Bound {}
        struct Element {}
        impl Bound for Element {}
        trait Code {
            comptime fn ordinary()->bool;
            ^staged comptime fn staged_value()->bool;
        }
        struct Subject where T {}
        impl Code for Subject<T> where T:-Bound {
            comptime fn ordinary()->bool {true}
        }
        comptime impl Code for Subject<U> where U:-Bound {
            comptime fn staged_value()->bool {false}
        }
        comptime fn run()->bool {
            Subject<Element>.as(Code).ordinary()
                and not Subject<Element>.as(Code).staged_value()
        }
    "#,
    );
    let package = support::compile(&source);
    assert_eq!(package.implementations.len(), 2);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}
