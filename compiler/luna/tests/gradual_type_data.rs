//! Type data can travel through ordinary values before an identity is needed.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::{ErrorKind, Value};
use nir::{Primitive, Term, program::DescriptionLiteral};

#[test]
fn recursive_runtime_identity_keeps_its_own_admitted_signature() {
    let source = r#"
        comptime fn keep(action: fn(bool, bool) -> bool) -> fn(bool, bool) -> bool { action }
        fn convert(value: Any, inspect: bool) -> Any {
            const callback = keep(convert);
            if inspect { let observed = callback(true, false); assumes observed; };
            value
        }
        fn run() -> bool {
            let byte: fn(u8, bool) -> u8 = convert;
            let flag: fn(bool, bool) -> bool = convert;
            byte(42, true) == 42 and flag(true, true)
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));

    let guarded = source.replace("-> Any {", "-> Any where requires inspect {");
    let error = support::compile(&guarded).unwrap_err();
    assert!(error.contains("erase program preconditions"), "{error}");
}

#[test]
fn descriptions_survive_calls_and_containers_without_supplying_identity() {
    let package = support::compile(
        r#"
        use builtin.Any as Field;
        struct Slot { domain: Type }
        const Unknown: Type = Field;
        const Stored: meta.List<Type> = [Unknown, u8];
        comptime fn Forward(value: Type) -> Type { value }
        comptime fn FromFields(choose: bool) -> Type {
            let fields: meta.List<Type> = [Forward(Stored[0]), bool];
            let slot = Slot { domain: fields[0] };
            if choose { slot.domain } else { fields[1] }
        }
        comptime fn compare(left: Type, right: Type) -> bool { left == right }
        "#,
    )
    .unwrap();
    let id = |name| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    let any = Value::Description(DescriptionLiteral::Any);
    let boolean = Value::Type(Term::Primitive(Primitive::Bool));
    for reference in [false, true] {
        support::with_comptime(&package, support::COMPTIME_BUDGET, reference, |vm| {
            let select = id("FromFields");
            assert_eq!(vm.invoke(select, vec![Value::Bool(true)]).unwrap(), any);
            assert_eq!(
                vm.invoke(select, vec![Value::Bool(false)]).unwrap(),
                boolean
            );
            let compare = id("compare");
            let error = vm
                .invoke(compare, vec![any.clone(), boolean.clone()])
                .unwrap_err();
            assert_eq!(error.kind, ErrorKind::TypeContractUnresolved);
            assert_eq!(
                vm.invoke(compare, vec![boolean.clone(), boolean.clone()])
                    .unwrap(),
                Value::Bool(true)
            );
        });
    }
}

#[test]
fn a_gradual_type_constant_cannot_supply_a_concrete_field_layout() {
    let error = support::compile("const Field: Type = Any; struct Payload { data: Field }")
        .expect_err("a stored field requires an identity");
    assert!(error.contains("gradual description"), "{error}");
}

#[test]
fn alias_declarations_and_value_uses_preserve_the_producers_type_data() {
    let package = support::compile(
        r#"
        pub mod fields {
            pub typealias Slot = Any;
            pub typealias Slots<T> = T;
        }
        comptime fn Select(flag: bool) -> Type { if flag { fields.Slot } else { u8 } }
        typealias Field = Select(true);
        typealias Second<A, B> = B;
        comptime fn packet() -> Tuple<Type, Type> { (bool, fields.Slots<Field>) }
        typealias Forward = Second<...packet()>;
        typealias Byte = Select(false);
        comptime fn FromAlias() -> Type {
            let fields: meta.List<Type> = [fields.Slots<Forward>, Byte];
            fields[0]
        }
        fn run(value: Byte) -> u8 { value }
    "#,
    )
    .unwrap();
    let entry = package
        .functions
        .iter()
        .find(|item| item.name == "FromAlias")
        .unwrap()
        .declaration;
    for reference in [false, true] {
        support::with_comptime(&package, support::COMPTIME_BUDGET, reference, |vm| {
            assert_eq!(
                vm.invoke(entry, vec![]).unwrap(),
                Value::Description(DescriptionLiteral::Any)
            );
        });
    }
    let error =
        support::compile("typealias Field = Any; struct Payload { data: Field }").unwrap_err();
    assert!(error.contains("requires a type identity"), "{error}");
}

#[test]
fn closed_alias_production_is_shared_across_unrelated_substitutions() {
    let source = r#"
        comptime fn Make() -> Type {
            var builder = meta.StructBuilder.new(.name = .Generated);
            builder.add_field(.flag, bool);
            builder.Build()
        }
        typealias Declared = Make();
        typealias Through<T> = Declared;
        typealias Fresh = Make();

        struct Holder where T { pub typealias Element = T; }
        typealias Octet = Holder<u8>.Element;
        fn retain(value: Holder<bool>.Element) -> bool { value }
        fn select(value: Through<bool>) -> Through<u8> { value }
        comptime fn check() -> bool { Through<bool> == Declared and Declared != Fresh }
        fn run() -> bool {
            let value = Declared { flag: true };
            let byte: Octet = 1;
            byte == 1 and retain(check()) and select(value).flag
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn a_computed_layout_consumer_reports_a_description_at_the_use() {
    let error =
        support::compile("comptime fn Field() -> Type { Any } struct Payload { value: Field() }")
            .unwrap_err();
    assert!(error.contains("requires a type identity"), "{error}");
}

#[test]
fn runtime_inputs_admit_completed_descriptions_at_each_call() {
    let source = r#"
        typealias Field = Any;
        comptime fn Select(flag: bool) -> Type { if flag { Field } else { u8 } }
        fn observe(value: Field) -> bool { true }
        fn selected(comptime flag: bool, value: defer Select(flag)) -> bool { observe(value) }
        fn count(calls: *u8) -> bool {
            let before = calls.*;
            if before < 255 { calls.* = before + 1; }
            true
        }
        fn run() -> bool {
            var calls: u8 = 0;
            let byte: u8 = 3;
            selected(true, count(calls.ref)) and selected(true, byte) and
                selected(false, byte) and calls == 1
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn a_description_does_not_grant_unchecked_body_operations() {
    let failure = support::compile("fn bad(value: Any) -> bool { value.missing }").unwrap_err();
    assert!(
        failure.contains("missing") || failure.contains("projection"),
        "{failure}"
    );
}

#[test]
fn a_gradual_result_checks_its_body_against_each_delivery_domain() {
    let source = r#"
        fn pass(value: Any) -> Any { value }
        fn run() -> bool {
            let integer: u32 = 42;
            let result: u32 = pass(integer);
            let flag: bool = pass(true);
            result == integer and flag
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    let incompatible = source.replace("pass(true)", "pass(integer)");
    let error = support::compile(&incompatible).unwrap_err();
    assert!(error.contains("bool") && error.contains("u32"), "{error}");
}

#[test]
fn an_identity_produced_by_a_deferred_domain_still_checks_the_actual() {
    let failure = support::compile(
        r#"
        comptime fn Select(flag: bool) -> Type { if flag { Any } else { u8 } }
        fn consume(comptime flag: bool, value: defer Select(flag)) {}
        fn run() { consume(false, true); }
    "#,
    )
    .unwrap_err();
    assert!(
        failure.contains("bool") && failure.contains("u8"),
        "{failure}"
    );
}

#[test]
fn distinct_gradual_inputs_preserve_named_order_defaults_and_inferred_clauses() {
    let source = r#"
        comptime fn Domain(flag: bool) -> Type { if flag { Any } else { u8 } }
        fn pick(comptime flag: bool, witness: T, .first: defer Domain(flag) = false,
                .second: Any = true) -> T where T { witness }
        fn record(state: *bool, next: bool) -> bool { state.* = next; next }
        fn run() -> bool {
            var state = false;
            let byte: u8 = 12;
            let result = pick(true, byte,
                .second = record(state.ref, false), .first = record(state.ref, true));
            result == byte and state and pick(true, true)
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn a_callable_target_instantiates_the_gradual_input_and_result_by_slot() {
    let source = r#"
        fn pass(value: Any) -> Any { value }
        fn call(f: fn(bool) -> bool) -> bool { f(true) }
        fn run() -> bool {
            let byte: fn(u8) -> u8 = pass;
            call(pass) and byte(7) == 7
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn an_annotation_completed_for_an_earlier_source_argument_runs_once() {
    let source = r#"
        typealias Selector = %{used: bool} comptime fn() -> Type;
        comptime fn make() -> Selector {
            var used = false;
            || { used = not used; if used { Any } else { bool } }
        }
        comptime fn consume(select: Selector, comptime .gate: bool = true,
                            .field: defer select() = false) -> bool {
            select() == bool
        }
        const result = consume(make(), .field = true, .gate = true);
        fn run() -> bool { result }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn produced_refinements_bind_each_call_subject_in_the_receiving_world() {
    let source = r#"
        trait Mark {}
        impl Mark for bool {}
        impl Mark for u8 {}
        impl Mark for u16 {}
        comptime fn Marked() -> Type { refines { Itself :- Mark } Any }
        comptime fn Small() -> Type {
            refines { Itself == u8 or Itself == bool } Marked()
        }
        typealias Field = Small();
        const fields: meta.List<Type> = [Field];
        comptime fn Load() -> Type { fields[0] }
        fn accept(value: defer Load()) -> bool { true }
        fn run() -> bool { let byte: u8 = 42; accept(byte) and accept(true) }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    for invalid in [
        source.replace("byte: u8", "byte: u16"),
        source.replace("impl Mark for bool {}", ""),
    ] {
        let error = support::compile(&invalid).unwrap_err();
        assert!(
            error.contains("refinement requirement is not established"),
            "{error}"
        );
    }
}

#[test]
fn foreign_refinement_at_the_same_slot_cannot_cross_vm_worlds() {
    let source = r#"
        trait Valid {}
        comptime fn Field() -> Type { refines { Itself :- Valid } Any }
        comptime fn Echo(value: Type) -> Type { value }
    "#;
    let first = support::compile(source).unwrap();
    let second = support::compile(source).unwrap();
    let entry = |pkg: &nir::program::Package, name| {
        pkg.functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    for reference in [false, true] {
        let field = support::with_comptime(&first, support::COMPTIME_BUDGET, reference, |vm| {
            vm.invoke(entry(&first, "Field"), vec![]).unwrap()
        });
        support::with_comptime(&second, support::COMPTIME_BUDGET, reference, |vm| {
            let local = vm.invoke(entry(&second, "Field"), vec![]).unwrap();
            assert_ne!(field, local);
            assert!(vm.invoke(entry(&second, "Echo"), vec![field]).is_err());
            assert_eq!(
                vm.invoke(entry(&second, "Echo"), vec![local.clone()])
                    .unwrap(),
                local
            );
        });
    }
}

#[test]
fn a_parameter_refinement_supplies_body_evidence_without_proving_the_caller() {
    let source = r#"
        trait Flag { fn flag(self) -> bool; }
        impl Flag for bool { fn flag(self) -> bool { self } }
        typealias Field = refines { Itself :- Flag } Any;
        fn read(value: Field) -> bool { value.flag() }
        fn run() -> bool { read(true) }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    let invalid = source.replace("read(true)", "do { let byte: u8 = 1; read(byte) }");
    let error = support::compile(&invalid).unwrap_err();
    assert!(
        error.contains("refinement requirement is not established"),
        "{error}"
    );
}

#[test]
fn normal_annotations_keep_callable_binders_and_computed_field_descriptions() {
    let source = r#"
        trait Flag {}
        impl Flag for bool {}
        typealias Callback = fn(comptime T: Type, value: T) -> T;
        pure comptime fn Pair<A, B> -> Type { A }
        comptime fn Field() -> Type { refines { Itself :- Flag } Any }
        typealias Payload = Pair<Callback, Field()>;
        fn accept(value: Payload) -> bool { true }
        fn forward(value: Pair<Callback, bool>) -> bool { accept(value) }
    "#;
    support::compile(source).unwrap();
    let wrong_callback = source.replace("Pair<Callback, bool>", "Pair<fn(bool) -> bool, bool>");
    let error = support::compile(&wrong_callback).unwrap_err();
    assert!(
        error.contains("concrete normal-form input does not match"),
        "{error}"
    );
    let wrong_field = source.replace("Pair<Callback, bool>", "Pair<Callback, u8>");
    let error = support::compile(&wrong_field).unwrap_err();
    assert!(
        error.contains("refinement requirement is not established"),
        "{error}"
    );
}

#[test]
fn constructed_descriptions_keep_the_head_formation_requirements() {
    let source = r#"
        pure comptime fn Checked<T, Gate> -> Type where requires Gate == bool { bool }
        comptime fn Make(gate: Type) -> Type { Checked<Any, gate> }
    "#;
    let package = support::compile(source).unwrap();
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "Make")
        .unwrap()
        .declaration;
    for tree in [false, true] {
        support::with_comptime(&package, support::COMPTIME_BUDGET, tree, |vm| {
            assert!(
                vm.invoke(
                    entry,
                    vec![Value::Type(nir::Term::Primitive(nir::Primitive::U8))]
                )
                .is_err()
            );
            assert!(matches!(
                vm.invoke(
                    entry,
                    vec![Value::Type(nir::Term::Primitive(nir::Primitive::Bool))]
                )
                .unwrap(),
                Value::Description(_)
            ));
        });
    }
}

#[test]
fn refined_applications_bind_field_and_whole_value_subjects_separately() {
    let source = r#"
        trait Field {}
        impl Field for u8 {}
        trait Packet { fn valid(self) -> bool; }
        impl Packet for Tuple<u8, bool> { fn valid(self) -> bool { true } }
        comptime fn Slots() -> Type { Tuple<refines { Itself :- Field } Any, Any> }
        comptime fn PacketDomain() -> Type { refines { Itself :- Packet } Slots() }
        typealias Payload = PacketDomain();
        fn accept(value: Payload) -> bool {
            inline for field in value'tuple_enumerate { };
            value.valid()
        }
        fn run() -> bool { let byte: u8 = 5; accept((byte, true)) }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    for (invalid, diagnostic) in [
        (
            source.replace("impl Field for u8 {}", ""),
            "refinement requirement is not established",
        ),
        (
            source.replace("accept((byte, true))", "accept((byte, byte))"),
            "refinement requirement is not established",
        ),
        (
            source.replace("accept((byte, true))", "accept(byte)"),
            "expected the normal-form head",
        ),
    ] {
        let error = support::compile(&invalid).unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
    }
}

#[test]
fn checked_result_guarantees_include_refined_literals_on_each_return_path() {
    let source = r#"
        comptime fn Slots() -> Type { Tuple<Any> }
        typealias Packet = refines { Itself == Tuple<bool> } Slots();
        pure comptime fn Select(flag: bool) -> R: Type
        where ensures Type.predicates.is_tuple(R)
        {
            let nested = || { return bool; };
            flag match { true => Packet, false => Packet }
        }
        fn consume(comptime flag: bool, ...args: defer Select(flag)) -> bool {
            inline for field in args'tuple_enumerate { };
            true
        }
        fn run() -> bool { consume(true, true) and consume(false, false) }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    let invalid = source.replace("false => Packet", "false => bool");
    let error = support::compile(&invalid).unwrap_err();
    assert!(
        error.contains("contract") || error.contains("tuple"),
        "{error}"
    );
}

#[test]
fn a_returned_signed_description_establishes_integer_without_selecting_a_width() {
    let source = r#"
        typealias Signed = refines { Type.predicates.is_signed_integer(Itself) } Any;
        pure comptime fn Domain() -> R: Type
        where ensures Type.predicates.is_integer(R)
        {
            Signed
        }
        fn accept(value: defer Domain()) -> bool { true }
        fn run() -> bool {
            let small: i8 = 1;
            let wide: i64 = 2;
            accept(small) and accept(wide)
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    let invalid = source.replace("let wide: i64", "let wide: u64");
    let error = support::compile(&invalid).unwrap_err();
    assert!(error.contains("refinement requirement"), "{error}");
}

#[test]
fn type_producer_contracts_specialize_observed_value_clauses() {
    let source = r#"
        struct Tag where Domain: Type, mark: Domain { ready: bool, }
        pure comptime fn Select(input: Type) -> R: Type
        where Domain: Type, mark: Domain, ensures R == Tag<Domain, mark>
        {
            Tag<Domain, mark>
        }
        pure comptime fn Relay(input: Type) -> R: Type
        where ensures R == Tag<bool, true>
        {
            Select<bool, true>(input)
        }
        fn take(comptime X: Type, tagged: defer Select<bool, true>(X)) -> bool {
            let x: Tag<bool, true> = tagged;
            x.ready
        }
        pure comptime fn Shape(T: Type) -> R: Type
        where one: bool, ensures Type.predicates.is_tuple(R)
        {
            if one { Tuple<T> } else { Tuple<T, T> }
        }
        pure comptime fn invert(flag: bool) -> bool { not flag }
        fn count(comptime X: Type, comptime flag: bool, ...values: defer Shape<invert(flag)>(X)) -> u8 {
            var n: u8 = 0;
            inline for value in values'tuple_enumerate {
                assumes n < 255;
                n = n + 1;
            }
            n
        }
        fn run() -> bool {
            const relay_ok = Relay(bool) == Tag<bool, true>;
            relay_ok and take(u8, Tag<bool, true> { ready: true })
                and count(bool, false, true) == 1 and count(bool, true, true, false) == 2
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));

    let wrong_identity = source.replace("let x: Tag<bool, true>", "let x: Tag<bool, false>");
    let error = support::compile(&wrong_identity).unwrap_err();
    assert!(error.contains("type mismatch"), "{error}");

    let undocumented = source.replace(
        "where one: bool, ensures Type.predicates.is_tuple(R)",
        "where one: bool",
    );
    let error = support::compile(&undocumented).unwrap_err();
    assert!(
        error.contains("tuple_enumerate requires a proven tuple"),
        "{error}"
    );

    let wrong_relay = source.replace(
        "where ensures R == Tag<bool, true>",
        "where ensures R == Tag<bool, false>",
    );
    let error = support::compile(&wrong_relay).unwrap_err();
    assert!(
        error.contains("result guarantee contract is not established"),
        "{error}"
    );
}

#[test]
fn suspended_producer_contracts_reach_comptime_and_pending_parameter_bodies() {
    let producer = r#"
        pure comptime fn Shape(T: Type) -> R: Type
        where one: bool, ensures Type.predicates.is_tuple(R)
        {
            if one { Tuple<T> } else { Tuple<T, T> }
        }

        pure comptime fn Domain(flag: bool) -> Type { if flag { Type } else { bool } }

    "#;
    let comptime_consumer = r#"
        comptime fn count(T: Type, ...fields: defer Shape<true>(T)) -> u8 {
            var n: u8 = 0;
            inline for _ in fields'tuple_enumerate { assumes n < 255; n = n + 1; };
            n
        }
    "#;
    let dependent_consumer = r#"
        fn later(comptime flag: bool, comptime T: defer Domain(flag), ...fields: defer Shape<false>(T)) -> u8 {
            var n: u8 = 0;
            inline for _ in fields'tuple_enumerate { assumes n < 255; n = n + 1; };
            n
        }
    "#;
    let entry = r#"
        fn run() -> bool {
            const first = count(bool, false);
            first == 1 and later(true, bool, true, false) == 2
        }
    "#;
    let valid = [producer, comptime_consumer, dependent_consumer, entry].concat();
    assert_eq!(support::run(&valid), Value::Bool(true));

    let undocumented = producer.replace(", ensures Type.predicates.is_tuple(R)", "");
    for consumer in [comptime_consumer, dependent_consumer] {
        let error = support::compile(&format!("{undocumented}{consumer}")).unwrap_err();
        assert!(
            error.contains("tuple_enumerate requires a proven tuple"),
            "{error}"
        );
    }
}

#[test]
fn forwarded_type_results_use_the_supplied_ordinary_slot_identity() {
    let source = r#"
        pure comptime fn Second(comptime A: Type, comptime B: Type) -> R: Type
        where ensures R == B
        {
            B
        }
        pure comptime fn Relay(comptime X: Type, comptime Y: Type) -> R: Type
        where ensures R == X
        {
            Second(Y, X)
        }
        fn run() -> bool {
            const valid = Relay(u8, bool) == u8 and Relay(bool, u8) == bool;
            valid
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    for body in ["Second(X, Y)", "Second(do { X = Y; Y }, X)"] {
        let error = support::compile(&source.replace("Second(Y, X)", body)).unwrap_err();
        assert!(
            error.contains("result guarantee contract is not established"),
            "{error}"
        );
    }
}

#[test]
fn deferred_type_result_equality_restores_sparse_source_parameters() {
    let producer = r#"
        pure comptime fn Echo(flag: bool, comptime T: Type) -> R: Type
        where ensures R == T
        {
            T
        }
    "#;
    let consumer = r#"
        fn count(comptime Y: Type, comptime flag: bool, comptime X: Type, ...items: defer Echo(flag, X)) -> u8
        where requires Type.predicates.is_tuple(X)
        {
            var n: u8 = 0;
            inline for _ in items'tuple_enumerate { assumes n < 255; n = n + 1; };
            n
        }
    "#;
    let entry = "fn run() -> bool { count(u8, false, Tuple<bool>, true) == 1 }";
    // The runtime consumer prepares a sparse capture frame. The ordinary
    // flag of the comptime consumer suspends the same annotation instead.
    let suspended = consumer
        .replace("fn count", "comptime fn count")
        .replace("comptime flag: bool", "flag: bool");
    for consumer in [consumer, suspended.as_str()] {
        assert_eq!(
            support::run(&format!("{producer}{consumer}{entry}")),
            Value::Bool(true)
        );
        let wrong = consumer.replace("Echo(flag, X)", "Echo(flag, Y)");
        let error = support::compile(&format!("{producer}{wrong}")).unwrap_err();
        assert!(
            error.contains("tuple_enumerate requires a proven tuple"),
            "{error}"
        );
    }
}

#[test]
fn constructed_type_results_keep_their_heads_and_mixed_input_identities() {
    let source = r#"
        struct Pair where A, B { flag: bool }
        pure comptime fn Opaque<A, B> -> Type { B }
        pure comptime fn Joined(comptime T: Type) -> R: Type
        where A, ensures R == Pair<A, ?T>
        {
            Pair<A, ?T>
        }
        pure comptime fn Relay(comptime T: Type, comptime U: Type) -> R: Type
        where ensures R == Pair<T, ?U>
        {
            Joined<T>(U)
        }
        pure comptime fn Wrapped(comptime T: Type) -> R: Type
        where A, ensures R == Opaque<A, T>
        {
            Opaque<A, T>
        }
        fn run() -> bool {
            const exact = Relay(bool, u8) == Pair<bool, ?u8>
                and Wrapped<bool>(u8) == Opaque<bool, u8>;
            exact
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    for (from, to) in [
        ("ensures R == Pair<A, ?T>", "ensures R == Pair<T, ?A>"),
        ("ensures R == Opaque<A, T>", "ensures R == T"),
        ("            Pair<A, ?T>", "            T = A; Pair<A, ?T>"),
    ] {
        let error = support::compile(&source.replace(from, to)).unwrap_err();
        assert!(
            error.contains("result guarantee contract is not established"),
            "{error}"
        );
    }
}

#[test]
fn constructed_result_proofs_observe_computed_packets_in_source_order() {
    let source = r#"
        pure comptime fn Opaque<A, B, C, D> -> Type { D }
        struct Quad where A, B, C, D { tag: bool }
        pure comptime fn Hidden(comptime T: Type) -> Type { T }

        pure comptime fn Packed(comptime T: Type) -> R: Type
        where A, ensures R == Opaque<A, T, ?T, A>
        {
            Opaque<...do { (A, T) }, ...do { (?T, A) }>
        }

        pure comptime fn Nominal(comptime T: Type) -> R: Type
        where A, ensures R == Quad<A, T, ?T, A>
        {
            Quad<...do { (A, T) }, ...do { (?T, A) }>
        }

        fn run() -> bool {
            const exact = Packed<bool>(u8) == Opaque<bool, u8, ?u8, bool>
                and Nominal<bool>(u8) == Quad<bool, u8, ?u8, bool>;
            exact
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    for last in ["(A, ?T)", "(?T, Hidden(A))"] {
        let altered = source.replacen("do { (?T, A) }", &format!("do {{ {last} }}"), 1);
        let error = support::compile(&altered).unwrap_err();
        assert!(
            error.contains("result guarantee contract is not established"),
            "{error}"
        );
    }
}

#[test]
fn collected_result_proofs_use_the_callers_dependent_element_domain() {
    let source = r#"
        pure comptime fn Packet<.Element: Type = Type, ...xs: meta.List<Element>> -> Type {
            Element
        }
        pure comptime fn Packed(comptime T: Type, comptime x: T) -> R: Type
        where ensures R == Packet<.Element = T, x, x>
        {
            Packet<x, x, .Element = T>
        }
        fn run() -> bool {
            const exact = Packed(bool, true) == Packet<.Element = bool, true, true>
                and Packed(u8, 7) == Packet<.Element = u8, 7, 7>;
            exact
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    let error =
        support::compile(&source.replace("Packet<x, x, .Element = T>", "Packet<x, .Element = T>"))
            .unwrap_err();
    assert!(
        error.contains("result guarantee contract is not established"),
        "{error}"
    );
}

#[test]
fn a_computed_description_reaches_the_dependent_slots_identity_boundary() {
    let definitions = r#"
        comptime fn Selected(flag: bool) -> Type { if flag { bool } else { Any } }
        fn empty(comptime ty: Type) -> ?ty { null }
        fn inspect() -> bool where flag: bool {
            empty(Selected(flag)) match { null => true, _ => false }
        }
    "#;
    assert_eq!(
        support::run(&format!(
            "{definitions} fn run() -> bool {{ inspect<true>() }}"
        )),
        Value::Bool(true)
    );
    let error = support::compile(&format!(
        "{definitions} fn run() -> bool {{ inspect<false>() }}"
    ))
    .expect_err("a gradual description cannot supply the payload identity");
    assert!(
        error.contains("requires a type identity") && error.contains("gradual description"),
        "{error}"
    );
}
