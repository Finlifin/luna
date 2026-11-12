//! Source contracts use completed values in their declared scalar domains.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::Value;

#[test]
fn code_method_program_refinement_uses_scalar_domains_with_real_sysroot() {
    for (domain, value) in [
        ("u8", "255"),
        ("i8", "-128"),
        ("u128", "340282366920938463463374607431768211455"),
        ("i128", "-170141183460469231731687303715884105728"),
        ("usize", "18446744073709551615"),
        ("char", "'界'"),
    ] {
        let source = format!(
            r#"
            trait Build where selected: {domain} {{
                comptime fn build(value: {domain}) -> <|- bool>
                where requires value == selected;
            }}
            struct Builder {{}}
            impl Build<{value}> for Builder {{
                comptime fn build(value: {domain}) -> <|- bool>
                where requires value == {value}
                {{ '(true) }}
            }}
            fn run() -> bool {{ ~<Builder.as(Build<{value}>).build({value})> }}
        "#
        );
        let package =
            sysroot::compile(&source).unwrap_or_else(|failure| panic!("{domain}: {failure}"));
        let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            Value::Bool(true),
            "{domain}"
        );
        if domain == "char" {
            let pattern = source.replace("requires value == '界'", "requires value matches '界'");
            assert_eq!(sysroot::run(&pattern), Value::Bool(true));
        }
        let invalid = source.replace(
            &format!("requires value == {value}"),
            &format!("requires value != {value}"),
        );
        let failure = sysroot::compile(&invalid).unwrap_err();
        assert!(
            failure.contains("strengthens trait program preconditions"),
            "{domain}: {failure}"
        );
    }
}
