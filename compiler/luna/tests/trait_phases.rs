//! Source membership phases remain independent of function execution stages.
#[path = "support/primitive_bindings.rs"]
mod primitive_bindings;
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
        "mod attrs {{{}}} use attrs.comptime_phase as staged; typealias Type=.Type'builtin; {body}",
        include!("support/attrs_source.rs")
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

#[test]
fn comptime_associated_values_and_shared_signature_types_execute() {
    let source = phase_source(
        r#"
        trait Code {
            assoc Value: Type = bool;
            ^staged assoc enabled: bool;
            ^staged comptime fn value()->Value;
        }
        struct Subject {}
        comptime impl Code for Subject {
            assoc enabled: bool = true;
            comptime fn value()->bool {Subject.as(Code).enabled}
        }
        comptime fn run()->bool {Subject.as(Code).value()}
    "#,
    );
    let package = support::compile(&source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn runtime_contribution_omits_comptime_associated_members_and_defaults() {
    let source = phase_source(
        r#"
        trait Code { ^staged assoc enabled: bool = true; }
        struct Subject {}
        impl Code for Subject {}
        comptime fn run()->bool {true}
    "#,
    );
    let package = support::compile(&source);
    assert!(package.implementations[0].associated.is_empty());
    let missing = source.replace(
        "comptime fn run()->bool {true}",
        "comptime fn run()->bool {Subject.as(Code).enabled}",
    );
    let diagnostic = support::compile_with_protocols(&missing, &Default::default()).unwrap_err();
    assert!(
        diagnostic.contains("lacks comptime trait phase"),
        "{diagnostic}"
    );
}

#[test]
fn associated_phase_checks_required_values_and_explicit_annotations() {
    for (implementation, expected) in [
        (
            "impl Code for Subject {assoc enabled: bool = true;}",
            "belongs to the comptime trait phase",
        ),
        (
            "comptime impl Code for Subject {}",
            "missing required associated value",
        ),
        (
            "comptime impl Code for Subject {^ordinary assoc enabled: bool = true;}",
            "associated phase does not match",
        ),
    ] {
        let source = phase_source(&format!(
            r#"
            const ordinary = {{comptime_phase:false}};
            trait Code {{ ^staged assoc enabled: bool; }}
            struct Subject {{}}
            {implementation}
        "#
        ));
        let diagnostic = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(diagnostic.contains(expected), "{diagnostic}");
    }
}

#[test]
fn generic_comptime_associated_type_specializes_owner_inputs() {
    let source = phase_source(
        r#"
        trait Code { ^staged assoc Value: Type; }
        struct Subject where T {}
        comptime impl Code for Subject<T> where T { assoc Value: Type = T; }
        comptime fn run()->bool {
            let value: Subject<bool>.as(Code).Value = true;
            value
        }
    "#,
    );
    let package = support::compile(&source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn associated_phase_selection_does_not_recover_a_broader_default() {
    let source = phase_source(
        r#"
        trait Code { ^staged assoc enabled: bool = true; }
        struct Subject where T {}
        comptime impl Code for Subject<T> where T {}
        impl Code for Subject<bool> {}
        comptime fn run()->bool {Subject<bool>.as(Code).enabled}
    "#,
    );
    let diagnostic = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
    assert!(
        diagnostic.contains("lacks comptime trait phase"),
        "{diagnostic}"
    );
}

#[test]
fn complementary_associated_values_must_agree() {
    let source = phase_source(
        r#"
        trait Code { assoc Value: Type = bool; }
        struct Subject {}
        impl Code for Subject {}
        comptime impl Code for Subject { assoc Value: Type = .char'builtin; }
    "#,
    );
    let diagnostic = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
    assert!(
        diagnostic.contains("complementary associated values differ"),
        "{diagnostic}"
    );
}

#[test]
fn complementary_associated_types_share_required_data_in_either_order() {
    for reverse in [false, true] {
        let runtime = "impl Code<bool> for Subject<bool> {assoc Value: Type = bool; comptime fn ordinary(value:Value)->Value {value}}";
        let staged = "comptime impl Code<bool> for Subject<bool> {assoc enabled:bool=true; comptime fn staged_value(value:bool)->bool {value}}";
        let definitions = if reverse {
            format!("{staged} {runtime}")
        } else {
            format!("{runtime} {staged}")
        };
        let source = phase_source(&format!(
            r#"
            trait Code where T {{
                assoc Value: Type;
                ^staged assoc enabled: bool;
                comptime fn ordinary(value:Value)->Value;
                ^staged comptime fn staged_value(value:Value)->Value;
            }}
            struct Subject where T {{}}
            {definitions}
            comptime fn run()->bool {{
                Subject<bool>.as(Code<bool>).ordinary(true)
                    and Subject<bool>.as(Code<bool>).staged_value(true)
                    and Subject<bool>.as(Code<bool>).enabled
            }}
        "#
        ));
        let package = support::compile(&source);
        assert_eq!(package.implementations.len(), 1);
        assert_eq!(package.implementations[0].associated.len(), 2);
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn complementary_generic_associated_types_keep_lexical_bindings() {
    for reverse in [false, true] {
        let runtime = "impl Code<T> for Subject<T> where T {assoc Value: Type=T; comptime fn ordinary(value:Value)->Value {value}}";
        let staged = "comptime impl Code<U> for Subject<U> where U {assoc Value: Type=U; comptime fn staged_value(value:Value)->Value {value}}";
        let definitions = if reverse {
            format!("{staged} {runtime}")
        } else {
            format!("{runtime} {staged}")
        };
        let source = phase_source(&format!(
            r#"
            trait Code where E {{
                assoc Value: Type;
                comptime fn ordinary(value:Value)->Value;
                ^staged comptime fn staged_value(value:Value)->Value;
            }}
            struct Subject where T {{}}
            {definitions}
            comptime fn run()->bool {{
                Subject<bool>.as(Code<bool>).ordinary(true)
                    and not Subject<bool>.as(Code<bool>).staged_value(false)
            }}
        "#
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
fn complementary_scalar_values_compare_results_and_reject_disagreement() {
    let source = phase_source(
        r#"
        trait Code { assoc enabled: bool; }
        struct Subject {}
        impl Code for Subject {assoc enabled: bool = true;}
        comptime impl Code for Subject {assoc enabled: bool = not false;}
        comptime fn run()->bool {Subject.as(Code).enabled}
    "#,
    );
    let package = support::compile(&source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let mismatch = source.replace("not false", "false");
    let diagnostic = support::compile_with_protocols(&mismatch, &Default::default()).unwrap_err();
    assert!(
        diagnostic.contains("complementary associated values differ"),
        "{diagnostic}"
    );
}

#[test]
fn complementary_generic_scalar_initializers_preserve_value_inputs() {
    let source = phase_source(
        r#"
        trait Code { assoc enabled: bool; }
        struct Subject where flag:bool {}
        impl Code for Subject<flag> where flag:bool {assoc enabled:bool=flag;}
        comptime impl Code for Subject<other> where other:bool {assoc enabled:bool=other;}
        comptime fn run()->bool {Subject<true>.as(Code).enabled}
    "#,
    );
    let package = support::compile(&source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let mismatch = source.replace("enabled:bool=other", "enabled:bool=not other");
    let diagnostic = support::compile_with_protocols(&mismatch, &Default::default()).unwrap_err();
    assert!(
        diagnostic.contains("complementary associated values differ"),
        "{diagnostic}"
    );
}
#[test]
fn member_prefix_attributes_can_use_the_enclosing_impl_self() {
    let source = "typealias Type=.Type'builtin; comptime fn metadata(T:Type)->.Object'builtin {let value={comptime_phase:T == Subject};value} trait Flag {^{comptime_phase:true} comptime fn flag()->bool;} struct Subject {} comptime impl Flag for Subject {^metadata(Self) comptime fn flag()->bool {true}} comptime fn run()->bool {Subject.as(Flag).flag()}";
    let package = support::compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn comptime_only_trait_identity_survives_lowering_without_changing_functions() {
    let source = "
        trait Ordinary {}
        pub pure comptime trait Stage {
            fn deferred(self)->bool;
            comptime fn run(self)->bool;
            pure comptime fn stable(self)->bool {true}
        }
    ";
    let Artifact::Hir(text) = compile(source, Emit::Hir).unwrap() else {
        panic!()
    };
    assert_eq!(
        text.matches("(trait-definition (comptime-only true)")
            .count(),
        1,
        "{text}"
    );
    assert_eq!(
        text.matches("(trait-definition (comptime-only false)")
            .count(),
        1,
        "{text}"
    );
    assert!(!text.contains("__flurry_kw_"), "{text}");
    for modifiers in [
        "(pure false) (comptime false)",
        "(pure false) (comptime true)",
        "(pure true) (comptime true)",
    ] {
        assert!(text.contains(modifiers), "{modifiers}: {text}");
    }
    let Artifact::Nir(text) = compile(source, Emit::Nir).unwrap() else {
        panic!()
    };
    assert!(text.to_string().contains("(comptime-only true)"), "{text}");
}

#[test]
fn comptime_only_trait_members_execute_with_explicit_interface_identity() {
    let source = "
        pure comptime trait Code {
            comptime fn value()->bool;
            comptime fn default_value()->bool {true}
            assoc Flag: bool = true;
        }
        struct Subject {}
        comptime impl Code for Subject { comptime fn value()->bool {true} }
        comptime fn run()->bool {Subject.as(Code).value() and Subject.as(Code).default_value()}
    ";
    let package = support::compile(source);
    let contract = package.traits.iter().find(|t| t.name == "Code").unwrap();
    assert!(contract.comptime_only);
    assert!(
        contract
            .methods
            .iter()
            .all(|m| m.phase == nir::program::TraitPhase::Comptime)
    );
    assert!(
        contract
            .associated
            .iter()
            .all(|m| m.phase == nir::program::TraitPhase::Comptime)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn comptime_only_trait_rejects_runtime_implementation_even_when_empty() {
    for body in ["", "comptime fn value()->bool;"] {
        let source = format!(
            "pure comptime trait Code {{{body}}} struct Subject {{}} impl Code for Subject {{}}"
        );
        let failure = compile(&source, Emit::Nir).err().expect("invalid phase");
        assert!(
            failure.contains("comptime-only trait requires comptime impl"),
            "{failure}"
        );
    }
}

#[test]
fn comptime_only_trait_member_attributes_are_idempotent_but_cannot_disable_phase() {
    let source = phase_source("pure comptime trait Code { ^staged comptime fn value()->bool; }");
    let package = support::compile(&source);
    assert!(package.traits.iter().any(|t| t.comptime_only));
    let source = phase_source(
        "pure comptime trait Code { ^{comptime_phase: false} comptime fn value()->bool; }",
    );
    let failure = compile(&source, Emit::Nir).err().expect("invalid override");
    assert!(
        failure.contains("comptime-only trait cannot contain runtime phase members"),
        "{failure}"
    );
}

#[test]
fn nir_rejects_forged_runtime_members_and_contributions_of_comptime_only_traits() {
    let source = "
        pure comptime trait Code {comptime fn value()->bool; assoc Flag: bool = true;}
        struct Subject {}
        comptime impl Code for Subject {comptime fn value()->bool {true}}
    ";
    let original = support::compile(source);
    nir::phases::validate(&original).unwrap();
    for associated in [false, true] {
        let mut package = original.clone();
        let contract = package
            .traits
            .iter_mut()
            .find(|t| t.name == "Code")
            .unwrap();
        if associated {
            contract.associated[0].phase = nir::program::TraitPhase::Runtime;
        } else {
            contract.methods[0].phase = nir::program::TraitPhase::Runtime;
        }
        let failure = nir::phases::validate(&package).unwrap_err();
        assert_eq!(
            failure.message,
            "comptime-only trait contains runtime phase members"
        );
    }
    let mut package = original.clone();
    package.implementations[0].phases.runtime = true;
    assert_eq!(
        nir::phases::validate(&package).unwrap_err().message,
        "comptime-only trait has a runtime implementation"
    );
    nir::phases::validate(&original).unwrap();
}

#[test]
fn comptime_only_trait_does_not_authorize_runtime_use_of_ordinary_fn() {
    let source = "
        pure comptime trait Code {fn value()->bool;}
        struct Subject {}
        comptime impl Code for Subject {fn value()->bool {true}}
        fn run()->bool {Subject.as(Code).value()}
    ";
    let failure = compile(source, Emit::Nir).err().expect("runtime use");
    assert!(
        failure.contains("comptime-only trait method requires comptime context"),
        "{failure}"
    );
}

#[test]
fn comptime_only_child_requires_comptime_parent_contribution() {
    for parent_phase in ["", "comptime "] {
        let source = format!(
            "
            trait Parent {{}}
            pure comptime trait Child :- Parent {{}}
            struct Subject {{}}
            {parent_phase}impl Parent for Subject {{}}
            comptime impl Child for Subject {{}}
        "
        );
        if parent_phase.is_empty() {
            let failure = compile(&source, Emit::Nir)
                .err()
                .expect("missing parent phase");
            assert!(
                failure.contains("parent interface lacks comptime phase"),
                "{failure}"
            );
        } else {
            compile(&source, Emit::Nir).unwrap();
        }
    }
}

#[test]
fn comptime_capability_members_prefer_comptime_only_trait_in_either_bound_order() {
    for bounds in ["Ordinary + Staged", "Staged + Ordinary"] {
        let source = format!(
            "
            trait Ordinary {{comptime fn read(self)->bool;}}
            pure comptime trait Staged {{comptime fn read(self)->bool;}}
            struct Subject {{}}
            impl Ordinary for Subject {{comptime fn read(self)->bool {{false}}}}
            comptime impl Staged for Subject {{comptime fn read(self)->bool {{true}}}}
            comptime fn choose(value:T)->bool where T :- {bounds} {{value.read()}}
            comptime fn run()->bool {{choose(Subject {{}})}}
        "
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
    }
}

#[test]
fn explicit_capability_source_and_definition_time_binding_keep_their_identity() {
    for expression in ["value.as(Ordinary).read()", "value.read()"] {
        let bounds = if expression.contains(".as(") {
            "Ordinary + Staged"
        } else {
            "Ordinary"
        };
        let source = format!(
            "
            trait Ordinary {{comptime fn read(self)->bool;}}
            pure comptime trait Staged {{comptime fn read(self)->bool;}}
            struct Subject {{}}
            impl Ordinary for Subject {{comptime fn read(self)->bool {{false}}}}
            comptime impl Staged for Subject {{comptime fn read(self)->bool {{true}}}}
            comptime fn choose(value:T)->bool where T :- {bounds} {{{expression}}}
            comptime fn run()->bool {{choose(Subject {{}})}}
        "
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(false)
        );
    }
}

#[test]
fn same_tier_comptime_capability_sources_remain_ambiguous() {
    for bounds in ["First + Second", "Second + First"] {
        let source = format!(
            "
            pure comptime trait First {{comptime fn read(self)->bool;}}
            pure comptime trait Second {{comptime fn read(self)->bool;}}
            comptime fn choose(value:T)->bool where T :- {bounds} {{value.read()}}
        "
        );
        let failure = compile(&source, Emit::Nir)
            .err()
            .expect("ambiguous sources");
        assert!(failure.contains("ambiguous capability member"), "{failure}");
    }
}

#[test]
fn chosen_comptime_capability_source_does_not_fall_back_after_signature_failure() {
    let source = "
        trait Ordinary {comptime fn read(self)->bool;}
        pure comptime trait Staged {comptime fn read(self, flag:bool)->bool;}
        comptime fn choose(value:T)->bool where T :- Ordinary + Staged {value.read()}
    ";
    let failure = compile(source, Emit::Nir)
        .err()
        .expect("missing selected argument");
    assert!(
        !failure.contains("ambiguous capability member"),
        "{failure}"
    );
    assert!(failure.contains("argument"), "{failure}");
}

#[test]
fn runtime_capability_lookup_excludes_comptime_only_sources() {
    let source = "
        trait Ordinary {fn read(self)->bool;}
        pure comptime trait Staged {fn read(self)->bool;}
        fn choose(value:T)->bool where T :- Ordinary + Staged {value.read()}
    ";
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!()
    };
    let ordinary = package
        .traits
        .iter()
        .find(|t| t.name == "Ordinary")
        .unwrap();
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "choose")
        .unwrap();
    assert!(
        function.body.to_string().contains(&format!(
            "(interface (interface-instance {}",
            ordinary.declaration
        )),
        "{}",
        function.body
    );
}

#[test]
fn member_phase_attribute_does_not_grant_interface_source_priority() {
    let source = phase_source(
        "
        trait Ordinary {comptime fn read(self)->bool;}
        trait StagedMember {^staged comptime fn read(self)->bool;}
        comptime fn choose(value:T)->bool where T :- Ordinary + StagedMember {value.read()}
    ",
    );
    let failure = compile(&source, Emit::Nir).err().expect("same source tier");
    assert!(failure.contains("ambiguous capability member"), "{failure}");
}

#[test]
fn implicit_inherited_sources_use_priority_but_explicit_parent_view_keeps_ambiguity() {
    let prefix = "
        trait Ordinary {comptime fn read(self)->bool;}
        pure comptime trait Staged {comptime fn read(self)->bool;}
        pure comptime trait Child :- Ordinary + Staged {}
    ";
    compile(
        &format!("{prefix} comptime fn choose(value:T)->bool where T :- Child {{value.read()}}"),
        Emit::Nir,
    )
    .unwrap();
    let failure = compile(
        &format!(
            "{prefix} comptime fn choose(value:T)->bool where T :- Child {{value.as(Child).read()}}"
        ),
        Emit::Nir,
    )
    .err()
    .expect("explicit ambiguity");
    assert!(failure.contains("ambiguous inherited member"), "{failure}");
}

#[test]
fn quoted_runtime_member_lookup_does_not_inherit_its_generator_stage() {
    let source = "
        trait Ordinary {fn read(self)->bool;}
        pure comptime trait Staged {fn read(self)->bool;}
        comptime fn build()-><value:T|-bool> where T :- Ordinary + Staged {'(value.read())}
    ";
    compile(source, Emit::Nir).unwrap();
}

#[test]
fn concrete_receiver_selects_visible_comptime_interface() {
    let package = support::compile(
        "
        struct Subject {}
        trait Ordinary {fn read(self)->bool;}
        pure comptime trait Staged {comptime fn read(self)->bool;}
        impl Ordinary for Subject {fn read(self)->bool {false}}
        comptime impl Staged for Subject {comptime fn read(self)->bool {true}}
        fn runtime_read(value:Subject)->bool {value.read()}
        comptime fn run()->bool {Subject{}.read()}
    ",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let ordinary = package
        .traits
        .iter()
        .find(|t| t.name == "Ordinary")
        .unwrap();
    let runtime = package
        .functions
        .iter()
        .find(|f| f.name == "runtime_read")
        .unwrap();
    assert!(
        runtime.body.to_string().contains(&format!(
            "(interface (interface-instance {}",
            ordinary.declaration
        )),
        "{}",
        runtime.body
    );
}

#[test]
fn concrete_receiver_rejects_same_tier_sources() {
    let failure = compile(
        "
        struct Subject {}
        pure comptime trait First {comptime fn read(self)->bool;}
        pure comptime trait Second {comptime fn read(self)->bool;}
        comptime impl First for Subject {comptime fn read(self)->bool {true}}
        comptime impl Second for Subject {comptime fn read(self)->bool {false}}
        comptime fn run()->bool {Subject{}.read()}
    ",
        Emit::Nir,
    )
    .err()
    .expect("ambiguous source");
    assert!(
        failure.contains("ambiguous visible trait member"),
        "{failure}"
    );
}

#[test]
fn concrete_receiver_explicit_view_bypasses_source_competition() {
    let package = support::compile(
        "
        struct Subject {}
        pure comptime trait First {comptime fn read(self)->bool;}
        pure comptime trait Second {comptime fn read(self)->bool;}
        comptime impl First for Subject {comptime fn read(self)->bool {true}}
        comptime impl Second for Subject {comptime fn read(self)->bool {false}}
        comptime fn run()->bool {Subject{}.as(First).read()}
    ",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn concrete_receiver_infers_interface_arguments_from_optional_subject() {
    let package = support::compile(
        "
        pure comptime trait Presence where T {comptime fn present(self)->bool;}
        comptime impl Presence<T> for ?T where T {
            comptime fn present(self)->bool = self match {_? => true, null => false,};
        }
        comptime fn run()->bool {let value:?bool = true; value.present()}
    ",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn concrete_receiver_does_not_discover_unimported_trait() {
    let failure = compile(
        "
        struct Subject {}
        mod hidden {
            pub pure comptime trait Protocol {comptime fn read(self)->bool;}
            comptime impl Protocol for Subject {comptime fn read(self)->bool {true}}
        }
        comptime fn run()->bool {Subject{}.read()}
    ",
        Emit::Nir,
    )
    .err()
    .expect("hidden source");
    assert!(failure.contains("unknown struct field read"), "{failure}");
}

#[test]
fn concrete_receiver_import_aliases_keep_one_interface_identity() {
    let package = support::compile(
        "
        struct Subject {}
        mod provider {
            pub pure comptime trait Protocol {comptime fn read(self)->bool;}
            comptime impl Protocol for Subject {comptime fn read(self)->bool {true}}
        }
        use provider.Protocol as First;
        use provider.Protocol as Second;
        comptime fn run()->bool {Subject{}.read()}
    ",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn concrete_receiver_discovers_inherited_method_through_imported_child() {
    let package = support::compile(
        "
        struct Subject {}
        mod protocols {
            pub pure comptime trait Parent where T {comptime fn read(self)->bool;}
            pub pure comptime trait Child :- Parent<T> where T {}
            comptime impl Parent<bool> for Subject {comptime fn read(self)->bool {true}}
            comptime impl Child<bool> for Subject {}
        }
        use protocols.Child;
        comptime fn run()->bool {Subject{}.read()}
    ",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn concrete_receiver_inherited_diamond_deduplicates_origin() {
    let package = support::compile(
        "
        struct Subject {}
        mod protocols {
            pub pure comptime trait Parent {comptime fn read(self)->bool;}
            pub pure comptime trait Left :- Parent {}
            pub pure comptime trait Right :- Parent {}
            pub pure comptime trait Child :- Left + Right {}
            comptime impl Parent for Subject {comptime fn read(self)->bool {true}}
            comptime impl Left for Subject {}
            comptime impl Right for Subject {}
            comptime impl Child for Subject {}
        }
        use protocols.Child;
        comptime fn run()->bool {Subject{}.read()}
    ",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn concrete_receiver_parent_impl_does_not_grant_child_visibility() {
    let failure = compile(
        "
        struct Subject {}
        mod protocols {
            pub pure comptime trait Parent {comptime fn read(self)->bool;}
            pub pure comptime trait Child :- Parent {}
            comptime impl Parent for Subject {comptime fn read(self)->bool {true}}
        }
        use protocols.Child;
        comptime fn run()->bool {Subject{}.read()}
    ",
        Emit::Nir,
    )
    .err()
    .expect("child not implemented");
    assert!(failure.contains("unknown struct field read"), "{failure}");
}

#[test]
fn concrete_receiver_inherited_sources_use_declaring_interface_phase() {
    let source = phase_source(
        "
        struct Subject {}
        mod protocols {
            pub trait Ordinary {^staged comptime fn read(self)->bool;}
            pub pure comptime trait Staged {comptime fn read(self)->bool;}
            pub pure comptime trait Child :- Ordinary + Staged {}
            comptime impl Ordinary for Subject {comptime fn read(self)->bool {false}}
            comptime impl Staged for Subject {comptime fn read(self)->bool {true}}
            comptime impl Child for Subject {}
        }
        use protocols.Child;
        comptime fn run()->bool {Subject{}.read()}
    ",
    );
    let package = support::compile(&source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn concrete_receiver_distinct_inherited_origins_remain_ambiguous() {
    let failure = compile(
        "
        struct Subject {}
        mod protocols {
            pub pure comptime trait First {comptime fn read(self)->bool;}
            pub pure comptime trait Second {comptime fn read(self)->bool;}
            pub pure comptime trait Child :- First + Second {}
            comptime impl First for Subject {comptime fn read(self)->bool {true}}
            comptime impl Second for Subject {comptime fn read(self)->bool {false}}
            comptime impl Child for Subject {}
        }
        use protocols.Child;
        comptime fn run()->bool {Subject{}.read()}
    ",
        Emit::Nir,
    )
    .err()
    .expect("inherited ambiguity");
    assert!(
        failure.contains("ambiguous visible trait member"),
        "{failure}"
    );
}

#[test]
fn concrete_receiver_child_declaration_shadows_inherited_member() {
    let package = support::compile(
        "
        struct Subject {}
        mod protocols {
            pub pure comptime trait Parent {comptime fn read(self)->bool;}
            pub pure comptime trait Child :- Parent {comptime fn read(self)->bool;}
            comptime impl Parent for Subject {comptime fn read(self)->bool {false}}
            comptime impl Child for Subject {comptime fn read(self)->bool {true}}
        }
        use protocols.Child;
        comptime fn run()->bool {Subject{}.read()}
    ",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn implicit_associated_callable_uses_comptime_source_priority() {
    let source = phase_source(
        "
        comptime fn accept(value:bool)->bool {value}
        comptime fn reject(value:bool)->bool {not value}
        struct Subject {}
        trait Ordinary {^staged assoc action:comptime fn(bool)->bool;}
        pure comptime trait Staged {assoc action:comptime fn(bool)->bool;}
        comptime impl Ordinary for Subject {assoc action:comptime fn(bool)->bool = reject;}
        comptime impl Staged for Subject {assoc action:comptime fn(bool)->bool = accept;}
        comptime fn run()->bool {Subject.action(true)}
    ",
    );
    let package = support::compile(&source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn implicit_associated_callable_inherits_generic_capability_source() {
    let package = support::compile(
        "
        comptime fn accept(value:bool)->bool {value}
        struct Subject {}
        pure comptime trait Parent {assoc action:comptime fn(bool)->bool;}
        pure comptime trait Child :- Parent {}
        comptime impl Parent for Subject {assoc action:comptime fn(bool)->bool = accept;}
        comptime impl Child for Subject {}
        comptime fn apply(value:bool)->bool where T :- Child {T.action(value)}
        comptime fn run()->bool {apply<Subject>(true)}
    ",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn implicit_associated_callable_inherits_visible_concrete_child() {
    let package = support::compile(
        "
        comptime fn accept(value:bool)->bool {value}
        struct Subject {}
        mod protocols {
            pub pure comptime trait Parent {assoc action:comptime fn(bool)->bool;}
            pub pure comptime trait Child :- Parent {}
            comptime impl Parent for Subject {assoc action:comptime fn(bool)->bool = accept;}
            comptime impl Child for Subject {}
        }
        use protocols.Child;
        comptime fn run()->bool {Subject.action(true)}
    ",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn implicit_associated_callable_does_not_recover_declaration_parameter_names() {
    let failure = compile(
        "
        comptime fn accept(value:bool)->bool {value}
        struct Subject {}
        pure comptime trait Protocol {assoc action:comptime fn(bool)->bool;}
        comptime impl Protocol for Subject {assoc action:comptime fn(bool)->bool = accept;}
        comptime fn run()->bool {Subject.action(.value=true)}
    ",
        Emit::Nir,
    )
    .err()
    .expect("stored callable has no parameter names");
    assert!(
        failure.contains("named") || failure.contains("parameter name"),
        "{failure}"
    );
}

#[test]
fn implicit_associated_noncallable_does_not_fall_back_to_ordinary_method() {
    let source = phase_source(
        "
        struct Subject {}
        trait Ordinary {^staged comptime fn action(value:bool)->bool;}
        pure comptime trait Staged {assoc action:bool;}
        comptime impl Ordinary for Subject {comptime fn action(value:bool)->bool {value}}
        comptime impl Staged for Subject {assoc action:bool = true;}
        comptime fn run()->bool {Subject.action(true)}
    ",
    );
    let failure = compile(&source, Emit::Nir)
        .err()
        .expect("selected value is not callable");
    assert!(
        failure.contains("callable") || failure.contains("function"),
        "{failure}"
    );
}

#[test]
fn concrete_source_search_retains_budget_for_multiple_small_candidates() {
    let mut source = String::from("struct Subject {}\n");
    for i in 0..8 {
        source.push_str(&format!(
            "
            pure comptime trait Protocol{i} {{comptime fn read(self)->bool;}}
            comptime impl Protocol{i} for Subject {{comptime fn read(self)->bool {{true}}}}
        "
        ));
    }
    source.push_str("comptime fn run()->bool {Subject{}.read()}");
    let failure = compile(&source, Emit::Nir)
        .err()
        .expect("eight distinct sources");
    assert!(
        failure.contains("ambiguous visible trait member"),
        "{failure}"
    );
}

#[test]
fn generic_comptime_child_checks_selected_parent_phase_at_concrete_use() {
    for parent_phase in ["", "comptime "] {
        let source = format!(
            "
            trait Parent {{}}
            pure comptime trait Child :- Parent {{comptime fn read(self)->bool;}}
            struct Subject {{}}
            {parent_phase}impl Parent for Subject {{}}
            comptime impl Child for T where T :- Parent {{
                comptime fn read(self)->bool {{true}}
            }}
            comptime fn run()->bool {{Subject{{}}.as(Child).read()}}
        "
        );
        if parent_phase.is_empty() {
            let failure = compile(&source, Emit::Nir)
                .err()
                .expect("runtime parent is insufficient");
            assert!(
                failure.contains("parent interface lacks comptime phase"),
                "{failure}"
            );
        } else {
            let package = support::compile(&source);
            assert_eq!(
                luna::invocation::invoke(&package, "run", 100_000).unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
        }
    }
}

#[test]
fn generic_comptime_child_does_not_fall_back_to_less_specific_parent_phase() {
    let failure = compile(
        "
        trait Parent {}
        pure comptime trait Child :- Parent {comptime fn read(self)->bool;}
        struct Subject where T {}
        comptime impl Parent for Subject<T> where T {}
        impl Parent for Subject<bool> {}
        comptime impl Child for T where T :- Parent {
            comptime fn read(self)->bool {true}
        }
        comptime fn run()->bool {Subject<bool>{}.as(Child).read()}
    ",
        Emit::Nir,
    )
    .err()
    .expect("selected precise parent has no comptime phase");
    assert!(
        failure.contains("parent interface lacks comptime phase"),
        "{failure}"
    );
}

#[test]
fn irrelevant_ambiguous_trait_names_do_not_poison_member_discovery() {
    let package = support::compile(
        "
        struct Subject {}
        mod a {pub pure comptime trait Protocol {comptime fn unrelated(self)->bool;}}
        mod b {pub pure comptime trait Protocol {comptime fn other(self)->bool;}}
        use a.*;
        use b.*;
        pure comptime trait Reader {comptime fn read(self)->bool;}
        comptime impl Reader for Subject {comptime fn read(self)->bool {true}}
        comptime fn run()->bool {Subject{}.read()}
        ",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn relevant_ambiguous_trait_names_preserve_inherited_source_errors() {
    for member in ["comptime fn read(self)->bool;", ""] {
        let parent = if member.is_empty() { " :- Parent" } else { "" };
        let source = format!(
            "
            struct Subject {{}}
            pure comptime trait Parent {{comptime fn read(self)->bool;}}
            mod a {{ pub pure comptime trait Protocol{parent} {{ {member} }} }}
            mod b {{pub pure comptime trait Protocol {{}}}}
            use a.*;
            use b.*;
            pure comptime trait Reader {{comptime fn read(self)->bool;}}
            comptime impl Reader for Subject {{comptime fn read(self)->bool {{true}}}}
            comptime fn run()->bool {{Subject{{}}.read()}}
        "
        );
        let failure = compile(&source, Emit::Nir)
            .err()
            .expect("relevant ambiguity");
        assert!(
            failure.contains("ambiguous glob name `Protocol`"),
            "{failure}"
        );
    }
}

#[test]
fn lower_tier_ambiguous_trait_names_do_not_block_comptime_source() {
    let package = support::compile(
        "
        struct Subject {}
        mod a {pub trait Protocol {fn read(self)->bool;}}
        mod b {pub trait Protocol {fn read(self)->bool;}}
        use a.*;
        use b.*;
        pure comptime trait Reader {comptime fn read(self)->bool;}
        comptime impl Reader for Subject {comptime fn read(self)->bool {true}}
        comptime fn run()->bool {Subject{}.read()}
        ",
    );
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn runtime_member_discovery_excludes_ambiguous_comptime_only_traits() {
    compile(
        "
        struct Subject {}
        mod a {pub pure comptime trait Protocol {comptime fn read(self)->bool;}}
        mod b {pub pure comptime trait Protocol {comptime fn read(self)->bool;}}
        use a.*;
        use b.*;
        trait Reader {fn read(self)->bool;}
        impl Reader for Subject {fn read(self)->bool {true}}
        fn run()->bool {Subject{}.read()}
        ",
        Emit::Nir,
    )
    .unwrap_or_else(|error| panic!("{error}"));
}

#[test]
fn ambiguous_nontrait_bindings_shadow_an_outer_trait_spelling() {
    compile(
        "
        trait Protocol {fn read(self)->bool;}
        struct Subject {}
        trait Reader {fn read(self)->bool;}
        impl Reader for Subject {fn read(self)->bool {true}}
        mod a {pub struct Protocol {}}
        mod b {pub struct Protocol {}}
        mod client {
            use a.*;
            use b.*;
            fn run()->bool {Subject{}.read()}
        }
        ",
        Emit::Nir,
    )
    .unwrap_or_else(|error| panic!("{error}"));
}
