//! Text-iteration prototype: algorithm checks do not establish view lifetimes.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

fn check(body: &str) {
    let source = format!(
        "{}\n{}\nmod iterator {{ {} }}\nmod text_bytes {{ {} }}\nmod text_iteration {{ {} }}\n{}",
        include_str!("../../../library/builtin/primitives.fl"),
        include_str!("../../../library/builtin/resources.fl"),
        include_str!("../../../library/builtin/iterator.fl"),
        include_str!("../../../library/builtin/text_bytes.fl"),
        include_str!("../../../development/library-prototypes/builtin/text_iteration.fl"),
        body
    );
    let package = support::compile_with_source_bindings_for_target(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            iterator: Some("input.iterator.Iterator"),
            copy: Some("input.Copy"),
            drop: Some("input.Drop"),
            ..Default::default()
        },
        Some(luna::frontend::native_test_target().unwrap()),
    )
    .unwrap();
    let mut expanded = package.clone();
    elaboration::program::inlining::expand(&mut expanded, &mut 2_000_000).unwrap();
    for package in [&package, &expanded] {
        assert_eq!(
            luna::execution::interpret(package, "run", 2_000_000).unwrap(),
            Value::Bool(true)
        );
        let error =
            luna::execution::interpret_with_protocols(package, "run", "Copy", "Drop", 2_000_000)
                .unwrap_err();
        assert!(error.contains("string-view memory provenance"), "{error}");
    }
}

#[test]
fn bytes_preserve_utf8_order_nuls_and_empty_input() {
    check(
        r#"
        fn run()->bool {
            let text="Aé中\0🦀";
            let alias=text;
            var count:u64=0;
            for value in text_iteration.bytes(alias) {
                let expected=text_bytes.byte(text,count);
                let equal=expected match {byte?=>byte == value,null=>false};
                let known:u8=count match {
                    0=>65,1=>195,2=>169,3=>228,4=>184,5=>173,
                    6=>0,7=>240,8=>159,9=>166,10=>128,_=>0,
                };
                assumes equal and value == known and count < 11;
                count = count + 1;
            }
            for value in text_iteration.bytes("") {return false;}
            count == 11 and text_bytes.byte_length(text) == 11
        }
        fn verify() {let passed=run();assumes passed;}
    "#,
    );
}

#[test]
fn bytes_compose_with_adapters_and_lexical_exits() {
    check(
        r#"
        use iterator.Iterator;
        fn first()->u8 {
            for value in text_iteration.bytes("abc") {return value;}
            0
        }
        fn run()->bool {
            var count:u64=0;
            for value in text_iteration.bytes("abc").as(Iterator).filter(|value| value.* != 98) {
                if value == 97 {continue;}
                assumes value == 99 and count == 0;
                count = 1;
                break;
            }
            count == 1 and first() == 97
        }
        fn verify() {let passed=run();assumes passed;}
    "#,
    );
}
