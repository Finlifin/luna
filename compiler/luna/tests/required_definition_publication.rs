//! A required invocation delivers its entire generated world to later consumers.
use comptime_interpretor::Value;
#[path = "support/sysroot_execution.rs"]
mod sysroot;

#[test]
fn explicit_and_required_builders_prove_fields_against_source_implementations() {
    let source = r#"
        trait Marker {}
        impl Marker for bool {}
        struct Checked where T :- Marker { value: T }
        comptime fn Build() -> Type {
            let field = Checked<bool>;
            let wrapper = meta.NewtypeBuilder.new(field).Build();
            let record = meta.StructBuilder.new();
            record.add_field(.wrapped, wrapper);
            record.add_field(.value, field);
            let choice = meta.EnumBuilder.new();
            choice.add_tuple(.some, [record.Build(), field]);
            choice.Build()
        }
        comptime fn inspect() -> bool {
            let choice = Build();
            let fields = choice.enum_variant_field_types()[0];
            let record = fields[0].struct_field_types();
            Type.predicates.is_enum(choice) and Type.predicates.is_struct(fields[0]) and
                Type.predicates.is_newtype(record[0]) and
                record[0].NewtypeRepresentation() == Checked<bool> and
                record[1] == Checked<bool> and fields[1] == Checked<bool>
        }
        fn run() -> bool { inspect() }
    "#;
    assert_eq!(
        sysroot::invoke(source, "inspect").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn required_builders_deliver_cross_kind_layouts_to_reflection_and_dispatch() {
    assert_eq!(
        sysroot::run(
            "
            comptime fn Build() -> Type {
                let wrapper = meta.NewtypeBuilder.new(bool, .name = .Flag).Build();
                let record = meta.StructBuilder.new(.name = .Record);
                record.add_field(.flag, wrapper);
                let choice = meta.EnumBuilder.new(.name = .Choice);
                choice.add_tuple(.some, [record.Build()]);
                choice.Build()
            }
            typealias Choice = Build();
            comptime fn inspect(subject: Type) -> bool {
                let record = subject.enum_variant_field_types()[0][0];
                let wrapper = record.struct_field_types()[0];
                wrapper.NewtypeRepresentation() == bool
            }
            trait Ready { fn ready() -> bool; }
            impl Ready for Choice {
                fn ready() -> bool { true }
            }
            fn run() -> bool {
                const reflected = inspect(Choice);
                reflected and Choice.as(Ready).ready()
            }
            "
        ),
        Value::Bool(true)
    );
}

#[test]
fn body_required_newtypes_reach_runtime_conversion() {
    assert_eq!(
        sysroot::run(
            "
            comptime fn Wrapper() -> Type {
                meta.NewtypeBuilder.new(bool, .name = .LateGenerated).Build()
            }
            fn run() -> bool {
                const Flag = Wrapper();
                true.as(Flag).as(bool)
            }
        "
        ),
        Value::Bool(true)
    );
}

#[test]
fn body_required_enums_reach_runtime_construction_and_matching() {
    assert_eq!(
        sysroot::run(
            "
            comptime fn EventType() -> Type {
                let draft = meta.EnumBuilder.new(.name = .LateEvent);
                draft.add_unit(.idle);
                draft.add_tuple(.data, [bool]);
                draft.Build()
            }
            fn run() -> bool {
                const Event = EventType();
                let value: Event = .data(true);
                value match { .idle => false, .data(flag) => flag }
            }
        "
        ),
        Value::Bool(true)
    );
}

#[test]
fn body_required_enums_support_qualified_construction() {
    assert_eq!(
        sysroot::run(
            "
            comptime fn EventType() -> Type {
                let draft = meta.EnumBuilder.new(.name = .LateEvent);
                draft.add_unit(.idle);
                draft.add_tuple(.data, [bool]);
                draft.Build()
            }
            fn run() -> bool {
                const Event = EventType();
                let value = Event.data(true);
                value match { Event.idle => false, Event.data(flag) => flag }
            }
            "
        ),
        Value::Bool(true)
    );
}

#[test]
fn late_mixed_descriptions_preserve_prior_definitions_and_fresh_identities() {
    let package = sysroot::compile("
        struct Existing { flag: bool }
        enum Original { ready }
        newtype Baseline = bool;
        comptime fn Wrapper() -> Type {
            meta.NewtypeBuilder.new(bool, .name = .LateWrapper).Build()
        }
        typealias Early = Wrapper();
        comptime fn Record() -> Type {
            let draft = meta.StructBuilder.new(.name = .LateRecord);
            draft.add_field(.flag, Wrapper());
            draft.Build()
        }
        comptime fn EventType() -> Type {
            let draft = meta.EnumBuilder.new(.name = .LateChoice);
            draft.add_tuple(.some, [Record()]);
            draft.Build()
        }
        fn run() -> bool {
            const First = EventType();
            const Second = EventType();
            const distinct = First != Second;
            const inspect = First.enum_variant_field_types()[0][0].struct_field_types()[0].NewtypeRepresentation() == bool;
            distinct and inspect and true.as(Early).as(bool)
        }
    ").unwrap();
    let all = package
        .structs
        .iter()
        .map(|item| item.declaration)
        .chain(package.enums.iter().map(|item| item.declaration))
        .chain(package.newtypes.iter().map(|item| item.declaration))
        .collect::<Vec<_>>();
    let unique = all
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        all.len(),
        unique.len(),
        "description publication must not duplicate identities"
    );
    for name in ["Existing", "LateRecord"] {
        assert!(
            package.structs.iter().any(|item| item.name == name),
            "{name}"
        );
    }
    for name in ["Original", "LateChoice"] {
        assert!(package.enums.iter().any(|item| item.name == name), "{name}");
    }
    for name in ["Baseline", "LateWrapper"] {
        assert!(
            package.newtypes.iter().any(|item| item.name == name),
            "{name}"
        );
    }
    let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn required_default_publishes_the_enum_before_checking_the_next_input() {
    assert_eq!(
        sysroot::run(
            "comptime fn EventType() -> Type {
                 let draft = meta.EnumBuilder.new();
                 draft.add_unit(.idle);
                 draft.add_tuple(.data, [bool]);
                 draft.Build()
             }
             fn select(comptime .T: Type = EventType(), value: T) -> T { value }
             fn run() -> bool {
                 let event = select(.data(true));
                 event match { .idle => false, .data(flag) => flag }
             }"
        ),
        Value::Bool(true)
    );
}

#[test]
fn a_closed_default_can_publish_with_unrelated_clause_binders_in_scope() {
    let source = r#"
        comptime fn Wrapper(subject: Type) -> Type {
            meta.NewtypeBuilder.new(subject, .name = .ScopeWrapper).Build()
        }
        fn accepts() -> bool where T, .Flag: Type = Wrapper(bool), .answer: bool = true {
            const answer_value = answer;
            answer_value
        }
        fn forward(value: T) -> bool where T { accepts<T, .answer = true>() }
        fn run() -> bool { forward(true) and accepts<bool, .Flag = bool>() }
    "#;
    let package = sysroot::compile(source).unwrap();
    let wrappers: Vec<_> = package
        .newtypes
        .iter()
        .filter(|item| item.name == "ScopeWrapper")
        .collect();
    assert_eq!(wrappers.len(), 1);
    assert_eq!(
        wrappers[0].representation,
        nir::Term::Primitive(nir::Primitive::Bool)
    );
}

#[test]
fn a_frozen_runtime_signature_keeps_the_nominal_identity_generated_by_its_type_program() {
    let source = r#"
        comptime fn Built(flag: bool) -> Type {
            let draft = meta.StructBuilder.new(.name = .ProjectedRecord);
            if flag { draft.add_field(.flag, bool); }
            draft.Build()
        }
        trait Source where flag: bool { assoc Item: Type = Built(flag); }
        struct Holder where flag: bool {}
        impl Source<flag> for Holder<flag> where flag: bool {}
        fn empty() -> ?T.as(Source<flag>).Item
        where flag: bool, T :- Source<flag>
        { null }
        fn run() -> bool {
            empty<true, Holder<true>>() match { null => true, _ => false }
        }
    "#;
    let package = sysroot::compile(source).unwrap();
    let record = package
        .structs
        .iter()
        .find(|record| record.name == "ProjectedRecord")
        .unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
    assert!(graph.nominals.iter().any(|nominal| nominal.ty
        == nir::Term::Nominal {
            declaration: record.declaration,
            arguments: vec![]
        }));
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}
