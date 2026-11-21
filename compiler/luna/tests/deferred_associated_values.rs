//! Associated values selected after ordinary Type inputs become available.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

const SOURCE: &str = "typealias Type=.Type'builtin; typealias char=.char'builtin;
    trait Flag { assoc enabled:bool; }
    impl Flag for bool { assoc enabled:bool=true; }
    impl Flag for char { assoc enabled:bool=false; }
    comptime fn enabled(comptime T:Type)->bool
    where requires T :- Flag
    { T.as(Flag).enabled }
    comptime fn run()->bool { enabled(bool) and not enabled(char) and enabled(bool) }";

#[test]
fn a_default_can_read_another_implementation_then_resume_its_own() {
    let source = r#"
        struct Slot {}
        struct Peer {}
        trait Flag where T {
            assoc enabled: bool;
            assoc check: bool = Peer.as(Flag<bool>).enabled and Self.as(Flag<T>).enabled;
        }
        impl Flag<bool> for Peer { assoc enabled: bool = true; }
        impl Flag<bool> for Slot { assoc enabled: bool = false; }
        impl Flag<Peer> for Slot { assoc enabled: bool = true; }
        comptime fn run() -> bool {
            not Slot.as(Flag<bool>).check and Slot.as(Flag<Peer>).check
        }
    "#;
    let package = support::compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_type_inputs_select_associated_values_during_invocation() {
    let package = support::compile_with_protocols(SOURCE, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn associated_contracts_reject_missing_or_wrong_sort_caller_binders() {
    for (index, sort) in [(1, nir::Sort::Type), (0, nir::Sort::ComptimeValue)] {
        let mut package = support::compile_with_protocols(SOURCE, &Default::default()).unwrap();
        let function = package
            .functions
            .iter_mut()
            .find(|f| f.name == "enabled")
            .unwrap();
        let mut changed = false;
        function
            .body
            .rewrite_types(&mut |ty| {
                Ok::<_, ()>(if matches!(ty, nir::Term::Bound { .. }) {
                    changed = true;
                    nir::Term::Bound {
                        depth: 0,
                        index,
                        sort,
                    }
                } else {
                    ty.clone()
                })
            })
            .unwrap();
        assert!(changed);
        let failure = nir::phases::validate(&package).unwrap_err();
        assert_eq!(
            failure.message,
            "associated value contract substitution failed"
        );
    }
}

#[test]
fn associated_domains_keep_ordinary_interface_argument_scope() {
    let source = "typealias Type=.Type'builtin;
        trait Source where U { assoc initial:U; }
        impl Source<bool> for bool { assoc initial:bool=true; }
        comptime fn initial(comptime T:Type)->T
        where requires bool :- Source<T>
        { bool.as(Source<T>).initial }
        comptime fn run()->bool { initial(bool) }";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn required_computations_select_associated_values_after_type_binding() {
    let source = include_str!("../../../development/repros/deferred-associated-required.fl");
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "result", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn demanded_associated_initializers_execute_in_the_required_world() {
    let source =
        include_str!("../../../development/repros/deferred-associated-required.fl").replace(
            "assoc enabled: bool = true;",
            "assoc enabled: bool = initial();",
        ) + "pure comptime fn initial()->bool { true }";
    let package = support::compile_with_protocols(&source, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "result", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn required_type_results_preserve_selected_associated_types() {
    let source = "typealias Type=.Type'builtin;
        trait Shape { assoc Item:Type; }
        impl Shape for bool { assoc Item:Type=bool; }
        comptime fn Selected(comptime T:Type)->Type
        where requires T :- Shape
        { T.as(Shape).Item }
        typealias SelectedBool=Selected(bool);
        comptime fn run()->Type { SelectedBool }";
    for source in [
        source.to_owned(),
        source.replace("assoc Item:Type=bool;", "assoc Item:Type=Initialized();")
            + "pure comptime fn Initialized()->Type {bool}",
    ] {
        let package = support::compile_with_protocols(&source, &Default::default()).unwrap();
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
        );
    }
}

#[test]
fn required_associated_values_use_the_selected_member_domain() {
    let source = "typealias Type=.Type'builtin;
        trait Source { assoc Item:Type; assoc initial:Self.as(Source).Item; }
        impl Source for bool { assoc Item:Type=bool; assoc initial:bool=true; }
        comptime fn initial(comptime T:Type)->T.as(Source).Item
        where requires T :- Source
        { T.as(Source).initial }
        const saved=initial(bool);
        comptime fn run()->bool { saved }";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn required_inputs_observe_associated_domains_after_prior_type_arguments() {
    for phase in ["", "comptime "] {
        let source = format!(
            "typealias Type=.Type'builtin;
            trait Source {{ assoc Item:Type; }}
            impl Source for bool {{ assoc Item:Type=bool; }}
            comptime fn echo(comptime T:Type, {phase}value:T.as(Source).Item)->T.as(Source).Item
            where requires T :- Source
            {{ value }}
            const saved=echo(bool, true);
            comptime fn run()->bool {{ saved }}"
        );
        let package = support::compile_with_protocols(&source, &Default::default()).unwrap();
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn inherent_methods_keep_associated_inputs_through_selection_and_instantiation() {
    let source = "typealias Type=.Type'builtin;
        trait Source { assoc Item:Type; }
        impl Source for bool { assoc Item:Type=bool; }
        struct Cell where T { value:T }
        impl Cell<T> where T {
            comptime fn get(self)->T { self.value }
        }
        comptime fn read(comptime T:Type, value:Cell<T.as(Source).Item>)->T.as(Source).Item
        where requires T :- Source
        { value.get() }
        const saved=read(bool, Cell<bool> {value:true});
        comptime fn run()->bool { saved }";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn open_associated_receivers_keep_the_known_trait_inputs() {
    let source = r#"
        typealias Type = .Type'builtin;
        struct First {}
        struct Second {}
        trait Flag where U { assoc enabled: bool; }
        impl Flag<bool> for First { assoc enabled: bool = true; }
        impl Flag<First> for Second { assoc enabled: bool = VALUE; }
        comptime fn read(comptime T: Type) -> bool
        where requires T :- Flag<bool>
        { T.as(Flag<bool>).enabled }
        const VALUE: bool = read(First);
        comptime fn run() -> bool { Second.as(Flag<First>).enabled }
    "#;
    let package = support::compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let cycle = source
        .replace("Flag<First>", "Flag<bool>")
        .replace("= read(First)", "= read(Second)");
    let error = support::compile_with_protocols(&cycle, &Default::default()).unwrap_err();
    assert!(error.contains("cyclic"), "{error}");
}

#[test]
fn open_interface_inputs_keep_the_receiver_and_generic_initializer_code() {
    let source = r#"
        typealias Type = .Type'builtin;
        struct Holder where flag: bool {}
        struct Other {}
        trait Flag where U { assoc enabled: bool; }
        comptime fn keep(value: bool) -> bool { value }
        impl Flag<bool> for Holder<flag> where flag: bool { assoc enabled: bool = keep(flag); }
        impl Flag<bool> for Other { assoc enabled: bool = VALUE; }
        comptime fn read(comptime U: Type) -> bool
        where requires Holder<true> :- Flag<U>
        { Holder<true>.as(Flag<U>).enabled }
        const VALUE: bool = read(bool);
        comptime fn run() -> bool { Other.as(Flag<bool>).enabled }
    "#;
    let package = support::compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn a_generic_initializer_only_enters_a_recursive_member_on_the_taken_branch() {
    for access in ["Self.as(Flag).enabled", "enabled"] {
        let source = format!(
            "struct Holder where flag: bool {{}}
             trait Flag {{ assoc enabled: bool; }}
             impl Flag for Holder<flag> where flag: bool {{
                 assoc enabled: bool = if flag {{ true }} else {{ {access} }};
             }}
             const value: bool = Holder<true>.as(Flag).enabled;
             comptime fn run() -> bool {{ value }}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let cycle = source.replace("Holder<true>", "Holder<false>");
        let error = support::compile_with_protocols(&cycle, &Default::default()).unwrap_err();
        assert!(error.contains("cyclic associated initializer"), "{error}");
    }
}

#[test]
fn arithmetic_evidence_comes_from_completed_closed_members() {
    let source = r#"
        typealias u8 = .u8'builtin;
        trait Counter { assoc start: u8 = 255; }
        struct Small {}
        struct Full {}
        impl Counter for Small { assoc start: u8 = 1; }
        impl Counter for Full {}
        comptime fn run() -> bool { Small.as(Counter).start + 1 == 2 }
    "#;
    let package = support::compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let overflow = source.replace("Small.as(Counter)", "Full.as(Counter)");
    let open = source
        .replace(
            "comptime fn run() -> bool",
            "comptime fn run() -> bool where T :- Counter",
        )
        .replace("Small.as(Counter)", "T.as(Counter)");
    for invalid in [
        overflow + " const result = run();",
        open + " const result = run<Full>();",
    ] {
        let error = support::compile_with_protocols(&invalid, &Default::default()).unwrap_err();
        assert!(
            error.contains("machine arithmetic range is not proven"),
            "{error}"
        );
    }
}

#[test]
fn value_indexed_associated_type_programs_select_each_instance() {
    let source = r#"
        typealias Type = .Type'builtin;
        typealias char = .char'builtin;
        comptime fn Selected(enabled: bool) -> Type {
            if enabled { bool } else { char }
        }
        trait Source where enabled: bool { assoc Item: Type = Selected(enabled); }
        struct Holder where enabled: bool {}
        impl Source<enabled> for Holder<enabled> where enabled: bool {}
        impl Source<false> for bool {}
        impl Source<true> for bool { assoc Item: Type = char; }

        typealias Yes = Holder<true>.as(Source<true>).Item;
        typealias No = Holder<false>.as(Source<false>).Item;
        comptime fn yes() -> Yes { true }
        comptime fn no() -> No { 'n' }
        comptime fn inherited() -> bool.as(Source<false>).Item { 'i' }
        comptime fn replaced() -> bool.as(Source<true>).Item { 'r' }
    "#;
    let package = support::compile(source);
    for (name, expected) in [
        ("yes", Value::Bool(true)),
        ("no", Value::Char('n')),
        ("inherited", Value::Char('i')),
        ("replaced", Value::Char('r')),
    ] {
        assert_eq!(
            luna::invocation::invoke(&package, name, 100_000).unwrap(),
            expected
        );
    }
    let wrong = source.replace("fn no() -> No { 'n' }", "fn no() -> No { false }");
    let error = support::compile_with_protocols(&wrong, &Default::default()).unwrap_err();
    assert!(
        error.contains("type mismatch") && error.contains("char"),
        "{error}"
    );
}

#[test]
fn closed_associated_type_programs_remain_available_in_specialized_signatures() {
    let source = r#"
        typealias Type = .Type'builtin;
        comptime fn Selected(enabled: bool) -> Type { if enabled { bool } else { Type } }
        struct Holder where flag: bool {}
        trait Source { assoc Item: Type; }
        impl Source for Holder<flag> where flag: bool { assoc Item: Type = Selected(true); }
        comptime fn keep(value: T.as(Source).Item) -> T.as(Source).Item where T :- Source { value }
        comptime fn run() -> bool { keep<Holder<false>>(true) }
    "#;
    let package = support::compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn computed_associated_type_identities_preserve_normal_form_heads() {
    let source = r#"
        typealias Type = .Type'builtin;
        pure comptime fn Wrapped<T> -> Type { T }
        comptime fn Selected(flag: bool) -> Type { if flag { Wrapped<bool> } else { bool } }
        struct Holder where flag: bool {}
        trait Source { assoc Item: Type; }
        impl Source for Holder<flag> where flag: bool { assoc Item: Type = Selected(flag); }
        typealias Opaque = Holder<true>.as(Source).Item;
        fn keep(value: Wrapped<bool>) -> Opaque { value }
        comptime fn plain() -> Holder<false>.as(Source).Item { true }
    "#;
    let package = support::compile(source);
    assert_eq!(
        luna::invocation::invoke(&package, "plain", 100_000).unwrap(),
        Value::Bool(true)
    );
    let unfolded = source.replace("fn keep(value: Wrapped<bool>)", "fn keep(value: bool)");
    let error = support::compile_with_protocols(&unfolded, &Default::default()).unwrap_err();
    assert!(
        error.contains("normal-form") && error.contains("bool"),
        "{error}"
    );
}
