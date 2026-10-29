//! Ordinary inference proposals are independent of builtin constructor roles.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

#[test]
fn user_normal_form_inputs_are_inferred_through_ordinary_calls() {
    let source = "
        typealias Type = .Type'builtin;
        typealias Integer = .Integer'builtin;
        pure comptime fn Shape<T, count: Integer> -> Type { T }
        fn identity(value: Shape<T, count>) -> Shape<T, count>
        where T, count: Integer
        { value }
        fn forward(value: Shape<bool, 3>) -> Shape<bool, 3> {
            identity(value)
        }
    ";
    support::compile_with_protocols(source, &Default::default()).unwrap();
    for rejected in [
        source.replace("-> Shape<bool, 3>", "-> bool"),
        source.replace("-> Shape<bool, 3>", "-> Shape<bool, 4>"),
    ] {
        assert!(support::compile_with_protocols(&rejected, &Default::default()).is_err());
    }
}
