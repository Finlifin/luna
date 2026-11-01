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
            let joined=operations.concat(bool,a,b);
            count(bool,joined) == 3 and joined[0] and not joined[1] and joined[2]
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
        comptime fn run()->bool {let trace=Trace {value:0};let values=concat(bool,trace.input(1),trace.input(2));trace.value == 12 and length(bool,values) == 2}"
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
            "comptime fn bad() {let xs:List<bool> = [];length(bool,xs,xs);}",
            "argument",
        ),
        ("comptime fn bad() {length(bool,true);}", "type"),
        ("fn bad()->Integer {divide(4,2)}", "bound Splice protocol"),
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
        comptime fn run()->bool {let a:List<Type> = [Other()];let b:List<Type> = [bool];let joined=concat(Type,a,b);let T=joined[length(Type,a)];let value:T=true;value}"
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
    assert!(error.contains("signature mismatch"), "{error}");
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
    run_carriers(source, "run", "run")
}

// Generic comptime bodies execute in the engine. Only their completed result
// crosses into a runtime instance; IIR must not retain their open Type inputs.
fn run_staged_carriers(source: &str) -> nir::program::Package {
    run_carriers(
        &format!(
            "{source}\nfn run() -> bool {{ inline if compute() {{ true }} else {{ false }} }}"
        ),
        "compute",
        "run",
    )
}

fn run_carriers(source: &str, invocation: &str, instance: &str) -> nir::program::Package {
    use comptime_interpretor::{Budget, Interpreter};
    let package = compile(source).unwrap();
    let entry = support::id(&package, invocation);
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
        declaration: support::id(&package, instance),
        arguments: vec![].into(),
    };
    let graph = elaboration::materialization::instantiate(
        &package,
        &[key.clone()],
        Default::default(),
        elaboration::ptc::Budget::default(),
    )
    .unwrap();
    if invocation != instance {
        let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
        assert_eq!(vm.invoke_instance(&key, vec![]).unwrap(), Value::Bool(true));
        return package;
    }
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
                and list_length(Symbol,variants(Choice)) == 2
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

#[test]
fn runtime_function_instances_remain_data_in_comptime_lists() {
    let package = run_all_carriers(
        "fn identity(value: T) -> T where T { value }
        comptime fn pick() -> fn(bool) -> bool {
            let choices: List<fn(bool) -> bool> = [identity<bool>];
            choices[0]
        }
        pure comptime fn Carrier<callback: fn(bool) -> bool> -> Type { bool }
        typealias Stored = Carrier<pick()>;
        comptime fn StoredType() -> Type { Stored }
        comptime fn run() -> bool { Stored == Carrier<identity<bool>> }",
    );
    let key = iir::InstanceKey {
        declaration: support::id(&package, "StoredType"),
        arguments: vec![].into(),
    };
    let graph = elaboration::materialization::instantiate(
        &package,
        &[key.clone()],
        Default::default(),
        elaboration::ptc::Budget::default(),
    )
    .unwrap();
    let mut vm =
        comptime_interpretor::Interpreter::from_iir_for_comptime(&graph, Default::default())
            .unwrap();
    let value = vm.invoke_instance(&key, vec![]).unwrap();
    vm.validate_immutable_input(&value, &nir::Term::Primitive(nir::Primitive::Type))
        .unwrap();
    let missing = iir::Package {
        normal_forms: graph.normal_forms.clone(),
        ..Default::default()
    };
    let mut missing_vm =
        comptime_interpretor::Interpreter::from_iir_for_comptime(&missing, Default::default())
            .unwrap();
    assert!(
        missing_vm
            .validate_immutable_input(&value, &nir::Term::Primitive(nir::Primitive::Type))
            .is_err()
    );
}

#[test]
fn staged_runtime_methods_can_be_stored_as_comptime_list_data() {
    let package = run_all_carriers(
        "trait Selector { fn select(comptime value: bool) -> bool; }
        impl Selector for bool { fn select(comptime value: bool) -> bool { true } }
        const length = .list_length'builtin;
        comptime fn callbacks() -> List<fn(comptime bool) -> bool> { [bool.select] }
        const saved = callbacks();
        comptime fn run() -> bool {
            let methods = saved;
            let copied = methods;
            length(fn(comptime bool) -> bool, copied) == 1
        }",
    );
    let key = iir::InstanceKey {
        declaration: support::id(&package, "run"),
        arguments: vec![].into(),
    };
    let mut graph = elaboration::materialization::instantiate(
        &package,
        &[key.clone()],
        Default::default(),
        elaboration::ptc::Budget::default(),
    )
    .unwrap();
    let method = graph
        .interfaces
        .iter_mut()
        .flat_map(|interface| &mut interface.methods)
        .find(|method| method.name == "select")
        .unwrap();
    method
        .requirements
        .push(nir::program::TypePredicate::Bool(false));
    for bytecode in [false, true] {
        let mut vm = if bytecode {
            comptime_interpretor::Interpreter::from_iir_for_comptime(&graph, Default::default())
        } else {
            comptime_interpretor::Interpreter::from_iir_for_comptime_tree(
                &graph,
                Default::default(),
            )
        }
        .unwrap();
        assert_eq!(
            vm.invoke_instance(&key, vec![]).unwrap_err().kind,
            comptime_interpretor::ErrorKind::TypeContractRejected
        );
    }
}

#[test]
fn dependent_object_operation_values_use_explicit_type_inputs() {
    run_staged_carriers(
        "typealias Object = .Object'builtin;
        typealias Symbol = .Symbol'builtin;
        const lookup: comptime fn(comptime T: Type, Object, Symbol) -> ?T = .object_get'builtin;
        const at: comptime fn(comptime T: Type, Object, Integer) -> ?T = .object_value'builtin;
        const alias = lookup;
        comptime fn apply(op: comptime fn(comptime T: Type, Object, Symbol) -> ?T, object: Object) -> ?bool {
            op(bool, object, .flag)
        }
        const checked = apply(alias, {flag: true});
        comptime fn compute() -> bool {
            let functions: List<comptime fn(comptime T: Type, Object, Symbol) -> ?T> = [lookup, alias];
            let index = at;
            let object: Object = {flag: true, count: 7};
            (checked matches true?) and (apply(functions[0], object) matches true?)
                and (functions[1](Integer, object, .count) matches 7?)
                and (functions[0](bool, object, .count) matches null)
                and (functions[0](bool, object, .missing) matches null)
                and (index(bool, object, 0) matches true?)
                and (index(Integer, object, 1) matches 7?)
                and (index(bool, object, 1) matches null)
        }",
    );
}

#[test]
fn dependent_source_callable_values_keep_invocation_binders() {
    run_staged_carriers(
        "comptime fn select(comptime T: Type, value: T) -> T { value }
        comptime fn compute() -> bool {
            let functions: List<comptime fn(comptime T: Type, value: T) -> T> = [select];
            functions[0](bool, true)
        }",
    );
}

#[test]
fn dependent_list_operations_are_ordinary_callable_values() {
    run_staged_carriers(
        "const length: comptime fn(comptime T: Type, List<T>) -> Integer = .list_length'builtin;
        const concat: comptime fn(comptime T: Type, List<T>, List<T>) -> List<T> = .list_concat'builtin;
        comptime fn apply(op: comptime fn(comptime T: Type, List<T>, List<T>) -> List<T>) -> List<bool> {
            op(bool, [true], [false])
        }
        comptime fn compute() -> bool {
            let joins: List<comptime fn(comptime T: Type, List<T>, List<T>) -> List<T>> = [concat];
            let count = length;
            let flags = apply(joins[0]);
            let types = joins[0](Type, [bool], [Integer]);
            count(bool, flags) == 2 and flags[0] and not flags[1]
                and count(Type, types) == 2 and types[0] == bool and types[1] == Integer
        }",
    );
}

#[test]
fn text_operation_values_use_comptime_callbacks_and_list_storage() {
    run_staged_carriers(
        "typealias str = .str'builtin; typealias Symbol = .Symbol'builtin;
        typealias u64 = .u64'builtin; typealias u8 = .u8'builtin;
        const length: comptime fn(str) -> u64 = .str_byte_length'builtin;
        const byte: comptime fn(str, u64) -> ?u8 = .str_byte'builtin;
        const symbol_length: comptime fn(Symbol) -> u64 = .symbol_byte_length'builtin;
        const symbol_byte: comptime fn(Symbol, u64) -> ?u8 = .symbol_byte'builtin;
        comptime fn count(f: comptime fn(str) -> u64, text: str) -> u64 { f(text) }
        comptime fn compute() -> bool {
            let readers: List<comptime fn(str, u64) -> ?u8> = [byte];
            let read = readers[0];
            let last = read(\"é\", 1) match { value? => value == 169, null => false };
            let past = symbol_byte(.hello, 5) match { _? => false, null => true };
            count(length, \"é\") == 2 and symbol_length(.hello) == 5 and last and past
        }",
    );
}

#[test]
fn text_operation_aliases_cannot_change_the_original_phase() {
    for declarations in [
        "const original: fn(str) -> u64 = .str_byte_length'builtin;
         const alias: comptime fn(str) -> u64 = original;",
        "const original: comptime fn(str) -> u64 = .str_byte_length'builtin;
         const alias: fn(str) -> u64 = original;",
        "const alias: comptime fn(str) -> bool = .str_byte_length'builtin;",
    ] {
        let error = compile(&format!(
            "typealias str = .str'builtin; typealias u64 = .u64'builtin; {declarations}"
        ))
        .unwrap_err();
        assert!(error.contains("signature mismatch"), "{error}");
    }
}

#[test]
fn dependent_list_binding_signatures_use_the_bound_constructor() {
    let package = support::compile_with_source_bindings(
        "typealias Type = .Type'builtin;
        typealias Integer = .Integer'builtin;
        pub const List: pure comptime fn<T: Type> -> Type = .List'builtin;
        const length: comptime fn(comptime T: Type, List<T>) -> Integer = .list_length'builtin;
        comptime fn run() -> bool { length(bool, [true]) == 1 }",
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn dependent_object_edit_values_preserve_payload_domains_and_snapshots() {
    run_staged_carriers(
        "typealias Object = .Object'builtin;
        typealias Symbol = .Symbol'builtin;
        typealias char = .char'builtin;
        const append: comptime fn(comptime T: Type, Object, ?Symbol, T) -> Object = .object_append'builtin;
        const replace: comptime fn(comptime T: Type, Object, Integer, T) -> Object = .object_replace'builtin;
        const get = .object_get'builtin;
        const at = .object_value'builtin;
        comptime fn apply(op: comptime fn(comptime T: Type, Object, ?Symbol, T) -> Object, object: Object) -> Object {
            op(bool, object, .flag, false)
        }
        comptime fn compute() -> bool {
            let edits: List<comptime fn(comptime T: Type, Object, ?Symbol, T) -> Object> = [append];
            let update = replace;
            let original: Object = {flag: true};
            let added = apply(edits[0], original);
            let text = edits[0](char, added, null, 'x');
            let changed = update(Integer, text, 0, 7);
            let empty: ?bool = null;
            let nested = edits[0](?bool, changed, .empty, empty);
            (get(bool, original, .flag) matches true?)
                and (get(bool, added, .flag) matches false?)
                and (at(bool, text, 0) matches true?)
                and (at(Integer, changed, 0) matches 7?)
                and (at(char, changed, 2) matches 'x'?)
                and (get(?bool, nested, .empty) match {v? => v matches null, null => false})
        }",
    );
}

#[test]
fn dependent_object_edit_payloads_are_checked_by_the_callable_signature() {
    for source in [
        "const append: comptime fn(comptime T: Type, Object, ?Symbol, bool) -> Object = .object_append'builtin;",
        "const replace = .object_replace'builtin; comptime fn bad() -> Object { replace(bool, {}, 0, 'x') }",
    ] {
        assert!(
            compile(&format!(
                "typealias Object = .Object'builtin; typealias Symbol = .Symbol'builtin; {source}"
            ))
            .is_err()
        );
    }
}

#[test]
fn open_comptime_invocation_inputs_do_not_become_iir_instances() {
    let package = compile("comptime fn select(comptime T: Type, value: T) -> T { value }").unwrap();
    let key = iir::InstanceKey {
        declaration: support::id(&package, "select"),
        arguments: vec![].into(),
    };
    let error = elaboration::materialization::instantiate(
        &package,
        &[key],
        Default::default(),
        elaboration::ptc::Budget::default(),
    )
    .unwrap_err();
    assert!(
        error.contains("requires invocation input staging before IIR"),
        "{error}"
    );
}

#[test]
fn dependent_object_operation_annotations_preserve_phase_and_result_dependency() {
    for signature in [
        "comptime fn(Type, Object, Integer) -> ?bool",
        "comptime fn(comptime T: Type, Object, Integer) -> T",
        "comptime fn(comptime T: Type, Object, Integer) -> ?bool",
        "fn(comptime T: Type, Object, Integer) -> ?T",
    ] {
        let source = format!(
            "typealias Object = .Object'builtin; const wrong: {signature} = .object_value'builtin;"
        );
        let error = compile(&source).unwrap_err();
        assert!(error.contains("signature mismatch"), "{signature}: {error}");
    }
}

#[test]
fn dependent_object_callback_bounds_fail_without_poisoning_the_next_call() {
    use comptime_interpretor::{Budget, ErrorKind, Interpreter};
    let package = compile(
        "typealias Object = .Object'builtin;
        const at = .object_value'builtin;
        comptime fn run(index: Integer) -> ?bool {
            let callback = at;
            callback(bool, {flag: true}, index)
        }",
    )
    .unwrap();
    for bytecode in [false, true] {
        let mut vm = if bytecode {
            Interpreter::for_comptime(&package, Budget::default())
        } else {
            Interpreter::for_comptime_tree(&package, Budget::default())
        }
        .unwrap();
        let entry = support::id(&package, "run");
        let number = |n| Value::Integer(comptime_runtime::Integer::from_u128(n));
        assert_eq!(
            vm.invoke(entry, vec![number(9)]).unwrap_err().kind,
            ErrorKind::IndexOutOfBounds
        );
        assert_eq!(
            vm.invoke(entry, vec![number(0)]).unwrap(),
            Value::Optional {
                ty: nir::Term::Optional(Box::new(nir::Term::Primitive(nir::Primitive::Bool))),
                value: Some(Box::new(Value::Bool(true))),
            }
        );
    }
}

#[test]
fn a_text_operation_alias_cannot_establish_the_original_phase() {
    let error = compile(
        "typealias str = .str'builtin; typealias u64 = .u64'builtin;
        const original = .str_byte_length'builtin;
        const alias: comptime fn(str) -> u64 = original;",
    )
    .unwrap_err();
    assert!(
        error.contains("explicit phase on its original binding"),
        "{error}"
    );
}

#[test]
fn integer_representation_values_keep_machine_width_through_callbacks() {
    run_staged_carriers(
        "typealias i8 = .i8'builtin; typealias u128 = .u128'builtin;
        const bits: comptime fn(i8) -> u128 = .integer_representation'builtin;
        comptime fn observe(f: comptime fn(i8) -> u128, value: i8) -> u128 { f(value) }
        comptime fn compute() -> bool {
            let operations: List<comptime fn(i8) -> u128> = [bits];
            observe(operations[0], -1) == 255 and observe(bits, -128) == 128
        }",
    );
}

#[test]
fn integer_representation_bindings_reject_wrong_domains_results_and_alias_phases() {
    for declarations in [
        "const wrong: comptime fn(bool) -> u128 = .integer_representation'builtin;",
        "const wrong: comptime fn(Integer) -> u128 = .integer_representation'builtin;",
        "const wrong: comptime fn(i8) -> i8 = .integer_representation'builtin;",
        "const wrong: comptime fn(i8, i8) -> u128 = .integer_representation'builtin;",
        "const original: comptime fn(i8) -> u128 = .integer_representation'builtin;
         const alias: fn(i8) -> u128 = original;",
        "const original: fn(i8) -> u128 = .integer_representation'builtin;
         const alias: fn(u8) -> u128 = original;",
    ] {
        let error = compile(&format!(
            "typealias i8 = .i8'builtin; typealias u8 = .u8'builtin;
             typealias u128 = .u128'builtin; {declarations}"
        ))
        .unwrap_err();
        assert!(
            error.contains("signature mismatch"),
            "{declarations}: {error}"
        );
    }
}
