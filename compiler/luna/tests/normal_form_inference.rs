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

#[test]
fn retained_effect_inputs_infer_through_callback_signatures() {
    let source = "typealias Type=.Type'builtin;
        pure comptime fn Shape<T> -> Type {T}
        effect read()->bool where key:Type;
        fn operation()->#[read<Shape<bool>>] bool {read<Shape<bool>>()#}
        fn apply(action:fn()->#[read<Shape<T>>] bool)->#[read<Shape<T>>] bool
        where T
        {action()}
        fn run()->bool {apply(operation)# {read<Shape<bool>>()=>true}}";
    assert_eq!(
        support::run_iir(source, "run", vec![]),
        comptime_interpretor::Value::Bool(true)
    );
    let bare = source.replace("read<Shape<bool>>()=>true", "read<bool>()=>true");
    assert!(support::compile_with_protocols(&bare, &Default::default()).is_err());
}
