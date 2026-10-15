#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};
use elaboration::arithmetic::Operator;
use support::{calls, compile_with_source_bindings};

#[test]
fn inferred_left_operand_dispatches_bound_rounding_protocol() {
    for (operator, symbol) in [(Operator::Divide, "/"), (Operator::Remainder, "%")] {
        for receiver in [
            "7",
            "do {7}",
            "if true {7} else {8}",
            "true match {true => 7, false => 8}",
        ] {
            let method = operator.method();
            let source = format!("typealias Int=.i32'builtin;
            trait Operation where Rhs {{assoc Output:.Type'builtin;fn {method}(self,rhs:Rhs)->Output;}}
            impl Operation<Int> for Int {{assoc Output:.Type'builtin=bool;
                fn {method}(self,rhs:Int)->bool {{self == 7 and rhs == 3}}}}
            fn right()->Int {{3}}
            fn run()->bool {{({receiver}) {symbol} right()}}");
            let mut selectors = luna::protocols::type_constructors::Selectors::default();
            selectors.arithmetic[operator as usize] = Some("input::Operation");
            let package =
                compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
            let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
            let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
                Value::Bool(true)
            );
            assert_eq!(calls(&package, &vm), vec!["run", "right", method]);
            let staged = compile_with_source_bindings(
                &source.replace("fn ", "comptime fn "),
                &Default::default(),
                selectors,
            )
            .unwrap();
            assert_eq!(
                luna::invocation::invoke(&staged, "run", 100_000).unwrap(),
                Value::Bool(true)
            );
            assert!(support::compile_with_protocols(&source, &Default::default()).is_err());
        }
    }
}

#[test]
fn inferred_protocol_operands_keep_source_order_and_early_exit() {
    let source = "typealias Int=.i32'builtin;
        trait Operation where Rhs {assoc Output:.Type'builtin;fn div(self,rhs:Rhs)->Output;}
        impl Operation<Int> for Int {assoc Output:.Type'builtin=bool;
            fn div(self,rhs:Int)->bool {self == 7 and rhs == 3}}
        fn mark()->bool {true}
        fn right()->Int {3}
        fn run()->bool {(do {mark();7}) / right()}
        fn early()->bool {(do {mark();if true {return true;}7}) / right()}";
    let mut selectors = luna::protocols::type_constructors::Selectors::default();
    selectors.arithmetic[Operator::Divide as usize] = Some("input::Operation");
    let package = compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    for (entry, trace) in [
        ("run", vec!["run", "mark", "right", "div"]),
        ("early", vec!["early", "mark"]),
    ] {
        let graph = luna::instantiation::instantiate(&package, entry, 100_000).unwrap();
        let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(calls(&package, &vm), trace);
    }
}

#[test]
fn inferred_float_remainder_uses_protocol_instead_of_float_division() {
    for scalar in ["f32", "f64"] {
        let source = format!(
            "typealias Float=.{scalar}'builtin;
            trait Operation where Rhs {{assoc Output:.Type'builtin;fn rem(self,rhs:Rhs)->Output;}}
            impl Operation<Float> for Float {{assoc Output:.Type'builtin=bool;
                fn rem(self,rhs:Float)->bool {{self == 7.0 and rhs == 3.0}}}}
            fn right()->Float {{3.0}}
            fn run()->bool {{7.0 % right()}}"
        );
        let mut selectors = luna::protocols::type_constructors::Selectors::default();
        selectors.arithmetic[Operator::Remainder as usize] = Some("input::Operation");
        let package =
            compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn inferred_native_unsigned_operations_keep_builtin_semantics() {
    for (operator, symbol, answer) in [(Operator::Divide, "/", 2), (Operator::Remainder, "%", 1)] {
        let method = operator.method();
        let source = format!("typealias Byte=.u8'builtin;
            trait Operation where Rhs {{assoc Output:.Type'builtin;fn {method}(self,rhs:Rhs)->Output;}}
            impl Operation<Byte> for Byte {{assoc Output:.Type'builtin=Byte;
                fn {method}(self,rhs:Byte)->Byte {{99}}}}
            fn run()->Byte {{7 {symbol} 3}}");
        let mut selectors = luna::protocols::type_constructors::Selectors::default();
        selectors.arithmetic[operator as usize] = Some("input::Operation");
        let package =
            compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::UnsignedInteger(answer)
        );
    }
}

#[test]
fn inferred_protocol_preserves_input_output_and_contract_checks() {
    let source = "typealias Int=.i32'builtin;
        trait Operation where Rhs, requires Rhs == Int {assoc Output:.Type'builtin;
            fn div(self,rhs:Rhs)->Output where requires rhs == 3;}
        impl Operation<Int> for Int {assoc Output:.Type'builtin=bool;
            fn div(self,rhs:Int)->bool where requires rhs == 3 {self == 7}}
        fn run()->bool {7 / (do {let divisor:Int=3;divisor})}";
    let mut selectors = luna::protocols::type_constructors::Selectors::default();
    selectors.arithmetic[Operator::Divide as usize] = Some("input::Operation");
    let package = compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    for invalid in [
        source.replace("divisor:Int=3", "divisor:Int=2"),
        source.replace("7 /", "2147483648 /"),
        source.replace("fn run()->bool", "fn run()->Int"),
        source.replace(
            "impl Operation<Int> for Int",
            "impl Operation<Int> for .i64'builtin",
        ),
    ] {
        assert!(
            compile_with_source_bindings(&invalid, &Default::default(), selectors).is_err(),
            "accepted {invalid}"
        );
    }
}

#[test]
fn untyped_comptime_integers_dispatch_after_defaulting() {
    let source = "typealias Int=.Integer'builtin;
        trait Operation where Rhs {assoc Output:.Type'builtin;comptime fn div(self,rhs:Rhs)->Output;}
        impl Operation<Int> for Int {assoc Output:.Type'builtin=bool;
            comptime fn div(self,rhs:Int)->bool {self == 7 and rhs == 3}}
        comptime fn run()->bool {7 / 3}";
    let mut selectors = luna::protocols::type_constructors::Selectors::default();
    selectors.arithmetic[Operator::Divide as usize] = Some("input::Operation");
    let package = compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn compound_assignment_uses_selected_operator_and_checks_writeback_type() {
    let source = "struct Number {ready:bool}
        trait Operation where Rhs {assoc Output:.Type'builtin;fn add(self,rhs:Rhs)->Output;}
        impl Operation<bool> for Number {assoc Output:.Type'builtin=Number;
            fn add(self,rhs:bool)->Number {Number {ready:self.ready and rhs}}}
        fn run()->bool {var number=Number {ready:true};number += false;not number.ready}";
    let mut selectors = luna::protocols::type_constructors::Selectors::default();
    selectors.arithmetic[Operator::Add as usize] = Some("input::Operation");
    let package = compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let invalid = source
        .replace(
            "assoc Output:.Type'builtin=Number",
            "assoc Output:.Type'builtin=bool",
        )
        .replace(
            "fn add(self,rhs:bool)->Number {Number {ready:self.ready and rhs}}",
            "fn add(self,rhs:bool)->bool {self.ready and rhs}",
        );
    assert!(compile_with_source_bindings(&invalid, &Default::default(), selectors).is_err());
}

#[test]
fn compound_remainder_assignment_uses_explicit_signed_rounding_protocol() {
    let source = "typealias Int=.i32'builtin;
        trait Operation where Rhs {assoc Output:.Type'builtin;fn rem(self,rhs:Rhs)->Output;}
        impl Operation<Int> for Int {assoc Output:.Type'builtin=Int;
            fn rem(self,rhs:Int)->Int {if self == 19 and rhs == 7 {5} else {0}}}
        fn run()->bool {var value:Int=19;value %= 7;value == 5}";
    let mut selectors = luna::protocols::type_constructors::Selectors::default();
    selectors.arithmetic[Operator::Remainder as usize] = Some("input::Operation");
    let package = compile_with_source_bindings(source, &Default::default(), selectors).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(calls(&package, &vm), ["run", "rem"]);
    assert!(support::compile_with_protocols(source, &Default::default()).is_err());
}
