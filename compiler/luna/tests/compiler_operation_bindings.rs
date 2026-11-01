//! Compiler operation bindings preserve declaration resolution and call checks.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "typealias Type=.Type'builtin;typealias Integer=.Integer'builtin;pure comptime fn List<T>->Type {{T}} {source}"
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            ..Default::default()
        },
    )
}

#[test]
fn compiler_operations_follow_imports_and_constant_aliases() {
    let package=compile(
        "mod operations {pub const length=.list_length'builtin;pub const concat=.list_concat'builtin;}
        use operations.length as size;
        const count=size;
        comptime fn run()->bool {
            let a:List<bool> = [true];let b:List<bool> = [false,true];
            let joined=operations.concat(a,b);
            count(joined) == 3 and joined[0] and not joined[1] and joined[2]
        }"
    ).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn compiler_operation_arguments_keep_source_order_and_single_evaluation() {
    let package=compile(
        "const concat=.list_concat'builtin;const length=.list_length'builtin;
        struct Trace {value:Integer}
        impl Trace {comptime fn input(self,digit:Integer)->List<bool> {self.value = self.value * 10 + digit;[true]}}
        comptime fn run()->bool {let trace=Trace {value:0};let values=concat(trace.input(1),trace.input(2));trace.value == 12 and length(values) == 2}"
    ).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn compiler_operation_bindings_retain_arity_domain_and_phase_errors() {
    for (body, diagnostic) in [
        (
            "comptime fn bad() {let xs:List<bool> = [];length(xs,xs);}",
            "positional list operands",
        ),
        ("comptime fn bad() {length(true);}", "bound meta.List"),
        ("fn bad()->Integer {divide(4,2)}", "bound Splice protocol"),
        (
            "comptime fn bad() {let escaped=length;}",
            "requires a direct call",
        ),
    ] {
        let error=compile(&format!("const length=.list_length'builtin;const divide=.comptime_integer_div_trunc'builtin;{body}")).unwrap_err();
        assert!(error.contains(diagnostic), "{body}: {error}");
    }
    assert!(
        compile("const a=b;const b=a;")
            .unwrap_err()
            .contains("cyclic")
    );
    assert!(compile("const ordinary=3;comptime fn run()->bool {ordinary == 3}").is_ok());
}

#[test]
fn compiler_operation_binding_can_supply_type_observation_without_replaying_calls() {
    let package=compile(
        "const concat=.list_concat'builtin;const length=.list_length'builtin;
        comptime fn Other()->Type {Integer}
        comptime fn run()->bool {let a:List<Type> = [Other()];let b:List<Type> = [bool];let joined=concat(a,b);let T=joined[length(a)];let value:T=true;value}"
    ).unwrap();
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm)
            .iter()
            .filter(|name| **name == "Other")
            .count(),
        1
    );
}

#[test]
fn ordinary_callable_parameters_shadow_operation_bindings() {
    let package = compile(
        "const length=.list_length'builtin;
        comptime fn negate(value:bool)->bool {not value}
        comptime fn apply(length:comptime fn(bool)->bool)->bool {length(false)}
        comptime fn run()->bool {apply(negate)}",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let mut source = "const op0=.list_length'builtin;".to_owned();
    for index in 1..=65 {
        source.push_str(&format!("const op{index}=op{};", index - 1));
    }
    assert!(compile(&source).unwrap_err().contains("excessively deep"));
}

#[test]
fn typed_operation_bindings_check_imported_aliases_and_keep_evaluation_order() {
    let package = compile(
        "mod operations {
            pub const divide: comptime fn(Integer, Integer) -> Integer = .comptime_integer_div_trunc'builtin;
        }
        use operations.divide;
        typealias Count = Integer;
        const alias: comptime fn(Count, Count) -> Count = divide;
        const unannotated = alias;
        struct Trace { value: Integer }
        impl Trace {
            comptime fn input(self, digit: Integer) -> Integer {
                self.value = self.value * 10 + digit;
                digit
            }
        }
        comptime fn run() -> bool {
            let trace = Trace { value: 0 };
            let result = unannotated(trace.input(8), trace.input(2));
            result == 4 and trace.value == 82
        }"
    ).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn typed_operation_bindings_reject_wrong_signatures_even_without_calls() {
    for signature in [
        "fn(Integer, Integer) -> Integer",
        "comptime fn(Integer) -> Integer",
        "comptime fn(bool, Integer) -> Integer",
        "comptime fn(Integer, Integer) -> bool",
        "comptime fn(comptime Integer, Integer) -> Integer",
    ] {
        for initializer in [".comptime_integer_div_trunc'builtin", "original"] {
            let source = format!(
                "const original = .comptime_integer_div_trunc'builtin; const wrong: {signature} = {initializer};"
            );
            let error = compile(&source).unwrap_err();
            assert!(error.contains("signature mismatch"), "{source}: {error}");
        }
    }
    let error =
        compile("const generic: comptime fn(List<bool>) -> Integer = .list_length'builtin;")
            .unwrap_err();
    assert!(error.contains("signature contract"), "{error}");
}

#[test]
fn typed_operation_bindings_preserve_phase_checks() {
    assert!(compile(
        "const divide: comptime fn(Integer, Integer) -> Integer = .comptime_integer_div_trunc'builtin;
        fn bad() -> Integer { let operation = divide; operation(8, 2) }"
    ).is_err());
    for (body, diagnostic) in [(
        "fn bad() -> Integer { divide(8, 2) }",
        "bound Splice protocol",
    )] {
        let source = format!(
            "const divide: comptime fn(Integer, Integer) -> Integer = .comptime_integer_div_trunc'builtin; {body}"
        );
        let error = compile(&source).unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
    }
}

#[test]
fn typed_reflection_signatures_preserve_normal_form_identity() {
    let package = compile(
        "const fields: comptime fn(Type) -> List<Type> = .struct_field_types'builtin;
        const Representation: comptime fn(Type) -> Type = .newtype_representation'builtin;
        struct Record { flag: bool }
        newtype Wrapper = bool;
        comptime fn run() -> bool {
            fields(Record)[0] == bool and Representation(Wrapper) == bool
        }",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let error = compile(
        "pure comptime fn Other<T> -> Type { T }
        const wrong: comptime fn(Type) -> Other<Type> = .struct_field_types'builtin;",
    )
    .unwrap_err();
    assert!(error.contains("signature mismatch"), "{error}");
    let error = compile(
        "const original: comptime fn(Type) -> List<Type> = .struct_field_types'builtin;
        const wrong: comptime fn(Type) -> List<bool> = original;
        const copied = wrong;",
    )
    .unwrap_err();
    assert!(error.contains("signature mismatch"), "{error}");
}

fn run_all_carriers(source: &str) -> nir::program::Package {
    use comptime_interpretor::{Budget, Interpreter};
    let package = compile(source).unwrap();
    let entry = support::id(&package, "run");
    for bytecode in [false, true] {
        let mut vm = if bytecode {
            Interpreter::for_comptime(&package, Budget::default())
        } else {
            Interpreter::for_comptime_tree(&package, Budget::default())
        }
        .unwrap();
        assert_eq!(
            vm.invoke(entry, vec![])
                .unwrap_or_else(|error| panic!("bytecode={bytecode}: {error:?}\n{package}")),
            Value::Bool(true)
        );
    }
    let key = iir::InstanceKey {
        declaration: entry,
        arguments: vec![].into(),
    };
    let graph = elaboration::materialization::instantiate(
        &package,
        &[key.clone()],
        Default::default(),
        elaboration::ptc::Budget::default(),
    )
    .unwrap();
    for bytecode in [false, true] {
        let mut vm = if bytecode {
            Interpreter::from_iir_for_comptime(&graph, Budget::default())
        } else {
            Interpreter::from_iir_for_comptime_tree(&graph, Budget::default())
        }
        .unwrap();
        assert_eq!(vm.invoke_instance(&key, vec![]).unwrap(), Value::Bool(true));
    }
    package
}

#[test]
fn fixed_operation_values_use_ordinary_callbacks_and_container_storage() {
    let package = run_all_carriers(
        "const divide: comptime fn(Integer, Integer) -> Integer = .comptime_integer_div_trunc'builtin;
        const alias = divide;
        const remainder = .comptime_integer_rem_trunc'builtin;
        comptime fn apply(op: comptime fn(Integer, Integer) -> Integer, a: Integer, b: Integer) -> Integer {
            op(a, b)
        }
        const prepared = apply(alias, 12, 3);
        comptime fn user_add(a: Integer, b: Integer) -> Integer { a + b }
        comptime fn run() -> bool {
            let functions: List<comptime fn(Integer, Integer) -> Integer> = [divide, alias, remainder, user_add];
            apply(functions[0], 9, 2) == 4 and functions[1](15, 3) == 5
                and functions[2](9, 2) == 1 and prepared == 4 and functions[3](9, 2) == 11
        }"
    );
    let functions: Vec<_> = package
        .functions
        .iter()
        .filter(|function| function.name.starts_with("<compiler-operation."))
        .collect();
    assert_eq!(functions.len(), 2);
    assert!(
        functions
            .iter()
            .all(|function| function.phase == nir::program::FunctionPhase::Comptime)
    );
}

#[test]
fn fixed_reflection_and_object_operations_are_callable_values() {
    run_all_carriers(
        "typealias Object = .Object'builtin;
        typealias Symbol = .Symbol'builtin;
        const fields = .struct_field_types'builtin;
        const Representation = .newtype_representation'builtin;
        const length = .object_length'builtin;
        const concatenate = .object_concat'builtin;
        const key = .object_key'builtin;
        const shapes = .enum_variant_shapes'builtin;
        const list_length = .list_length'builtin;
        struct Record { flag: bool }
        newtype Wrapper = bool;
        enum Choice { empty, payload(bool) }
        comptime fn observe(subject: Type, inspect: comptime fn(Type) -> List<Type>) -> bool {
            inspect(subject)[0] == bool
        }
        comptime fn run() -> bool {
            let inspect = fields;
            let Repr = Representation;
            let size = length;
            let join = concatenate;
            let lookup = key;
            let variants = shapes;
            let entries: Object = join({flag: true}, {tail: false});
            observe(Record, inspect) and Repr(Wrapper) == bool
                and size(entries) == 2 and (lookup(entries, 0) matches .flag?)
                and list_length(variants(Choice)) == 2
        }",
    );
}

#[test]
fn operation_callback_failure_does_not_poison_later_invocation() {
    use comptime_interpretor::{Budget, Interpreter};
    let package = compile(
        "const divide = .comptime_integer_div_trunc'builtin;
        comptime fn run(a: Integer, b: Integer) -> Integer {
            let operation = divide;
            operation(a, b)
        }",
    )
    .unwrap();
    let entry = support::id(&package, "run");
    let key = iir::InstanceKey {
        declaration: entry,
        arguments: vec![].into(),
    };
    let graph = elaboration::materialization::instantiate(
        &package,
        &[key.clone()],
        Default::default(),
        elaboration::ptc::Budget::default(),
    )
    .unwrap();
    let number = |n| Value::Integer(comptime_runtime::Integer::from_u128(n));
    for bytecode in [false, true] {
        for instantiated in [false, true] {
            let mut vm = match (instantiated, bytecode) {
                (false, false) => Interpreter::for_comptime_tree(&package, Budget::default()),
                (false, true) => Interpreter::for_comptime(&package, Budget::default()),
                (true, false) => Interpreter::from_iir_for_comptime_tree(&graph, Budget::default()),
                (true, true) => Interpreter::from_iir_for_comptime(&graph, Budget::default()),
            }
            .unwrap();
            let mut invoke = |arguments| {
                if instantiated {
                    vm.invoke_instance(&key, arguments)
                } else {
                    vm.invoke(entry, arguments)
                }
            };
            assert!(invoke(vec![number(9), number(0)]).is_err());
            assert_eq!(invoke(vec![number(9), number(2)]).unwrap(), number(4));
        }
    }
}

#[test]
fn named_fixed_operations_use_ordinary_call_nodes_and_source_order() {
    use nir::program::ExprKind as K;
    let package = run_all_carriers(
        "const divide = .comptime_integer_div_trunc'builtin;
        struct Trace { value: Integer }
        impl Trace {
            comptime fn input(self, digit: Integer) -> Integer {
                self.value = self.value * 10 + digit;
                digit
            }
        }
        comptime fn run() -> bool {
            let trace = Trace { value: 0 };
            let quotient = divide(trace.input(8), trace.input(2));
            quotient == 4 and trace.value == 82
        }",
    );
    let operation = package
        .functions
        .iter()
        .find(|function| function.name == "<compiler-operation.comptime_integer_div_trunc>")
        .unwrap()
        .declaration;
    let run = package
        .functions
        .iter()
        .find(|function| function.name == "run")
        .unwrap();
    let mut calls = 0;
    nir::visit::expressions(run, &mut 10_000, |expression, _| {
        assert!(!matches!(
            expression.kind,
            K::IntegerBinary {
                op: nir::program::IntegerOp::DivideTrunc,
                ..
            }
        ));
        if let K::Call { callee, .. } = &expression.kind {
            if matches!(callee.kind, K::Function(id) if id == operation) {
                calls += 1;
            }
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(calls, 1);
}

#[test]
fn named_operation_calls_splice_only_the_comptime_result() {
    let source = "
        typealias Type = .Type'builtin;
        typealias Integer = .Integer'builtin;
        trait Splice {
            assoc RuntimeTarget: Type;
            fn splice(comptime data: Self) -> RuntimeTarget;
        }
        impl Splice for bool {
            assoc RuntimeTarget: Type = bool;
            fn splice(comptime data: bool) -> bool { data }
        }
        comptime fn equals_four(value: Integer) -> bool { value == 4 }
        impl Splice for Integer {
            assoc RuntimeTarget: Type = bool;
            fn splice(comptime data: Integer) -> bool {
                equals_four(data)
            }
        }
        const divide = .comptime_integer_div_trunc'builtin;
        fn run() -> bool { divide(9, 2) }
    ";
    let package =
        support::compile_with_bindings(source, &Default::default(), Some("Splice")).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let invalid = source.replace(
        "fn run() -> bool { divide(9, 2) }",
        "fn run(input: Integer) -> bool { divide(input, 2) }",
    );
    assert!(support::compile_with_bindings(&invalid, &Default::default(), Some("Splice")).is_err());
}

#[test]
fn definition_operation_values_preserve_fresh_type_identity() {
    let package = run_all_carriers(
        "typealias Symbol = .Symbol'builtin;
        const Newtype: comptime fn(Symbol, Type) -> Type = .build_newtype'builtin;
        const Record: comptime fn(Symbol, List<Symbol>, List<Type>) -> Type = .build_struct'builtin;
        comptime fn Build(make: comptime fn(Symbol, Type) -> Type) -> Type {
            let makers: List<comptime fn(Symbol, Type) -> Type> = [make];
            makers[0](.Wrapper, bool)
        }
        comptime fn BuildViaValue() -> Type { Build(Newtype) }
        comptime fn RejectViaValue() -> Type {
            let make = Record;
            make(.Invalid, [.flag, .flag], [bool, bool])
        }
        const First = Build(Newtype);
        const Second = Build(Newtype);
        comptime fn BuildRecord() -> Type {
            let make = Record;
            make(.Record, [.flag], [bool])
        }
        const One = BuildRecord();
        const Two = BuildRecord();
        const Representation = .newtype_representation'builtin;
        const fields = .struct_field_types'builtin;
        comptime fn run() -> bool {
            Representation(First) == bool and Representation(Second) == bool
                and fields(One)[0] == bool and fields(Two)[0] == bool
        }",
    );
    let entry = support::id(&package, "BuildViaValue");
    let rejected = support::id(&package, "RejectViaValue");
    let rejected_key = iir::InstanceKey {
        declaration: rejected,
        arguments: vec![].into(),
    };
    let key = iir::InstanceKey {
        declaration: entry,
        arguments: vec![].into(),
    };
    let graph = elaboration::materialization::instantiate(
        &package,
        &[key.clone(), rejected_key.clone()],
        Default::default(),
        elaboration::ptc::Budget::default(),
    )
    .unwrap();
    for instantiated in [false, true] {
        for bytecode in [false, true] {
            let publisher = elaboration::definitions::AllocatedDefinitionPublisher::new(
                &package, 9900, 1, 1_000_000,
            );
            let publish = |request: &comptime_interpretor::NewtypeBuildRequest, steps: &mut u64| {
                publisher.prepare_newtype(request, steps)
            };
            let publish_struct = |request: &comptime_interpretor::StructBuildRequest,
                                  steps: &mut u64| {
                publisher.prepare_struct(request, steps)
            };
            use comptime_interpretor::{Budget, Interpreter};
            let mut vm = match (instantiated, bytecode) {
                (false, false) => Interpreter::for_comptime_tree(&package, Budget::default()),
                (false, true) => Interpreter::for_comptime(&package, Budget::default()),
                (true, false) => Interpreter::from_iir_for_comptime_tree(&graph, Budget::default()),
                (true, true) => Interpreter::from_iir_for_comptime(&graph, Budget::default()),
            }
            .unwrap()
            .with_newtype_builder(&publish)
            .with_struct_builder(&publish_struct);
            let failure = if instantiated {
                vm.invoke_instance(&rejected_key, vec![])
            } else {
                vm.invoke(rejected, vec![])
            };
            assert!(failure.is_err());
            let mut invoke = || {
                if instantiated {
                    vm.invoke_instance(&key, vec![])
                } else {
                    vm.invoke(entry, vec![])
                }
            };
            let first = invoke().unwrap();
            let second = invoke().unwrap();
            assert_ne!(first, second);
            drop(vm);
            let (published, _) = publisher.finish();
            assert_eq!(published.newtypes.len(), 2);
            assert!(published.structs.is_empty());
        }
    }
    assert_eq!(package.newtypes.len(), 2);
    assert_ne!(
        package.newtypes[0].declaration,
        package.newtypes[1].declaration
    );
    assert_eq!(package.structs.len(), 2);
    assert_ne!(
        package.structs[0].declaration,
        package.structs[1].declaration
    );
    for name in ["build_newtype", "build_struct"] {
        let function = package
            .functions
            .iter()
            .find(|function| function.name == format!("<compiler-operation.{name}>"))
            .unwrap();
        assert!(!function.pure, "definition publication is effectful");
    }
}
