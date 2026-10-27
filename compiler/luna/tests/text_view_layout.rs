//! Source-level layouts for future text-region contracts, without lifetime grants.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use elaboration::verification::memory::{self, BorrowedInputLeaf};

#[test]
fn nominal_wrappers_and_tuple_heads_keep_conditional_text_slots() {
    let package = support::compile_with_tuple(
        "typealias str=.str'builtin;
        newtype Wrapped = str;
        struct Packet {text:Wrapped, optional:?str}
        enum Choice {Empty, Item(Packet)}
        fn inspect(value:Tuple<bool,Choice>) {}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "inspect")
        .unwrap();
    let nir::Term::Function { parameters, .. } = &function.signature else {
        panic!("callable")
    };
    assert!(matches!(parameters[0].ty, nir::Term::NormalForm { .. }));
    let layout =
        memory::borrowed_input_layout(Some(&package), &parameters[0].ty, &mut 100_000).unwrap();
    assert_eq!(layout.len(), 2);
    let BorrowedInputLeaf::Text(first) = &layout[0] else {
        panic!("text leaf")
    };
    let BorrowedInputLeaf::Text(second) = &layout[1] else {
        panic!("text leaf")
    };
    assert_eq!(first.storage_fields, [1, 0, 0, 0]);
    assert_eq!(first.guards, [(vec![1], 1)]);
    assert_eq!(second.storage_fields, [1, 0, 1, 0]);
    assert_eq!(second.guards, [(vec![1], 1), (vec![1, 0, 1], 1)]);
    assert_eq!(
        first.to_string(),
        "(text-input (input-fields 1 (variant 1) 0 0 0) (storage-fields 1 0 0 0) (guards (guard (path 1) (tag 1))))"
    );
    // Learning where a view lives does not establish where its bytes live.
    let error =
        memory::pointer_input_layout(Some(&package), &parameters[0].ty, &mut 100_000).unwrap_err();
    assert!(
        error.contains("backing character storage") && error.contains("text-input"),
        "{error}"
    );
    assert!(memory::borrowed_input_layout(Some(&package), &parameters[0].ty, &mut 1).is_err());
    assert_eq!(
        memory::borrowed_input_layout(Some(&package), &parameters[0].ty, &mut 100_000).unwrap(),
        layout
    );
}

#[test]
fn text_descriptor_storage_operations_form_without_granting_resource_safety() {
    for copy_impl in ["", "impl Duplicate for str {}"] {
        let package = support::compile_with_tuple(&format!(
            "typealias str=.str'builtin;
            trait Duplicate {{}}
            trait Retire {{fn drop(*self);}}
            {copy_impl}
            struct Box {{text:str}}
            fn read(pointer:*str)->str {{pointer.*}}
            fn field(pointer:*Box)->str {{pointer.*.text}}
            fn aggregate(pointer:*Box)->Box {{pointer.*}}
            fn tuple(pointer:*Tuple<bool,str>)->Tuple<bool,str> {{pointer.*}}
            fn replace(pointer:*str,value:str) {{pointer.* = value;}}
            fn direct()->bool {{let text=\"aé\";read(text.ref) == \"aé\"}}
            fn projected()->bool {{let box=Box {{text:\"stored\"}};field(box.ref) == \"stored\"}}
            fn copied()->bool {{let box=Box {{text:\"record\"}};aggregate(box.ref).text == \"record\"}}
            fn tupled()->bool {{let value=(true,\"tuple\");let (flag,text)=tuple(value.ref);flag and text == \"tuple\"}}
            fn written()->bool {{var text=\"old\";replace(text.ref,\"new\");text == \"new\"}}"
        ));
        for name in ["read", "field", "aggregate", "tuple"] {
            let function = package.functions.iter().find(|f| f.name == name).unwrap();
            let mut reads = 0;
            nir::visit::expressions(function, &mut 100_000, |expr, _| {
                if matches!(expr.kind, nir::program::ExprKind::ReadPointer(_)) {
                    reads += 1;
                }
                Ok::<_, String>(())
            })
            .unwrap();
            assert_eq!(reads, 1, "{name}");
        }
        for name in ["direct", "projected", "copied", "tupled", "written"] {
            let error = luna::execution::interpret(&package, name, 1_000_000).unwrap_err();
            assert!(error.contains("Unsupported"), "{name}: {error}");
            let declaration = package
                .functions
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .declaration;
            // This isolated representation test supplies permission explicitly.
            // Neither ordinary execution nor the resource verifier inherits it.
            let mut vm = comptime_interpretor::Interpreter::new(
                &package,
                comptime_interpretor::Budget {
                    steps: 1_000_000,
                    depth: 128,
                },
            )
            .unwrap()
            .with_pointer_copy_permission(&|_| true);
            assert_eq!(
                vm.invoke(declaration, vec![]).unwrap(),
                comptime_interpretor::Value::Bool(true),
                "{name}"
            );
            let error = luna::execution::interpret_with_protocols(
                &package,
                name,
                "Duplicate",
                "Retire",
                1_000_000,
            )
            .unwrap_err();
            assert!(
                error.contains("string-view memory provenance") || error.contains("Copy"),
                "{name}: {error}"
            );
        }
    }
}
