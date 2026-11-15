//! Open definitions check NF inputs through their callable or declaration domain.
#[path = "support/sysroot_execution.rs"]
mod sysroot;
use comptime_interpretor::Value;

const HEADS: &str = "
    pure comptime fn Identity<T> -> Type { T }
    pure comptime fn Other<T> -> Type { T }
    comptime fn Pick(flag: bool) -> Type { bool }
";

fn check_both(source: &str) {
    let package = sysroot::compile(source).unwrap();
    let entry = package
        .functions
        .iter()
        .find(|function| function.name == "check")
        .unwrap()
        .declaration;
    for reference in [false, true] {
        sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, reference, |vm| {
            assert_eq!(
                vm.invoke(entry, vec![]).unwrap(),
                Value::Bool(true),
                "reference={reference}"
            );
        });
    }
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn open_definitions_accept_normal_form_function_parameters_and_local_aliases() {
    for head in ["factory", "saved", "do { factory }"] {
        let source = format!(
            "{HEADS}
            comptime fn apply(factory: pure comptime fn<Type> -> Type,
                              input: Type, expected: Type, flag: bool) -> defer Pick(flag)
            {{
                let saved = factory;
                let formed = {head}<input>;
                formed == expected
            }}
            comptime fn check() -> bool {{
                apply(Identity, u8, Identity<u8>, true)
                    and apply(Other, bool, Other<bool>, false)
            }}
            fn run() -> bool {{ const value = check(); value }}
        "
        );
        check_both(&source);
    }
}

#[test]
fn open_normal_form_results_retain_the_supplied_head_and_input_identity() {
    let source = format!(
        "{HEADS}
        comptime fn ResultDomain(flag: bool) -> Type {{ Type }}
        comptime fn Apply(factory: pure comptime fn<Type> -> Type,
                          input: Type, flag: bool) -> defer ResultDomain(flag)
        {{
            let saved = factory;
            saved<input>
        }}
        comptime fn First() -> Type {{ Apply(Identity, u8, true) }}
        comptime fn Second() -> Type {{ Apply(Other, bool, false) }}
    "
    );
    let package = sysroot::compile(&source).unwrap();
    let id = |name| {
        package
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap()
            .declaration
    };
    for reference in [false, true] {
        sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, reference, |vm| {
            for (entry, head, input) in [
                ("First", "Identity", nir::Primitive::U8),
                ("Second", "Other", nir::Primitive::Bool),
            ] {
                assert_eq!(
                    vm.invoke(id(entry), vec![]).unwrap(),
                    Value::Type(nir::Term::NormalForm {
                        head: id(head),
                        sort: nir::Sort::Type,
                        arguments: vec![nir::Term::Primitive(input)],
                    }),
                    "{entry}, reference={reference}"
                );
            }
        });
    }
}

#[test]
fn open_normal_form_callables_share_collected_value_inputs_with_library_heads() {
    let source = format!(
        "{HEADS}
        pure comptime fn Bundle<...Es: meta.List<Type>> -> Type {{ bool }}
        comptime fn collect(factory: pure comptime fn<...Es: meta.List<Type>> -> Type,
                            inputs: meta.List<Type>, expected: Type,
                            flag: bool) -> defer Pick(flag)
        {{
            let formed = factory<...inputs>;
            formed == expected
        }}
        comptime fn check() -> bool {{
            collect(Tuple, [u8, bool], Tuple<.Es = [u8, bool]>, true)
                and collect(Bundle, [], Bundle<.Es = []>, false)
                and collect(Bundle, [bool], Bundle<.Es = [bool]>, true)
        }}
        fn run() -> bool {{ const value = check(); value }}
    "
    );
    check_both(&source);
}

#[test]
fn uninvoked_open_definitions_reject_bad_dynamic_normal_form_inputs() {
    for application in ["factory<true>", "factory<>", "factory<input, input>"] {
        let source = format!(
            "{HEADS}
            comptime fn unused(factory: pure comptime fn<Type> -> Type,
                               input: Type, flag: bool) -> defer Pick(flag)
            {{
                let formed = {application};
                true
            }}
        "
        );
        let failure = sysroot::compile(&source).unwrap_err();
        assert!(
            failure.contains("mismatch") || failure.contains("expected a type"),
            "{application}: {failure}"
        );
    }
}

#[test]
fn open_declaration_inputs_use_ordinary_control_flow_checks() {
    let source = format!(
        "{HEADS}
        comptime fn select(flag: bool) -> defer Pick(flag) {{
            let formed = Identity<when {{ flag => u8, else => bool }}>;
            formed == if flag {{ Identity<u8> }} else {{ Identity<bool> }}
        }}
        comptime fn check() -> bool {{ select(true) and select(false) }}
        fn run() -> bool {{ const value = check(); value }}
    "
    );
    check_both(&source);
    // Both branches must be checked at definition time, including an unused
    // function whose invalid branch would not be selected by a sample call.
    let invalid = format!(
        "{HEADS}
        comptime fn unused(flag: bool) -> defer Pick(flag) {{
            let formed = Identity<when {{ flag => u8, else => true }}>;
            true
        }}
    "
    );
    assert!(sysroot::compile(&invalid).is_err());
}

#[test]
fn open_definitions_do_not_treat_ordinary_function_values_as_normal_form_heads() {
    let source = format!(
        "{HEADS}
        comptime fn unused(factory: comptime fn(Type) -> Type,
                           input: Type, flag: bool) -> defer Pick(flag)
        {{
            let formed = factory<input>;
            true
        }}
    "
    );
    let failure = sysroot::compile(&source).unwrap_err();
    assert!(
        failure.contains("explicit instantiation requires a resolved generic function"),
        "{failure}"
    );
}
