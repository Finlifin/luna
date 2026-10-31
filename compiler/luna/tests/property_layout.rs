//! Property layout follows the target domain; producer evaluation follows source.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};

const HEADER: &str = "comptime fn schema(...fields:T)->T where T {fields} const MODEL=schema {left:true,right:false};typealias Config=MODEL'type;";

#[test]
fn reordered_properties_preserve_producer_order_at_runtime_and_comptime() {
    for phase in ["", "comptime "] {
        let source = format!("{HEADER}
            {phase}fn first(p:*bool)->bool {{let old=p.*;p.* = not old;not old}}
            {phase}fn second(p:*bool)->bool {{p.*}}
            {phase}fn marker()->bool {{true}}
            {phase}fn inspect(head:bool,...fields:Config,tail:bool)->bool {{head and tail and fields.left and fields.right}}
            {phase}fn run()->bool {{let state=false;inspect {{marker(),right:first(state.ref),marker(),left:second(state.ref)}}}}");
        let package = support::compile(&source);
        let mut vm = if phase.is_empty() {
            Interpreter::new(&package, Budget::default())
        } else {
            Interpreter::for_comptime(&package, Budget::default())
        }
        .unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            support::calls(&package, &vm),
            vec!["run", "marker", "first", "marker", "second", "inspect"]
        );
        if phase.is_empty() {
            assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
        }
    }
}

#[test]
fn reordered_properties_retain_field_type_contexts() {
    let package = support::compile(
        "typealias Byte=.u8'builtin;comptime fn schema(...fields:T)->T where T {fields} fn identity(value:bool)->bool {value}const MODEL=schema {number:do {let value:Byte=0;value},callback:identity};typealias Config=MODEL'type;fn inspect(...fields:Config)->bool {fields.number==7 and fields.callback(false)}fn run()->bool {inspect {callback:|value| not value,number:7}}",
    );
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn reordered_properties_preserve_early_exit() {
    let package = support::compile(&format!(
        "{HEADER} fn later()->bool {{false}} fn inspect(...fields:Config)->bool {{false}}fn run()->bool {{inspect {{right:do {{return true;}},left:later()}}}}"
    ));
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(support::calls(&package, &vm), vec!["run"]);
}

#[test]
fn property_layout_matching_rejects_missing_extra_and_duplicate_fields() {
    for fields in [
        "left:true",
        "right:true,left:true,extra:true",
        "right:true,right:false",
    ] {
        let source = format!(
            "{HEADER} fn inspect(...fields:Config)->bool {{true}} fn run()->bool {{inspect {{{fields}}}}}"
        );
        let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        assert!(
            error.contains("property") || error.contains("field"),
            "{error}"
        );
    }
}

#[test]
fn reordered_properties_merge_with_variadic_and_spread_preparation() {
    for (arguments, order) in [
        (
            "marker(),right:first(state.ref),marker(),left:second(state.ref)",
            vec!["run", "marker", "first", "marker", "second", "inspect"],
        ),
        (
            "...(marker(),marker()),right:first(state.ref),left:second(state.ref)",
            vec!["run", "marker", "marker", "first", "second", "inspect"],
        ),
    ] {
        let source = format!("{HEADER}
            fn first(p:*bool)->bool {{let old=p.*;p.* = not old;not old}}
            fn second(p:*bool)->bool {{p.*}}
            fn marker()->bool {{true}}
            fn inspect(...items:Tuple<bool,bool>,...fields:Config)->bool {{let (head,tail)=items;head and tail and fields.left and fields.right}}
            fn run()->bool {{let state=false;inspect {{{arguments}}}}}");
        let package = support::compile_with_tuple(&source);
        let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::calls(&package, &vm), order);
    }
}

#[test]
fn reordered_property_peers_infer_callback_domains() {
    let source="fn packet(...fields:T)->T where T {fields}
        fn exemplar(value:T)->U where T,U {exemplar<T,U>(value)}
        fn sample()->T where T {sample<T>()}
        fn seed()->bool {false}
        fn apply(...fields:(packet {value:sample<T>(),callback:exemplar<T,U>})'type)->U where T,U {fields.callback(fields.value)}
        fn run()->bool {apply {callback:|value|not value,value:seed()}}";
    for staged in [false, true] {
        let source = if staged {
            source.replace("fn ", "comptime fn ")
        } else {
            source.to_owned()
        };
        let package = support::compile(&source);
        let result = if staged {
            luna::invocation::invoke(&package, "run", 100_000).unwrap()
        } else {
            luna::execution::interpret(&package, "run", 100_000).unwrap()
        };
        assert_eq!(result, Value::Bool(true));
    }
}

#[test]
fn whole_record_supply_does_not_gain_an_implicit_layout_conversion() {
    let source = format!(
        "{HEADER} fn packet(...fields:T)->T where T {{fields}}fn inspect(...fields:Config)->bool {{true}}fn run()->bool {{let existing=packet {{right:true,left:true}};inspect(.fields=existing)}}"
    );
    let error = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
    assert!(
        error.contains("mismatch") || error.contains("type equality"),
        "{error}"
    );
}

#[test]
fn reordered_properties_forward_open_and_deferred_inputs() {
    let source = "fn packet(...fields:T)->T where T {fields}
        fn exemplar(value:T)->U where T,U {exemplar<T,U>(value)}
        fn sample()->T where T {sample<T>()}
        fn seed()->bool {false}
        fn apply(comptime flag:bool,...fields:(packet {value:sample<T>(),callback:exemplar<T,U>})'type)->U where T,U {fields.callback(fields.value)}
        fn forward(comptime flag:bool)->bool {apply {flag,callback:|value|not value,value:seed()}}
        fn run()->bool {forward(true)}";
    let deferred = format!(
        "typealias Type=.Type'builtin; comptime fn Select(flag:bool,T:Type)->Type {{if flag {{T}} else {{Select(flag,T)}}}} {}",
        source.replace(
            "->U where T,U {fields.callback",
            "->defer Select(flag,U) where T,U {fields.callback"
        )
    );
    for source in [source, &deferred] {
        let package = support::compile(source);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
    }
}

#[test]
fn enclosing_domain_can_contextualize_a_nested_packet_constructor() {
    let source = format!(
        "{HEADER} fn packet(...fields:T)->T where T {{fields}}fn inspect(...fields:Config)->bool {{fields.left and not fields.right}}fn run()->bool {{inspect(.fields=packet {{right:false,left:true}})}}"
    );
    assert_eq!(support::run_iir(&source, "run", vec![]), Value::Bool(true));
}
