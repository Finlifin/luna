#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::Value;
use support::compile_with_protocols;

#[test]
fn local_type_names_preserve_enum_namespaces_and_payload_order() {
    let package = sysroot::compile(
        "
        enum Event {
            idle,
            network.{data(bool), record {flag: bool}},
            struct_fields(bool),
        }
        struct Counter { calls: Integer }
        impl Counter {
            comptime fn next(self) -> bool {
                self.calls = self.calls + 1;
                self.calls == 2
            }
        }
        comptime fn enum_namespace_order() -> bool {
            const Selected = Event;
            const Alias = Selected;
            let counter = Counter { calls: 0 };
            let first = Alias.struct_fields(counter.next());
            let second = Selected.network.data(counter.next());
            let third = Alias.network.record { flag: counter.next() };
            let idle = Selected.idle;
            (first match { .struct_fields(flag) => not flag, _ => false })
                and (second match { Alias.network.data(flag) => flag, _ => false })
                and (third match { Selected.network.record {flag: flag} => not flag, _ => false })
                and (idle match { Alias.idle => true, _ => false })
                and (Selected.network.data(true) match { Alias.network => true, _ => false })
                and counter.calls == 3
        }
        ",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "enum_namespace_order", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn local_enum_namespace_rejects_groups_missing_members_and_wrong_payloads() {
    for body in [
        "Selected.network",
        "Selected.network.missing",
        "Selected.network.data(false, true)",
        "Selected.network.record {wrong: true}",
    ] {
        let source = format!(
            "enum Event {{ network.{{data(bool),record {{flag: bool}}}} }}
            fn bad() -> Event {{ const Selected = Event; {body} }}"
        );
        assert!(sysroot::compile(&source).is_err(), "accepted {body}");
    }
}

#[test]
fn declared_enum_names_keep_precedence_over_type_receiver_methods() {
    for argument in ["true", "selected()"] {
        let source = format!(
            "enum Choice where flag: bool {{ struct_fields(bool) }}
            comptime fn selected() -> bool {{ true }}
            struct Counter {{ calls: Integer }}
            impl Counter {{
                comptime fn next(self) -> bool {{ self.calls = self.calls + 1; true }}
            }}
            comptime fn enum_constructor_precedence() -> bool {{
                let counter = Counter {{ calls: 0 }};
                let value = Choice<{argument}>.struct_fields(counter.next());
                (value match {{ .struct_fields(flag) => flag }}) and counter.calls == 1
            }}"
        );
        let package = sysroot::compile(&source).unwrap();
        assert_eq!(
            luna::invocation::invoke(&package, "enum_constructor_precedence", 1_000_000).unwrap(),
            Value::Bool(true)
        );
        if argument == "selected()" {
            let invalid = source.replace(
                "comptime fn selected() -> bool { true }",
                "comptime fn selected() -> bool { let values: meta.List<bool> = []; values[0] }",
            );
            assert!(
                sysroot::compile(&invalid).is_err(),
                "constructor input failure was erased"
            );
        }
    }
}

#[test]
fn grouped_leaves_construct_the_outer_enum_and_whole_groups_match() {
    let source = "typealias Byte=.u8'builtin;
        enum Event {network.{connected,received(Byte),nested.{stalled}},stopped,}
        fn network(value:Event)->bool {value match {.network=>true,.stopped=>false}}
        fn run()->bool {network(Event.network.connected) and network(.network.received(3))
            and network(.network.nested.stalled) and not network(Event.stopped)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    let definition = &package.enums[0];
    assert_eq!(definition.groups, vec!["network", "network.nested"]);
    assert_eq!(
        definition
            .variants
            .iter()
            .map(|variant| variant.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "network.connected",
            "network.received",
            "network.nested.stalled",
            "stopped"
        ]
    );
    assert!(
        package
            .to_string()
            .contains("(groups \"network\" \"network.nested\")")
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let staged =
        compile_with_protocols(&source.replace("fn ", "comptime fn "), &Default::default())
            .unwrap();
    assert_eq!(
        luna::invocation::invoke(&staged, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn nested_payloads_defaults_and_contextual_inference_keep_leaf_indices() {
    let source = "typealias Byte=.u8'builtin;
        enum Packet where T {left.{value {payload:T,tag:Byte=7}},right.{value(T)},}
        fn accept(value:Packet<T>)->bool where T {value match {Packet<T>.left=>true,.right.value(_)=>false}}
        fn read(value:Packet<bool>)->Byte {value match {.left.value {tag:tag}=>tag,.right=>0}}
        fn run()->bool {accept(.left.value {payload:true}) and not accept(.right.value(true))
            and read(Packet<bool>.left.value {payload:false}) == 7}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn group_names_are_neither_values_nor_types_and_do_not_hide_missing_leaves() {
    let prefix = "enum E {a.{one,two},b.{one},}";
    for suffix in [
        "fn bad()->E {E.a}",
        "fn bad()->E {.a}",
        "typealias Bad=E.a;",
        "fn bad()->E {E.a.one(true)}",
        "fn bad(value:E)->bool {value match {.a.one=>true,.b=>false}}",
        "fn bad(value:E)->bool {value match {.missing=>true}}",
    ] {
        assert!(
            compile_with_protocols(&format!("{prefix}{suffix}"), &Default::default()).is_err(),
            "accepted {suffix}"
        );
    }
    for source in [
        "enum E {a.{one,one}}",
        "enum E {a.{one},a}",
        "enum E {a.{one},a.{two}}",
    ] {
        assert!(
            compile_with_protocols(source, &Default::default()).is_err(),
            "accepted {source}"
        );
    }
}

#[test]
fn group_encoding_constraints_are_checked_across_group_boundaries() {
    let source = "typealias Byte=.u8'builtin;typealias Nibble=.BitVec'builtin<4>;
        enum Code {.base_type Byte,.discrete_tag true,
            control.{idle=0,stop=1},data.{byte:0xA (value:Nibble)},}
        fn code(value:Code)->Byte {(value'bitvec).as(Byte)}
        fn control(value:Code)->bool {value match {.control=>true,.data=>false}}
        fn run()->bool {code(Code.data.byte {value:5})==0xa5 and control(.control.idle) and not control(.data.byte {value:0})}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let error = compile_with_protocols(&source.replace("idle=0", "idle=0xa5"), &Default::default())
        .err()
        .unwrap();
    assert!(error.contains("overlap"), "{error}");
}

#[test]
fn empty_and_prefix_neighbor_groups_have_exact_leaf_sets() {
    let source = "enum E {a.{one,nested.{two}},ab.{three},empty.{},}
        fn select(value:E)->bool {value match {.a.nested=>true,.a.one=>true,.ab=>false,.empty=>false}}
        fn run()->bool {select(.a.one) and select(E.a.nested.two) and not select(.ab.three)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let mismatch = "enum A {group.{one}} enum B {group.{one}} fn bad(value:A)->bool {value match {B.group=>true}}";
    assert!(compile_with_protocols(mismatch, &Default::default()).is_err());
}

#[test]
fn unused_group_defaults_are_checked_and_paths_are_validated_in_ir() {
    let invalid = "enum E {group.{item {value:bool=3}}}";
    assert!(compile_with_protocols(invalid, &Default::default()).is_err());
    let source = "enum E {group.{one}} fn run()->E {E.group.one}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    for groups in [
        vec![],
        vec!["group".to_string(), "group".to_string()],
        vec!["absent.child".to_string()],
    ] {
        let mut invalid = package.clone();
        invalid.enums[0].groups = groups;
        assert!(nir::phases::validate(&invalid).is_err());
        assert!(comptime_interpretor::Interpreter::new(&invalid, Default::default()).is_err());
    }
    let mut graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for nominal in &mut graph.nominals {
        if let iir::NominalDefinition::Enum(definition) = &mut nominal.definition {
            definition.groups.clear();
        }
    }
    assert!(comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).is_err());
}

#[test]
fn type_alias_group_prefix_is_not_evaluated_as_a_method_receiver() {
    let source = "enum Source {net.{ready,nested.{data(bool)}},other} typealias Event=Source;
        fn read(value:Event)->bool {value match {.net=>true,.other=>false}}
        fn run()->bool {read(Event.net.ready) and read(Event.net.nested.data(true)) and not read(Event.other)}";
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn named_payload_effects_follow_source_order_with_known_and_partial_targets() {
    let source = r#"
        enum Packet where T {
            group.{record { first: T, second: T, defaulted: bool = true }},
        }
        fn mark(state: *bool) -> bool {
            let previous = state.*;
            state.* = not previous;
            previous
        }
        fn read(value: Packet<T>) -> bool where T, requires T == bool {
            value matches .group.record { first: true, second: false, defaulted: true }
        }
        fn run() -> bool {
            var state = false;
            let known: Packet<bool> = .group.record {
                second: mark(state.ref), first: mark(state.ref)
            };
            read(known) and read(.group.record { second: mark(state.ref), first: mark(state.ref) })
                and not state
        }
    "#;
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn contextual_variant_and_symbol_literals_deliver_through_result_tags() {
    let source = r#"
        typealias Symbol = .Symbol'builtin;
        enum State { waiting, ready(bool) }
        enum ReadErr { Missing }
        fn run() -> bool {
            let state: !ReadErr ??State = .ready(true);
            let empty: ?State = .waiting;
            let key: !ReadErr ??Symbol = .waiting;
            (state matches State.ready(true)??!) and (empty matches State.waiting?)
                and (key matches .waiting??!)
        }
    "#;
    let package = compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn grouped_qualified_values_follow_the_payload_namespace_through_tags() {
    let source = r#"
        enum Packet { group.{byte(u8), idle} }
        enum ReadErr { Missing }
        comptime fn inspect() -> bool {
            let value: !ReadErr ?Packet = .group.byte(7);
            let idle: ?Packet = .group.idle;
            let present: ?u8 = .group.byte(0);
            let absent: ??u8 = .group.byte(99);
            (value matches .group.byte(7)?!) and (idle matches .group.idle?)
                and (present matches 103?) and (absent matches null?)
        }
        enum Message where T { group.{byte(T)} }
        fn present(value: ?Message<T>) -> bool where T { value matches .group.byte(_)? }
        fn run() -> bool {
            let item: u8 = 7;
            inspect() and present(.group.byte(item))
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let invalid = source.replace(".group.byte(7);", ".group.byte(256);");
    assert!(sysroot::compile(&invalid).is_err());
}
