//! Method selection does not require revealing an opaque normal-form shape.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

const DECLARATIONS: &str = "
    typealias Type=.Type'builtin;
    pure comptime fn Identity<T>->Type {T}
    trait Mark {comptime fn marked()->bool;}
    impl Mark for meta.List<T> where T {comptime fn marked()->bool {true}}
    trait Boolean {comptime fn marked()->bool;}
    impl Boolean for bool {comptime fn marked()->bool {true}}
";

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod meta {{ {} }} {source}",
            include_str!("../../../library/meta/lib.fl")
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )
}

#[test]
fn normal_form_subjects_select_static_trait_methods() {
    let package = compile(&format!(
        "{DECLARATIONS}
        comptime fn run()->bool {{
            meta.List<bool>.as(Mark).marked()
        }}"
    ))
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true),
    );
}

#[test]
fn normal_form_method_lookup_does_not_unfold_heads_or_invent_members() {
    for body in [
        "bool.as(Mark).marked()",
        "bool.inherent()",
        "meta.List<bool>.missing()",
        "Identity<bool>.as(Boolean).marked()",
    ] {
        let source = format!("{DECLARATIONS} comptime fn bad()->bool {{{body}}}");
        assert!(
            compile(&source).is_err(),
            "{body} must not resolve through an invented type identity",
        );
    }
}
