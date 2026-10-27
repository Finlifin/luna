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
