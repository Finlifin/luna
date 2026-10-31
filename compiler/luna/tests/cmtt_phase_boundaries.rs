#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

#[test]
fn unused_code_generators_cannot_hide_comptime_storage_in_newtypes() {
    let source = "newtype Hidden=.Type'builtin;
        comptime fn identity(code:<|-Hidden>)-><|-Hidden> {code}";
    let result = support::compile_with_protocols(source, &Default::default());
    assert!(
        result.is_err(),
        "accepted a runtime code interface with a Type representation"
    );
    let error = result.unwrap_err();
    assert!(error.contains("runtime"), "{error}");
}

#[test]
fn code_interfaces_inspect_wrapped_storage_in_results_contexts_and_patterns() {
    for underlying in [
        ".Type'builtin",
        ".Integer'builtin",
        ".Float'builtin",
        ".Object'builtin",
        "comptime fn()->bool",
    ] {
        for declaration in [
            format!("newtype Hidden={underlying};"),
            format!("newtype Wrapper<T> = T;newtype Hidden=Wrapper<{underlying}>;"),
            format!("newtype Inner={underlying};struct Hidden {{value:Inner}}"),
            format!("newtype Inner={underlying};enum Hidden {{empty,wrapped(Inner),}}"),
        ] {
            for signature in [
                "comptime fn identity(code:<|-Hidden>)-><|-Hidden> {code}",
                "comptime fn identity(code:<value:Hidden|-bool>)-><value:Hidden|-bool> {code}",
                "comptime fn identity(code:<|-?Hidden>)-><|-?Hidden> {code}",
                "comptime fn identity(code:<|-fn()->Hidden>)-><|-fn()->Hidden> {code}",
                "comptime fn identity(code:<Hidden -| value:Hidden>)-><Hidden -| value:Hidden> {code}",
                "comptime fn identity(code:<bool -| value:Hidden>)-><bool -| value:Hidden> {code}",
                "comptime fn ignored(code:<Hidden -| value:Hidden>)-><|-bool> {'(true)}",
                "fn unused()->bool {const code:<x:Hidden|-bool> = '(true);true}",
                "fn unused()->bool {const code:<Hidden -| value:Hidden> = '<value>;true}",
            ] {
                let source = format!("{declaration} {signature}");
                let result = support::compile_with_protocols(&source, &Default::default());
                assert!(result.is_err(), "accepted {source}");
                let error = result.unwrap_err();
                assert!(!error.contains("parsing failed"), "{source}: {error}");
                assert!(error.contains("runtime interface"), "{source}: {error}");
            }
        }
    }
}

#[test]
fn runtime_newtypes_and_phantom_comptime_inputs_remain_valid_code() {
    let source = "newtype Wrapper<T> = T;newtype Flag=Wrapper<bool>;
        newtype Phantom<T> = bool;
        comptime fn forward(code:<x:Flag|-Flag>)-><x:Flag|-Flag> {code}
        comptime fn keep_unused(code:<|-Phantom<.Type'builtin>>)-><|-Phantom<.Type'builtin>> {code}
        comptime fn pattern(code:<Flag -| value:Flag>)-><Flag -| value:Flag> {code}
        fn run()->bool {
            let original=true.as(Wrapper<bool>).as(Flag);
            const code:<x:Flag|-Flag> = '(x);
            let selected=~[x=original]<forward(code)>;
            const p=pattern('<value>);
            let ~<p> = selected;
            value.as(Wrapper<bool>).as(bool)
        }";
    assert_eq!(
        support::run_iir(source, "run", vec![]),
        comptime_interpretor::Value::Bool(true)
    );
}
