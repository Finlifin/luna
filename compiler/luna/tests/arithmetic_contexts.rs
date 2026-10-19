#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};
use elaboration::arithmetic::Operator;
use support::{calls, compile_with_source_bindings};

#[test]
fn bit_compound_assignment_infers_rhs_from_operator_protocol() {
    for rhs in [
        ".enable",
        "do {.enable}",
        "if true {.enable} else {.disable}",
    ] {
        let source = format!(
            "typealias Bits=.BitVec'builtin<4>;typealias Word=.u8'builtin;
            enum Switch {{enable,disable}}
            trait Operation where Rhs {{assoc Output:.Type'builtin;fn add(self,rhs:Rhs)->Output;}}
            impl Operation<Switch> for Bits {{assoc Output:.Type'builtin=Bits;
                fn add(self,rhs:Switch)->Bits {{rhs match {{.enable=>15,.disable=>0}}}}}}
            fn run()->bool {{var word:Word=0xA0;word'bitvec[0..4] += {rhs};word == 0xAF}}"
        );
        for (operator, symbol) in [
            (Operator::Add, "+="),
            (Operator::Subtract, "-="),
            (Operator::Multiply, "*="),
            (Operator::Divide, "/="),
            (Operator::Remainder, "%="),
        ] {
            let source = source
                .replace("add(", &format!("{}(", operator.method()))
                .replace("+=", symbol);
            let mut selectors = luna::protocols::type_constructors::Selectors::default();
            selectors.arithmetic[operator as usize] = Some("input::Operation");
            for source in [source.clone(), source.replace("fn ", "comptime fn ")] {
                let package =
                    compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
                let result = if source.contains("comptime fn run") {
                    luna::invocation::invoke(&package, "run", 100_000)
                } else {
                    luna::execution::interpret(&package, "run", 100_000)
                };
                assert_eq!(result.unwrap(), Value::Bool(true));
            }
        }
    }
}

#[test]
fn bit_compound_assignment_prepares_location_and_preserves_rhs_updates() {
    let source = "typealias Word=.u8'builtin;typealias Bits=.BitVec'builtin<4>;
        typealias Empty=.BitVec'builtin<0>;
        trait Operation where Rhs {assoc Output:.Type'builtin;fn add(self,rhs:Rhs)->Output;}
        impl Operation<Bits> for Bits {assoc Output:.Type'builtin=Bits;
            fn add(self,rhs:Bits)->Bits {self or rhs}}
        impl Operation<bool> for Empty {assoc Output:.Type'builtin=Empty;
            fn add(self,rhs:bool)->Empty {self}}
        fn target(p:*Word)->*Word {p}
        fn low()->result:Word where ensures result == 0 {0}
        fn high()->result:Word where ensures result == 4 {4}
        fn payload(p:*Word)->Bits {p.* = 0xB0;2}
        fn run()->bool {var word:Word=0xA1;
            target(word.ref).*'bitvec[low()..high()] += payload(word.ref);
            word == 0xB3}
        fn local()->bool {var word:Word=0xA1;word'bitvec[0..4] += 2;word == 0xA3}
        fn early()->bool {var word:Word=0xA1;
            word'bitvec[do {return true;}..4] += payload(word.ref);false}
        fn early_rhs()->bool {var word:Word=0xA1;
            word'bitvec[0..4] += do {return true;};false}
        fn nested()->bool {var word:Word=0xA1;
            word'bitvec[0..8][0..4] += 2;word == 0xA3}
        fn rebound()->bool {let first:Word=0xA1;let second:Word=0;
            var pointer=first.ref;let delta:Bits=2;
            pointer.*'bitvec[do {pointer=second.ref;low()}..high()] += delta;
            first == 0xA3 and second == 0}
        fn empty()->bool {var word:Word=0;
            word'bitvec[4..4] += do {word=0xB0;true};word == 0xB0}";
    let mut selectors = luna::protocols::type_constructors::Selectors::default();
    selectors.arithmetic[Operator::Add as usize] = Some("input::Operation");
    for source in [source.to_owned(), source.replace("fn ", "comptime fn ")] {
        let package =
            compile_with_source_bindings(&source, &Default::default(), selectors).unwrap();
        if !source.contains("comptime fn run") {
            let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
            let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
            assert_eq!(
                vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
                Value::Bool(true)
            );
            assert_eq!(
                calls(&package, &vm),
                vec!["run", "target", "low", "high", "payload", "add"]
            );
        }
        for name in [
            "run",
            "local",
            "early",
            "early_rhs",
            "nested",
            "rebound",
            "empty",
        ] {
            let result = if source.contains("comptime fn run") {
                luna::invocation::invoke(&package, name, 100_000)
            } else {
                luna::execution::interpret(&package, name, 100_000)
            };
            assert_eq!(result.unwrap(), Value::Bool(true));
        }
    }
    let immutable = source.replace("var word:Word", "let word:Word");
    let failure =
        compile_with_source_bindings(&immutable, &Default::default(), selectors).unwrap_err();
    assert!(
        failure.contains("cannot reassign a let binding"),
        "{failure}"
    );
    let out_of_bounds = source.replace("[0..8][0..4]", "[0..9][0..4]");
    assert!(compile_with_source_bindings(&out_of_bounds, &Default::default(), selectors).is_err());
    let wrong_output = source
        .replace(
            "assoc Output:.Type'builtin=Bits",
            "assoc Output:.Type'builtin=Word",
        )
        .replace(
            "fn add(self,rhs:Bits)->Bits {self or rhs}",
            "fn add(self,rhs:Bits)->Word {0}",
        );
    assert!(compile_with_source_bindings(&wrong_output, &Default::default(), selectors).is_err());
    let owned = format!(
        "{source}trait Duplicate {{}}trait Retire {{fn drop(*self);}}
        struct Holder {{value:Word}}impl Retire for Holder {{fn drop(*self) {{}}}}
        fn owned()->bool {{let holder=Holder {{value:0xA1}};
            holder.value'bitvec[0..4] += 2;holder.value == 0xA3}}"
    );
    let package = compile_with_source_bindings(&owned, &Default::default(), selectors).unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(
            &package,
            "owned",
            "Duplicate",
            "Retire",
            100_000
        )
        .unwrap(),
        Value::Bool(true)
    );
}

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
