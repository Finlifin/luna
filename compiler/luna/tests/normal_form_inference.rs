//! Ordinary inference proposals are independent of builtin constructor roles.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

#[test]
fn type_fields_in_ordinary_description_values_infer_without_erasing_other_fields() {
    let source = "typealias Type=.Type'builtin;
        struct Description {enabled:bool,kind:Type}
        pure comptime fn Marker<description:Description>->Type {while true {} bool}
        pure comptime fn Other<description:Description>->Type {while true {} bool}
        struct Holder where T {flag:bool}
        fn recover(value:Holder<Marker<Description {enabled:true,kind:T}>>)->Holder<T>
        where T
        {Holder<T>{flag:value.flag}}
        fn run()->bool {
            let result=recover(Holder<Marker<Description {enabled:true,kind:bool}>>{flag:true});
            result.flag
        }";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    for rejected in [
        source.replace("enabled:true,kind:bool", "enabled:false,kind:bool"),
        source.replace(
            "Holder<Marker<Description {enabled:true,kind:bool}",
            "Holder<Other<Description {enabled:true,kind:bool}",
        ),
    ] {
        assert!(support::compile_with_protocols(&rejected, &Default::default()).is_err());
    }
}

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

#[test]
fn collected_normal_form_values_keep_the_declared_list_domain() {
    for (arguments, whole) in [
        ("", ".items=[]"),
        ("true,false", ".items=[true,false]"),
        ("true,...[false,true]", ".items=[true,false,true]"),
        (".T=.u8'builtin", ".T=.u8'builtin,.items=[]"),
        ("7,9,.T=.u8'builtin", ".T=.u8'builtin,.items=[7,9]"),
    ] {
        let source = format!(
            "typealias Type=.Type'builtin;
            pure comptime fn List<T>->Type {{T}}
            pure comptime fn Indexed<.T:Type=bool,...items:List<T>>->Type {{while true {{}} bool}}
            struct Holder where T {{}}
            fn holder()->Holder<Indexed<{arguments}>> {{Holder<Indexed<{whole}>>{{}}}}
            comptime fn run()->bool {{
                let collected=Indexed<{arguments}>;
                let supplied=Indexed<{whole}>;
                collected == supplied
            }}"
        );
        let package = support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                meta_list: Some("input.List"),
                ..Default::default()
            },
        )
        .unwrap_or_else(|failure| panic!("{arguments}: {failure}"));
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            comptime_interpretor::Value::Bool(true),
            "{arguments}"
        );
    }
}

#[test]
fn contextual_collection_spreads_execute_once_before_later_type_inputs() {
    let source = "typealias Type=.Type'builtin;
        typealias Integer=.Integer'builtin;
        pure comptime fn List<T>->Type {T}
        pure comptime fn Indexed<.T:Type=bool,...items:List<T>>->Type {while true {} bool}
        comptime fn run()->bool {
            var trace:Integer=0;
            let actual=Indexed<
                do {trace = trace * 10 + 1;true},
                ...[do {trace = trace * 10 + 2;false}],
                .T=do {trace = trace * 10 + 3;bool}>;
            trace == 123 and actual == Indexed<.items=[true,false]>
        }";
    let package = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}
