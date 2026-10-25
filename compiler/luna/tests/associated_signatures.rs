//! Associated types in implementation method signatures and clause domains.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

#[test]
fn parent_projection_supplies_method_value_parameter_domain() {
    let source = "typealias Type=.Type'builtin;
        trait Base {assoc Key:Type;}
        trait Child :- Base {fn selected()->bool where key:Self.as(Base).Key;}
        struct Slot {}
        impl Child for Slot {fn selected()->bool where key:bool {key}}
        impl Base for Slot {assoc Key:Type=bool;}
        fn run()->bool {Slot.as(Child).selected<true>() and not Slot.as(Child).selected<false>()}";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generic_parent_projection_checks_each_method_value_argument() {
    let source = "typealias Type=.Type'builtin;
        trait Base {assoc Key:Type;}
        trait Child :- Base {fn selected()->bool where key:Self.as(Base).Key;}
        struct Slot where T {}
        impl Child for Slot<T> where T {fn selected()->bool where key:T {true}}
        impl Base for Slot<T> where T {assoc Key:Type=T;}
        fn run()->bool {Slot<bool>.as(Child).selected<false>() and Slot<.u8'builtin>.as(Child).selected<7>()}";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for bad in [
        "fn bad()->bool {Slot<bool>.as(Child).selected<7>()}",
        "fn bad()->bool {Slot<.u8'builtin>.as(Child).selected<true>()}",
        "fn bad()->bool {Slot<.u8'builtin>.as(Child).selected<256>()}",
    ] {
        assert!(
            support::compile_with_protocols(&format!("{source}{bad}"), &Default::default())
                .is_err()
        );
    }
}

#[test]
fn projected_clause_domains_do_not_accept_incompatible_implementations() {
    for actual_domain in [".u8'builtin", "Identity<bool>"] {
        let source = format!(
            "typealias Type=.Type'builtin;
            pure comptime fn Identity<T>->Type {{T}}
            trait Base {{assoc Key:Type;}}
            trait Child :- Base {{fn selected()->bool where key:Self.as(Base).Key;}}
            struct Slot {{}}
            impl Child for Slot {{fn selected()->bool where key:{actual_domain} {{true}}}}
            impl Base for Slot {{assoc Key:Type=bool;}}"
        );
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("method clause domains do not match")
                || error.contains("normal-form parameter type is not yet supported"),
            "{error}"
        );
    }
}

#[test]
fn projected_method_defaults_use_prior_inputs_and_explicit_overrides() {
    let source = "typealias Type=.Type'builtin;
        trait Base {assoc Key:Type;}
        trait Child :- Base {
            fn selected()->Self.as(Base).Key where first:Self.as(Base).Key,
                .second:Self.as(Base).Key=first;
        }
        struct Slot where T {}
        impl Child for Slot<T> where T {
            fn selected()->T where first:T,.second:T=first {second}
        }
        impl Base for Slot<T> where T {assoc Key:Type=T;}
        fn run()->bool {
            Slot<bool>.as(Child).selected<true>()
            and not Slot<bool>.as(Child).selected<true,.second=false>()
            and Slot<.u8'builtin>.as(Child).selected<7>() == 7
            and Slot<.u8'builtin>.as(Child).selected<7,.second=9>() == 9
        }";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn projected_method_default_can_read_selected_parent_associated_value() {
    let source = "typealias Type=.Type'builtin;
        trait Base {assoc Key:Type;assoc initial:Key;}
        trait Child :- Base {
            fn selected()->Self.as(Base).Key where .key:Self.as(Base).Key=Self.as(Base).initial;
        }
        struct Slot {}
        impl Child for Slot {
            fn selected()->bool where .key:bool=false {key}
        }
        impl Base for Slot {assoc Key:Type=bool;assoc initial:bool=true;}
        fn run()->bool {Slot.as(Child).selected() and not Slot.as(Child).selected<.key=false>()}";
    let package = support::compile(source);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn generic_associated_defaults_keep_selected_value_identity() {
    for (domain, first, second) in [("bool", "true", "false"), (".u8'builtin", "7", "9")] {
        let source = format!(
            "typealias Type=.Type'builtin;
            trait Base {{assoc Key:Type;assoc initial:Key;}}
            trait Child :- Base {{
                fn selected()->Self.as(Base).Key where .key:Self.as(Base).Key=Self.as(Base).initial;
            }}
            struct Slot where seed:{domain} {{}}
            impl Base for Slot<seed> where seed:{domain} {{
                assoc Key:Type={domain};assoc initial:{domain}=seed;
            }}
            impl Child for Slot<seed> where seed:{domain} {{
                fn selected()->{domain} where .key:{domain}=seed {{key}}
            }}
            fn run()->bool {{
                Slot<{first}>.as(Base).initial == {first}
                and Slot<{first}>.as(Child).selected() == {first}
                and Slot<{second}>.as(Child).selected() == {second}
                and Slot<{first}>.as(Child).selected<.key={second}>() == {second}
            }}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn dependent_value_indices_select_associated_defaults_without_cross_instance_reuse() {
    let source = include_str!("../../../development/repros/dependent-associated-default.fl");
    let source = format!(
        "{source}
        struct Packet {{flag:bool}}
        fn packets()->bool {{
            Slot<Packet,Packet {{flag:true}}>.as(Child).selected().flag
            and not Slot<Packet,Packet {{flag:false}}>.as(Child).selected().flag
        }}
        fn override_value()->bool {{
            not Slot<bool,true>.as(Child).selected<.key=false>()
            and Slot<.u8'builtin,7>.as(Child).selected<.key=11>() == 11
        }}"
    );
    let package = support::compile(&source);
    for entry in ["run", "packets", "override_value"] {
        assert_eq!(
            luna::execution::interpret(&package, entry, 100_000).unwrap(),
            Value::Bool(true)
        );
    }
    for bad in [
        "fn bad()->bool {Slot<bool,7>.as(Child).selected()}",
        "fn bad()->bool {Slot<.u8'builtin,true>.as(Child).selected() == 1}",
        "fn bad()->bool {Slot<.u8'builtin,256>.as(Child).selected() == 1}",
        "fn bad()->bool {Slot<bool,true>.as(Child).selected<.key=7>()}",
    ] {
        assert!(
            support::compile_with_protocols(&format!("{source}{bad}"), &Default::default())
                .is_err()
        );
    }
}
