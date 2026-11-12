//! Runtime collection preparation preserves comptime inputs and source order.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

#[test]
fn computed_tuple_domains_keep_variadic_arity_checks() {
    // A concrete-domain control for the descriptor delivery still needed by
    // Parse. This is deliberately not the public formatting implementation:
    // its elements are bool, whereas each {} accepts its own Display type.
    let definitions = r#"
        pure comptime fn BooleanFields(count: Integer) -> Type {
            var fields: meta.List<Type> = [];
            var index: Integer = 0;
            while index < count {
                fields = fields.concat([bool]);
                index = index + 1;
            }
            Tuple<.Es = fields>
        }
        fn accept(comptime count: Integer, ...values: defer BooleanFields(count)) -> bool {
            true
        }
    "#;
    assert_eq!(
        support::run(&format!(
            "{definitions} fn run() -> bool {{ accept(0) and accept(2, true, false) }}"
        )),
        Value::Bool(true)
    );
    for arguments in ["2, true", "1, true, false", "1, 7"] {
        assert!(
            support::compile(&format!(
                "{definitions} fn run() -> bool {{ accept({arguments}) }}"
            ))
            .is_err(),
            "invalid collection was accepted: {arguments}"
        );
    }
}

#[test]
fn staged_inputs_do_not_escape_collection_calls_into_runtime_storage() {
    assert_eq!(
        support::run(
            r#"
            fn visit(state: *u8, before: u8, after: u8) -> bool {
                let correct = state.* == before;
                assumes correct;
                state.* = after;
                true
            }
            fn collect(comptime count: Integer, first: bool,
                ...values: Tuple<bool, bool>, .last: bool = false) -> bool {
                first and last
            }
            fn staged(comptime ...types: Tuple<Type, Type>, .result: bool = false) -> bool {
                result
            }
            fn run() -> bool {
                var state: u8 = 0;
                let collected = collect(2, visit(state.ref, 0, 1),
                    visit(state.ref, 1, 2), .last = visit(state.ref, 2, 3),
                    visit(state.ref, 3, 4));
                collected and state == 4 and staged(bool, u32, .result = true)
            }
            "#,
        ),
        Value::Bool(true)
    );
}
