//! Control transfers during contextual tuple preparation.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};
use support::{calls, compile, id};

#[test]
fn prepared_never_initializers_keep_formed_pattern_domains() {
    for (declaration, pattern) in [
        ("fn pair(a:bool,b:bool)->(bool,bool) {(a,b)}", "(a,b)"),
        (
            "fn pair(a:bool,b:bool)->((bool,bool),bool) {((a,b),a)}",
            "((a,b),_)",
        ),
        (
            "struct Pair {x:bool,y:bool} fn pair(a:bool,b:bool)->Pair {Pair {x:a,y:b}}",
            "{x:a,y:b}",
        ),
        (
            "enum Pair {pair(bool,bool)} fn pair(a:bool,b:bool)->Pair {.pair(a,b)}",
            ".pair(a,b)",
        ),
    ] {
        let source = format!(
            "{declaration}
            fn first()->bool {{true}}
            fn run()->bool {{
                let {pattern}=pair(first(),...do {{return true;(false,)}});
                a and b
            }}"
        );
        let package = support::compile_with_protocols(&source, &Default::default())
            .unwrap_or_else(|error| panic!("{pattern}: {error}"));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(calls(&package, &vm), ["run", "first"]);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let matched = source.replace(&format!("let {pattern}="), "").replace(
            ";\n                a and b",
            &format!(" match {{{pattern}=>a and b}}"),
        );
        let matched = support::compile_with_protocols(&matched, &Default::default())
            .unwrap_or_else(|error| panic!("match {pattern}: {error}"));
        assert_eq!(
            luna::execution::interpret(&matched, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        let wrong = source.replace(&format!("let {pattern}="), "let (a,b,c)=");
        assert!(
            support::compile_with_protocols(&wrong, &Default::default()).is_err(),
            "accepted an incompatible pattern: {pattern}"
        );
    }
    assert!(
        support::compile_with_protocols(
            "fn run()->bool {let (a,b)=do {return true;};a and b}",
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn contextual_enum_packets_can_exit_before_unpacking() {
    for packet in [
        "do {return true; (false,9)}",
        "if true {return true;} else {(false,9)}",
    ] {
        let package = compile(&format!(
            "typealias Byte=.u8'builtin;
            enum Packet where T,E {{pair(T,T,E)}}
            fn first()->bool {{true}}
            fn accept(value:Packet<T,Byte>)->bool where T {{false}}
            fn run()->bool {{accept(.pair(first(),...{packet}))}}"
        ));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(calls(&package, &vm), ["run", "first"]);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn contextual_enum_packets_preserve_named_loop_transfers() {
    for transfer in ["break outer", "continue outer"] {
        let package = compile(&format!(
            "typealias Byte=.u8'builtin;
            enum Packet where T,E {{pair(T,T,E)}}
            fn first()->bool {{true}}
            fn accept(value:Packet<T,Byte>)->bool where T {{false}}
            fn run()->bool {{
                var active=true;
                while:outer active {{
                    accept(.pair(first(),...do {{active=false; {transfer}; (false,9)}}));
                }}
                true
            }}"
        ));
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(calls(&package, &vm), ["run", "first"]);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn diverging_enum_packet_keeps_bound_tuple_layout_and_checks_dead_source() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Tuple<...Es:List<Type>>->Type {bool}
        typealias Byte=.u8'builtin;
        enum Packet where T,E {pair(T,T,E)}
        fn first()->bool {true}
        fn accept(value:Packet<T,Byte>)->bool where T {false}
        fn run()->bool {accept(.pair(first(),...do {return true; (false,9)}))}";
    let selectors = luna::protocols::type_constructors::Selectors {
        meta_list: Some("input::List"),
        tuple: Some("input::Tuple"),
        ..Default::default()
    };
    let package =
        support::compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for invalid in [
        source.replace("(false,9)", "(false,'x')"),
        source.replace("(false,9)", "(false,256)"),
        source.replace("(false,9)", "(false,9,true)"),
    ] {
        assert!(
            support::compile_with_source_bindings(&invalid, &Default::default(), selectors)
                .is_err()
        );
    }
}

#[test]
fn prepared_constructor_type_constrains_an_outer_effect_without_dispatch() {
    let package = compile(
        "typealias Byte=.u8'builtin;
        enum Packet where T,E {pair(T,T,E)}
        effect deliver(value:Packet<T,Byte>)->bool where T;
        fn first()->bool {true}
        fn handled()->bool {false}
        fn subject()->#[deliver<bool>] bool {
            deliver(.pair(first(),...do {return true; (false,9)}))#
        }
        fn run()->bool {subject()# {deliver<bool>(value)=>handled(),}}",
    );
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(calls(&package, &vm), ["run", "subject", "first"]);
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn arbitrary_divergence_does_not_invent_generic_type_evidence() {
    assert!(
        support::compile_with_protocols(
            "fn ignore(value:T)->bool where T {true}
         fn run()->bool {ignore(do {return true;})}",
            &Default::default()
        )
        .is_err()
    );
}
