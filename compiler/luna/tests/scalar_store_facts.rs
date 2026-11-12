//! Pointer stores use storage identity, not field spelling, to retain facts.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::Value;

#[test]
fn sibling_stores_preserve_nested_and_aliased_observations() {
    let source = r#"
        struct Pair { left: u8, right: u8 }
        struct Tree { left: Pair, right: Pair }
        fn direct(pair: *Pair, value: u8) -> u8 {
            assumes pair.left.* < 255;
            pair.right.* = value;
            pair.left.* + 1
        }
        fn aliased(pair: *Pair, value: u8) -> u8 {
            let alias = pair;
            assumes alias.left.* < 255;
            alias.right.* = value;
            alias.left.* + 1
        }
        fn field_alias(pair: *Pair, value: u8) -> u8 {
            let left = pair.left;
            assumes left.* < 255;
            pair.right.* = value;
            left.* + 1
        }
        fn difference(tree: *Tree, value: u8) -> u8 {
            assumes tree.left.left.* <= tree.left.right.*;
            tree.right.right.* = value;
            tree.left.right.* - tree.left.left.*
        }
        fn nested(tree: *Tree, value: u8, flag: bool) -> u8 {
            assumes tree.left.left.* < 255;
            if flag { tree.right.left.* = value; }
            else { tree.left.right.* = value; }
            tree.left.left.* + 1
        }
        fn run() -> bool {
            var pair = Pair { left: 41, right: 0 };
            var tree = Tree { left: Pair { left: 10, right: 0 }, right: Pair { left: 0, right: 0 } };
            direct(pair.ref, 255) == 42 and aliased(pair.ref, 254) == 42
                and field_alias(pair.ref, 253) == 42 and pair.right == 253 and nested(tree.ref, 254, true) == 11
                and nested(tree.ref, 255, false) == 11
                and tree.left.right == 255 and tree.right.left == 254
                and difference(tree.ref, 42) == 245 and tree.right.right == 42
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn sibling_spelling_does_not_authorize_stale_or_aliased_facts() {
    let declarations = r#"
        struct Pair { left: u8, right: u8 }
        struct Tree { left: Pair, right: Pair }
        struct Links { left: *u8, right: *u8 }
        fn set(pointer: *u8, value: u8) { pointer.* = value; }
    "#;
    for (name, body) in [
        (
            "same field",
            "fn bad(pair: *Pair, value: u8) -> u8 { assumes pair.left.* < 255; pair.left.* = value; pair.left.* + 1 }",
        ),
        (
            "different roots may alias",
            "fn bad(pair: *Pair, other: *Pair, value: u8) -> u8 { assumes pair.left.* < 255; other.left.* = value; pair.left.* + 1 }",
        ),
        (
            "whole object",
            "fn bad(pair: *Pair, value: u8) -> u8 { assumes pair.left.* < 255; pair.* = Pair { left: value, right: 0 }; pair.left.* + 1 }",
        ),
        (
            "ancestor field",
            "fn bad(tree: *Tree, value: u8) -> u8 { assumes tree.left.left.* < 255; tree.left.* = Pair { left: value, right: 0 }; tree.left.left.* + 1 }",
        ),
        (
            "RHS changes observed field",
            "fn bad(pair: *Pair, value: u8) -> u8 { assumes pair.left.* < 255; pair.right.* = do { pair.left.* = value; 0 }; pair.left.* + 1 }",
        ),
        (
            "RHS rebinds address",
            "fn bad(pair: *Pair, other: *Pair) -> u8 { var cursor = pair; assumes cursor.left.* < 255; cursor.right.* = do { cursor = other; 0 }; cursor.left.* + 1 }",
        ),
        (
            "pointer fields may alias",
            "fn bad(links: Links, value: u8) -> u8 { assumes links.left.* < 255; links.right.* = value; links.left.* + 1 }",
        ),
        (
            "loaded pointer fields may alias",
            "fn bad(links: *Links, value: u8) -> u8 { assumes links.left.*.* < 255; links.right.*.* = value; links.left.*.* + 1 }",
        ),
        (
            "retained sibling does not preserve overwritten field",
            "fn bad(pair: *Pair, value: u8) -> u8 { assumes pair.left.* < 255 and pair.right.* < 255; let alias = pair; alias.left.* = value; pair.left.* + 1 }",
        ),
        (
            "call writes observed field",
            "fn bad(pair: *Pair, value: u8) -> u8 { assumes pair.left.* < 255; set(pair.left, value); pair.left.* + 1 }",
        ),
        (
            "opaque call",
            "fn bad(pair: *Pair, action: fn(*Pair)) -> u8 { assumes pair.left.* < 255; action(pair); pair.left.* + 1 }",
        ),
    ] {
        let error = match sysroot::compile(&format!("{declarations}\n{body}")) {
            Err(error) => error,
            Ok(_) => panic!("accepted unsafe arithmetic after {name}"),
        };
        assert!(
            error.contains("machine arithmetic range is not proven"),
            "{name}: {error}"
        );
    }
}
