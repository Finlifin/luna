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
            include!("support/meta_source.rs")
        ),
        &support::meta_fixture_protocols(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )
}

#[test]
fn normal_form_subjects_select_static_trait_methods() {
    for declarations in [
        DECLARATIONS.to_owned(),
        format!("{DECLARATIONS} const Alias = meta.List;"),
    ] {
        let head = if declarations.contains("const Alias") {
            "Alias"
        } else {
            "meta.List"
        };
        let package = compile(&format!(
            "{declarations} comptime fn run()->bool {{ {head}<bool>.as(Mark).marked() }}"
        ))
        .unwrap();
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            Value::Bool(true),
        );
    }
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

#[test]
fn list_normal_form_resolves_inherent_receiver_methods() {
    let package =
        compile("comptime fn run()->bool {let xs:meta.List<bool> = [true];xs.length() == 1}")
            .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn list_methods_do_not_unfold_factory_heads() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn Identity<T>->Type {T}
        comptime fn bad(values:Identity<meta.List<bool>>)->.Integer'builtin {values.length()}";
    assert!(compile(source).is_err());
}

#[test]
fn builtin_callable_aliases_preserve_associated_type_subjects() {
    let package = compile(&format!(
        "{DECLARATIONS}
        const Alias = meta.List;
        trait Element {{ assoc Item: Type; }}
        impl Element for meta.List<T> where T {{ assoc Item: Type = T; }}
        comptime fn Direct() -> Type {{ meta.List<bool>.as(Element).Item }}
        comptime fn ViaAlias() -> Type {{ Alias<bool>.as(Element).Item }}
    "
    ))
    .unwrap();
    for entry in ["Direct", "ViaAlias"] {
        assert_eq!(
            luna::invocation::invoke(&package, entry, 100_000).unwrap(),
            Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
        );
    }
}
