//! Sequential pattern computations share invocation's linear control flow.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};

#[test]
fn chained_patterns_and_let_bindings_use_compiled_calls() {
    let package = support::compile(
        "comptime fn flip(value:bool)->bool {not value}
        comptime fn run(flag:bool)->bool {
            flag match {x and flip(x) is y and flip(y) is z => z}
        }
        comptime fn bound(flag:bool)->bool {
            let x and flip(x) is y=flag;
            y
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for flag in [true, false, true] {
        for (name, expected, calls) in [("run", flag, 3), ("bound", !flag, 2)] {
            let before = vm.bytecode_invocations();
            assert_eq!(
                vm.invoke(support::id(&package, name), vec![Value::Bool(flag)])
                    .unwrap(),
                Value::Bool(expected)
            );
            assert_eq!(vm.bytecode_invocations() - before, calls, "{name}");
        }
    }
}

#[test]
fn failed_candidates_keep_effects_but_discard_pattern_bindings() {
    let package = support::compile(
        "comptime fn flip(value:bool)->bool {not value}
        comptime fn run(flag:bool)->bool {
            var trace=false;
            flag match {
                false and do {trace=true;false} is true => false,
                x and flip(x) is y if y => trace,
                _ => not trace,
            }
        }
        comptime fn early()->bool {
            false match {_ and do {return true;} is _ => false}
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for flag in [false, true, false] {
        let id = support::id(&package, "run");
        let before = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(id, vec![Value::Bool(flag)]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            tree.invoke(id, vec![Value::Bool(flag)]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(vm.bytecode_invocations() - before, 2);
    }
    let before = vm.bytecode_invocations();
    assert_eq!(
        vm.invoke(support::id(&package, "early"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(vm.bytecode_invocations() - before, 1);
}

#[test]
fn contextual_normal_inputs_resume_pattern_calls_before_domain_conversion() {
    use nir::{Primitive, Term};
    let package = support::compile(
        "typealias Type=.Type'builtin;
        pure comptime fn Payload<T,value:T>->Type {T}
        comptime fn flip(value:bool)->bool {not value}
        comptime fn Matched(input:Type,flag:bool)->Type {
            Payload<input,flag match {x and flip(x) is y if y=>17,_=>23}>
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for primitive in [Primitive::U8, Primitive::Integer] {
        for flag in [true, false] {
            let before = vm.bytecode_invocations();
            assert_eq!(
                vm.invoke(
                    support::id(&package, "Matched"),
                    vec![Value::Type(Term::Primitive(primitive)), Value::Bool(flag)]
                )
                .unwrap(),
                Value::Type(Term::NormalForm {
                    head: support::id(&package, "Payload"),
                    sort: nir::Sort::Type,
                    arguments: vec![
                        Term::Primitive(primitive),
                        Term::Integer(comptime_runtime::Integer::from_i128(if flag {
                            23
                        } else {
                            17
                        }))
                    ],
                })
            );
            assert_eq!(vm.bytecode_invocations() - before, 2);
        }
    }
}

#[test]
fn or_candidates_resume_calls_and_remap_only_the_selected_bindings() {
    let package = support::compile(
        "comptime fn flip(value:bool)->bool {not value}
        comptime fn run(flag:bool)->bool {
            var first=false;
            var second=false;
            flag match {
                ((x and do {first=true;flip(x)} is (y if y))
                    or (x and do {second=true;x} is y))
                    => y and first and (second==flag),
            }
        }
        comptime fn bound(flag:bool)->bool {
            let ((x and flip(x) is (y if y)) or (x and x is y))=flag;
            y
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for flag in [true, false, true, false] {
        for name in ["run", "bound"] {
            let id = support::id(&package, name);
            let before = vm.bytecode_invocations();
            assert_eq!(
                vm.invoke(id, vec![Value::Bool(flag)]).unwrap(),
                Value::Bool(true)
            );
            assert_eq!(
                tree.invoke(id, vec![Value::Bool(flag)]).unwrap(),
                Value::Bool(true)
            );
            assert_eq!(vm.bytecode_invocations() - before, 2, "{name}");
        }
    }
}

#[test]
fn failed_outer_guard_does_not_retry_a_successful_or_candidate() {
    let package = support::compile(
        "comptime fn run()->bool {
            var second=false;
            let result=true match {
                ((_ and false is y) or (_ and do {second=true;true} is y)) if y => true,
                _ => false,
            };
            not result and not second
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(vm.bytecode_invocations(), 1);
}
