#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

#[test]
fn immediate_callable_results_retain_partial_field_evidence() {
    use comptime_interpretor::{Event, Interpreter, Value};
    let package = support::compile(
        "typealias Type = .Type'builtin;
        typealias Byte = .u8'builtin;
        struct Fields { hidden: Type, selected: Type }
        comptime fn Other() -> Type { Byte }
        comptime fn run() -> bool {
            let fields = (|first: Type, second: Type| Fields {hidden: first, selected: second})(Other(), bool);
            let Selected = fields.selected;
            let value: Selected = true;
            value
        }",
    );
    for bytecode in [false, true] {
        let mut vm = if bytecode {
            Interpreter::for_comptime(&package, Default::default())
        } else {
            Interpreter::for_comptime_tree(&package, Default::default())
        }
        .unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
        let other = support::id(&package, "Other");
        assert_eq!(
            vm.events()
                .iter()
                .filter(|event| matches!(event, Event::EnterFunction(id) if *id == other))
                .count(),
            1
        );
    }
}

#[test]
fn observed_callable_fields_do_not_skip_nonterminating_arguments() {
    use comptime_interpretor::{Budget, ErrorKind, Interpreter};
    let package = support::compile(
        "typealias Type = .Type'builtin;
        struct Fields { hidden: Type, selected: Type }
        comptime fn Pending() -> Type { while true {} bool }
        comptime fn run() -> bool {
            let fields = (|first: Type, second: Type| Fields {hidden: first, selected: second})(Pending(), bool);
            let Selected = fields.selected;
            let value: Selected = true;
            value
        }",
    );
    for bytecode in [false, true] {
        let budget = Budget {
            steps: 10_000,
            ..Default::default()
        };
        let mut vm = if bytecode {
            Interpreter::for_comptime(&package, budget)
        } else {
            Interpreter::for_comptime_tree(&package, budget)
        }
        .unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![])
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
    }
}

#[test]
fn associated_field_input_and_result_domains_survive_generic_signatures() {
    let source = "typealias Type = .Type'builtin;
trait FieldType { assoc Item: Type; }
impl FieldType for bool { assoc Item: Type = bool; }
struct Holder where T { item: T }
comptime fn run() -> bool { read_holder(bool, Holder<bool> {item: true}) }
const checked = run();
comptime fn read_holder(comptime T: Type, holder: Holder<T.as(FieldType).Item>) -> T.as(FieldType).Item
where requires T :- FieldType
{ holder.item }";
    let package = support::compile_with_protocols(source, &Default::default())
        .unwrap_or_else(|error| panic!("{error}\n{source}"));
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn lazy_contracts_keep_type_requirements_separate_from_program_preconditions() {
    for flag in [true, false] {
        let source = format!(
            "typealias Type = .Type'builtin;
comptime fn run() -> bool {{ take(bool, {flag}, true) }}
const checked = run();
comptime fn take(comptime T: Type, comptime flag: bool, item: T) -> T
where requires T == bool, requires flag
{{ item }}"
        );
        let result = support::compile_with_protocols(&source, &Default::default());
        if flag {
            let package = result.unwrap();
            assert_eq!(
                luna::invocation::invoke(&package, "run", 100_000).unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
        } else {
            let error = result.unwrap_err();
            assert!(
                error.contains("precondition") || error.contains("contract"),
                "{error}"
            );
        }
    }
}

#[test]
fn restoring_lazy_type_inputs_does_not_assume_an_unimplemented_trait() {
    let source = "typealias Type = .Type'builtin;
trait Required {}
comptime fn run() -> bool { take(bool, true) }
const checked = run();
comptime fn take(comptime T: Type, value: T) -> T
where requires T :- Required
{ value }";
    let error = support::compile_with_protocols(source, &Default::default()).unwrap_err();
    assert!(
        error.contains("trait") || error.contains("requirement"),
        "{error}"
    );
}
