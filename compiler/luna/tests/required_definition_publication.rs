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

#[test]
fn first_use_in_runtime_specializations_delivers_distinct_generated_layouts() {
    use iir::NominalDefinition as D;
    use nir::{Primitive, Term};
    let source = r#"
        comptime fn Built(flag: bool) -> Type {
            let payload = if flag { bool } else { char };
            let scalar = meta.NewtypeBuilder.new(payload, .name = .LateScalar).Build();
            let row = meta.StructBuilder.new(.name = .LateRow);
            row.add_field(.value, scalar);
            let choice = meta.EnumBuilder.new(.name = .LateChoice);
            choice.add_tuple(.some, [row.Build()]);
            choice.Build()
        }
        trait Source where flag: bool { assoc Item: Type = Built(flag); }
        struct Holder where flag: bool {}
        impl Source<flag> for Holder<flag> where flag: bool {}
        fn leaf() -> ?Holder<flag>.as(Source<flag>).Item
        where flag: bool
        { null }
        fn relay() -> bool
        where flag: bool
        {
            let callback = leaf<flag>;
            callback() match { null => true, _ => false }
        }
        fn first() -> bool { relay<true>() }
        fn second() -> bool { relay<false>() }
        fn run() -> bool { first() and second() }
    "#;
    let package = sysroot::compile(source).unwrap();
    assert!(package.observed_associated_types.is_empty());
    assert!(package.enums.iter().all(|e| e.name != "LateChoice"));
    let graph = luna::instantiation::instantiate(&package, "run", 10_000_000).unwrap();
    let nominal = |ty: &Term| {
        &graph
            .nominals
            .iter()
            .find(|n| &n.ty == ty)
            .unwrap()
            .definition
    };
    let mut ids = std::collections::BTreeSet::new();
    for (flag, primitive) in [(true, Primitive::Bool), (false, Primitive::Char)] {
        let leaf = graph
            .functions
            .iter()
            .find(|f| f.function.name == "leaf" && f.key.arguments.as_ref() == [Term::Bool(flag)])
            .unwrap();
        let Term::Function { result, .. } = &leaf.function.signature else {
            panic!("expected callable");
        };
        let Term::Optional(choice) = result.as_ref() else {
            panic!("expected optional");
        };
        let D::Enum(e) = nominal(choice) else {
            panic!("expected choice");
        };
        let D::Struct(row) = nominal(&e.variants[0].fields[0]) else {
            panic!("expected record");
        };
        let D::Newtype(scalar) = nominal(&row.fields[0].1) else {
            panic!("expected newtype");
        };
        assert_eq!(
            (&*e.name, &*row.name, &*scalar.name),
            ("LateChoice", "LateRow", "LateScalar")
        );
        assert_eq!(row.fields[0].0, "value");
        assert_eq!(scalar.representation, Term::Primitive(primitive));
        for id in [e.declaration, row.declaration, scalar.declaration] {
            assert!(id.local >= package.definition_identity_cursor && ids.insert(id));
        }
    }
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert!(package.observed_associated_types.is_empty());
    assert!(package.enums.iter().all(|e| e.name != "LateChoice"));

    // An incremental client extends one declaration world across root requests.
    // Keep the original result alive while a detached source publishes the next.
    use elaboration::materialization::{
        DeclarationSnapshot, DeclarationSource, InstantiationSession, extend_with_types_in_session,
    };
    let entry = |name| iir::InstanceKey {
        declaration: package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration,
        arguments: vec![].into(),
    };
    let roots = [entry("first"), entry("second")];
    let source = DeclarationSnapshot::capture_member_programs(package, &mut 10_000_000).unwrap();
    let mut session = InstantiationSession::new(DeclarationSource::Snapshot(&source));
    let budget = || elaboration::ptc::Budget {
        steps: 10_000_000,
        ..Default::default()
    };
    let first = extend_with_types_in_session(
        &mut session,
        &roots[..1],
        &[],
        Default::default(),
        &mut budget(),
    )
    .unwrap();
    let extended =
        extend_with_types_in_session(&mut session, &roots, &[], Default::default(), &mut budget())
            .unwrap();
    let generated = |graph: &iir::Package| {
        graph
            .nominals
            .iter()
            .filter_map(|n| match &n.definition {
                D::Enum(def) if def.name == "LateChoice" => Some(def.declaration),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>()
    };
    let old = generated(&first);
    let new = generated(&extended);
    assert_eq!(old.len(), 1);
    assert_eq!(new.len(), 2);
    assert!(old.is_subset(&new));
    for item in &first.nominals {
        assert!(
            extended.nominals.contains(item),
            "changed an existing layout: {item}"
        );
    }
    let mut vm =
        comptime_interpretor::Interpreter::from_iir(&extended, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&roots[1], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert!(
        source
            .package()
            .enums
            .iter()
            .all(|e| e.name != "LateChoice")
    );
}

#[test]
fn staged_operands_publish_generated_layouts_with_their_function_instances() {
    let src = r#"
        comptime fn Built(flag: bool) -> Type {
            let row = meta.StructBuilder.new(.name = .Operand);
            if flag {
                row.add_field(.flag, bool);
                row.Build()
            } else {
                meta.NewtypeBuilder.new(bool, .name = .Operand).Build()
            }
        }
        fn inspect(comptime ty: Type, comptime is_record: bool) -> bool {
            is_record
        }
        fn relay(comptime flag: bool) -> bool { inspect(Built(flag), flag) }
        fn run() -> bool { relay(true) and not relay(false) }
    "#;
    let package = sysroot::compile(src).unwrap();
    assert!(package.structs.iter().all(|s| s.name != "Operand"));
    let graph = luna::instantiation::instantiate(&package, "run", 10_000_000).unwrap();
    let mut shapes = std::collections::BTreeSet::new();
    for instance in graph
        .functions
        .iter()
        .filter(|f| f.function.name == "inspect")
    {
        let [ty, nir::Term::Bool(is_record)] = instance.key.arguments.as_ref() else {
            panic!("unexpected staged key: {}", instance.key)
        };
        let nominal = graph.nominals.iter().find(|n| &n.ty == ty).unwrap();
        let id = match &nominal.definition {
            iir::NominalDefinition::Struct(row) => {
                assert!(*is_record);
                assert_eq!(
                    row.fields,
                    [("flag".into(), nir::Term::Primitive(nir::Primitive::Bool))]
                );
                row.declaration
            }
            iir::NominalDefinition::Newtype(wrapper) => {
                assert!(!is_record);
                assert_eq!(
                    wrapper.representation,
                    nir::Term::Primitive(nir::Primitive::Bool)
                );
                wrapper.declaration
            }
            other => panic!("unexpected nominal: {other}"),
        };
        assert_eq!(id.package, instance.key.declaration.package);
        assert!(shapes.insert((id, *is_record)));
    }
    assert_eq!(shapes.len(), 2);
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn computed_builder_input_supplies_a_dependent_optional_result() {
    let package = sysroot::compile(
        r#"
        comptime fn Payload(flag: bool) -> Type {
            let row = meta.StructBuilder.new(.name = .BranchPayload);
            if flag { row.add_field(.content, bool); }
            else { row.add_field(.content, char); }
            row.Build()
        }
        fn empty(comptime ty: Type) -> ?ty { null }
        fn inspect() -> bool where flag: bool {
            empty(Payload(flag)) match { null => true, _ => false }
        }
        fn run() -> bool { inspect<true>() and inspect<false>() }
    "#,
    )
    .unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 10_000_000).unwrap();
    let mut payloads = Vec::new();
    for instance in graph
        .functions
        .iter()
        .filter(|f| f.function.name == "empty")
    {
        let [ty] = instance.key.arguments.as_ref() else {
            panic!("unexpected key {}", instance.key)
        };
        let nir::Term::Function { result, .. } = instance.function.signature.callable_signature()
        else {
            panic!("expected function signature")
        };
        assert_eq!(result.as_ref(), &nir::Term::Optional(Box::new(ty.clone())));
        let def = graph.nominals.iter().find(|def| &def.ty == ty).unwrap();
        let iir::NominalDefinition::Struct(record) = &def.definition else {
            panic!("expected generated struct")
        };
        assert_eq!(record.name, "BranchPayload");
        let [(field, content)] = record.fields.as_slice() else {
            panic!("expected one field")
        };
        assert_eq!(field, "content");
        payloads.push(content.clone());
    }
    assert_eq!(payloads.len(), 2);
    assert!(payloads.contains(&nir::Term::Primitive(nir::Primitive::Bool)));
    assert!(payloads.contains(&nir::Term::Primitive(nir::Primitive::Char)));
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}
